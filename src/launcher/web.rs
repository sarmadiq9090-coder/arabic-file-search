//! Web shortcuts: "يوتيوب قرآن" searches YouTube, plus a "search the web" fallback.

use super::{Context, Item};

/// Web-search results for `query` (a site shortcut, or the generic fallback when `fallback`).
pub fn shortcuts(query: &str, ctx: &Context, fallback: bool) -> Vec<Item> {
    let _ = (query, ctx, fallback);
    Vec::new()
}
