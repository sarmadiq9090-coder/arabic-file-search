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
        let f = fs::File::open(path).ok()?;
        let cache: CacheFile = bincode::deserialize_from(BufReader::new(f)).ok()?;
        if cache.version != CACHE_VERSION {
            return None;
        }
        let mut index = cache.index;
        fill_norm_names(&mut index.entries);
        Some(index)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        // Saves can come from several threads (scan end, rename, delete);
        // they share one temp file, so they take turns.
        static SAVE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _turn = SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Write to a temp file first so a crash never leaves a broken cache.
        let tmp = path.with_extension("tmp");
        {
            let f = BufWriter::new(fs::File::create(&tmp)?);
            #[derive(Serialize)]
            struct CacheRef<'a> {
                version: u32,
                index: &'a Index,
            }
            bincode::serialize_into(f, &CacheRef { version: CACHE_VERSION, index: self })
                .map_err(std::io::Error::other)?;
        }
        fs::rename(tmp, path)
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
        for c in changes {
            match c {
                Change::Added(p) | Change::Modified(p) | Change::Removed(p) => {
                    if !skipped_path(p) {
                        dirty.push(p.clone());
                    }
                }
                Change::Renamed(old, new) => {
                    let (old_seen, new_seen) = (!skipped_path(old), !skipped_path(new));
                    if old_seen && new_seen && !self.descendants(old).is_empty() {
                        // Paths noted earlier in the batch move along too.
                        for d in &mut dirty {
                            if is_under(d, old) {
                                *d = format!("{new}{}", &d[old.len()..]);
                            }
                        }
                        if self.rename_path(old, new) {
                            out.changed += 1;
                        }
                    }
                    // Moved into the recycle bin and the like: just gone.
                    if old_seen {
                        dirty.push(old.clone());
                    }
                    if new_seen {
                        dirty.push(new.clone());
                    }
                }
                Change::Rescan(root) => out.scan.push(root.clone()),
            }
        }
        dirty.sort_unstable();
        dirty.dedup();

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
                    if (e.size, e.mtime, e.is_dir) != (size, mtime, is_dir) {
                        (e.size, e.mtime, e.is_dir) = (size, mtime, is_dir);
                        out.changed += 1;
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
}
