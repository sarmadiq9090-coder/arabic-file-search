//! Drives that come and go while the program runs.
//!
//! The index roots are the drives there were at the first scan. A drive
//! plugged in later (a USB stick, an external disk, a new partition) is
//! added as a root by itself and walked alone, without a new full scan
//! ([`plan`] says which). A drive that goes keeps its entries; its results
//! are marked «القرص غير متصل» / "drive not connected" ([`is_absent`]) until
//! it is back, when the watcher walks it again.
//!
//! Only local drives are added: fixed or removable, with a file system
//! Windows itself reads from a disk ([`LOCAL_FILE_SYSTEMS`]). Network
//! drives (a NAS, a mapped share) and the virtual drives of cloud apps
//! (Google Drive and the like, which report other file system names) are
//! never indexed on their own.

use std::sync::atomic::{AtomicU32, Ordering};

/// File systems of local disks. Cloud drives that pose as a fixed disk give
/// their own names (`GoogleDriveFS`, `CLOUDFS`, Dokan or WinFsp names), and
/// are left out.
pub const LOCAL_FILE_SYSTEMS: &[&str] = &["NTFS", "ReFS", "FAT", "FAT12", "FAT16", "FAT32", "exFAT", "UDF"];

/// Indexed drives that are not connected now, one bit per letter (A = 1).
static ABSENT: AtomicU32 = AtomicU32::new(0);

/// The bit of the drive `path` is on (`C:\…`, `\\?\C:\…`), if any.
pub fn drive_bit(path: &str) -> Option<u32> {
    let b = path.strip_prefix(r"\\?\").unwrap_or(path).as_bytes();
    (b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic()).then(|| 1 << (b[0].to_ascii_uppercase() - b'A'))
}

/// The drives of `roots`, as bits.
pub fn mask_of(roots: &[String]) -> u32 {
    roots.iter().filter_map(|r| drive_bit(r)).fold(0, |m, b| m | b)
}

/// `X:\` for bit `i`.
fn root_of(i: u32) -> String {
    format!("{}:\\", (b'A' + i as u8) as char)
}

/// True when `path` is on an indexed drive that is not connected now.
pub fn is_absent(path: &str) -> bool {
    drive_bit(path).is_some_and(|b| ABSENT.load(Ordering::Relaxed) & b != 0)
}

/// Sets the drives [`is_absent`] reports; true when that changed.
pub fn set_absent(mask: u32) -> bool {
    ABSENT.swap(mask, Ordering::Relaxed) != mask
}

/// The note shown with a result on a drive that is not connected.
#[cfg(windows)]
pub fn absent_note() -> &'static str {
    tr!("القرص غير متصل", "drive not connected")
}

/// What a look at the drives found, one bit per letter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Look {
    /// Drives that answer now (a disk in it, readable).
    pub ready: u32,
    /// Of those, the ones worth indexing: local, fixed or removable, with
    /// a local file system.
    pub local: u32,
}

/// What to do after a [`Look`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// New roots to add and walk, `X:\`, in letter order.
    pub add: Vec<String>,
    /// Indexed drives that are not connected (for [`set_absent`]).
    pub absent: u32,
}

/// Compares the drives indexed or being added (`roots`) with `look`.
pub fn plan(roots: &[String], look: Look) -> Plan {
    let have = mask_of(roots);
    let new = look.local & look.ready & !have;
    Plan { add: (0..26).filter(|i| new & (1 << i) != 0).map(root_of).collect(), absent: have & !look.ready }
}

/// Roots of the old index whose entries a full scan keeps: indexed drives
/// that are not connected now (unplugged, or a card reader with no card),
/// which the scan cannot see. Their entries stay, marked, instead of going.
pub fn kept_roots(old: &[String], look: Look) -> Vec<String> {
    old.iter().filter(|r| drive_bit(r).is_some_and(|b| look.ready & b == 0)).cloned().collect()
}

/// True for a file system name of a local disk.
pub fn local_file_system(name: &str) -> bool {
    LOCAL_FILE_SYSTEMS.iter().any(|f| f.eq_ignore_ascii_case(name.trim()))
}

/// Looks at every drive letter Windows has now: whether it answers, and
/// whether it is a local fixed or removable disk. A few calls per drive,
/// with Windows' "insert a disk" box turned off for empty card readers.
#[cfg(windows)]
pub fn look() -> Look {
    use winapi::um::errhandlingapi::SetThreadErrorMode;
    use winapi::um::fileapi::{GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW};
    use winapi::um::winbase::{DRIVE_FIXED, DRIVE_REMOVABLE, SEM_FAILCRITICALERRORS};
    let letters = unsafe { GetLogicalDrives() };
    let mut old_mode = 0;
    // SAFETY: plain Win32 calls; buffers live for the calls.
    unsafe { SetThreadErrorMode(SEM_FAILCRITICALERRORS, &mut old_mode) };
    let mut out = Look::default();
    for i in 0..26u32 {
        let bit = 1 << i;
        if letters & bit == 0 {
            continue;
        }
        let root: Vec<u16> = root_of(i).encode_utf16().chain([0]).collect();
        let kind = unsafe { GetDriveTypeW(root.as_ptr()) };
        let mut fs = [0u16; 64];
        let ready = unsafe {
            GetVolumeInformationW(root.as_ptr(), std::ptr::null_mut(), 0, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), fs.as_mut_ptr(), fs.len() as u32)
        } != 0;
        if !ready {
            continue;
        }
        out.ready |= bit;
        let len = fs.iter().position(|&c| c == 0).unwrap_or(fs.len());
        if (kind == DRIVE_FIXED || kind == DRIVE_REMOVABLE) && local_file_system(&String::from_utf16_lossy(&fs[..len])) {
            out.local |= bit;
        }
    }
    unsafe { SetThreadErrorMode(old_mode, std::ptr::null_mut()) };
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(r: &[&str]) -> Vec<String> {
        r.iter().map(|s| s.to_string()).collect()
    }

    const C: u32 = 1 << 2;
    const D: u32 = 1 << 3;
    const E: u32 = 1 << 4;
    const F: u32 = 1 << 5;
    const Q: u32 = 1 << 16;
    const Z: u32 = 1 << 25;

    #[test]
    fn drive_bits() {
        assert_eq!(drive_bit(r"C:\Users\x.txt"), Some(C));
        assert_eq!(drive_bit(r"e:\"), Some(E));
        assert_eq!(drive_bit(r"\\?\D:\long\path"), Some(D));
        assert_eq!(drive_bit(r"\\server\share\a"), None);
        assert_eq!(drive_bit("relative"), None);
        assert_eq!(drive_bit(""), None);
        assert_eq!(mask_of(&roots(&[r"C:\", r"D:\", r"\\nas\share"])), C | D);
    }

    #[test]
    fn a_new_local_drive_is_added_alone() {
        let indexed = roots(&[r"C:\", r"D:\"]);
        // Nothing new.
        assert_eq!(plan(&indexed, Look { ready: C | D, local: C | D }), Plan::default());
        // A USB stick as E: is added; nothing else changes.
        assert_eq!(plan(&indexed, Look { ready: C | D | E, local: C | D | E }), Plan { add: vec![r"E:\".into()], absent: 0 });
        // Two at once, in letter order.
        assert_eq!(plan(&indexed, Look { ready: C | D | Z | E, local: C | D | Z | E }).add, vec![r"E:\".to_string(), r"Z:\".to_string()]);
        // Already being added (its walk runs): not again.
        let adding = roots(&[r"C:\", r"D:\", r"E:\"]);
        assert!(plan(&adding, Look { ready: C | D | E, local: C | D | E }).add.is_empty());
    }

    #[test]
    fn network_cloud_and_empty_drives_are_not_added() {
        let indexed = roots(&[r"C:\"]);
        // F: answers but is not local (a mapped NAS share, a cloud drive).
        assert!(plan(&indexed, Look { ready: C | F, local: C }).add.is_empty());
        // An empty card reader: local kind, but nothing to read.
        assert!(plan(&indexed, Look { ready: C, local: C | E }).add.is_empty());
        assert!(local_file_system("NTFS") && local_file_system("exfat") && local_file_system(" FAT32 "));
        assert!(!local_file_system("GoogleDriveFS") && !local_file_system("CLOUDFS") && !local_file_system("Dokan"));
        assert!(!local_file_system(""));
    }

    #[test]
    fn a_removed_drive_is_absent_until_it_is_back() {
        let indexed = roots(&[r"C:\", r"E:\"]);
        let gone = plan(&indexed, Look { ready: C, local: C });
        assert_eq!(gone, Plan { add: Vec::new(), absent: E });
        // Its entries stay; it is only reported absent. Back: not absent,
        // and not added again (the watcher walks it).
        let back = plan(&indexed, Look { ready: C | E, local: C | E });
        assert_eq!(back, Plan::default());
        // Only indexed drives count as absent.
        assert_eq!(plan(&roots(&[r"C:\"]), Look { ready: C, local: C }).absent, 0);
    }

    #[test]
    fn a_full_scan_keeps_drives_that_are_not_connected() {
        let old = roots(&[r"C:\", r"E:\", r"F:\", r"\\nas\share"]);
        // E: is out (unplugged, or its reader is empty); C: and F: are scanned.
        assert_eq!(kept_roots(&old, Look { ready: C | F, local: C | F }), roots(&[r"E:\"]));
        // Everything connected: nothing kept.
        assert!(kept_roots(&old, Look { ready: C | E | F, local: C | E | F }).is_empty());
    }

    #[test]
    fn results_on_an_absent_drive_are_marked() {
        // One test owns the shared mask, with a letter no other test uses.
        assert!(set_absent(Q));
        assert!(!set_absent(Q));
        assert!(is_absent(r"Q:\photos\a.jpg") && is_absent(r"q:\b") && is_absent(r"\\?\Q:\c"));
        assert!(!is_absent(r"C:\a") && !is_absent(r"\\server\share\a"));
        assert!(set_absent(0));
        assert!(!is_absent(r"Q:\photos\a.jpg"));
    }
}
