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

#[cfg(test)]
mod tests {
    use super::*;

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
