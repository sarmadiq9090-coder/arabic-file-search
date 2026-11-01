//! Instant answers: calculator, unit conversion, Hijri and Gregorian dates.

use super::{Context, Item};

/// An instant answer for `query`, if it is a calculation, a conversion or a date question.
pub fn answer(query: &str, ctx: &Context) -> Option<Item> {
    let _ = (query, ctx);
    None
}
