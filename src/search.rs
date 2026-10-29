//! Query parsing, filename search over the index, and content search inside documents.

// Only the Windows UI and the tests use this module.

use rayon::prelude::*;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::extract;
use crate::index::Entry;
use crate::normalize::{normalize, normalize_with_map};

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

/// Prefixes of a token that keeps only folders (ASCII case ignored).
pub const FOLDER_PREFIXES: [&str; 2] = ["folder:", "مجلد:"];

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

/// Length of the folder prefix `tok` starts with, if any.
fn folder_prefix(tok: &str) -> Option<usize> {
    FOLDER_PREFIXES
        .iter()
        .find(|p| tok.get(..p.len()).is_some_and(|h| h.eq_ignore_ascii_case(p)))
        .map(|p| p.len())
}

/// The word after `!`, or after `-` when a letter follows (so `-` alone and
/// `2023-01` stay search words).
fn exclude_word(tok: &str) -> Option<&str> {
    tok.strip_prefix('!')
        .or_else(|| tok.strip_prefix('-').filter(|w| w.starts_with(char::is_alphabetic)))
}

fn push_unique(alts: &mut Vec<String>, a: String) {
    if !alts.contains(&a) {
        alts.push(a);
    }
}

/// Adds `alt` (already normalised) and, unless `exact` (a quoted phrase), the
/// bare word too when it starts with an Arabic article that leaves at least 3 letters.
fn push_alt(alts: &mut Vec<String>, alt: String, exact: bool) {
    let bare = (!exact)
        .then(|| {
            ARTICLES
                .iter()
                .find_map(|p| alt.strip_prefix(p))
                .filter(|rest| rest.chars().count() >= 3)
                .map(str::to_string)
        })
        .flatten();
    for a in std::iter::once(alt).chain(bare) {
        push_unique(alts, a);
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
                    // A filter between two words ends any pending `|`.
                    join = false;
                    continue;
                }
                if let Some(len) = folder_prefix(&tok) {
                    q.dirs_only = true;
                    if tok.len() == len {
                        join = false;
                        continue;
                    }
                    pieces = vec![&tok[len..]];
                } else if let Some(word) = exclude_word(&tok) {
                    let n = normalize(word);
                    if !n.trim().is_empty() {
                        q.excludes.push(n);
                        join = false;
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
                    push_alt(&mut alts, n, quoted);
                }
            }
            let join_prev = join || pieces.first() == Some(&"") && pieces.len() > 1;
            join = pieces.len() > 1 && pieces.last() == Some(&"");
            if alts.is_empty() {
                join |= join_prev;
                continue;
            }
            match q.terms.last_mut() {
                Some(last) if join_prev => alts.into_iter().for_each(|a| push_unique(&mut last.alts, a)),
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

    /// Type and folder filters on an entry.
    fn kind_ok(&self, e: &Entry) -> bool {
        (!self.dirs_only || e.is_dir) && self.ext_ok(e)
    }

    /// Type, folder and excluded-word filters on an entry's name.
    fn entry_ok(&self, e: &Entry) -> bool {
        self.kind_ok(e) && !self.excludes.iter().any(|x| e.norm_name.contains(x.as_str()))
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
        .filter(|(_, e)| !e.is_dir && e.size > 0 && e.size <= extract::MAX_FILE_SIZE)
        .filter(|(_, e)| extract::is_supported(&e.ext()) && q.kind_ok(e))
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

/// How the result list is ordered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SortKey {
    /// The order [`search_names`] returned (best matches first).
    #[default]
    Relevance,
    Name,
    Path,
    Size,
    Date,
}

impl SortKey {
    /// The key for a result-list column: 0 name, 1 path, 2 size, 3 date.
    pub fn from_column(col: usize) -> SortKey {
        match col {
            0 => SortKey::Name,
            1 => SortKey::Path,
            2 => SortKey::Size,
            3 => SortKey::Date,
            _ => SortKey::Relevance,
        }
    }

    /// The column this key sorts by, `None` for relevance.
    pub fn column(self) -> Option<usize> {
        match self {
            SortKey::Name => Some(0),
            SortKey::Path => Some(1),
            SortKey::Size => Some(2),
            SortKey::Date => Some(3),
            SortKey::Relevance => None,
        }
    }
}

/// ASCII-case-insensitive byte order. Non-ASCII letters (Arabic has no
/// case) compare by their UTF-8 bytes, which keeps the order total and lets
/// [`sort_key`] stay consistent with it.
fn cmp_caseless(a: &str, b: &str) -> std::cmp::Ordering {
    a.bytes().map(|c| c.to_ascii_lowercase()).cmp(b.bytes().map(|c| c.to_ascii_lowercase()))
}

/// The first 16 bytes of `bytes`, big-endian and zero-padded, so that
/// comparing keys agrees with comparing the byte strings whenever keys differ.
fn sort_key(bytes: impl Iterator<Item = u8>) -> u128 {
    let mut k = [0u8; 16];
    for (d, s) in k.iter_mut().zip(bytes) {
        *d = s;
    }
    u128::from_be_bytes(k)
}

/// Sorts `hits` (indices into `entries`) by `key`. Ties keep index order,
/// which is path order since the index is sorted by path. `Path` ignores
/// ASCII case; `Size` lists folders before files; `descending` reverses the
/// whole order (ties stay ascending). [`SortKey::Relevance`] leaves `hits` untouched.
///
/// Each hit gets a 16-byte key computed once, so the sort compares plain
/// integers and only touches the entries again when two keys are equal.
pub fn sort_hits(entries: &[Entry], hits: &mut [u32], key: SortKey, descending: bool) {
    use std::cmp::Ordering;
    let Some(&first) = hits.first() else { return };
    let e = |i: u32| &entries[i as usize];
    // Paths share long prefixes ("C:\Users\…"), so path keys start after the common one.
    let skip = match key {
        SortKey::Relevance => return,
        SortKey::Path => {
            let f = e(first).path.as_bytes();
            hits.par_iter()
                .map(|&i| e(i).path.bytes().zip(f).take_while(|(x, y)| x.eq_ignore_ascii_case(y)).count())
                .min()
                .unwrap_or(0)
        }
        _ => 0,
    };
    let key_of = |x: &Entry| -> u128 {
        match key {
            SortKey::Name => sort_key(x.norm_name.bytes()),
            SortKey::Path => sort_key(x.path.as_bytes()[skip..].iter().map(u8::to_ascii_lowercase)),
            SortKey::Size => ((!x.is_dir as u128) << 64) | x.size as u128,
            SortKey::Date => (x.mtime as u64 ^ (1 << 63)) as u128,
            SortKey::Relevance => 0,
        }
    };
    let tie: fn(&Entry, &Entry) -> Ordering = match key {
        SortKey::Name => |a, b| a.norm_name.cmp(&b.norm_name),
        SortKey::Path => |a, b| cmp_caseless(&a.path, &b.path),
        _ => |_, _| Ordering::Equal,
    };
    let mut keyed: Vec<(u128, u32)> = hits.par_iter().map(|&i| (key_of(e(i)), i)).collect();
    keyed.par_sort_unstable_by(|&(ka, a), &(kb, b)| {
        let o = ka.cmp(&kb).then_with(|| tie(e(a), e(b)));
        if descending { o.reverse() } else { o }.then(a.cmp(&b))
    });
    hits.par_iter_mut().zip(keyed.par_iter()).for_each(|(h, &(_, i))| *h = i);
}

/// Byte ranges `[start, end)` of `name` (the original, un-normalised name)
/// covered by any occurrence of any alternative of any term, sorted and
/// merged. Diacritics right after a match are included so the highlighted
/// word stays whole. Excluded words are not highlighted.
pub fn match_ranges(name: &str, q: &Query) -> Vec<(usize, usize)> {
    let (norm, map) = normalize_with_map(name);
    let src_end = |last: usize| {
        let s = map[last] as usize;
        let char_end = s + name[s..].chars().next().map_or(0, char::len_utf8);
        // Characters between this one and the next mapped one were dropped by normalisation.
        let next = map.get(last + 1).map_or(name.len(), |&n| n as usize);
        char_end.max(next)
    };
    let mut ranges = Vec::new();
    for alt in q.terms.iter().flat_map(|t| &t.alts).filter(|a| !a.is_empty()) {
        let mut from = 0;
        while let Some(p) = norm[from..].find(alt.as_str()) {
            let s = from + p;
            ranges.push((map[s] as usize, src_end(s + alt.len() - 1)));
            // Step one char so overlapping occurrences are found too.
            from = s + norm[s..].chars().next().map_or(1, char::len_utf8);
        }
    }
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
    for (s, e) in ranges {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    merged
}

/// Moves the hits listed in `boosted` (entry index, score) to the front,
/// highest score first, and keeps the rest in their current order.
pub fn promote(hits: &mut Vec<u32>, boosted: &[(u32, f64)]) {
    if boosted.is_empty() || hits.is_empty() {
        return;
    }
    let scores: std::collections::HashMap<u32, f64> = boosted.iter().copied().collect();
    let mut front: Vec<(f64, u32)> = Vec::new();
    hits.retain(|i| match scores.get(i) {
        Some(&s) => {
            front.push((s, *i));
            false
        }
        None => true,
    });
    front.sort_unstable_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    hits.splice(0..0, front.into_iter().map(|(_, i)| i));
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

    #[test]
    fn review_fixes() {
        // A filter or exclude between `|` and the next word ends the OR.
        let q = Query::parse("عقد | ext:pdf تقرير");
        assert_eq!(q.terms.len(), 2);
        assert_eq!(Query::parse("a | !b c").terms.len(), 2);
        // Quoted phrases keep their article.
        assert_eq!(Query::parse("\"التقرير السنوي\"").terms[0].alts, vec!["التقرير السنوي"]);
        // Excludes in content search apply to the text, not the file name.
        let q = Query::parse("عقد !قديم");
        let e = entry(r"C:\Docs\قديم - عقد.docx", false);
        assert!(q.kind_ok(&e) && !q.entry_ok(&e));
        assert!(match_text("هذا عقد الإيجار", &q).is_some());
    }

    fn sized(path: &str, is_dir: bool, size: u64, mtime: i64) -> Entry {
        Entry { size, mtime, ..entry(path, is_dir) }
    }

    #[test]
    fn sort_keys_and_columns() {
        for c in 0..4 {
            assert_eq!(SortKey::from_column(c).column(), Some(c));
        }
        assert_eq!(SortKey::from_column(9), SortKey::Relevance);
        assert_eq!(SortKey::default().column(), None);
    }

    #[test]
    fn sort_matches_plain_comparison() {
        // Long shared prefixes and names longer than the 16-byte key exercise the tie fallback.
        let entries: Vec<Entry> = (0..3000u32)
            .map(|i| {
                let path = format!(
                    r"C:\Data\{}\ملف طويل جدا {}{}.TXT",
                    ["X", "x", "تقارير"][i as usize % 3],
                    i % 7,
                    i * 31 % 101
                );
                sized(&path, i % 5 == 0, (i % 13) as u64, (i % 17) as i64 - 8)
            })
            .collect();
        let all: Vec<u32> = (0..entries.len() as u32).collect();
        for key in [SortKey::Name, SortKey::Path, SortKey::Size, SortKey::Date] {
            for desc in [false, true] {
                let mut h = all.clone();
                sort_hits(&entries, &mut h, key, desc);
                let mut want = all.clone();
                want.sort_by(|&a, &b| {
                    let (x, y) = (&entries[a as usize], &entries[b as usize]);
                    let o = match key {
                        SortKey::Name => x.norm_name.cmp(&y.norm_name),
                        SortKey::Path => x.path.to_ascii_lowercase().cmp(&y.path.to_ascii_lowercase()),
                        SortKey::Size => y.is_dir.cmp(&x.is_dir).then(x.size.cmp(&y.size)),
                        _ => x.mtime.cmp(&y.mtime),
                    };
                    if desc { o.reverse() } else { o }.then(a.cmp(&b))
                });
                assert_eq!(h, want, "{key:?} {desc}");
            }
        }
    }

    #[test]
    fn sorting() {
        let entries = vec![
            sized(r"C:\a\Beta.txt", false, 30, 5),
            sized(r"C:\a\alpha.txt", false, 10, 9),
            sized(r"C:\b", true, 0, 1),
            sized(r"c:\A\gamma.txt", false, 10, 2),
        ];
        let sorted = |key, desc| {
            let mut h = vec![0, 1, 2, 3];
            sort_hits(&entries, &mut h, key, desc);
            h
        };
        assert_eq!(sorted(SortKey::Relevance, true), vec![0, 1, 2, 3]);
        assert_eq!(sorted(SortKey::Name, false), vec![1, 2, 0, 3]);
        assert_eq!(sorted(SortKey::Name, true), vec![3, 0, 2, 1]);
        assert_eq!(sorted(SortKey::Path, false), vec![1, 0, 3, 2]);
        assert_eq!(sorted(SortKey::Size, false), vec![2, 1, 3, 0]);
        // Ties stay in index order even when descending.
        assert_eq!(sorted(SortKey::Size, true), vec![0, 1, 3, 2]);
        assert_eq!(sorted(SortKey::Date, false), vec![2, 3, 0, 1]);
    }

    #[test]
    fn caseless_compare() {
        use std::cmp::Ordering::*;
        assert_eq!(cmp_caseless("ABC", "abc"), Equal);
        assert_eq!(cmp_caseless("abc", "ABD"), Less);
        assert_eq!(cmp_caseless("ab", "AB c"), Less);
        assert_eq!(cmp_caseless("a", "ab"), Less);
        assert_eq!(cmp_caseless("تقرير ب", "تقرير أ"), "ب".cmp("أ"));
    }

    #[test]
    #[ignore = "benchmark: cargo test --release -- --ignored --nocapture"]
    fn sort_two_million() {
        let entries: Vec<Entry> = (0..2_000_000u32)
            .map(|i| {
                let path = format!(r"C:\Users\Saif\Documents\مشروع {}\ملف {}.txt", i % 977, i.wrapping_mul(2654435761));
                sized(&path, i % 10 == 0, (i as u64 * 7919) % 100_000, (i as i64 * 104729) % 1_000_000)
            })
            .collect();
        for key in [SortKey::Name, SortKey::Path, SortKey::Size, SortKey::Date] {
            let mut hits: Vec<u32> = (0..entries.len() as u32).collect();
            let t = std::time::Instant::now();
            sort_hits(&entries, &mut hits, key, false);
            println!("{key:?}: {:?}", t.elapsed());
        }
    }

    fn highlighted<'a>(name: &'a str, q: &str) -> Vec<&'a str> {
        match_ranges(name, &Query::parse(q)).into_iter().map(|(s, e)| &name[s..e]).collect()
    }

    #[test]
    fn highlight_ranges() {
        assert_eq!(highlighted("Report REPORT.pdf", "report"), vec!["Report", "REPORT"]);
        assert_eq!(highlighted("التَّقرير السنوي.docx", "تقرير"), vec!["تَّقرير"]);
        // The article alternative covers the whole word.
        assert_eq!(highlighted("التقرير", "التقرير"), vec!["التقرير"]);
        // Overlapping and adjacent matches merge; excludes are not highlighted.
        assert_eq!(highlighted("aaa bc", "aa b|c !aaa"), vec!["aaa", "bc"]);
        assert_eq!(highlighted("ملف ٢٠٢٤", "2024"), vec!["٢٠٢٤"]);
        assert_eq!(highlighted("كـــتاب", "كتاب"), vec!["كـــتاب"]);
        // A trailing diacritic is part of the word.
        assert_eq!(highlighted("كتابٌ جديد", "كتاب"), vec!["كتابٌ"]);
        assert!(highlighted("abc", "x").is_empty());
        assert!(highlighted("abc", "ext:pdf").is_empty());
    }

    #[test]
    fn promote_boosted() {
        let mut hits = vec![5, 3, 9, 1, 7];
        promote(&mut hits, &[(7, 1.0), (1, 3.0), (42, 9.0)]);
        assert_eq!(hits, vec![1, 7, 5, 3, 9]);
        let mut hits = vec![2, 1];
        promote(&mut hits, &[]);
        assert_eq!(hits, vec![2, 1]);
    }

    #[test]
    fn parse_quotes_and_filters_edge_cases() {
        // An unterminated quote runs to the end; empty quotes add nothing.
        assert_eq!(alts(&Query::parse(r#"a "b c"#)), vec![vec!["a"], vec!["b c"]]);
        assert!(Query::parse(r#""""#).is_empty());
        assert_eq!(alts(&Query::parse(r#"a "" b"#)), vec![vec!["a"], vec!["b"]]);
        // Inside quotes `|`, `!`, `ext:` and `مجلد:` are plain text.
        assert_eq!(alts(&Query::parse(r#""a|b""#)), vec![vec!["a|b"]]);
        let q = Query::parse(r#""!x" "ext:pdf" "مجلد:صور""#);
        assert_eq!(alts(&q), vec![vec!["!x"], vec!["ext:pdf"], vec!["مجلد:صور"]]);
        assert!(q.excludes.is_empty() && q.exts.is_empty() && !q.dirs_only);
        // Quoted phrases are normalised like words.
        assert_eq!(alts(&Query::parse(r#""أحمد عليّ""#)), vec![vec!["احمد علي"]]);
        // Extension filters: case, empty lists and separators.
        assert_eq!(Query::parse("EXT:PDF *.Docx").exts, vec!["pdf", "docx"]);
        assert_eq!(Query::parse("ext:PDF,,DOCX;").exts, vec!["pdf", "docx"]);
        assert!(Query::parse("ext:").is_empty());
        assert!(Query::parse("*.").is_empty());
        assert_eq!(alts(&Query::parse(".")), vec![vec!["."]]);
        assert_eq!(alts(&Query::parse(".tar.gz")), vec![vec![".tar.gz"]]);
        // Tabs and newlines separate words too.
        assert_eq!(alts(&Query::parse("a\tb\nc")), vec![vec!["a"], vec!["b"], vec!["c"]]);
    }

    #[test]
    fn parse_pipe_edge_cases() {
        assert_eq!(alts(&Query::parse("a||b")), vec![vec!["a", "b"]]);
        assert_eq!(alts(&Query::parse("a|a")), vec![vec!["a"]]);
        assert_eq!(alts(&Query::parse("a | | b")), vec![vec!["a", "b"]]);
        assert_eq!(alts(&Query::parse("a |")), vec![vec!["a"]]);
        assert_eq!(alts(&Query::parse("|")), Vec::<Vec<&str>>::new());
        // Each alternative gets its own article-less form.
        assert_eq!(alts(&Query::parse("التقرير|عقد")), vec![vec!["التقرير", "تقرير", "عقد"]]);
        // A folder token between `|` and the next word ends the OR as well.
        assert_eq!(alts(&Query::parse("a | مجلد: b")), vec![vec!["a"], vec!["b"]]);
    }

    #[test]
    fn parse_excludes_edge_cases() {
        let q = Query::parse("!أحمد -Report");
        assert_eq!(q.excludes, vec!["احمد", "report"]);
        assert!(q.terms.is_empty());
        // Excludes alone still narrow the results.
        assert!(!q.is_empty());
        // A bare `!` is a search word, and `-` before a digit is not an exclude.
        assert_eq!(alts(&Query::parse("!")), vec![vec!["!"]]);
        assert_eq!(alts(&Query::parse("-٢٠٢٣")), vec![vec!["-2023"]]);
        assert!(Query::parse("-٢٠٢٣").excludes.is_empty());
    }

    #[test]
    fn parse_folder_prefixes() {
        for input in ["مجلد:التقارير", "folder:التقارير", "Folder:التقارير", "مجلد: التقارير"] {
            let q = Query::parse(input);
            assert!(q.dirs_only, "{input}");
            assert_eq!(alts(&q), vec![vec!["التقارير", "تقارير"]], "{input}");
        }
        let q = Query::parse("مجلد:");
        assert!(q.dirs_only && q.is_empty());
        // Only a leading prefix counts.
        let q = Query::parse("صور:مجلد:");
        assert!(!q.dirs_only);
        assert_eq!(alts(&q), vec![vec!["صور:مجلد:"]]);
    }

    #[test]
    fn content_search_in_files() {
        let dir = std::env::temp_dir().join(format!("bahth_content_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let files = [("تقرير.txt", "هذا التقرير السنوي"), ("اخر.txt", "لا شيء هنا"), ("صورة.png", "التقرير")];
        let mut entries: Vec<Entry> = files
            .iter()
            .map(|(name, text)| {
                let p = dir.join(name);
                std::fs::write(&p, text).unwrap();
                Entry { size: text.len() as u64, ..entry(p.to_str().unwrap(), false) }
            })
            .collect();
        entries.push(entry(dir.to_str().unwrap(), true));
        let run = |q: &str, cancel: bool| {
            let progress = ContentProgress::default();
            progress.cancel.store(cancel, Ordering::Relaxed);
            let hits = Mutex::new(Vec::new());
            search_content(&entries, &Query::parse(q), "", &progress, &hits);
            assert!(progress.done.load(Ordering::Relaxed));
            let hits: Vec<(u32, String)> = hits.into_inner().unwrap().into_iter().map(|h| (h.index, h.snippet)).collect();
            (progress.total.load(Ordering::Relaxed), progress.checked.load(Ordering::Relaxed), hits)
        };
        // Only the two supported documents are candidates; the png is not.
        let (total, checked, hits) = run("تقرير", false);
        assert_eq!((total, checked), (2, 2));
        assert_eq!(hits, vec![(0, "هذا التقرير السنوي".to_string())]);
        assert!(run("تقرير !السنوي", false).2.is_empty());
        assert_eq!(run("شيء|السنوي", false).2.len(), 2);
        // No terms: only counts candidates. Cancelled: checks nothing.
        assert_eq!(run("ext:txt", false), (2, 0, vec![]));
        assert_eq!(run("تقرير", true), (2, 0, vec![]));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
