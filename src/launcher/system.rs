//! System commands (shut down, restart, lock…), always confirmed by the UI before running.

use super::{Context, Item};

/// Commands whose words match `query`, including "end <program>" for open windows.
pub fn commands(query: &str, ctx: &Context) -> Vec<Item> {
    let _ = (query, ctx);
    Vec::new()
}
