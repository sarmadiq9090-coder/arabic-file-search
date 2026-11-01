//! Open top-level windows, for "switch to" results.

/// One visible top-level window.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenWindow {
    pub hwnd: isize,
    pub title: String,
    /// Executable file name ("WINWORD.EXE").
    pub exe: String,
    pub pid: u32,
}

use super::{Context, Item};

/// "Switch to" results for open windows whose title or program matches `query`.
pub fn matches(query: &str, ctx: &Context) -> Vec<Item> {
    let _ = (query, ctx);
    Vec::new()
}
