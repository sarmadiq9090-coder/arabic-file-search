//! Thin glue between the quick-search UI and `crate::launcher`: the UI only
//! calls the launcher's Windows-side functions through here.

use crate::launcher::{Catalog, SystemCommand, apps, settings_pages, system, windows, windows::OpenWindow};

/// The apps and settings catalog. Scans the Start menu apps, so it runs on a
/// background thread.
pub(super) fn new_catalog() -> Catalog {
    Catalog::new(apps::scan(), settings_pages::PAGES.to_vec())
}

/// Visible top-level windows of other programs; called each time the popup opens.
pub(super) fn list_windows() -> Vec<OpenWindow> {
    windows::list()
}

/// Brings a window to the front (restoring it when minimised). True when it worked.
pub(super) fn focus_window(hwnd: isize) -> bool {
    windows::focus(hwnd)
}

/// The question asked before running `cmd`, in the interface language.
pub(super) fn confirm_text(cmd: &SystemCommand) -> String {
    system::confirm_text(cmd, crate::i18n::is_en())
}

/// Why `cmd` must not run (shown instead of the question), if it mustn't.
pub(super) fn refusal(cmd: &SystemCommand) -> Option<String> {
    system::refusal(cmd, crate::i18n::is_en())
}

/// Whether `cmd` is run on a background thread (see `quick::run_action`).
pub(super) fn runs_in_background(cmd: &SystemCommand) -> bool {
    system::runs_in_background(cmd)
}

/// The message when `cmd` failed.
pub(super) fn failed_text(cmd: &SystemCommand) -> String {
    system::failed_text(cmd, crate::i18n::is_en())
}

/// Runs a command the user confirmed. False (and a log line) when it failed.
pub(super) fn run_system(cmd: &SystemCommand) -> bool {
    match system::run(cmd) {
        Ok(()) => true,
        Err(e) => {
            super::startup::log(&format!("quick: system command {cmd:?} failed: {e}"));
            false
        }
    }
}
