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
const OWN_KEY: &str = "Software\\BahthSari";
const CHOSEN: &str = "AutostartChosen";

/// Command-line flag of a launch at sign-in: start hidden in the tray.
pub const STARTUP_ARG: &str = "--startup";

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

fn open(path: &str, write: bool) -> Option<Key> {
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

/// Whether the program starts with Windows.
pub fn is_enabled() -> bool {
    open(RUN_KEY, false).is_some_and(|k| has_value(&k, VALUE))
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
        set_string(&key, VALUE, &format!("\"{}\" {STARTUP_ARG}", exe.display()))
    } else {
        let name = wide(VALUE);
        let r = unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
        r == ERROR_SUCCESS as i32 || !has_value(&key, VALUE)
    }
}

/// On by default: the first run turns it on once; after that only the user's
/// choice counts. A stale path (the exe moved) is refreshed.
pub fn apply_default() {
    let chosen = open(OWN_KEY, false).is_some_and(|k| has_value(&k, CHOSEN));
    if !chosen || is_enabled() {
        set_enabled(true);
    }
}
