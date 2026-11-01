//! What the quick search finds besides files: installed apps, Windows
//! settings pages, open windows, instant answers (calculator, units, Hijri
//! date), web shortcuts and system commands.
//!
//! Everything here is platform independent and unit-tested on Linux, except
//! the parts that ask Windows for data (`apps::scan`, `windows::list`), which
//! are `cfg(windows)` inside their modules.
//!
//! The UI calls [`Catalog::query`] on every keystroke, so it must stay well
//! under a millisecond for a few hundred entries.

pub mod answers;
pub mod apps;
pub mod settings_pages;
pub mod system;
pub mod web;
pub mod windows;

/// Which group a result is shown in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// An instant answer (calculation, unit conversion, date).
    Answer,
    /// An open window of a running program.
    Window,
    /// An installed program (Start menu shortcut or Store app).
    App,
    /// A Windows settings page.
    Setting,
    /// A system command (shut down, lock…); always confirmed first.
    Command,
    /// Search a website for the query.
    Web,
}

/// A command that changes the state of the computer. The UI must ask for
/// confirmation before running any of these.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SystemCommand {
    Shutdown,
    Restart,
    Sleep,
    Lock,
    SignOut,
    EmptyRecycleBin,
    /// End a program: process id and its executable name, for the prompt.
    EndProcess { pid: u32, name: String },
}

/// What happens when a result is opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Start a program: a `.lnk`/`.exe` path, or `shell:AppsFolder\<AUMID>`
    /// for a Store app.
    Launch { target: String, args: String },
    /// Open a URI with the shell: `ms-settings:…`, `https://…`.
    OpenUri(String),
    /// Copy text to the clipboard (calculator result).
    CopyText(String),
    /// Bring an open window to the front (HWND as an integer).
    FocusWindow(isize),
    /// Run a system command after the user confirms it.
    System(SystemCommand),
}

/// One row of the quick-search results.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub kind: Kind,
    /// Main text, shown in the interface language where one exists.
    pub title: String,
    /// Second line: a path, the settings section, "Store app"…
    pub subtitle: String,
    pub action: Action,
    /// Higher is better; only comparable within one query.
    pub score: u32,
    /// File the icon is taken from (exe, lnk), if any.
    pub icon_path: Option<String>,
}

/// The data the quick search looks through besides files, built once in the
/// background and then queried on every keystroke.
#[derive(Default)]
pub struct Catalog {
    pub apps: Vec<apps::App>,
    pub settings: Vec<settings_pages::Page>,
}

/// What the caller knows about the moment of the query.
pub struct Context<'a> {
    /// The interface language is English (titles and answers follow it).
    pub english: bool,
    /// Windows build number, to hide settings pages that do not exist (0 = unknown, show all).
    pub windows_build: u32,
    /// Open windows, listed by the UI just before querying (empty on Linux tests).
    pub windows: &'a [windows::OpenWindow],
    /// How often the user opened a target before (`Action` key → count), for the boost.
    pub launches: &'a dyn Fn(&str) -> u32,
    /// Today's date (year, month, day), for the Hijri answer.
    pub today: (i32, u32, u32),
}

impl Catalog {
    /// Results for `query`, best first, at most `limit`.
    pub fn query(&self, query: &str, ctx: &Context, limit: usize) -> Vec<Item> {
        let _ = (query, ctx, limit);
        Vec::new()
    }
}

/// A stable key for an action, used to count launches for the boost.
pub fn action_key(action: &Action) -> String {
    match action {
        Action::Launch { target, .. } => format!("launch:{}", target.to_lowercase()),
        Action::OpenUri(u) => format!("uri:{u}"),
        Action::CopyText(_) => String::new(),
        Action::FocusWindow(_) => String::new(),
        Action::System(c) => format!("sys:{c:?}"),
    }
}
