//! In-memory index of every file and folder on the selected drives.
//!
//! The index is built by a parallel directory walk and cached on disk, so the
//! next start-up is instant and filename searches never touch the disk.

// Only the Windows UI and the tests use this module.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::UNIX_EPOCH;

use crate::normalize::normalize;

/// Bumped whenever the on-disk format changes.
const CACHE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone)]
pub struct Entry {
    pub path: Box<str>,
    pub size: u64,
    /// Modification time, seconds since the Unix epoch.
    pub mtime: i64,
    pub is_dir: bool,
    /// Normalised file name, rebuilt after loading (not stored).
    #[serde(skip)]
    pub norm_name: Box<str>,
}

impl Entry {
    pub fn name(&self) -> &str {
        file_name(&self.path)
    }

    /// Lower-case extension without the dot, or "" for none.
    pub fn ext(&self) -> String {
        let name = self.name();
        match name.rfind('.') {
            Some(i) if i > 0 && !self.is_dir => name[i + 1..].to_lowercase(),
            _ => String::new(),
        }
    }
}

fn file_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['\\', '/']);
    match trimmed.rfind(['\\', '/']) {
        Some(i) => &trimmed[i + 1..],
        None => trimmed,
    }
}

#[derive(Serialize, Deserialize, Default)]
pub struct Index {
    pub entries: Vec<Entry>,
    /// Roots that were scanned, e.g. `C:\`.
    pub roots: Vec<String>,
    /// When the index was built, seconds since the Unix epoch.
    pub built_at: i64,
}

#[derive(Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    index: Index,
}

/// Shared progress counters for a running scan.
#[derive(Default)]
pub struct ScanProgress {
    pub files: AtomicUsize,
    pub cancel: AtomicBool,
}

/// Folders that are never worth indexing and are slow or access-denied.
pub fn skip_dir(name: &str) -> bool {
    name.eq_ignore_ascii_case("System Volume Information")
        || name.eq_ignore_ascii_case("$Recycle.Bin")
        || name.eq_ignore_ascii_case("$WinREAgent")
        || name.eq_ignore_ascii_case("WinSxS")
        || [".git", ".svn", ".hg", "node_modules", "__pycache__"].iter().any(|d| name.eq_ignore_ascii_case(d))
}

/// True when `path` lies inside a folder the walker never enters. The folder
/// itself is indexed (it is listed by its parent), so only ancestors count.
pub fn skipped_path(path: &str) -> bool {
    let mut parts: Vec<&str> = path.split(['\\', '/']).filter(|p| !p.is_empty()).collect();
    parts.pop();
    parts.into_iter().any(skip_dir)
}

/// One change on disk, as reported by the file watcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Added(String),
    Removed(String),
    Modified(String),
    Renamed(String, String),
    /// Changes under this folder were lost (the watch overflowed or broke);
    /// it has to be walked again.
    Rescan(String),
}

impl Change {
    /// True when the change concerns `dir` or something inside it.
    pub fn touches(&self, dir: &str) -> bool {
        let under = |p: &str| p == dir || is_under(p, dir);
        match self {
            Change::Added(p) | Change::Removed(p) | Change::Modified(p) | Change::Rescan(p) => under(p),
            Change::Renamed(a, b) => under(a) || under(b),
        }
    }
}

/// A changed path as the index spells it: without a trailing separator.
/// `None` for a drive root or a path without any separator, which is never
/// an entry; read as a folder, a change to it would hit everything below.
pub fn change_path(p: &str) -> Option<&str> {
    let t = p.trim_end_matches(['\\', '/']);
    t.contains(['\\', '/']).then_some(t)
}

/// `path` is strictly inside the folder `dir`.
fn is_under(path: &str, dir: &str) -> bool {
    path.strip_prefix(dir).is_some_and(|rest| {
        !rest.is_empty() && (dir.ends_with(['\\', '/']) || rest.starts_with(['\\', '/']))
    })
}

/// What `apply_changes` did.
#[derive(Debug, Default, PartialEq)]
pub struct Applied {
    /// Entries added, updated or removed.
    pub changed: usize,
    /// Folders whose contents are unknown (moved in from elsewhere, or a
    /// watch that lost events) and need `scan_subtree`.
    pub scan: Vec<String>,
}

impl Index {
    /// Scans all `roots` in parallel and returns the finished index.
    pub fn build(roots: &[String], progress: &ScanProgress) -> Index {
        Index { entries: walk_all(roots, 500_000, progress), roots: roots.to_vec(), built_at: now_secs() }
    }

    /// Everything below `dir` (not `dir` itself), sorted, with the same rules
    /// as a full scan. Used for folders that appear with unknown contents.
    pub fn scan_subtree(dir: &str, progress: &ScanProgress) -> Vec<Entry> {
        walk_all(&[dir.to_string()], 0, progress)
    }

    pub fn load(path: &Path) -> Option<Index> {
        use bincode::Options;
        let f = fs::File::open(path).ok()?;
        // A damaged file can claim a string of any length, and reading from
        // a stream allocates that length up front: nothing may claim more
        // bytes than the file has. Same encoding as `bincode::serialize`.
        let limit = f.metadata().ok()?.len();
        let cache: CacheFile = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .allow_trailing_bytes()
            .with_limit(limit)
            .deserialize_from(BufReader::new(f))
            .ok()?;
        if cache.version != CACHE_VERSION {
            return None;
        }
        let mut index = cache.index;
        // Lookups, renames and merges rely on sorted, unique paths.
        if !index.entries.windows(2).all(|w| w[0].path < w[1].path) {
            sort_by_path(&mut index.entries);
            index.entries.dedup_by(|a, b| a.path == b.path);
        }
        fill_norm_names(&mut index.entries);
        Some(index)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        // Saves can come from several threads (scan end, rename, delete);
        // they share one temp file, so they take turns.
        static SAVE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _turn = SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        self.save_through(path, &tmp_path(path), &|from, to| fs::rename(from, to))
    }

    /// [`save`](Self::save) through the temp file `tmp`, put in place with
    /// `rename` (tried a few times: on Windows a scanner or another reader
    /// that has `index.bin` open makes it fail for a moment). When it keeps
    /// failing the temp file is removed, the old `index.bin` stays as it
    /// was, and the error says so.
    fn save_through(
        &self,
        path: &Path,
        tmp: &Path,
        rename: &dyn Fn(&Path, &Path) -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        // Write to a temp file first so a crash never leaves a broken cache.
        {
            let mut f = BufWriter::new(fs::File::create(tmp)?);
            #[derive(Serialize)]
            struct CacheRef<'a> {
                version: u32,
                index: &'a Index,
            }
            bincode::serialize_into(&mut f, &CacheRef { version: CACHE_VERSION, index: self })
                .map_err(std::io::Error::other)?;
            // Flush and sync explicitly: dropping a BufWriter ignores a write
            // error (a full disk), and the rename would then put a cut-off
            // file in place of the good one.
            f.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        }
        let mut tries = 0;
        loop {
            tries += 1;
            match rename(tmp, path) {
                Ok(()) => return Ok(()),
                Err(e) if tries >= RENAME_TRIES => {
                    let _ = fs::remove_file(tmp);
                    return Err(std::io::Error::new(
                        e.kind(),
                        format!("cannot put the new index in place after {tries} tries ({e}); the last saved index is kept"),
                    ));
                }
                Err(_) => std::thread::sleep(RENAME_PAUSE),
            }
        }
    }

    /// Position of the entry with exactly this path (binary search).
    pub fn find(&self, path: &str) -> Option<usize> {
        self.entries.binary_search_by(|e| (*e.path).cmp(path)).ok()
    }

    /// Index ranges of `path` itself and everything below it. Descendants
    /// start with `path` plus a separator, so in byte order they form one run
    /// per separator; siblings like `C:\ab` or `C:\a b` fall outside it.
    fn subtree(&self, path: &str) -> Vec<std::ops::Range<usize>> {
        let mut ranges = self.descendants(path);
        // A drive root is not an entry of its own.
        if !path.ends_with(['\\', '/'])
            && let Some(i) = self.find(path)
        {
            ranges.push(i..i + 1);
        }
        ranges.sort_unstable_by_key(|r| r.start);
        ranges
    }

    /// Index ranges of everything below `path`, but not `path` itself.
    fn descendants(&self, path: &str) -> Vec<std::ops::Range<usize>> {
        // A path ending in a separator (a drive root) is its own prefix.
        let prefixes =
            if path.ends_with(['\\', '/']) { vec![path.to_string()] } else { vec![format!("{path}\\"), format!("{path}/")] };
        let mut ranges = Vec::new();
        for p in prefixes {
            let start = self.entries.partition_point(|e| *e.path < *p);
            let len = self.entries[start..].partition_point(|e| e.path.starts_with(&p));
            if len > 0 {
                ranges.push(start..start + len);
            }
        }
        ranges.sort_unstable_by_key(|r| r.start);
        ranges
    }

    /// Removes `path` and, for a folder, everything inside it. Returns how
    /// many entries were removed. Entry indices held elsewhere become stale.
    pub fn remove_path(&mut self, path: &str) -> usize {
        let ranges = self.subtree(path);
        let mut removed = 0;
        for r in ranges.into_iter().rev() {
            removed += r.len();
            self.entries.drain(r);
        }
        removed
    }

    /// Renames `old` to `new`, moving everything inside it along for a
    /// folder, and keeps the entries sorted. An entry already at `new` is
    /// replaced. Returns false when `old` is not in the index. Entry indices
    /// held elsewhere become stale.
    pub fn rename_path(&mut self, old: &str, new: &str) -> bool {
        if old == new {
            return self.find(old).is_some();
        }
        let ranges = self.subtree(old);
        if ranges.is_empty() {
            return false;
        }
        let mut moved = Vec::with_capacity(ranges.iter().map(|r| r.len()).sum());
        for r in ranges.into_iter().rev() {
            moved.extend(self.entries.drain(r));
        }
        for e in &mut moved {
            let renamed_self = e.path.len() == old.len();
            e.path = format!("{new}{}", &e.path[old.len()..]).into();
            if renamed_self {
                e.norm_name = normalize(e.name()).into();
            }
        }
        self.remove_path(new);
        sort_by_path(&mut moved);
        let rest = std::mem::take(&mut self.entries);
        self.entries = merge_by_path(rest, moved);
        true
    }

    /// Applies a batch of watcher changes. `stat` reports what is on disk now
    /// (size, mtime, is_dir), or `None` for a path that is gone or is not
    /// indexed (a link). Disk is the truth, so a file created and deleted
    /// within one batch simply never appears.
    ///
    /// Each path is reconciled once and all inserts and removals are done in
    /// one pass each, so a big batch on millions of entries stays cheap. Only
    /// a folder renamed with contents is moved on its own, so its contents
    /// come along without a scan. Entry indices held elsewhere become stale.
    pub fn apply_changes(&mut self, changes: &[Change], stat: impl Fn(&str) -> Option<(u64, i64, bool)>) -> Applied {
        let mut out = Applied::default();
        // Paths whose entry must be made to match the disk.
        let mut dirty: Vec<String> = Vec::with_capacity(changes.len());
        // Paths reported gone (deleted or renamed away): a folder found
        // there now is another one, whose contents are unknown.
        let mut removed: Vec<String> = Vec::new();
        for c in changes {
            match c {
                Change::Added(p) | Change::Modified(p) | Change::Removed(p) => {
                    if let Some(p) = change_path(p)
                        && !skipped_path(p)
                    {
                        if let Change::Removed(_) = c {
                            removed.push(p.to_string());
                        }
                        dirty.push(p.to_string());
                    }
                }
                Change::Renamed(old, new) => {
                    let (old_seen, new_seen) = (
                        change_path(old).filter(|p| !skipped_path(p)),
                        change_path(new).filter(|p| !skipped_path(p)),
                    );
                    // Into a folder name the walker never enters (renamed to
                    // node_modules): the contents go, as if scanned.
                    if let (Some(old), Some(new)) = (old_seen, new_seen)
                        && !skip_dir(file_name(new))
                        && !self.descendants(old).is_empty()
                    {
                        // Paths noted earlier in the batch move along too.
                        for d in dirty.iter_mut().chain(&mut removed) {
                            if is_under(d, old) {
                                *d = format!("{new}{}", &d[old.len()..]);
                            }
                        }
                        if self.rename_path(old, new) {
                            out.changed += 1;
                        }
                    }
                    // Moved into the recycle bin and the like: just gone.
                    removed.extend(old_seen.map(str::to_string));
                    dirty.extend(old_seen.into_iter().chain(new_seen).map(str::to_string));
                }
                Change::Rescan(root) => out.scan.push(root.clone()),
            }
        }
        dirty.sort_unstable();
        dirty.dedup();
        removed.sort_unstable();

        let mut upserts = Vec::new();
        let mut gone = Vec::new();
        for p in dirty {
            match stat(&p) {
                Some((size, mtime, is_dir)) => upserts.push((p, size, mtime, is_dir)),
                None => gone.push(p),
            }
        }

        // Removals: collect every range, then drop them all in one pass.
        let mut ranges: Vec<std::ops::Range<usize>> = gone.iter().flat_map(|p| self.subtree(p)).collect();
        if !ranges.is_empty() {
            ranges.sort_unstable_by_key(|r| r.start);
            let before = self.entries.len();
            let (mut i, mut r) = (0, 0);
            self.entries.retain(|_| {
                while r < ranges.len() && ranges[r].end <= i {
                    r += 1;
                }
                let keep = !(r < ranges.len() && ranges[r].contains(&i));
                i += 1;
                keep
            });
            out.changed += before - self.entries.len();
        }

        // Updates in place; new entries are gathered (already in path order,
        // since `dirty` was sorted) and merged in once.
        let mut fresh = Vec::new();
        for (p, size, mtime, is_dir) in upserts {
            match self.find(&p) {
                Some(i) => {
                    let e = &mut self.entries[i];
                    let was_dir = e.is_dir;
                    if (e.size, e.mtime, e.is_dir) != (size, mtime, is_dir) {
                        (e.size, e.mtime, e.is_dir) = (size, mtime, is_dir);
                        out.changed += 1;
                    }
                    // Deleted and made again, or a file replaced by a folder
                    // (or the other way round): what was inside is stale.
                    let replaced = was_dir != is_dir || removed.binary_search(&p).is_ok();
                    if replaced && was_dir {
                        for r in self.descendants(&p).into_iter().rev() {
                            out.changed += r.len();
                            self.entries.drain(r);
                        }
                    }
                    if replaced && is_dir && !skip_dir(file_name(&p)) {
                        out.scan.push(p);
                    }
                }
                None => {
                    let norm_name = normalize(file_name(&p)).into();
                    let e = Entry { path: p.into(), size, mtime, is_dir, norm_name };
                    // A folder that just appeared may have been moved in
                    // with contents; the walker would not enter skipped ones.
                    if is_dir && !skip_dir(e.name()) {
                        out.scan.push(e.path.to_string());
                    }
                    fresh.push(e);
                }
            }
        }
        if !fresh.is_empty() {
            out.changed += fresh.len();
            let rest = std::mem::take(&mut self.entries);
            self.entries = merge_by_path(rest, fresh);
        }
        out.scan.sort_unstable();
        out.scan.dedup();
        out
    }

    /// Replaces everything below `dir` with `found` (from `scan_subtree`,
    /// sorted by path). `dir`'s own entry is kept. Returns the new count of
    /// entries below `dir`.
    pub fn replace_subtree(&mut self, dir: &str, found: Vec<Entry>) -> usize {
        for r in self.descendants(dir).into_iter().rev() {
            self.entries.drain(r);
        }
        let n = found.len();
        let rest = std::mem::take(&mut self.entries);
        self.entries = merge_by_path(rest, found);
        n
    }
}

/// How often putting a finished save in place is tried, and the pause between.
const RENAME_TRIES: u32 = 5;
const RENAME_PAUSE: std::time::Duration = std::time::Duration::from_millis(100);

/// The temp file a save of `path` is written to before it is renamed into
/// place: `index.<process id>.tmp`. One per process, so that two copies of
/// the program (one closing while the next starts, after an update) never
/// write into the same file; each renames a whole file, and the last wins.
pub fn tmp_path(path: &Path) -> PathBuf {
    tmp_path_of(path, std::process::id())
}

fn tmp_path_of(path: &Path, pid: u32) -> PathBuf {
    path.with_extension(format!("{pid}.tmp"))
}

/// Removes temp files left next to `path` by saves that never finished (the
/// program was closed or killed in the middle): `index.tmp` of older
/// versions and `index.<pid>.tmp` of other processes, when they have not
/// been written to for `older_than` (a copy that is still closing may be in
/// the middle of its last save). Returns how many were removed.
pub fn remove_stale_tmp(path: &Path, older_than: std::time::Duration) -> usize {
    let (Some(dir), Some(stem)) = (path.parent(), path.file_stem().and_then(|s| s.to_str())) else { return 0 };
    let own = tmp_path(path);
    let Ok(rd) = fs::read_dir(dir) else { return 0 };
    let mut removed = 0;
    for ent in rd.flatten() {
        let p = ent.path();
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else { continue };
        // `index.tmp` or `index.<digits>.tmp`, nothing else.
        let Some(middle) = name.strip_prefix(stem).and_then(|r| r.strip_suffix(".tmp")) else { continue };
        let ours = middle.is_empty() || middle.strip_prefix('.').is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()));
        if !ours || p == own || !ent.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let idle = ent.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
        if idle.is_some_and(|d| d >= older_than) && fs::remove_file(&p).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Walks `roots` in parallel; the result is sorted with names filled in.
fn walk_all(roots: &[String], capacity: usize, progress: &ScanProgress) -> Vec<Entry> {
    let out: Mutex<Vec<Entry>> = Mutex::new(Vec::with_capacity(capacity));
    rayon::scope(|s| {
        for root in roots {
            let out = &out;
            s.spawn(move |s| walk(s, PathBuf::from(root), out, progress));
        }
    });
    let mut entries = out.into_inner().unwrap_or_default();
    sort_by_path(&mut entries);
    fill_norm_names(&mut entries);
    entries
}

fn sort_by_path(entries: &mut [Entry]) {
    entries.par_sort_unstable_by(|a, b| a.path.cmp(&b.path));
}

/// Rebuilds the normalised names, which the cache does not store.
fn fill_norm_names(entries: &mut [Entry]) {
    entries.par_iter_mut().for_each(|e| e.norm_name = normalize(e.name()).into());
}

/// Merges two runs sorted by path in one pass (cheaper than re-sorting).
fn merge_by_path(a: Vec<Entry>, b: Vec<Entry>) -> Vec<Entry> {
    let mut merged = Vec::with_capacity(a.len() + b.len());
    let mut a = a.into_iter().peekable();
    let mut b = b.into_iter().peekable();
    loop {
        let take_a = match (a.peek(), b.peek()) {
            (Some(x), Some(y)) => x.path <= y.path,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        merged.extend(if take_a { a.next() } else { b.next() });
    }
    merged
}

fn walk<'s>(s: &rayon::Scope<'s>, dir: PathBuf, out: &'s Mutex<Vec<Entry>>, progress: &'s ScanProgress) {
    if progress.cancel.load(Ordering::Relaxed) {
        return;
    }
    let Ok(rd) = fs::read_dir(&dir) else { return };
    let mut local = Vec::new();
    for ent in rd.flatten() {
        // On Windows file_type() and metadata() come straight from the
        // directory listing, so they cost no extra system call.
        let Ok(ft) = ent.file_type() else { continue };
        // Symlinks and junctions are skipped to avoid loops and duplicates.
        if ft.is_symlink() {
            continue;
        }
        let path = ent.path();
        let Some(path_str) = path.to_str() else { continue };
        let (size, mtime) = match ent.metadata() {
            Ok(m) => (
                if ft.is_dir() { 0 } else { m.len() },
                m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64),
            ),
            Err(_) => (0, 0),
        };
        local.push(Entry { path: path_str.into(), size, mtime, is_dir: ft.is_dir(), norm_name: Box::default() });
        if ft.is_dir() && !ent.file_name().to_str().is_some_and(skip_dir) {
            s.spawn(move |s| walk(s, path, out, progress));
        }
    }
    progress.files.fetch_add(local.len(), Ordering::Relaxed);
    if let Ok(mut v) = out.lock() {
        v.append(&mut local);
    }
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Where the index cache lives: `%LOCALAPPDATA%\BahthSari\index.bin`.
#[cfg(windows)]
pub fn cache_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("BahthSari").join("index.bin")
}

/// Local fixed and removable drives, e.g. `["C:\\", "D:\\"]`.
#[cfg(windows)]
pub fn list_drives() -> Vec<String> {
    use winapi::um::fileapi::{GetDriveTypeW, GetLogicalDrives};
    use winapi::um::winbase::{DRIVE_FIXED, DRIVE_REMOVABLE};
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| format!("{}:\\", (b'A' + i) as char))
        .filter(|root| {
            let wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
            let t = unsafe { GetDriveTypeW(wide.as_ptr()) };
            t == DRIVE_FIXED || t == DRIVE_REMOVABLE
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skipped_dirs() {
        for d in ["node_modules", ".GIT", "__pycache__", ".svn", ".Hg", "$Recycle.Bin"] {
            assert!(skip_dir(d), "{d}");
        }
        assert!(!skip_dir("git") && !skip_dir("Documents"));
    }

    #[test]
    fn build_save_load_roundtrip() {
        let root = std::env::temp_dir().join(format!("bahth_idx_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("مستندات/فرعي")).unwrap();
        fs::write(root.join("مستندات/تقرير المبيعات.txt"), "x").unwrap();
        fs::write(root.join("مستندات/فرعي/Report.PDF"), "y").unwrap();

        let progress = ScanProgress::default();
        let idx = Index::build(&[root.to_str().unwrap().to_string()], &progress);
        assert_eq!(idx.entries.len(), 4);
        let e = idx.entries.iter().find(|e| e.name() == "Report.PDF").unwrap();
        assert_eq!(e.ext(), "pdf");
        assert_eq!(&*e.norm_name, "report.pdf");

        let cache = root.join("cache/index.bin");
        idx.save(&cache).unwrap();
        let loaded = Index::load(&cache).unwrap();
        assert_eq!(loaded.entries.len(), 4);
        assert!(loaded.entries.iter().any(|e| &*e.norm_name == "تقرير المبيعات.txt"));
        fs::remove_dir_all(&root).unwrap();
    }

    fn index_of(paths: &[&str]) -> Index {
        let mut entries: Vec<Entry> = paths
            .iter()
            .map(|p| Entry {
                path: (*p).into(),
                size: 1,
                mtime: 0,
                is_dir: !p.contains('.'),
                norm_name: normalize(file_name(p)).into(),
            })
            .collect();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Index { entries, ..Index::default() }
    }

    fn paths(idx: &Index) -> Vec<&str> {
        idx.entries.iter().map(|e| &*e.path).collect()
    }

    const TREE: &[&str] = &[
        r"C:\a",
        r"C:\a\x.txt",
        r"C:\a\sub",
        r"C:\a\sub\y.txt",
        r"C:\a b.txt",
        r"C:\a-c",
        r"C:\ab",
        r"C:\ab\z.txt",
        r"C:\b.txt",
    ];

    #[test]
    fn find_by_path() {
        let idx = index_of(TREE);
        for p in TREE {
            assert_eq!(idx.entries[idx.find(p).unwrap()].path.as_ref(), *p);
        }
        assert_eq!(idx.find(r"C:\nope"), None);
    }

    #[test]
    fn remove_folder_and_file() {
        let mut idx = index_of(TREE);
        assert_eq!(idx.remove_path(r"C:\a"), 4);
        assert_eq!(paths(&idx), vec![r"C:\a b.txt", r"C:\a-c", r"C:\ab", r"C:\ab\z.txt", r"C:\b.txt"]);
        assert_eq!(idx.remove_path(r"C:\b.txt"), 1);
        assert_eq!(idx.remove_path(r"C:\b.txt"), 0);
        assert_eq!(idx.remove_path(r"C:\"), 4);
        assert!(idx.entries.is_empty());
        let mut idx = index_of(&["/home/u", "/home/u/f.txt", "/home/u2"]);
        assert_eq!(idx.remove_path("/home/u"), 2);
    }

    #[test]
    fn rename_folder_moves_children() {
        let mut idx = index_of(TREE);
        assert!(idx.rename_path(r"C:\a", r"C:\zz"));
        let mut want: Vec<String> = TREE.iter().map(|p| p.replacen(r"C:\a\", r"C:\zz\", 1)).collect();
        want[0] = r"C:\zz".into();
        want.sort();
        assert_eq!(paths(&idx), want);
        let e = &idx.entries[idx.find(r"C:\zz").unwrap()];
        assert_eq!(&*e.norm_name, "zz");
        assert_eq!(&*idx.entries[idx.find(r"C:\zz\sub\y.txt").unwrap()].norm_name, "y.txt");
        assert!(!idx.rename_path(r"C:\a", r"C:\q"));
    }

    #[test]
    fn rename_file_keeps_order_and_replaces_target() {
        let mut idx = index_of(TREE);
        assert!(idx.rename_path(r"C:\b.txt", r"C:\A\تقرير.TXT"));
        assert!(idx.entries.windows(2).all(|w| w[0].path < w[1].path));
        assert_eq!(&*idx.entries[idx.find(r"C:\A\تقرير.TXT").unwrap()].norm_name, "تقرير.txt");
        // Renaming onto an existing path replaces it instead of duplicating it.
        assert!(idx.rename_path(r"C:\ab\z.txt", r"C:\a\x.txt"));
        assert_eq!(idx.entries.len(), TREE.len() - 1);
        assert!(idx.find(r"C:\ab\z.txt").is_none());
        assert!(idx.entries.windows(2).all(|w| w[0].path < w[1].path));
    }

    const TREE_AR: &[&str] = &[
        r"C:\تقارير",
        r"C:\تقارير\٢٠٢٤.pdf",
        r"C:\تقارير\فرعي",
        r"C:\تقارير\فرعي\ملخص.docx",
        r"C:\تقارير 2023.txt",
        r"C:\تقاريرقديمة",
        r"C:\تقاريرقديمة\قديم.txt",
        r"D:\مستندات\عقد.pdf",
    ];

    fn is_sorted(idx: &Index) -> bool {
        idx.entries.windows(2).all(|w| w[0].path < w[1].path)
    }

    #[test]
    fn arabic_find_and_ext() {
        let idx = index_of(TREE_AR);
        for p in TREE_AR {
            assert_eq!(&*idx.entries[idx.find(p).unwrap()].path, *p);
        }
        // Lookups are exact: no normalisation, no prefix matches.
        assert_eq!(idx.find(r"C:\تقاريرر"), None);
        assert_eq!(idx.find(r"C:\تقارير\"), None);
        let e = &idx.entries[idx.find(r"C:\تقارير\٢٠٢٤.pdf").unwrap()];
        assert_eq!((e.name(), e.ext().as_str(), &*e.norm_name), ("٢٠٢٤.pdf", "pdf", "2024.pdf"));
        let dir = Entry { path: r"C:\مجلد.قديم".into(), size: 0, mtime: 0, is_dir: true, norm_name: Box::default() };
        assert_eq!(dir.ext(), "");
        let hidden = Entry { path: r"C:\.ملف".into(), is_dir: false, ..dir };
        assert_eq!(hidden.ext(), "");
    }

    #[test]
    fn arabic_remove_keeps_prefix_siblings() {
        let mut idx = index_of(TREE_AR);
        assert_eq!(idx.remove_path(r"C:\تقارير"), 4);
        assert_eq!(paths(&idx), vec![r"C:\تقارير 2023.txt", r"C:\تقاريرقديمة", r"C:\تقاريرقديمة\قديم.txt", r"D:\مستندات\عقد.pdf"]);
        assert_eq!(idx.remove_path(r"D:\مستندات"), 1);
        assert_eq!(idx.remove_path(r"D:\مستندات"), 0);
        assert_eq!(idx.entries.len(), 3);
    }

    #[test]
    fn arabic_rename() {
        let mut idx = index_of(TREE_AR);
        // A folder moved under a new parent takes its children along.
        assert!(idx.rename_path(r"C:\تقارير", r"C:\أرشيف\تقارير"));
        assert!(is_sorted(&idx));
        assert_eq!(idx.entries.len(), TREE_AR.len());
        assert!(idx.find(r"C:\تقارير").is_none());
        let e = &idx.entries[idx.find(r"C:\أرشيف\تقارير\فرعي\ملخص.docx").unwrap()];
        assert_eq!(&*e.norm_name, "ملخص.docx");
        assert_eq!(&*idx.entries[idx.find(r"C:\أرشيف\تقارير").unwrap()].norm_name, "تقارير");

        // A renamed file gets a fresh normalised name.
        assert!(idx.rename_path(r"D:\مستندات\عقد.pdf", r"D:\مستندات\عقد الإيجار.PDF"));
        let e = &idx.entries[idx.find(r"D:\مستندات\عقد الإيجار.PDF").unwrap()];
        assert_eq!((&*e.norm_name, e.ext().as_str()), ("عقد الايجار.pdf", "pdf"));

        // Same name: true only when present. Missing source: nothing changes.
        assert!(idx.rename_path(r"C:\تقاريرقديمة", r"C:\تقاريرقديمة"));
        assert!(!idx.rename_path(r"C:\غير موجود", r"C:\غير موجود"));
        let before = paths(&idx).iter().map(|p| p.to_string()).collect::<Vec<_>>();
        assert!(!idx.rename_path(r"C:\غير موجود", r"C:\تقاريرقديمة"));
        assert_eq!(paths(&idx), before);
    }

    #[test]
    fn rename_folder_onto_existing_folder_replaces_its_subtree() {
        let mut idx = index_of(TREE_AR);
        assert!(idx.rename_path(r"C:\تقاريرقديمة", r"C:\تقارير"));
        assert!(is_sorted(&idx));
        assert_eq!(paths(&idx), vec![r"C:\تقارير", r"C:\تقارير 2023.txt", r"C:\تقارير\قديم.txt", r"D:\مستندات\عقد.pdf"]);
    }

    /// A fake disk for `apply_changes`: path -> (size, mtime, is_dir).
    type Disk = std::collections::HashMap<String, (u64, i64, bool)>;

    fn disk_of(paths: &[&str]) -> Disk {
        paths.iter().map(|p| (p.to_string(), (1, 0, !p.contains('.')))).collect()
    }

    fn apply(idx: &mut Index, disk: &Disk, changes: &[Change]) -> Applied {
        idx.apply_changes(changes, |p| disk.get(p).copied())
    }

    fn added(p: &str) -> Change {
        Change::Added(p.into())
    }

    #[test]
    fn skipped_paths_match_the_walker() {
        assert!(skipped_path(r"C:\p\node_modules\x.js"));
        assert!(skipped_path(r"C:\$Recycle.Bin\S-1-5\$R1.txt"));
        assert!(skipped_path(r"C:\Windows\WinSxS\amd64\a.dll"));
        // The skipped folder itself is listed by its parent.
        assert!(!skipped_path(r"C:\p\node_modules"));
        assert!(!skipped_path(r"C:\Users\a.txt") && !skipped_path(r"C:\"));
        assert!(skipped_path("/home/u/.git/HEAD") && !skipped_path("/home/u/.gitignore"));
    }

    #[test]
    fn add_new_file_and_folder() {
        let mut idx = index_of(TREE);
        let mut disk = disk_of(TREE);
        for p in [r"C:\a\new.txt", r"C:\a b", r"C:\zz.txt", r"C:\a\sub\node_modules"] {
            disk.insert(p.into(), (5, 7, !p.contains('.') || p.ends_with("node_modules")));
        }
        let changes: Vec<Change> =
            [r"C:\a\new.txt", r"C:\a b", r"C:\zz.txt", r"C:\a\sub\node_modules", r"C:\a\sub\node_modules\m.js"]
                .iter()
                .map(|p| added(p))
                .collect();
        let got = apply(&mut idx, &disk, &changes);
        // New folders may hold files nobody reported, except skipped ones.
        assert_eq!(got, Applied { changed: 4, scan: vec![r"C:\a b".into()] });
        assert!(is_sorted(&idx));
        let e = &idx.entries[idx.find(r"C:\a\new.txt").unwrap()];
        assert_eq!((e.size, e.mtime, e.is_dir, &*e.norm_name), (5, 7, false, "new.txt"));
        assert!(idx.find(r"C:\a\sub\node_modules\m.js").is_none());
        // Added twice, or added and gone again before the batch: no change.
        assert_eq!(apply(&mut idx, &disk, &[added(r"C:\zz.txt"), added(r"C:\gone.txt")]).changed, 0);
        assert_eq!(idx.entries.len(), TREE.len() + 4);
    }

    #[test]
    fn modify_updates_size_in_place() {
        let mut idx = index_of(TREE);
        let mut disk = disk_of(TREE);
        disk.insert(r"C:\a\x.txt".into(), (4096, 1_700_000_000, false));
        let got = apply(&mut idx, &disk, &[Change::Modified(r"C:\a\x.txt".into()), Change::Modified(r"C:\a".into())]);
        assert_eq!(got.changed, 1);
        let e = &idx.entries[idx.find(r"C:\a\x.txt").unwrap()];
        assert_eq!((e.size, e.mtime), (4096, 1_700_000_000));
        assert_eq!(idx.entries.len(), TREE.len());
    }

    #[test]
    fn remove_folder_with_children() {
        let mut idx = index_of(TREE);
        let mut disk = disk_of(TREE);
        for p in [r"C:\a", r"C:\a\x.txt", r"C:\a\sub", r"C:\a\sub\y.txt", r"C:\b.txt"] {
            disk.remove(p);
        }
        // Windows reports the children first, then the folder; any order works.
        let changes = [Change::Removed(r"C:\a".into()), Change::Removed(r"C:\a\sub\y.txt".into()), Change::Removed(r"C:\b.txt".into())];
        assert_eq!(apply(&mut idx, &disk, &changes).changed, 5);
        assert_eq!(paths(&idx), vec![r"C:\a b.txt", r"C:\a-c", r"C:\ab", r"C:\ab\z.txt"]);
        // Removed but back on disk (deleted and re-created): kept.
        assert_eq!(apply(&mut idx, &disk, &[Change::Removed(r"C:\ab\z.txt".into())]).changed, 0);
        assert_eq!(idx.entries.len(), 4);
    }

    #[test]
    fn rename_across_folders() {
        let mut idx = index_of(TREE);
        let mut disk = disk_of(TREE);
        let mv = |disk: &mut Disk, old: &str, new: &str| {
            let moved: Vec<(String, _)> = disk.iter().filter(|(p, _)| *p == old || p.starts_with(&format!("{old}\\"))).map(|(p, v)| (p.clone(), *v)).collect();
            for (p, v) in moved {
                disk.remove(&p);
                disk.insert(format!("{new}{}", &p[old.len()..]), v);
            }
        };
        // A folder with contents moves without a scan; so does a file.
        mv(&mut disk, r"C:\a\sub", r"C:\ab\sub2");
        mv(&mut disk, r"C:\b.txt", r"C:\a\b.txt");
        let got = apply(
            &mut idx,
            &disk,
            &[Change::Renamed(r"C:\a\sub".into(), r"C:\ab\sub2".into()), Change::Renamed(r"C:\b.txt".into(), r"C:\a\b.txt".into())],
        );
        assert!(got.scan.is_empty() && got.changed >= 2, "{got:?}");
        assert!(is_sorted(&idx));
        assert_eq!(idx.entries.len(), TREE.len());
        assert!(idx.find(r"C:\a\sub").is_none() && idx.find(r"C:\b.txt").is_none());
        assert_eq!(&*idx.entries[idx.find(r"C:\ab\sub2\y.txt").unwrap()].norm_name, "y.txt");
        assert_eq!(&*idx.entries[idx.find(r"C:\ab\sub2").unwrap()].norm_name, "sub2");
        assert!(idx.find(r"C:\a\b.txt").is_some());

        // A file created in a folder that is renamed in the same batch.
        disk.insert(r"C:\ab\sub2\late.txt".into(), (1, 0, false));
        mv(&mut disk, r"C:\ab\sub2", r"C:\sub3");
        apply(
            &mut idx,
            &disk,
            &[added(r"C:\ab\sub2\late.txt"), Change::Renamed(r"C:\ab\sub2".into(), r"C:\sub3".into())],
        );
        assert!(idx.find(r"C:\sub3\late.txt").is_some() && idx.find(r"C:\sub3\y.txt").is_some());
        assert!(!paths(&idx).iter().any(|p| p.contains("sub2")));

        // Into the recycle bin is a delete; out of it, a folder to scan.
        mv(&mut disk, r"C:\sub3", r"C:\$Recycle.Bin\S-1\$R1");
        apply(&mut idx, &disk, &[Change::Renamed(r"C:\sub3".into(), r"C:\$Recycle.Bin\S-1\$R1".into())]);
        assert!(!paths(&idx).iter().any(|p| p.contains("sub3") || p.contains("Recycle")));
        mv(&mut disk, r"C:\$Recycle.Bin\S-1\$R1", r"C:\sub3");
        let got = apply(&mut idx, &disk, &[Change::Renamed(r"C:\$Recycle.Bin\S-1\$R1".into(), r"C:\sub3".into())]);
        assert_eq!(got.scan, vec![r"C:\sub3".to_string()]);
        assert!(is_sorted(&idx));
    }

    #[test]
    fn arabic_changes() {
        let mut idx = index_of(TREE_AR);
        let mut disk = disk_of(TREE_AR);
        disk.insert(r"C:\تقارير\الربع الأول.xlsx".into(), (9, 1, false));
        disk.remove(r"C:\تقارير\٢٠٢٤.pdf");
        disk.insert(r"C:\تقارير\ملخص ٢٠٢٤.pdf".into(), (1, 0, false));
        disk.insert(r"C:\تقارير\مجلد جديد".into(), (0, 0, true));
        let got = apply(
            &mut idx,
            &disk,
            &[
                added(r"C:\تقارير\الربع الأول.xlsx"),
                Change::Renamed(r"C:\تقارير\٢٠٢٤.pdf".into(), r"C:\تقارير\ملخص ٢٠٢٤.pdf".into()),
                added(r"C:\تقارير\مجلد جديد"),
            ],
        );
        assert_eq!(got.scan, vec![r"C:\تقارير\مجلد جديد".to_string()]);
        assert!(is_sorted(&idx));
        assert_eq!(&*idx.entries[idx.find(r"C:\تقارير\ملخص ٢٠٢٤.pdf").unwrap()].norm_name, "ملخص 2024.pdf");
        assert_eq!(&*idx.entries[idx.find(r"C:\تقارير\الربع الأول.xlsx").unwrap()].norm_name, "الربع الاول.xlsx");
        assert!(idx.find(r"C:\تقارير\٢٠٢٤.pdf").is_none());
        // Removing a folder leaves its look-alike siblings alone.
        for p in TREE_AR.iter().filter(|p| p.starts_with(r"C:\تقارير\") || **p == r"C:\تقارير") {
            disk.remove(*p);
        }
        disk.retain(|p, _| !p.starts_with(r"C:\تقارير\"));
        apply(&mut idx, &disk, &[Change::Removed(r"C:\تقارير".into())]);
        assert_eq!(paths(&idx), vec![r"C:\تقارير 2023.txt", r"C:\تقاريرقديمة", r"C:\تقاريرقديمة\قديم.txt", r"D:\مستندات\عقد.pdf"]);
    }

    #[test]
    fn rescan_is_passed_on_and_subtree_replaced() {
        let mut idx = index_of(TREE);
        let got = apply(&mut idx, &disk_of(TREE), &[Change::Rescan(r"C:\".into())]);
        assert_eq!(got, Applied { changed: 0, scan: vec![r"C:\".into()] });
        let found = index_of(&[r"C:\a\new", r"C:\a\new\q.txt"]).entries;
        assert_eq!(idx.replace_subtree(r"C:\a", found), 2);
        assert!(is_sorted(&idx) && idx.find(r"C:\a").is_some() && idx.find(r"C:\a\new\q.txt").is_some());
        assert!(idx.find(r"C:\a\x.txt").is_none() && idx.find(r"C:\a\sub").is_none());
        assert_eq!(idx.entries.len(), TREE.len() - 3 + 2);
        assert!(Change::Renamed(r"C:\x".into(), r"C:\a\y".into()).touches(r"C:\a"));
        assert!(!Change::Added(r"C:\ab\y".into()).touches(r"C:\a"));
        assert!(Change::Added(r"C:\ab\y".into()).touches(r"C:\"));
    }

    #[test]
    fn scan_subtree_walks_one_folder() {
        let root = std::env::temp_dir().join(format!("bahth_sub_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("نقل/داخلي/node_modules/x")).unwrap();
        fs::write(root.join("نقل/داخلي/ملف.txt"), "abc").unwrap();
        fs::write(root.join("خارج.txt"), "").unwrap();
        let dir = root.join("نقل");
        let found = Index::scan_subtree(dir.to_str().unwrap(), &ScanProgress::default());
        let names: Vec<&str> = found.iter().map(|e| e.name()).collect();
        assert_eq!(names, vec!["داخلي", "node_modules", "ملف.txt"]);
        assert_eq!(found[2].size, 3);
        assert_eq!(&*found[2].norm_name, "ملف.txt");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn big_batch_stays_fast() {
        // 200k entries in 2,000 folders of 99 files, then 10k mixed changes.
        let mut entries = Vec::with_capacity(200_000);
        for d in 0..2000 {
            entries.push(format!(r"C:\data\d{d:04}"));
            for f in 0..99 {
                entries.push(format!(r"C:\data\d{d:04}\f{f:03}.txt"));
            }
        }
        let mut idx = index_of(&entries.iter().map(String::as_str).collect::<Vec<_>>());
        let mut disk: Disk = entries.iter().map(|p| (p.clone(), (1, 0, !p.contains('.')))).collect();
        let mut changes = Vec::new();
        for i in 0..10_000 {
            let d = (i * 7) % 2000;
            match i % 4 {
                0 => {
                    let p = format!(r"C:\data\d{d:04}\new{i:05}.txt");
                    disk.insert(p.clone(), (3, 3, false));
                    changes.push(Change::Added(p));
                }
                1 => {
                    let p = format!(r"C:\data\d{d:04}\f{:03}.txt", i % 99);
                    disk.insert(p.clone(), (8, 8, false));
                    changes.push(Change::Modified(p));
                }
                2 => {
                    let p = format!(r"C:\data\d{d:04}\f{:03}.txt", (i + 50) % 99);
                    disk.remove(&p);
                    changes.push(Change::Removed(p));
                }
                _ => {
                    let old = format!(r"C:\data\d{d:04}\f{:03}.txt", (i + 25) % 99);
                    let new = format!(r"C:\data\d{:04}\moved{i:05}.txt", (d + 1) % 2000);
                    if let Some(v) = disk.remove(&old) {
                        disk.insert(new.clone(), v);
                    }
                    changes.push(Change::Renamed(old, new));
                }
            }
        }
        let started = std::time::Instant::now();
        let got = apply(&mut idx, &disk, &changes);
        let ms = started.elapsed().as_millis();
        assert!(is_sorted(&idx));
        let mut want: Vec<&String> = disk.keys().collect();
        want.sort();
        assert_eq!(paths(&idx), want.iter().map(|s| s.as_str()).collect::<Vec<_>>());
        assert!(got.changed >= 7500 && got.scan.is_empty(), "{}", got.changed);
        // A Vec::insert/remove per change would take many seconds here.
        assert!(ms < 3000, "apply took {ms} ms");
    }

    #[test]
    fn merge_keeps_path_order() {
        let a = index_of(&[r"C:\a", r"C:\c", r"C:\e"]).entries;
        let b = index_of(&[r"C:\b", r"C:\d", r"C:\f", r"C:\g"]).entries;
        let merged = merge_by_path(a, b);
        let got: Vec<&str> = merged.iter().map(|e| &*e.path).collect();
        assert_eq!(got, vec![r"C:\a", r"C:\b", r"C:\c", r"C:\d", r"C:\e", r"C:\f", r"C:\g"]);
        assert!(merge_by_path(Vec::new(), Vec::new()).is_empty());
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
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bahth_idx_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// The bytes `save` writes for `paths`.
    fn cache_bytes(paths: &[&str]) -> Vec<u8> {
        let d = temp("bytes");
        index_of(paths).save(&d.join("i.bin")).unwrap();
        let b = fs::read(d.join("i.bin")).unwrap();
        fs::remove_dir_all(&d).unwrap();
        b
    }

    #[test]
    fn damaged_cache_files_load_as_none() {
        let d = temp("damaged");
        let f = d.join("index.bin");
        let good = cache_bytes(TREE_AR);
        // Cut off anywhere: never a panic, and never a partial index.
        for n in 0..good.len() {
            fs::write(&f, &good[..n]).unwrap();
            assert!(Index::load(&f).is_none(), "loaded {n} of {} bytes", good.len());
        }
        // Another version.
        let mut old = good.clone();
        old[..4].copy_from_slice(&7u32.to_le_bytes());
        fs::write(&f, &old).unwrap();
        assert!(Index::load(&f).is_none());
        // A length field claiming a gigantic string: used to allocate it up
        // front (a capacity overflow panic or an out-of-memory abort).
        let mut huge = Vec::new();
        huge.extend_from_slice(&CACHE_VERSION.to_le_bytes());
        huge.extend_from_slice(&1u64.to_le_bytes()); // one entry
        huge.extend_from_slice(&(u64::MAX / 2).to_le_bytes()); // its path length
        huge.extend_from_slice(b"C:\\x");
        fs::write(&f, &huge).unwrap();
        assert!(Index::load(&f).is_none());
        huge[12..20].copy_from_slice(&(3u64 << 30).to_le_bytes());
        fs::write(&f, &huge).unwrap();
        assert!(Index::load(&f).is_none());
        // A billion entries claimed, none there.
        huge[4..12].copy_from_slice(&1_000_000_000u64.to_le_bytes());
        fs::write(&f, &huge).unwrap();
        assert!(Index::load(&f).is_none());
        // Random bytes, and random bytes behind a valid version.
        let mut rng = Rng(0xDEAD_BEEF);
        for i in 0..400 {
            let mut b: Vec<u8> = (0..rng.below(200)).map(|_| rng.below(256) as u8).collect();
            if i % 2 == 0 && b.len() >= 4 {
                b[..4].copy_from_slice(&CACHE_VERSION.to_le_bytes());
            }
            fs::write(&f, &b).unwrap();
            if let Some(idx) = Index::load(&f) {
                assert!(is_sorted(&idx));
            }
        }
        // Flipped bytes in a good file: whatever loads is usable.
        for i in 0..good.len() {
            let mut b = good.clone();
            b[i] ^= 0x80 | rng.below(128) as u8;
            fs::write(&f, &b).unwrap();
            if let Some(mut idx) = Index::load(&f) {
                assert!(is_sorted(&idx));
                for e in idx.entries.clone() {
                    let _ = (e.name(), e.ext());
                    idx.rename_path(&e.path, r"C:\renamed");
                    idx.remove_path(r"C:\renamed");
                }
            }
        }
        assert!(Index::load(&d.join("missing.bin")).is_none());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn unsorted_or_duplicate_entries_are_put_in_order_on_load() {
        let d = temp("unsorted");
        let f = d.join("index.bin");
        let mut idx = index_of(&[r"C:\a", r"C:\a\x.txt", r"C:\b"]);
        idx.entries.reverse();
        idx.entries.push(idx.entries[0].clone());
        idx.save(&f).unwrap();
        let mut got = Index::load(&f).unwrap();
        assert_eq!(paths(&got), vec![r"C:\a", r"C:\a\x.txt", r"C:\b"]);
        // The renames that rely on that order work again.
        assert!(got.rename_path(r"C:\a", r"C:\ب"));
        assert_eq!(paths(&got), vec![r"C:\b", r"C:\ب", r"C:\ب\x.txt"]);
        fs::remove_dir_all(&d).unwrap();
    }

    /// Two copies of the program saving at once (one closing while the next
    /// starts): each writes its own temp file, so whatever the timing the
    /// index on disk is always one whole save and always loads.
    #[test]
    fn two_processes_saving_at_once_leave_a_whole_file() {
        let d = temp("twoprocs");
        let f = d.join("index.bin");
        index_of(TREE).save(&f).unwrap();
        assert_ne!(tmp_path_of(&f, 100), tmp_path_of(&f, 200));
        assert_eq!(tmp_path(&f).file_name().unwrap().to_str().unwrap(), format!("index.{}.tmp", std::process::id()));
        let stop = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|s| {
            let writers: Vec<_> = [100u32, 200]
                .into_iter()
                .map(|pid| {
                    let f = &f;
                    s.spawn(move || {
                        // (No shared lock: these stand for two processes.)
                        let mut saved = 0;
                        for i in 0..150usize {
                            let n = 1 + (pid as usize + i) % 9;
                            let paths: Vec<String> = (0..n).map(|k| format!(r"C:\p{pid}\f{k}.txt")).collect();
                            let idx = index_of(&paths.iter().map(String::as_str).collect::<Vec<_>>());
                            // A save may lose to the other writer or to the
                            // reader (Windows refuses a rename now and then):
                            // it then leaves the file as it was.
                            saved += usize::from(idx.save_through(f, &tmp_path_of(f, pid), &|a, b| fs::rename(a, b)).is_ok());
                        }
                        assert!(saved > 0, "writer {pid} never got a save through");
                    })
                })
                .collect();
            s.spawn(|| {
                while !stop.load(Ordering::Relaxed) {
                    let idx = Index::load(&f).expect("a whole index at all times");
                    assert!(is_sorted(&idx) && !idx.entries.is_empty());
                    // Never a mix of the two writers.
                    let of = |pid: &str| idx.entries.iter().filter(|e| e.path.contains(pid)).count();
                    assert!(of(r"\p100\") == 0 || of(r"\p200\") == 0, "{:?}", paths(&idx));
                }
            });
            for w in writers {
                w.join().unwrap();
            }
            stop.store(true, Ordering::Relaxed);
        });
        fs::remove_dir_all(&d).unwrap();
    }

    /// The program killed in the middle of a save, twenty times over: what
    /// it leaves is a cut-off temp file; `index.bin` is the last finished
    /// save and loads, and the leftovers are cleared away later.
    #[test]
    fn killed_in_the_middle_of_a_save() {
        let d = temp("killed");
        let f = d.join("index.bin");
        index_of(TREE).save(&f).unwrap();
        let whole = fs::read(&f).unwrap();
        for kill in 0..20u32 {
            // What a save by process `kill` had written when it died.
            let cut = whole.len() * (kill as usize + 1) / 21;
            fs::write(tmp_path_of(&f, 5000 + kill), &whole[..cut]).unwrap();
            assert_eq!(Index::load(&f).unwrap().entries.len(), TREE.len(), "after kill {kill}");
        }
        // 0.10.0 and older left `index.tmp`.
        fs::write(d.join("index.tmp"), b"cut off").unwrap();
        // Other files are never touched, whatever their names.
        for other in ["index.bin.bak", "index.x.tmp", "index.12a.tmp", "notes.tmp", "index.tmp.txt"] {
            fs::write(d.join(other), b"keep").unwrap();
        }
        fs::create_dir(d.join("index.777.tmp")).unwrap();
        // Fresh leftovers may belong to a copy that is still closing.
        assert_eq!(remove_stale_tmp(&f, std::time::Duration::from_secs(3600)), 0);
        assert_eq!(remove_stale_tmp(&f, std::time::Duration::ZERO), 21);
        let mut left: Vec<String> = fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, ["index.12a.tmp", "index.777.tmp", "index.bin", "index.bin.bak", "index.tmp.txt", "index.x.tmp", "notes.tmp"]);
        // This process's own temp file is never removed (a save may be using it).
        fs::write(tmp_path(&f), b"in use").unwrap();
        assert_eq!(remove_stale_tmp(&f, std::time::Duration::ZERO), 0);
        // The next save replaces it and leaves nothing behind.
        index_of(&[r"C:\b.txt"]).save(&f).unwrap();
        assert!(!tmp_path(&f).exists());
        assert_eq!(Index::load(&f).unwrap().entries.len(), 1);
        fs::remove_dir_all(&d).unwrap();
    }

    /// Putting the new file in place fails (a scanner holds `index.bin`): it
    /// is tried again; when it keeps failing the old index stays, the temp
    /// file is removed and the error is reported, not swallowed.
    #[test]
    fn a_rename_that_fails_is_retried_then_reported() {
        let d = temp("rename");
        let f = d.join("index.bin");
        index_of(TREE).save(&f).unwrap();
        let tmp = tmp_path_of(&f, 42);
        // Twice refused, then it works.
        let calls = std::cell::Cell::new(0);
        let flaky = |a: &Path, b: &Path| {
            calls.set(calls.get() + 1);
            if calls.get() < 3 { Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "sharing violation")) } else { fs::rename(a, b) }
        };
        index_of(&[r"C:\one.txt"]).save_through(&f, &tmp, &flaky).unwrap();
        assert_eq!((calls.get(), Index::load(&f).unwrap().entries.len()), (3, 1));
        // Always refused.
        calls.set(0);
        let never = |_: &Path, _: &Path| {
            calls.set(calls.get() + 1);
            Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "sharing violation"))
        };
        let err = index_of(&[r"C:\a.txt", r"C:\b.txt"]).save_through(&f, &tmp, &never).unwrap_err();
        assert_eq!(calls.get(), RENAME_TRIES);
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        let text = err.to_string();
        assert!(text.contains("sharing violation") && text.contains("the last saved index is kept"), "{text}");
        assert!(!tmp.exists(), "the temp file is not left behind");
        assert_eq!(Index::load(&f).unwrap().entries.len(), 1, "the old index is untouched");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_failed_save_keeps_the_old_cache() {
        let d = temp("failsave");
        let f = d.join("index.bin");
        index_of(TREE).save(&f).unwrap();
        // The temp file can't be created: the save fails, the cache stays.
        fs::create_dir_all(tmp_path(&f)).unwrap();
        assert!(index_of(&[r"C:\other"]).save(&f).is_err());
        assert_eq!(Index::load(&f).unwrap().entries.len(), TREE.len());
        fs::remove_dir_all(tmp_path(&f)).unwrap();
        // Saves from several threads at once: the file is always whole.
        std::thread::scope(|s| {
            for t in 0..4 {
                let f = &f;
                s.spawn(move || {
                    for i in 0..10 {
                        let n = 1 + (t * 10 + i) % 7;
                        let paths: Vec<String> = (0..n).map(|k| format!(r"C:\t{t}\f{k}.txt")).collect();
                        index_of(&paths.iter().map(String::as_str).collect::<Vec<_>>()).save(f).unwrap();
                    }
                });
            }
            s.spawn(|| {
                for _ in 0..40 {
                    let idx = Index::load(&f).expect("a whole cache at all times");
                    assert!(is_sorted(&idx) && !idx.entries.is_empty());
                }
            });
        });
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn trailing_separators_and_roots_in_changes() {
        let mut idx = index_of(TREE);
        let disk = disk_of(TREE);
        // A folder reported with a trailing separator is still that folder:
        // its contents used to be dropped as if it were a drive root.
        let got = apply(&mut idx, &disk, &[Change::Modified(r"C:\a\".into()), Change::Added(r"C:\a\sub\\".into())]);
        assert_eq!(got.changed, 0);
        assert_eq!(idx.entries.len(), TREE.len());
        // A change "to" a drive root or an empty path is not a deletion of
        // everything on the drive.
        for p in [r"C:\", "", "/", r"\\", "C:"] {
            apply(&mut idx, &disk, &[Change::Removed(p.into()), Change::Renamed(p.into(), r"C:\q".into())]);
            assert_eq!(idx.entries.len(), TREE.len(), "{p:?}");
        }
        // A folder that is really gone, with a trailing separator.
        let mut disk = disk;
        disk.retain(|p, _| !p.starts_with(r"C:\ab"));
        apply(&mut idx, &disk, &[Change::Removed(r"C:\ab\".into())]);
        assert!(idx.find(r"C:\ab").is_none() && idx.find(r"C:\ab\z.txt").is_none());
        assert_eq!(change_path(r"C:\a\\"), Some(r"C:\a"));
        assert_eq!(change_path(r"C:\"), None);
        assert_eq!(change_path("/home/u/"), Some("/home/u"));
    }

    #[test]
    fn folder_deleted_and_made_again_loses_its_old_contents() {
        // Only the folder's removal was reported, then a new folder of the
        // same name appeared: its old files used to stay in the index.
        let mut idx = index_of(TREE);
        let mut disk = disk_of(TREE);
        disk.retain(|p, _| !p.starts_with(r"C:\a\"));
        let got = apply(&mut idx, &disk, &[Change::Removed(r"C:\a".into()), added(r"C:\a")]);
        assert!(idx.find(r"C:\a").is_some() && idx.find(r"C:\a\x.txt").is_none() && idx.find(r"C:\a\sub").is_none());
        assert_eq!(got.scan, vec![r"C:\a".to_string()]);
        // A folder replaced by a file of the same name.
        let mut idx = index_of(TREE);
        let mut disk = disk_of(TREE);
        disk.retain(|p, _| !p.starts_with(r"C:\ab"));
        disk.insert(r"C:\ab".into(), (3, 3, false));
        let got = apply(&mut idx, &disk, &[Change::Modified(r"C:\ab".into())]);
        assert!(got.scan.is_empty());
        assert!(idx.find(r"C:\ab\z.txt").is_none() && !idx.entries[idx.find(r"C:\ab").unwrap()].is_dir);
        // And a file replaced by a folder: its contents need a walk.
        disk.insert(r"C:\b.txt".into(), (0, 0, true));
        let got = apply(&mut idx, &disk, &[Change::Modified(r"C:\b.txt".into())]);
        assert_eq!(got.scan, vec![r"C:\b.txt".to_string()]);
        assert!(is_sorted(&idx));
    }

    #[test]
    fn renamed_into_a_skipped_name_drops_the_contents() {
        let mut idx = index_of(&[r"C:\p\pkgs", r"C:\p\pkgs\m.js", r"C:\p\pkgs\lib", r"C:\p\pkgs\lib\x.js"]);
        let disk = disk_of(&[r"C:\p\node_modules", r"C:\p\node_modules\m.js", r"C:\p\node_modules\lib", r"C:\p\node_modules\lib\x.js"]);
        let got = apply(&mut idx, &disk, &[Change::Renamed(r"C:\p\pkgs".into(), r"C:\p\node_modules".into())]);
        // Like a scan: the folder is listed, the walker never enters it.
        assert_eq!(paths(&idx), vec![r"C:\p\node_modules"]);
        assert!(got.scan.is_empty());
    }

    /// Random batches of changes on a fake disk, applied the way the
    /// watcher reports them, always leave the index equal to a fresh scan.
    #[test]
    fn random_changes_match_a_rescan() {
        const ROOT: &str = r"C:\r";
        const NAMES: &[&str] = &[
            "a", "A", "b.txt", "B.TXT", "a b", "a-b", "ab", "تقرير", "تقرير.pdf", "ملف ١.docx", "node_modules", ".git",
            "x\u{0651}y", "😀.png", "\u{200F}ي", "e\u{301}", "Σ.md",
        ];
        type Disk = std::collections::BTreeMap<String, (u64, i64, bool)>;
        fn under_or_at(p: &str, dir: &str) -> bool {
            p == dir || p.strip_prefix(dir).is_some_and(|r| r.starts_with('\\'))
        }
        fn expected(disk: &Disk, dir: Option<&str>) -> Vec<(String, u64, i64, bool)> {
            disk.iter()
                .filter(|(p, _)| !skipped_path(p) && dir.is_none_or(|d| is_under(p, d)))
                .map(|(p, &(s, m, d))| (p.clone(), s, m, d))
                .collect()
        }
        fn scan(disk: &Disk, dir: &str) -> Vec<Entry> {
            expected(disk, Some(dir))
                .into_iter()
                .map(|(p, size, mtime, is_dir)| Entry {
                    norm_name: normalize(file_name(&p)).into(),
                    path: p.into(),
                    size,
                    mtime,
                    is_dir,
                })
                .collect()
        }

        let mut rng = Rng(0x0BAD_5EED);
        // `Some(cap)`: the changes go through the watcher's bounded queue
        // (`watch_pace::ChangeQueue`), which holds at most `cap` of them and
        // past that asks for a walk of the root instead; and they wait there
        // for a few batches, as while the index is busy.
        for (round, cap) in (0..60).map(|r| (r, None)).chain((0..60).map(|r| (r, Some(1 + r % 12)))) {
            let mut disk = Disk::new();
            let mut idx = Index::default();
            let mut tick = 0i64;
            let mut prev: Vec<Change> = Vec::new();
            let mut queue = cap.map(|c| crate::watch_pace::ChangeQueue::new(c, usize::MAX).under(&[ROOT.to_string()]));
            let mut overflows = 0;
            for batch in 0..25 {
                let mut changes = Vec::new();
                for _ in 0..1 + rng.below(8) {
                    tick += 1;
                    let mut dirs: Vec<String> = disk.iter().filter(|(_, v)| v.2).map(|(p, _)| p.clone()).collect();
                    dirs.push(ROOT.to_string());
                    let all: Vec<String> = disk.keys().cloned().collect();
                    let dir = dirs[rng.below(dirs.len())].clone();
                    let new_path = format!("{dir}\\{}", NAMES[rng.below(NAMES.len())]);
                    match rng.below(9) {
                        // Create a file or a folder.
                        0 | 1 if !disk.contains_key(&new_path) => {
                            let is_dir = rng.below(2) == 0;
                            disk.insert(new_path.clone(), (if is_dir { 0 } else { tick as u64 }, tick, is_dir));
                            changes.push(Change::Added(new_path));
                        }
                        // Modify a file (or touch a folder, maybe with a trailing separator).
                        2 if !all.is_empty() => {
                            let p = all[rng.below(all.len())].clone();
                            let v = disk.get_mut(&p).unwrap();
                            if !v.2 {
                                *v = (v.0 + 1, tick, false);
                            }
                            let p = if v.2 && rng.below(2) == 0 { format!("{p}\\") } else { p };
                            changes.push(Change::Modified(p));
                        }
                        // Delete, reporting the contents first or only the item.
                        3 if !all.is_empty() => {
                            let p = all[rng.below(all.len())].clone();
                            let gone: Vec<String> = disk.keys().filter(|k| under_or_at(k, &p)).cloned().collect();
                            let full = rng.below(2) == 0;
                            for k in gone.iter().rev() {
                                disk.remove(k);
                                if full || *k == p {
                                    changes.push(Change::Removed(k.clone()));
                                }
                            }
                        }
                        // Rename or move, maybe only the letter case.
                        4 | 5 if !all.is_empty() => {
                            let old = all[rng.below(all.len())].clone();
                            let new = if rng.below(3) == 0 {
                                let (parent, name) = old.rsplit_once('\\').unwrap();
                                format!("{parent}\\{}", name.to_uppercase())
                            } else {
                                new_path
                            };
                            if new == old || disk.contains_key(&new) || under_or_at(&new, &old) {
                                continue;
                            }
                            let moved: Vec<String> = disk.keys().filter(|k| under_or_at(k, &old)).cloned().collect();
                            for k in moved {
                                let v = disk.remove(&k).unwrap();
                                disk.insert(format!("{new}{}", &k[old.len()..]), v);
                            }
                            changes.push(Change::Renamed(old, new));
                        }
                        // A folder moved in from elsewhere, with contents nobody reports.
                        6 if !disk.contains_key(&new_path) => {
                            disk.insert(new_path.clone(), (0, tick, true));
                            for i in 0..rng.below(4) {
                                disk.insert(format!("{new_path}\\in{i}.txt"), (i as u64, tick, false));
                            }
                            changes.push(Change::Added(new_path));
                        }
                        // Lost events: the root is walked again.
                        7 if rng.below(5) == 0 => changes.push(Change::Rescan(ROOT.to_string())),
                        _ => {}
                    }
                }
                if let Some(q) = &mut queue {
                    q.extend(changes, []);
                    // The index is busy for up to three batches.
                    if batch % (1 + round % 3) != 0 && batch != 24 {
                        continue;
                    }
                    let taken = q.take();
                    overflows += taken.collapsed.len();
                    assert!(taken.collapsed.is_empty() || taken.collapsed == [ROOT], "{:?}", taken.collapsed);
                    changes = taken.changes;
                }
                let got = idx.apply_changes(&changes, |p| disk.get(p).copied());
                for dir in &got.scan {
                    idx.replace_subtree(dir, scan(&disk, dir));
                }
                let have: Vec<(String, u64, i64, bool)> =
                    idx.entries.iter().map(|e| (e.path.to_string(), e.size, e.mtime, e.is_dir)).collect();
                assert_eq!(have, expected(&disk, None), "round {round} (queue of {cap:?}), after {changes:#?}, before that {prev:#?}");
                prev = changes;
                assert!(idx.entries.iter().all(|e| *e.norm_name == *normalize(e.name())));
            }
            // The small queues did overflow, so the walk path was exercised.
            assert!(cap.is_none_or(|c| c > 6) || overflows > 0, "round {round}: a queue of {cap:?} never overflowed");
        }
    }
}
