//! TEMPORARY: stand-ins for launcher functions that are being written in
//! parallel (src/launcher/). Each one is replaced at merge time by the real
//! function named in its comment; then delete this file and its `mod` line
//! in `src/ui/mod.rs`. Nothing else in the UI calls the launcher's
//! Windows-side functions directly.

use crate::launcher::{Catalog, SystemCommand, apps, windows::OpenWindow};

/// TEMPORARY → `crate::launcher::Catalog::new()` (which scans the apps and
/// fills the settings pages). Runs on a background thread.
pub(super) fn new_catalog() -> Catalog {
    // → crate::launcher::apps::scan()
    let apps: Vec<apps::App> = Vec::new();
    // → crate::launcher::settings_pages::PAGES.to_vec()
    let settings = Vec::new();
    Catalog { apps, settings }
}

/// TEMPORARY → `crate::launcher::windows::list()`: visible top-level windows
/// of other programs. Called on the UI thread each time the popup opens.
pub(super) fn list_windows() -> Vec<OpenWindow> {
    Vec::new()
}

/// TEMPORARY → `crate::launcher::windows::focus(hwnd)`: brings a window to
/// the front (restoring it when minimised). True when it worked.
pub(super) fn focus_window(hwnd: isize) -> bool {
    use winapi::um::winuser::{IsIconic, IsWindow, SW_RESTORE, SetForegroundWindow, ShowWindow};
    let h = hwnd as winapi::shared::windef::HWND;
    unsafe {
        if h.is_null() || IsWindow(h) == 0 {
            return false;
        }
        if IsIconic(h) != 0 {
            ShowWindow(h, SW_RESTORE);
        }
        SetForegroundWindow(h) != 0
    }
}

/// TEMPORARY → `crate::launcher::system::confirm_text(cmd)`: the question
/// asked before running `cmd`, in the interface language.
pub(super) fn confirm_text(cmd: &SystemCommand) -> String {
    match cmd {
        SystemCommand::Shutdown => tr!("إيقاف تشغيل الكمبيوتر الآن؟", "Shut down the computer now?").to_string(),
        SystemCommand::Restart => tr!("إعادة تشغيل الكمبيوتر الآن؟", "Restart the computer now?").to_string(),
        SystemCommand::Sleep => tr!("وضع الكمبيوتر في السكون؟", "Put the computer to sleep?").to_string(),
        SystemCommand::Lock => tr!("قفل الكمبيوتر؟", "Lock the computer?").to_string(),
        SystemCommand::SignOut => tr!("تسجيل الخروج الآن؟", "Sign out now?").to_string(),
        SystemCommand::EmptyRecycleBin => tr!("إفراغ سلة المحذوفات؟", "Empty the Recycle Bin?").to_string(),
        SystemCommand::EndProcess { name, .. } => trf!("إنهاء البرنامج {name}؟", "End {name}?"),
    }
}

/// TEMPORARY → `crate::launcher::system::run(cmd)`: runs a confirmed
/// command. The stand-in only locks the computer; it reports false for the
/// rest so nothing drastic happens before the real code lands.
pub(super) fn run_system(cmd: &SystemCommand) -> bool {
    match cmd {
        SystemCommand::Lock => unsafe { winapi::um::winuser::LockWorkStation() != 0 },
        _ => false,
    }
}
