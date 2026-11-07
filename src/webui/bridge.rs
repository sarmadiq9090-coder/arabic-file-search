//! The messages between the new interface's page and the program.
//!
//! The page asks with `window.ipc.postMessage(JSON)` ([`Request`], checked by
//! [`parse`]); the program answers by running `window.__instafind.recv(JSON)`
//! ([`Outbound`], made by [`script`]). The page never names a path: it asks
//! for "row 7 of search 42" and the program looks the path up in what it
//! showed ([`Shown`]), so the page can't make the program open or read
//! anything it did not list.
//!
//! The search itself is the main window's filename search (the same parsing,
//! type filter, fallbacks, frequently-opened files first and "did you mean"),
//! run here so it can be tested without Windows.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;

use crate::fuzzy::{self, Fallback};
use crate::i18n::{KIND_LABELS_EN, Lang};
use crate::index::{Entry, Index};
use crate::indexing_status::{self, Indexing};
use crate::search::{self, KINDS, Query};
use crate::suggest::{self, Situation, Suggestion};

/// Rows sent with the results; the page asks for the rest as it scrolls.
pub const FIRST_PAGE: usize = 60;
/// Rows sent for each later request.
pub const MORE_PAGE: usize = 200;
/// Longer messages are refused unread.
pub const MAX_MESSAGE: usize = 16 * 1024;
/// Longest search text accepted, in characters.
pub const MAX_QUERY: usize = 1000;

// ------------------------------------------------------------------ page → program

/// Which list a row belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum List {
    /// The search results of search `seq`.
    Results,
    /// The recently opened files shown while the box is empty.
    Recent,
}

/// A request of the page.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    /// The page has loaded: send the file types, the status and the recent files.
    Hello,
    /// Search file names for `q` with file type `kind` (an index into [`KINDS`]).
    /// `seq` numbers the searches; answers carry it back.
    Search {
        seq: u64,
        q: String,
        #[serde(default)]
        kind: usize,
    },
    /// The rows of search `seq` from `from` on.
    More { seq: u64, from: usize },
    /// Open the file of row `id`.
    Open { list: List, seq: u64, id: u32 },
    /// Show the file of row `id` in Explorer.
    Reveal { list: List, seq: u64, id: u32 },
    /// Copy the full path of row `id`.
    Copy { list: List, seq: u64, id: u32 },
    /// Search the web for `q` in the default browser.
    Web { q: String },
    /// Hide the window (Esc in an empty box).
    Hide,
}

/// Why a message was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    /// It did not come from the program's own page.
    Origin,
    TooLong,
    Malformed,
    /// It lacked this launch's token.
    Token,
    /// A value out of range.
    Invalid,
}

/// The request in `body`, sent by the page at `source` (its URL), if it
/// comes from the program's own page and carries `token`.
pub fn parse(body: &str, source: &str, token: &str) -> Result<Request, Rejected> {
    let origin = super::assets::ORIGIN;
    if !source.strip_prefix(origin).is_some_and(|rest| rest.is_empty() || rest.starts_with('/')) {
        return Err(Rejected::Origin);
    }
    if body.len() > MAX_MESSAGE {
        return Err(Rejected::TooLong);
    }
    let mut value: serde_json::Value = serde_json::from_str(body).map_err(|_| Rejected::Malformed)?;
    let sent = value.as_object_mut().and_then(|o| o.remove("t")).ok_or(Rejected::Token)?;
    if !sent.as_str().is_some_and(|t| same(t.as_bytes(), token.as_bytes())) || token.is_empty() {
        return Err(Rejected::Token);
    }
    let req: Request = serde_json::from_value(value).map_err(|_| Rejected::Malformed)?;
    let ok = match &req {
        Request::Search { q, kind, .. } => *kind < KINDS.len() && q.chars().count() <= MAX_QUERY,
        Request::Web { q } => !q.trim().is_empty() && q.chars().count() <= MAX_QUERY,
        _ => true,
    };
    if ok { Ok(req) } else { Err(Rejected::Invalid) }
}

/// Compares in time that does not depend on where the texts differ.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A fresh 128-bit token for this launch, as 32 hex digits. It comes from
/// the standard library's per-process random hash keys (seeded by the
/// system's random generator), mixed with the time and the process id.
pub fn new_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let part = |salt: u64| {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(salt);
        h.write_u128(nanos);
        h.write_u32(std::process::id());
        h.finish()
    };
    format!("{:016x}{:016x}", part(0x5eed_0001), part(0x5eed_0002))
}

// ------------------------------------------------------------------ program → page

/// One row of a list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RowOut {
    /// Its place in the list; what the page sends back to act on it.
    pub id: u32,
    /// The name in pieces, `true` for the pieces that match the search.
    pub name: Vec<(String, bool)>,
    /// The folder it is in.
    pub dir: String,
    pub size: u64,
    /// Modified, seconds since 1970 (UTC).
    pub mtime: i64,
    /// Lower-case extension, "" for none.
    pub ext: String,
    pub folder: bool,
}

/// Something to try when a search found nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SuggestionOut {
    /// `query`, `all_kinds`, `all_drives`, `contents` or `web`.
    pub kind: &'static str,
    /// What it says, in the interface language.
    pub label: String,
    /// The text to search for instead (`query` only).
    pub q: String,
}

/// A message to the page.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Outbound {
    Hello {
        /// The file-type filter's choices, in the interface language.
        kinds: Vec<String>,
        /// Write numbers with Arabic-Indic digits.
        digits: bool,
        version: &'static str,
    },
    Results {
        seq: u64,
        total: usize,
        ms: usize,
        rows: Vec<RowOut>,
        /// How a fallback found the rows (other keyboard layout, Arabizi, typos).
        note: Option<String>,
        suggestions: Vec<SuggestionOut>,
        /// Names in the index, for "we searched N file names".
        files: usize,
        /// The file type searched, "" for all.
        kind_label: String,
    },
    Rows {
        seq: u64,
        from: usize,
        rows: Vec<RowOut>,
    },
    Recent {
        rows: Vec<RowOut>,
    },
    Status {
        /// `loading`, `names` (drive scan), `contents` (document texts) or `ready`.
        state: &'static str,
        files: usize,
        /// The progress in words, while indexing runs.
        text: Option<String>,
    },
    /// Put this text in the box and search (from the quick popup).
    Query {
        q: String,
    },
    /// Put the caret in the search box.
    Focus,
    /// A short message for the status bar.
    Notice {
        text: String,
    },
}

/// The script that hands `msg` to the page.
pub fn script(msg: &Outbound) -> String {
    // JSON is a JavaScript expression (U+2028/U+2029 included since ES2019).
    let json = serde_json::to_string(msg).unwrap_or_else(|_| "null".into());
    format!("window.__instafind&&window.__instafind.recv({json})")
}

/// The file-type filter's choices in `lang`.
pub fn kinds(lang: Lang) -> Vec<String> {
    KINDS.iter().zip(KIND_LABELS_EN).map(|((ar, _), en)| lang.pick(*ar, en).to_string()).collect()
}

/// The status message for the page.
pub fn status(loading: bool, ix: Indexing, files: usize, lang: Lang) -> Outbound {
    let state = match ix {
        _ if loading => "loading",
        Indexing::Names { .. } => "names",
        Indexing::Contents { .. } if ix.running() => "contents",
        _ => "ready",
    };
    Outbound::Status { state, files, text: indexing_status::progress_text(lang, ix, &indexing_status::group_digits) }
}

// ------------------------------------------------------------------ rows

/// The folder part of `path`: `C:\a\b.txt` → `C:\a`, `C:\b.txt` → `C:\`.
pub fn parent(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['\\', '/']);
    match trimmed.rfind(['\\', '/']) {
        // Keep the separator after a drive letter.
        Some(i) if i > 0 && trimmed[..i].ends_with(':') => &trimmed[..=i],
        Some(i) => &trimmed[..i],
        None => "",
    }
}

/// `name` cut at the byte ranges of `marks` (sorted, merged), `true` inside them.
pub fn pieces(name: &str, marks: &[(usize, usize)]) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut at = 0;
    for &(s, e) in marks {
        let (s, e) = (s.max(at).min(name.len()), e.min(name.len()));
        if s >= e || !name.is_char_boundary(s) || !name.is_char_boundary(e) {
            continue;
        }
        if s > at {
            out.push((name[at..s].to_string(), false));
        }
        out.push((name[s..e].to_string(), true));
        at = e;
    }
    if at < name.len() || out.is_empty() {
        out.push((name[at..].to_string(), false));
    }
    out
}

/// Row `id` for `e`, with the parts matching `q` marked.
pub fn row(id: u32, e: &Entry, q: Option<&Query>) -> RowOut {
    let name = e.name();
    let marks = q.map(|q| search::match_ranges(name, q)).unwrap_or_default();
    RowOut {
        id,
        name: pieces(name, &marks),
        dir: parent(&e.path).to_string(),
        size: e.size,
        mtime: e.mtime,
        ext: e.ext(),
        folder: e.is_dir,
    }
}

/// The recently opened files that still exist, as rows (the caller checks
/// the disk: `stat` gives size, modified time and whether it is a folder).
pub fn recent_rows(paths: &[&str], mut stat: impl FnMut(&str) -> Option<(u64, i64, bool)>) -> (Vec<String>, Vec<RowOut>) {
    let found = suggest::pick_recent(paths.iter().copied(), suggest::RECENT_ROWS, suggest::MAX_PROBES, &mut stat);
    let rows = found
        .iter()
        .enumerate()
        .map(|(i, (p, (size, mtime, is_dir)))| {
            let e = Entry { path: p.as_str().into(), size: *size, mtime: *mtime, is_dir: *is_dir, norm_name: "".into() };
            row(i as u32, &e, None)
        })
        .collect();
    (found.into_iter().map(|(p, _)| p).collect(), rows)
}

// ------------------------------------------------------------------ search

/// What a search needs besides the index.
#[derive(Debug, Clone, Default)]
pub struct Params {
    pub seq: u64,
    pub text: String,
    /// Index into [`KINDS`].
    pub kind: usize,
    /// Folder searched; empty for all drives.
    pub scope: String,
    /// Windows' own files are searched too.
    pub show_system: bool,
    /// Often opened files, to put first ([`crate::history::History::boosted`]).
    pub boosted: Vec<(u32, f64)>,
    /// Look for a "did you mean" when nothing is found (only once the index
    /// is complete).
    pub suggest: bool,
}

/// The rows a search found, kept to answer the page's later requests.
pub struct Shown {
    pub seq: u64,
    pub index: Arc<Index>,
    pub hits: Vec<u32>,
    /// The query whose matches are marked (a fallback's, when one found the rows).
    pub marked: Query,
    pub params: Params,
}

impl Shown {
    /// Rows `from..from + n`.
    pub fn page(&self, from: usize, n: usize) -> Vec<RowOut> {
        let end = from.saturating_add(n).min(self.hits.len());
        (from.min(end)..end)
            .filter_map(|i| self.index.entries.get(self.hits[i] as usize).map(|e| row(i as u32, e, Some(&self.marked))))
            .collect()
    }

    /// The full path of row `id`.
    pub fn path(&self, id: u32) -> Option<&str> {
        let i = *self.hits.get(id as usize)?;
        self.index.entries.get(i as usize).map(|e| &*e.path)
    }
}

/// How the search turns a text into a [`Query`].
fn parser(kind: usize, show_system: bool) -> impl Fn(&str) -> Query + Sync {
    move |t: &str| {
        let mut q = Query::parse(t);
        q.apply_kind(kind);
        q.show_system |= show_system;
        q
    }
}

/// Runs the filename search of `p` over `index`, and the first message for
/// the page. Meant for a background thread.
pub fn search(index: Arc<Index>, p: Params, lang: Lang) -> (Shown, Outbound) {
    let started = Instant::now();
    let parse = parser(p.kind, p.show_system);
    let q = parse(&p.text);
    let mut hits = search::search_names(&index.entries, &q, &p.scope);
    // Wrong keyboard layout ("hgjrvdv" → "التقرير"), Arabizi ("3abd"), then typos.
    let mut fallback: Option<Fallback> = None;
    if hits.is_empty()
        && let Some((alt, fb)) = fuzzy::name_fallback(&index.entries, &p.text, &p.scope, &parse)
    {
        hits = alt;
        fallback = Some(fb);
    }
    // Files the user opens often come first.
    search::promote(&mut hits, &p.boosted);
    let did_you_mean = if p.suggest && hits.is_empty() && !q.is_empty() {
        suggest::did_you_mean(&index.entries, &p.text, &p.scope, &parse)
    } else {
        None
    };
    let ms = started.elapsed().as_millis() as usize;
    let marked = match &fallback {
        Some(fb) => parse(&fb.query),
        None => q,
    };
    let suggestions = if hits.is_empty() { suggestions(&p.text, p.kind, did_you_mean) } else { Vec::new() };
    let shown = Shown { seq: p.seq, index, hits, marked, params: p };
    let msg = Outbound::Results {
        seq: shown.seq,
        total: shown.hits.len(),
        ms,
        rows: shown.page(0, FIRST_PAGE),
        note: fallback.map(|fb| fb.status(&shown.params.text)),
        suggestions,
        files: shown.index.entries.len(),
        kind_label: if shown.params.kind == 0 { String::new() } else { kinds(lang).swap_remove(shown.params.kind) },
    };
    (shown, msg)
}

/// What to offer when `text` found nothing: a better spelling, all file
/// types, the web.
pub fn suggestions(text: &str, kind: usize, did_you_mean: Option<String>) -> Vec<SuggestionOut> {
    let situation = Situation { did_you_mean, kind_filtered: kind != 0, scoped: false, contents: false, web: true };
    suggest::suggestions(text, &situation)
        .into_iter()
        .map(|s| {
            let label = s.label();
            let (kind, q) = match s {
                Suggestion::Query(t) => ("query", t),
                Suggestion::AllKinds => ("all_kinds", String::new()),
                Suggestion::AllDrives => ("all_drives", String::new()),
                Suggestion::Contents => ("contents", String::new()),
                Suggestion::Web => ("web", String::new()),
            };
            SuggestionOut { kind, label, q }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "http://instafind.localhost/";
    const TOKEN: &str = "00112233445566778899aabbccddeeff";

    fn entry(path: &str, size: u64, is_dir: bool) -> Entry {
        let name = path.rsplit('\\').next().unwrap();
        Entry { path: path.into(), size, mtime: 1_700_000_000, is_dir, norm_name: crate::normalize::normalize(name).into() }
    }

    fn index() -> Arc<Index> {
        let idx = Index {
            entries: vec![
            entry("C:\\Users\\Sara\\Documents\\العمل\\التقرير السنوي.pdf", 2_400_000, false),
            entry("C:\\Users\\Sara\\Documents\\تقرير المبيعات 2026.xlsx", 48_000, false),
            entry("C:\\Users\\Sara\\Desktop\\notes.txt", 2_000, false),
            entry("D:\\Projects\\تقارير", 0, true),
            ],
            ..Index::default()
        };
        Arc::new(idx)
    }

    #[test]
    fn accepts_only_the_page_with_the_token() {
        let ok = |b: &str| parse(b, SRC, TOKEN);
        assert_eq!(
            ok(&format!(r#"{{"t":"{TOKEN}","op":"search","seq":3,"q":"تقرير","kind":1}}"#)),
            Ok(Request::Search { seq: 3, q: "تقرير".into(), kind: 1 })
        );
        assert_eq!(ok(&format!(r#"{{"t":"{TOKEN}","op":"hello"}}"#)), Ok(Request::Hello));
        assert_eq!(
            ok(&format!(r#"{{"t":"{TOKEN}","op":"open","list":"recent","seq":0,"id":2}}"#)),
            Ok(Request::Open { list: List::Recent, seq: 0, id: 2 })
        );
        // Another page, a missing or wrong token, junk, unknown fields or values.
        let hello = format!(r#"{{"t":"{TOKEN}","op":"hello"}}"#);
        for src in ["https://evil.example/", "http://instafind.localhost.evil.example/", "http://evil.localhost/", "file:///C:/x.html", ""] {
            assert_eq!(parse(&hello, src, TOKEN), Err(Rejected::Origin), "{src}");
        }
        assert_eq!(parse(&hello, "http://instafind.localhost", TOKEN), Ok(Request::Hello));
        assert_eq!(ok(r#"{"op":"hello"}"#), Err(Rejected::Token));
        assert_eq!(ok(r#"{"t":"0011","op":"hello"}"#), Err(Rejected::Token));
        assert_eq!(ok(r#"{"t":5,"op":"hello"}"#), Err(Rejected::Token));
        assert_eq!(parse(&hello, SRC, ""), Err(Rejected::Token));
        assert_eq!(ok("not json"), Err(Rejected::Malformed));
        assert_eq!(ok(&format!(r#"{{"t":"{TOKEN}","op":"read_file","path":"C:\\x"}}"#)), Err(Rejected::Malformed));
        assert_eq!(ok(&format!(r#"{{"t":"{TOKEN}","op":"open","list":"results","seq":1,"id":1,"path":"C:\\x"}}"#)), Err(Rejected::Malformed));
        assert_eq!(ok(&format!(r#"{{"t":"{TOKEN}","op":"search","seq":1,"q":"a","kind":99}}"#)), Err(Rejected::Invalid));
        assert_eq!(ok(&format!(r#"{{"t":"{TOKEN}","op":"web","q":"  "}}"#)), Err(Rejected::Invalid));
        let long = format!(r#"{{"t":"{TOKEN}","op":"search","seq":1,"q":"{}"}}"#, "a".repeat(MAX_QUERY + 1));
        assert_eq!(ok(&long), Err(Rejected::Invalid));
        assert_eq!(ok(&format!(r#"{{"t":"{TOKEN}","op":"web","q":"{}"}}"#, "a".repeat(MAX_MESSAGE))), Err(Rejected::TooLong));
    }

    #[test]
    fn tokens_are_hex_and_differ() {
        let (a, b) = (new_token(), new_token());
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
        assert!(super::super::assets::page(Lang::Ar, &a).is_some());
    }

    #[test]
    fn folders_and_marked_pieces() {
        assert_eq!(parent("C:\\a\\b.txt"), "C:\\a");
        assert_eq!(parent("C:\\b.txt"), "C:\\");
        assert_eq!(parent("D:\\Projects\\x\\"), "D:\\Projects");
        assert_eq!(parent("\\\\server\\share\\f.doc"), "\\\\server\\share");
        assert_eq!(parent("name"), "");
        assert_eq!(pieces("abcdef", &[(1, 3)]), vec![("a".into(), false), ("bc".into(), true), ("def".into(), false)]);
        assert_eq!(pieces("abc", &[(0, 3)]), vec![("abc".into(), true)]);
        assert_eq!(pieces("", &[]), vec![(String::new(), false)]);
        // A range off a character boundary or past the end is ignored, never a panic.
        assert_eq!(pieces("تقرير", &[(1, 4)]), vec![("تقرير".into(), false)]);
        assert_eq!(pieces("تقرير", &[(2, 99)]), vec![("ت".into(), false), ("قرير".into(), true)]);
    }

    #[test]
    fn search_marks_matches_and_pages() {
        let p = Params { seq: 7, text: "تقرير".into(), ..Params::default() };
        let (shown, msg) = search(index(), p, Lang::Ar);
        let Outbound::Results { seq, total, rows, note, suggestions, files, .. } = msg else { panic!() };
        assert_eq!((seq, files, note, suggestions.len()), (7, 4, None, 0));
        assert!(total >= 2, "{total}");
        let names: Vec<String> = rows.iter().map(|r| r.name.iter().map(|(t, _)| t.as_str()).collect()).collect();
        assert!(names.contains(&"تقرير المبيعات 2026.xlsx".to_string()), "{names:?}");
        let sales = rows.iter().find(|r| r.ext == "xlsx").unwrap();
        assert_eq!(sales.name[0], ("تقرير".to_string(), true));
        assert_eq!(sales.dir, "C:\\Users\\Sara\\Documents");
        assert_eq!(shown.path(sales.id), Some("C:\\Users\\Sara\\Documents\\تقرير المبيعات 2026.xlsx"));
        assert_eq!(shown.path(99), None);
        assert_eq!(shown.page(1, 1).len(), 1);
        assert_eq!(shown.page(1, 1)[0].id, 1);
        assert!(shown.page(50, 10).is_empty());
        assert!(shown.page(usize::MAX, usize::MAX).is_empty());
    }

    #[test]
    fn type_filter_and_nothing_found() {
        let p = Params { seq: 1, text: "تقرير".into(), kind: 1, ..Params::default() };
        let (shown, msg) = search(index(), p, Lang::En);
        assert!(shown.hits.iter().all(|&i| !shown.index.entries[i as usize].is_dir));
        let Outbound::Results { kind_label, .. } = msg else { panic!() };
        assert_eq!(kind_label, "Documents");

        let p = Params { seq: 2, text: "zzqqxx".into(), kind: 2, ..Params::default() };
        let (_, msg) = search(index(), p, Lang::Ar);
        let Outbound::Results { total, suggestions, .. } = msg else { panic!() };
        assert_eq!(total, 0);
        let kinds: Vec<&str> = suggestions.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, ["all_kinds", "web"]);
    }

    #[test]
    fn script_and_status() {
        let s = script(&Outbound::Query { q: "a\"b</script>\u{2028}".into() });
        assert!(s.starts_with("window.__instafind&&window.__instafind.recv({\"type\":\"query\""), "{s}");
        assert!(s.contains(r#"a\"b</script>"#));
        assert!(matches!(status(true, Indexing::Idle, 0, Lang::Ar), Outbound::Status { state: "loading", .. }));
        assert!(matches!(status(false, Indexing::Names { found: 5 }, 0, Lang::Ar), Outbound::Status { state: "names", text: Some(_), .. }));
        assert!(matches!(status(false, Indexing::Contents { checked: 3, total: 3 }, 9, Lang::Ar), Outbound::Status { state: "ready", text: None, files: 9 }));
        assert_eq!(kinds(Lang::En)[1], "Documents");
        assert_eq!(kinds(Lang::Ar)[1], "مستندات");
    }

    #[test]
    fn recent_rows_skip_missing_files() {
        let paths = ["C:\\a\\gone.txt", "C:\\a\\here.pdf", "D:\\dir"];
        let (kept, rows) = recent_rows(&paths, |p| match p {
            "C:\\a\\here.pdf" => Some((10, 5, false)),
            "D:\\dir" => Some((0, 6, true)),
            _ => None,
        });
        assert_eq!(kept, ["C:\\a\\here.pdf", "D:\\dir"]);
        assert_eq!((rows[0].id, rows[0].dir.as_str(), rows[0].ext.as_str()), (0, "C:\\a", "pdf"));
        assert!(rows[1].folder);
    }
}
