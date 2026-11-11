//! The new interface (1.0, being built in phases; see `docs/ui-redesign/`):
//! the main window as a page in WebView2, embedded in the exe.
//!
//! - `assets`: the page, its script, styles and fonts, and the strict CSP
//! - `bridge`: the typed messages between the page and the program, the
//!   per-launch token, and the search behind them (platform independent and
//!   unit-tested on Linux)
//! - `host` (Windows, MSVC builds only): the WebView2 control (wry) inside a
//!   Win32 window, serving `assets` through the `instafind://` protocol
//!
//! The window itself, wired to the running program, is `ui/webui.rs`. While
//! the new interface is incomplete it is opt-in ([`chosen`]): «جرّب الشكل
//! الجديد» in the classic window's Options menu turns it on (the `new_ui`
//! setting), «العودة إلى الشكل القديم» on its settings page turns it off,
//! and the classic Win32 window stays the default and the fallback when
//! WebView2 is missing or fails.

// Only MSVC builds host the page (see Cargo.toml); elsewhere the shared parts
// are compiled for their tests and for the opt-in switch alone.
#![cfg_attr(not(all(windows, target_env = "msvc")), allow(dead_code))]

pub mod assets;
pub mod bridge;
#[cfg(all(windows, target_env = "msvc"))]
pub mod host;

/// The switch that turns the new interface on for one launch, whatever the
/// settings say (the tests use it).
pub const ARG: &str = "--webui";
/// The same switch as an environment variable (`BAHTH_WEBUI=1`).
pub const ENV: &str = "BAHTH_WEBUI";

/// Whether the new interface was asked for, by [`ARG`] or by [`ENV`] set to
/// anything but empty, `0`, `false` or `off`.
pub fn requested_by<S: AsRef<str>>(args: &[S], env: Option<&str>) -> bool {
    args.iter().any(|a| a.as_ref() == ARG)
        || env.is_some_and(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "" | "0" | "false" | "off"))
}

/// [`requested_by`] for this process.
pub fn requested() -> bool {
    let args: Vec<String> = std::env::args().skip(1).collect();
    requested_by(&args, std::env::var(ENV).ok().as_deref())
}

/// Whether this launch should use the new interface: chosen in the settings
/// (`new_ui`, see `crate::settings`), or asked for by [`ARG`] or [`ENV`].
/// (WebView2 must be installed too; the window checks that.)
pub fn chosen(new_ui: bool) -> bool {
    new_ui || requested()
}

/// What a failed WebView2 process means for the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// The browser process is gone: only a new page can show it again.
    Browser,
    /// The page's renderer exited or hangs: reloading shows it again.
    Page,
}

/// `COREWEBVIEW2_PROCESS_FAILED_KIND` values that concern the page (the
/// host checks them against WebView2's own at compile time).
pub mod failed_kind {
    pub const BROWSER_PROCESS_EXITED: i32 = 0;
    pub const RENDER_PROCESS_EXITED: i32 = 1;
    pub const RENDER_PROCESS_UNRESPONSIVE: i32 = 2;
}

/// The failure a `ProcessFailed` event of `kind` is, or `None` when the page
/// is not affected: the GPU, utility, sandbox helper or plug-in processes,
/// and frames (the page has none), which WebView2 restarts by itself. Only
/// a failure counts towards giving the new look up.
pub fn failure(kind: i32) -> Option<Failure> {
    match kind {
        failed_kind::BROWSER_PROCESS_EXITED => Some(Failure::Browser),
        failed_kind::RENDER_PROCESS_EXITED | failed_kind::RENDER_PROCESS_UNRESPONSIVE => Some(Failure::Page),
        _ => None,
    }
}

/// How long a page must have worked (said hello and not failed since) for
/// its earlier failures to be forgiven: two failures days apart in a
/// program left running in the tray are not a broken new look, a page that
/// fails again right after each reload is.
pub const STABLE: std::time::Duration = std::time::Duration::from_secs(60);

/// Whether a page that said hello `ready_for` ago (`None`: not since its
/// last failure) has worked long enough for its failures to be forgiven.
pub fn stable(ready_for: Option<std::time::Duration>) -> bool {
    ready_for.is_some_and(|d| d >= STABLE)
}

/// After the new look could not be started at launch (both attempts):
/// `(new_ui, new_ui_failed)` to save. One bad start (WebView2 updating
/// during login) keeps the new look for the next launch; a second start in
/// a row that fails turns it off. Nothing is turned on that was not
/// (`--webui` alone).
pub fn after_failed_start(new_ui: bool, failed_before: bool) -> (bool, bool) {
    if new_ui && !failed_before { (true, true) } else { (false, false) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_page_failures_count() {
        assert_eq!(failure(0), Some(Failure::Browser));
        assert_eq!(failure(1), Some(Failure::Page));
        assert_eq!(failure(2), Some(Failure::Page));
        // Frame renderer, utility, sandbox helper, GPU, plug-ins, unknown.
        for kind in [3, 4, 5, 6, 7, 8, 9, -1, 42] {
            assert_eq!(failure(kind), None, "{kind}");
        }
    }

    #[test]
    fn failures_forgiven_after_a_good_while() {
        use std::time::Duration;
        assert!(!stable(None));
        assert!(!stable(Some(Duration::from_secs(5))));
        assert!(stable(Some(STABLE)));
        assert!(stable(Some(Duration::from_secs(3 * 86_400))));
    }

    #[test]
    fn a_second_bad_start_in_a_row_turns_the_new_look_off() {
        // First bad start: kept, remembered.
        assert_eq!(after_failed_start(true, false), (true, true));
        // The next start fails too: off.
        assert_eq!(after_failed_start(true, true), (false, false));
        // Only forced by --webui: stays off.
        assert_eq!(after_failed_start(false, false), (false, false));
        assert_eq!(after_failed_start(false, true), (false, false));
    }

    #[test]
    fn opt_in_switch() {
        let none: [&str; 0] = [];
        assert!(!requested_by(&none, None));
        assert!(requested_by(&["--startup", "--webui"], None));
        assert!(!requested_by(&["--webuix"], None));
        for on in ["1", "yes", "true", " on "] {
            assert!(requested_by(&none, Some(on)), "{on}");
        }
        for off in ["", "0", "false", "OFF"] {
            assert!(!requested_by(&none, Some(off)), "{off}");
        }
    }
}
