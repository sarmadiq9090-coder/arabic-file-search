//! Windows shell helpers: opening, revealing and recycling files, file-type
//! icons and local time formatting.

use std::os::windows::process::CommandExt;
use winapi::um::minwinbase::SYSTEMTIME;
use winapi::um::shellapi::{
    SHFILEINFOW, SHGFI_SMALLICON, SHGFI_SYSICONINDEX, SHGFI_USEFILEATTRIBUTES, SHGetFileInfoW, ShellExecuteW,
};
use winapi::um::timezoneapi::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use winapi::um::winnt::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
use winapi::um::winuser::SW_SHOWNORMAL;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Opens a file with its default program.
pub fn open(path: &str) -> bool {
    let (verb, file) = (wide("open"), wide(path));
    let r = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    r as isize > 32
}

/// Moves a file or folder to the Recycle Bin, with Windows' own
/// confirmation. True if it is gone.
pub fn recycle(path: &str) -> bool {
    use winapi::um::shellapi::{FO_DELETE, FOF_ALLOWUNDO, SHFILEOPSTRUCTW, SHFileOperationW};
    // The list of paths ends with two NULs.
    let mut from: Vec<u16> = path.encode_utf16().collect();
    from.extend([0, 0]);
    let mut op: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
    op.wFunc = FO_DELETE as u32;
    op.pFrom = from.as_ptr();
    op.fFlags = FOF_ALLOWUNDO;
    let r = unsafe { SHFileOperationW(&mut op) };
    r == 0 && op.fAnyOperationsAborted == 0 && !std::path::Path::new(path).exists()
}

/// Moves several files to the Recycle Bin in one operation, without asking
/// again (the caller has confirmed), but with Windows' warning when a file
/// can't go to the Recycle Bin. Returns the paths that are gone.
pub fn recycle_many(paths: &[String]) -> Vec<String> {
    use winapi::um::shellapi::{FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_WANTNUKEWARNING, SHFILEOPSTRUCTW, SHFileOperationW};
    // NUL-separated paths, ending with two NULs.
    let mut from: Vec<u16> = Vec::new();
    for p in paths {
        from.extend(p.encode_utf16());
        from.push(0);
    }
    from.push(0);
    let mut op: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
    op.wFunc = FO_DELETE as u32;
    op.pFrom = from.as_ptr();
    op.fFlags = FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_WANTNUKEWARNING;
    unsafe { SHFileOperationW(&mut op) };
    paths.iter().filter(|p| !std::path::Path::new(p.as_str()).exists()).cloned().collect()
}

/// Opens Explorer with the file selected.
pub fn reveal(path: &str) {
    let _ = std::process::Command::new("explorer.exe").raw_arg(format!("/select,\"{path}\"")).spawn();
}

fn query_icon(name: &str, attrs: u32, flags: u32) -> (usize, i32) {
    let name = wide(name);
    let mut info: SHFILEINFOW = unsafe { std::mem::zeroed() };
    let r = unsafe {
        SHGetFileInfoW(name.as_ptr(), attrs, &mut info, std::mem::size_of::<SHFILEINFOW>() as u32, flags)
    };
    (r, info.iIcon)
}

/// The system's small-icon image list, shared by every program.
pub fn system_image_list() -> usize {
    query_icon(".txt", FILE_ATTRIBUTE_NORMAL, SHGFI_SYSICONINDEX | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES).0
}

/// Icon index for an extension, or for folders when `ext` is "/".
pub fn type_icon(ext: &str) -> i32 {
    let flags = SHGFI_SYSICONINDEX | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES;
    if ext == "/" {
        query_icon("folder", FILE_ATTRIBUTE_DIRECTORY, flags).1
    } else {
        query_icon(&format!("file.{ext}"), FILE_ATTRIBUTE_NORMAL, flags).1
    }
}

fn local_time(secs: i64) -> Option<SYSTEMTIME> {
    // FILETIME counts 100 ns intervals since 1601-01-01.
    let ft = (secs as u64 + 11_644_473_600) * 10_000_000;
    let ft = winapi::shared::minwindef::FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
    unsafe {
        let mut utc: SYSTEMTIME = std::mem::zeroed();
        let mut local: SYSTEMTIME = std::mem::zeroed();
        if FileTimeToSystemTime(&ft, &mut utc) == 0
            || SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) == 0
        {
            return None;
        }
        Some(local)
    }
}

fn same_day(a: &SYSTEMTIME, b: &SYSTEMTIME) -> bool {
    (a.wYear, a.wMonth, a.wDay) == (b.wYear, b.wMonth, b.wDay)
}

/// Formats Unix seconds as local time: `YYYY/MM/DD HH:MM`, or with
/// `relative` "قبل 5 دقائق", "اليوم 14:30" and "أمس 09:12" for recent files.
pub fn format_time(secs: i64, relative: bool) -> String {
    if secs <= 0 {
        return String::new();
    }
    let Some(t) = local_time(secs) else { return String::new() };
    let hm = format!("{:02}:{:02}", t.wHour, t.wMinute);
    if relative {
        let now = crate::index::now_secs();
        let age = now - secs;
        if (0..3600).contains(&age) {
            return super::format::digits(super::format::minutes_ago(age / 60));
        }
        if let Some(today) = local_time(now) {
            if same_day(&t, &today) {
                return super::format::digits(trf!("اليوم {hm}", "Today {hm}"));
            }
            if local_time(now - 86_400).is_some_and(|y| same_day(&t, &y)) {
                return super::format::digits(trf!("أمس {hm}", "Yesterday {hm}"));
            }
        }
    }
    let s = format!("{:04}/{:02}/{:02} {hm}", t.wYear, t.wMonth, t.wDay);
    if relative { super::format::digits(s) } else { s }
}
