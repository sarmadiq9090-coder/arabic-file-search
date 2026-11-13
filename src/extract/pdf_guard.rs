//! Refuses the PDFs that pdf-extract cannot walk within a bounded stack.
//!
//! pdf-extract (0.12) recurses in two places with no limit, and a stack
//! overflow ends the process whatever catches panics:
//! - `Do` runs the named XObject as a nested content stream, taking the
//!   stream's own `/Resources` or else the caller's. A form that draws
//!   itself, two forms drawing each other, or a chain of thousands of forms
//!   never returns; forms that each draw the next one twice return after
//!   2^depth runs.
//! - the page's `/Resources` and `/MediaBox` are looked up along `/Parent`,
//!   so a node that is its own parent (or a very long chain) never ends.
//!
//! [`bounded`] walks the same references on the parsed document first, with
//! a depth cap and a budget, following only the `Do` operators that are
//! really in each stream (resource dictionaries shared by many forms are
//! common and harmless). It over-approximates where pdf-extract would stop
//! with a panic, never the other way round.

use std::borrow::Cow;
use std::collections::HashMap;

use lopdf::content::Content;
use lopdf::{Dictionary, Document, Object, Stream};

/// XObjects nested deeper than this are refused. Real documents nest a
/// handful of levels; pdf-extract needs a few kilobytes of stack per level
/// (a debug build overflows 2 MB between 200 and 400 levels).
const MAX_FORM_DEPTH: u32 = 32;
/// Nested stream runs (beyond the XObjects a page draws directly) allowed
/// in one document.
const MAX_NESTED_RUNS: u64 = 1_000_000;
/// `/Parent` links followed from a page (lopdf stops at 256 levels of
/// `/Kids` on the way down).
const MAX_PARENTS: usize = 256;

/// What drawing a stream once sets off below it.
#[derive(Clone, Copy)]
struct Below {
    /// Levels of nested streams under it (0 for one that draws nothing).
    depth: u32,
    /// Stream runs under it.
    runs: u64,
}

/// The document refused: too deep, endless or too much.
struct Refused;

/// The object `o` stands for, as pdf-extract resolves it.
fn deref<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Object> {
    match o {
        Object::Reference(id) => doc.get_object(*id).ok(),
        _ => Some(o),
    }
}

fn dict_at<'a>(doc: &'a Document, dict: &'a Dictionary, key: &[u8]) -> Option<&'a Dictionary> {
    deref(doc, dict.get(key).ok()?)?.as_dict().ok()
}

/// The content pdf-extract hands its parser for an XObject.
fn content_of(s: &Stream) -> Cow<'_, [u8]> {
    if s.filters().is_ok() {
        s.decompressed_content().map_or(Cow::Borrowed(s.content.as_slice()), Cow::Owned)
    } else {
        Cow::Borrowed(&s.content)
    }
}

/// The XObject names drawn by `content`, in order, repeats included.
fn drawn(content: &[u8]) -> Vec<Vec<u8>> {
    // An operator is spelled out in the stream: no "Do", nothing drawn.
    if !content.windows(2).any(|w| w == b"Do") {
        return Vec::new();
    }
    let Ok(parsed) = Content::decode(content) else { return Vec::new() };
    parsed
        .operations
        .iter()
        .filter(|op| op.operator == "Do")
        .filter_map(|op| op.operands.first()?.as_name().ok().map(<[u8]>::to_vec))
        .collect()
}

struct Walk<'a> {
    doc: &'a Document,
    /// Per (stream, resources it runs with): what it sets off.
    seen: HashMap<(*const Stream, *const Dictionary), Below>,
}

impl<'a> Walk<'a> {
    /// What the stream `name` of `resources` sets off when drawn at nesting
    /// `level` (1 for an XObject drawn by a page). `Ok(None)` when
    /// pdf-extract would not find such a stream.
    fn draw(&mut self, resources: &'a Dictionary, name: &[u8], level: u32) -> Result<Option<Below>, Refused> {
        let Some(xobjects) = dict_at(self.doc, resources, b"XObject") else { return Ok(None) };
        let Some(Ok(stream)) = xobjects.get(name).ok().and_then(|o| deref(self.doc, o)).map(Object::as_stream) else {
            return Ok(None);
        };
        // A loop comes back here ever deeper, until this refuses it.
        if level > MAX_FORM_DEPTH {
            return Err(Refused);
        }
        let own = dict_at(self.doc, &stream.dict, b"Resources").unwrap_or(resources);
        let key = (std::ptr::from_ref(stream), std::ptr::from_ref(own));
        if let Some(&below) = self.seen.get(&key) {
            return if level + below.depth > MAX_FORM_DEPTH { Err(Refused) } else { Ok(Some(below)) };
        }
        let mut below = Below { depth: 0, runs: 0 };
        for child in drawn(&content_of(stream)) {
            if let Some(b) = self.draw(own, &child, level + 1)? {
                below.depth = below.depth.max(b.depth + 1);
                below.runs = below.runs.saturating_add(b.runs).saturating_add(1);
                if below.runs > MAX_NESTED_RUNS {
                    return Err(Refused);
                }
            }
        }
        self.seen.insert(key, below);
        Ok(Some(below))
    }
}

/// The page's inherited `/Resources`, the way pdf-extract finds them; `Err`
/// when its `/Parent` chain does not end within [`MAX_PARENTS`] links.
fn page_resources<'a>(doc: &'a Document, page: &'a Dictionary) -> Result<Option<&'a Dictionary>, Refused> {
    let mut resources = None;
    let mut node = page;
    for _ in 0..=MAX_PARENTS {
        if resources.is_none() {
            resources = dict_at(doc, node, b"Resources");
        }
        // The whole chain is walked: the MediaBox may be further up.
        let parent = node.get(b"Parent").and_then(Object::as_reference).and_then(|id| doc.get_dictionary(id));
        match parent {
            Ok(p) => node = p,
            Err(_) => return Ok(resources),
        }
    }
    Err(Refused)
}

/// True when pdf-extract can walk `doc` without recursing past
/// [`MAX_FORM_DEPTH`] nested XObjects, [`MAX_NESTED_RUNS`] nested stream
/// runs or [`MAX_PARENTS`] page-tree parents.
pub(super) fn bounded(doc: &Document) -> bool {
    check(doc).is_ok()
}

fn check(doc: &Document) -> Result<(), Refused> {
    let mut walk = Walk { doc, seen: HashMap::new() };
    let mut runs: u64 = 0;
    for (_, page_id) in doc.get_pages() {
        let Ok(page) = doc.get_dictionary(page_id) else { continue };
        let Some(resources) = page_resources(doc, page)? else { continue };
        let Some(xobjects) = dict_at(doc, resources, b"XObject") else { continue };
        // Every XObject the page could draw, without reading its content.
        let mut nested = false;
        for (name, _) in xobjects.iter() {
            nested |= walk.draw(resources, name, 1)?.is_some_and(|b| b.runs > 0);
        }
        // Only when one of them draws others does it matter how often the
        // page draws each.
        if nested {
            for name in drawn(&doc.get_page_content(page_id).unwrap_or_default()) {
                if let Some(b) = walk.draw(resources, &name, 1)? {
                    runs = runs.saturating_add(b.runs);
                    if runs > MAX_NESTED_RUNS {
                        return Err(Refused);
                    }
                }
            }
        }
    }
    Ok(())
}
