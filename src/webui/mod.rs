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
//! The window itself, wired to the running program, is `ui/webui.rs`. Since
//! 1.0 it is the only look users get ([`chosen`]; `new_ui` is turned on for
//! good once, `Settings::make_new_look_only`, and nothing offers the way
//! back). The classic Win32 window is a hidden safety fallback, for one run
//! only, when WebView2 is missing or fails (and in mingw builds): the next
//! start tries the new look again, and «جرّب الشكل الجديد» in the classic
//! window's Options menu tries it at once. The tests ask for the classic
//! window with [`CLASSIC_ARG`] / [`CLASSIC_ENV`].

// Only MSVC builds host the page (see Cargo.toml); elsewhere the shared parts
// are compiled for their tests and for the opt-in switch alone.
#![cfg_attr(not(all(windows, target_env = "msvc")), allow(dead_code))]

pub mod assets;
pub mod bridge;
pub mod png;
#[cfg(all(windows, target_env = "msvc"))]
pub mod host;

/// The switch that turns the new interface on for one launch, whatever the
/// settings say (the tests use it).
pub const ARG: &str = "--webui";
/// The same switch as an environment variable (`BAHTH_WEBUI=1`).
pub const ENV: &str = "BAHTH_WEBUI";
/// For the tests of the classic window (`ci/ui-test.ps1`): the classic
/// window for one launch, whatever the settings say, without the note that
/// WebView2 is not working. Wins over [`ARG`] / [`ENV`]. Not offered to users.
pub const CLASSIC_ARG: &str = "--classic";
/// The same as an environment variable (`BAHTH_CLASSIC=1`).
pub const CLASSIC_ENV: &str = "BAHTH_CLASSIC";
/// For the test of the fallback (`ci/newlook-test.ps1`): WebView2 is taken
/// as failing to start, as on a machine where it is broken.
pub const FAIL_ENV: &str = "BAHTH_WEBUI_FAIL";

/// Whether switch `arg` is in `args`, or `env` is set to anything but empty,
/// `0`, `false` or `off`.
fn switch_by<S: AsRef<str>>(arg: &str, args: &[S], env: Option<&str>) -> bool {
    args.iter().any(|a| a.as_ref() == arg)
        || env.is_some_and(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "" | "0" | "false" | "off"))
}

/// Whether the new interface was asked for, by [`ARG`] or by [`ENV`].
pub fn requested_by<S: AsRef<str>>(args: &[S], env: Option<&str>) -> bool {
    switch_by(ARG, args, env)
}

/// Whether the classic window was asked for, by [`CLASSIC_ARG`] or by [`CLASSIC_ENV`].
pub fn classic_requested_by<S: AsRef<str>>(args: &[S], env: Option<&str>) -> bool {
    switch_by(CLASSIC_ARG, args, env)
}

/// [`requested_by`] for this process.
pub fn requested() -> bool {
    let args: Vec<String> = std::env::args().skip(1).collect();
    requested_by(&args, std::env::var(ENV).ok().as_deref())
}

/// [`classic_requested_by`] for this process.
pub fn classic_requested() -> bool {
    let args: Vec<String> = std::env::args().skip(1).collect();
    classic_requested_by(&args, std::env::var(CLASSIC_ENV).ok().as_deref())
}

/// Whether [`FAIL_ENV`] is set for this process (anything but empty, `0`,
/// `false` or `off`).
pub fn fail_requested() -> bool {
    let none: [&str; 0] = [];
    switch_by("", &none, std::env::var(FAIL_ENV).ok().as_deref())
}

/// Whether this launch should use the new interface: `new_ui` (always on
/// since 1.0, see `crate::settings`) or [`ARG`] / [`ENV`], unless the tests
/// asked for the classic window. (WebView2 must be installed too; the
/// window checks that.)
pub fn chosen(new_ui: bool) -> bool {
    pick(new_ui, requested(), classic_requested())
}

/// [`chosen`] from its parts.
fn pick(new_ui: bool, requested: bool, classic: bool) -> bool {
    !classic && (new_ui || requested)
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

/// How the new look starts after the classic window took over at the last
/// launch (`new_ui_failed`, see `crate::settings::Settings::note_fallback`):
/// it is tried again (WebView2 may have been repaired or updated since),
/// but without the second chances, so a WebView2 that is still broken
/// hands over to the classic window quickly instead of every start
/// waiting through the retries. `(attempts to create WebView2, failures
/// already counted)`: normally two attempts and none counted, so the
/// first failure of the page is retried and the second gives up; after a
/// fallback one attempt and one counted, so the first failure gives up.
/// A page that then works a while (`STABLE`) gets the second chances back.
pub fn start_tolerance(failed_last_time: bool) -> (u32, u32) {
    if failed_last_time { (1, 1) } else { (2, 0) }
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
    fn after_a_fallback_the_next_start_tries_again_quickly() {
        // Normal: two attempts, the first page failure is retried.
        assert_eq!(start_tolerance(false), (2, 0));
        // The classic window took over last time: still tried (never off
        // for good), but one attempt, and the first failure gives up.
        assert_eq!(start_tolerance(true), (1, 1));
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
        // The tests' classic window: its own switch, not --webui's.
        assert!(!classic_requested_by(&none, None));
        assert!(classic_requested_by(&["--classic"], None));
        assert!(!classic_requested_by(&["--webui"], None));
        assert!(!requested_by(&["--classic"], None));
        assert!(classic_requested_by(&none, Some("1")) && !classic_requested_by(&none, Some("0")));
    }

    #[test]
    fn the_new_look_unless_the_tests_ask_for_the_classic_window() {
        assert!(pick(true, false, false));
        assert!(pick(false, true, false));
        assert!(!pick(false, false, false));
        // --classic / BAHTH_CLASSIC win, also over --webui.
        assert!(!pick(true, false, true));
        assert!(!pick(true, true, true));
    }
}
