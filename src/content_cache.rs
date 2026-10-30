//! Persistent store of the text extracted from documents, so a content
//! search reads each docx/pdf/xlsx only once until it changes.
//!
//! The store lives in one folder (`%LOCALAPPDATA%\BahthSari\content\` in the
//! app) and holds two files:
//! - `content.dat`: a header (magic, version, store id) followed by
//!   lz4-compressed texts, appended one after another;
//! - `content.idx`: a bincode map from path to [`Meta`] (size, mtime, where
//!   the text sits in `content.dat`), followed by a checksum.
//!
//! An entry is used only while the file's size and modification time still
//! match. Files with no text (or whose extraction failed) are remembered
//! too, so they are not re-parsed on every search. A missing, corrupt or
//! mismatched store is thrown away and started afresh; nothing here panics.
//!
//! Reads take a shared lock only, so the parallel content search reads many
//! documents at once; writes go through one mutex.

// Only the Windows UI and the tests use this module.

use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, RwLock};
use std::time::Duration;

use crate::extract;
use crate::index::Entry;

/// Bumped whenever the on-disk format changes; an older store is rebuilt.
const VERSION: u32 = 1;
const MAGIC: [u8; 8] = *b"BHTHTXT\0";
/// Magic, version (u32) and store id (u64).
const HEADER_LEN: u64 = 20;
/// Texts longer than this are not stored (they are re-extracted each time).
/// The extractors stop at about 64 MB of output; this leaves a little slack.
pub const MAX_TEXT: usize = 80 * 1024 * 1024;
/// Default cap on the size of `content.dat`.
pub const DEFAULT_MAX_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// The background warm-up saves its progress after this many documents.
const FLUSH_EVERY: usize = 200;

const DAT: &str = "content.dat";
const IDX: &str = "content.idx";

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// Compressed text at `offset`.
    Text,
    /// The document has no text, or extraction failed.
    NoText,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
struct Meta {
    size: u64,
    mtime: i64,
    offset: u64,
    /// Compressed length in `content.dat`.
    len: u32,
    /// Length of the text in bytes.
    raw_len: u32,
    /// FNV-1a of the compressed bytes, to catch a damaged `content.dat`.
    hash: u64,
    kind: Kind,
}

type Map = HashMap<Box<str>, Meta>;

#[derive(Deserialize)]
struct IdxFile {
    version: u32,
    id: u64,
    dat_len: u64,
    entries: Map,
}

/// Same layout as [`IdxFile`], borrowing the map.
#[derive(Serialize)]
struct IdxRef<'a> {
    version: u32,
    id: u64,
    dat_len: u64,
    entries: &'a Map,
}

/// What the store holds, for logs and the status bar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// Documents with a cached result (text or "no text").
    pub documents: usize,
    pub texts: usize,
    pub no_text: usize,
    /// Size of `content.dat`.
    pub bytes: u64,
    /// Bytes in `content.dat` no entry points to any more.
    pub garbage: u64,
    /// The size cap was reached: new texts are no longer stored.
    pub full: bool,
}

struct Inner {
    map: Map,
    id: u64,
    /// Where the next record goes; everything after it is ignored.
    dat_len: u64,
    garbage: u64,
    /// The map changed since the last flush.
    dirty: bool,
    full: bool,
    writer: Option<File>,
}

pub struct ContentCache {
    dir: PathBuf,
    max_bytes: u64,
    inner: Mutex<Inner>,
    /// Shared handle for positional reads; replaced only by `compact`.
    reader: RwLock<Option<File>>,
    /// Content searches running now; the warm-up pauses meanwhile.
    searching: AtomicUsize,
}

/// Marks a content search as running until dropped.
pub struct SearchGuard<'a>(&'a ContentCache);

impl Drop for SearchGuard<'_> {
    fn drop(&mut self) {
        self.0.searching.fetch_sub(1, Ordering::Relaxed);
    }
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

fn header(id: u64) -> [u8; HEADER_LEN as usize] {
    let mut h = [0u8; HEADER_LEN as usize];
    h[..8].copy_from_slice(&MAGIC);
    h[8..12].copy_from_slice(&VERSION.to_le_bytes());
    h[12..].copy_from_slice(&id.to_le_bytes());
    h
}

fn new_id() -> u64 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    fnv(&[t.to_le_bytes(), u64::from(std::process::id()).to_le_bytes()].concat()) | 1
}

#[cfg(unix)]
fn read_at(f: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(f, buf, offset)
}

#[cfg(windows)]
fn read_at(f: &File, mut buf: &mut [u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        match f.seek_read(buf, offset) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => {
                buf = &mut buf[n..];
                offset += n as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// The idx payload, or `None` when the checksum or the encoding is wrong.
fn decode_idx(bytes: &[u8]) -> Option<IdxFile> {
    let (payload, sum) = bytes.split_at_checked(bytes.len().checked_sub(8)?)?;
    if fnv(payload).to_le_bytes() != sum {
        return None;
    }
    bincode::deserialize(payload).ok()
}

/// A text or a document whose text is worth caching: supported and not
/// beyond the extraction size limit.
pub fn is_document(e: &Entry) -> bool {
    !e.is_dir && e.size > 0 && e.size <= extract::MAX_FILE_SIZE && extract::is_supported(&e.ext())
}

impl ContentCache {
    /// Opens the store in `dir`, creating it, or starting it afresh when it
    /// is damaged or from another version.
    pub fn open(dir: &Path) -> io::Result<ContentCache> {
        Self::open_with_limit(dir, DEFAULT_MAX_BYTES)
    }

    /// [`open`](Self::open) with a cap of `max_bytes` on `content.dat`.
    pub fn open_with_limit(dir: &Path, max_bytes: u64) -> io::Result<ContentCache> {
        fs::create_dir_all(dir)?;
        let (inner, reader) = match Self::load(dir) {
            Some(loaded) => loaded,
            None => Self::create_fresh(dir)?,
        };
        Ok(ContentCache {
            dir: dir.to_path_buf(),
            max_bytes,
            inner: Mutex::new(inner),
            reader: RwLock::new(Some(reader)),
            searching: AtomicUsize::new(0),
        })
    }

    fn load(dir: &Path) -> Option<(Inner, File)> {
        let idx = decode_idx(&fs::read(dir.join(IDX)).ok()?)?;
        if idx.version != VERSION || idx.dat_len < HEADER_LEN {
            return None;
        }
        let mut f = OpenOptions::new().read(true).write(true).open(dir.join(DAT)).ok()?;
        let mut head = [0u8; HEADER_LEN as usize];
        f.read_exact(&mut head).ok()?;
        if head != header(idx.id) {
            return None;
        }
        let len = f.metadata().ok()?.len();
        if len < idx.dat_len {
            return None;
        }
        if len > idx.dat_len {
            // Records written after the last flush: nothing points to them.
            f.set_len(idx.dat_len).ok()?;
        }
        let mut live = 0u64;
        for m in idx.entries.values().filter(|m| m.kind == Kind::Text) {
            if m.offset < HEADER_LEN || m.offset + u64::from(m.len) > idx.dat_len {
                return None;
            }
            live += u64::from(m.len);
        }
        let garbage = (idx.dat_len - HEADER_LEN).checked_sub(live)?;
        let reader = File::open(dir.join(DAT)).ok()?;
        let inner =
            Inner { map: idx.entries, id: idx.id, dat_len: idx.dat_len, garbage, dirty: false, full: false, writer: Some(f) };
        Some((inner, reader))
    }

    fn create_fresh(dir: &Path) -> io::Result<(Inner, File)> {
        let _ = fs::remove_file(dir.join(IDX));
        let id = new_id();
        let mut f = OpenOptions::new().read(true).write(true).create(true).truncate(true).open(dir.join(DAT))?;
        f.write_all(&header(id))?;
        let reader = File::open(dir.join(DAT))?;
        let mut inner =
            Inner { map: Map::new(), id, dat_len: HEADER_LEN, garbage: 0, dirty: true, full: false, writer: Some(f) };
        write_idx(dir, &mut inner)?;
        Ok((inner, reader))
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The cached text of `path`: `Some(Some(text))`, `Some(None)` when the
    /// document is known to have no text, or `None` when nothing usable is
    /// stored (never seen, changed since, or the record is damaged).
    pub fn get(&self, path: &str, size: u64, mtime: i64) -> Option<Option<String>> {
        let reader = self.reader.read().unwrap_or_else(|e| e.into_inner());
        let m = *self.lock().map.get(path)?;
        if m.size != size || m.mtime != mtime {
            return None;
        }
        if m.kind == Kind::NoText {
            return Some(None);
        }
        let mut buf = vec![0u8; m.len as usize];
        read_at(reader.as_ref()?, &mut buf, m.offset).ok()?;
        drop(reader);
        if fnv(&buf) != m.hash {
            return None;
        }
        let raw = lz4_flex::block::decompress(&buf, m.raw_len as usize).ok()?;
        if raw.len() != m.raw_len as usize {
            return None;
        }
        String::from_utf8(raw).ok().map(Some)
    }

    /// True when `path` has a stored result for this size and mtime.
    pub fn is_fresh(&self, path: &str, size: u64, mtime: i64) -> bool {
        self.lock().map.get(path).is_some_and(|m| m.size == size && m.mtime == mtime)
    }

    /// Stores the extraction result of `path` (`None`: no text or failed).
    /// Texts over [`MAX_TEXT`] are skipped, and once the store reaches its
    /// size cap no new texts are added.
    pub fn put(&self, path: &str, size: u64, mtime: i64, text: Option<&str>) {
        let rec = match text {
            Some(t) if t.len() > MAX_TEXT => return,
            Some(t) => Some((lz4_flex::block::compress(t.as_bytes()), t.len() as u32)),
            None => None,
        };
        let mut inner = self.lock();
        let meta = match rec {
            None => Meta { size, mtime, offset: 0, len: 0, raw_len: 0, hash: 0, kind: Kind::NoText },
            Some((data, raw_len)) => {
                if inner.full || inner.dat_len + data.len() as u64 > self.max_bytes {
                    inner.full = true;
                    return;
                }
                let offset = inner.dat_len;
                let Some(w) = inner.writer.as_mut() else { return };
                if w.seek(SeekFrom::Start(offset)).and_then(|_| w.write_all(&data)).is_err() {
                    return;
                }
                inner.dat_len += data.len() as u64;
                Meta { size, mtime, offset, len: data.len() as u32, raw_len, hash: fnv(&data), kind: Kind::Text }
            }
        };
        if let Some(old) = inner.map.insert(path.into(), meta)
            && old.kind == Kind::Text
        {
            inner.garbage += u64::from(old.len);
        }
        inner.dirty = true;
    }

    /// Writes `content.idx` (via a temp file and a rename) if anything changed.
    pub fn flush(&self) -> io::Result<()> {
        let mut inner = self.lock();
        if !inner.dirty {
            return Ok(());
        }
        write_idx(&self.dir, &mut inner)
    }

    /// Rewrites `content.dat` without unused records when they are more than
    /// half of it. Returns whether it did.
    pub fn compact(&self) -> io::Result<bool> {
        let mut reader = self.reader.write().unwrap_or_else(|e| e.into_inner());
        let mut inner = self.lock();
        let live = inner.dat_len - HEADER_LEN - inner.garbage;
        if inner.garbage == 0 || inner.garbage <= live {
            return Ok(false);
        }
        let Some(src) = reader.as_ref() else { return Ok(false) };
        let tmp = self.dir.join("content.dat.tmp");
        let id = new_id();
        // New offsets are applied only once the new file is in place.
        let mut moved: Vec<(Box<str>, u64)> = Vec::new();
        let mut pos = HEADER_LEN;
        {
            let mut out = BufWriter::new(File::create(&tmp)?);
            out.write_all(&header(id))?;
            let mut texts: Vec<(&Box<str>, &Meta)> = inner.map.iter().filter(|(_, m)| m.kind == Kind::Text).collect();
            texts.sort_unstable_by_key(|(_, m)| m.offset);
            let mut buf = Vec::new();
            for (path, m) in texts {
                buf.resize(m.len as usize, 0);
                read_at(src, &mut buf, m.offset)?;
                out.write_all(&buf)?;
                moved.push((path.clone(), pos));
                pos += u64::from(m.len);
            }
            out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        }
        // Windows can't replace a file that is open here: close it first.
        *reader = None;
        inner.writer = None;
        let renamed = fs::rename(&tmp, self.dir.join(DAT));
        *reader = File::open(self.dir.join(DAT)).ok();
        inner.writer = OpenOptions::new().read(true).write(true).open(self.dir.join(DAT)).ok();
        renamed?;
        for (path, offset) in moved {
            if let Some(m) = inner.map.get_mut(&path) {
                m.offset = offset;
            }
        }
        inner.id = id;
        inner.dat_len = pos;
        inner.garbage = 0;
        inner.full = false;
        inner.dirty = true;
        write_idx(&self.dir, &mut inner)?;
        Ok(true)
    }

    pub fn stats(&self) -> Stats {
        let inner = self.lock();
        let texts = inner.map.values().filter(|m| m.kind == Kind::Text).count();
        Stats {
            documents: inner.map.len(),
            texts,
            no_text: inner.map.len() - texts,
            bytes: inner.dat_len,
            garbage: inner.garbage,
            full: inner.full,
        }
    }

    pub fn is_full(&self) -> bool {
        self.lock().full
    }

    /// Call while a content search runs; the warm-up waits until it ends.
    pub fn begin_search(&self) -> SearchGuard<'_> {
        self.searching.fetch_add(1, Ordering::Relaxed);
        SearchGuard(self)
    }

    pub fn searching(&self) -> bool {
        self.searching.load(Ordering::Relaxed) > 0
    }
}

fn write_idx(dir: &Path, inner: &mut Inner) -> io::Result<()> {
    let idx = IdxRef { version: VERSION, id: inner.id, dat_len: inner.dat_len, entries: &inner.map };
    let mut bytes = bincode::serialize(&idx).map_err(io::Error::other)?;
    bytes.extend_from_slice(&fnv(&bytes).to_le_bytes());
    let tmp = dir.join("content.idx.tmp");
    fs::write(&tmp, &bytes)?;
    fs::rename(&tmp, dir.join(IDX))?;
    inner.dirty = false;
    Ok(())
}

/// Counters of a running [`warm`].
#[derive(Default)]
pub struct WarmProgress {
    /// Documents that were not cached yet when the warm-up started.
    pub total: AtomicUsize,
    pub checked: AtomicUsize,
    /// Documents this warm-up extracted and stored.
    pub added: AtomicUsize,
}

/// Fills `cache` with the text of every document in `entries` that is not
/// cached yet, most recently modified first. Meant for one low-priority
/// background thread: it pauses while a content search runs, saves every
/// [`FLUSH_EVERY`] documents, and stops at `cancel` or when the store is full.
pub fn warm(entries: &[Entry], cache: &ContentCache, progress: &WarmProgress, cancel: &AtomicBool) {
    let mut todo: Vec<&Entry> =
        entries.iter().filter(|e| is_document(e) && !cache.is_fresh(&e.path, e.size, e.mtime)).collect();
    todo.sort_by_key(|e| Reverse(e.mtime));
    progress.total.store(todo.len(), Ordering::Relaxed);
    let mut unsaved = 0;
    for e in todo {
        while cache.searching() && !cancel.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
        }
        if cancel.load(Ordering::Relaxed) || cache.is_full() {
            break;
        }
        // A search may have stored it meanwhile.
        if !cache.is_fresh(&e.path, e.size, e.mtime) {
            let text = extract::extract_text(Path::new(&*e.path));
            cache.put(&e.path, e.size, e.mtime, text.as_deref());
            progress.added.fetch_add(1, Ordering::Relaxed);
            unsaved += 1;
            if unsaved >= FLUSH_EVERY {
                let _ = cache.flush();
                unsaved = 0;
            }
        }
        progress.checked.fetch_add(1, Ordering::Relaxed);
    }
    let _ = cache.flush();
    let _ = cache.compact();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bahth_cc_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    const AR: &str = "تقرير المبيعات السنوي للعام ٢٠٢٤\nإجمالي الإيرادات: ١٢٣٤٥ ريال";

    #[test]
    fn roundtrip_stale_and_no_text() {
        let dir = temp_dir("rt");
        let c = ContentCache::open(&dir).unwrap();
        assert_eq!(c.get("C:\\a.docx", 10, 5), None);
        c.put("C:\\a.docx", 10, 5, Some(AR));
        assert_eq!(c.get("C:\\a.docx", 10, 5), Some(Some(AR.to_string())));
        // Changed size or mtime: not cached.
        assert_eq!(c.get("C:\\a.docx", 11, 5), None);
        assert_eq!(c.get("C:\\a.docx", 10, 6), None);
        assert!(c.is_fresh("C:\\a.docx", 10, 5) && !c.is_fresh("C:\\a.docx", 10, 6));
        // A failed extraction is remembered as "no text".
        c.put("C:\\تالف.pdf", 99, 1, None);
        assert_eq!(c.get("C:\\تالف.pdf", 99, 1), Some(None));
        assert_eq!(c.get("C:\\تالف.pdf", 98, 1), None);
        // Replacing an entry leaves its old record as garbage.
        c.put("C:\\a.docx", 12, 7, Some("نص جديد"));
        assert_eq!(c.get("C:\\a.docx", 12, 7).unwrap().unwrap(), "نص جديد");
        assert_eq!(c.get("C:\\a.docx", 10, 5), None);
        let s = c.stats();
        assert_eq!((s.documents, s.texts, s.no_text, s.full), (2, 1, 1, false));
        assert!(s.garbage > 0 && s.bytes > HEADER_LEN + s.garbage);
        // Too long a text is not stored at all.
        c.put("C:\\big.txt", 1, 1, Some(&"x".repeat(MAX_TEXT + 1)));
        assert_eq!(c.get("C:\\big.txt", 1, 1), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reopen_from_disk() {
        let dir = temp_dir("reopen");
        {
            let c = ContentCache::open(&dir).unwrap();
            c.put("a", 1, 2, Some(AR));
            c.put("b", 3, 4, None);
            c.flush().unwrap();
            // Written but never flushed: lost on reopen, harmlessly.
            c.put("c", 5, 6, Some("ضائع"));
        }
        let c = ContentCache::open(&dir).unwrap();
        assert_eq!(c.get("a", 1, 2), Some(Some(AR.to_string())));
        assert_eq!(c.get("b", 3, 4), Some(None));
        assert_eq!(c.get("c", 5, 6), None);
        assert_eq!(c.stats().garbage, 0);
        // New records go after the last flushed one.
        c.put("d", 7, 8, Some("بعد"));
        c.flush().unwrap();
        drop(c);
        let c = ContentCache::open(&dir).unwrap();
        assert_eq!(c.get("d", 7, 8).unwrap().unwrap(), "بعد");
        assert_eq!(c.get("a", 1, 2).unwrap().unwrap(), AR);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn corrupt_store_is_rebuilt() {
        let dir = temp_dir("corrupt");
        let fill = || {
            let c = ContentCache::open(&dir).unwrap();
            c.put("a", 1, 2, Some(AR));
            c.flush().unwrap();
        };
        fill();
        // A damaged idx: start empty, and the store works again.
        let idx = dir.join(IDX);
        let mut bytes = fs::read(&idx).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xFF;
        fs::write(&idx, &bytes).unwrap();
        let c = ContentCache::open(&dir).unwrap();
        assert_eq!(c.get("a", 1, 2), None);
        assert_eq!(c.stats().documents, 0);
        c.put("a", 1, 2, Some(AR));
        assert_eq!(c.get("a", 1, 2).unwrap().unwrap(), AR);
        drop(c);
        // Garbage idx, truncated idx, empty idx.
        for junk in [&b"not an index at all"[..], &bytes[..10], &[]] {
            fill();
            fs::write(&idx, junk).unwrap();
            assert_eq!(ContentCache::open(&dir).unwrap().stats().documents, 0);
        }
        // A dat from another store (id mismatch) or cut short.
        fill();
        let dat = dir.join(DAT);
        let mut d = fs::read(&dat).unwrap();
        d[15] ^= 1;
        fs::write(&dat, &d).unwrap();
        assert_eq!(ContentCache::open(&dir).unwrap().stats().documents, 0);
        fill();
        let len = fs::metadata(&dat).unwrap().len();
        OpenOptions::new().write(true).open(&dat).unwrap().set_len(len - 3).unwrap();
        assert_eq!(ContentCache::open(&dir).unwrap().stats().documents, 0);
        // A damaged record is reported as not cached, not as wrong text.
        fill();
        let mut d = fs::read(&dat).unwrap();
        let last = d.len() - 2;
        d[last] ^= 0x55;
        fs::write(&dat, &d).unwrap();
        let c = ContentCache::open(&dir).unwrap();
        assert_eq!(c.stats().documents, 1);
        assert_eq!(c.get("a", 1, 2), None);
        drop(c);
        // A missing dat, or no folder contents at all.
        fs::remove_file(&dat).unwrap();
        assert_eq!(ContentCache::open(&dir).unwrap().stats().documents, 0);
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(ContentCache::open(&dir).unwrap().stats().documents, 0);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn compaction() {
        let dir = temp_dir("compact");
        let c = ContentCache::open(&dir).unwrap();
        let text = |i: u32| format!("{AR} — نسخة رقم {i}");
        for i in 0..20 {
            c.put(&format!("f{i}"), 1, 1, Some(&text(i)));
        }
        // Not worth it while little is garbage.
        c.put("f0", 1, 2, Some(&text(0)));
        assert!(!c.compact().unwrap());
        // Rewrite most entries so garbage passes half the file.
        for i in 0..20 {
            c.put(&format!("f{i}"), 2, 2, Some(&text(i + 100)));
        }
        c.put("gone", 1, 1, None);
        let before = c.stats();
        assert!(before.garbage * 2 > before.bytes);
        assert!(c.compact().unwrap());
        let after = c.stats();
        assert_eq!((after.garbage, after.documents), (0, before.documents));
        assert_eq!(after.bytes, before.bytes - before.garbage);
        assert_eq!(fs::metadata(dir.join(DAT)).unwrap().len(), after.bytes);
        for i in 0..20 {
            assert_eq!(c.get(&format!("f{i}"), 2, 2).unwrap().unwrap(), text(i + 100));
        }
        assert_eq!(c.get("gone", 1, 1), Some(None));
        // Still writable, and the compacted store reopens.
        c.put("new", 3, 3, Some("جديد"));
        c.flush().unwrap();
        drop(c);
        let c = ContentCache::open(&dir).unwrap();
        assert_eq!(c.get("f7", 2, 2).unwrap().unwrap(), text(107));
        assert_eq!(c.get("new", 3, 3).unwrap().unwrap(), "جديد");
        assert!(!dir.join("content.dat.tmp").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn size_cap() {
        let dir = temp_dir("cap");
        let c = ContentCache::open_with_limit(&dir, 2000).unwrap();
        let mut stored = 0;
        for i in 0..100 {
            c.put(&format!("f{i}"), 1, 1, Some(&format!("{AR} {i}")));
            stored += c.is_fresh(&format!("f{i}"), 1, 1) as usize;
        }
        let s = c.stats();
        assert!(s.full && s.bytes <= 2000 && stored > 0 && stored < 100, "{s:?}");
        assert_eq!(s.texts, stored);
        // "No text" records still fit.
        c.put("empty", 1, 1, None);
        assert_eq!(c.get("empty", 1, 1), Some(None));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn concurrent_reads_and_writes() {
        let dir = temp_dir("threads");
        let c = Arc::new(ContentCache::open(&dir).unwrap());
        for i in 0..50 {
            c.put(&format!("f{i}"), i, 1, Some(&format!("{AR} {i}")));
        }
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let c = c.clone();
                std::thread::spawn(move || {
                    for round in 0..200u64 {
                        let i = (round * 7 + t) % 50;
                        assert_eq!(c.get(&format!("f{i}"), i, 1).unwrap().unwrap(), format!("{AR} {i}"));
                        if round % 20 == 0 {
                            c.put(&format!("t{t}-{round}"), 1, 1, Some("كتابة متزامنة"));
                        }
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(c.stats().documents, 50 + 8 * 10);
        fs::remove_dir_all(&dir).unwrap();
    }

    fn file_entry(path: &Path, mtime: i64) -> Entry {
        let size = fs::metadata(path).unwrap().len();
        Entry { path: path.to_str().unwrap().into(), size, mtime, is_dir: false, norm_name: "".into() }
    }

    #[test]
    fn warm_fills_newest_first_and_stops() {
        let dir = temp_dir("warm");
        let docs = dir.join("docs");
        fs::create_dir_all(&docs).unwrap();
        let mut entries = Vec::new();
        for i in 0..5 {
            let p = docs.join(format!("ملف{i}.txt"));
            fs::write(&p, format!("المبيعات {i}")).unwrap();
            entries.push(file_entry(&p, 100 + i));
        }
        let png = docs.join("صورة.png");
        fs::write(&png, "x").unwrap();
        entries.push(file_entry(&png, 1));
        let empty = docs.join("فارغ.txt");
        fs::write(&empty, "   ").unwrap();
        entries.push(file_entry(&empty, 1));
        let cache = ContentCache::open(&dir.join("store")).unwrap();

        // Cancelled at once: nothing stored.
        let p = WarmProgress::default();
        warm(&entries, &cache, &p, &AtomicBool::new(true));
        assert_eq!((p.total.load(Ordering::Relaxed), p.added.load(Ordering::Relaxed)), (6, 0));

        let p = WarmProgress::default();
        warm(&entries, &cache, &p, &AtomicBool::new(false));
        assert_eq!((p.total.load(Ordering::Relaxed), p.added.load(Ordering::Relaxed)), (6, 6));
        let e = &entries[3];
        assert_eq!(cache.get(&e.path, e.size, e.mtime).unwrap().unwrap(), "المبيعات 3");
        let e = &entries[6];
        assert_eq!(cache.get(&e.path, e.size, e.mtime), Some(None));
        assert!(!cache.is_fresh(&entries[5].path, entries[5].size, entries[5].mtime));
        // Saved on disk, and a second warm-up has nothing to do.
        assert_eq!(ContentCache::open(&dir.join("store")).unwrap().stats().documents, 6);
        let p = WarmProgress::default();
        warm(&entries, &cache, &p, &AtomicBool::new(false));
        assert_eq!(p.total.load(Ordering::Relaxed), 0);

        // It waits while a search runs.
        entries[0].mtime = 999;
        let cache = Arc::new(cache);
        let guard_cache = cache.clone();
        let p = Arc::new(WarmProgress::default());
        let (c2, p2) = (cache.clone(), p.clone());
        let entries2 = entries.clone();
        let release = Arc::new(AtomicBool::new(false));
        let r2 = release.clone();
        let searcher = std::thread::spawn(move || {
            let _g = guard_cache.begin_search();
            while !r2.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        while !cache.searching() {
            std::thread::sleep(Duration::from_millis(5));
        }
        let warmer = std::thread::spawn(move || warm(&entries2, &c2, &p2, &AtomicBool::new(false)));
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(p.added.load(Ordering::Relaxed), 0);
        release.store(true, Ordering::Relaxed);
        searcher.join().unwrap();
        warmer.join().unwrap();
        assert_eq!(p.added.load(Ordering::Relaxed), 1);
        fs::remove_dir_all(&dir).unwrap();
    }
}
