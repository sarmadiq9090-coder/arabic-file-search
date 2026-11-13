//! Cloud-only files: OneDrive "Files On-Demand" and every other cloud-files
//! provider leave a placeholder on the disk whose contents stay online.
//! Reading one makes Windows download it («InstaFind is downloading from
//! OneDrive»), which costs data and disk space and breaks the promise that
//! the files and their index stay on this PC. So nothing the program does on
//! its own (the content index, a search inside documents, duplicates,
//! thumbnails, icons) reads such a file, unless the user switched on
//! "Read contents of cloud-only files" (`Settings::read_cloud_files`).
//! Opening one from the results is the user's own action and still works.
//!
//! The names come from the folder listing, which never downloads anything,
//! so name search lists these files as usual.
//!
//! Two guards: the file's attributes, read without opening it
//! ([`probe`], `GetFileAttributesExW`), decide whether its contents are
//! read at all ([`may_read`]); and the reads themselves open it with
//! `FILE_FLAG_OPEN_NO_RECALL` ([`open`]), so a file that went online-only
//! in between makes the read fail instead of downloading it.

use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`: reading the data downloads it
/// (an online-only OneDrive file).
pub const RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
/// `FILE_ATTRIBUTE_RECALL_ON_OPEN`: even opening it fetches something.
pub const RECALL_ON_OPEN: u32 = 0x0004_0000;
/// `FILE_ATTRIBUTE_OFFLINE`: the data is not available right away (moved
/// to remote storage; older providers and HSM set this one).
pub const OFFLINE: u32 = 0x0000_1000;
/// `FILE_ATTRIBUTE_PINNED`: "Always keep on this device".
#[cfg(test)]
pub const PINNED: u32 = 0x0008_0000;
/// `FILE_ATTRIBUTE_UNPINNED`: "Free up space" was asked for; on its own it
/// says nothing about where the data is now (a downloaded file keeps it
/// until Windows takes the data away, and then it gets RECALL_ON_DATA_ACCESS).
#[cfg(test)]
pub const UNPINNED: u32 = 0x0010_0000;

/// Any of these: the contents are not on this PC.
const NOT_LOCAL: u32 = RECALL_ON_DATA_ACCESS | RECALL_ON_OPEN | OFFLINE;

/// `FILE_FLAG_OPEN_NO_RECALL` for `CreateFileW`: "the file data is
/// requested, but it should continue to be located in remote storage". The
/// same bit as `FILE_ATTRIBUTE_UNPINNED`, a different meaning.
#[cfg(windows)]
const FILE_FLAG_OPEN_NO_RECALL: u32 = 0x0010_0000;

/// The user's choice (Options): read cloud-only files too, which downloads
/// them. Off unless switched on.
static READ_CLOUD: AtomicBool = AtomicBool::new(false);

/// Sets the user's choice (at start-up and when it is switched).
#[cfg_attr(not(windows), allow(dead_code))] // set by the Windows UI
pub fn set_read_cloud(on: bool) {
    READ_CLOUD.store(on, Ordering::Relaxed);
}

pub fn read_cloud() -> bool {
    READ_CLOUD.load(Ordering::Relaxed)
}

/// Whether a file with these attributes has its contents elsewhere: reading
/// them (or, for RECALL_ON_OPEN, opening it) would download them.
pub fn contents_elsewhere(attrs: u32) -> bool {
    attrs & NOT_LOCAL != 0
}

/// Whether the program may read the contents of a file with these
/// attributes on its own: always for a local file, for a cloud-only one only
/// when the user allowed downloads (`read_cloud`).
pub fn may_read(attrs: u32, read_cloud: bool) -> bool {
    read_cloud || !contents_elsewhere(attrs)
}

/// A regular file as it is on disk now: size, modification time (whole
/// seconds since 1970, as `index::Entry` keeps it) and attributes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub size: u64,
    pub mtime: i64,
    pub attrs: u32,
}

impl Stamp {
    /// [`may_read`] with the user's current choice.
    pub fn readable(&self) -> bool {
        may_read(self.attrs, read_cloud())
    }
}

fn from_metadata(m: &std::fs::Metadata) -> Option<Stamp> {
    if !m.is_file() {
        return None;
    }
    let mtime = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);
    #[cfg(windows)]
    let attrs = std::os::windows::fs::MetadataExt::file_attributes(m);
    #[cfg(not(windows))]
    let attrs = 0;
    Some(Stamp { size: m.len(), mtime, attrs })
}

/// FILETIME (100 ns since 1601) as whole seconds since 1970; 0 before 1970,
/// as `SystemTime::duration_since(UNIX_EPOCH)` failing gives elsewhere.
#[cfg_attr(not(windows), allow(dead_code))]
fn filetime_secs(ft: u64) -> i64 {
    const EPOCH: u64 = 116_444_736_000_000_000;
    ft.checked_sub(EPOCH).map_or(0, |d| (d / 10_000_000) as i64)
}

/// The [`Stamp`] of the regular file at `path`, `None` when it is gone or is
/// not a file. On Windows the attributes are read by name
/// (`GetFileAttributesExW`): no handle is opened, so nothing is downloaded.
pub fn probe(path: &Path) -> Option<Stamp> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use winapi::um::fileapi::{GetFileAttributesExW, WIN32_FILE_ATTRIBUTE_DATA};
        use winapi::um::minwinbase::GetFileExInfoStandard;
        use winapi::um::winnt::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT};
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        // SAFETY: an all-zero WIN32_FILE_ATTRIBUTE_DATA is valid, `wide` is
        // NUL-terminated and both live for the call.
        let mut d: WIN32_FILE_ATTRIBUTE_DATA = unsafe { std::mem::zeroed() };
        let ok = unsafe { GetFileAttributesExW(wide.as_ptr(), GetFileExInfoStandard, (&mut d as *mut WIN32_FILE_ATTRIBUTE_DATA).cast()) };
        if ok != 0 {
            let attrs = d.dwFileAttributes;
            if attrs & FILE_ATTRIBUTE_DIRECTORY != 0 {
                return None;
            }
            // A symbolic link answers for itself, not for its target: then
            // as the standard library sees it (which follows the link). A
            // placeholder still online-only is answered here, unopened.
            if attrs & FILE_ATTRIBUTE_REPARSE_POINT == 0 || contents_elsewhere(attrs) {
                let size = (u64::from(d.nFileSizeHigh) << 32) | u64::from(d.nFileSizeLow);
                let ft = (u64::from(d.ftLastWriteTime.dwHighDateTime) << 32) | u64::from(d.ftLastWriteTime.dwLowDateTime);
                return Some(Stamp { size, mtime: filetime_secs(ft), attrs });
            }
        }
        // A link, or a path the call can't take (longer than MAX_PATH).
    }
    from_metadata(&std::fs::metadata(path).ok()?)
}

/// Whether the program may read the contents of the file at `path` on its
/// own now; false when it is gone or unreadable as well.
pub fn readable(path: &Path) -> bool {
    probe(path).is_some_and(|s| s.readable())
}

/// Opens `path` to read its contents. Unless the user allowed downloads, it
/// is opened with `FILE_FLAG_OPEN_NO_RECALL`: a cloud-only file is then not
/// downloaded (the read fails instead). The flag means nothing to a local
/// file. A file system that refuses the flag (`ERROR_INVALID_PARAMETER`, on
/// some network shares) gets a plain open, but only for a file whose
/// attributes say it is local.
pub fn open(path: &Path) -> io::Result<File> {
    #[cfg(windows)]
    if !read_cloud() {
        use std::os::windows::fs::OpenOptionsExt;
        const ERROR_INVALID_PARAMETER: i32 = 87;
        match std::fs::OpenOptions::new().read(true).custom_flags(FILE_FLAG_OPEN_NO_RECALL).open(path) {
            Err(e) if e.raw_os_error() == Some(ERROR_INVALID_PARAMETER) => {
                if !probe(path).is_some_and(|s| !contents_elsewhere(s.attrs)) {
                    return Err(e);
                }
            }
            r => return r,
        }
    }
    File::open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [u32; 5] = [RECALL_ON_DATA_ACCESS, RECALL_ON_OPEN, OFFLINE, PINNED, UNPINNED];

    /// Every combination of the five attributes, with and without the
    /// user's permission to download.
    #[test]
    fn decision_for_every_combination() {
        for bits in 0..1u32 << ALL.len() {
            let attrs: u32 = ALL.iter().enumerate().filter(|(i, _)| bits & (1 << i) != 0).map(|(_, a)| a).sum();
            let elsewhere = attrs & (RECALL_ON_DATA_ACCESS | RECALL_ON_OPEN | OFFLINE) != 0;
            assert_eq!(contents_elsewhere(attrs), elsewhere, "{attrs:#x}");
            assert_eq!(may_read(attrs, false), !elsewhere, "{attrs:#x}");
            assert!(may_read(attrs, true), "{attrs:#x}");
            // The everyday ones do not change it.
            for other in [0x1, 0x2, 0x4, 0x20, 0x80, 0x400, 0x800, 0x2000, 0x4000] {
                assert_eq!(may_read(attrs | other, false), !elsewhere, "{attrs:#x} | {other:#x}");
            }
        }
    }

    #[test]
    fn what_explorer_shows() {
        // A local file, "always keep on this device", downloaded but free to go.
        assert!(may_read(0x20, false) && may_read(0x20 | PINNED, false) && may_read(0x20 | UNPINNED, false));
        // OneDrive online-only (the cloud icon): RECALL_ON_DATA_ACCESS, often with OFFLINE and UNPINNED.
        assert!(!may_read(0x20 | RECALL_ON_DATA_ACCESS, false));
        assert!(!may_read(0x20 | RECALL_ON_DATA_ACCESS | OFFLINE | UNPINNED, false));
        assert!(!may_read(0x400 | RECALL_ON_OPEN, false));
        assert!(may_read(0x20 | RECALL_ON_DATA_ACCESS | OFFLINE | UNPINNED, true));
    }

    #[test]
    fn the_setting_is_off_until_switched_on() {
        // No test switches it on: the default is what every test sees.
        assert!(!read_cloud());
        let s = Stamp { size: 1, mtime: 0, attrs: RECALL_ON_DATA_ACCESS };
        assert!(!s.readable());
        assert!(Stamp { attrs: 0x20, ..s }.readable());
    }

    #[test]
    fn filetimes() {
        assert_eq!(filetime_secs(0), 0);
        assert_eq!(filetime_secs(116_444_736_000_000_000), 0);
        assert_eq!(filetime_secs(116_444_736_000_000_000 + 15_000_000), 1);
        assert_eq!(filetime_secs(133_000_000_000_000_000), 1_655_526_400);
    }

    #[test]
    fn probe_and_open_a_local_file() {
        let dir = std::env::temp_dir().join(format!("bahth_cloud_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.txt");
        std::fs::write(&f, b"hello").unwrap();
        let s = probe(&f).unwrap();
        assert_eq!(s.size, 5);
        assert!(!contents_elsewhere(s.attrs) && readable(&f));
        let m = std::fs::metadata(&f).unwrap();
        let mtime = m.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        assert_eq!(s.mtime, mtime);
        let mut text = String::new();
        io::Read::read_to_string(&mut open(&f).unwrap(), &mut text).unwrap();
        assert_eq!(text, "hello");
        // A folder and a missing file are no files.
        assert_eq!(probe(&dir), None);
        assert_eq!(probe(&dir.join("gone.txt")), None);
        assert!(!readable(&dir.join("gone.txt")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
