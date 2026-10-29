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

/// One search term: it matches when any of its alternatives is contained.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Term {
    /// Normalised alternatives; never empty.
    pub alts: Vec<String>,
}

impl Term {
    /// Byte position of the earliest alternative found in `hay`.
    fn find_in(&self, hay: &str) -> Option<usize> {
        self.alts.iter().filter_map(|a| hay.find(a.as_str())).min()
    }

    fn matches(&self, hay: &str) -> bool {
        self.alts.iter().any(|a| hay.contains(a.as_str()))
    }
}

/// A parsed search query.
///
/// Words are AND-ed together, `"quoted text"` is matched as one phrase,
/// `a|b` (or `a | b`) matches either, `!word` or `-word` excludes, `ext:pdf`,
/// `*.pdf` or `.pdf` restricts the file type and `folder:` / `مجلد:` keeps
/// only folders. A leading Arabic article (`ال`, `وال`, `لل`…) is optional.
#[derive(Debug, Default, PartialEq)]
pub struct Query {
    pub terms: Vec<Term>,
    pub excludes: Vec<String>,
    pub exts: Vec<String>,
    pub dirs_only: bool,
}

/// File-type presets for the type filter: (label, extensions). An empty list
/// means no extension filter; index [`KIND_FOLDERS`] means folders only.
pub const KINDS: &[(&str, &[&str])] = &[
    ("كل الأنواع", &[]),
    (
        "مستندات",
        &["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "txt", "csv", "md", "epub"],
    ),
    ("صور", &["jpg", "jpeg", "png", "gif", "bmp", "webp", "heic", "tif", "tiff", "svg"]),
    ("صوت", &["mp3", "wav", "m4a", "aac", "flac", "ogg", "wma", "opus"]),
    ("فيديو", &["mp4", "mkv", "avi", "mov", "wmv", "webm", "m4v", "3gp"]),
    ("مضغوطات", &["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "iso"]),
    ("برامج", &["exe", "msi", "bat", "cmd", "ps1", "lnk", "appx", "msix"]),
    ("مجلدات", &[]),
];

/// Index of the "folders only" entry in [`KINDS`].
pub const KIND_FOLDERS: usize = 7;

/// An extension no file has, used when filters leave no allowed type.
const NO_EXT: &str = "\0";

/// Arabic prefixes stripped to also match the bare word, longest first.
const ARTICLES: [&str; 6] = ["وال", "بال", "كال", "فال", "لل", "ال"];

/// Splits the input into (token, quoted) pairs.
fn tokenize(input: &str) -> Vec<(String, bool)> {
    let mut tokens = Vec::new();
    let mut rest = input.trim();
    while !rest.is_empty() {
        if let Some(stripped) = rest.strip_prefix('"') {
            let end = stripped.find('"').unwrap_or(stripped.len());
            tokens.push((stripped[..end].to_string(), true));
            rest = stripped.get(end + 1..).unwrap_or("").trim_start();
        } else {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            tokens.push((rest[..end].to_string(), false));
            rest = rest[end..].trim_start();
        }
    }
    tokens
}

/// The extension list of an `ext:`, `*.` or `.ext` token.
fn ext_filter(lower: &str) -> Option<&str> {
    lower
        .strip_prefix("ext:")
        .or_else(|| lower.strip_prefix("*."))
        .or_else(|| lower.strip_prefix('.').filter(|e| !e.is_empty() && !e.contains('.')))
}

/// Adds `alt` (already normalised) and, if it starts with an Arabic article
/// that leaves at least 3 letters, the bare word too.
fn push_alt(alts: &mut Vec<String>, alt: String) {
    let bare =
        ARTICLES.iter().find_map(|p| alt.strip_prefix(p)).filter(|rest| rest.chars().count() >= 3).map(str::to_string);
    for a in std::iter::once(alt).chain(bare) {
        if !alts.contains(&a) {
            alts.push(a);
        }
    }
}

impl Query {
    pub fn parse(input: &str) -> Query {
        let mut q = Query::default();
        // Set by a trailing or standalone `|`: the next term joins the last one.
        let mut join = false;
        for (tok, quoted) in tokenize(input) {
            let mut pieces: Vec<&str> = vec![&tok];
            if !quoted {
                let lower = tok.to_lowercase();
                if let Some(ext) = ext_filter(&lower) {
                    let exts = ext.split([',', ';']).filter(|e| !e.is_empty());
                    q.exts.extend(exts.map(str::to_string));
                    continue;
                }
                let folder =
                    ["folder:", "مجلد:"].iter().find(|p| tok.get(..p.len()).is_some_and(|h| h.eq_ignore_ascii_case(p)));
                if let Some(p) = folder {
                    q.dirs_only = true;
                    if tok.len() == p.len() {
                        continue;
                    }
                    pieces = vec![&tok[p.len()..]];
                } else if let Some(word) = tok
                    .strip_prefix('!')
                    .or_else(|| tok.strip_prefix('-').filter(|w| w.starts_with(char::is_alphabetic)))
                {
                    let n = normalize(word);
                    if !n.trim().is_empty() {
                        q.excludes.push(n);
                        continue;
                    }
                } else if tok.contains('|') {
                    pieces = tok.split('|').collect();
                }
            }
            let mut alts = Vec::new();
            for p in &pieces {
                let n = normalize(p);
                if !n.trim().is_empty() {
                    push_alt(&mut alts, n);
                }
            }
            let join_prev = join || pieces.first() == Some(&"") && pieces.len() > 1;
            join = pieces.len() > 1 && pieces.last() == Some(&"");
            if alts.is_empty() {
                join |= join_prev;
                continue;
            }
            match q.terms.last_mut() {
                Some(last) if join_prev => {
                    for a in alts {
                        if !last.alts.contains(&a) {
                            last.alts.push(a);
                        }
                    }
                }
                _ => q.terms.push(Term { alts }),
            }
        }
        q
    }

    /// True when nothing that narrows the results was typed: no terms, no
    /// excludes and no extensions. `dirs_only` alone does not count, so an
    /// empty box never lists the whole disk.
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.exts.is_empty() && self.excludes.is_empty()
    }

    /// Applies a type preset from [`KINDS`]. Index 0 does nothing,
    /// [`KIND_FOLDERS`] keeps only folders, the others restrict `exts` to the
    /// preset (intersected with typed extensions, matching nothing if they
    /// don't overlap). An empty query stays empty so that picking a type in
    /// an empty box still lists nothing.
    pub fn apply_kind(&mut self, kind: usize) {
        if self.is_empty() {
            return;
        }
        match kind {
            KIND_FOLDERS => self.dirs_only = true,
            k => {
                let Some(&(_, preset)) = KINDS.get(k) else { return };
                if preset.is_empty() {
                    return;
                }
                if self.exts.is_empty() {
                    self.exts = preset.iter().map(|e| e.to_string()).collect();
                } else {
                    self.exts.retain(|e| preset.contains(&e.as_str()));
                    if self.exts.is_empty() {
                        self.exts.push(NO_EXT.to_string());
                    }
                }
            }
        }
    }

    fn ext_ok(&self, e: &Entry) -> bool {
        self.exts.is_empty() || (!e.is_dir && self.exts.contains(&e.ext()))
    }

    /// Type, folder and excluded-word filters on an entry's name.
    fn entry_ok(&self, e: &Entry) -> bool {
        (!self.dirs_only || e.is_dir)
            && self.ext_ok(e)
            && !self.excludes.iter().any(|x| e.norm_name.contains(x.as_str()))
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

/// Returns indices into `entries` whose file name matches every term and
/// passes the filters, best matches first (exact name, then names starting
/// with the first term).
pub fn search_names(entries: &[Entry], q: &Query, scope: &str) -> Vec<u32> {
    if q.is_empty() {
        return Vec::new();
    }
    let scope_lower = scope.to_lowercase();
    let mut hits: Vec<(u8, u32, u32)> = entries
        .par_iter()
        .enumerate()
        .filter(|(_, e)| q.terms.iter().all(|t| t.matches(&e.norm_name)))
        .filter(|(_, e)| q.entry_ok(e) && in_scope(&e.path, &scope_lower))
        .map(|(i, e)| {
            let name: &str = &e.norm_name;
            let rank = match q.terms.first() {
                Some(t) if t.alts.iter().any(|a| name == a) => 0,
                Some(t) if t.alts.iter().any(|a| name.starts_with(a.as_str())) => 1,
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
        .filter(|(_, e)| extract::is_supported(&e.ext()) && q.entry_ok(e))
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
                && let Some(snippet) = match_text(&text, q)
                && let Ok(mut h) = hits.lock()
            {
                h.push(ContentHit { index: i, snippet });
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

/// If `text` matches every term and contains no excluded word, returns a
/// snippet around the earliest match.
pub fn match_text(text: &str, q: &Query) -> Option<String> {
    let (norm, map) = normalize_with_map(text);
    if q.excludes.iter().any(|x| norm.contains(x.as_str())) {
        return None;
    }
    let mut first: Option<usize> = None;
    for t in &q.terms {
        let pos = t.find_in(&norm)?;
        first = Some(first.map_or(pos, |f| f.min(pos)));
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

    fn alts(q: &Query) -> Vec<Vec<&str>> {
        q.terms.iter().map(|t| t.alts.iter().map(String::as_str).collect()).collect()
    }

    #[test]
    fn parse_query() {
        let q = Query::parse(r#"أحمد "تقرير المبيعات" ext:pdf,docx *.txt"#);
        assert_eq!(alts(&q), vec![vec!["احمد"], vec!["تقرير المبيعات"]]);
        assert_eq!(q.exts, vec!["pdf", "docx", "txt"]);
        assert_eq!(Query::parse(".xlsx").exts, vec!["xlsx"]);
        assert_eq!(Query::parse("ext:jpg;png").exts, vec!["jpg", "png"]);
        assert!(!q.dirs_only);
    }

    #[test]
    fn parse_alternatives() {
        assert_eq!(alts(&Query::parse("a|b|c d")), vec![vec!["a", "b", "c"], vec!["d"]]);
        assert_eq!(alts(&Query::parse("a | b x")), vec![vec!["a", "b"], vec!["x"]]);
        assert_eq!(alts(&Query::parse("a| b")), vec![vec!["a", "b"]]);
        assert_eq!(alts(&Query::parse("a |b")), vec![vec!["a", "b"]]);
        assert_eq!(alts(&Query::parse(r#"x | "a b""#)), vec![vec!["x", "a b"]]);
        assert_eq!(alts(&Query::parse("| a")), vec![vec!["a"]]);
    }

    #[test]
    fn parse_excludes_and_folders() {
        let q = Query::parse("تقرير !مسودة -Old - 2023-01");
        assert_eq!(alts(&q), vec![vec!["تقرير"], vec!["-"], vec!["2023-01"]]);
        assert_eq!(q.excludes, vec!["مسوده", "old"]);
        assert!(Query::parse("folder: صور").dirs_only);
        let q = Query::parse("مجلد:صور");
        assert!(q.dirs_only);
        assert_eq!(alts(&q), vec![vec!["صور"]]);
        assert!(Query::parse("FOLDER:").dirs_only);
        assert!(Query::parse("folder:").is_empty());
    }

    #[test]
    fn parse_arabic_article() {
        assert_eq!(alts(&Query::parse("التقرير")), vec![vec!["التقرير", "تقرير"]]);
        assert_eq!(alts(&Query::parse("والكتاب")), vec![vec!["والكتاب", "كتاب"]]);
        assert_eq!(alts(&Query::parse("للطلاب")), vec![vec!["للطلاب", "طلاب"]]);
        // Too short once stripped, or only a one-letter prefix.
        assert_eq!(alts(&Query::parse("الله")), vec![vec!["الله"]]);
        assert_eq!(alts(&Query::parse("بكتاب")), vec![vec!["بكتاب"]]);
    }

    #[test]
    fn kinds() {
        assert_eq!(KINDS.len(), 8);
        assert_eq!(KINDS[KIND_FOLDERS].0, "مجلدات");
        let mut q = Query::parse("تقرير");
        q.apply_kind(0);
        assert_eq!(q, Query::parse("تقرير"));
        q.apply_kind(2);
        assert!(q.exts.contains(&"png".to_string()));
        let mut q = Query::parse("تقرير ext:pdf,png");
        q.apply_kind(1);
        assert_eq!(q.exts, vec!["pdf"]);
        let mut q = Query::parse("تقرير ext:png");
        q.apply_kind(1);
        assert_eq!(q.exts.len(), 1);
        assert!(!q.exts.contains(&"png".to_string()));
        let mut q = Query::parse("تقرير");
        q.apply_kind(KIND_FOLDERS);
        assert!(q.dirs_only);
        // An empty box stays empty whatever type is picked.
        let mut q = Query::parse("  ");
        q.apply_kind(1);
        assert!(q.is_empty());
    }

    #[test]
    fn name_search_ranking_and_filters() {
        let entries = vec![
            entry(r"C:\Docs\ملخص تقرير.docx", false),
            entry(r"C:\Docs\تقرير.pdf", false),
            entry(r"D:\تقارير\تقرير السنة.pdf", false),
            entry(r"C:\Docs\تقرير", true),
            entry(r"C:\Docs\صورة.png", false),
        ];
        let r = search_names(&entries, &Query::parse("التقرير"), "");
        assert_eq!(r[0], 3); // the article is optional
        assert_eq!(r.len(), 4);
        let r = search_names(&entries, &Query::parse("تقرير"), "");
        assert_eq!(r[0], 3); // exact name first
        assert_eq!(r.len(), 4);
        let r = search_names(&entries, &Query::parse("تقرير .pdf"), r"c:\docs");
        assert_eq!(r, vec![1]);
        let r = search_names(&entries, &Query::parse("ext:pdf"), "");
        assert_eq!(r.len(), 2);
        let r = search_names(&entries, &Query::parse("تقرير -ملخص !السنة"), "");
        assert_eq!(r, vec![3, 1]);
        let r = search_names(&entries, &Query::parse("ملخص|صورة"), "");
        assert_eq!(r.len(), 2);
        let r = search_names(&entries, &Query::parse("تقرير folder:"), "");
        assert_eq!(r, vec![3]);
        let mut q = Query::parse("تقرير");
        q.apply_kind(2);
        assert!(search_names(&entries, &q, "").is_empty());
        let mut q = Query::parse("ext:pdf");
        q.apply_kind(2);
        assert!(search_names(&entries, &q, "").is_empty());
        assert!(search_names(&entries, &Query::parse("folder:"), "").is_empty());
    }

    #[test]
    fn content_match_with_snippet() {
        let text = "هذا نص طويل. وفيه التَّقرير السنوي للمبيعات لعام ٢٠٢٤ مع تفاصيل.";
        let s = match_text(text, &Query::parse("تقرير 2024")).unwrap();
        assert!(s.contains("التَّقرير"), "{s}");
        assert!(match_text(text, &Query::parse("غير موجود")).is_none());
        assert!(match_text(text, &Query::parse("تقرير -السنوي")).is_none());
        assert!(match_text(text, &Query::parse("موجود|المبيعات")).is_some());
        // Snippet starts at the earliest match, not the first term.
        let s = match_text(text, &Query::parse("تفاصيل هذا")).unwrap();
        assert!(s.starts_with("هذا"), "{s}");
    }
}
