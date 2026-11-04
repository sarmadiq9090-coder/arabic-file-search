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
/// The program's own key; `AutostartChosen` there records that the user
/// chose (kept for older versions, which read it).
pub(super) const OWN_KEY: &str = "Software\\BahthSari";
const CHOSEN: &str = "AutostartChosen";
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
/// points at this exe, which the updater keeps at the same path.
pub fn set_enabled(on: bool) -> bool {
    let Some(key) = open(RUN_KEY, true) else { return false };
    if let Some(own) = open(OWN_KEY, true) {
        set_string(&own, CHOSEN, "1");
    }
    if on {
        let Ok(exe) = std::env::current_exe() else { return false };
        unblock();
        set_string(&key, VALUE, &command(&exe))
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

/// Off by default: only the user turns it on, in the first-run question
/// (`tray::ask_first_run`) or in Options. Up to 0.9.1 the first run turned
/// it on by itself; an entry that is on stays on and is pointed at this exe
/// again, in case it moved.
pub fn apply_default() {
    if is_enabled()
        && let Ok(exe) = std::env::current_exe()
    {
        point_to(&exe);
    }
}
