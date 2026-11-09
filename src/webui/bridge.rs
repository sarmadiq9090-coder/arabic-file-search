//! The messages between the new interface's page and the program.
//!
//! The page asks with `window.ipc.postMessage(JSON)` ([`Request`], checked by
//! [`parse`]); the program answers by running `window.__instafind.recv(JSON)`
//! ([`Outbound`], made by [`script`]). The page never names a path: it asks
//! for "row 7 of search 42" and the program looks the path up in what it
//! showed ([`Shown`]), so the page can't make the program open, read, rename
//! or delete anything it did not list. Folders to search in are chosen in
//! Windows' own folder dialog, never typed by the page.
//!
//! The searches themselves are the main window's: the file-name search (the
//! same parsing, type, size and date filters, fallbacks, frequently-opened
//! files first, sorting and "did you mean") and the search inside documents
//! (snippets, match counts and every matching line), run here so they can be
//! tested without Windows.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use crate::fuzzy::{self, Fallback};
use crate::history::History;
use crate::i18n::{KIND_LABELS_EN, Lang};
use crate::index::{Entry, Index};
use crate::indexing_status::{self, Indexing};
use crate::search::{self, ContentHit, KINDS, Query, SortKey};
use crate::suggest::{self, Situation, Suggestion};

/// Rows sent with the results; the page asks for the rest as it scrolls.
pub const FIRST_PAGE: usize = 60;
/// Rows sent for each later request.
pub const MORE_PAGE: usize = 200;
/// Longer messages are refused unread.
pub const MAX_MESSAGE: usize = 16 * 1024;
/// Longest search text accepted, in characters.
pub const MAX_QUERY: usize = 1000;
/// Longest new file name accepted, in characters (Windows' own limit).
pub const MAX_NAME: usize = 255;
/// Recent searches listed in the history menu.
pub const RECENT_SEARCHES: usize = 15;

/// The size filter's choices: (the `size:` filter it stands for, Arabic, English).
pub const SIZES: &[(&str, &str, &str)] = &[
    ("", "أي حجم", "Any size"),
    ("size:<100kb", "أصغر من 100 كيلوبايت", "Under 100 KB"),
    ("size:100kb..10mb", "من 100 كيلوبايت إلى 10 ميغابايت", "100 KB to 10 MB"),
    ("size:>10mb", "أكبر من 10 ميغابايت", "Over 10 MB"),
    ("size:>1gb", "أكبر من 1 غيغابايت", "Over 1 GB"),
];

/// The date filter's choices: (the `dm:` filter it stands for, Arabic, English).
pub const DATES: &[(&str, &str, &str)] = &[
    ("", "أي وقت", "Any time"),
    ("dm:today", "اليوم", "Today"),
    ("dm:7d", "آخر 7 أيام", "Last 7 days"),
    ("dm:30d", "آخر 30 يوماً", "Last 30 days"),
    ("dm:thisyear", "هذه السنة", "This year"),
    ("dm:<1y", "أقدم من سنة", "Older than a year"),
];

// ------------------------------------------------------------------ page → program

/// Which list a row belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum List {
    /// The search results of search `seq`.
    Results,
    /// The recently opened files shown while the box is empty.
    Recent,
}

/// How the results are ordered (the classic window's column sort).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Sort {
    /// Best matches first (and often-opened files before them); path order
    /// for a search inside documents, as in the classic window.
    #[default]
    Best,
    Name,
    Path,
    Size,
    Date,
}

impl Sort {
    pub fn key(self) -> SortKey {
        match self {
            Sort::Best => SortKey::Relevance,
            Sort::Name => SortKey::Name,
            Sort::Path => SortKey::Path,
            Sort::Size => SortKey::Size,
            Sort::Date => SortKey::Date,
        }
    }

    /// As the settings keep it: the classic window's column, -1 for best match.
    pub fn column(self) -> i32 {
        self.key().column().map_or(-1, |c| c as i32)
    }

    pub fn from_column(col: i32) -> Sort {
        match usize::try_from(col).map(SortKey::from_column) {
            Ok(SortKey::Name) => Sort::Name,
            Ok(SortKey::Path) => Sort::Path,
            Ok(SortKey::Size) => Sort::Size,
            Ok(SortKey::Date) => Sort::Date,
            _ => Sort::Best,
        }
    }
}

/// An on/off setting the page can change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Setting {
    /// Arabic-Indic digits (٠١٢٣).
    Digits,
    /// Start with Windows, in the background.
    Autostart,
    /// Index document contents in the background.
    ContentIndex,
    /// Update the index when files change.
    AutoIndex,
    /// Leave Windows' own files out of the results.
    HideSystem,
    /// Ctrl+Alt+F opens the quick search (otherwise this window).
    HotkeyQuick,
    /// The search box above the clock.
    Floating,
    /// The icon in the notification area.
    TrayIcon,
}

/// The interface language the page can choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LangCode {
    Ar,
    En,
}

impl LangCode {
    pub fn lang(self) -> Lang {
        match self {
            LangCode::Ar => Lang::Ar,
            LangCode::En => Lang::En,
        }
    }
}

/// A request of the page.
///
/// A request about rows names the list they were sent in: `seq` (the search)
/// and `ver`, the version of the list the program gave it (see [`Lists`]).
/// A row is named by its place, and the same search can be listed again in
/// another order or with other files under the same `seq`, so a request for
/// another version than the one listed now is refused, never applied to
/// whatever file is at that place now.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    /// The page has loaded: send the filters, settings, status, history and
    /// the recent files.
    Hello {},
    /// Search for `q` with file type `kind` (an index into [`KINDS`]), size
    /// `size` ([`SIZES`]) and date `date` ([`DATES`]), file names only or
    /// inside documents too (`content`), ordered by `sort`. `seq` numbers
    /// the searches; answers carry it back.
    Search {
        seq: u64,
        q: String,
        #[serde(default)]
        kind: usize,
        #[serde(default)]
        size: usize,
        #[serde(default)]
        date: usize,
        #[serde(default)]
        content: bool,
        #[serde(default)]
        sort: Sort,
        #[serde(default)]
        desc: bool,
    },
    /// The rows of search `seq` (version `ver`) from `from` on.
    More { seq: u64, ver: u64, from: usize },
    /// Orders the results of search `seq` again without searching again
    /// (a search inside documents; a name search simply runs again).
    Sort { seq: u64, sort: Sort, #[serde(default)] desc: bool },
    /// Stops search `seq` inside documents.
    Stop { seq: u64 },
    /// Every matching line of row `id` of search `seq` (inside documents).
    Matches { seq: u64, ver: u64, id: u32 },
    /// Open the file of row `id`.
    Open { list: List, seq: u64, ver: u64, id: u32 },
    /// Show the file of row `id` in Explorer.
    Reveal { list: List, seq: u64, ver: u64, id: u32 },
    /// Copy the full path of row `id`.
    Copy { list: List, seq: u64, ver: u64, id: u32 },
    /// Put the file of row `id` itself on the clipboard (to paste in Explorer).
    CopyFile { list: List, seq: u64, ver: u64, id: u32 },
    /// Copy the paths of every result of search `seq`, one per line.
    CopyAll { seq: u64, ver: u64 },
    /// Rename the file of row `id` to `name` (a name, not a path).
    Rename { list: List, seq: u64, ver: u64, id: u32, name: String },
    /// Move the file of row `id` to the Recycle Bin (the page asked first).
    Recycle { list: List, seq: u64, ver: u64, id: u32 },
    /// Search only in the folder of row `id` (the folder itself for a folder).
    Scope { list: List, seq: u64, ver: u64, id: u32 },
    /// Choose the folder to search in with Windows' folder dialog.
    PickFolder {},
    /// Search all drives again.
    AllDrives {},
    /// Run saved search `id` (its place in the history's saved list).
    Saved { id: u32 },
    /// Save the current search (with the folder searched now).
    SaveSearch {
        q: String,
        #[serde(default)]
        kind: usize,
        #[serde(default)]
        content: bool,
    },
    /// Forget the recent searches.
    ClearRecent {},
    /// Delete every saved search.
    ClearSaved {},
    /// Switch `key` on or off.
    Set { key: Setting, on: bool },
    /// Use `lang` from the next start.
    Lang { lang: LangCode },
    /// Start the program again now (after a language change).
    Restart {},
    /// Index the drives again.
    Reindex {},
    /// Search the web for `q` in the default browser.
    Web { q: String },
    /// Hide the window (Esc in an empty box).
    Hide {},
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
    let text_ok = |q: &str| q.chars().count() <= MAX_QUERY;
    let ok = match &req {
        Request::Search { q, kind, size, date, .. } => *kind < KINDS.len() && *size < SIZES.len() && *date < DATES.len() && text_ok(q),
        Request::SaveSearch { q, kind, .. } => *kind < KINDS.len() && !q.trim().is_empty() && text_ok(q),
        Request::Web { q } => !q.trim().is_empty() && text_ok(q),
        Request::Rename { name, .. } => !name.trim().is_empty() && name.chars().count() <= MAX_NAME,
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

/// Text cut in pieces, `true` for the pieces that match the search.
pub type Pieces = Vec<(String, bool)>;

/// One row of a list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RowOut {
    /// Its place in the list; what the page sends back to act on it.
    pub id: u32,
    /// The name in pieces, `true` for the pieces that match the search.
    pub name: Pieces,
    /// The folder it is in.
    pub dir: String,
    pub size: u64,
    /// Modified, seconds since 1970 (UTC).
    pub mtime: i64,
    /// Lower-case extension, "" for none.
    pub ext: String,
    pub folder: bool,
    /// The text around the first match inside the document (search inside documents).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<Pieces>,
    /// How many places in the document match.
    #[serde(skip_serializing_if = "is_zero")]
    pub matches: u32,
    /// Moved to the Recycle Bin since it was listed.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub gone: bool,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
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

/// A saved search, as the history menu lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SavedOut {
    pub id: u32,
    pub name: String,
    /// It searches inside documents.
    pub content: bool,
    /// It is limited to a folder.
    pub scoped: bool,
}

/// The settings page's values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingsOut {
    /// The language the interface is in now.
    pub lang: &'static str,
    /// The language chosen for the next start (differs after a change).
    pub next_lang: &'static str,
    pub digits: bool,
    pub autostart: bool,
    pub content_index: bool,
    pub auto_index: bool,
    pub hide_system: bool,
    pub hotkey_quick: bool,
    pub floating: bool,
    pub tray_icon: bool,
    pub version: &'static str,
}

/// A message to the page.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Outbound {
    Hello {
        /// The file-type filter's choices, in the interface language.
        kinds: Vec<String>,
        /// The size and date filters' choices.
        sizes: Vec<String>,
        dates: Vec<String>,
        /// The file type and order used last.
        kind: usize,
        sort: Sort,
        desc: bool,
        /// Write numbers with Arabic-Indic digits.
        digits: bool,
        version: &'static str,
    },
    Results {
        seq: u64,
        /// The version of this list ([`Lists`]): requests about its rows name it.
        ver: u64,
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
        /// The folder searched, "" for all drives.
        scope: String,
        /// A search inside documents, and whether it is still running.
        content: bool,
        running: bool,
        /// Problems in the search text, in the interface language.
        warnings: Vec<String>,
    },
    Rows {
        seq: u64,
        ver: u64,
        from: usize,
        rows: Vec<RowOut>,
    },
    /// How far a search inside documents got.
    Progress {
        seq: u64,
        /// Documents found so far.
        total: usize,
        done: bool,
        text: String,
    },
    /// Every matching line of a document: (line number, the line in pieces).
    Matches {
        seq: u64,
        ver: u64,
        id: u32,
        count: u32,
        lines: Vec<(u32, Pieces)>,
    },
    Recent {
        ver: u64,
        rows: Vec<RowOut>,
    },
    /// The recent and saved searches.
    History {
        recent: Vec<String>,
        saved: Vec<SavedOut>,
    },
    /// The folder searched now, "" for all drives; `search`: run the
    /// current search again in it.
    Scope {
        path: String,
        search: bool,
    },
    /// Run this search (a saved one): its text, type, and inside documents or not.
    Apply {
        q: String,
        kind: usize,
        content: bool,
    },
    /// A row's file was renamed.
    Renamed {
        list: List,
        seq: u64,
        ver: u64,
        id: u32,
        name: String,
    },
    /// A row's file went to the Recycle Bin.
    Removed {
        list: List,
        seq: u64,
        ver: u64,
        id: u32,
    },
    Settings(SettingsOut),
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

/// The labels of [`SIZES`] or [`DATES`] in `lang`.
pub fn labels(list: &[(&str, &str, &str)], lang: Lang) -> Vec<String> {
    list.iter().map(|(_, ar, en)| lang.pick(*ar, *en).to_string()).collect()
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

/// The recent and saved searches for the history menu.
pub fn history(h: &History) -> Outbound {
    Outbound::History {
        recent: h.recent.iter().take(RECENT_SEARCHES).cloned().collect(),
        saved: h
            .saved
            .iter()
            .enumerate()
            .map(|(i, s)| SavedOut { id: i as u32, name: s.name.clone(), content: s.content, scoped: !s.scope.is_empty() })
            .collect(),
    }
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

/// The folder to search in for "search in this folder" on `path`: the
/// folder itself, or the one a file is in; with a final `\`, as the classic
/// window's folder chooser gives it.
pub fn scope_for(path: &str, is_dir: bool) -> String {
    let dir = if is_dir { path.trim_end_matches(['\\', '/']) } else { parent(path) };
    let mut dir = dir.to_string();
    if !dir.is_empty() && !dir.ends_with(['\\', '/']) {
        dir.push('\\');
    }
    dir
}

/// `name` cut at the byte ranges of `marks` (sorted, merged), `true` inside them.
pub fn pieces(name: &str, marks: &[(usize, usize)]) -> Pieces {
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

/// `text` with the parts matching `q` marked.
pub fn marked(text: &str, q: &Query) -> Pieces {
    pieces(text, &search::match_ranges(text, q))
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
        snippet: None,
        matches: 0,
        gone: false,
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

// ------------------------------------------------------------------ rename

/// Why a new name was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadName {
    /// Empty, or the same name.
    Unchanged,
    /// It holds `\ / : * ? " < > |` or a control character.
    Characters,
    /// Windows does not allow it (a device name, a final dot or space).
    Reserved,
}

impl BadName {
    pub fn message(self, lang: Lang) -> &'static str {
        match self {
            BadName::Unchanged => lang.pick("الاسم لم يتغيّر.", "The name did not change."),
            BadName::Characters => lang.pick(
                "لا يمكن أن يحتوي الاسم على أي من هذه الرموز: \\ / : * ? \" < > |",
                "A name cannot contain any of these characters: \\ / : * ? \" < > |",
            ),
            BadName::Reserved => lang.pick("هذا الاسم غير مسموح في ويندوز.", "Windows does not allow this name."),
        }
    }
}

/// The full new path for renaming `old` to `name`, or why `name` can't be
/// used (the same checks as the classic window's in-place rename). Whether
/// another file already has that name is for the caller to check on disk.
pub fn rename_target(old: &str, name: &str) -> Result<String, BadName> {
    let name = name.trim();
    let old_name = old.rsplit(['\\', '/']).next().unwrap_or(old);
    if name.is_empty() || name == old_name {
        return Err(BadName::Unchanged);
    }
    if name.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) || name.chars().any(char::is_control) {
        return Err(BadName::Characters);
    }
    // Windows drops a final dot or space and reserves device names.
    if name.ends_with(['.', ' ']) || reserved_name(name) {
        return Err(BadName::Reserved);
    }
    Ok(match old.rfind(['\\', '/']) {
        Some(i) => format!("{}\\{name}", &old[..i]),
        None => name.to_string(),
    })
}

/// Whether Windows keeps `name` for a device, with or without an
/// extension: CON, PRN, AUX, NUL, CONIN$, CONOUT$, COM0-9 and LPT0-9, and
/// COM/LPT with a superscript ¹ ² ³ (Windows reads those as digits too).
pub fn reserved_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$") {
        return true;
    }
    let port = stem.strip_prefix("COM").or_else(|| stem.strip_prefix("LPT"));
    let mut rest = port.into_iter().flat_map(str::chars);
    matches!((rest.next(), rest.next()), (Some('0'..='9' | '¹' | '²' | '³'), None))
}

// ------------------------------------------------------------------ the lists on the page

/// Why the row a request names can't be acted on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Miss {
    /// The request names a list that is no longer the one listed (a newer
    /// search, the same one run again, or a new order came since): its row
    /// number may be another file now.
    Stale,
    /// No such row, or its file went to the Recycle Bin.
    Gone,
}

impl Miss {
    /// What to tell the user, if anything.
    pub fn message(self, lang: Lang) -> Option<&'static str> {
        match self {
            Miss::Stale => Some(lang.pick("تغيّرت القائمة للتو، فلم يُنفَّذ شيء. حاول مرة أخرى.", "The list just changed, so nothing was done. Try again.")),
            Miss::Gone => None,
        }
    }
}

/// The recently opened files listed while the search box is empty.
#[derive(Debug, Default)]
pub struct RecentList {
    /// Its version; 0 before the first one was sent.
    pub ver: u64,
    pub paths: Vec<String>,
}

/// The lists the page was sent, to act on their rows.
///
/// Every list sent, and every new order of one, gets a version of its own
/// ([`Lists::next_ver`], never given twice in a run), and the page names it
/// with every request about a row. The rows are named by their place, so
/// the version is what keeps a request about an older list (a name search
/// run again in the background, a search inside documents put in order when
/// it ends, another order chosen) from reaching the file at that place now.
#[derive(Default)]
pub struct Lists {
    /// The last version given.
    last: u64,
    /// Versions up to this one were given before the page last loaded: their
    /// lists are not the page's.
    floor: u64,
    /// The search results listed now.
    pub shown: Option<Shown>,
    pub recent: RecentList,
    /// The newest name search asked for while one was running.
    pub pending: Option<Params>,
}

impl Lists {
    /// A new version, for a list about to be made or reordered.
    pub fn next_ver(&mut self) -> u64 {
        self.last += 1;
        self.last
    }

    /// The page (re)loaded: it lists nothing yet, and nothing made for the
    /// page before (a search still running, one waiting) is for this one.
    pub fn reset(&mut self) {
        self.floor = self.last;
        self.shown = None;
        self.recent = RecentList::default();
        self.pending = None;
    }

    /// Whether a list numbered `ver` was made for the page loaded now.
    pub fn fresh(&self, ver: u64) -> bool {
        ver > self.floor
    }

    /// Keeps `paths` as the recent files listed now; returns their version.
    pub fn set_recent(&mut self, paths: Vec<String>) -> u64 {
        let ver = self.next_ver();
        self.recent = RecentList { ver, paths };
        ver
    }

    /// The results listed now, if they are version `ver` of search `seq`.
    pub fn shown_at(&self, seq: u64, ver: u64) -> Result<&Shown, Miss> {
        self.shown.as_ref().filter(|s| s.seq == seq && s.ver == ver).ok_or(Miss::Stale)
    }

    pub fn shown_at_mut(&mut self, seq: u64, ver: u64) -> Result<&mut Shown, Miss> {
        self.shown.as_mut().filter(|s| s.seq == seq && s.ver == ver).ok_or(Miss::Stale)
    }

    /// The full path of row `id` of version `ver` of `list` (`seq` for the results).
    pub fn path(&self, list: List, seq: u64, ver: u64, id: u32) -> Result<&str, Miss> {
        match list {
            List::Results => self.shown_at(seq, ver)?.path(id).ok_or(Miss::Gone),
            List::Recent if ver == 0 || ver != self.recent.ver => Err(Miss::Stale),
            List::Recent => self.recent.paths.get(id as usize).map(String::as_str).ok_or(Miss::Gone),
        }
    }

    /// Puts the results of search `seq` in another order, as a new version.
    pub fn resort(&mut self, seq: u64, sort: Sort, desc: bool) -> Option<&mut Shown> {
        let ver = self.next_ver();
        let s = self.shown.as_mut().filter(|s| s.seq == seq)?;
        s.resort(sort, desc);
        s.ver = ver;
        Some(s)
    }

    /// A name search ended with `shown`. It becomes the list unless a newer
    /// one is listed or the page loaded again since it started. Returns
    /// whether to send it to the page (not when a newer search waits, which
    /// the page would drop it for), and the search waiting to run.
    pub fn finished(&mut self, shown: Shown) -> (bool, Option<Params>) {
        let pending = self.pending.take();
        if !self.fresh(shown.ver) || self.shown.as_ref().is_some_and(|s| s.seq > shown.seq) {
            return (false, pending);
        }
        let send = pending.as_ref().is_none_or(|p| p.seq == shown.seq);
        self.shown = Some(shown);
        (send, pending)
    }

    /// The search to remember in the history when a row of `list` is used:
    /// the results' own search; none for the recent files, which no search listed.
    pub fn used_search(&self, list: List) -> Option<&str> {
        match list {
            List::Results => self.shown.as_ref().map(|s| s.params.text.as_str()).filter(|t| !t.trim().is_empty()),
            List::Recent => None,
        }
    }
}

// ------------------------------------------------------------------ search

/// What a search needs besides the index.
#[derive(Debug, Clone, Default)]
pub struct Params {
    pub seq: u64,
    /// The version its list gets ([`Lists::next_ver`], numbered when the search starts).
    pub ver: u64,
    pub text: String,
    /// Index into [`KINDS`].
    pub kind: usize,
    /// Index into [`SIZES`].
    pub size: usize,
    /// Index into [`DATES`].
    pub date: usize,
    /// Inside documents too.
    pub content: bool,
    pub sort: Sort,
    pub desc: bool,
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

/// What a search inside documents found in one row.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Found {
    pub snippet: String,
    pub count: u32,
    pub lines: Vec<(u32, String)>,
}

/// The rows a search found, kept to answer the page's later requests.
pub struct Shown {
    pub seq: u64,
    /// This list's version: a new one for every list and every new order ([`Lists`]).
    pub ver: u64,
    pub index: Arc<Index>,
    pub hits: Vec<u32>,
    /// The query whose matches are marked (a fallback's, when one found the rows).
    pub marked: Query,
    pub params: Params,
    /// For a search inside documents, what was found in each row (by entry).
    pub found: HashMap<u32, Found>,
    /// Rows renamed since they were listed (their new path), or moved to
    /// the Recycle Bin (`None`). The index changed under them, so they are
    /// looked up here first.
    pub changed: HashMap<u32, Option<String>>,
    /// The search inside documents has finished (always true for names).
    pub done: bool,
}

impl Shown {
    /// An empty list for a search inside documents with `q`, filled as it runs.
    pub fn documents(index: Arc<Index>, q: Query, params: Params) -> Shown {
        Shown { seq: params.seq, ver: params.ver, index, hits: Vec::new(), marked: q, params, found: HashMap::new(), changed: HashMap::new(), done: false }
    }

    /// Adds what a search inside documents found since the last call.
    pub fn absorb(&mut self, hits: Vec<ContentHit>) {
        for h in hits {
            if let std::collections::hash_map::Entry::Vacant(slot) = self.found.entry(h.index) {
                slot.insert(Found { snippet: h.snippet, count: h.count, lines: h.lines });
                self.hits.push(h.index);
            }
        }
    }

    /// Orders the rows by `sort` without searching again. Best match is
    /// path order for a search inside documents (its hits have no score).
    pub fn resort(&mut self, sort: Sort, desc: bool) {
        self.params.sort = sort;
        self.params.desc = desc;
        // Renamed and removed rows are found by their place: keep them with their files.
        let changed: Vec<(u32, Option<String>)> =
            self.changed.drain().filter_map(|(id, c)| self.hits.get(id as usize).map(|&e| (e, c))).collect();
        if sort == Sort::Best {
            // (A name search runs again instead, for its own best order.)
            self.hits.sort_unstable();
        } else {
            sort_hits(&self.index, &mut self.hits, sort, desc);
        }
        let place: HashMap<u32, u32> = self.hits.iter().enumerate().map(|(i, &e)| (e, i as u32)).collect();
        self.changed = changed.into_iter().filter_map(|(e, c)| place.get(&e).map(|&id| (id, c))).collect();
    }

    /// The message with rows `from..from + n`, for the page's "more".
    pub fn rows(&self, from: usize, n: usize) -> Outbound {
        Outbound::Rows { seq: self.seq, ver: self.ver, from, rows: self.page(from, n) }
    }

    /// Rows `from..from + n`.
    pub fn page(&self, from: usize, n: usize) -> Vec<RowOut> {
        let end = from.saturating_add(n).min(self.hits.len());
        (from.min(end)..end).filter_map(|i| self.row(i)).collect()
    }

    fn row(&self, i: usize) -> Option<RowOut> {
        let entry = *self.hits.get(i)?;
        let e = self.index.entries.get(entry as usize)?;
        let id = i as u32;
        let mut r = match self.changed.get(&id) {
            Some(Some(new)) => {
                let renamed = Entry { path: new.as_str().into(), ..e.clone() };
                row(id, &renamed, Some(&self.marked))
            }
            Some(None) => RowOut { gone: true, ..row(id, e, None) },
            None => row(id, e, Some(&self.marked)),
        };
        if let Some(f) = self.found.get(&entry) {
            r.snippet = Some(marked(&f.snippet, &self.marked));
            r.matches = f.count;
        }
        Some(r)
    }

    /// The full path of row `id`, `None` once it went to the Recycle Bin.
    pub fn path(&self, id: u32) -> Option<&str> {
        if let Some(c) = self.changed.get(&id) {
            return c.as_deref();
        }
        let i = *self.hits.get(id as usize)?;
        self.index.entries.get(i as usize).map(|e| &*e.path)
    }

    /// Whether row `id` is a folder.
    pub fn is_dir(&self, id: u32) -> bool {
        self.hits.get(id as usize).and_then(|&i| self.index.entries.get(i as usize)).is_some_and(|e| e.is_dir)
    }

    /// The paths of every row still there, in order.
    pub fn all_paths(&self) -> Vec<&str> {
        (0..self.hits.len() as u32).filter_map(|id| self.path(id)).collect()
    }

    /// Every matching line of row `id` (search inside documents only).
    pub fn matches(&self, id: u32) -> Option<Outbound> {
        let f = self.found.get(self.hits.get(id as usize)?)?;
        let lines = f.lines.iter().map(|(n, l)| (*n, marked(l, &self.marked))).collect();
        Some(Outbound::Matches { seq: self.seq, ver: self.ver, id, count: f.count, lines })
    }

    /// The message with the first rows (sent again when a search inside
    /// documents ends or the order changes).
    pub fn results(&self, ms: usize, note: Option<String>, suggestions: Vec<SuggestionOut>, lang: Lang) -> Outbound {
        Outbound::Results {
            seq: self.seq,
            ver: self.ver,
            total: self.hits.len(),
            ms,
            rows: self.page(0, FIRST_PAGE),
            note,
            suggestions,
            files: self.index.entries.len(),
            kind_label: if self.params.kind == 0 { String::new() } else { kinds(lang).swap_remove(self.params.kind) },
            scope: self.params.scope.clone(),
            content: self.params.content,
            running: !self.done,
            warnings: self.marked.warnings.clone(),
        }
    }
}

/// [`search::sort_hits`], with folders sorted by everything inside them by size.
fn sort_hits(index: &Arc<Index>, hits: &mut [u32], sort: Sort, desc: bool) {
    match sort {
        Sort::Best => {}
        Sort::Size => crate::folder_sizes::sort_hits_by_total(hits, &crate::folder_sizes::for_index(index), desc),
        _ => search::sort_hits(&index.entries, hits, sort.key(), desc),
    }
}

/// The query of a filter choice (`size:` / `dm:`), read with the
/// program's own parser; `None` for "any".
fn filter_query(list: &[(&str, &str, &str)], choice: usize) -> Option<Query> {
    list.get(choice).filter(|c| !c.0.is_empty()).map(|c| Query::parse(c.0))
}

/// The overlap of a typed range and a chosen one (empty when they don't
/// meet, so nothing passes).
fn overlap<T: Ord + Copy>(typed: Option<(T, T)>, chosen: Option<(T, T)>) -> Option<(T, T)> {
    match (typed, chosen) {
        (Some((a, b)), Some((c, d))) => Some((a.max(c), b.min(d))),
        (t, c) => c.or(t),
    }
}

/// How the search turns a text into a [`Query`]: the type, size and date
/// filters chosen, and Windows' own files when they are shown. An empty
/// text stays empty, so a filter alone never lists the whole disk.
pub fn parser(p: &Params) -> impl Fn(&str) -> Query + Sync + use<> {
    let (kind, show_system) = (p.kind, p.show_system);
    let size = filter_query(SIZES, p.size).and_then(|q| q.size);
    let date = filter_query(DATES, p.date).and_then(|q| q.mtime);
    move |t: &str| {
        let mut q = Query::parse(t);
        if !q.is_empty() {
            q.size = overlap(q.size, size);
            q.mtime = overlap(q.mtime, date);
        }
        q.apply_kind(kind);
        q.show_system |= show_system;
        q
    }
}

/// Runs the filename search of `p` over `index`, and the first message for
/// the page. Meant for a background thread.
pub fn search(index: Arc<Index>, p: Params, lang: Lang) -> (Shown, Outbound) {
    let started = Instant::now();
    let parse = parser(&p);
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
    if p.sort == Sort::Best {
        // Files the user opens often come first.
        search::promote(&mut hits, &p.boosted);
    } else {
        sort_hits(&index, &mut hits, p.sort, p.desc);
    }
    let did_you_mean = if p.suggest && hits.is_empty() && !q.is_empty() {
        suggest::did_you_mean(&index.entries, &p.text, &p.scope, &parse)
    } else {
        None
    };
    let ms = started.elapsed().as_millis() as usize;
    let contents = q.has_text_criteria();
    let marked = match &fallback {
        Some(fb) => parse(&fb.query),
        None => q,
    };
    let situation = Situation {
        did_you_mean,
        kind_filtered: p.kind != 0,
        scoped: !p.scope.is_empty(),
        contents,
        web: true,
    };
    let suggestions = if hits.is_empty() { suggestions(&p.text, situation) } else { Vec::new() };
    let note = fallback.map(|fb| fb.status(&p.text));
    let shown = Shown { seq: p.seq, ver: p.ver, index, hits, marked, params: p, found: HashMap::new(), changed: HashMap::new(), done: true };
    let msg = shown.results(ms, note, suggestions, lang);
    (shown, msg)
}

/// The query of a search inside documents, or `None` when it has nothing
/// to look for in a text (only filters, or nothing typed).
pub fn content_query(p: &Params) -> Option<Query> {
    let q = parser(p)(&p.text);
    q.has_text_criteria().then_some(q)
}

/// What to offer when a search inside documents for `p` found nothing.
pub fn content_suggestions(p: &Params) -> Vec<SuggestionOut> {
    suggestions(&p.text, Situation { kind_filtered: p.kind != 0, scoped: !p.scope.is_empty(), web: true, ..Situation::default() })
}

/// What to offer when `text` found nothing: a better spelling, a wider
/// search (all types, all drives, inside documents), the web.
pub fn suggestions(text: &str, situation: Situation) -> Vec<SuggestionOut> {
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

/// How far a search inside documents got, for the status bar.
pub struct ContentState {
    /// Documents read, and documents to read.
    pub checked: usize,
    pub total: usize,
    /// Documents found.
    pub found: usize,
    /// Texts that came from the content index.
    pub cached: usize,
    pub done: bool,
    /// What is being indexed meanwhile.
    pub indexing: Indexing,
    /// Windows can't read Arabic text in images.
    pub ocr_arabic_missing: bool,
}

/// The status bar's text for a search inside documents (the classic window's wording).
pub fn content_text(s: &ContentState, lang: Lang) -> String {
    let n = indexing_status::group_digits;
    if s.done {
        let cached = match (s.cached, lang) {
            (0, _) => String::new(),
            (c, Lang::Ar) => format!(" (من الفهرس: {})", n(c)),
            (c, Lang::En) => format!(" ({} from the index)", n(c)),
        };
        let mut t = match lang {
            Lang::Ar => format!("انتهى البحث داخل {} مستند: وجدت {} ملف.{cached}", n(s.total), n(s.found)),
            Lang::En => format!("Searched {} documents: found {} files.{cached}", n(s.total), n(s.found)),
        };
        // Documents not in the index yet were not searched.
        if s.found == 0 && s.indexing.names_incomplete() {
            t = format!("{t} {}.", indexing_status::incomplete_text(lang));
        }
        if s.ocr_arabic_missing {
            t = format!("{t} {}", indexing_status::ocr_arabic_missing_text(lang));
        }
        return t;
    }
    if s.found == 0
        && let Some(t) = indexing_status::content_search_text(lang, s.indexing, s.checked, s.total, &n)
    {
        return t;
    }
    let (checked, total, found) = (n(s.checked.min(s.total)), n(s.total), n(s.found));
    match lang {
        Lang::Ar => format!("جارِ البحث داخل المستندات… تم فحص {checked} من {total} — وجدت {found}"),
        Lang::En => format!("Searching inside documents… checked {checked} of {total} — found {found}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "http://instafind.localhost/";
    const TOKEN: &str = "00112233445566778899aabbccddeeff";

    fn entry(path: &str, size: u64, mtime: i64, is_dir: bool) -> Entry {
        let name = path.rsplit('\\').next().unwrap();
        Entry { path: path.into(), size, mtime, is_dir, norm_name: crate::normalize::normalize(name).into() }
    }

    /// Synthetic files, sorted by path as the index keeps them.
    fn index() -> Arc<Index> {
        let now = crate::index::now_secs();
        let mut entries = vec![
            entry("C:\\Users\\Sara\\Documents\\العمل\\التقرير السنوي.pdf", 2_400_000, now - 3 * 86_400, false),
            entry("C:\\Users\\Sara\\Documents\\تقرير المبيعات 2026.xlsx", 48_000, now - 3600, false),
            entry("C:\\Users\\Sara\\Desktop\\notes.txt", 2_000, now - 400 * 86_400, false),
            entry("D:\\Projects\\تقرير الصيانة", 0, now - 10 * 86_400, true),
            entry("D:\\Projects\\تقرير الصيانة\\تقرير قديم.docx", 20_000_000, now - 500 * 86_400, false),
        ];
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Arc::new(Index { entries, ..Index::default() })
    }

    fn name_of(r: &RowOut) -> String {
        r.name.iter().map(|(t, _)| t.as_str()).collect()
    }

    fn names(msg: &Outbound) -> Vec<String> {
        let Outbound::Results { rows, .. } = msg else { panic!("{msg:?}") };
        rows.iter().map(name_of).collect()
    }

    fn req(body: &str) -> Result<Request, Rejected> {
        parse(&format!(r#"{{"t":"{TOKEN}",{body}}}"#), SRC, TOKEN)
    }

    #[test]
    fn accepts_only_the_page_with_the_token() {
        let ok = |b: &str| parse(b, SRC, TOKEN);
        assert_eq!(
            req(r#""op":"search","seq":3,"q":"تقرير","kind":1"#),
            Ok(Request::Search { seq: 3, q: "تقرير".into(), kind: 1, size: 0, date: 0, content: false, sort: Sort::Best, desc: false })
        );
        assert_eq!(req(r#""op":"hello""#), Ok(Request::Hello {}));
        assert_eq!(req(r#""op":"open","list":"recent","seq":0,"ver":4,"id":2"#), Ok(Request::Open { list: List::Recent, seq: 0, ver: 4, id: 2 }));
        // Another page, a missing or wrong token, junk, unknown fields or values.
        let hello = format!(r#"{{"t":"{TOKEN}","op":"hello"}}"#);
        for src in ["https://evil.example/", "http://instafind.localhost.evil.example/", "http://evil.localhost/", "file:///C:/x.html", ""] {
            assert_eq!(parse(&hello, src, TOKEN), Err(Rejected::Origin), "{src}");
        }
        assert_eq!(parse(&hello, "http://instafind.localhost", TOKEN), Ok(Request::Hello {}));
        assert_eq!(ok(r#"{"op":"hello"}"#), Err(Rejected::Token));
        assert_eq!(ok(r#"{"t":"0011","op":"hello"}"#), Err(Rejected::Token));
        assert_eq!(ok(r#"{"t":5,"op":"hello"}"#), Err(Rejected::Token));
        assert_eq!(parse(&hello, SRC, ""), Err(Rejected::Token));
        assert_eq!(ok("not json"), Err(Rejected::Malformed));
        assert_eq!(req(r#""op":"read_file","path":"C:\\x""#), Err(Rejected::Malformed));
        assert_eq!(req(r#""op":"open","list":"results","seq":1,"ver":1,"id":1,"path":"C:\\x""#), Err(Rejected::Malformed));
        assert_eq!(req(r#""op":"search","seq":1,"q":"a","kind":99"#), Err(Rejected::Invalid));
        assert_eq!(req(r#""op":"web","q":"  ""#), Err(Rejected::Invalid));
        let long = format!(r#""op":"search","seq":1,"q":"{}""#, "a".repeat(MAX_QUERY + 1));
        assert_eq!(req(&long), Err(Rejected::Invalid));
        assert_eq!(req(&format!(r#""op":"web","q":"{}""#, "a".repeat(MAX_MESSAGE))), Err(Rejected::TooLong));
    }

    #[test]
    fn phase_two_requests_are_typed_and_checked() {
        assert_eq!(
            req(r#""op":"search","seq":4,"q":"x","size":3,"date":2,"content":true,"sort":"date","desc":true"#),
            Ok(Request::Search { seq: 4, q: "x".into(), kind: 0, size: 3, date: 2, content: true, sort: Sort::Date, desc: true })
        );
        for bad in [r#""size":99"#, r#""date":99"#, r#""sort":"random""#, r#""content":"yes""#, r#""scope":"C:\\""#] {
            assert!(req(&format!(r#""op":"search","seq":1,"q":"x",{bad}"#)).is_err(), "{bad}");
        }
        assert_eq!(req(r#""op":"sort","seq":2,"sort":"size""#), Ok(Request::Sort { seq: 2, sort: Sort::Size, desc: false }));
        assert_eq!(req(r#""op":"matches","seq":2,"ver":3,"id":0"#), Ok(Request::Matches { seq: 2, ver: 3, id: 0 }));
        assert_eq!(req(r#""op":"copy_file","list":"results","seq":2,"ver":3,"id":1"#), Ok(Request::CopyFile { list: List::Results, seq: 2, ver: 3, id: 1 }));
        assert_eq!(req(r#""op":"copy_all","seq":2,"ver":3"#), Ok(Request::CopyAll { seq: 2, ver: 3 }));
        assert_eq!(req(r#""op":"recycle","list":"recent","seq":0,"ver":1,"id":3"#), Ok(Request::Recycle { list: List::Recent, seq: 0, ver: 1, id: 3 }));
        assert_eq!(req(r#""op":"scope","list":"results","seq":5,"ver":6,"id":0"#), Ok(Request::Scope { list: List::Results, seq: 5, ver: 6, id: 0 }));
        assert_eq!(req(r#""op":"pick_folder""#), Ok(Request::PickFolder {}));
        assert_eq!(req(r#""op":"all_drives""#), Ok(Request::AllDrives {}));
        // The page can't name a folder or a path, only rows.
        assert_eq!(req(r#""op":"pick_folder","path":"C:\\Windows""#), Err(Rejected::Malformed));
        assert_eq!(req(r#""op":"hello","debug":true"#), Err(Rejected::Malformed));
        assert_eq!(req(r#""op":"hide","now":1"#), Err(Rejected::Malformed));
        assert_eq!(req(r#""op":"scope","path":"C:\\""#), Err(Rejected::Malformed));
        assert_eq!(
            req(r#""op":"rename","list":"results","seq":1,"ver":2,"id":0,"name":"جديد.txt""#),
            Ok(Request::Rename { list: List::Results, seq: 1, ver: 2, id: 0, name: "جديد.txt".into() })
        );
        assert_eq!(req(r#""op":"rename","list":"results","seq":1,"ver":2,"id":0,"name":" ""#), Err(Rejected::Invalid));
        assert_eq!(req(&format!(r#""op":"rename","list":"results","seq":1,"ver":2,"id":0,"name":"{}""#, "a".repeat(MAX_NAME + 1))), Err(Rejected::Invalid));
        assert_eq!(req(r#""op":"saved","id":2"#), Ok(Request::Saved { id: 2 }));
        assert_eq!(req(r#""op":"save_search","q":"عقد","content":true"#), Ok(Request::SaveSearch { q: "عقد".into(), kind: 0, content: true }));
        assert_eq!(req(r#""op":"save_search","q":"","content":true"#), Err(Rejected::Invalid));
        assert_eq!(req(r#""op":"save_search","q":"a","scope":"D:\\""#), Err(Rejected::Malformed));
        assert_eq!(req(r#""op":"clear_recent""#), Ok(Request::ClearRecent {}));
        assert_eq!(req(r#""op":"set","key":"hide_system","on":false"#), Ok(Request::Set { key: Setting::HideSystem, on: false }));
        assert_eq!(req(r#""op":"set","key":"run_command","on":true"#), Err(Rejected::Malformed));
        assert_eq!(req(r#""op":"set","key":"digits""#), Err(Rejected::Malformed));
        assert_eq!(req(r#""op":"lang","lang":"en""#), Ok(Request::Lang { lang: LangCode::En }));
        assert_eq!(req(r#""op":"lang","lang":"fr""#), Err(Rejected::Malformed));
        assert_eq!(req(r#""op":"restart""#), Ok(Request::Restart {}));
        assert_eq!(req(r#""op":"reindex""#), Ok(Request::Reindex {}));
        assert_eq!(req(r#""op":"stop","seq":9"#), Ok(Request::Stop { seq: 9 }));
    }

    #[test]
    fn sort_columns_match_the_classic_settings() {
        for s in [Sort::Best, Sort::Name, Sort::Path, Sort::Size, Sort::Date] {
            assert_eq!(Sort::from_column(s.column()), s);
        }
        assert_eq!(Sort::Best.column(), -1);
        assert_eq!(Sort::Date.column(), 3);
        assert_eq!(Sort::from_column(42), Sort::Best);
        assert_eq!(Sort::from_column(-7), Sort::Best);
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
        assert_eq!(scope_for("C:\\a\\b.txt", false), "C:\\a\\");
        assert_eq!(scope_for("C:\\b.txt", false), "C:\\");
        assert_eq!(scope_for("D:\\Projects\\تقرير الصيانة", true), "D:\\Projects\\تقرير الصيانة\\");
        assert_eq!(scope_for("D:\\", true), "D:\\");
        assert_eq!(scope_for("name", false), "");
    }

    #[test]
    fn search_marks_matches_and_pages() {
        let p = Params { seq: 7, text: "تقرير".into(), ..Params::default() };
        let (shown, msg) = search(index(), p, Lang::Ar);
        let Outbound::Results { seq, total, ref rows, ref note, ref suggestions, files, content, running, .. } = msg else { panic!() };
        assert_eq!((seq, files, note.clone(), suggestions.len(), content, running), (7, 5, None, 0, false, false));
        assert!(total >= 2, "{total}");
        let names: Vec<String> = rows.iter().map(name_of).collect();
        assert!(names.contains(&"تقرير المبيعات 2026.xlsx".to_string()), "{names:?}");
        let sales = rows.iter().find(|r| r.ext == "xlsx").unwrap();
        assert_eq!(sales.name[0], ("تقرير".to_string(), true));
        assert_eq!(sales.dir, "C:\\Users\\Sara\\Documents");
        assert_eq!((sales.snippet.as_ref(), sales.matches, sales.gone), (None, 0, false));
        assert_eq!(shown.path(sales.id), Some("C:\\Users\\Sara\\Documents\\تقرير المبيعات 2026.xlsx"));
        assert_eq!(shown.path(99), None);
        assert_eq!(shown.page(1, 1).len(), 1);
        assert_eq!(shown.page(1, 1)[0].id, 1);
        assert!(shown.page(50, 10).is_empty());
        assert!(shown.page(usize::MAX, usize::MAX).is_empty());
        // The row's JSON leaves out what only document results have.
        let json = serde_json::to_string(&shown.page(0, 1)[0]).unwrap();
        assert!(!json.contains("snippet") && !json.contains("matches") && !json.contains("gone"), "{json}");
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
        assert_eq!(kinds, ["all_kinds", "contents", "web"]);

        // In a folder: offer all drives; inside documents already: no "contents".
        let p = Params { seq: 3, text: "zzqqxx".into(), scope: "D:\\".into(), ..Params::default() };
        let (_, msg) = search(index(), p, Lang::Ar);
        let Outbound::Results { suggestions, scope, .. } = msg else { panic!() };
        assert_eq!(scope, "D:\\");
        assert_eq!(suggestions.iter().map(|s| s.kind).collect::<Vec<_>>(), ["all_drives", "contents", "web"]);
        let p = Params { text: "zzqqxx".into(), content: true, scope: "D:\\".into(), ..Params::default() };
        assert_eq!(content_suggestions(&p).iter().map(|s| s.kind).collect::<Vec<_>>(), ["all_drives", "web"]);
    }

    #[test]
    fn size_and_date_filters() {
        let find = |size: usize, date: usize, text: &str| {
            let p = Params { text: text.into(), size, date, ..Params::default() };
            let mut n = names(&search(index(), p, Lang::Ar).1);
            n.sort();
            n
        };
        assert_eq!(find(3, 0, "تقرير"), ["تقرير قديم.docx"]);
        assert_eq!(find(1, 0, "تقرير"), ["تقرير المبيعات 2026.xlsx"]);
        assert_eq!(find(0, 2, "تقرير"), ["التقرير السنوي.pdf", "تقرير المبيعات 2026.xlsx"]);
        assert_eq!(find(0, 5, "تقرير"), ["تقرير قديم.docx"]);
        assert_eq!(find(0, 5, "notes"), ["notes.txt"]);
        // A typed filter narrows the chosen one; ranges that don't meet find nothing.
        assert_eq!(find(2, 0, "تقرير size:>1mb"), ["التقرير السنوي.pdf"]);
        assert!(find(1, 0, "تقرير size:>1gb").is_empty());
        // A filter alone does not list the whole disk.
        assert!(find(3, 1, "").is_empty());
        // Every choice is understood by the parser.
        for (token, ..) in &SIZES[1..] {
            assert!(Query::parse(token).size.is_some(), "{token}");
        }
        for (token, ..) in &DATES[1..] {
            assert!(Query::parse(token).mtime.is_some(), "{token}");
        }
        assert_eq!(labels(SIZES, Lang::En)[3], "Over 10 MB");
        assert_eq!(labels(DATES, Lang::Ar)[1], "اليوم");
    }

    #[test]
    fn sorting_and_resorting() {
        let p = Params { text: "تقرير".into(), sort: Sort::Size, desc: true, ..Params::default() };
        let (mut shown, msg) = search(index(), p, Lang::Ar);
        // The folder holds the biggest file, so by total size it comes first.
        assert_eq!(names(&msg)[..3], ["تقرير الصيانة".to_string(), "تقرير قديم.docx".to_string(), "التقرير السنوي.pdf".to_string()]);
        let p = Params { text: "تقرير".into(), sort: Sort::Name, ..Params::default() };
        let by_name = names(&search(index(), p, Lang::Ar).1);
        let mut sorted = by_name.clone();
        sorted.sort_by_key(|n| crate::normalize::normalize(n));
        assert_eq!(by_name, sorted);
        // A renamed row stays with its file when the order changes.
        let old = shown.path(1).unwrap().to_string();
        shown.changed.insert(1, Some(format!("{old}.bak")));
        shown.resort(Sort::Date, false);
        let id = shown.all_paths().iter().position(|p| *p == format!("{old}.bak")).unwrap();
        assert!(name_of(&shown.page(id, 1)[0]).ends_with(".bak"));
        let dates: Vec<i64> = shown.page(0, 10).iter().map(|r| r.mtime).collect();
        assert!(dates.windows(2).all(|w| w[0] <= w[1]), "{dates:?}");
    }

    fn hit(index: &Index, path: &str, snippet: &str, count: u32) -> ContentHit {
        ContentHit { index: index.find(path).unwrap() as u32, snippet: snippet.into(), count, lines: vec![(3, snippet.into()), (9, format!("ثم {snippet}"))] }
    }

    #[test]
    fn documents_fill_as_found_and_mark_snippets() {
        let idx = index();
        let p = Params { seq: 11, text: "عقد إيجار".into(), content: true, ..Params::default() };
        let q = content_query(&p).unwrap();
        let mut shown = Shown::documents(idx.clone(), q, p.clone());
        let Outbound::Results { total, running, content, .. } = shown.results(0, None, Vec::new(), Lang::Ar) else { panic!() };
        assert_eq!((total, running, content), (0, true, true));
        let notes = "C:\\Users\\Sara\\Desktop\\notes.txt";
        let pdf = "C:\\Users\\Sara\\Documents\\العمل\\التقرير السنوي.pdf";
        shown.absorb(vec![hit(&idx, notes, "…نص عقد الإيجار الموقّع…", 4)]);
        // The same document twice is listed once.
        shown.absorb(vec![hit(&idx, pdf, "بند عقد الإيجار", 1), hit(&idx, notes, "x", 1)]);
        assert_eq!(shown.hits.len(), 2);
        let rows = shown.page(0, 10);
        assert_eq!(rows[0].matches, 4);
        let snippet = rows[0].snippet.as_ref().unwrap();
        assert!(snippet.iter().any(|(t, m)| *m && t.contains("عقد")), "{snippet:?}");
        assert!(snippet.iter().any(|(t, m)| *m && t.contains("إيجار")), "{snippet:?}");
        let Some(Outbound::Matches { count, lines, seq, id, .. }) = shown.matches(0) else { panic!() };
        assert_eq!((count, seq, id, lines.len(), lines[1].0), (4, 11, 0, 2, 9));
        assert!(shown.matches(5).is_none());
        // At the end, best match is path order, as in the classic window.
        shown.done = true;
        shown.resort(Sort::Best, false);
        assert_eq!(shown.all_paths(), [notes, pdf]);
        let Outbound::Results { running, total, .. } = shown.results(0, None, Vec::new(), Lang::Ar) else { panic!() };
        assert_eq!((running, total), (false, 2));
        // Nothing to look for in a text: no search inside documents.
        assert!(content_query(&Params { text: "size:>1mb".into(), content: true, ..Params::default() }).is_none());
        assert!(content_query(&Params { text: "  ".into(), content: true, ..Params::default() }).is_none());
    }

    #[test]
    fn renamed_and_removed_rows() {
        let p = Params { seq: 1, text: "تقرير".into(), sort: Sort::Path, ..Params::default() };
        let (mut shown, _) = search(index(), p, Lang::Ar);
        let n = shown.hits.len();
        let first = shown.path(0).unwrap().to_string();
        let new = rename_target(&first, "جديد.pdf").unwrap();
        shown.changed.insert(0, Some(new.clone()));
        shown.changed.insert(1, None);
        assert_eq!(shown.path(0), Some(new.as_str()));
        assert_eq!(shown.path(1), None);
        let rows = shown.page(0, 2);
        assert_eq!(name_of(&rows[0]), "جديد.pdf");
        assert!(rows[1].gone && !rows[0].gone);
        assert_eq!(shown.all_paths().len(), n - 1);
    }

    #[test]
    fn rename_rules() {
        let old = "C:\\Users\\Sara\\Documents\\عقد.docx";
        assert_eq!(rename_target(old, " عقد 2026.docx "), Ok("C:\\Users\\Sara\\Documents\\عقد 2026.docx".into()));
        assert_eq!(rename_target(old, "عقد.docx"), Err(BadName::Unchanged));
        assert_eq!(rename_target(old, "  "), Err(BadName::Unchanged));
        for bad in ["a\\b", "..\\x", "a/b", "C:x", "a*", "a?", "\"q\"", "<a>", "a|b", "a\u{0}b", "a\nb"] {
            assert_eq!(rename_target(old, bad), Err(BadName::Characters), "{bad:?}");
        }
        for bad in ["CON", "nul.txt", "com1.doc", "LPT9", "name.", "aux .txt", "CONIN$", "conout$.log", "COM¹", "lpt³.txt", "Com² .txt", "COM0"] {
            assert_eq!(rename_target(old, bad), Err(BadName::Reserved), "{bad:?}");
        }
        for ok in ["COM10.txt", "console.txt", "COM⁴.txt", "COM", "LPT.txt", "CONIN.txt", "conin$x.txt", "my CON.txt"] {
            assert!(rename_target(old, ok).is_ok(), "{ok:?}");
        }
        assert!(BadName::Characters.message(Lang::En).contains('|'));
    }

    #[test]
    fn history_lists_recent_and_saved() {
        let mut h = History::default();
        for i in 0..20 {
            h.add_recent(&format!("بحث {i}"));
        }
        h.add_saved(crate::history::SavedSearch { name: "عقود".into(), query: "عقد".into(), kind: 1, scope: "D:\\".into(), content: true });
        let Outbound::History { recent, saved } = history(&h) else { panic!() };
        assert_eq!(recent.len(), RECENT_SEARCHES);
        assert_eq!(saved, [SavedOut { id: 0, name: "عقود".into(), content: true, scoped: true }]);
        // The saved search's folder is not sent to the page.
        let json = serde_json::to_string(&history(&h)).unwrap();
        assert!(!json.contains("D:\\\\"), "{json}");
    }

    #[test]
    fn content_progress_texts() {
        let s = |found, done, indexing| ContentState { checked: 1200, total: 4000, found, cached: 0, done, indexing, ocr_arabic_missing: false };
        assert_eq!(content_text(&s(3, false, Indexing::Idle), Lang::En), "Searching inside documents… checked 1,200 of 4,000 — found 3");
        assert!(content_text(&s(0, false, Indexing::Names { found: 9 }), Lang::Ar).contains("قد تكون النتائج ناقصة"));
        assert_eq!(content_text(&s(2, true, Indexing::Idle), Lang::Ar), "انتهى البحث داخل 4,000 مستند: وجدت 2 ملف.");
        let done = ContentState { cached: 50, ocr_arabic_missing: true, ..s(0, true, Indexing::Names { found: 1 }) };
        let t = content_text(&done, Lang::En);
        assert!(t.starts_with("Searched 4,000 documents: found 0 files. (50 from the index) Results may be incomplete"), "{t}");
        assert!(t.contains("Arabic"), "{t}");
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
        let settings = Outbound::Settings(SettingsOut {
            lang: "ar",
            next_lang: "en",
            digits: false,
            autostart: true,
            content_index: true,
            auto_index: true,
            hide_system: true,
            hotkey_quick: true,
            floating: false,
            tray_icon: true,
            version: "1.0.0",
        });
        let json = serde_json::to_string(&settings).unwrap();
        assert!(json.starts_with(r#"{"type":"settings","lang":"ar","next_lang":"en""#), "{json}");
        let json = serde_json::to_string(&Outbound::Renamed { list: List::Recent, seq: 0, ver: 3, id: 1, name: "a".into() }).unwrap();
        assert_eq!(json, r#"{"type":"renamed","list":"recent","seq":0,"ver":3,"id":1,"name":"a"}"#);
    }

    /// A name search over the test index, numbered by `lists` as the program does.
    fn listed(lists: &mut Lists, seq: u64, sort: Sort) -> (Shown, Outbound) {
        let p = Params { seq, ver: lists.next_ver(), text: "تقرير".into(), sort, ..Params::default() };
        search(index(), p, Lang::Ar)
    }

    /// [`Lists::finished`], with the waiting search by its seq.
    fn fin(lists: &mut Lists, shown: Shown) -> (bool, Option<u64>) {
        let (send, next) = lists.finished(shown);
        (send, next.map(|p| p.seq))
    }

    fn ver_of(msg: &Outbound) -> u64 {
        match msg {
            Outbound::Results { ver, .. } | Outbound::Rows { ver, .. } | Outbound::Matches { ver, .. } | Outbound::Recent { ver, .. } => *ver,
            _ => panic!("{msg:?}"),
        }
    }

    #[test]
    fn rows_are_named_with_the_version_of_their_list() {
        let mut lists = Lists::default();
        let (shown, msg) = listed(&mut lists, 4, Sort::Name);
        let v1 = ver_of(&msg);
        assert!(lists.finished(shown).0);
        let first = lists.path(List::Results, 4, v1, 0).unwrap().to_string();
        assert_eq!(ver_of(&lists.shown.as_ref().unwrap().rows(0, 5)), v1);

        // The same search run again in the background (the index changed), in
        // another order: same seq, same row numbers, other files at them.
        let (again, msg) = listed(&mut lists, 4, Sort::Date);
        let v2 = ver_of(&msg);
        assert_ne!(v1, v2);
        assert_eq!(fin(&mut lists, again), (true, None));
        assert_ne!(lists.path(List::Results, 4, v2, 0).unwrap(), first, "the test needs another file at row 0");
        // A request about the list the page had before is refused, not applied to row 0 now.
        assert_eq!(lists.path(List::Results, 4, v1, 0), Err(Miss::Stale));
        assert!(Miss::Stale.message(Lang::En).unwrap().contains("Try again"));
        assert_eq!(Miss::Gone.message(Lang::Ar), None);

        // Another order of the same list (Sort, or a search inside documents
        // put in order when it ends) is a new version too.
        let s = lists.resort(4, Sort::Size, true).unwrap();
        let (v3, top) = (s.ver, s.path(0).unwrap().to_string());
        assert!(v3 > v2);
        let Outbound::Results { ver, .. } = s.results(0, None, Vec::new(), Lang::Ar) else { panic!() };
        assert_eq!(ver, v3);
        assert_eq!(lists.path(List::Results, 4, v2, 0), Err(Miss::Stale));
        assert_eq!(lists.path(List::Results, 4, v3, 0), Ok(top.as_str()));
        // Another search, or no list at all.
        assert_eq!(lists.path(List::Results, 5, v3, 0), Err(Miss::Stale));
        assert!(lists.resort(9, Sort::Name, false).is_none());
        assert!(lists.shown_at_mut(4, v2).is_err() && lists.shown_at_mut(4, v3).is_ok());
        assert_eq!(Lists::default().path(List::Results, 0, 0, 0), Err(Miss::Stale));
        // A file sent to the Recycle Bin: nothing to do, nothing to say.
        lists.shown_at_mut(4, v3).unwrap().changed.insert(0, None);
        assert_eq!(lists.path(List::Results, 4, v3, 0), Err(Miss::Gone));
        assert_eq!(lists.path(List::Results, 4, v3, 999), Err(Miss::Gone));
    }

    #[test]
    fn a_dialog_acts_on_the_list_it_was_opened_for() {
        // The rename dialog is opened on row 0 of version v1; while it is
        // open the list is listed again. Its OK names v1, which is refused.
        let mut lists = Lists::default();
        let (shown, msg) = listed(&mut lists, 1, Sort::Name);
        let opened = ver_of(&msg);
        lists.finished(shown);
        let (again, _) = listed(&mut lists, 1, Sort::Date);
        lists.finished(again);
        let ok = req(&format!(r#""op":"rename","list":"results","seq":1,"ver":{opened},"id":0,"name":"جديد.pdf""#)).unwrap();
        let Request::Rename { list, seq, ver, id, .. } = ok else { panic!() };
        assert_eq!(lists.path(list, seq, ver, id), Err(Miss::Stale));
        // Every request about rows must name a version.
        for body in [
            r#""op":"rename","list":"results","seq":1,"id":0,"name":"x""#,
            r#""op":"recycle","list":"results","seq":1,"id":0"#,
            r#""op":"open","list":"recent","seq":0,"id":0"#,
            r#""op":"copy_all","seq":1"#,
            r#""op":"more","seq":1,"from":60"#,
            r#""op":"matches","seq":1,"id":0"#,
            r#""op":"scope","list":"results","seq":1,"id":0"#,
        ] {
            assert_eq!(req(body), Err(Rejected::Malformed), "{body}");
        }
        assert_eq!(req(r#""op":"more","seq":1,"ver":2,"from":60"#), Ok(Request::More { seq: 1, ver: 2, from: 60 }));
    }

    #[test]
    fn recent_files_have_versions_too() {
        let mut lists = Lists::default();
        // Nothing listed yet: version 0 names nothing.
        assert_eq!(lists.path(List::Recent, 0, 0, 0), Err(Miss::Stale));
        let v1 = lists.set_recent(vec!["C:\\a\\one.txt".into(), "C:\\a\\two.txt".into()]);
        assert_eq!(lists.path(List::Recent, 0, v1, 1), Ok("C:\\a\\two.txt"));
        assert_eq!(lists.path(List::Recent, 0, 0, 1), Err(Miss::Stale));
        // A file opened moves to the top: the list is sent again, and row 1 is another file.
        let v2 = lists.set_recent(vec!["C:\\a\\two.txt".into(), "C:\\a\\one.txt".into()]);
        assert_ne!(v1, v2);
        assert_eq!(lists.path(List::Recent, 0, v1, 1), Err(Miss::Stale));
        assert_eq!(lists.path(List::Recent, 0, v2, 1), Ok("C:\\a\\one.txt"));
        assert_eq!(lists.path(List::Recent, 0, v2, 2), Err(Miss::Gone));
        // Results and recent files never share a version.
        let (shown, _) = listed(&mut lists, 1, Sort::Best);
        assert!(shown.ver > v2);
        let json = serde_json::to_string(&Outbound::Recent { ver: v2, rows: Vec::new() }).unwrap();
        assert_eq!(json, format!(r#"{{"type":"recent","ver":{v2},"rows":[]}}"#));
    }

    #[test]
    fn hello_forgets_the_lists_of_the_page_before() {
        let mut lists = Lists::default();
        let (shown, _) = listed(&mut lists, 37, Sort::Best);
        lists.finished(shown);
        lists.set_recent(vec!["C:\\a.txt".into()]);
        // A search of the old page still running, and one waiting.
        let (late, _) = listed(&mut lists, 38, Sort::Best);
        lists.pending = Some(Params { seq: 39, ver: lists.next_ver(), ..Params::default() });
        let (old_shown, old_recent) = (lists.shown.as_ref().unwrap().ver, lists.recent.ver);

        lists.reset();
        assert!(lists.shown.is_none() && lists.pending.is_none() && lists.recent.paths.is_empty());
        assert_eq!(lists.path(List::Results, 37, old_shown, 0), Err(Miss::Stale));
        assert_eq!(lists.path(List::Recent, 0, old_recent, 0), Err(Miss::Stale));
        // The old page's search ends: dropped, so it can't hide the new page's
        // first search (seq 1, lower than 38).
        assert!(!lists.fresh(late.ver));
        assert_eq!(fin(&mut lists, late), (false, None));
        assert!(lists.shown.is_none());
        let (first, _) = listed(&mut lists, 1, Sort::Best);
        assert!(lists.fresh(first.ver));
        assert!(lists.finished(first).0);
        assert_eq!(lists.shown.as_ref().map(|s| s.seq), Some(1));
    }

    #[test]
    fn a_waiting_search_holds_back_the_answer() {
        let mut lists = Lists::default();
        let (shown, _) = listed(&mut lists, 2, Sort::Best);
        lists.pending = Some(Params { seq: 3, ver: lists.next_ver(), ..Params::default() });
        let (send, next) = lists.finished(shown);
        assert!(!send);
        assert_eq!(next.map(|p| p.seq), Some(3));
        // An older search ending after a newer list is dropped.
        let (old, _) = listed(&mut lists, 1, Sort::Best);
        assert_eq!(fin(&mut lists, old), (false, None));
        assert_eq!(lists.shown.as_ref().map(|s| s.seq), Some(2));
    }

    #[test]
    fn only_results_add_their_search_to_the_history() {
        let mut lists = Lists::default();
        assert_eq!(lists.used_search(List::Results), None);
        let (shown, _) = listed(&mut lists, 1, Sort::Best);
        lists.finished(shown);
        lists.set_recent(vec!["C:\\a.txt".into()]);
        assert_eq!(lists.used_search(List::Results), Some("تقرير"));
        // A recent file was listed by no search: opening it adds none.
        assert_eq!(lists.used_search(List::Recent), None);
        lists.shown.as_mut().unwrap().params.text = "  ".into();
        assert_eq!(lists.used_search(List::Results), None);
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
