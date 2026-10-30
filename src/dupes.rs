//! Finding duplicate files among index entries.
//!
//! Cheap steps first, so a million-entry index stays quick: entries are
//! grouped by size (no disk access), then only sizes shared by two or more
//! files are read. A partial hash of the first and last 64 KB splits those
//! groups further, and only files that still match get a full SHA-256.
//! A file of up to 128 KB is read whole by the partial step, so its partial
//! hash is already a hash of all of it.
//!
//! The quick mode compares only the name (ignoring case) and the size, and
//! reads nothing.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use sha2::{Digest, Sha256};

use crate::index::{Entry, Index};

/// Bytes read from each end of a file for the partial hash.
const EDGE: u64 = 64 * 1024;

/// How files are compared.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Same size and the same SHA-256 of the whole content.
    Content,
    /// Same file name (ignoring case) and the same size; nothing is read.
    NameAndSize,
}

/// Which entries to look at.
#[derive(Clone)]
pub enum Scope {
    /// These files (e.g. the current search results), looked up in the
    /// index; paths not in it are skipped.
    Paths(Arc<Vec<String>>),
    /// Everything under this folder (`C:\Photos\`); empty means the whole index.
    Folder(String),
}

/// Counters shared with the UI while a search runs.
#[derive(Default)]
pub struct Progress {
    pub cancel: AtomicBool,
    /// 0 grouping by size, 1 partial hashes, 2 full hashes.
    pub stage: AtomicUsize,
    pub files_done: AtomicUsize,
    pub files_total: AtomicUsize,
    pub bytes_done: AtomicU64,
    pub bytes_total: AtomicU64,
}

impl Progress {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// One file of a duplicate group.
#[derive(Clone, Debug, PartialEq)]
pub struct DupFile {
    pub path: String,
    pub mtime: i64,
}

/// Files with the same content (or name and size in the quick mode).
#[derive(Clone, Debug)]
pub struct Group {
    pub size: u64,
    pub files: Vec<DupFile>,
}

impl Group {
    /// Space freed by keeping only one copy.
    pub fn wasted(&self) -> u64 {
        self.size * (self.files.len().saturating_sub(1)) as u64
    }
}

#[derive(Default, Debug)]
pub struct Report {
    /// Largest waste first.
    pub groups: Vec<Group>,
    /// Files that could not be read (gone, locked, no permission).
    pub unreadable: usize,
}

impl Report {
    pub fn wasted(&self) -> u64 {
        self.groups.iter().map(Group::wasted).sum()
    }

    pub fn file_count(&self) -> usize {
        self.groups.iter().map(|g| g.files.len()).sum()
    }
}

/// Whether `path` lies under `folder` (which ends with a backslash),
/// ignoring case.
fn under(path: &str, folder: &str) -> bool {
    if folder.is_empty() {
        return true;
    }
    let n = folder.len();
    if path.len() < n {
        return false;
    }
    if path.is_ascii() && folder.is_ascii() {
        return path.as_bytes()[..n].eq_ignore_ascii_case(folder.as_bytes());
    }
    // Lower-casing can change the length, so compare whole characters.
    let mut p = path.chars().flat_map(char::to_lowercase);
    folder.chars().flat_map(char::to_lowercase).all(|c| p.next() == Some(c))
}

/// Groups of entry indices whose files share a size (two or more each).
/// Folders, empty files and files under `min_size` are left out.
pub fn size_groups(entries: &[Entry], ids: impl Iterator<Item = usize>, min_size: u64) -> Vec<Vec<usize>> {
    let min_size = min_size.max(1);
    let mut v: Vec<(u64, usize)> = ids
        .filter_map(|i| entries.get(i).filter(|e| !e.is_dir && e.size >= min_size).map(|e| (e.size, i)))
        .collect();
    v.sort_unstable();
    v.dedup_by_key(|x| x.1);
    let mut out = Vec::new();
    let mut i = 0;
    while i < v.len() {
        let mut j = i + 1;
        while j < v.len() && v[j].0 == v[i].0 {
            j += 1;
        }
        if j - i >= 2 {
            out.push(v[i..j].iter().map(|x| x.1).collect());
        }
        i = j;
    }
    out
}

/// SHA-256 of the first and last 64 KB (the whole file when it is small).
fn partial_hash(path: &str, size: u64, progress: &Progress) -> Option<[u8; 32]> {
    let mut f = File::open(path).ok()?;
    if f.metadata().ok()?.len() != size {
        return None;
    }
    let mut h = Sha256::new();
    if size <= 2 * EDGE {
        let mut buf = Vec::with_capacity(size as usize);
        f.read_to_end(&mut buf).ok()?;
        if buf.len() as u64 != size {
            return None;
        }
        h.update(&buf);
    } else {
        let mut buf = vec![0u8; EDGE as usize];
        f.read_exact(&mut buf).ok()?;
        h.update(&buf);
        f.seek(SeekFrom::Start(size - EDGE)).ok()?;
        f.read_exact(&mut buf).ok()?;
        h.update(&buf);
    }
    progress.bytes_done.fetch_add(size.min(2 * EDGE), Ordering::Relaxed);
    Some(h.finalize().into())
}

/// SHA-256 of the whole file, or `None` when it can't be read or the search
/// was cancelled.
fn full_hash(path: &str, size: u64, progress: &Progress) -> Option<[u8; 32]> {
    let mut f = File::open(path).ok()?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        if progress.cancelled() {
            return None;
        }
        let n = match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        };
        h.update(&buf[..n]);
        total += n as u64;
        progress.bytes_done.fetch_add(n as u64, Ordering::Relaxed);
    }
    (total == size).then(|| h.finalize().into())
}

/// Splits each group by a key; keeps the parts with two or more members.
/// `None` keys (unreadable files) are counted and dropped.
fn split_by<K: std::hash::Hash + Eq + Clone>(
    groups: Vec<Vec<usize>>,
    progress: &Progress,
    unreadable: &mut usize,
    mut key: impl FnMut(usize) -> Option<K>,
) -> Option<Vec<Vec<usize>>> {
    let mut out = Vec::new();
    for g in groups {
        let mut parts: HashMap<K, Vec<usize>> = HashMap::new();
        let mut order: Vec<K> = Vec::new();
        for id in g {
            if progress.cancelled() {
                return None;
            }
            match key(id) {
                Some(k) => {
                    if !parts.contains_key(&k) {
                        order.push(k.clone());
                    }
                    parts.entry(k).or_default().push(id);
                }
                None => *unreadable += 1,
            }
            progress.files_done.fetch_add(1, Ordering::Relaxed);
        }
        for k in order {
            if let Some(p) = parts.remove(&k).filter(|p| p.len() >= 2) {
                out.push(p);
            }
        }
    }
    Some(out)
}

fn start_stage(progress: &Progress, stage: usize, entries: &[Entry], groups: &[Vec<usize>], bytes: impl Fn(u64) -> u64) {
    let files: usize = groups.iter().map(Vec::len).sum();
    let total: u64 = groups.iter().flatten().map(|&i| bytes(entries[i].size)).sum();
    progress.files_done.store(0, Ordering::Relaxed);
    progress.files_total.store(files, Ordering::Relaxed);
    progress.bytes_done.store(0, Ordering::Relaxed);
    progress.bytes_total.store(total, Ordering::Relaxed);
    progress.stage.store(stage, Ordering::Relaxed);
}

/// Finds the duplicate files in `scope`. `None` when cancelled.
pub fn find(index: &Index, scope: &Scope, mode: Mode, min_size: u64, progress: &Progress) -> Option<Report> {
    let entries = &index.entries;
    progress.stage.store(0, Ordering::Relaxed);
    let groups = match scope {
        Scope::Paths(paths) => size_groups(entries, paths.iter().filter_map(|p| index.find(p)), min_size),
        Scope::Folder(dir) => {
            size_groups(entries, (0..entries.len()).filter(|&i| under(&entries[i].path, dir)), min_size)
        }
    };
    if progress.cancelled() {
        return None;
    }
    let mut unreadable = 0;
    let groups = match mode {
        Mode::NameAndSize => split_by(groups, progress, &mut unreadable, |i| {
            Some(entries[i].name().to_lowercase())
        })?,
        Mode::Content => {
            start_stage(progress, 1, entries, &groups, |s| s.min(2 * EDGE));
            let groups = split_by(groups, progress, &mut unreadable, |i| {
                let e = &entries[i];
                partial_hash(&e.path, e.size, progress)
            })?;
            // Small files were read whole: their partial hash is final.
            let (small, big): (Vec<_>, Vec<_>) = groups.into_iter().partition(|g| entries[g[0]].size <= 2 * EDGE);
            start_stage(progress, 2, entries, &big, |s| s);
            let mut big = split_by(big, progress, &mut unreadable, |i| {
                let e = &entries[i];
                full_hash(&e.path, e.size, progress)
            })?;
            if progress.cancelled() {
                return None;
            }
            big.extend(small);
            big
        }
    };
    let mut groups: Vec<Group> = groups
        .into_iter()
        .map(|g| {
            let mut files: Vec<DupFile> = g
                .iter()
                .map(|&i| DupFile { path: entries[i].path.to_string(), mtime: entries[i].mtime })
                .collect();
            files.sort_by(|a, b| a.path.cmp(&b.path));
            Group { size: entries[g[0]].size, files }
        })
        .collect();
    groups.sort_by(|a, b| b.wasted().cmp(&a.wasted()).then_with(|| a.files[0].path.cmp(&b.files[0].path)));
    Some(Report { groups, unreadable })
}

/// Removes deleted files from the report and drops groups left with one file.
pub fn forget_paths(report: &mut Report, gone: &[String]) {
    for g in &mut report.groups {
        g.files.retain(|f| !gone.contains(&f.path));
    }
    report.groups.retain(|g| g.files.len() >= 2);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    struct TempDir(PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp(name: &str) -> TempDir {
        let d = std::env::temp_dir().join(format!("bahth_dupes_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        TempDir(d)
    }

    fn entry(path: &str, size: u64, is_dir: bool) -> Entry {
        Entry { path: path.into(), size, mtime: 0, is_dir, norm_name: "".into() }
    }

    /// Writes the files and returns an index of them, sorted like a real one.
    fn index_of(dir: &Path, files: &[(&str, Vec<u8>)]) -> Index {
        let mut entries: Vec<Entry> = files
            .iter()
            .map(|(name, data)| {
                let p = dir.join(name);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(&p, data).unwrap();
                entry(&p.to_string_lossy(), data.len() as u64, false)
            })
            .collect();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Index { entries, ..Default::default() }
    }

    fn all(_: &Index) -> Scope {
        Scope::Folder(String::new())
    }

    fn names(r: &Report) -> Vec<Vec<String>> {
        r.groups
            .iter()
            .map(|g| g.files.iter().map(|f| f.path.rsplit(['\\', '/']).next().unwrap_or_default().to_string()).collect())
            .collect()
    }

    #[test]
    fn size_groups_skip_folders_empty_and_small_files() {
        let e = vec![
            entry("a", 0, false),
            entry("b", 0, false),
            entry("c", 10, true),
            entry("d", 10, true),
            entry("e", 500, false),
            entry("f", 500, false),
            entry("g", 5000, false),
            entry("h", 5000, false),
            entry("i", 5000, false),
            entry("j", 7000, false),
        ];
        assert_eq!(size_groups(&e, 0..e.len(), 0), vec![vec![4, 5], vec![6, 7, 8]]);
        assert_eq!(size_groups(&e, 0..e.len(), 1024), vec![vec![6, 7, 8]]);
        // The same entry given twice is not its own duplicate.
        assert!(size_groups(&e, [9, 9].into_iter(), 0).is_empty());
    }

    #[test]
    fn same_content_is_grouped_and_different_content_is_not() {
        let d = temp("content");
        let idx = index_of(
            &d.0,
            &[
                ("a.txt", b"hello world".to_vec()),
                ("sub/b.txt", b"hello world".to_vec()),
                ("c.txt", b"hello WORLD".to_vec()),
                ("d.txt", b"other".to_vec()),
            ],
        );
        let r = find(&idx, &all(&idx), Mode::Content, 0, &Progress::default()).unwrap();
        assert_eq!(names(&r), vec![vec!["a.txt", "b.txt"]]);
        assert_eq!(r.wasted(), 11);
        assert_eq!(r.file_count(), 2);
    }

    #[test]
    fn big_files_differing_only_in_the_middle_are_not_duplicates() {
        let d = temp("middle");
        let base = vec![7u8; 400 * 1024];
        let mut middle = base.clone();
        middle[200 * 1024] = 8;
        let mut end = base.clone();
        *end.last_mut().unwrap() = 9;
        let idx = index_of(
            &d.0,
            &[("a.bin", base.clone()), ("b.bin", base.clone()), ("c.bin", middle), ("d.bin", end)],
        );
        let p = Progress::default();
        let r = find(&idx, &all(&idx), Mode::Content, 0, &p).unwrap();
        assert_eq!(names(&r), vec![vec!["a.bin", "b.bin"]]);
        assert_eq!(r.wasted(), 400 * 1024);
        // a, b and c reached the full hash; d was split off by the partial one.
        assert_eq!(p.stage.load(Ordering::Relaxed), 2);
        assert_eq!(p.files_total.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn groups_are_sorted_by_wasted_space_and_min_size_applies() {
        let d = temp("sort");
        let idx = index_of(
            &d.0,
            &[
                ("s1", vec![1; 100]),
                ("s2", vec![1; 100]),
                ("s3", vec![1; 100]),
                ("b1", vec![2; 2000]),
                ("b2", vec![2; 2000]),
            ],
        );
        let r = find(&idx, &all(&idx), Mode::Content, 0, &Progress::default()).unwrap();
        assert_eq!(names(&r), vec![vec!["b1", "b2"], vec!["s1", "s2", "s3"]]);
        assert_eq!(r.wasted(), 2000 + 200);
        let r = find(&idx, &all(&idx), Mode::Content, 1024, &Progress::default()).unwrap();
        assert_eq!(names(&r), vec![vec!["b1", "b2"]]);
    }

    #[test]
    fn scope_limits_the_search() {
        let d = temp("scope");
        let idx = index_of(
            &d.0,
            &[("in/a", b"same".to_vec()), ("in/b", b"same".to_vec()), ("out/c", b"same".to_vec())],
        );
        let folder = format!("{}", d.0.join("IN").join("").to_string_lossy());
        // Folder matching ignores case (Windows paths do).
        let r = find(&idx, &Scope::Folder(folder.to_lowercase()), Mode::Content, 0, &Progress::default()).unwrap();
        assert_eq!(names(&r), vec![vec!["a", "b"]]);
        let paths = vec![idx.entries[0].path.to_string(), idx.entries[2].path.to_string()];
        let r = find(&idx, &Scope::Paths(Arc::new(paths)), Mode::Content, 0, &Progress::default()).unwrap();
        assert_eq!(names(&r), vec![vec!["a", "c"]]);
        let paths = vec![idx.entries[1].path.to_string(), idx.entries[2].path.to_string(), "missing".into()];
        let r = find(&idx, &Scope::Paths(Arc::new(paths)), Mode::Content, 0, &Progress::default()).unwrap();
        assert_eq!(names(&r), vec![vec!["b", "c"]]);
    }

    #[test]
    fn unreadable_and_changed_files_are_skipped() {
        let d = temp("gone");
        let mut idx = index_of(&d.0, &[("a", b"xyz".to_vec()), ("b", b"xyz".to_vec()), ("c", b"xyz".to_vec())]);
        std::fs::remove_file(d.0.join("c")).unwrap();
        // The index says 3 bytes, the file now has 4.
        std::fs::write(d.0.join("b"), b"xyzw").unwrap();
        idx.entries.push(entry(&d.0.join("zz").to_string_lossy(), 3, false));
        let r = find(&idx, &all(&idx), Mode::Content, 0, &Progress::default()).unwrap();
        assert!(r.groups.is_empty());
        // b changed, c and zz are gone.
        assert_eq!(r.unreadable, 3);
    }

    #[test]
    fn quick_mode_compares_name_and_size_only() {
        let d = temp("quick");
        let idx = index_of(
            &d.0,
            &[
                ("x/Report.pdf", b"aaaa".to_vec()),
                ("y/report.PDF", b"bbbb".to_vec()),
                ("z/other.pdf", b"aaaa".to_vec()),
            ],
        );
        let r = find(&idx, &all(&idx), Mode::NameAndSize, 0, &Progress::default()).unwrap();
        assert_eq!(names(&r), vec![vec!["Report.pdf", "report.PDF"]]);
        let r = find(&idx, &all(&idx), Mode::Content, 0, &Progress::default()).unwrap();
        assert_eq!(names(&r), vec![vec!["Report.pdf", "other.pdf"]]);
    }

    #[test]
    fn cancel_stops_the_search() {
        let d = temp("cancel");
        let idx = index_of(&d.0, &[("a", b"same".to_vec()), ("b", b"same".to_vec())]);
        let p = Progress::default();
        p.cancel.store(true, Ordering::Relaxed);
        assert!(find(&idx, &all(&idx), Mode::Content, 0, &p).is_none());
    }

    #[test]
    fn forgetting_deleted_files_drops_single_file_groups() {
        let f = |p: &str| DupFile { path: p.into(), mtime: 0 };
        let mut r = Report {
            groups: vec![
                Group { size: 5, files: vec![f("a"), f("b")] },
                Group { size: 5, files: vec![f("c"), f("d"), f("e")] },
            ],
            unreadable: 0,
        };
        forget_paths(&mut r, &["a".into(), "c".into()]);
        assert_eq!(r.groups.len(), 1);
        assert_eq!(r.groups[0].files, vec![f("d"), f("e")]);
        assert_eq!(r.wasted(), 5);
    }

    #[test]
    fn folder_prefix_ignores_case() {
        assert!(under(r"C:\Photos\a.jpg", r"c:\photos\"));
        assert!(!under(r"C:\Photos2\a.jpg", r"c:\photos\"));
        assert!(under(r"C:\صور\a.jpg", r"c:\صور\"));
        assert!(!under(r"C:\صو", r"c:\صور\"));
        assert!(under("anything", ""));
    }
}
