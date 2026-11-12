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
//! An entry is used only while the file's size and modification time on
//! disk still match. Files with no text (or whose extraction failed) are
//! remembered too, so they are not re-parsed on every search, but only when
//! the failure doesn't look passing (see `worth_storing`). A missing, corrupt or
//! mismatched store is thrown away and started afresh; nothing here panics.
//!
//! Reads take a shared lock only, so the parallel content search reads many
//! documents at once; writes go through one mutex.
//!
//! **Crash-loop guard.** A parser bug that ends the process (a stack overflow
//! or a failed allocation, which no `catch_unwind` stops) would otherwise
//! hit the same file again at every start, since the warm-up begins with the
//! newest documents. So while a file is being extracted, its path, size and
//! mtime sit in a small `inflight-N.bin` file of this folder (one per
//! extraction running at once, rewritten in place, no fsync: a dying process
//! still leaves its writes to the OS). Opening the store finds a marker left
//! behind, remembers that file as "no text" until it changes, and reports it
//! (`recovered`). A normal exit clears the markers first
//! (`clear_markers_at_exit`); a kill from outside during an extraction
//! (Task Manager, the updater, a logoff) also leaves one, and that one file
//! is then skipped until it changes.

// Only the Windows UI and the tests use this module.

use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    /// Markers of the extractions running now (see the module docs).
    inflight: InFlight,
    /// Files a previous run died extracting, now stored as "no text".
    recovered: Vec<String>,
}

// ---------------------------------------------------------------------------
// Crash-loop guard: which files are being extracted right now
// ---------------------------------------------------------------------------

const INFLIGHT_PREFIX: &str = "inflight-";
const INFLIGHT_MAGIC: [u8; 4] = *b"BHIF";
/// Markers longer than this are not ours (paths are at most 32k UTF-16 units).
const MAX_MARKER: u64 = 128 * 1024;

/// Set when the program exits normally: no new markers from then on.
static EXITING: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
fn write_at(f: &File, buf: &[u8], offset: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(f, buf, offset)
}

#[cfg(windows)]
fn write_at(f: &File, mut buf: &[u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        match f.seek_write(buf, offset) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => {
                buf = &buf[n..];
                offset += n as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// A marker: magic, size, mtime, path length, path (UTF-8), then a checksum
/// of all that, so a write cut short is never read as a marker.
fn encode_marker(path: &str, size: u64, mtime: i64) -> Vec<u8> {
    let mut m = INFLIGHT_MAGIC.to_vec();
    m.extend(size.to_le_bytes());
    m.extend(mtime.to_le_bytes());
    m.extend((path.len() as u32).to_le_bytes());
    m.extend(path.as_bytes());
    m.extend(fnv(&m).to_le_bytes());
    m
}

fn decode_marker(bytes: &[u8]) -> Option<(String, u64, i64)> {
    let rest = bytes.strip_prefix(&INFLIGHT_MAGIC)?;
    let size = u64::from_le_bytes(rest.get(..8)?.try_into().ok()?);
    let mtime = i64::from_le_bytes(rest.get(8..16)?.try_into().ok()?);
    let len = u32::from_le_bytes(rest.get(16..20)?.try_into().ok()?) as usize;
    let path = std::str::from_utf8(rest.get(20..20usize.checked_add(len)?)?).ok()?;
    let end = 4 + 20 + len;
    (bytes.get(end..end + 8)? == fnv(&bytes[..end]).to_le_bytes()).then(|| (path.to_owned(), size, mtime))
}

/// The marker files of the store in `dir`.
fn marker_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(dir) else { return Vec::new() };
    rd.flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(INFLIGHT_PREFIX) && n.ends_with(".bin")))
        .collect()
}

/// The marker slots of one store: the open files not in use right now.
struct InFlight {
    dir: PathBuf,
    free: Mutex<Vec<File>>,
    /// Slot files created so far (the next one is `inflight-{created}.bin`).
    created: AtomicUsize,
}

/// One extraction's marker, cleared when dropped.
pub struct Marker<'a> {
    owner: &'a InFlight,
    file: Option<File>,
}

impl InFlight {
    fn new(dir: &Path) -> InFlight {
        InFlight { dir: dir.to_path_buf(), free: Mutex::new(Vec::new()), created: AtomicUsize::new(0) }
    }

    /// Records that `path` (as `size`, `mtime`) is being extracted, until the
    /// returned marker is dropped. `None` when it could not be written (the
    /// extraction goes ahead unguarded) or the program is exiting.
    fn begin(&self, path: &str, size: u64, mtime: i64) -> Option<Marker<'_>> {
        if EXITING.load(Ordering::Relaxed) {
            return None;
        }
        let reused = self.free.lock().unwrap_or_else(|e| e.into_inner()).pop();
        let file = match reused {
            Some(f) => f,
            None => {
                let n = self.created.fetch_add(1, Ordering::Relaxed);
                let p = self.dir.join(format!("{INFLIGHT_PREFIX}{n}.bin"));
                OpenOptions::new().write(true).create(true).truncate(true).open(p).ok()?
            }
        };
        let marker = Marker { owner: self, file: Some(file) };
        // Written in place: a longer marker before it may still follow, but
        // the checksum ends this one.
        write_at(marker.file.as_ref()?, &encode_marker(path, size, mtime), 0).ok()?;
        Some(marker)
    }
}

impl Drop for Marker<'_> {
    fn drop(&mut self) {
        if let Some(f) = self.file.take() {
            // A cleared magic is no marker. Not returned to the pool when
            // even that failed: a new slot is safer than a stale marker.
            if write_at(&f, &[0; 4], 0).is_ok() {
                self.owner.free.lock().unwrap_or_else(|e| e.into_inner()).push(f);
            }
        }
    }
}

/// The store whose markers [`clear_markers_now`] clears at exit.
static EXIT_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Clears the markers of the store registered with
/// [`ContentCache::clear_markers_at_exit`]: an extraction cut off by a
/// normal exit did not crash. Takes no lock, so it is safe however the
/// other threads stand at exit.
extern "C" fn clear_markers_now() {
    EXITING.store(true, Ordering::Relaxed);
    if let Some(dir) = EXIT_DIR.get() {
        clear_markers_in(dir);
    }
}

fn clear_markers_in(dir: &Path) {
    for p in marker_files(dir) {
        if let Ok(f) = OpenOptions::new().write(true).open(&p) {
            let _ = write_at(&f, &[0; 4], 0);
        }
    }
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

/// At most `cap` bytes of `r`.
fn read_capped_all(r: impl Read, cap: u64) -> io::Result<Vec<u8>> {
    let mut v = Vec::new();
    r.take(cap).read_to_end(&mut v)?;
    Ok(v)
}

/// The idx payload, or `None` when the checksum or the encoding is wrong.
fn decode_idx(bytes: &[u8]) -> Option<IdxFile> {
    let (payload, sum) = bytes.split_at_checked(bytes.len().checked_sub(8)?)?;
    if fnv(payload).to_le_bytes() != sum {
        return None;
    }
    bincode::deserialize(payload).ok()
}

/// A file's size and modification time as it is on disk now, in the units
/// of [`Entry`] (bytes, whole seconds since 1970). `None` when it is gone or
/// is not a file.
pub fn stamp(path: &Path) -> Option<(u64, i64)> {
    let m = fs::metadata(path).ok().filter(|m| m.is_file())?;
    let mtime = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
    Some((m.len(), mtime))
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// A file modified this recently may still be written to: an extraction
/// that found no text then is not remembered. (Seconds.)
const RECENT_SECS: i64 = 120;
/// Mtimes are whole seconds, so a text read within the second of the last
/// change could be followed by a same-size edit with the same mtime: such a
/// text is not stored. (Seconds.)
const SETTLE_SECS: i64 = 2;

/// Whether the result of extracting `path`, whose [`stamp`] was `before`
/// just ahead of the extraction, can be stored and trusted until the file
/// changes. Not when the file changed meanwhile or was changed too recently;
/// and "no text" only when the file can be opened now, so a document that
/// was locked, denied or being written is tried again next time.
fn worth_storing(path: &Path, before: (u64, i64), has_text: bool, now: i64) -> bool {
    if stamp(path) != Some(before) {
        return false;
    }
    let age = now - before.1;
    if age < SETTLE_SECS {
        return false;
    }
    has_text || (age >= RECENT_SECS && File::open(path).is_ok())
}

/// Extracts the text of `path` and stores it in `cache` when it can be
/// trusted (see `worth_storing`). True when it was stored.
fn extract_and_store(cache: &ContentCache, path: &str, before: (u64, i64)) -> (Option<String>, bool) {
    let p = Path::new(path);
    let marker = cache.inflight.begin(path, before.0, before.1);
    let text = extract::extract_text(p);
    drop(marker);
    let store = worth_storing(p, before, text.is_some(), now_secs());
    if store {
        cache.put(path, before.0, before.1, text.as_deref());
    }
    (text, store)
}

/// The text of `path` for a content search: from `cache` when it holds a
/// result for the file as it is on disk now (whatever the index says),
/// otherwise extracted, and stored when it can be trusted. The flag is true
/// when the text came from the cache.
pub fn text_of(cache: Option<&ContentCache>, path: &str) -> (Option<String>, bool) {
    let (Some(cache), Some(st)) = (cache, stamp(Path::new(path))) else {
        return (extract::extract_text(Path::new(path)), false);
    };
    if let Some(text) = cache.get(path, st.0, st.1) {
        return (text, true);
    }
    (extract_and_store(cache, path, st).0, false)
}

/// A text or a document whose text is worth caching: supported and not
/// beyond the extraction size limit.
pub fn is_document(e: &Entry) -> bool {
    !e.is_dir
        && e.size > 0
        && e.size <= extract::MAX_FILE_SIZE
        && extract::is_supported(&e.ext())
        && !in_folder(&e.path, own_data_dir())
}

/// The program's own data folder (`%LOCALAPPDATA%\BahthSari\`, with the
/// trailing separator): its history and logs hold what was just searched for,
/// so they would match every content search.
fn own_data_dir() -> &'static str {
    static DIR: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        #[cfg(windows)]
        {
            let dir = crate::index::cache_path().with_file_name("");
            dir.to_string_lossy().into_owned()
        }
        #[cfg(not(windows))]
        String::new()
    })
}

/// True when `path` is inside `dir` (which ends with a separator), ignoring
/// ASCII case like Windows does. An empty `dir` contains nothing.
fn in_folder(path: &str, dir: &str) -> bool {
    !dir.is_empty() && path.len() > dir.len() && path.as_bytes()[..dir.len()].eq_ignore_ascii_case(dir.as_bytes())
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
        let mut cache = ContentCache {
            dir: dir.to_path_buf(),
            max_bytes,
            inner: Mutex::new(inner),
            reader: RwLock::new(Some(reader)),
            searching: AtomicUsize::new(0),
            inflight: InFlight::new(dir),
            recovered: Vec::new(),
        };
        cache.recover();
        Ok(cache)
    }

    /// Turns the markers a previous run left behind into "no text" entries
    /// (for the file as it was then: once it changes it is read again), then
    /// removes them.
    fn recover(&mut self) {
        let files = marker_files(&self.dir);
        for p in &files {
            let bytes = File::open(p).and_then(|f| read_capped_all(f, MAX_MARKER)).unwrap_or_default();
            if let Some((path, size, mtime)) = decode_marker(&bytes) {
                if stamp(Path::new(&path)) == Some((size, mtime)) {
                    self.put(&path, size, mtime, None);
                }
                self.recovered.push(path);
            }
        }
        if !self.recovered.is_empty() && self.flush().is_err() {
            return; // keep the markers: the next start tries again
        }
        for p in files {
            let _ = fs::remove_file(p);
        }
    }

    /// Files a previous run died extracting (see the module docs): each is
    /// now stored as "no text" until it changes. For the log.
    pub fn recovered(&self) -> &[String] {
        &self.recovered
    }

    /// Clears this store's markers when the program exits normally (C
    /// `atexit`, which runs on `exit` and on return from `main`, not on a
    /// kill from outside). Call once, for the app's own store.
    #[cfg_attr(not(windows), allow(dead_code))] // only the Windows UI calls it
    pub fn clear_markers_at_exit(&self) {
        unsafe extern "C" {
            fn atexit(f: extern "C" fn()) -> std::ffi::c_int;
        }
        if EXIT_DIR.set(self.dir.clone()).is_ok() {
            // SAFETY: registers a plain function with no arguments.
            unsafe {
                atexit(clear_markers_now);
            }
        }
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
            // Checked: a damaged offset near u64::MAX must not overflow.
            if m.offset < HEADER_LEN || m.offset.checked_add(u64::from(m.len))? > idx.dat_len {
                return None;
            }
            live = live.checked_add(u64::from(m.len))?;
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
        // `put` never stores more; a damaged length must not allocate gigabytes.
        if m.raw_len as usize > MAX_TEXT {
            return None;
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

    /// Documents with a cached result; cheaper than [`stats`](Self::stats).
    pub fn documents(&self) -> usize {
        self.lock().map.len()
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

/// How many threads [`warm`] should use on this machine: half the logical
/// processors, between 1 and 4, so the PC stays responsive (in the app the
/// threads also run at background priority).
pub fn warm_threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get() / 2).clamp(1, 4)
}

/// Files over this size are extracted by one warm-up thread at a time, so
/// several threads never hold several huge texts at once.
const BIG_FILE: u64 = 8 * 1024 * 1024;

/// Compressed documents (archives, zip-based office files) over this size
/// count as big too: their text can be many times their size on disk.
const BIG_COMPRESSED: u64 = 2 * 1024 * 1024;

/// Zip-based office formats, whose text is compressed inside.
const ZIPPED_DOCS: &[&str] = &["docx", "docm", "pptx", "odt", "odp", "ods", "xlsx", "xlsm", "xlsb", "epub"];

/// Whether extracting `e` may need a lot of memory, so the warm-up does it
/// on one thread at a time.
fn is_heavy(e: &Entry) -> bool {
    if e.size > BIG_FILE {
        return true;
    }
    let ext = e.ext();
    e.size > BIG_COMPRESSED && (extract::is_archive(&ext) || ZIPPED_DOCS.contains(&ext.as_str()))
}

/// Images, read only through OCR (Windows): by far the slowest documents.
const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "bmp", "tif", "tiff", "gif"];

/// Where `e` goes in the warm-up order (lower first), so the documents a
/// first content search most likely wants are cached first: the user's own
/// folders, then everything else, then program and system folders, then big
/// files and archives, then images (OCR).
fn warm_rank(e: &Entry) -> u8 {
    let ext = e.ext();
    if IMAGE_EXTS.contains(&ext.as_str()) {
        return 4;
    }
    if e.size > BIG_FILE || extract::is_archive(&ext) {
        return 3;
    }
    let p = e.path.to_lowercase().replace('/', "\\");
    let has = |parts: &[&str]| parts.iter().any(|x| p.contains(x));
    if has(&[
        "\\appdata\\",
        "\\program files",
        "\\programdata\\",
        "\\windows\\",
        "\\$recycle.bin\\",
        "\\node_modules\\",
        "\\.git\\",
    ]) {
        2
    } else if has(&["\\desktop\\", "\\documents\\", "\\downloads\\", "\\onedrive"]) {
        0
    } else {
        1
    }
}

/// When the warm-up saves the index. Each save rewrites the whole map (about
/// 10 MB for 100k documents), so saving after every few hundred documents
/// wrote gigabytes on a big first warm-up. Now: at most every
/// [`FLUSH_MIN`], and only once the documents added since the last save are
/// an eighth of the store (so the total written stays a small multiple of
/// the final index), or [`FLUSH_MAX`] has passed: a crash or a logoff loses
/// at most a few minutes of work.
const FLUSH_MIN: Duration = Duration::from_secs(20);
const FLUSH_MAX: Duration = Duration::from_secs(300);

fn time_to_save(since: Duration, added_since: usize, documents: usize) -> bool {
    since >= FLUSH_MAX || (since >= FLUSH_MIN && added_since > 0 && added_since >= documents / 8)
}

/// Fills `cache` with the text of every document in `entries` that is not
/// cached yet, the likely most useful first (see `warm_rank`; within a rank,
/// most recently modified first). Runs on the calling thread plus
/// `threads - 1` more, each of which calls `thread_init` first (to lower its
/// priority). It pauses while a content search runs, saves now and then
/// (see `time_to_save`), and stops at `cancel` or when the store is full; all
/// its threads have ended when it returns.
pub fn warm(
    entries: &[Entry],
    cache: &ContentCache,
    progress: &WarmProgress,
    cancel: &AtomicBool,
    threads: usize,
    thread_init: &(dyn Fn() + Sync),
) {
    let mut todo: Vec<&Entry> =
        entries.iter().filter(|e| is_document(e) && !cache.is_fresh(&e.path, e.size, e.mtime)).collect();
    todo.sort_by_cached_key(|e| (warm_rank(e), Reverse(e.mtime)));
    progress.total.store(todo.len(), Ordering::Relaxed);
    let next = AtomicUsize::new(0);
    // When the warm-up last saved, and how many documents it had added.
    let last_save = Mutex::new((Instant::now(), 0));
    let big = Mutex::new(());
    let work = || {
        loop {
            while cache.searching() && !cancel.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(100));
            }
            if cancel.load(Ordering::Relaxed) || cache.is_full() {
                break;
            }
            let Some(e) = todo.get(next.fetch_add(1, Ordering::Relaxed)) else { break };
            let one_big = is_heavy(e).then(|| big.lock().unwrap_or_else(|e| e.into_inner()));
            // The file as it is now (the index may lag behind); a search may
            // also have stored it meanwhile.
            if let Some(st) = stamp(Path::new(&*e.path))
                && !cache.is_fresh(&e.path, st.0, st.1)
                && extract_and_store(cache, &e.path, st).1
            {
                progress.added.fetch_add(1, Ordering::Relaxed);
            }
            drop(one_big);
            progress.checked.fetch_add(1, Ordering::Relaxed);
            // One thread saves (if anything changed); the others carry on.
            if let Ok(mut last) = last_save.try_lock() {
                let added = progress.added.load(Ordering::Relaxed);
                // Only then counts the documents, which takes the store's lock.
                let since = last.0.elapsed();
                if since >= FLUSH_MIN && time_to_save(since, added.saturating_sub(last.1), cache.documents()) {
                    let _ = cache.flush();
                    *last = (Instant::now(), added);
                }
            }
        }
    };
    std::thread::scope(|s| {
        for i in 1..threads.max(1) {
            // A thread that can't be started: the others do its share.
            let _ = std::thread::Builder::new().name(format!("content-warm-{i}")).spawn_scoped(s, || {
                thread_init();
                work();
            });
        }
        work();
    });
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
    fn own_data_folder_is_not_searched() {
        let dir = "C:\\Users\\a\\AppData\\Local\\BahthSari\\";
        assert!(in_folder("c:\\users\\a\\appdata\\local\\bahthsari\\history.json", dir));
        assert!(!in_folder("C:\\Users\\a\\AppData\\Local\\BahthSariX\\x.txt", dir));
        assert!(!in_folder("C:\\Users\\a\\Documents\\history.json", dir));
        assert!(!in_folder("C:\\anything.txt", ""));
    }

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

    /// Searches read while the warm-up rewrites texts, flushes and compacts
    /// (the situation of a content search started while the warm-up runs):
    /// a read gives the right text or nothing, never a panic or another
    /// document's text.
    #[test]
    fn reads_during_compaction_and_flush() {
        let dir = temp_dir("compact_race");
        let c = Arc::new(ContentCache::open(&dir).unwrap());
        let text = |i: u64, v: u64| format!("{AR} {i} نسخة {v} {}", "ا".repeat((i * 37 % 300) as usize));
        for i in 0..40 {
            c.put(&format!("f{i}"), i, 0, Some(&text(i, 0)));
        }
        let stop = Arc::new(AtomicBool::new(false));
        let writer = {
            let (c, stop) = (c.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut v: u64 = 1;
                while !stop.load(Ordering::Relaxed) {
                    // Each new version leaves the old record as garbage.
                    for i in 0..40 {
                        c.put(&format!("f{i}"), i, v as i64, Some(&text(i, v)));
                    }
                    let _ = c.flush();
                    let _ = c.compact();
                    v += 1;
                }
                v
            })
        };
        let readers: Vec<_> = (0..6u64)
            .map(|t| {
                let c = c.clone();
                std::thread::spawn(move || {
                    let _searching = c.begin_search();
                    let mut seen = 0;
                    for round in 0..3000u64 {
                        let i = (round * 13 + t) % 40;
                        for v in [round % 8, 0] {
                            if let Some(got) = c.get(&format!("f{i}"), i, v as i64) {
                                assert_eq!(got.as_deref(), Some(text(i, v).as_str()));
                                seen += 1;
                            }
                        }
                    }
                    seen
                })
            })
            .collect();
        let seen: usize = readers.into_iter().map(|r| r.join().unwrap()).sum();
        stop.store(true, Ordering::Relaxed);
        let versions = writer.join().unwrap();
        assert!(versions > 1 && seen > 0, "{versions} {seen}");
        assert!(!c.searching());
        // What is on disk opens again and agrees.
        c.flush().unwrap();
        let reopened = ContentCache::open(&dir).unwrap();
        assert_eq!(reopened.stats().documents, 40);
        drop(c);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// The entry of `path`, whose mtime on disk is set to `mtime` first.
    fn file_entry(path: &Path, mtime: i64) -> Entry {
        let f = File::options().write(true).open(path).unwrap();
        f.set_modified(UNIX_EPOCH + Duration::from_secs(mtime as u64)).unwrap();
        drop(f);
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
        warm(&entries, &cache, &p, &AtomicBool::new(true), 3, &|| {});
        assert_eq!((p.total.load(Ordering::Relaxed), p.added.load(Ordering::Relaxed)), (6, 0));

        let p = WarmProgress::default();
        warm(&entries, &cache, &p, &AtomicBool::new(false), 3, &|| {});
        assert_eq!((p.total.load(Ordering::Relaxed), p.added.load(Ordering::Relaxed)), (6, 6));
        let e = &entries[3];
        assert_eq!(cache.get(&e.path, e.size, e.mtime).unwrap().unwrap(), "المبيعات 3");
        let e = &entries[6];
        assert_eq!(cache.get(&e.path, e.size, e.mtime), Some(None));
        assert!(!cache.is_fresh(&entries[5].path, entries[5].size, entries[5].mtime));
        // Saved on disk, and a second warm-up has nothing to do.
        assert_eq!(ContentCache::open(&dir.join("store")).unwrap().stats().documents, 6);
        let p = WarmProgress::default();
        warm(&entries, &cache, &p, &AtomicBool::new(false), 3, &|| {});
        assert_eq!(p.total.load(Ordering::Relaxed), 0);

        // It waits while a search runs.
        entries[0] = file_entry(Path::new(&*entries[0].path), 999);
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
        let warmer = std::thread::spawn(move || warm(&entries2, &c2, &p2, &AtomicBool::new(false), 3, &|| {}));
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(p.added.load(Ordering::Relaxed), 0);
        release.store(true, Ordering::Relaxed);
        searcher.join().unwrap();
        warmer.join().unwrap();
        assert_eq!(p.added.load(Ordering::Relaxed), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn warm_order_puts_likely_documents_first() {
        let e = |path: &str, size: u64| Entry { path: path.into(), size, mtime: 0, is_dir: false, norm_name: "".into() };
        let rank = |path: &str| warm_rank(&e(path, 1000));
        assert_eq!(rank("C:\\Users\\a\\Documents\\تقرير.docx"), 0);
        assert_eq!(rank("C:\\Users\\a\\DESKTOP\\x.pdf"), 0);
        assert_eq!(rank("C:\\Users\\a\\OneDrive - Org\\x.xlsx"), 0);
        assert_eq!(rank("D:\\عمل\\x.docx"), 1);
        assert_eq!(rank("C:\\Users\\a\\AppData\\Local\\x\\Documents\\log.txt"), 2);
        assert_eq!(rank("C:\\Program Files (x86)\\x\\readme.txt"), 2);
        assert_eq!(rank("/home/a/Documents/x.txt"), 0);
        assert_eq!(rank("C:\\Users\\a\\Documents\\x.zip"), 3);
        assert_eq!(warm_rank(&e("C:\\Users\\a\\Documents\\big.pdf", BIG_FILE + 1)), 3);
        assert_eq!(rank("C:\\Users\\a\\Documents\\صورة.JPG"), 4);
        assert!((1..=4).contains(&warm_threads()));
    }

    #[test]
    fn compressed_documents_are_heavy_sooner() {
        let e = |path: &str, size: u64| Entry { path: path.into(), size, mtime: 0, is_dir: false, norm_name: "".into() };
        let mb = 1024 * 1024;
        assert!(is_heavy(&e("C:\\a.pdf", BIG_FILE + 1)) && !is_heavy(&e("C:\\a.pdf", 3 * mb)));
        assert!(is_heavy(&e("C:\\a.DOCX", 3 * mb)) && is_heavy(&e("C:\\a.zip", 3 * mb)) && is_heavy(&e("C:\\a.7z", 3 * mb)));
        assert!(!is_heavy(&e("C:\\a.docx", mb)) && !is_heavy(&e("C:\\a.zip", mb)));
    }

    #[test]
    fn saves_are_spaced_out_on_big_stores() {
        let s = Duration::from_secs;
        // Not before FLUSH_MIN, and not with nothing new.
        assert!(!time_to_save(s(5), 1000, 10));
        assert!(!time_to_save(s(30), 0, 0));
        assert!(time_to_save(s(30), 1, 0));
        // A big store waits for an eighth more documents, or FLUSH_MAX.
        assert!(!time_to_save(s(60), 1000, 100_000));
        assert!(time_to_save(s(60), 12_500, 100_000));
        assert!(time_to_save(FLUSH_MAX, 1, 100_000));
    }

    /// Several warm-up threads, stopped part way: what they stored is right,
    /// and they have all ended when `warm` returns.
    #[test]
    fn parallel_warm_stops_cleanly() {
        let dir = temp_dir("warm_par");
        let docs = dir.join("docs");
        fs::create_dir_all(&docs).unwrap();
        let entries: Vec<Entry> = (0..400)
            .map(|i| {
                let p = docs.join(format!("ملف{i}.txt"));
                fs::write(&p, format!("نص رقم {i}")).unwrap();
                file_entry(&p, 1000 + i)
            })
            .collect();
        let cache = Arc::new(ContentCache::open(&dir.join("store")).unwrap());
        let cancel = Arc::new(AtomicBool::new(false));
        let p = Arc::new(WarmProgress::default());
        let started = Arc::new(AtomicUsize::new(0));
        let warmer = {
            let (cache, cancel, p, started, entries) = (cache.clone(), cancel.clone(), p.clone(), started.clone(), entries.clone());
            std::thread::spawn(move || {
                warm(&entries, &cache, &p, &cancel, 4, &|| {
                    started.fetch_add(1, Ordering::Relaxed);
                })
            })
        };
        while p.added.load(Ordering::Relaxed) < 20 && !warmer.is_finished() {
            std::thread::yield_now();
        }
        cancel.store(true, Ordering::Relaxed);
        warmer.join().unwrap();
        let added = p.added.load(Ordering::Relaxed);
        assert_eq!(started.load(Ordering::Relaxed), 3);
        // Nothing more is added once it has returned.
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(cache.stats().documents, added);
        assert_eq!(p.checked.load(Ordering::Relaxed), added);
        for (i, e) in entries.iter().enumerate() {
            if let Some(t) = cache.get(&e.path, e.size, e.mtime) {
                assert_eq!(t.unwrap(), format!("نص رقم {i}"));
            }
        }
        // Saved on return; a second run finishes the rest.
        assert_eq!(ContentCache::open(&dir.join("store")).unwrap().stats().documents, added);
        cancel.store(false, Ordering::Relaxed);
        let p = WarmProgress::default();
        warm(&entries, &cache, &p, &cancel, 4, &|| {});
        assert_eq!(p.total.load(Ordering::Relaxed), 400 - added);
        assert_eq!(cache.stats().documents, 400);
        drop(cache);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn passing_failures_are_not_remembered() {
        let dir = temp_dir("worth");
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("مستند.txt");
        fs::write(&p, "نص").unwrap();
        let st = file_entry(&p, 10_000);
        let st = (st.size, st.mtime);
        // Settled and unchanged: text and "no text" both kept.
        assert!(worth_storing(&p, st, true, 20_000));
        assert!(worth_storing(&p, st, false, 20_000));
        // Changed just now: a text only after the mtime second has passed,
        // "no text" only once the file has been left alone a while.
        assert!(!worth_storing(&p, st, true, 10_001));
        assert!(worth_storing(&p, st, true, 10_002));
        assert!(!worth_storing(&p, st, false, 10_002));
        assert!(worth_storing(&p, st, false, 10_000 + RECENT_SECS));
        // Changed while it was read, or gone (the read failed): not kept.
        assert!(!worth_storing(&p, (st.0 + 1, st.1), true, 20_000));
        assert!(!worth_storing(&p, (st.0, st.1 - 1), false, 20_000));
        fs::remove_file(&p).unwrap();
        assert!(!worth_storing(&p, st, false, 20_000));
        assert_eq!(stamp(&p), None);
        assert_eq!(stamp(&dir), None);

        // Through text_of: a failure on a fresh file is tried again; text is
        // served from the cache only while the disk still matches.
        let cache = ContentCache::open(&dir.join("store")).unwrap();
        let empty = dir.join("فارغ.txt");
        fs::write(&empty, " ").unwrap();
        let path = empty.to_str().unwrap();
        assert_eq!(text_of(Some(&cache), path), (None, false));
        assert_eq!(cache.stats().documents, 0);
        file_entry(&empty, 10_000);
        assert_eq!(text_of(Some(&cache), path), (None, false));
        assert_eq!(text_of(Some(&cache), path), (None, true));
        fs::write(&empty, "نص").unwrap();
        file_entry(&empty, 10_000);
        assert_eq!(text_of(Some(&cache), path), (Some("نص".into()), false));
        assert_eq!(text_of(Some(&cache), path), (Some("نص".into()), true));
        drop(cache);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A small xorshift generator: the same cases on every run.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n.max(1) as u64) as usize
        }

        fn bytes(&mut self, max: usize) -> Vec<u8> {
            (0..self.below(max)).map(|_| self.below(256) as u8).collect()
        }
    }

    /// An idx with a valid checksum around `payload`, as `write_idx` makes.
    fn sealed(mut payload: Vec<u8>) -> Vec<u8> {
        let sum = fnv(&payload);
        payload.extend_from_slice(&sum.to_le_bytes());
        payload
    }

    fn idx_bytes(id: u64, dat_len: u64, entries: &[(&str, Meta)]) -> Vec<u8> {
        let map: Map = entries.iter().map(|(p, m)| (Box::from(*p), *m)).collect();
        sealed(bincode::serialize(&IdxRef { version: VERSION, id, dat_len, entries: &map }).unwrap())
    }

    #[test]
    fn random_idx_bytes_never_panic() {
        let mut rng = Rng(0xC0FF_EE11);
        for i in 0..3000 {
            let b = rng.bytes(300);
            assert!(decode_idx(&b).is_none() || b.len() >= 8);
            // Random payloads that pass the checksum, some behind a good
            // version: lengths and counts are whatever the bytes say.
            let mut p = rng.bytes(200);
            if i % 2 == 0 && p.len() >= 4 {
                p[..4].copy_from_slice(&VERSION.to_le_bytes());
            }
            let _ = decode_idx(&sealed(p));
        }
        // A map claiming billions of entries, a path claiming exabytes.
        let mut p = Vec::new();
        for v in [u64::from(VERSION), 7, 100, u64::MAX / 3, u64::MAX / 5] {
            p.extend_from_slice(&v.to_le_bytes());
        }
        assert!(decode_idx(&sealed(p.clone())).is_none());
        p[4..8].copy_from_slice(&[0; 4]);
        assert!(decode_idx(&sealed(p)).is_none());
    }

    #[test]
    fn damaged_idx_files_open_as_an_empty_store() {
        let dir = temp_dir("fuzzopen");
        let c = ContentCache::open(&dir).unwrap();
        c.put("a", 1, 2, Some(AR));
        c.flush().unwrap();
        drop(c);
        let good_idx = fs::read(dir.join(IDX)).unwrap();
        let good_dat = fs::read(dir.join(DAT)).unwrap();
        let mut rng = Rng(0x5EED_0001);
        for i in 0..200 {
            let mut b = good_idx.clone();
            match i % 3 {
                0 => b.truncate(rng.below(b.len())),
                1 => {
                    let at = rng.below(b.len());
                    b[at] ^= 1 + rng.below(255) as u8;
                }
                _ => b = rng.bytes(100),
            }
            fs::write(dir.join(IDX), &b).unwrap();
            fs::write(dir.join(DAT), &good_dat).unwrap();
            let c = ContentCache::open(&dir).unwrap();
            assert_ne!(c.get("a", 1, 2), Some(Some("wrong".into())));
            c.put("b", 1, 1, Some("ب"));
            assert_eq!(c.get("b", 1, 1), Some(Some("ب".into())));
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn idx_with_impossible_records_is_thrown_away() {
        let dir = temp_dir("impossible");
        drop(ContentCache::open(&dir).unwrap());
        let id = decode_idx(&fs::read(dir.join(IDX)).unwrap()).unwrap().id;
        let text = Meta { size: 1, mtime: 1, offset: HEADER_LEN, len: 4, raw_len: 4, hash: 0, kind: Kind::Text };
        // Offset + length past u64::MAX: an overflow panic in debug builds,
        // a wrapped (accepted) range in release ones.
        let wraps = Meta { offset: u64::MAX - 1, ..text };
        fs::write(dir.join(IDX), idx_bytes(id, HEADER_LEN, &[("x", wraps)])).unwrap();
        assert_eq!(ContentCache::open(&dir).unwrap().stats().documents, 0);
        // Records past the end of the dat, or before its start.
        drop(ContentCache::open(&dir).unwrap());
        let id = decode_idx(&fs::read(dir.join(IDX)).unwrap()).unwrap().id;
        for bad in [Meta { offset: HEADER_LEN + 1, ..text }, Meta { offset: 3, ..text }] {
            fs::write(dir.join(IDX), idx_bytes(id, HEADER_LEN, &[("x", bad)])).unwrap();
            assert_eq!(ContentCache::open(&dir).unwrap().stats().documents, 0);
            drop(ContentCache::open(&dir).unwrap());
        }
        // A record claiming a gigantic text is not decompressed into one.
        let c = ContentCache::open(&dir).unwrap();
        c.put("y", 1, 1, Some("abcd"));
        c.flush().unwrap();
        drop(c);
        let idx = decode_idx(&fs::read(dir.join(IDX)).unwrap()).unwrap();
        let mut m = idx.entries["y"];
        m.raw_len = u32::MAX;
        fs::write(dir.join(IDX), idx_bytes(idx.id, idx.dat_len, &[("y", m)])).unwrap();
        let c = ContentCache::open(&dir).unwrap();
        assert_eq!(c.stats().documents, 1);
        assert_eq!(c.get("y", 1, 1), None);
        drop(c);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A one-page PDF with `lines` of text, with correct xref offsets.
    fn bench_pdf(lines: &[String]) -> Vec<u8> {
        let body: String = lines.iter().map(|l| format!("({l}) Tj T* ")).collect();
        let stream = format!("BT /F1 12 Tf 14 TL 72 720 Td {body}ET");
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_owned(),
            format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_owned(),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
        }
        let xref = pdf.len();
        pdf.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
        for off in offsets {
            pdf.extend(format!("{off:010} 00000 n \n").bytes());
        }
        pdf.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).bytes());
        pdf
    }

    /// Writes a synthetic corpus of `n` documents (docx, txt, pdf, md) under
    /// `dir` and returns their entries.
    fn bench_corpus(dir: &Path, n: usize) -> Vec<Entry> {
        let mut rng = Rng(0xBE7C_0001);
        let words = ["تقرير", "المبيعات", "السنوي", "الإيرادات", "ميزانية", "report", "budget", "invoice", "عقد", "موظف"];
        let mut sentence = |k: usize| -> String {
            (0..k).map(|_| words[rng.below(words.len())]).collect::<Vec<_>>().join(" ")
        };
        let mut out = Vec::new();
        for i in 0..n {
            let sub = dir.join(format!("d{}", i % 50));
            fs::create_dir_all(&sub).unwrap();
            let (p, bytes): (PathBuf, Vec<u8>) = match i % 4 {
                0 => {
                    let p = sub.join(format!("doc{i}.docx"));
                    let paras: String = (0..60)
                        .map(|_| format!("<w:p><w:r><w:t>{}</w:t></w:r></w:p>", sentence(25)))
                        .collect();
                    let xml = format!(
                        r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{paras}</w:body></w:document>"#
                    );
                    let mut zw = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
                    zw.start_file("word/document.xml", zip::write::SimpleFileOptions::default()).unwrap();
                    zw.write_all(xml.as_bytes()).unwrap();
                    (p, zw.finish().unwrap().into_inner())
                }
                1 => (sub.join(format!("t{i}.txt")), (0..80).map(|_| sentence(20) + "\n").collect::<String>().into_bytes()),
                2 => {
                    let lines: Vec<String> = (0..40).map(|j| format!("Annual report line {j} budget invoice {i}")).collect();
                    (sub.join(format!("p{i}.pdf")), bench_pdf(&lines))
                }
                _ => (sub.join(format!("n{i}.md")), (0..40).map(|_| sentence(15) + "\n").collect::<String>().into_bytes()),
            };
            fs::write(&p, bytes).unwrap();
            out.push(file_entry(&p, 1_600_000_000 + i as i64));
        }
        out
    }

    /// `cargo test --release bench_warm -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_warm() {
        let n: usize = std::env::var("BENCH_DOCS").ok().and_then(|s| s.parse().ok()).unwrap_or(4000);
        let dir = temp_dir("bench");
        let entries = bench_corpus(&dir.join("docs"), n);
        let cache = ContentCache::open(&dir.join("store")).unwrap();
        if let Ok(cmd) = std::env::var("BENCH_BEFORE") {
            // e.g. dropping the OS file cache, to read from the disk.
            std::process::Command::new("sh").args(["-c", &cmd]).status().unwrap();
        }
        let threads: usize = std::env::var("BENCH_THREADS").ok().and_then(|s| s.parse().ok()).unwrap_or_else(warm_threads);
        let p = WarmProgress::default();
        let t = std::time::Instant::now();
        warm(&entries, &cache, &p, &AtomicBool::new(false), threads, &|| {});
        let s = cache.stats();
        eprintln!(
            "warm {n} docs on {threads} threads: {} ms, added {}, texts {}, {} KB",
            t.elapsed().as_millis(),
            p.added.load(Ordering::Relaxed),
            s.texts,
            s.bytes / 1024
        );
        drop(cache);
        // Index writes while 100k documents are added, saving every 200 (the
        // old policy) and once at the end.
        let big = ContentCache::open(&dir.join("big")).unwrap();
        let (mut ms, mut bytes) = (0, 0);
        for i in 0..100_000 {
            big.put(&format!("C:\\Users\\someone\\Documents\\folder {}\\a document {i}.docx", i % 300), 1, 1, None);
            if i % 200 == 199 {
                let t = std::time::Instant::now();
                big.flush().unwrap();
                ms += t.elapsed().as_millis();
                bytes += fs::metadata(dir.join("big").join(IDX)).unwrap().len();
            }
        }
        let last = fs::metadata(dir.join("big").join(IDX)).unwrap().len();
        eprintln!("100k documents, saved every 200: {ms} ms, {} MB written; one save: {} KB", bytes >> 20, last >> 10);
        // The same with `time_to_save`, at a few extraction speeds.
        let per_doc = last as f64 / 100_000.0;
        for rate in [10u64, 50, 200, 1000] {
            let (mut saves, mut written, mut at, mut at_added) = (0, 0.0, 0u64, 0usize);
            for i in 1..=100_000usize {
                let now_ms = i as u64 * 1000 / rate;
                if time_to_save(Duration::from_millis(now_ms - at), i - at_added, i) {
                    saves += 1;
                    written += per_doc * i as f64;
                    (at, at_added) = (now_ms, i);
                }
            }
            eprintln!("  new policy at {rate} docs/s: {saves} saves, {} MB written", written as u64 >> 20);
        }
        drop(big);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_crash_between_writes_keeps_a_usable_store() {
        // The idx is replaced by a rename: a leftover temp file from a crash
        // mid-write changes nothing, and the old idx still opens.
        let dir = temp_dir("crash");
        let c = ContentCache::open(&dir).unwrap();
        c.put("a", 1, 2, Some(AR));
        c.flush().unwrap();
        drop(c);
        fs::write(dir.join("content.idx.tmp"), b"half an id").unwrap();
        fs::write(dir.join("content.dat.tmp"), b"half a dat").unwrap();
        let c = ContentCache::open(&dir).unwrap();
        assert_eq!(c.get("a", 1, 2), Some(Some(AR.to_string())));
        // Records appended after the last flush, cut off mid-write.
        c.put("b", 1, 1, Some(&AR.repeat(50)));
        drop(c);
        let dat = dir.join(DAT);
        let len = fs::metadata(&dat).unwrap().len();
        OpenOptions::new().write(true).open(&dat).unwrap().set_len(len - 7).unwrap();
        let c = ContentCache::open(&dir).unwrap();
        assert_eq!(c.get("a", 1, 2), Some(Some(AR.to_string())));
        assert_eq!(c.get("b", 1, 1), None);
        c.flush().unwrap();
        c.put("c", 1, 1, Some("ج"));
        c.flush().unwrap();
        drop(c);
        let c = ContentCache::open(&dir).unwrap();
        assert_eq!(c.get("c", 1, 1), Some(Some("ج".into())));
        drop(c);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn markers_round_trip_and_reject_damage() {
        let m = encode_marker("C:\\تقارير\\ملف.pdf", 1234, -5);
        assert_eq!(decode_marker(&m), Some(("C:\\تقارير\\ملف.pdf".into(), 1234, -5)));
        // A longer old marker after it changes nothing; a cut or a flip does.
        let mut longer = m.clone();
        longer.extend(b"rest of an older, longer marker");
        assert!(decode_marker(&longer).is_some());
        assert_eq!(decode_marker(&m[..m.len() - 1]), None);
        let mut flipped = m.clone();
        flipped[10] ^= 1;
        assert_eq!(decode_marker(&flipped), None);
        assert_eq!(decode_marker(&[0, 0, 0, 0]), None);
        assert_eq!(decode_marker(&[]), None);
        let mut huge = INFLIGHT_MAGIC.to_vec();
        huge.extend([0u8; 16]);
        huge.extend(u32::MAX.to_le_bytes());
        assert_eq!(decode_marker(&huge), None);
    }

    #[test]
    fn a_file_the_last_run_died_in_is_skipped_until_it_changes() {
        let dir = temp_dir("inflight");
        let doc = dir.with_extension("doc.txt");
        fs::write(&doc, "نص").unwrap();
        let st = stamp(&doc).unwrap();
        let path = doc.to_str().unwrap();
        {
            let c = ContentCache::open(&dir).unwrap();
            // A finished extraction leaves no marker; slots are reused.
            drop(c.inflight.begin(path, st.0, st.1).unwrap());
            drop(c.inflight.begin("other", 1, 1).unwrap());
            assert_eq!(marker_files(&dir).len(), 1);
            // One that never finishes (the process dies inside it).
            std::mem::forget(c.inflight.begin(path, st.0, st.1).unwrap());
            // Two at once use two slots.
            let _a = c.inflight.begin("a", 1, 1).unwrap();
            assert_eq!(marker_files(&dir).len(), 2);
            // Simulates the death: the second marker stays as it is.
            std::mem::forget(_a);
        }
        let c = ContentCache::open(&dir).unwrap();
        // "a" does not exist: reported, nothing stored for it.
        assert_eq!(c.recovered().len(), 2);
        assert!(c.recovered().iter().any(|p| p == path));
        assert_eq!(c.get(path, st.0, st.1), Some(None));
        assert_eq!(text_of(Some(&c), path), (None, true));
        assert!(marker_files(&dir).is_empty());
        drop(c);
        // Reported once only; and the file is read again once it changes.
        let c = ContentCache::open(&dir).unwrap();
        assert!(c.recovered().is_empty());
        assert_eq!(c.get(path, st.0, st.1), Some(None));
        assert_eq!(c.get(path, st.0 + 1, st.1), None);
        fs::remove_dir_all(&dir).unwrap();
        fs::remove_file(&doc).unwrap();
    }

    #[test]
    fn a_marker_for_a_changed_file_poisons_nothing() {
        let dir = temp_dir("inflight_changed");
        let doc = dir.with_extension("changed.txt");
        fs::write(&doc, "قديم").unwrap();
        let st = stamp(&doc).unwrap();
        {
            let c = ContentCache::open(&dir).unwrap();
            std::mem::forget(c.inflight.begin(doc.to_str().unwrap(), st.0, st.1).unwrap());
        }
        fs::write(&doc, "نص أطول من القديم").unwrap();
        let c = ContentCache::open(&dir).unwrap();
        assert_eq!(c.recovered().len(), 1);
        assert_eq!(c.stats().documents, 0);
        fs::remove_dir_all(&dir).unwrap();
        fs::remove_file(&doc).unwrap();
    }

    #[test]
    fn markers_cleared_at_a_normal_exit() {
        let dir = temp_dir("inflight_exit");
        let c = ContentCache::open(&dir).unwrap();
        std::mem::forget(c.inflight.begin("cut off by the exit", 1, 1).unwrap());
        // What the atexit handler does, for this store.
        clear_markers_in(&dir);
        drop(c);
        let c = ContentCache::open(&dir).unwrap();
        assert!(c.recovered().is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn marker_overhead() {
        // Two small in-place writes per document, no fsync.
        let dir = temp_dir("inflight_cost");
        let c = ContentCache::open(&dir).unwrap();
        let path = "C:\\Users\\someone\\Documents\\تقارير ٢٠٢٦\\تقرير المبيعات السنوي.pdf";
        let n = 20_000;
        let t = Instant::now();
        for i in 0..n {
            drop(c.inflight.begin(path, i, 1));
        }
        let per = t.elapsed() / n as u32;
        eprintln!("marker set + clear: {} ns per document", per.as_nanos());
        assert!(per < Duration::from_millis(1), "{per:?}");
        drop(c);
        fs::remove_dir_all(&dir).unwrap();
    }
}
