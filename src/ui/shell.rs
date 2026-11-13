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

/// Puts `text` on the clipboard for `owner`, trying again for a moment
/// while another program holds the clipboard open. False when it is not
/// there, so the caller can say so (the user would paste the old content).
pub fn copy_text<C: Into<native_windows_gui::ControlHandle> + Copy>(owner: C, text: &str) -> bool {
    let ok = crate::quick::retry_briefly(
        6,
        || native_windows_gui::Clipboard::set_data_text(owner, text),
        || std::thread::sleep(std::time::Duration::from_millis(15)),
    );
    if !ok {
        super::startup::log("clipboard: another program kept it open; nothing copied");
    }
    ok
}

/// What the user is told when [`copy_text`] failed.
pub fn copy_failed_text() -> &'static str {
    tr!(
        "تعذر النسخ: برنامج آخر يستخدم الحافظة الآن. جرّب مرة أخرى.",
        "Could not copy: another program is using the clipboard. Try again."
    )
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Renames `old` to `new`, never over another file: unlike `fs::rename`
/// (MoveFileExW with MOVEFILE_REPLACE_EXISTING), a file that appears at
/// `new` after the caller looked makes this fail ([`already_exists`])
/// instead of being replaced. Changing only the letter case works.
pub fn rename_no_replace(old: &str, new: &str) -> std::io::Result<()> {
    let (from, to) = (wide(old), wide(new));
    if unsafe { winapi::um::winbase::MoveFileExW(from.as_ptr(), to.as_ptr(), 0) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Whether `e` says the new name is taken.
pub fn already_exists(e: &std::io::Error) -> bool {
    use winapi::shared::winerror::{ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS};
    matches!(e.raw_os_error(), Some(c) if c == ERROR_ALREADY_EXISTS as i32 || c == ERROR_FILE_EXISTS as i32)
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

/// Runs a shell verb of a file's own Windows menu, such as "properties"
/// (the Properties window) or "openas" (Windows' "Open with" choice), over
/// `owner`. True if Windows took it.
pub fn invoke_verb(owner: Option<winapi::shared::windef::HWND>, path: &str, verb: &str) -> bool {
    use winapi::um::shellapi::{SEE_MASK_INVOKEIDLIST, SHELLEXECUTEINFOW, ShellExecuteExW};
    let (verb, file) = (wide(verb), wide(path));
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    // The verb comes from the item's own shell menu, as Explorer does it.
    info.fMask = SEE_MASK_INVOKEIDLIST;
    info.hwnd = owner.unwrap_or(std::ptr::null_mut());
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.nShow = SW_SHOWNORMAL;
    unsafe { ShellExecuteExW(&mut info) != 0 }
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
    // An empty name would end the list early (and none would leave a list
    // with a single NUL, read past its end).
    let paths: Vec<String> = paths.iter().filter(|p| !p.is_empty() && !p.contains('\0')).cloned().collect();
    if paths.is_empty() {
        return Vec::new();
    }
    // NUL-separated paths, ending with two NULs.
    let mut from: Vec<u16> = Vec::new();
    for p in &paths {
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

/// Opens Explorer with the file selected. Explorer is started by its full
/// path: a bare `explorer.exe` would be looked for in this exe's own folder
/// first (Downloads, for a portable copy), where one could have been planted.
pub fn reveal(path: &str) {
    let Some(explorer) = windows_program("explorer.exe") else { return };
    let _ = std::process::Command::new(explorer).raw_arg(format!("/select,\"{path}\"")).spawn();
}

/// A folder Windows reports through `get` (`GetSystemDirectoryW`,
/// `GetWindowsDirectoryW`).
fn windows_folder(get: unsafe extern "system" fn(*mut u16, u32) -> u32) -> Option<std::path::PathBuf> {
    let mut buf = [0u16; 260];
    let n = unsafe { get(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    (n > 0 && n < buf.len()).then(|| std::path::PathBuf::from(String::from_utf16_lossy(&buf[..n])))
}

/// The full path of a program that comes with Windows (`explorer.exe`,
/// `control.exe`, `devmgmt.msc`, `regedit.exe`…): in the system folder
/// (System32; SysWOW64 for the 32-bit exe on 64-bit Windows), then the
/// 64-bit System32 as the 32-bit exe sees it (`Sysnative`, for tools that
/// only exist there), then the Windows folder; `None` when it is in none of
/// them (say `gpedit.msc` on Windows Home). Never a bare name, which
/// `CreateProcess` and `ShellExecute` would look for in this exe's folder or
/// the current folder first.
pub fn windows_program(name: &str) -> Option<std::path::PathBuf> {
    use winapi::um::sysinfoapi::{GetSystemDirectoryW, GetWindowsDirectoryW};
    let windows = windows_folder(GetWindowsDirectoryW);
    [windows_folder(GetSystemDirectoryW), windows.as_ref().map(|w| w.join("Sysnative")), windows]
        .into_iter()
        .flatten()
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
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

/// Unix seconds as a FILETIME count (100 ns intervals since 1601-01-01),
/// or `None` when out of range: a damaged index or a file stamped in the
/// far future must not overflow (a panic in a debug build, nonsense in a
/// release one).
fn unix_to_filetime(secs: i64) -> Option<u64> {
    let ft = secs.checked_add(11_644_473_600)?.checked_mul(10_000_000)?;
    // FileTimeToSystemTime takes values below 2^63 only.
    u64::try_from(ft).ok()
}

fn local_time(secs: i64) -> Option<SYSTEMTIME> {
    let ft = unix_to_filetime(secs)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filetime_of_unix_seconds() {
        assert_eq!(unix_to_filetime(0), Some(116_444_736_000_000_000));
        assert_eq!(unix_to_filetime(1), Some(116_444_736_010_000_000));
        // Before 1970 down to 1601 is still a valid FILETIME.
        assert_eq!(unix_to_filetime(-11_644_473_600), Some(0));
        assert_eq!(unix_to_filetime(-11_644_473_601), None);
        // Far-future stamps from a damaged index: no overflow.
        assert_eq!(unix_to_filetime(i64::MAX), None);
        assert_eq!(unix_to_filetime(i64::MIN), None);
        assert!(unix_to_filetime(900_000_000_000).is_some());
        assert_eq!(unix_to_filetime(1_000_000_000_000), None);
    }

    #[test]
    fn huge_times_format_without_panicking() {
        for secs in [1, 1_700_000_000, 900_000_000_000, 1_000_000_000_000, i64::MAX] {
            let _ = format_time(secs, true);
            let _ = format_time(secs, false);
        }
        assert_eq!(format_time(i64::MAX, false), "");
    }

    #[test]
    fn recycling_nothing_does_nothing() {
        assert!(recycle_many(&[]).is_empty());
        assert!(recycle_many(&[String::new()]).is_empty());
    }
}
