//! Starting with Windows: a value under the current user's `Run` key (no
//! admin rights needed) that launches the program hidden in the tray.

use winapi::shared::minwindef::{DWORD, HKEY};
use winapi::shared::winerror::ERROR_SUCCESS;
use winapi::um::winnt::{KEY_READ, KEY_WRITE, REG_SZ};
use winapi::um::winreg::{
    HKEY_CURRENT_USER, RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW,
    RegSetValueExW,
};

use super::shell::wide;

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE: &str = "BahthSari";
/// Remembers that the user has been given the default once, so turning it
/// off is respected.
pub(super) const OWN_KEY: &str = "Software\\BahthSari";
const CHOSEN: &str = "AutostartChosen";
/// Set once 0.8.1 has turned starting with Windows back on (the user asked
/// for it to be on); after that the user's choice counts again.
const RESET_081: &str = "Autostart081";
/// Where Task Manager's "Startup apps" page keeps apps it turned off.
const APPROVED_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run";

/// Command-line flag of a launch at sign-in: start hidden in the tray.
pub const STARTUP_ARG: &str = "--startup";

pub(super) struct Key(pub(super) HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

pub(super) fn open(path: &str, write: bool) -> Option<Key> {
    let mut key: HKEY = std::ptr::null_mut();
    let path = wide(path);
    let ok = unsafe {
        if write {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                path.as_ptr(),
                0,
                std::ptr::null_mut(),
                0,
                KEY_READ | KEY_WRITE,
                std::ptr::null_mut(),
                &mut key,
                std::ptr::null_mut(),
            )
        } else {
            RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_READ, &mut key)
        }
    };
    (ok == ERROR_SUCCESS as i32).then_some(Key(key))
}

fn has_value(key: &Key, name: &str) -> bool {
    let name = wide(name);
    unsafe {
        RegQueryValueExW(
            key.0,
            name.as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        ) == ERROR_SUCCESS as i32
    }
}

fn set_string(key: &Key, name: &str, value: &str) -> bool {
    let (name, value) = (wide(name), wide(value));
    unsafe {
        RegSetValueExW(
            key.0,
            name.as_ptr(),
            0,
            REG_SZ,
            value.as_ptr().cast(),
            (value.len() * 2) as DWORD,
        ) == ERROR_SUCCESS as i32
    }
}

/// A string value (REG_SZ), if the key has one under `name`.
fn get_string(key: &Key, name: &str) -> Option<String> {
    let name = wide(name);
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as DWORD;
    let mut kind: DWORD = 0;
    let ok = unsafe {
        RegQueryValueExW(key.0, name.as_ptr(), std::ptr::null_mut(), &mut kind, buf.as_mut_ptr().cast(), &mut size)
    };
    if ok != ERROR_SUCCESS as i32 || kind != REG_SZ {
        return None;
    }
    let n = (size as usize / 2).min(buf.len());
    let s = &buf[..n];
    let end = s.iter().position(|&c| c == 0).unwrap_or(n);
    Some(String::from_utf16_lossy(&s[..end]))
}

/// The folders `run_entry` tells Downloads and temp folders by. The same
/// lookup as `ui/relocate.rs` (kept apart so the two files stay separate).
fn folders() -> crate::relocate::Folders {
    crate::relocate::Folders::from_env(&|name| std::env::var(name).ok(), downloads_folder())
}

/// The Downloads folder, also when the user moved it.
fn downloads_folder() -> Option<String> {
    use winapi::shared::winerror::SUCCEEDED;
    use winapi::um::combaseapi::CoTaskMemFree;
    use winapi::um::knownfolders::FOLDERID_Downloads;
    use winapi::um::shlobj::SHGetKnownFolderPath;
    let mut p = std::ptr::null_mut();
    // SAFETY: `p` receives a string the shell allocated; it is freed below
    // even when the call fails.
    let hr = unsafe { SHGetKnownFolderPath(&FOLDERID_Downloads, 0, std::ptr::null_mut(), &mut p) };
    if p.is_null() {
        return None;
    }
    let s = unsafe { super::format::wide_to_string(p) };
    unsafe { CoTaskMemFree(p.cast()) };
    SUCCEEDED(hr).then_some(s)
}

/// The first byte of Task Manager's record for our entry, if it has one:
/// even (2, 6) means allowed, odd (3, 7) means turned off there.
fn approved_flag() -> Option<u8> {
    let key = open(APPROVED_KEY, false)?;
    let name = wide(VALUE);
    let mut buf = [0u8; 12];
    let mut size = buf.len() as DWORD;
    let ok = unsafe {
        RegQueryValueExW(key.0, name.as_ptr(), std::ptr::null_mut(), std::ptr::null_mut(), buf.as_mut_ptr(), &mut size)
    };
    (ok == ERROR_SUCCESS as i32 && size > 0).then_some(buf[0])
}

/// Whether Task Manager's "Startup apps" turned our entry off.
fn blocked_by_task_manager() -> bool {
    approved_flag().is_some_and(|b| b & 1 == 1)
}

/// Clears a "turned off" mark Task Manager left on our entry.
fn unblock() {
    if !blocked_by_task_manager() {
        return;
    }
    if let Some(key) = open(APPROVED_KEY, true) {
        let name = wide(VALUE);
        unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
    }
}

/// Whether the program starts with Windows: our `Run` entry is there and
/// Task Manager has not turned it off.
pub fn is_enabled() -> bool {
    open(RUN_KEY, false).is_some_and(|k| has_value(&k, VALUE)) && !blocked_by_task_manager()
}

/// Turns starting with Windows on or off for the current user. The entry
/// points at this exe, which the updater keeps at the same path, except
/// that a copy in Downloads or a temp folder keeps an entry that starts
/// another copy which still exists (the one it was moved to:
/// `crate::run_entry::target`).
pub fn set_enabled(on: bool) -> bool {
    let Some(key) = open(RUN_KEY, true) else { return false };
    if let Some(own) = open(OWN_KEY, true) {
        set_string(&own, CHOSEN, "1");
    }
    if on {
        let Ok(exe) = std::env::current_exe() else { return false };
        let current = exe.to_string_lossy();
        let existing = get_string(&key, VALUE);
        let target =
            crate::run_entry::target(&current, existing.as_deref(), &folders(), &|p| std::path::Path::new(p).is_file());
        unblock();
        set_string(&key, VALUE, &command(std::path::Path::new(target)))
    } else {
        let name = wide(VALUE);
        let r = unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
        r == ERROR_SUCCESS as i32 || !has_value(&key, VALUE)
    }
}

/// What the `Run` entry starts.
fn command(exe: &std::path::Path) -> String {
    format!("\"{}\" {STARTUP_ARG}", exe.display())
}

/// After the exe was copied to a permanent place (`relocate`): an existing
/// entry now starts `exe`. Nothing else changes: an entry that is off stays
/// off (no entry, or turned off in Task Manager, which keeps its mark).
pub fn point_to(exe: &std::path::Path) -> bool {
    let Some(key) = open(RUN_KEY, true) else { return false };
    !has_value(&key, VALUE) || set_string(&key, VALUE, &command(exe))
}

/// On by default: the first run turns it on once; after that only the user's
/// choice counts. A stale path (the exe moved) is refreshed.
pub fn apply_default() {
    let own = open(OWN_KEY, false);
    let chosen = own.as_ref().is_some_and(|k| has_value(k, CHOSEN));
    // 0.8.1 turns it on once more, for those who had it off without meaning to.
    let reset_done = own.as_ref().is_some_and(|k| has_value(k, RESET_081));
    drop(own);
    if !chosen || !reset_done || is_enabled() {
        set_enabled(true);
    }
    if !reset_done && let Some(own) = open(OWN_KEY, true) {
        set_string(&own, RESET_081, "1");
    }
}
