//! The one-time request to rate InstaFind in the Microsoft Store.
//!
//! Only the Store (MSIX-packaged) build asks, once, after a first real
//! success: the user opened files from search results at least
//! [`OPENS_BEFORE_ASKING`] times. The count stays in `settings.txt` on this
//! computer; nothing is sent anywhere. The question is a calm row above the
//! results (never at start-up, never while typing, never a modal box); once it
//! has been shown, it is not shown again, whatever the answer. Packaged builds
//! also keep a permanent "Rate in the Store" menu item. No rewards.
//!
//! The decision is plain logic, unit-tested on every platform; only
//! [`is_packaged`] and the UI in `ui/rating.rs` are Windows-only.

/// InstaFind's product ID in the Microsoft Store.
pub const STORE_PRODUCT_ID: &str = "9MTK4KMFQ657";

/// Files opened from results before the question may come.
pub const OPENS_BEFORE_ASKING: u32 = 5;

/// Quiet time after the last keystroke in the search box before the question
/// may appear, in UI timer ticks (200 ms each): 3 seconds.
pub const QUIET_TICKS: u32 = 15;

/// The Store's "write a review" page for InstaFind.
pub fn review_uri() -> String {
    format!("ms-windows-store://review/?ProductId={STORE_PRODUCT_ID}")
}

/// The count after one more file was opened from the results. Counting stops
/// once the question was asked, so the settings file stops changing.
pub fn count_open(opens: u32, asked: bool) -> u32 {
    if asked { opens } else { opens.saturating_add(1) }
}

/// Whether the question is due: a packaged (Store) build that has not asked
/// yet, after enough opened files.
pub fn rating_due(packaged: bool, asked: bool, opens: u32) -> bool {
    packaged && !asked && opens >= OPENS_BEFORE_ASKING
}

/// What the window is doing right now, as far as the question cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Moment {
    /// The main window is on screen (not hidden in the tray).
    pub visible: bool,
    /// A message box or another modal window of the program is open.
    pub modal: bool,
    /// Timer ticks since the last keystroke in the search box.
    pub ticks_since_typing: u32,
}

/// Whether a due question may be shown now: only in the visible window, with
/// no message box open, and not while the user is typing.
pub fn show_now(due: bool, m: Moment) -> bool {
    due && m.visible && !m.modal && m.ticks_since_typing >= QUIET_TICKS
}

/// Whether this copy runs from an MSIX package (the Microsoft Store build).
/// `GetCurrentPackageFullName` is looked up at run time, so Windows 7 (which
/// has no packages) simply gets `false`; so does every unpackaged exe.
#[cfg(windows)]
pub fn is_packaged() -> bool {
    use std::sync::OnceLock;
    static PACKAGED: OnceLock<bool> = OnceLock::new();
    *PACKAGED.get_or_init(|| {
        use winapi::um::libloaderapi::{GetModuleHandleW, GetProcAddress};
        // LONG GetCurrentPackageFullName(UINT32 *packageFullNameLength, PWSTR packageFullName)
        type GetCurrentPackageFullName = unsafe extern "system" fn(*mut u32, *mut u16) -> i32;
        const ERROR_INSUFFICIENT_BUFFER: i32 = 122;
        let kernel32: Vec<u16> = "kernel32.dll".encode_utf16().chain(Some(0)).collect();
        unsafe {
            let module = GetModuleHandleW(kernel32.as_ptr());
            if module.is_null() {
                return false;
            }
            let f = GetProcAddress(module, c"GetCurrentPackageFullName".as_ptr());
            if f.is_null() {
                return false;
            }
            let f: GetCurrentPackageFullName = std::mem::transmute(f);
            // With an empty buffer a packaged process gets "buffer too small";
            // an unpackaged one gets APPMODEL_ERROR_NO_PACKAGE (15700).
            let mut len = 0u32;
            f(&mut len, std::ptr::null_mut()) == ERROR_INSUFFICIENT_BUFFER
        }
    })
}

/// Outside Windows there are no packages.
#[cfg(not(windows))]
#[allow(dead_code)]
pub fn is_packaged() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUIET: Moment = Moment { visible: true, modal: false, ticks_since_typing: QUIET_TICKS };

    #[test]
    fn asks_only_packaged_builds_once_after_five_opens() {
        // Unpackaged copies (GitHub exe, winget, Scoop, Chocolatey) never ask.
        assert!(!rating_due(false, false, 100));
        // Packaged: not before the fifth opened file.
        for opens in 0..OPENS_BEFORE_ASKING {
            assert!(!rating_due(true, false, opens), "{opens}");
        }
        assert!(rating_due(true, false, 5) && rating_due(true, false, u32::MAX));
        // Asked once (either answer, or just shown): never again.
        assert!(!rating_due(true, true, 5) && !rating_due(true, true, 500));
    }

    #[test]
    fn counting_stops_after_asking() {
        let mut n = 0;
        for _ in 0..5 {
            n = count_open(n, false);
        }
        assert_eq!(n, 5);
        assert!(rating_due(true, false, n));
        assert_eq!(count_open(n, true), n);
        assert_eq!(count_open(u32::MAX, false), u32::MAX);
    }

    #[test]
    fn never_while_typing_hidden_or_behind_a_message_box() {
        assert!(show_now(true, QUIET));
        assert!(!show_now(false, QUIET));
        assert!(!show_now(true, Moment { visible: false, ..QUIET }));
        assert!(!show_now(true, Moment { modal: true, ..QUIET }));
        assert!(!show_now(true, Moment { ticks_since_typing: QUIET_TICKS - 1, ..QUIET }));
        assert!(!show_now(true, Moment { ticks_since_typing: 0, ..QUIET }));
        assert!(show_now(true, Moment { ticks_since_typing: u32::MAX, ..QUIET }));
    }

    #[test]
    fn store_review_link() {
        assert_eq!(review_uri(), "ms-windows-store://review/?ProductId=9MTK4KMFQ657");
    }

    #[test]
    fn nothing_is_packaged_off_windows() {
        #[cfg(not(windows))]
        assert!(!is_packaged());
    }
}
