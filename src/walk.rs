//! The directory walk behind a full index build and `Index::scan_subtree`.
//!
//! Each root gets its own small pool of walk threads, sized by the core count
//! and the kind of disk ([`workers_for`]): several on an SSD, where more
//! requests in flight make a cold walk faster, and two on a spinning disk,
//! where more only adds seeks. The threads share a work-stealing queue of
//! folders ([`pool`]): each takes its own newest folder first (depth first,
//! so the queue stays short) and steals the oldest one of another thread
//! when it runs dry. Threads are started only while folders wait, so a small
//! subtree is walked by its caller alone; their number never passes the
//! root's limit, and all walks together never pass [`MAX_HELPERS`]. So a
//! folder never gets a thread of its own, and the walk no longer fills
//! rayon's global pool (which searches use) with threads waiting on the disk.
//!
//! On Windows a folder is listed with `FindFirstFileExW(FindExInfoBasic,
//! FIND_FIRST_EX_LARGE_FETCH)`: no 8.3 short names, and a large buffer per
//! system call. Each thread keeps its buffers; per entry the only allocation
//! is the path the index keeps, built once from the UTF-16 name.
//!
//! `BAHTH_WALK` picks the walker, for measuring (`ci/walk-bench.ps1`):
//! `legacy` is the walk this replaced (rayon tasks over `std::fs::read_dir`),
//! `dirinfo` lists folders with `GetFileInformationByHandleEx
//! (FileFullDirectoryInfo)` instead; anything else is the default.
//! `BAHTH_WALK_THREADS=N` fixes the threads per root. Every walker gives the
//! same entries; the caller sorts them, so the order they arrive in does not
//! matter.

use std::fmt;
use std::sync::Mutex;
use std::sync::atomic::Ordering;

use crate::index::{Entry, ScanProgress, skip_dir};

/// Entries a thread gathers before adding them to the shared list.
const BATCH: usize = 1024;

/// Walk threads beyond the callers', for all walks running at once (a full
/// build and up to four folder walks of the watcher).
pub const MAX_HELPERS: usize = 32;

/// Highest `BAHTH_WALK_THREADS` taken.
const MAX_THREADS: usize = 64;

/// Which walker lists the folders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// `FindFirstFileExW` with the basic info level and the large fetch.
    Fast,
    /// `GetFileInformationByHandleEx(FileFullDirectoryInfo)`, 64 KB a call.
    DirInfo,
    /// The walk before 1.0: rayon tasks over `std::fs::read_dir`.
    Legacy,
}

impl Mode {
    pub fn parse(v: Option<&str>) -> Mode {
        match v.map(str::trim) {
            Some(v) if v.eq_ignore_ascii_case("legacy") => Mode::Legacy,
            Some(v) if v.eq_ignore_ascii_case("dirinfo") => Mode::DirInfo,
            _ => Mode::Fast,
        }
    }

    /// From `BAHTH_WALK`, read at every walk (the bench switches it).
    pub fn from_env() -> Mode {
        Mode::parse(std::env::var("BAHTH_WALK").ok().as_deref())
    }

    fn name(self) -> &'static str {
        match self {
            Mode::Fast => "fast",
            Mode::DirInfo => "dirinfo",
            Mode::Legacy => "legacy",
        }
    }
}

/// `BAHTH_WALK_THREADS`: a fixed number of threads per root, if valid.
pub fn threads_override(v: Option<&str>) -> Option<usize> {
    v?.trim().parse::<usize>().ok().filter(|&n| n > 0).map(|n| n.min(MAX_THREADS))
}

/// What the disk under a root is, as far as Windows says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disk {
    /// No seek penalty: an SSD (or a flash drive).
    Ssd,
    /// A spinning disk.
    Hdd,
    /// A network path, or a volume that did not answer.
    Unknown,
}

impl Disk {
    fn name(self) -> &'static str {
        match self {
            Disk::Ssd => "ssd",
            Disk::Hdd => "hdd",
            Disk::Unknown => "disk?",
        }
    }
}

/// Walk threads for one root. An SSD answers many requests at once, so a
/// cold walk gains from about two per core (most of them wait on the disk,
/// not the CPU), up to 16; a spinning disk gets two, as more only adds seeks;
/// an unknown disk gets what rayon used to give, one per core, up to 8.
pub fn workers_for(cores: usize, disk: Disk) -> usize {
    let cores = cores.max(1);
    match disk {
        Disk::Ssd => (cores * 2).clamp(4, 16),
        Disk::Hdd => 2,
        Disk::Unknown => cores.clamp(2, 8),
    }
}

/// What a walk did, for the log and the bench.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub mode: Option<Mode>,
    /// Per root: the root, its disk, and the threads that walked it.
    pub roots: Vec<(String, Disk, usize)>,
    /// rayon's threads, for the legacy walk.
    pub rayon_threads: usize,
}

impl Stats {
    /// Threads that walked: every root's, or rayon's for the legacy walk.
    #[cfg_attr(not(test), allow(dead_code))] // the bench and the tests
    pub fn threads(&self) -> usize {
        if self.mode == Some(Mode::Legacy) { self.rayon_threads } else { self.roots.iter().map(|r| r.2).sum() }
    }
}

impl fmt::Display for Stats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mode = self.mode.unwrap_or(Mode::Fast);
        if mode == Mode::Legacy {
            return write!(f, "legacy walk, {} rayon threads", self.rayon_threads);
        }
        write!(f, "{} walk", mode.name())?;
        for (i, (root, disk, threads)) in self.roots.iter().enumerate() {
            write!(f, "{} {root} {} {threads} threads", if i == 0 { ":" } else { "," }, disk.name())?;
        }
        Ok(())
    }
}

/// Walks `roots` (their contents, not the roots themselves), unsorted.
pub fn walk(roots: &[String], capacity: usize, progress: &ScanProgress) -> (Vec<Entry>, Stats) {
    let threads = threads_override(std::env::var("BAHTH_WALK_THREADS").ok().as_deref());
    walk_with(roots, capacity, progress, Mode::from_env(), threads)
}

/// [`walk`] with the walker and the threads per root given.
pub fn walk_with(roots: &[String], capacity: usize, progress: &ScanProgress, mode: Mode, threads: Option<usize>) -> (Vec<Entry>, Stats) {
    if mode == Mode::Legacy {
        let entries = legacy::walk_all(roots, capacity, progress);
        return (entries, Stats { mode: Some(mode), roots: Vec::new(), rayon_threads: rayon::current_num_threads() });
    }
    let out = Mutex::new(Vec::with_capacity(capacity));
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    let plan: Vec<(String, Disk, usize)> = roots
        .iter()
        .map(|r| {
            let disk = disk_of(r);
            (r.clone(), disk, threads.unwrap_or_else(|| workers_for(cores, disk)))
        })
        .collect();
    let used: Vec<usize> = match plan.as_slice() {
        [] => Vec::new(),
        [(root, _, n)] => vec![walk_root(root, *n, mode, &out, progress)],
        _ => std::thread::scope(|s| {
            // One thread per root besides this one; a root whose thread
            // Windows refuses is walked here after the first.
            let others: Vec<_> = plan[1..]
                .iter()
                .map(|(root, _, n)| {
                    let out = &out;
                    std::thread::Builder::new()
                        .name("walk-root".into())
                        .spawn_scoped(s, move || walk_root(root, *n, mode, out, progress))
                        .map_err(|_| (root, *n))
                })
                .collect();
            let mut used = vec![walk_root(&plan[0].0, plan[0].2, mode, &out, progress)];
            for h in others {
                used.push(match h {
                    Ok(h) => h.join().unwrap_or_else(|p| std::panic::resume_unwind(p)),
                    Err((root, n)) => walk_root(root, n, mode, &out, progress),
                });
            }
            used
        }),
    };
    let roots = plan.into_iter().zip(used).map(|((r, d, _), t)| (r, d, t)).collect();
    (out.into_inner().unwrap_or_else(|e| e.into_inner()), Stats { mode: Some(mode), roots, rayon_threads: 0 })
}

/// Walks one root with at most `workers` threads; returns how many ran.
fn walk_root(root: &str, workers: usize, mode: Mode, out: &Mutex<Vec<Entry>>, progress: &ScanProgress) -> usize {
    let (walkers, threads) = pool::run(
        vec![Box::<str>::from(root)],
        workers,
        &progress.cancel,
        || Walker { lister: Lister::new(mode), batch: Vec::with_capacity(BATCH) },
        |w: &mut Walker, dir: Box<str>, kids: &mut Vec<Box<str>>| w.folder(&dir, kids, out, progress),
    );
    for mut w in walkers {
        w.flush(out);
    }
    threads
}

/// One thread's buffers.
struct Walker {
    lister: Lister,
    batch: Vec<Entry>,
}

impl Walker {
    fn folder(&mut self, dir: &str, kids: &mut Vec<Box<str>>, out: &Mutex<Vec<Entry>>, progress: &ScanProgress) {
        let before = self.batch.len();
        let batch = &mut self.batch;
        self.lister.list(dir, &mut |path, name_at, size, mtime, is_dir| {
            let path: Box<str> = path.into();
            if is_dir && !skip_dir(&path[name_at..]) {
                kids.push(path.clone());
            }
            batch.push(Entry { path, size, mtime, is_dir, norm_name: Box::default() });
        });
        progress.files.fetch_add(self.batch.len() - before, Ordering::Relaxed);
        if self.batch.len() >= BATCH {
            self.flush(out);
        }
    }

    fn flush(&mut self, out: &Mutex<Vec<Entry>>) {
        if self.batch.is_empty() {
            return;
        }
        out.lock().unwrap_or_else(|e| e.into_inner()).append(&mut self.batch);
        // A folder of 100,000 files leaves a big batch; let it go.
        if self.batch.capacity() > 8 * BATCH {
            self.batch = Vec::with_capacity(BATCH);
        }
    }
}

/// Called per entry of a folder: the full path, where the name starts in
/// it, the size (0 for folders), the modification time (Unix seconds, 0
/// before 1970) and whether it is a folder. Links are left out.
type Each<'a> = dyn FnMut(&str, usize, u64, i64, bool) + 'a;

/// Puts `dir` and the separator a child's name goes after into `path`, as
/// `PathBuf::push` would (so the paths match the old walker's), and returns
/// where the name starts.
fn start_path(path: &mut String, dir: &str) -> usize {
    path.clear();
    path.push_str(dir);
    if needs_separator(dir) {
        path.push(std::path::MAIN_SEPARATOR);
    }
    path.len()
}

#[cfg(windows)]
fn needs_separator(dir: &str) -> bool {
    let b = dir.as_bytes();
    // `C:` + `x` is `C:x` (relative to the drive's current folder).
    !matches!(b.last(), None | Some(b'\\' | b'/')) && !(b.len() == 2 && b[1] == b':')
}

#[cfg(not(windows))]
fn needs_separator(dir: &str) -> bool {
    !dir.is_empty() && !dir.ends_with('/')
}

enum Lister {
    #[cfg(windows)]
    Find(Box<win::Find>),
    #[cfg(windows)]
    DirInfo(win::DirInfo),
    #[cfg(not(windows))]
    Std(String),
}

impl Lister {
    #[cfg_attr(not(windows), allow(unused_variables))]
    fn new(mode: Mode) -> Lister {
        #[cfg(windows)]
        {
            if mode == Mode::DirInfo { Lister::DirInfo(win::DirInfo::new()) } else { Lister::Find(Box::new(win::Find::new())) }
        }
        #[cfg(not(windows))]
        Lister::Std(String::new())
    }

    fn list(&mut self, dir: &str, each: &mut Each<'_>) {
        match self {
            #[cfg(windows)]
            Lister::Find(l) => l.list(dir, each),
            #[cfg(windows)]
            Lister::DirInfo(l) => l.list(dir, each),
            #[cfg(not(windows))]
            Lister::Std(path) => list_std(path, dir, each),
        }
    }
}

/// The portable lister (Linux tests): the same calls as the old walker,
/// with the path built in a kept buffer.
#[cfg(not(windows))]
fn list_std(path: &mut String, dir: &str, each: &mut Each<'_>) {
    use std::time::UNIX_EPOCH;
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let base = start_path(path, dir);
    for ent in rd.flatten() {
        let Ok(ft) = ent.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        let name = ent.file_name();
        let Some(name) = name.to_str() else { continue };
        path.truncate(base);
        path.push_str(name);
        let (size, mtime) = match ent.metadata() {
            Ok(m) => (
                if ft.is_dir() { 0 } else { m.len() },
                m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64),
            ),
            Err(_) => (0, 0),
        };
        each(path, base, size, mtime, ft.is_dir());
    }
}

/// Unix seconds of a FILETIME, as `SystemTime` and `duration_since` give
/// them (0 before 1970).
#[cfg_attr(not(windows), allow(dead_code))]
fn unix_secs(filetime: u64) -> i64 {
    const EPOCH: i64 = 116_444_736_000_000_000;
    let t = filetime as i64;
    if t < EPOCH { 0 } else { (t - EPOCH) / 10_000_000 }
}

/// Appends UTF-16 `name` to `path`; false (path unchanged in length is not
/// promised) when it is not valid UTF-16, which the old walker skipped too.
#[cfg_attr(not(windows), allow(dead_code))]
fn push_utf16(path: &mut String, name: &[u16]) -> bool {
    path.reserve(name.len());
    // Most names are ASCII: no decoding for those.
    if name.iter().all(|&c| c < 0x80) {
        path.extend(name.iter().map(|&c| c as u8 as char));
        return true;
    }
    for c in char::decode_utf16(name.iter().copied()) {
        match c {
            Ok(c) => path.push(c),
            Err(_) => return false,
        }
    }
    true
}

/// A name surrogate (a symbolic link or a junction): never followed, so a
/// link cannot loop or index a folder twice. Other reparse points (OneDrive
/// files, deduplicated files) are walked, as `FileType::is_symlink` says.
#[cfg_attr(not(windows), allow(dead_code))]
fn is_link(attributes: u32, reparse_tag: u32) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 && reparse_tag & 0x2000_0000 != 0
}

/// One child as a Windows listing gives it.
#[cfg_attr(not(windows), allow(dead_code))]
struct Raw<'a> {
    name: &'a [u16],
    attributes: u32,
    /// The reparse tag of a reparse point (`dwReserved0`, `EaSize`).
    reparse_tag: u32,
    size: u64,
    /// The last write time, as a FILETIME.
    write_time: u64,
}

/// One listed child, from either Windows listing.
#[cfg_attr(not(windows), allow(dead_code))]
fn emit(path: &mut String, base: usize, raw: Raw<'_>, each: &mut Each<'_>) {
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    let Raw { name, attributes, reparse_tag, size, write_time } = raw;
    if name == [b'.' as u16] || name == [b'.' as u16, b'.' as u16] || is_link(attributes, reparse_tag) {
        return;
    }
    path.truncate(base);
    if !push_utf16(path, name) {
        return;
    }
    let is_dir = attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
    each(path, base, if is_dir { 0 } else { size }, unix_secs(write_time), is_dir);
}

#[cfg(windows)]
mod win {
    use super::{Each, Raw, emit, start_path};
    use std::ptr::null_mut;
    use winapi::shared::minwindef::FILETIME;
    use winapi::shared::winerror::ERROR_INVALID_PARAMETER;
    use winapi::um::errhandlingapi::GetLastError;
    use winapi::um::fileapi::{CreateFileW, FindClose, FindFirstFileExW, FindNextFileW, GetFullPathNameW, OPEN_EXISTING};
    use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
    use winapi::um::minwinbase::{FIND_FIRST_EX_LARGE_FETCH, FileFullDirectoryInfo, FindExInfoBasic, FindExSearchNameMatch, WIN32_FIND_DATAW};
    use winapi::um::winbase::{FILE_FLAG_BACKUP_SEMANTICS, GetFileInformationByHandleEx};
    use winapi::um::winnt::{FILE_LIST_DIRECTORY, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE};

    fn filetime(t: &FILETIME) -> u64 {
        (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
    }

    /// `dir` (with `\*` when `star`) as the wide, NUL-ended path the API
    /// takes. Like `std::fs`: as it is when short and absolute, otherwise
    /// made absolute and given the `\\?\` (or `\\?\UNC\`) prefix, so
    /// folders deeper than 260 characters are listed too.
    pub(super) fn wide_path(dir: &str, star: bool, wide: &mut Vec<u16>) {
        const SEP: u16 = b'\\' as u16;
        const ALT: u16 = b'/' as u16;
        const Q: u16 = b'?' as u16;
        const COLON: u16 = b':' as u16;
        const DOT: u16 = b'.' as u16;
        const LEGACY_MAX_PATH: usize = 248;
        wide.clear();
        wide.extend(dir.encode_utf16());
        if star {
            if super::needs_separator(dir) {
                wide.push(SEP);
            }
            wide.push(b'*' as u16);
        }
        wide.push(0);
        if wide.starts_with(&[SEP, SEP, Q, SEP]) || wide.starts_with(&[SEP, Q, Q, SEP]) {
            return;
        }
        if wide.len() < LEGACY_MAX_PATH {
            match wide.as_slice() {
                [d, COLON, 0] | [d, COLON, SEP | ALT, ..] if *d != SEP && *d != ALT => return,
                [SEP | ALT, SEP | ALT, ..] => return,
                _ => {}
            }
        }
        let mut abs = vec![0u16; wide.len() + 64];
        loop {
            // SAFETY: `wide` is NUL-ended; `abs` has the length passed.
            let n = unsafe { GetFullPathNameW(wide.as_ptr(), abs.len() as u32, abs.as_mut_ptr(), null_mut()) } as usize;
            if n == 0 {
                return; // Left as it is: the listing reports the error.
            }
            if n < abs.len() {
                abs.truncate(n);
                break;
            }
            abs.resize(n, 0);
        }
        let (prefix, rest): (&[u16], &[u16]) = match abs.as_slice() {
            [_, COLON, SEP, ..] => (&[SEP, SEP, Q, SEP], &abs),
            [SEP, SEP, DOT, SEP, ..] => (&[SEP, SEP, Q, SEP], &abs[4..]),
            [SEP, SEP, Q, SEP, ..] | [SEP, Q, Q, SEP, ..] => (&[], &abs),
            [SEP, SEP, ..] => (&[SEP, SEP, Q, SEP, b'U' as u16, b'N' as u16, b'C' as u16, SEP], &abs[2..]),
            _ => (&[], &abs),
        };
        wide.clear();
        wide.extend_from_slice(prefix);
        wide.extend_from_slice(rest);
        wide.push(0);
    }

    /// `FindFirstFileExW` / `FindNextFileW`, buffers kept.
    pub struct Find {
        wide: Vec<u16>,
        path: String,
        data: WIN32_FIND_DATAW,
    }

    impl Find {
        pub fn new() -> Find {
            // SAFETY: WIN32_FIND_DATAW is plain data; all zeroes is valid.
            Find { wide: Vec::with_capacity(512), path: String::with_capacity(512), data: unsafe { std::mem::zeroed() } }
        }

        pub fn list(&mut self, dir: &str, each: &mut Each<'_>) {
            wide_path(dir, true, &mut self.wide);
            let base = start_path(&mut self.path, dir);
            let mut first = |flags| {
                // SAFETY: `wide` is NUL-ended; `data` is a WIN32_FIND_DATAW,
                // which FindExInfoBasic fills.
                unsafe {
                    FindFirstFileExW(
                        self.wide.as_ptr(),
                        FindExInfoBasic,
                        (&raw mut self.data).cast(),
                        FindExSearchNameMatch,
                        null_mut(),
                        flags,
                    )
                }
            };
            let mut h = first(FIND_FIRST_EX_LARGE_FETCH);
            // Before Windows 7 the large fetch was refused.
            if h == INVALID_HANDLE_VALUE && unsafe { GetLastError() } == ERROR_INVALID_PARAMETER {
                h = first(0);
            }
            if h == INVALID_HANDLE_VALUE {
                return; // Access denied, gone, or empty: nothing to add.
            }
            loop {
                let d = &self.data;
                let len = d.cFileName.iter().position(|&c| c == 0).unwrap_or(d.cFileName.len());
                let size = (u64::from(d.nFileSizeHigh) << 32) | u64::from(d.nFileSizeLow);
                let raw = Raw {
                    name: &d.cFileName[..len],
                    attributes: d.dwFileAttributes,
                    reparse_tag: d.dwReserved0,
                    size,
                    write_time: filetime(&d.ftLastWriteTime),
                };
                emit(&mut self.path, base, raw, each);
                // SAFETY: `h` is an open find handle; `data` as above.
                if unsafe { FindNextFileW(h, &mut self.data) } == 0 {
                    break;
                }
            }
            // SAFETY: `h` is an open find handle, closed once.
            unsafe { FindClose(h) };
        }
    }

    /// `GetFileInformationByHandleEx(FileFullDirectoryInfo)`, 64 KB a call.
    pub struct DirInfo {
        wide: Vec<u16>,
        path: String,
        /// u64 for the 8-byte alignment the records need.
        buf: Vec<u64>,
    }

    impl DirInfo {
        pub fn new() -> DirInfo {
            DirInfo { wide: Vec::with_capacity(512), path: String::with_capacity(512), buf: vec![0u64; 64 * 1024 / 8] }
        }

        pub fn list(&mut self, dir: &str, each: &mut Each<'_>) {
            wide_path(dir, false, &mut self.wide);
            let base = start_path(&mut self.path, dir);
            // SAFETY: `wide` is NUL-ended; a folder handle for listing.
            let h = unsafe {
                CreateFileW(
                    self.wide.as_ptr(),
                    FILE_LIST_DIRECTORY,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    null_mut(),
                    OPEN_EXISTING,
                    FILE_FLAG_BACKUP_SEMANTICS,
                    null_mut(),
                )
            };
            if h == INVALID_HANDLE_VALUE {
                return;
            }
            let bytes = self.buf.len() * 8;
            // SAFETY: `buf` has `bytes` bytes and is 8-byte aligned.
            while unsafe { GetFileInformationByHandleEx(h, FileFullDirectoryInfo, self.buf.as_mut_ptr().cast(), bytes as u32) } != 0 {
                // SAFETY: reading u8s of a u64 buffer.
                let b: &[u8] = unsafe { std::slice::from_raw_parts(self.buf.as_ptr().cast::<u8>(), bytes) };
                let u32_at = |o: usize| b.get(o..o + 4).map_or(0, |s| u32::from_le_bytes(s.try_into().unwrap_or([0; 4])));
                let u64_at = |o: usize| b.get(o..o + 8).map_or(0, |s| u64::from_le_bytes(s.try_into().unwrap_or([0; 8])));
                // FILE_FULL_DIR_INFO: NextEntryOffset 0, LastWriteTime 24,
                // EndOfFile 40, FileAttributes 56, FileNameLength 60,
                // EaSize 64 (the reparse tag of a reparse point), FileName 68.
                let mut at = 0usize;
                let mut name = Vec::<u16>::new();
                loop {
                    let name_len = u32_at(at + 60) as usize;
                    let Some(raw) = b.get(at + 68..at + 68 + name_len) else { break };
                    name.clear();
                    name.extend(raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])));
                    let raw = Raw { name: &name, attributes: u32_at(at + 56), reparse_tag: u32_at(at + 64), size: u64_at(at + 40), write_time: u64_at(at + 24) };
                    emit(&mut self.path, base, raw, each);
                    let next = u32_at(at) as usize;
                    if next == 0 {
                        break;
                    }
                    at += next;
                }
            }
            // SAFETY: `h` is an open handle, closed once.
            unsafe { CloseHandle(h) };
        }
    }

    /// Whether the volume of drive `letter` has a seek penalty (`Some(true)`:
    /// a spinning disk), asked without admin rights: the volume is opened
    /// with no access, which `IOCTL_STORAGE_QUERY_PROPERTY` allows.
    pub(super) fn seek_penalty(letter: u8) -> Option<bool> {
        use winapi::um::ioapiset::DeviceIoControl;
        const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D_1400;
        const STORAGE_DEVICE_SEEK_PENALTY_PROPERTY: u32 = 7;
        const PROPERTY_STANDARD_QUERY: u32 = 0;
        #[repr(C)]
        struct Query {
            property_id: u32,
            query_type: u32,
            additional: [u8; 4],
        }
        #[repr(C)]
        #[derive(Default)]
        struct SeekPenalty {
            version: u32,
            size: u32,
            incurs_seek_penalty: u8,
        }
        let volume: Vec<u16> = format!(r"\\.\{}:", letter as char).encode_utf16().chain([0]).collect();
        // SAFETY: NUL-ended path; no access asked, so no admin needed.
        let h = unsafe { CreateFileW(volume.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, null_mut(), OPEN_EXISTING, 0, null_mut()) };
        if h == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut query = Query { property_id: STORAGE_DEVICE_SEEK_PENALTY_PROPERTY, query_type: PROPERTY_STANDARD_QUERY, additional: [0; 4] };
        let mut answer = SeekPenalty::default();
        let mut returned = 0u32;
        // SAFETY: in and out buffers live for the call, with their sizes.
        let ok = unsafe {
            DeviceIoControl(
                h,
                IOCTL_STORAGE_QUERY_PROPERTY,
                (&raw mut query).cast(),
                size_of::<Query>() as u32,
                (&raw mut answer).cast(),
                size_of::<SeekPenalty>() as u32,
                &mut returned,
                null_mut(),
            )
        } != 0;
        // SAFETY: an open handle, closed once.
        unsafe { CloseHandle(h) };
        (ok && returned as usize >= size_of::<SeekPenalty>() - 3).then_some(answer.incurs_seek_penalty != 0)
    }
}

/// The drive letter a root is on (`C:\…` or `\\?\C:\…`), if any.
#[cfg_attr(not(windows), allow(dead_code))]
fn drive_letter(root: &str) -> Option<u8> {
    let b = root.strip_prefix(r"\\?\").unwrap_or(root).as_bytes();
    (b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic()).then(|| b[0].to_ascii_uppercase())
}

/// The kind of disk under `root`, asked once per drive.
#[cfg(windows)]
fn disk_of(root: &str) -> Disk {
    static KNOWN: Mutex<[Option<Disk>; 26]> = Mutex::new([None; 26]);
    let Some(letter) = drive_letter(root) else { return Disk::Unknown };
    let slot = usize::from(letter - b'A');
    if let Some(d) = KNOWN.lock().unwrap_or_else(|e| e.into_inner())[slot] {
        return d;
    }
    let d = match win::seek_penalty(letter) {
        Some(false) => Disk::Ssd,
        Some(true) => Disk::Hdd,
        None => Disk::Unknown,
    };
    KNOWN.lock().unwrap_or_else(|e| e.into_inner())[slot] = Some(d);
    d
}

#[cfg(not(windows))]
fn disk_of(_root: &str) -> Disk {
    Disk::Unknown
}

/// The walk this one replaced, kept for `BAHTH_WALK=legacy` (the bench
/// compares the two): a rayon task per folder on the global pool.
mod legacy {
    use super::{Entry, ScanProgress, skip_dir};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::sync::atomic::Ordering;
    use std::time::UNIX_EPOCH;

    pub fn walk_all(roots: &[String], capacity: usize, progress: &ScanProgress) -> Vec<Entry> {
        let out: Mutex<Vec<Entry>> = Mutex::new(Vec::with_capacity(capacity));
        rayon::scope(|s| {
            for root in roots {
                let out = &out;
                s.spawn(move |s| walk(s, PathBuf::from(root), out, progress));
            }
        });
        out.into_inner().unwrap_or_default()
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
}

/// A bounded pool of threads sharing a work-stealing queue of items (here
/// folders), where working on an item may add more.
pub mod pool {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Condvar, Mutex, MutexGuard};
    use std::thread::Scope;
    use std::time::Duration;

    /// Helper threads of all pools running now (see `super::MAX_HELPERS`).
    static HELPERS: AtomicUsize = AtomicUsize::new(0);

    /// How long an idle thread sleeps before looking again (a wake-up can
    /// be missed between its last look and its sleep).
    const NAP: Duration = Duration::from_millis(1);

    struct Shared<'h, T> {
        /// One queue per thread slot: the owner pushes and pops at the back,
        /// thieves take from the front (the oldest, biggest folders).
        decks: Vec<Mutex<VecDeque<T>>>,
        /// Items queued or being worked on; 0 means the walk is over.
        pending: AtomicUsize,
        /// Items queued and not yet taken.
        queued: AtomicUsize,
        /// Thread slots handed out (the caller's is 0).
        started: AtomicUsize,
        /// Threads asleep waiting for work.
        sleepers: AtomicUsize,
        /// A thread panicked: everyone stops.
        stop: AtomicBool,
        nap: Mutex<()>,
        wake: Condvar,
        /// Helper threads running in all pools that share this limit.
        helpers: &'h AtomicUsize,
        global_cap: usize,
        #[cfg(windows)]
        priority: i32,
    }

    fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(|e| e.into_inner())
    }

    impl<T> Shared<'_, T> {
        fn take(&self, slot: usize) -> Option<T> {
            let mut item = lock(&self.decks[slot]).pop_back();
            if item.is_none() && self.queued.load(Ordering::Acquire) > 0 {
                let n = self.started.load(Ordering::Acquire).min(self.decks.len());
                item = (1..n).find_map(|i| lock(&self.decks[(slot + i) % n]).pop_front());
            }
            if item.is_some() {
                self.queued.fetch_sub(1, Ordering::AcqRel);
            }
            item
        }

        fn put(&self, slot: usize, items: &mut Vec<T>) {
            let n = items.len();
            if n == 0 {
                return;
            }
            // Counted before they can be taken, so `pending` never reads 0
            // while one exists.
            self.pending.fetch_add(n, Ordering::AcqRel);
            lock(&self.decks[slot]).extend(items.drain(..));
            self.queued.fetch_add(n, Ordering::AcqRel);
            if self.sleepers.load(Ordering::Acquire) > 0 {
                let _g = lock(&self.nap);
                if n == 1 { self.wake.notify_one() } else { self.wake.notify_all() }
            }
        }

        fn wake_all(&self) {
            let _g = lock(&self.nap);
            self.wake.notify_all();
        }

        /// Another thread is worth starting: more items wait than this
        /// thread (which takes the next itself) and the sleeping ones will
        /// take, and the pool's and the process's limits allow.
        fn reserve_helper(&self) -> Option<usize> {
            if self.queued.load(Ordering::Acquire) <= self.sleepers.load(Ordering::Acquire) + 1 {
                return None;
            }
            let cap = self.decks.len();
            let slot = self.started.fetch_update(Ordering::AcqRel, Ordering::Acquire, |s| (s < cap).then_some(s + 1)).ok()?;
            if self.helpers.fetch_update(Ordering::AcqRel, Ordering::Acquire, |h| (h < self.global_cap).then_some(h + 1)).is_err() {
                // The process has enough walk threads; this slot stays unused.
                return None;
            }
            Some(slot)
        }
    }

    /// Ends an item even when its work panics: `pending` must reach 0, and
    /// after a panic every thread stops instead of waiting for ever.
    struct Done<'a, 'h, T>(&'a Shared<'h, T>);

    impl<T> Drop for Done<'_, '_, T> {
        fn drop(&mut self) {
            if std::thread::panicking() {
                self.0.stop.store(true, Ordering::Release);
            }
            if self.0.pending.fetch_sub(1, Ordering::AcqRel) == 1 || std::thread::panicking() {
                self.0.wake_all();
            }
        }
    }

    /// Works through `roots` and everything `work` adds (into its `Vec`),
    /// on this thread and at most `workers - 1` more, started only while
    /// items wait. Once `cancel` is set the remaining items are dropped
    /// unworked. Returns each thread's state and how many threads ran.
    pub fn run<T, S, I, W>(roots: Vec<T>, workers: usize, cancel: &AtomicBool, init: I, work: W) -> (Vec<S>, usize)
    where
        T: Send,
        S: Send,
        I: Fn() -> S + Sync,
        W: Fn(&mut S, T, &mut Vec<T>) + Sync,
    {
        run_capped(roots, workers, &HELPERS, super::MAX_HELPERS, cancel, init, work)
    }

    /// [`run`] with the counter of helpers shared between pools and its
    /// limit given (the tests use their own).
    pub fn run_capped<T, S, I, W>(
        roots: Vec<T>,
        workers: usize,
        helpers: &AtomicUsize,
        global_cap: usize,
        cancel: &AtomicBool,
        init: I,
        work: W,
    ) -> (Vec<S>, usize)
    where
        T: Send,
        S: Send,
        I: Fn() -> S + Sync,
        W: Fn(&mut S, T, &mut Vec<T>) + Sync,
    {
        let workers = workers.max(1);
        let shared = Shared {
            decks: (0..workers).map(|_| Mutex::new(VecDeque::new())).collect(),
            pending: AtomicUsize::new(roots.len()),
            queued: AtomicUsize::new(roots.len()),
            started: AtomicUsize::new(1),
            sleepers: AtomicUsize::new(0),
            stop: AtomicBool::new(false),
            nap: Mutex::new(()),
            wake: Condvar::new(),
            helpers,
            global_cap,
            #[cfg(windows)]
            priority: current_priority(),
        };
        lock(&shared.decks[0]).extend(roots);
        let ctx = Ctx { shared: &shared, cancel, init: &init, work: &work, states: Mutex::new(Vec::new()) };
        std::thread::scope(|s| worker(s, &ctx, 0));
        let threads = shared.started.load(Ordering::Acquire).min(workers);
        (ctx.states.into_inner().unwrap_or_else(|e| e.into_inner()), threads)
    }

    struct Ctx<'a, T, S, I, W> {
        shared: &'a Shared<'a, T>,
        cancel: &'a AtomicBool,
        init: &'a I,
        work: &'a W,
        states: Mutex<Vec<S>>,
    }

    fn worker<'scope, 'env, T, S, I, W>(s: &'scope Scope<'scope, 'env>, ctx: &'env Ctx<'env, T, S, I, W>, slot: usize)
    where
        T: Send,
        S: Send,
        I: Fn() -> S + Sync,
        W: Fn(&mut S, T, &mut Vec<T>) + Sync,
    {
        let sh = ctx.shared;
        let mut state = (ctx.init)();
        let mut kids = Vec::new();
        while !sh.stop.load(Ordering::Acquire) {
            if let Some(item) = sh.take(slot) {
                let done = Done(sh);
                if !ctx.cancel.load(Ordering::Relaxed) {
                    (ctx.work)(&mut state, item, &mut kids);
                }
                sh.put(slot, &mut kids);
                if let Some(next) = sh.reserve_helper() {
                    let spawned = std::thread::Builder::new().name(format!("walk-{next}")).spawn_scoped(s, move || {
                        #[cfg(windows)]
                        set_priority(ctx.shared.priority);
                        worker(s, ctx, next);
                        ctx.shared.helpers.fetch_sub(1, Ordering::AcqRel);
                    });
                    if spawned.is_err() {
                        sh.helpers.fetch_sub(1, Ordering::AcqRel);
                    }
                }
                drop(done);
                continue;
            }
            if sh.pending.load(Ordering::Acquire) == 0 {
                break;
            }
            let g = lock(&sh.nap);
            sh.sleepers.fetch_add(1, Ordering::AcqRel);
            if sh.queued.load(Ordering::Acquire) == 0 && sh.pending.load(Ordering::Acquire) > 0 && !sh.stop.load(Ordering::Acquire) {
                drop(sh.wake.wait_timeout(g, NAP));
            } else {
                drop(g);
            }
            sh.sleepers.fetch_sub(1, Ordering::AcqRel);
        }
        lock(&ctx.states).push(state);
    }

    /// The calling thread's priority, which the helpers take (a background
    /// walk stays in the background).
    #[cfg(windows)]
    fn current_priority() -> i32 {
        use winapi::um::processthreadsapi::{GetCurrentThread, GetThreadPriority};
        // SAFETY: the pseudo handle of this thread.
        unsafe { GetThreadPriority(GetCurrentThread()) }
    }

    #[cfg(windows)]
    fn set_priority(priority: i32) {
        use winapi::um::processthreadsapi::{GetCurrentThread, SetThreadPriority};
        use winapi::um::winbase::THREAD_PRIORITY_ERROR_RETURN;
        if priority != THREAD_PRIORITY_ERROR_RETURN as i32 && priority != 0 {
            // SAFETY: the pseudo handle of this thread.
            unsafe { SetThreadPriority(GetCurrentThread(), priority) };
        }
    }

    /// Helper threads running now, in every pool.
    #[cfg(test)]
    pub fn helpers_now() -> usize {
        HELPERS.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::time::Duration;

    /// A tree of `width` children per folder, `depth` levels deep: items
    /// are paths like `0/2/1`.
    fn tree_kids(item: &str, width: usize, depth: usize, kids: &mut Vec<String>) {
        if item.split('/').count() < depth {
            kids.extend((0..width).map(|i| format!("{item}/{i}")));
        }
    }

    #[test]
    fn every_item_is_worked_once() {
        for workers in [1, 2, 3, 8] {
            let seen = Mutex::new(Vec::new());
            let (states, threads) = pool::run_capped(
                vec!["r".to_string()],
                workers,
                &AtomicUsize::new(0),
                usize::MAX,
                &AtomicBool::new(false),
                || 0usize,
                |n: &mut usize, item: String, kids: &mut Vec<String>| {
                    *n += 1;
                    tree_kids(&item, 4, 6, kids);
                    seen.lock().unwrap().push(item);
                },
            );
            let seen = seen.into_inner().unwrap();
            // 1 + 4 + 16 + 64 + 256 + 1024 items.
            assert_eq!(seen.len(), 1365, "{workers} workers");
            assert_eq!(seen.iter().collect::<HashSet<_>>().len(), 1365);
            assert_eq!(states.iter().sum::<usize>(), 1365);
            assert_eq!(states.len(), threads);
            assert!((1..=workers).contains(&threads), "{threads} threads for {workers}");
        }
    }

    #[test]
    fn threads_stay_within_the_limit_and_grow_only_with_work() {
        let running = AtomicUsize::new(0);
        let most = AtomicUsize::new(0);
        let (_, threads) = pool::run_capped(
            vec!["r".to_string()],
            4,
            &AtomicUsize::new(0),
            usize::MAX,
            &AtomicBool::new(false),
            || (),
            |_: &mut (), item: String, kids: &mut Vec<String>| {
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                most.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(2));
                tree_kids(&item, 8, 3, kids);
                running.fetch_sub(1, Ordering::SeqCst);
            },
        );
        assert_eq!(threads, 4);
        assert!(most.load(Ordering::SeqCst) <= 4);

        // A chain (one folder inside the next) never has two folders
        // waiting, so the caller walks it alone.
        let (_, threads) = pool::run_capped(
            vec![0u32],
            8,
            &AtomicUsize::new(0),
            usize::MAX,
            &AtomicBool::new(false),
            || (),
            |_: &mut (), depth: u32, kids: &mut Vec<u32>| {
                if depth < 200 {
                    kids.push(depth + 1);
                }
            },
        );
        assert_eq!(threads, 1);
    }

    #[test]
    fn the_process_wide_limit_holds_across_pools() {
        // With no helpers allowed, a pool runs on its caller only.
        let (states, threads) = pool::run_capped(
            vec!["r".to_string()],
            8,
            &AtomicUsize::new(0),
            0,
            &AtomicBool::new(false),
            || 0usize,
            |n: &mut usize, item: String, kids: &mut Vec<String>| {
                *n += 1;
                tree_kids(&item, 6, 4, kids);
            },
        );
        assert_eq!(states.iter().sum::<usize>(), 1 + 6 + 36 + 216);
        assert_eq!(states.len(), 1);
        assert!(threads <= 8);
        // Several pools at once never pass the real limit together.
        let most = AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    pool::run(
                        vec!["r".to_string()],
                        16,
                        &AtomicBool::new(false),
                        || (),
                        |_: &mut (), item: String, kids: &mut Vec<String>| {
                            most.fetch_max(pool::helpers_now(), Ordering::SeqCst);
                            std::thread::sleep(Duration::from_micros(200));
                            tree_kids(&item, 5, 4, kids);
                        },
                    );
                });
            }
        });
        assert!(most.load(Ordering::SeqCst) <= MAX_HELPERS);
    }

    #[test]
    fn cancelling_drops_the_rest() {
        let cancel = AtomicBool::new(false);
        let worked = AtomicUsize::new(0);
        pool::run_capped(
            vec!["r".to_string()],
            4,
            &AtomicUsize::new(0),
            usize::MAX,
            &cancel,
            || (),
            |_: &mut (), item: String, kids: &mut Vec<String>| {
                if worked.fetch_add(1, Ordering::SeqCst) == 10 {
                    cancel.store(true, Ordering::SeqCst);
                }
                tree_kids(&item, 10, 6, kids);
            },
        );
        // A few in flight finish; the 111,111 others are dropped unworked.
        assert!(worked.load(Ordering::SeqCst) < 100, "{}", worked.load(Ordering::SeqCst));
    }

    #[test]
    fn no_roots_and_many_roots() {
        let (states, _) = pool::run(Vec::<u8>::new(), 4, &AtomicBool::new(false), || 0, |n: &mut u32, _: u8, _: &mut Vec<u8>| *n += 1);
        assert_eq!(states.iter().sum::<u32>(), 0);
        let (states, _) = pool::run((0..500u32).collect(), 4, &AtomicBool::new(false), || 0, |n: &mut u32, _: u32, _: &mut Vec<u32>| *n += 1);
        assert_eq!(states.iter().sum::<u32>(), 500);
    }

    #[test]
    fn a_panic_ends_the_pool_instead_of_hanging() {
        let r = std::panic::catch_unwind(|| {
            pool::run_capped(
                vec!["r".to_string()],
                4,
                &AtomicUsize::new(0),
                usize::MAX,
                &AtomicBool::new(false),
                || (),
                |_: &mut (), item: String, kids: &mut Vec<String>| {
                    assert!(item != "r/3/3", "boom");
                    tree_kids(&item, 6, 5, kids);
                },
            )
        });
        assert!(r.is_err());
    }

    #[test]
    fn workers_follow_cores_and_disk() {
        assert_eq!(workers_for(1, Disk::Ssd), 4);
        assert_eq!(workers_for(4, Disk::Ssd), 8);
        assert_eq!(workers_for(8, Disk::Ssd), 16);
        assert_eq!(workers_for(32, Disk::Ssd), 16);
        assert_eq!(workers_for(32, Disk::Hdd), 2);
        assert_eq!(workers_for(1, Disk::Unknown), 2);
        assert_eq!(workers_for(6, Disk::Unknown), 6);
        assert_eq!(workers_for(64, Disk::Unknown), 8);
        assert_eq!(workers_for(0, Disk::Ssd), 4);
    }

    #[test]
    fn switches_are_read() {
        assert_eq!(Mode::parse(None), Mode::Fast);
        assert_eq!(Mode::parse(Some("legacy")), Mode::Legacy);
        assert_eq!(Mode::parse(Some(" LEGACY ")), Mode::Legacy);
        assert_eq!(Mode::parse(Some("dirinfo")), Mode::DirInfo);
        assert_eq!(Mode::parse(Some("fast")), Mode::Fast);
        assert_eq!(Mode::parse(Some("")), Mode::Fast);
        assert_eq!(threads_override(None), None);
        assert_eq!(threads_override(Some("0")), None);
        assert_eq!(threads_override(Some("x")), None);
        assert_eq!(threads_override(Some(" 12 ")), Some(12));
        assert_eq!(threads_override(Some("1000")), Some(MAX_THREADS));
    }

    #[test]
    fn small_helpers() {
        assert_eq!(drive_letter(r"C:\"), Some(b'C'));
        assert_eq!(drive_letter(r"d:\x"), Some(b'D'));
        assert_eq!(drive_letter(r"\\?\e:\x"), Some(b'E'));
        assert_eq!(drive_letter(r"\\server\share"), None);
        assert_eq!(unix_secs(0), 0);
        assert_eq!(unix_secs(116_444_736_000_000_000), 0);
        assert_eq!(unix_secs(116_444_736_000_000_000 + 25_000_000), 2);
        assert_eq!(unix_secs(u64::MAX), 0); // negative as a FILETIME: before 1970
        let mut s = String::from("x\\");
        assert!(push_utf16(&mut s, &"Report.pdf".encode_utf16().collect::<Vec<_>>()));
        assert!(push_utf16(&mut s, &"تقرير".encode_utf16().collect::<Vec<_>>()));
        assert_eq!(s, "x\\Report.pdfتقرير");
        assert!(!push_utf16(&mut s, &[0xD800, b'a' as u16]));
        assert!(is_link(0x400, 0xA000_0003)); // junction
        assert!(is_link(0x400, 0xA000_000C)); // symbolic link
        assert!(!is_link(0x400, 0x9000_601A)); // OneDrive placeholder
        assert!(!is_link(0x10, 0xA000_0003)); // not a reparse point
        let stats = Stats { mode: Some(Mode::Fast), roots: vec![(r"C:\".into(), Disk::Ssd, 16), (r"D:\".into(), Disk::Hdd, 2)], rayon_threads: 0 };
        assert_eq!(stats.threads(), 18);
        assert_eq!(stats.to_string(), r"fast walk: C:\ ssd 16 threads, D:\ hdd 2 threads");
        let legacy = Stats { mode: Some(Mode::Legacy), roots: Vec::new(), rayon_threads: 8 };
        assert_eq!((legacy.threads(), legacy.to_string()), (8, "legacy walk, 8 rayon threads".to_string()));
    }

    /// (path, size, mtime, is_dir) of a walk, sorted.
    fn listing(entries: Vec<Entry>) -> Vec<(String, u64, i64, bool)> {
        let mut v: Vec<_> = entries.into_iter().map(|e| (e.path.into(), e.size, e.mtime, e.is_dir)).collect();
        v.sort();
        v
    }

    /// A tree with everything the walker treats specially.
    fn synthetic_tree(root: &Path) {
        let w = |p: &str, bytes: usize| {
            let p = root.join(p);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, vec![b'x'; bytes]).unwrap();
        };
        // Many folders and files, more than one batch and more than one thread's worth.
        for a in 0..12 {
            for b in 0..12 {
                for f in 0..12 {
                    w(&format!("d{a}/e{b}/f{f}.txt"), (a * 7 + b * 3 + f) % 50);
                }
            }
        }
        w("مستندات/تقرير المبيعات.txt", 12);
        w("مستندات/فرعي/Report.PDF", 3);
        w("名前/ファイル.doc", 1);
        w("with space/ space .txt", 2);
        w("node_modules/pkg/index.js", 5); // the folder is listed, never entered
        w("deep/.git/objects/x", 1);
        w("a/WinSxS/x.dll", 1);
        fs::create_dir_all(root.join("empty/inner")).unwrap();
        // A long path (past 260 characters).
        let mut long = String::new();
        for i in 0..14 {
            long.push_str(&format!("long-folder-name-{i:02}/"));
        }
        w(&format!("{long}end.txt"), 9);
        // A link back to the root: never followed.
        #[cfg(unix)]
        {
            let _ = std::os::unix::fs::symlink(root, root.join("loop"));
            let _ = std::os::unix::fs::symlink(root.join("d0/e0/f0.txt"), root.join("file-link"));
        }
        #[cfg(windows)]
        {
            // A junction needs no admin rights (a symbolic link would).
            let _ = std::process::Command::new("cmd")
                .args(["/c", "mklink", "/J"])
                .arg(root.join("loop"))
                .arg(root)
                .output();
            // Hidden and system files are indexed like any other.
            let _ = std::process::Command::new("attrib").args(["+h", "+s"]).arg(root.join("d0/e0/f1.txt")).output();
        }
    }

    #[test]
    fn the_new_walk_finds_exactly_what_the_old_one_did() {
        let tmp = crate::test_temp::TempPath::in_own_dir("bahth_walk_eq", "tree");
        let root = tmp.to_path_buf();
        fs::create_dir_all(&root).unwrap();
        synthetic_tree(&root);
        let root = root.to_str().unwrap().to_string();
        let progress = ScanProgress::default();
        let (old, _) = walk_with(std::slice::from_ref(&root), 0, &progress, Mode::Legacy, None);
        let old_count = progress.files.load(Ordering::Relaxed);
        let old = listing(old);
        assert!(old.len() > 12 * 12 * 12, "{}", old.len());
        assert!(old.iter().any(|e| e.0.ends_with("end.txt") && e.0.len() > 300 && e.1 == 9));
        assert!(old.iter().any(|e| e.0.ends_with("node_modules") && e.3));
        assert!(!old.iter().any(|e| e.0.contains("pkg") || e.0.contains("objects") || e.0.contains("x.dll")));
        assert!(!old.iter().any(|e| e.0.contains("loop") || e.0.contains("file-link")));
        for (mode, threads) in [(Mode::Fast, None), (Mode::Fast, Some(1)), (Mode::Fast, Some(3)), (Mode::DirInfo, Some(8))] {
            let progress = ScanProgress::default();
            let (new, stats) = walk_with(std::slice::from_ref(&root), 0, &progress, mode, threads);
            assert_eq!(progress.files.load(Ordering::Relaxed), old_count);
            assert!(stats.threads() >= 1);
            assert_eq!(listing(new), old, "{mode:?} with {threads:?} threads");
        }
        // Two roots at once, and a root with a trailing separator.
        let a = format!("{root}{}d1", std::path::MAIN_SEPARATOR);
        let b = format!("{root}{}d2{}", std::path::MAIN_SEPARATOR, std::path::MAIN_SEPARATOR);
        let two = [a, b];
        let (old, _) = walk_with(&two, 0, &ScanProgress::default(), Mode::Legacy, None);
        let (new, stats) = walk_with(&two, 0, &ScanProgress::default(), Mode::Fast, Some(2));
        assert_eq!(stats.roots.len(), 2);
        assert_eq!(listing(new), listing(old));
        // A folder that is not there gives nothing, as before.
        let gone = format!("{root}{}nothing-here", std::path::MAIN_SEPARATOR);
        assert!(walk_with(&[gone], 0, &ScanProgress::default(), Mode::Fast, None).0.is_empty());
    }

    #[test]
    fn a_cancelled_walk_stops() {
        let tmp = crate::test_temp::TempPath::in_own_dir("bahth_walk_cancel", "tree");
        let root = tmp.to_path_buf();
        fs::create_dir_all(root.join("a/b")).unwrap();
        let progress = ScanProgress::default();
        progress.cancel.store(true, Ordering::Relaxed);
        let (found, _) = walk_with(&[root.to_str().unwrap().to_string()], 0, &progress, Mode::Fast, None);
        assert!(found.is_empty());
    }

    /// Private bytes now and at their peak (`K32GetProcessMemoryInfo`).
    #[cfg(windows)]
    fn peak_private_bytes() -> u64 {
        #[repr(C)]
        #[derive(Default)]
        struct Counters {
            cb: u32,
            page_fault_count: u32,
            peak_working_set: usize,
            working_set: usize,
            quota_peak_paged: usize,
            quota_paged: usize,
            quota_peak_non_paged: usize,
            quota_non_paged: usize,
            pagefile_usage: usize,
            peak_pagefile_usage: usize,
            private_usage: usize,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn K32GetProcessMemoryInfo(process: *mut core::ffi::c_void, counters: *mut Counters, cb: u32) -> i32;
            fn GetCurrentProcess() -> *mut core::ffi::c_void;
        }
        let mut c = Counters { cb: size_of::<Counters>() as u32, ..Default::default() };
        // SAFETY: the out parameter lives for the call, with its size.
        let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) } != 0;
        if ok { c.peak_pagefile_usage.max(c.private_usage) as u64 } else { 0 }
    }

    /// A fingerprint of a sorted index (path, size, time, folder).
    #[cfg(windows)]
    fn fingerprint(entries: &[Entry]) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for e in entries {
            (&e.path, e.size, e.mtime, e.is_dir).hash(&mut h);
        }
        h.finish()
    }

    /// One whole index build of `BAHTH_WALK_BENCH_ROOT` (default `C:\`),
    /// exactly as the program does it (walk, sort, names), with the walker
    /// `BAHTH_WALK` picks. Prints one `walk-bench` line for
    /// `ci/walk-bench.ps1`; run one build per process so the peak memory
    /// is that build's.
    #[cfg(windows)]
    #[test]
    #[ignore = "walks a whole drive; run by ci/walk-bench.ps1"]
    fn walk_bench() {
        let root = std::env::var("BAHTH_WALK_BENCH_ROOT").unwrap_or_else(|_| r"C:\".to_string());
        let progress = ScanProgress::default();
        let started = std::time::Instant::now();
        let (idx, stats) = crate::index::Index::build_measured(std::slice::from_ref(&root), &progress);
        let ms = started.elapsed().as_millis();
        println!(
            "walk-bench mode={} entries={} ms={ms} threads={} peak_private_mb={} fingerprint={:016x} detail=\"{stats}\"",
            stats.mode.unwrap_or(Mode::Fast).name(),
            idx.entries.len(),
            stats.threads(),
            peak_private_bytes() / (1024 * 1024),
            fingerprint(&idx.entries),
        );
    }

    /// Walks `BAHTH_WALK_BENCH_ROOT` with the old and the new walkers back
    /// to back and prints how the indexes differ (a live drive changes a
    /// little between two walks: logs, temp files).
    #[cfg(windows)]
    #[test]
    #[ignore = "walks a whole drive three times; run by ci/walk-bench.ps1"]
    fn walk_compare() {
        let root = std::env::var("BAHTH_WALK_BENCH_ROOT").unwrap_or_else(|_| r"C:\".to_string());
        let walk = |mode| listing(walk_with(std::slice::from_ref(&root), 500_000, &ScanProgress::default(), mode, None).0);
        let old = walk(Mode::Legacy);
        for mode in [Mode::Fast, Mode::DirInfo] {
            let new = walk(mode);
            let a: HashSet<_> = old.iter().collect();
            let b: HashSet<_> = new.iter().collect();
            let only_old: Vec<_> = old.iter().filter(|e| !b.contains(e)).collect();
            let only_new: Vec<_> = new.iter().filter(|e| !a.contains(e)).collect();
            println!(
                "walk-compare mode={} legacy={} new={} only_legacy={} only_new={}",
                mode.name(),
                old.len(),
                new.len(),
                only_old.len(),
                only_new.len()
            );
            for e in only_old.iter().take(15) {
                println!("  only legacy: {e:?}");
            }
            for e in only_new.iter().take(15) {
                println!("  only {}: {e:?}", mode.name());
            }
        }
    }
}
