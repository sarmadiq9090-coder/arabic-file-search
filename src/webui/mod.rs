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

/// How a start that may create the new look's page goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartLook {
    /// The new look; `careful`: as after a fallback ([`start_tolerance`]).
    New { careful: bool },
    /// The classic window for this run, saying why, as after a fallback.
    Classic,
}

/// A hard crash (an access violation in WebView2's loader or in the page
/// path, a stack overflow) leaves no fallback behind: `new_ui_failed` is
/// set only when the classic window takes over in an orderly way. So a
/// start saves `new_ui_trying` ([`trying_before_page`]) right before it
/// creates the page, and the page's hello clears it: a start that finds it
/// set knows the page never said hello at the last starts. `trying` is that
/// count, `failed_last_time` is `new_ui_failed`:
/// - none: as usual (careful after a fallback);
/// - one: the new look, careful, as after a fallback;
/// - two or more: the classic window for this run; the count goes back to
///   [`TRYING_AFTER_CLASSIC`], so the next start tries the new look again
///   (carefully), and one more crash gives the classic window again. Never
///   a loop of crashes, and the new look is never turned off for good.
pub fn start_look(trying: u32, failed_last_time: bool) -> StartLook {
    match trying {
        0 => StartLook::New { careful: failed_last_time },
        1 => StartLook::New { careful: true },
        _ => StartLook::Classic,
    }
}

/// `new_ui_trying` to save right before the page is created (once per run).
pub fn trying_before_page(trying: u32) -> u32 {
    trying.saturating_add(1).min(MAX_TRYING)
}

/// `new_ui_trying` once the classic window took over after crashed starts:
/// one unclean start still remembered.
pub const TRYING_AFTER_CLASSIC: u32 = 1;

/// The largest `new_ui_trying` kept (anything from 2 means the same).
pub const MAX_TRYING: u32 = 2;

/// How long the window must have been hidden (to the tray), without a
/// break, before its WebView2 is closed altogether, so that its processes
/// exit (about 140 MB on a real PC); the next show creates the page again.
/// Until then the hidden page is only suspended ([`idle_action`]).
pub const RELEASE_AFTER: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// For the test of the release (`ci/newlook-test.ps1`): [`RELEASE_AFTER`]
/// in seconds instead (`BAHTH_WEBUI_RELEASE_SECS=20`), never under
/// [`RELEASE_MIN_SECS`]. Not offered to users.
pub const RELEASE_ENV: &str = "BAHTH_WEBUI_RELEASE_SECS";

/// The shortest release time [`RELEASE_ENV`] can set.
pub const RELEASE_MIN_SECS: u64 = 5;

/// [`RELEASE_AFTER`], or what [`RELEASE_ENV`] says (`env`: its value).
pub fn release_after_by(env: Option<&str>) -> std::time::Duration {
    env.and_then(|v| v.trim().parse::<u64>().ok())
        .map_or(RELEASE_AFTER, |s| std::time::Duration::from_secs(s.max(RELEASE_MIN_SECS)))
}

/// [`release_after_by`] for this process (read once).
pub fn release_after() -> std::time::Duration {
    static AFTER: std::sync::OnceLock<std::time::Duration> = std::sync::OnceLock::new();
    *AFTER.get_or_init(|| release_after_by(std::env::var(RELEASE_ENV).ok().as_deref()))
}

/// How a duration reads in the log: "10 min", or "20 s" when not whole minutes.
pub fn log_duration(d: std::time::Duration) -> String {
    let s = d.as_secs();
    if s >= 60 && s.is_multiple_of(60) { format!("{} min", s / 60) } else { format!("{s} s") }
}

/// The page of the hidden window, as the program's timer sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Idle {
    /// How long the window has been hidden without a break; `None`: on screen.
    pub hidden_for: Option<std::time::Duration>,
    /// There is a page (WebView2) in the window.
    pub has_page: bool,
    /// It was already asked to suspend since the window was hidden.
    pub suspend_asked: bool,
    /// A Save/Open dialog waits to be moved to a result's folder
    /// (Ctrl+Alt+G): the page is part of that hand-off.
    pub hand_off: bool,
    /// A modal dialog or menu is open, or the page is being created, or a
    /// dialog runs inside the page's own message handler.
    pub modal: bool,
    /// A search inside documents is still running for the page.
    pub busy: bool,
}

/// What to do with the page of a hidden window ([`idle_action`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleAction {
    /// Nothing (on screen, no page, already done, or not now).
    Keep,
    /// Freeze its processes (`TrySuspend`): kept in memory, back at once.
    Suspend,
    /// Close its WebView2, so its processes exit; created again on show.
    Release,
}

/// The decision of the timer for the page of a window that may be hidden:
/// suspended as soon as it is hidden, closed once it has been hidden for
/// `release_after`. Never while a Save/Open dialog's hand-off is under way,
/// while a modal is open, or while a search inside documents runs; then it
/// waits (the hidden time still counts, so the release comes once that is over).
pub fn idle_action(idle: Idle, release_after: std::time::Duration) -> IdleAction {
    let Some(hidden_for) = idle.hidden_for else { return IdleAction::Keep };
    if !idle.has_page || idle.hand_off || idle.modal {
        return IdleAction::Keep;
    }
    if hidden_for >= release_after && !idle.busy {
        return IdleAction::Release;
    }
    if idle.suspend_asked || idle.busy {
        return IdleAction::Keep;
    }
    IdleAction::Suspend
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
    fn crashed_starts_give_the_classic_window_for_one_run() {
        assert_eq!(start_look(0, false), StartLook::New { careful: false });
        assert_eq!(start_look(0, true), StartLook::New { careful: true });
        assert_eq!(start_look(1, false), StartLook::New { careful: true });
        assert_eq!(start_look(2, false), StartLook::Classic);
        assert_eq!(start_look(u32::MAX, true), StartLook::Classic);
        assert_eq!(trying_before_page(0), 1);
        assert_eq!(trying_before_page(1), 2);
        assert_eq!(trying_before_page(MAX_TRYING), MAX_TRYING);
        assert_eq!(trying_before_page(u32::MAX), MAX_TRYING);

        // Starts in a row, the way the window saves the marker: `crash`
        // says whether this start dies before the page says hello.
        let run = |crashes: &[bool]| {
            let (mut trying, mut failed) = (0u32, false);
            let mut looks = Vec::new();
            for &crash in crashes {
                let look = start_look(trying, failed);
                looks.push(look);
                match look {
                    StartLook::Classic => {
                        trying = TRYING_AFTER_CLASSIC;
                        failed = true; // Settings::note_fallback
                    }
                    StartLook::New { .. } => {
                        trying = trying_before_page(trying);
                        if !crash {
                            trying = 0; // the page said hello
                            failed = false; // and worked a while (webui_forgive)
                        }
                    }
                }
            }
            looks
        };
        use StartLook::{Classic, New};
        let careful = New { careful: true };
        // A page that always crashes: two crashes, then the classic window
        // every other start; the new look is still tried every other start.
        let looks = run(&[true; 9]);
        assert_eq!(looks, [New { careful: false }, careful, Classic, careful, Classic, careful, Classic, careful, Classic]);
        assert!(looks.windows(2).all(|w| w != [Classic, Classic]));
        // One crash: the next start is careful, and once the page works, normal again.
        assert_eq!(run(&[true, false, false]), [New { careful: false }, careful, New { careful: false }]);
        // Fixed after the classic run: the new look is back for good.
        assert_eq!(run(&[true, true, false, false, false]), [New { careful: false }, careful, Classic, careful, New { careful: false }]);
        // Never crashing: never the classic window.
        assert!(run(&[false; 5]).iter().all(|&l| l == New { careful: false }));
    }

    #[test]
    fn a_hidden_page_is_suspended_then_released() {
        use std::time::Duration;
        let after = RELEASE_AFTER;
        let hidden = |secs: u64| Idle { hidden_for: Some(Duration::from_secs(secs)), has_page: true, ..Idle::default() };
        // On screen, or no page (a start in the tray, or already released): nothing.
        assert_eq!(idle_action(Idle { has_page: true, ..Idle::default() }, after), IdleAction::Keep);
        assert_eq!(idle_action(Idle { has_page: false, ..hidden(3600) }, after), IdleAction::Keep);
        // Just hidden: suspended, once.
        assert_eq!(idle_action(hidden(0), after), IdleAction::Suspend);
        assert_eq!(idle_action(Idle { suspend_asked: true, ..hidden(5) }, after), IdleAction::Keep);
        assert_eq!(idle_action(Idle { suspend_asked: true, ..hidden(599) }, after), IdleAction::Keep);
        // Hidden ten minutes without a break: closed, suspended or not.
        assert_eq!(idle_action(Idle { suspend_asked: true, ..hidden(600) }, after), IdleAction::Release);
        assert_eq!(idle_action(hidden(600), after), IdleAction::Release);
        assert_eq!(idle_action(Idle { suspend_asked: true, ..hidden(86_400) }, after), IdleAction::Release);
        // Never during a Save/Open dialog's hand-off or under a modal: it waits.
        for held in [Idle { hand_off: true, ..hidden(0) }, Idle { modal: true, ..hidden(0) }] {
            assert_eq!(idle_action(held, after), IdleAction::Keep);
            assert_eq!(idle_action(Idle { hidden_for: Some(Duration::from_secs(3600)), ..held }, after), IdleAction::Keep);
        }
        assert_eq!(idle_action(Idle { hand_off: true, modal: true, suspend_asked: true, ..hidden(3600) }, after), IdleAction::Keep);
        // A search inside documents still running: neither, until it ends.
        assert_eq!(idle_action(Idle { busy: true, ..hidden(0) }, after), IdleAction::Keep);
        assert_eq!(idle_action(Idle { busy: true, ..hidden(3600) }, after), IdleAction::Keep);
        // The test's shorter time.
        assert_eq!(idle_action(Idle { suspend_asked: true, ..hidden(20) }, Duration::from_secs(20)), IdleAction::Release);
    }

    #[test]
    fn release_time_and_its_test_switch() {
        use std::time::Duration;
        assert_eq!(RELEASE_AFTER, Duration::from_secs(600));
        assert_eq!(release_after_by(None), RELEASE_AFTER);
        assert_eq!(release_after_by(Some("20")), Duration::from_secs(20));
        assert_eq!(release_after_by(Some(" 90 ")), Duration::from_secs(90));
        // Never shorter than a few seconds; nonsense keeps the ten minutes.
        assert_eq!(release_after_by(Some("0")), Duration::from_secs(RELEASE_MIN_SECS));
        for bad in ["", "x", "-5", "1.5"] {
            assert_eq!(release_after_by(Some(bad)), RELEASE_AFTER, "{bad}");
        }
        assert_eq!(log_duration(RELEASE_AFTER), "10 min");
        assert_eq!(log_duration(Duration::from_secs(20)), "20 s");
        assert_eq!(log_duration(Duration::from_secs(90)), "90 s");
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
