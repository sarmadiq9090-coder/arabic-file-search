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
//!
//! The list is a picture of an earlier moment. Before anything is deleted,
//! [`plan_delete`] looks at the disk again and lets a file go only while a
//! copy that stays is still there and still the same.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use sha2::{Digest, Sha256};

use crate::index::{Entry, Index};

/// Bytes read from each end of a file for the partial hash.
const EDGE: u64 = 64 * 1024;

/// How files are compared.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    /// Same size and the same SHA-256 of the whole content.
    #[default]
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

/// Which file a path leads to: the volume serial number and file ID on
/// Windows, the device and inode elsewhere. Two paths with the same ID are
/// one file (hard links, a junction or symbolic link to a folder, a `subst`
/// drive), however different the paths look.
pub type FileId = (u64, u64);

/// The [`FileId`] of the file at `path`; `None` when it can't be opened.
pub fn file_id(path: &str) -> Option<FileId> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use winapi::um::fileapi::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle};
        // Opened, never read: no download either way.
        let f = crate::cloud::open(std::path::Path::new(path)).ok()?;
        // SAFETY: an all-zero BY_HANDLE_FILE_INFORMATION is valid; the
        // handle stays open for the call and `info` is a valid out pointer.
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        let ok = unsafe { GetFileInformationByHandle(f.as_raw_handle().cast(), &mut info) };
        (ok != 0).then(|| {
            (u64::from(info.dwVolumeSerialNumber), (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow))
        })
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::metadata(path).ok()?;
        Some((m.dev(), m.ino()))
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = path;
        None
    }
}

/// One file of a duplicate group.
#[derive(Clone, Debug, PartialEq)]
pub struct DupFile {
    pub path: String,
    pub mtime: i64,
    /// `None` when it could not be read; such a file counts as its own copy.
    pub id: Option<FileId>,
}

/// Files with the same content (or name and size in the quick mode).
#[derive(Clone, Debug)]
pub struct Group {
    pub size: u64,
    pub files: Vec<DupFile>,
}

impl Group {
    /// The file listed before file `f` that is the very same file (another
    /// path to it), if any.
    pub fn same_file_as(&self, f: usize) -> Option<usize> {
        let id = self.files.get(f)?.id?;
        self.files[..f].iter().position(|o| o.id == Some(id))
    }

    /// How many different files the group has: paths to the same file
    /// count once.
    pub fn copies(&self) -> usize {
        (0..self.files.len()).filter(|&f| self.same_file_as(f).is_none()).count()
    }

    /// Space freed by keeping only one copy. Another path to the same file
    /// takes no space.
    pub fn wasted(&self) -> u64 {
        // Saturating: sizes come from the index cache, which may be damaged.
        self.size.saturating_mul(self.copies().saturating_sub(1) as u64)
    }
}

#[derive(Default, Debug)]
pub struct Report {
    /// Largest waste first.
    pub groups: Vec<Group>,
    /// Files that could not be read (gone, locked, no permission).
    pub unreadable: usize,
    /// How the groups were made. A group of the quick mode says nothing
    /// about the files' contents, so the list is only ever acted on in the
    /// mode that made it.
    pub mode: Mode,
}

impl Report {
    pub fn wasted(&self) -> u64 {
        self.groups.iter().map(Group::wasted).fold(0, u64::saturating_add)
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
/// A cloud-only file is not read (that would download it) unless the user
/// allowed it: it counts as unreadable.
fn partial_hash(path: &str, size: u64, progress: &Progress) -> Option<[u8; 32]> {
    if !crate::cloud::readable(std::path::Path::new(path)) {
        return None;
    }
    let mut f = crate::cloud::open(std::path::Path::new(path)).ok()?;
    if f.metadata().ok()?.len() != size {
        return None;
    }
    let mut h = Sha256::new();
    if size <= 2 * EDGE {
        let mut buf = Vec::with_capacity(size as usize);
        // One byte more than expected is enough to see that it grew.
        (&mut f).take(size + 1).read_to_end(&mut buf).ok()?;
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
    // A file that keeps growing (a log being written, a device) would
    // otherwise be read for as long as it grows: stop one byte past `size`.
    if !crate::cloud::readable(std::path::Path::new(path)) {
        return None;
    }
    let mut f = crate::cloud::open(std::path::Path::new(path)).ok()?.take(size.saturating_add(1));
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
    let total = groups.iter().flatten().map(|&i| bytes(entries[i].size)).fold(0, u64::saturating_add);
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
                .map(|&i| {
                    let path = entries[i].path.to_string();
                    DupFile { id: file_id(&path), path, mtime: entries[i].mtime }
                })
                .collect();
            files.sort_by(|a, b| a.path.cmp(&b.path));
            Group { size: entries[g[0]].size, files }
        })
        // Only paths to one file (hard links, a junction, a subst drive):
        // nothing to free.
        .filter(|g: &Group| g.copies() >= 2)
        .collect();
    groups.sort_by(|a, b| b.wasted().cmp(&a.wasted()).then_with(|| a.files[0].path.cmp(&b.files[0].path)));
    Some(Report { groups, unreadable, mode })
}

/// Removes deleted files from the report and drops groups left with one file.
pub fn forget_paths(report: &mut Report, gone: &[String]) {
    for g in &mut report.groups {
        g.files.retain(|f| !gone.contains(&f.path));
    }
    report.groups.retain(|g| g.copies() >= 2);
}

// ---------------------------------------------------------------- deleting

/// What is at a path right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    pub size: u64,
    /// SHA-256 of the whole content, read now. `None` when it was not read:
    /// the quick mode reads nothing, and a file of another size needs no hash.
    pub hash: Option<[u8; 32]>,
    /// Which file the path leads to now ([`file_id`]).
    pub id: Option<FileId>,
}

/// Why a selected file is left where it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kept {
    /// The list was made in another comparison mode.
    ModeChanged,
    /// Every copy of its group is selected.
    AllSelected,
    /// The file is gone, is no longer a file, or can't be read.
    Gone,
    /// Its size is no longer the one the search saw.
    Changed,
    /// No copy that is not selected is still on disk and still the same.
    NoCopyLeft,
}

/// Which of the selected files may go, and why the others stay.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DeletePlan {
    pub delete: Vec<String>,
    pub kept: Vec<(String, Kept)>,
}

/// Whether what is at a path is still what its group was made of. The
/// content itself is compared between the files, by their hashes.
fn still_fits(seen: &Seen, size: u64, mode: Mode) -> bool {
    seen.size == size && (mode == Mode::NameAndSize || seen.hash.is_some())
}

/// Decides, right before a delete, which of the `selected` files may go.
///
/// The list in `report` is from an earlier moment, so it is not trusted:
/// `look` says what is at a path now (see [`look`]). A selected file goes
/// only when a copy that is NOT selected is still on disk, with the size the
/// search saw and, in the content mode, with the very same content as the
/// selected file has now, and which is not the selected file itself under
/// another path (a hard link, a junction, a `subst` drive: their file IDs
/// are compared, and a copy whose ID can't be read does not count).
/// Otherwise it stays, with the reason. `mode` is the
/// mode the user sees; a list made in another mode is not acted on at all.
/// Paths that are in no group are never deleted.
///
/// The quick mode compares no content, here either: the caller must say so
/// when asking the user.
pub fn plan_delete(
    report: &Report,
    mode: Mode,
    selected: &[String],
    look: &mut dyn FnMut(&str, u64) -> Option<Seen>,
) -> DeletePlan {
    let chosen: HashSet<&str> = selected.iter().map(String::as_str).collect();
    let mut plan = DeletePlan::default();
    for g in &report.groups {
        let (going, staying): (Vec<&DupFile>, Vec<&DupFile>) = g.files.iter().partition(|f| chosen.contains(f.path.as_str()));
        if going.is_empty() {
            continue;
        }
        let whole = if report.mode != mode {
            Some(Kept::ModeChanged)
        } else if staying.is_empty() {
            Some(Kept::AllSelected)
        } else {
            None
        };
        if let Some(why) = whole {
            plan.kept.extend(going.iter().map(|f| (f.path.clone(), why)));
            continue;
        }
        // The copies that stay are looked at when first needed, once each.
        let mut seen: Vec<Option<Option<Seen>>> = vec![None; staying.len()];
        for f in going {
            let why = match look(&f.path, g.size) {
                None => Some(Kept::Gone),
                Some(now) if !still_fits(&now, g.size, mode) => Some(Kept::Changed),
                Some(now) => {
                    let has_copy = staying.iter().zip(seen.iter_mut()).any(|(c, slot)| {
                        slot.get_or_insert_with(|| look(&c.path, g.size))
                            .as_ref()
                            .is_some_and(|c| {
                                still_fits(c, g.size, mode) && c.hash == now.hash && c.id.is_some() && now.id.is_some() && c.id != now.id
                            })
                    });
                    (!has_copy).then_some(Kept::NoCopyLeft)
                }
            };
            match why {
                Some(why) => plan.kept.push((f.path.clone(), why)),
                None => plan.delete.push(f.path.clone()),
            }
        }
    }
    plan
}

/// What is at `path` now, for [`plan_delete`]: its size and, in the content
/// mode while the size is still `size`, the SHA-256 of all of it. `None`
/// when the file is gone, is not a file any more, can't be read, changed
/// while it was read, or the work was cancelled.
pub fn look(path: &str, size: u64, mode: Mode, progress: &Progress) -> Option<Seen> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let len = meta.len();
    let hash = if mode == Mode::Content && len == size { Some(full_hash(path, size, progress)?) } else { None };
    progress.files_done.fetch_add(1, Ordering::Relaxed);
    Some(Seen { size: len, hash, id: file_id(path) })
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
                // Native separators, so paths match the folder scope on Windows too.
                let p = name.split('/').fold(dir.to_path_buf(), |p, part| p.join(part));
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
        let f = |p: &str| DupFile { path: p.into(), mtime: 0, id: None };
        let mut r = Report {
            groups: vec![
                Group { size: 5, files: vec![f("a"), f("b")] },
                Group { size: 5, files: vec![f("c"), f("d"), f("e")] },
            ],
            ..Default::default()
        };
        forget_paths(&mut r, &["a".into(), "c".into()]);
        assert_eq!(r.groups.len(), 1);
        assert_eq!(r.groups[0].files, vec![f("d"), f("e")]);
        assert_eq!(r.wasted(), 5);
    }

    /// The paths of `names` under `dir`, as the index spells them.
    fn paths(dir: &Path, names: &[&str]) -> Vec<String> {
        names.iter().map(|n| dir.join(n).to_string_lossy().into_owned()).collect()
    }

    /// The plan for deleting `names`, read from the real disk.
    fn plan(r: &Report, mode: Mode, dir: &Path, names: &[&str]) -> DeletePlan {
        let p = Progress::default();
        plan_delete(r, mode, &paths(dir, names), &mut |path: &str, size: u64| look(path, size, mode, &p))
    }

    fn kept(dir: &Path, names: &[(&str, Kept)]) -> Vec<(String, Kept)> {
        names.iter().map(|(n, why)| (dir.join(n).to_string_lossy().into_owned(), *why)).collect()
    }

    /// The list is from an earlier moment: the copy that was to stay has
    /// since been deleted or moved in Explorer. The selected file is then
    /// the last copy and must stay.
    #[test]
    fn delete_is_refused_when_the_other_copy_is_gone_from_disk() {
        let d = temp("plan_gone");
        let idx = index_of(&d.0, &[("a.txt", b"same".to_vec()), ("b.txt", b"same".to_vec()), ("x/c.txt", b"other one".to_vec()), ("x/d.txt", b"other one".to_vec())]);
        let r = find(&idx, &all(&idx), Mode::Content, 0, &Progress::default()).unwrap();
        assert_eq!(r.groups.len(), 2);
        // Nothing changed: the selected copy may go.
        assert_eq!(plan(&r, Mode::Content, &d.0, &["b.txt"]), DeletePlan { delete: paths(&d.0, &["b.txt"]), kept: Vec::new() });
        // b is deleted, d is moved away: a and c are the last copies.
        std::fs::remove_file(d.0.join("b.txt")).unwrap();
        std::fs::rename(d.0.join("x").join("d.txt"), d.0.join("moved.txt")).unwrap();
        let c = Path::new("x").join("c.txt");
        let c = c.to_str().unwrap();
        let got = plan(&r, Mode::Content, &d.0, &["a.txt", c]);
        assert_eq!(got.delete, Vec::<String>::new());
        // In the list's order: the group that wastes most comes first.
        assert_eq!(got.kept, kept(&d.0, &[(c, Kept::NoCopyLeft), ("a.txt", Kept::NoCopyLeft)]));
        // A selected file that is itself gone is reported, not "deleted".
        let got = plan(&r, Mode::Content, &d.0, &["b.txt"]);
        assert_eq!((got.delete.len(), got.kept), (0, kept(&d.0, &[("b.txt", Kept::Gone)])));
    }

    /// The copy that was to stay has been edited since the search: same
    /// size or not, it no longer holds what the selected file holds.
    #[test]
    fn delete_is_refused_when_the_other_copy_changed() {
        let d = temp("plan_changed");
        let idx = index_of(&d.0, &[("a", b"12345".to_vec()), ("b", b"12345".to_vec())]);
        let r = find(&idx, &all(&idx), Mode::Content, 0, &Progress::default()).unwrap();
        // Same size, other content.
        std::fs::write(d.0.join("b"), b"1234X").unwrap();
        assert_eq!(plan(&r, Mode::Content, &d.0, &["a"]).kept, kept(&d.0, &[("a", Kept::NoCopyLeft)]));
        // The edited file itself is not a duplicate any more either.
        assert_eq!(plan(&r, Mode::Content, &d.0, &["b"]).kept, kept(&d.0, &[("b", Kept::NoCopyLeft)]));
        // Another size.
        std::fs::write(d.0.join("b"), b"123456").unwrap();
        assert_eq!(plan(&r, Mode::Content, &d.0, &["a"]).kept, kept(&d.0, &[("a", Kept::NoCopyLeft)]));
        let got = plan(&r, Mode::Content, &d.0, &["b"]);
        assert_eq!((got.delete.len(), got.kept), (0, kept(&d.0, &[("b", Kept::Changed)])));
        // Both changed in the same way: still copies of each other.
        std::fs::write(d.0.join("a"), b"abcde").unwrap();
        std::fs::write(d.0.join("b"), b"abcde").unwrap();
        assert_eq!(plan(&r, Mode::Content, &d.0, &["a"]).delete, paths(&d.0, &["a"]));
    }

    /// With three copies, one good copy that stays is enough, whichever it is.
    #[test]
    fn delete_needs_one_good_copy_among_those_that_stay() {
        let d = temp("plan_three");
        let idx = index_of(&d.0, &[("a", b"copy".to_vec()), ("b", b"copy".to_vec()), ("c", b"copy".to_vec())]);
        let r = find(&idx, &all(&idx), Mode::Content, 0, &Progress::default()).unwrap();
        assert_eq!(plan(&r, Mode::Content, &d.0, &["a", "b"]).delete, paths(&d.0, &["a", "b"]));
        // b changed: c still stands in for a.
        std::fs::write(d.0.join("b"), b"edit").unwrap();
        assert_eq!(plan(&r, Mode::Content, &d.0, &["a"]).delete, paths(&d.0, &["a"]));
        // …but not once c is gone too.
        std::fs::remove_file(d.0.join("c")).unwrap();
        let got = plan(&r, Mode::Content, &d.0, &["a"]);
        assert_eq!((got.delete.len(), got.kept), (0, kept(&d.0, &[("a", Kept::NoCopyLeft)])));
    }

    #[test]
    fn delete_is_refused_for_a_whole_group_and_after_a_mode_change() {
        let d = temp("plan_all");
        let idx = index_of(&d.0, &[("a", b"copy".to_vec()), ("b", b"copy".to_vec()), ("k/n.txt", b"1111".to_vec()), ("l/n.txt", b"1111".to_vec())]);
        let r = find(&idx, &all(&idx), Mode::Content, 0, &Progress::default()).unwrap();
        // Every copy of one group selected: none of them goes, and the disk
        // is not even read; the other group is judged on its own.
        let n = Path::new("k").join("n.txt");
        let n = n.to_str().unwrap();
        let mut looked = Vec::new();
        let p = Progress::default();
        let got = plan_delete(&r, Mode::Content, &paths(&d.0, &["a", "b", n]), &mut |path: &str, size: u64| {
            looked.push(path.to_string());
            look(path, size, Mode::Content, &p)
        });
        assert_eq!(got.delete, paths(&d.0, &[n]));
        assert_eq!(got.kept, kept(&d.0, &[("a", Kept::AllSelected), ("b", Kept::AllSelected)]));
        assert!(looked.iter().all(|l| l.ends_with("n.txt")), "{looked:?}");
        // A list made by comparing contents is not acted on in the quick
        // mode, nor the other way round.
        let got = plan_delete(&r, Mode::NameAndSize, &paths(&d.0, &["a"]), &mut |_: &str, _: u64| panic!("the disk is not read"));
        assert_eq!((got.delete.len(), got.kept), (0, kept(&d.0, &[("a", Kept::ModeChanged)])));
        let quick = find(&idx, &all(&idx), Mode::NameAndSize, 0, &Progress::default()).unwrap();
        assert_eq!((quick.mode, r.mode), (Mode::NameAndSize, Mode::Content));
        let got = plan(&quick, Mode::Content, &d.0, &[n]);
        assert_eq!((got.delete.len(), got.kept), (0, kept(&d.0, &[(n, Kept::ModeChanged)])));
        // A path that is in no group is never deleted.
        assert_eq!(plan(&r, Mode::Content, &d.0, &["zz"]), DeletePlan::default());
    }

    /// The quick mode reads no content, before a delete either: it checks
    /// that a file of the same name and size stays.
    #[test]
    fn quick_mode_delete_rechecks_size_only() {
        let d = temp("plan_quick");
        let idx = index_of(&d.0, &[("k/n.txt", b"1111".to_vec()), ("l/n.txt", b"2222".to_vec())]);
        let r = find(&idx, &all(&idx), Mode::NameAndSize, 0, &Progress::default()).unwrap();
        assert_eq!(r.groups.len(), 1);
        let (k, l) = (Path::new("k").join("n.txt"), Path::new("l").join("n.txt"));
        let (k, l) = (k.to_str().unwrap(), l.to_str().unwrap());
        let p = Progress::default();
        let got = plan(&r, Mode::NameAndSize, &d.0, &[k]);
        assert_eq!(got.delete, paths(&d.0, &[k]));
        let seen = look(&paths(&d.0, &[k])[0], 4, Mode::NameAndSize, &p).unwrap();
        assert_eq!((seen.size, seen.hash, seen.id.is_some()), (4, None, true));
        assert_eq!(p.bytes_done.load(Ordering::Relaxed), 0);
        // The file that stays grew: not the same size any more.
        std::fs::write(d.0.join("l").join("n.txt"), b"22222").unwrap();
        assert_eq!(plan(&r, Mode::NameAndSize, &d.0, &[k]).kept, kept(&d.0, &[(k, Kept::NoCopyLeft)]));
        assert_eq!(plan(&r, Mode::NameAndSize, &d.0, &[l]).kept, kept(&d.0, &[(l, Kept::Changed)]));
        // It was replaced by a folder of that name.
        std::fs::remove_file(d.0.join("l").join("n.txt")).unwrap();
        std::fs::create_dir(d.0.join("l").join("n.txt")).unwrap();
        assert_eq!(plan(&r, Mode::NameAndSize, &d.0, &[k]).kept, kept(&d.0, &[(k, Kept::NoCopyLeft)]));
    }

    /// Two names of one file (a hard link here; a junction or a `subst`
    /// drive on Windows) are one file, not two copies: deleting the "copy"
    /// would delete the only one.
    #[cfg(unix)]
    #[test]
    fn two_paths_to_one_file_are_not_duplicates() {
        let d = temp("links");
        let mut idx = index_of(&d.0, &[("a", b"only one".to_vec()), ("dir/x", b"one more".to_vec())]);
        std::fs::hard_link(d.0.join("a"), d.0.join("b")).unwrap();
        // `alias` leads to `dir`, as a junction would.
        std::os::unix::fs::symlink(d.0.join("dir"), d.0.join("alias")).unwrap();
        for n in ["b", "alias/x"] {
            idx.entries.push(entry(&d.0.join(n).to_string_lossy(), 8, false));
        }
        idx.entries.sort_by(|a, b| a.path.cmp(&b.path));
        for mode in [Mode::Content, Mode::NameAndSize] {
            let r = find(&idx, &all(&idx), mode, 0, &Progress::default()).unwrap();
            assert!(r.groups.is_empty(), "{mode:?}: {:?}", names(&r));
        }
        // A real copy next to them: one group, whose second name of `a`
        // is shown as the same file and wastes nothing.
        std::fs::write(d.0.join("c"), b"only one").unwrap();
        idx.entries.push(entry(&d.0.join("c").to_string_lossy(), 8, false));
        idx.entries.sort_by(|a, b| a.path.cmp(&b.path));
        let r = find(&idx, &all(&idx), Mode::Content, 0, &Progress::default()).unwrap();
        assert_eq!(names(&r), vec![vec!["a", "b", "c"]]);
        let g = &r.groups[0];
        assert_eq!((g.copies(), g.wasted(), g.same_file_as(1), g.same_file_as(2)), (2, 8, Some(0), None));
        // Deleting `a` while only its other name `b` stays would delete the
        // file: refused. `c` may go, as `b` (that is, `a`) stays.
        let got = plan(&r, Mode::Content, &d.0, &["a", "c"]);
        assert_eq!((got.delete, got.kept), (paths(&d.0, &["c"]), kept(&d.0, &[("a", Kept::NoCopyLeft)])));
        // Deleting it while `c` stays is fine.
        assert_eq!(plan(&r, Mode::Content, &d.0, &["a"]).delete, paths(&d.0, &["a"]));
        // The same through the folder alias, in a report made from the
        // list alone (as an older list would be).
        let f = |p: &str| DupFile { path: d.0.join(p).to_string_lossy().into_owned(), mtime: 0, id: None };
        let r = Report { groups: vec![Group { size: 8, files: vec![f("alias/x"), f("dir/x")] }], ..Default::default() };
        let got = plan(&r, Mode::Content, &d.0, &["alias/x"]);
        assert_eq!((got.delete.len(), got.kept), (0, kept(&d.0, &[("alias/x", Kept::NoCopyLeft)])));
        // After the delete the group is the one file with two names: gone.
        let mut r = find(&idx, &all(&idx), Mode::Content, 0, &Progress::default()).unwrap();
        forget_paths(&mut r, &paths(&d.0, &["c"]));
        assert!(r.groups.is_empty());
    }

    #[test]
    fn huge_sizes_from_a_damaged_index_do_not_overflow() {
        let f = |p: &str| DupFile { path: p.into(), mtime: 0, id: None };
        let g = Group { size: u64::MAX / 2 + 1, files: vec![f("a"), f("b"), f("c")] };
        assert_eq!(g.wasted(), u64::MAX);
        let r = Report { groups: vec![g.clone(), g], ..Default::default() };
        assert_eq!(r.wasted(), u64::MAX);
        // Entries claiming exabytes: grouped, the files' real sizes differ,
        // so they are unreadable rather than a panic in the byte totals.
        let d = temp("huge");
        let mut idx = index_of(&d.0, &[("a", b"x".to_vec()), ("b", b"x".to_vec()), ("c", b"x".to_vec())]);
        for e in &mut idx.entries {
            e.size = u64::MAX - 1;
        }
        let p = Progress::default();
        let r = find(&idx, &all(&idx), Mode::Content, 0, &p).unwrap();
        assert!(r.groups.is_empty() && r.unreadable == 3);
        let r = find(&idx, &all(&idx), Mode::NameAndSize, 0, &p).unwrap();
        assert!(r.groups.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn endless_files_are_not_read_forever() {
        // /dev/zero never ends; a growing file behaves the same way.
        let p = Progress::default();
        assert_eq!(full_hash("/dev/zero", 3 << 20, &p), None);
        assert!(p.bytes_done.load(Ordering::Relaxed) <= (3 << 20) + 1);
        assert_eq!(partial_hash("/dev/zero", 10, &p), None);
        // A folder where the index says file.
        assert_eq!(full_hash("/", 10, &p), None);
    }

    #[test]
    fn random_scopes_and_entries_never_panic() {
        let mut seed = 0x1357_9BDF_u64;
        let mut below = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        const PIECES: &[&str] = &["C:", "\\", "/", "a", "A", "İ", "ß", "ẞ", "صور", "\u{0651}", "😀", "", ".", "Σ", "ς"];
        fn s(below: &mut dyn FnMut(usize) -> usize, n: usize) -> String {
            (0..below(n)).map(|_| PIECES[below(PIECES.len())]).collect()
        }
        for _ in 0..2000 {
            let (path, folder) = (s(&mut below, 8), s(&mut below, 5));
            let _ = under(&path, &folder);
            assert!(under(&path, ""));
        }
        let entries: Vec<Entry> =
            (0..300).map(|i| entry(&format!("e{i}"), [0, 1, 7, u64::MAX][below(4)], below(3) == 0)).collect();
        let ids: Vec<usize> = (0..600).map(|_| below(400)).collect();
        for g in size_groups(&entries, ids.into_iter(), below(10) as u64) {
            assert!(g.len() >= 2 && g.iter().all(|&i| entries[i].size == entries[g[0]].size && !entries[i].is_dir));
        }
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
