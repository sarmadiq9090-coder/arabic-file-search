//! Query parsing, filename search over the index, and content search inside documents.

use rayon::prelude::*;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::extract;
use crate::index::Entry;
use crate::normalize::{normalize, normalize_with_map};

/// Documents bigger than this are skipped by the content search.
const MAX_CONTENT_SIZE: u64 = 100 * 1024 * 1024;

/// A parsed search query.
///
/// Words are AND-ed together, `"quoted text"` is matched as one phrase and
/// `ext:pdf`, `*.pdf` or `.pdf` restricts the file type.
#[derive(Debug, Default, PartialEq)]
pub struct Query {
    pub terms: Vec<String>,
    pub exts: Vec<String>,
}

impl Query {
    pub fn parse(input: &str) -> Query {
        let mut q = Query::default();
        let mut tokens = Vec::new();
        let mut rest = input.trim();
        while !rest.is_empty() {
            if let Some(stripped) = rest.strip_prefix('"') {
                let end = stripped.find('"').unwrap_or(stripped.len());
                tokens.push(stripped[..end].to_string());
                rest = stripped.get(end + 1..).unwrap_or("").trim_start();
            } else {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                tokens.push(rest[..end].to_string());
                rest = rest[end..].trim_start();
            }
        }
        for tok in tokens {
            let lower = tok.to_lowercase();
            let ext = lower
                .strip_prefix("ext:")
                .or_else(|| lower.strip_prefix("*."))
                .or_else(|| lower.strip_prefix('.').filter(|e| !e.is_empty() && !e.contains('.')));
            if let Some(ext) = ext {
                q.exts.extend(ext.split(',').filter(|e| !e.is_empty()).map(str::to_string));
            } else {
                let n = normalize(&tok);
                if !n.trim().is_empty() {
                    q.terms.push(n);
                }
            }
        }
        q
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.exts.is_empty()
    }

    fn ext_ok(&self, e: &Entry) -> bool {
        self.exts.is_empty() || (!e.is_dir && self.exts.contains(&e.ext()))
    }
}

/// Case-insensitive "is `path` inside `scope`" check. An empty scope means everything.
pub fn in_scope(path: &str, scope_lower: &str) -> bool {
    if scope_lower.is_empty() {
        return true;
    }
    let n = scope_lower.len();
    path.len() >= n && path.is_char_boundary(n) && path[..n].to_lowercase() == scope_lower
}

/// Returns indices into `entries` whose file name contains every term,
/// best matches first (exact name, then names starting with the query).
pub fn search_names(entries: &[Entry], q: &Query, scope: &str) -> Vec<u32> {
    if q.is_empty() {
        return Vec::new();
    }
    let scope_lower = scope.to_lowercase();
    let mut hits: Vec<(u8, u32, u32)> = entries
        .par_iter()
        .enumerate()
        .filter(|(_, e)| q.terms.iter().all(|t| e.norm_name.contains(t.as_str())))
        .filter(|(_, e)| q.ext_ok(e) && in_scope(&e.path, &scope_lower))
        .map(|(i, e)| {
            let rank = match q.terms.first() {
                Some(t) if e.norm_name.as_ref() == t => 0,
                Some(t) if e.norm_name.starts_with(t.as_str()) => 1,
                _ => 2,
            };
            (rank, e.norm_name.chars().count() as u32, i as u32)
        })
        .collect();
    hits.par_sort_unstable();
    hits.into_iter().map(|(_, _, i)| i).collect()
}

/// One content-search result.
pub struct ContentHit {
    pub index: u32,
    /// A short piece of the document text around the first match.
    pub snippet: String,
}

#[derive(Default)]
pub struct ContentProgress {
    pub total: AtomicUsize,
    pub checked: AtomicUsize,
    pub cancel: AtomicBool,
    pub done: AtomicBool,
}

/// Searches inside every supported document in scope. Hits are pushed into
/// `hits` as they are found so the UI can show them while the search runs.
pub fn search_content(
    entries: &[Entry],
    q: &Query,
    scope: &str,
    progress: &ContentProgress,
    hits: &Mutex<Vec<ContentHit>>,
) {
    let scope_lower = scope.to_lowercase();
    let candidates: Vec<u32> = entries
        .par_iter()
        .enumerate()
        .filter(|(_, e)| !e.is_dir && e.size > 0 && e.size <= MAX_CONTENT_SIZE)
        .filter(|(_, e)| extract::is_supported(&e.ext()) && q.ext_ok(e))
        .filter(|(_, e)| in_scope(&e.path, &scope_lower))
        .map(|(i, _)| i as u32)
        .collect();
    progress.total.store(candidates.len(), Ordering::Relaxed);

    if q.terms.is_empty() {
        progress.done.store(true, Ordering::Relaxed);
        return;
    }

    // A dedicated pool keeps the global one free for instant filename search.
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get().saturating_sub(1).max(1));
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build();
    let run = || {
        candidates.par_iter().for_each(|&i| {
            if progress.cancel.load(Ordering::Relaxed) {
                return;
            }
            let entry = &entries[i as usize];
            if let Some(text) = extract::extract_text(Path::new(&*entry.path))
                && let Some(snippet) = match_text(&text, &q.terms)
            {
                if let Ok(mut h) = hits.lock() {
                    h.push(ContentHit { index: i, snippet });
                }
            }
            progress.checked.fetch_add(1, Ordering::Relaxed);
        })
    };
    match pool {
        Ok(pool) => pool.install(run),
        Err(_) => run(),
    }
    progress.done.store(true, Ordering::Relaxed);
}

/// If `text` contains every term, returns a snippet around the first one.
pub fn match_text(text: &str, terms: &[String]) -> Option<String> {
    let (norm, map) = normalize_with_map(text);
    let mut first = None;
    for t in terms {
        let pos = norm.find(t.as_str())?;
        first.get_or_insert(pos);
    }
    let pos = map[first?] as usize;
    Some(snippet_at(text, pos))
}

/// About 40 characters either side of byte offset `pos`, on one line.
fn snippet_at(text: &str, pos: usize) -> String {
    const CONTEXT: usize = 40;
    let start = text[..pos].char_indices().rev().nth(CONTEXT).map_or(0, |(i, _)| i);
    let end = text[pos..].char_indices().nth(CONTEXT * 2).map_or(text.len(), |(i, _)| pos + i);
    let mut s: String = text[start..end].split_whitespace().collect::<Vec<_>>().join(" ");
    if start > 0 {
        s.insert(0, '…');
    }
    if end < text.len() {
        s.push('…');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, is_dir: bool) -> Entry {
        let name = path.rsplit(['\\', '/']).next().unwrap();
        Entry { path: path.into(), size: 1, mtime: 0, is_dir, norm_name: normalize(name).into() }
    }

    #[test]
    fn parse_query() {
        let q = Query::parse(r#"أحمد "تقرير المبيعات" ext:pdf,docx *.txt"#);
        assert_eq!(q.terms, vec!["احمد", "تقرير المبيعات"]);
        assert_eq!(q.exts, vec!["pdf", "docx", "txt"]);
        assert_eq!(Query::parse(".xlsx").exts, vec!["xlsx"]);
    }

    #[test]
    fn name_search_ranking_and_filters() {
        let entries = vec![
            entry(r"C:\Docs\ملخص تقرير.docx", false),
            entry(r"C:\Docs\تقرير.pdf", false),
            entry(r"D:\تقارير\تقرير السنة.pdf", false),
            entry(r"C:\Docs\تقرير", true),
        ];
        let r = search_names(&entries, &Query::parse("التقرير"), "");
        assert!(r.is_empty());
        let r = search_names(&entries, &Query::parse("تقرير"), "");
        assert_eq!(r[0], 3); // exact name first
        assert_eq!(r.len(), 4);
        let r = search_names(&entries, &Query::parse("تقرير .pdf"), r"c:\docs");
        assert_eq!(r, vec![1]);
        let r = search_names(&entries, &Query::parse("ext:pdf"), "");
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn content_match_with_snippet() {
        let text = "هذا نص طويل. وفيه التَّقرير السنوي للمبيعات لعام ٢٠٢٤ مع تفاصيل.";
        let s = match_text(text, &Query::parse("تقرير 2024").terms).unwrap();
        assert!(s.contains("التَّقرير"), "{s}");
        assert!(match_text(text, &Query::parse("غير موجود").terms).is_none());
    }
}
