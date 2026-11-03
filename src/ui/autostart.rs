//! Starting with Windows: a value under the current user's `Run` key (no
//! admin rights needed) that launches the program hidden in the tray.
//! The Microsoft Store version uses its package's StartupTask instead
//! (`crate::store`): a packaged program's `Run` value is never read.

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

/// Whether this run started with Windows (hidden, in the background): the
/// `Run` entry passes `--startup`; the Store's startup task passes nothing.
pub fn launched_at_sign_in() -> bool {
    std::env::args().any(|a| a == STARTUP_ARG) || crate::store::launched_by_startup_task()
}

/// What to tell the user when `set_enabled` fails.
pub fn change_failed_message() -> String {
    match crate::store::startup_state() {
        Some(state) => crate::store::startup_refused_message(state),
        None => tr!("تعذر تغيير إعداد التشغيل مع ويندوز.", "Could not change the start-with-Windows setting.").to_string(),
    }
}

/// Whether the program starts with Windows: our `Run` entry is there and
/// Task Manager has not turned it off.
pub fn is_enabled() -> bool {
    if crate::store::is_packaged() {
        return crate::store::startup_state().is_some_and(|s| s.is_on());
    }
    open(RUN_KEY, false).is_some_and(|k| has_value(&k, VALUE)) && !blocked_by_task_manager()
}

/// Turns starting with Windows on or off for the current user. The entry
/// points at this exe, which the updater keeps at the same path.
pub fn set_enabled(on: bool) -> bool {
    if crate::store::is_packaged() {
        if let Some(own) = open(OWN_KEY, true) {
            set_string(&own, CHOSEN, "1");
        }
        return crate::store::set_startup(on).is_ok();
    }
    let Some(key) = open(RUN_KEY, true) else { return false };
    if let Some(own) = open(OWN_KEY, true) {
        set_string(&own, CHOSEN, "1");
    }
    if on {
        let Ok(exe) = std::env::current_exe() else { return false };
        unblock();
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
    if crate::store::is_packaged() {
        // Store: turned on once, through the startup task, unless the user
        // already chose or turned it off in Settings / Task Manager.
        let chosen = open(OWN_KEY, false).is_some_and(|k| has_value(&k, CHOSEN));
        if crate::store::startup_state().is_some_and(|s| crate::store::enable_on_first_run(chosen, s)) {
            set_enabled(true);
        }
        return;
    }
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
