//! Query parsing, filename search over the index, and content search inside documents.

// Only the Windows UI and the tests use this module.

use rayon::prelude::*;
use regex::{Regex, RegexBuilder};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};

use crate::content_cache::{self, ContentCache};
use crate::extract;
use crate::index::Entry;
use crate::normalize::{normalize, normalize_with_map};

mod members;
#[cfg(test)]
mod robustness;

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

    pub(crate) fn matches(&self, hay: &str) -> bool {
        self.alts.iter().any(|a| hay.contains(a.as_str()))
    }

    /// True when an alternative occurs in `hay` as a whole word.
    fn matches_word(&self, hay: &str) -> bool {
        self.alts.iter().any(|a| word_matches(hay, a).next().is_some())
    }

    /// Every occurrence `(start, end)` of every alternative in `hay`, sorted,
    /// with overlapping ones (an article form and its bare word) merged.
    fn spans(&self, hay: &str, whole_word: bool) -> Vec<(usize, usize)> {
        let mut spans: Vec<(usize, usize)> = Vec::new();
        for a in self.alts.iter().filter(|a| !a.is_empty()) {
            if whole_word {
                spans.extend(word_matches(hay, a).map(|s| (s, s + a.len())));
            } else {
                spans.extend(hay.match_indices(a.as_str()).map(|(s, _)| (s, s + a.len())));
            }
        }
        merge_spans(spans)
    }
}

/// Sorts `spans` and merges the ones that overlap or touch.
fn merge_spans(mut spans: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    spans.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(spans.len());
    for (s, e) in spans {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    merged
}

/// Letters and digits make up words; anything else (spaces, punctuation,
/// `_`, `-`, `.`) separates them. Unicode-aware, so Arabic works.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
}

/// Start offsets of the occurrences of `w` in `hay` that are whole words.
fn word_matches<'a>(hay: &'a str, w: &'a str) -> impl Iterator<Item = usize> + 'a {
    hay.match_indices(w).map(|(s, _)| s).filter(move |&s| {
        !w.is_empty()
            && !hay[..s].chars().next_back().is_some_and(is_word_char)
            && !hay[s + w.len()..].chars().next().is_some_and(is_word_char)
    })
}

/// The words of `hay` as (byte offset, word), split on non-word characters.
pub(crate) fn split_words(hay: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut rest = hay.char_indices().peekable();
    std::iter::from_fn(move || {
        while rest.next_if(|&(_, c)| !is_word_char(c)).is_some() {}
        let (start, _) = *rest.peek()?;
        let mut end = start;
        while let Some((i, c)) = rest.next_if(|&(_, c)| is_word_char(c)) {
            end = i + c.len_utf8();
        }
        Some((start, &hay[start..end]))
    })
}

/// A `regex:` pattern, matched case-insensitively against the original
/// (un-normalised) file name or document text.
#[derive(Debug, Clone)]
pub struct Pattern {
    /// The pattern as typed.
    pub source: String,
    /// `None` when the pattern is invalid: it then matches nothing.
    re: Option<Regex>,
}

impl PartialEq for Pattern {
    fn eq(&self, other: &Pattern) -> bool {
        self.source == other.source
    }
}

impl Pattern {
    /// Compiles `source`, with limits so a hostile pattern can't eat memory.
    fn new(source: &str) -> Pattern {
        let re = RegexBuilder::new(source)
            .case_insensitive(true)
            .multi_line(true)
            .size_limit(1 << 20)
            .dfa_size_limit(4 << 20)
            .build()
            .ok();
        Pattern { source: source.to_string(), re }
    }

    fn is_valid(&self) -> bool {
        self.re.is_some()
    }

    fn is_match(&self, hay: &str) -> bool {
        self.re.as_ref().is_some_and(|r| r.is_match(hay))
    }

    /// Non-empty matches as `(start, end)` byte ranges of `hay`.
    fn ranges<'a>(&'a self, hay: &'a str) -> impl Iterator<Item = (usize, usize)> + 'a {
        self.re.iter().flat_map(move |r| r.find_iter(hay)).filter(|m| !m.is_empty()).map(|m| (m.start(), m.end()))
    }
}

/// A proximity search, `"a b"~5`: every word occurs within `dist` words of
/// the others, in any order.
#[derive(Debug, Clone, PartialEq)]
pub struct Near {
    /// One term per word; each word may carry its article-less form.
    pub words: Vec<Term>,
    pub dist: u32,
}

impl Near {
    /// For each word of `hay` that contains one of the terms: (word number,
    /// term number, byte offset of the match), in text order.
    fn hits(&self, hay: &str) -> Vec<(u32, usize, usize)> {
        let mut hits = Vec::new();
        for (n, (off, word)) in split_words(hay).enumerate() {
            for (t, term) in self.words.iter().enumerate() {
                if let Some(p) = term.find_in(word) {
                    hits.push((n as u32, t, off + p));
                }
            }
        }
        hits
    }

    /// Byte offset in `hay` of the first window holding every word within
    /// `dist` words, if there is one.
    fn find(&self, hay: &str) -> Option<usize> {
        let hits = self.hits(hay);
        let mut counts = vec![0u32; self.words.len()];
        let (mut covered, mut left) = (0, 0);
        for &(n, t, _) in &hits {
            counts[t] += 1;
            covered += usize::from(counts[t] == 1);
            while covered == self.words.len() {
                let (first, lt, off) = hits[left];
                if n - first <= self.dist {
                    return Some(off);
                }
                counts[lt] -= 1;
                covered -= usize::from(counts[lt] == 0);
                left += 1;
            }
        }
        None
    }

    /// Byte offsets of every word occurrence that has each of the other words
    /// within `dist` words of it (exact for two words, which is the usual case).
    fn all(&self, hay: &str) -> Vec<usize> {
        let hits = self.hits(hay);
        let mut by_term: Vec<Vec<u32>> = vec![Vec::new(); self.words.len()];
        for &(n, t, _) in &hits {
            by_term[t].push(n);
        }
        let close = |list: &[u32], n: u32| {
            let i = list.partition_point(|&m| m + self.dist < n);
            list.get(i).is_some_and(|&m| m <= n + self.dist)
        };
        hits.iter()
            .filter(|&&(n, t, _)| by_term.iter().enumerate().all(|(o, list)| o == t || close(list, n)))
            .map(|&(_, _, off)| off)
            .collect()
    }
}

/// A parsed search query.
///
/// Words are AND-ed together, `"quoted text"` is matched as one phrase,
/// `a|b` (or `a | b`) matches either, `!word` or `-word` excludes, `ext:pdf`,
/// `*.pdf` or `.pdf` restricts the file type and `folder:` / `مجلد:` keeps
/// only folders. A leading Arabic article (`ال`, `وال`, `لل`…) is optional.
///
/// Filters: `size:>10mb` / `حجم:` (size), `dm:today` / `تاريخ:` (modified
/// date), `regex:` / `re:` (regular expression), `ww:` / `كلمة:` (whole word)
/// and `"a b"~5` (words near each other).
#[derive(Debug, Default, PartialEq)]
pub struct Query {
    pub terms: Vec<Term>,
    pub excludes: Vec<String>,
    pub exts: Vec<String>,
    pub dirs_only: bool,
    /// Inclusive size range in bytes from `size:`. Folders never pass it.
    pub size: Option<(u64, u64)>,
    /// Modification-time range `[from, to)` in Unix seconds from `dm:`.
    pub mtime: Option<(i64, i64)>,
    /// `regex:` patterns; all must match.
    pub regexes: Vec<Pattern>,
    /// `ww:` terms, matched as whole words.
    pub words: Vec<Term>,
    /// `"a b"~N` proximity searches.
    pub near: Vec<Near>,
    /// `franco:` / `فرانكو:`: Latin words also match their Arabic
    /// transliterations (Arabizi, `3abd` → عبد).
    pub franco: bool,
    /// `fuzzy:` / `تقريب:`: words may match with small typos.
    pub fuzzy: bool,
    /// `root:` / `جذر:`: Arabic words match words with the same 3-letter root.
    pub root: bool,
    /// Problems found while parsing (in Arabic), for the UI to show.
    pub warnings: Vec<String>,
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

/// Prefixes of the tokens that switch on Arabizi, typo-tolerant and root
/// matching for the whole query (ASCII case ignored).
pub const FRANCO_PREFIXES: [&str; 2] = ["franco:", "فرانكو:"];
pub const FUZZY_PREFIXES: [&str; 2] = ["fuzzy:", "تقريب:"];
pub const ROOT_PREFIXES: [&str; 2] = ["root:", "جذر:"];

/// Arabic prefixes stripped to also match the bare word, longest first.
pub(crate) const ARTICLES: [&str; 6] = ["وال", "بال", "كال", "فال", "لل", "ال"];

/// Prefixes of the filters that take a value (ASCII case ignored).
const SIZE_PREFIXES: [&str; 2] = ["size:", "حجم:"];
const DATE_PREFIXES: [&str; 3] = ["dm:", "datemodified:", "تاريخ:"];
const REGEX_PREFIXES: [&str; 2] = ["regex:", "re:"];
const WORD_PREFIXES: [&str; 3] = ["ww:", "wholeword:", "كلمة:"];

/// Largest `~N` accepted for a proximity search.
const MAX_NEAR: u32 = 10_000;

/// One piece of the input.
struct Token {
    text: String,
    quoted: bool,
    /// `N` of a `"quoted"~N` proximity suffix.
    near: Option<u32>,
}

/// The value of an ASCII, Arabic-Indic or Persian digit.
fn digit_value(c: char) -> Option<u32> {
    match c {
        '0'..='9' => Some(c as u32 - '0' as u32),
        '\u{0660}'..='\u{0669}' => Some(c as u32 - 0x0660),
        '\u{06F0}'..='\u{06F9}' => Some(c as u32 - 0x06F0),
        _ => None,
    }
}

/// Parses a `~N` suffix at the start of `rest` (followed by a space or the
/// end), returning N and what follows it.
fn near_suffix(rest: &str) -> Option<(u32, &str)> {
    let digits = rest.strip_prefix('~')?;
    let end = digits.find(|c| digit_value(c).is_none()).unwrap_or(digits.len());
    let after = &digits[end..];
    if end == 0 || !after.is_empty() && !after.starts_with(char::is_whitespace) {
        return None;
    }
    let n = digits[..end].chars().filter_map(digit_value).fold(0u32, |n, d| n.saturating_mul(10).saturating_add(d));
    Some((n.clamp(1, MAX_NEAR), after))
}

/// Length of the value-filter prefix `tok` starts with, if any.
fn value_prefix(tok: &str) -> Option<usize> {
    [&SIZE_PREFIXES[..], &DATE_PREFIXES, &REGEX_PREFIXES, &WORD_PREFIXES]
        .into_iter()
        .find_map(|list| prefix_len(tok, list))
}

/// Splits the input into tokens. A filter value may be quoted to hold
/// spaces (`regex:"a b"`), and a quoted phrase may end in `~N`.
fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut rest = input.trim();
    while !rest.is_empty() {
        if let Some(stripped) = rest.strip_prefix('"') {
            let end = stripped.find('"').unwrap_or(stripped.len());
            let text = stripped[..end].to_string();
            rest = stripped.get(end + 1..).unwrap_or("");
            let near = near_suffix(rest).map(|(n, after)| {
                rest = after;
                n
            });
            tokens.push(Token { text, quoted: true, near });
            rest = rest.trim_start();
        } else {
            let mut end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            if let Some(len) = value_prefix(rest)
                && let Some(value) = rest[len..].strip_prefix('"')
            {
                // Up to the closing quote, then to the next space.
                let close = value.find('"').map_or(rest.len(), |i| len + 2 + i);
                end = close + rest[close..].find(char::is_whitespace).unwrap_or(rest.len() - close);
            }
            tokens.push(Token { text: rest[..end].to_string(), quoted: false, near: None });
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

/// Length of the prefix from `list` that `tok` starts with, if any.
fn prefix_len(tok: &str, list: &[&str]) -> Option<usize> {
    list.iter().find(|p| tok.get(..p.len()).is_some_and(|h| h.eq_ignore_ascii_case(p))).map(|p| p.len())
}

/// Length of the folder prefix `tok` starts with, if any.
fn folder_prefix(tok: &str) -> Option<usize> {
    prefix_len(tok, &FOLDER_PREFIXES)
}

/// A filter value with one pair of surrounding quotes removed.
fn unquote(v: &str) -> &str {
    let v = v.strip_prefix('"').unwrap_or(v);
    v.strip_suffix('"').unwrap_or(v)
}

/// Splits a comparison off the front: `>=`, `<=`, `>`, `<` or `=`.
fn split_op(v: &str) -> (&str, &str) {
    for op in [">=", "<=", ">", "<", "="] {
        if let Some(rest) = v.strip_prefix(op) {
            return (op, rest);
        }
    }
    ("", v)
}

/// The value with Arabic digits, case and letter forms folded and `٫` as `.`.
fn fold_value(v: &str) -> String {
    normalize(unquote(v)).replace('\u{066B}', ".")
}

// ------------------------------------------------------------------ sizes

const KB: u64 = 1024;
const MB: u64 = 1024 * KB;
const GB: u64 = 1024 * MB;

/// Named sizes as in Everything: (names, inclusive range).
const NAMED_SIZES: &[(&[&str], (u64, u64))] = &[
    (&["empty", "فارغ"], (0, 0)),
    (&["tiny", "صغير_جدا"], (1, 10 * KB)),
    (&["small", "صغير"], (10 * KB + 1, 100 * KB)),
    (&["medium", "متوسط"], (100 * KB + 1, MB)),
    (&["large", "كبير"], (MB + 1, 16 * MB)),
    (&["huge", "كبير_جدا"], (16 * MB + 1, 128 * MB)),
    (&["gigantic", "ضخم"], (128 * MB + 1, u64::MAX)),
];

/// A size such as `10`, `1.5mb` or `10م`, in bytes.
fn size_value(v: &str) -> Option<u64> {
    let split = v.find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(v.len());
    let (num, unit) = (&v[..split], v[split..].trim());
    let unit = match unit {
        "" | "b" | "ب" => 1,
        "k" | "kb" | "ك" => KB,
        "m" | "mb" | "م" => MB,
        "g" | "gb" | "ج" => GB,
        "t" | "tb" | "ت" => 1024 * GB,
        _ => return None,
    };
    let n: f64 = num.parse().ok().filter(|n: &f64| n.is_finite())?;
    let bytes = (n * unit as f64).round();
    (bytes <= u64::MAX as f64).then_some(bytes as u64)
}

/// Parses a `size:` value into an inclusive byte range.
fn parse_size(v: &str) -> Option<(u64, u64)> {
    let v = fold_value(v).replace(' ', "_");
    if let Some(&(_, range)) = NAMED_SIZES.iter().find(|(names, _)| names.contains(&v.as_str())) {
        return Some(range);
    }
    if let Some((a, b)) = v.split_once("..") {
        let lo = if a.is_empty() { 0 } else { size_value(a)? };
        let hi = if b.is_empty() { u64::MAX } else { size_value(b)? };
        return (lo <= hi && !(a.is_empty() && b.is_empty())).then_some((lo, hi));
    }
    let (op, rest) = split_op(&v);
    let n = size_value(rest)?;
    match op {
        ">=" => Some((n, u64::MAX)),
        ">" => Some((n.checked_add(1)?, u64::MAX)),
        "<=" => Some((0, n)),
        "<" => Some((0, n.checked_sub(1)?)),
        _ => Some((n, n)),
    }
}

// ------------------------------------------------------------------ dates

const DAY: i64 = 86_400;

/// First day of the week for `dm:thisweek`: 0 is Sunday, 6 is Saturday
/// (the usual start in Arabic calendars).
const WEEK_START: i64 = 6;

/// Local offset from UTC in seconds for calendar words such as `dm:today`.
static UTC_OFFSET: AtomicI32 = AtomicI32::new(0);

/// Sets the local time zone's offset from UTC (east positive), used by
/// [`Query::parse`] to work out where "today", "this week"… start.
pub fn set_utc_offset_secs(secs: i32) {
    UTC_OFFSET.store(secs, Ordering::Relaxed);
}

/// Reads the Windows time zone (with daylight saving) into
/// [`set_utc_offset_secs`]. Call once at start-up.
#[cfg(windows)]
pub fn init_local_utc_offset() {
    use winapi::um::timezoneapi::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    let mut tzi: TIME_ZONE_INFORMATION = unsafe { std::mem::zeroed() };
    let bias = match unsafe { GetTimeZoneInformation(&mut tzi) } {
        1 => tzi.Bias + tzi.StandardBias, // TIME_ZONE_ID_STANDARD
        2 => tzi.Bias + tzi.DaylightBias, // TIME_ZONE_ID_DAYLIGHT
        0 => tzi.Bias,                    // no daylight saving
        _ => return,
    };
    // Bias is minutes to add to local time to get UTC.
    set_utc_offset_secs(-bias * 60);
}

/// The offset set by [`set_utc_offset_secs`] (0 until it is called).
pub fn utc_offset_secs() -> i32 {
    UTC_OFFSET.load(Ordering::Relaxed)
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Days since 1970-01-01 of a proleptic Gregorian date (H. Hinnant's algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let (m, d) = (m as i64, d as i64);
    let doy = (153 * if m > 2 { m - 3 } else { m + 9 } + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The (year, month, day) of a day number from [`days_from_civil`].
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        2 if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Calendar words: (names, what they mean). Arabic names are compared after
/// normalisation, so hamza and ta marbuta forms don't matter.
#[derive(Clone, Copy)]
enum Span {
    Day(i64),
    Week(i64),
    Month(i64),
    Year(i64),
}

const DATE_WORDS: &[(&[&str], Span)] = &[
    (&["today", "اليوم"], Span::Day(0)),
    (&["yesterday", "أمس", "البارحة"], Span::Day(-1)),
    (&["thisweek", "هذا_الأسبوع"], Span::Week(0)),
    (&["lastweek", "الأسبوع_الماضي"], Span::Week(-1)),
    (&["thismonth", "هذا_الشهر"], Span::Month(0)),
    (&["lastmonth", "الشهر_الماضي"], Span::Month(-1)),
    (&["thisyear", "هذه_السنة", "هذا_العام"], Span::Year(0)),
    (&["lastyear", "السنة_الماضية", "العام_الماضي"], Span::Year(-1)),
];

/// Times are worked out in local time; `off` is the offset from UTC.
struct Clock {
    now: i64,
    off: i64,
}

impl Clock {
    /// Unix time at which local day number `day` starts.
    fn day_start(&self, day: i64) -> i64 {
        day * DAY - self.off
    }

    fn today(&self) -> i64 {
        (self.now + self.off).div_euclid(DAY)
    }

    /// Unix time at the start of the month `months` after (y, m).
    fn month_start(&self, y: i64, m: u32, months: i64) -> i64 {
        let total = y * 12 + i64::from(m) - 1 + months;
        self.day_start(days_from_civil(total.div_euclid(12), (total.rem_euclid(12) + 1) as u32, 1))
    }

    /// The same local time `months` months ago, the day clamped to the month.
    fn months_ago(&self, months: i64) -> i64 {
        let local = self.now + self.off;
        let (y, m, d) = civil_from_days(local.div_euclid(DAY));
        let total = y * 12 + i64::from(m) - 1 - months;
        let (y, m) = (total.div_euclid(12), (total.rem_euclid(12) + 1) as u32);
        self.day_start(days_from_civil(y, m, d.min(days_in_month(y, m)))) + local.rem_euclid(DAY)
    }

    fn span(&self, span: Span) -> (i64, i64) {
        let today = self.today();
        let (y, m, _) = civil_from_days(today);
        match span {
            Span::Day(k) => (self.day_start(today + k), self.day_start(today + k + 1)),
            Span::Week(k) => {
                // 1970-01-01 was a Thursday (4).
                let start = today - (today + 4 - WEEK_START).rem_euclid(7) + 7 * k;
                (self.day_start(start), self.day_start(start + 7))
            }
            Span::Month(k) => (self.month_start(y, m, k), self.month_start(y, m, k + 1)),
            Span::Year(k) => (self.month_start(y + k, 1, 0), self.month_start(y + k + 1, 1, 0)),
        }
    }

    /// One date as a `[start, end)` range: a calendar word, `2024`,
    /// `2024-05`, `2024-05-12`, or a recent period `7d`, `3w`, `2m`, `1y`.
    fn point(&self, v: &str) -> Option<(i64, i64)> {
        let key: String = v.chars().filter(|c| !matches!(c, '_' | '-' | ' ')).collect();
        let is_word = |w: &&str| normalize(w).replace('_', "") == key;
        if let Some(&(_, span)) = DATE_WORDS.iter().find(|(names, _)| names.iter().any(is_word)) {
            return Some(self.span(span));
        }
        if let Some(num) = v.strip_suffix(['d', 'w', 'm', 'y'])
            && let Ok(n) = num.parse::<u32>()
        {
            let n = i64::from(n.min(100_000));
            let from = match v.as_bytes()[num.len()] {
                b'd' => self.now - n * DAY,
                b'w' => self.now - n * 7 * DAY,
                b'm' => self.months_ago(n),
                _ => self.months_ago(n * 12),
            };
            return Some((from, i64::MAX));
        }
        let parts: Vec<&str> = v.split(['-', '/', '.']).collect();
        let num = |i: usize, digits: std::ops::RangeInclusive<usize>| {
            parts.get(i).filter(|p| digits.contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit()))?.parse::<u32>().ok()
        };
        let y = i64::from(num(0, 4..=4)?);
        match parts.len() {
            1 => Some((self.month_start(y, 1, 0), self.month_start(y + 1, 1, 0))),
            2 => {
                let m = num(1, 1..=2).filter(|m| (1..=12).contains(m))?;
                Some((self.month_start(y, m, 0), self.month_start(y, m, 1)))
            }
            3 => {
                let m = num(1, 1..=2).filter(|m| (1..=12).contains(m))?;
                let d = num(2, 1..=2).filter(|&d| d >= 1 && d <= days_in_month(y, m))?;
                let day = days_from_civil(y, m, d);
                Some((self.day_start(day), self.day_start(day + 1)))
            }
            _ => None,
        }
    }

    /// Parses a `dm:` value into a `[from, to)` range of Unix seconds.
    fn parse(&self, v: &str) -> Option<(i64, i64)> {
        let v = fold_value(v);
        let (from, to) = if let Some((a, b)) = v.split_once("..") {
            let from = if a.is_empty() { i64::MIN } else { self.point(a)?.0 };
            let to = if b.is_empty() { i64::MAX } else { self.point(b)?.1 };
            (from, to)
        } else {
            let (op, rest) = split_op(&v);
            let (start, end) = self.point(rest)?;
            match op {
                ">=" => (start, i64::MAX),
                ">" => (end, i64::MAX),
                "<=" => (i64::MIN, end),
                "<" => (i64::MIN, start),
                _ => (start, end),
            }
        };
        (from < to).then_some((from, to))
    }
}

/// Narrows an optional range to its overlap with `new`.
fn intersect<T: Ord + Copy>(old: Option<(T, T)>, new: (T, T)) -> Option<(T, T)> {
    Some(old.map_or(new, |(a, b)| (a.max(new.0), b.min(new.1))))
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
    /// Parses `input`. Calendar words in `dm:` use the current time and the
    /// offset from [`set_utc_offset_secs`].
    pub fn parse(input: &str) -> Query {
        Query::parse_at(input, now_secs(), utc_offset_secs())
    }

    /// Parses `input` as if the time were `now` (Unix seconds) in a time zone
    /// `utc_offset` seconds east of UTC.
    pub fn parse_at(input: &str, now: i64, utc_offset: i32) -> Query {
        let clock = Clock { now, off: i64::from(utc_offset) };
        let mut q = Query::default();
        // Set by a trailing or standalone `|`: the next term joins the last one.
        let mut join = false;
        for Token { text: tok, quoted, near } in tokenize(input) {
            if let Some(dist) = near {
                let mut words = Vec::new();
                for w in split_words(&normalize(&tok)).map(|(_, w)| w) {
                    let mut alts = Vec::new();
                    push_alt(&mut alts, w.to_string(), false);
                    words.push(Term { alts });
                }
                if words.len() >= 2 {
                    q.near.push(Near { words, dist });
                    join = false;
                    continue;
                }
            }
            let mut pieces: Vec<&str> = vec![&tok];
            if !quoted {
                if q.value_filter(&tok, &clock) {
                    join = false;
                    continue;
                }
                let lower = tok.to_lowercase();
                if let Some(ext) = ext_filter(&lower) {
                    let exts = ext.split([',', ';']).filter(|e| !e.is_empty());
                    q.exts.extend(exts.map(str::to_string));
                    // A filter between two words ends any pending `|`.
                    join = false;
                    continue;
                }
                if let Some(len) = q.mode_prefix(&tok) {
                    if tok.len() == len {
                        join = false;
                        continue;
                    }
                    pieces = vec![&tok[len..]];
                } else if let Some(len) = folder_prefix(&tok) {
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
        if q.franco {
            for t in &mut q.terms {
                let latin: Vec<String> = t.alts.iter().filter(|a| crate::franco::is_arabizi(a)).cloned().collect();
                for c in latin.iter().flat_map(|a| crate::franco::candidates(a)) {
                    push_alt(&mut t.alts, c, false);
                }
            }
        }
        q
    }

    /// Handles a `franco:`, `fuzzy:` or `root:` prefix (and their Arabic
    /// forms) at the start of `tok`: sets the flag and returns its length.
    fn mode_prefix(&mut self, tok: &str) -> Option<usize> {
        if let Some(len) = prefix_len(tok, &FRANCO_PREFIXES) {
            self.franco = true;
            Some(len)
        } else if let Some(len) = prefix_len(tok, &FUZZY_PREFIXES) {
            self.fuzzy = true;
            Some(len)
        } else if let Some(len) = prefix_len(tok, &ROOT_PREFIXES) {
            self.root = true;
            Some(len)
        } else {
            None
        }
    }

    /// Handles a `size:`, `dm:`, `regex:` or `ww:` token. Returns false when
    /// `tok` is not one, or its value can't be understood: then it is searched
    /// for as a plain word and a warning says so.
    fn value_filter(&mut self, tok: &str, clock: &Clock) -> bool {
        if let Some(len) = prefix_len(tok, &SIZE_PREFIXES) {
            let v = &tok[len..];
            if v.is_empty() {
                return true;
            }
            match parse_size(v) {
                Some(range) => self.size = intersect(self.size, range),
                None => {
                    self.warnings.push(trf!("لم أفهم الحجم «{v}»، فبحثت عن «{tok}» كنص عادي.", "Did not understand the size “{v}”, so searched for “{tok}” as plain text."));
                    return false;
                }
            }
        } else if let Some(len) = prefix_len(tok, &DATE_PREFIXES) {
            let v = &tok[len..];
            if v.is_empty() {
                return true;
            }
            match clock.parse(v) {
                Some(range) => self.mtime = intersect(self.mtime, range),
                None => {
                    self.warnings.push(trf!("لم أفهم التاريخ «{v}»، فبحثت عن «{tok}» كنص عادي.", "Did not understand the date “{v}”, so searched for “{tok}” as plain text."));
                    return false;
                }
            }
        } else if let Some(len) = prefix_len(tok, &REGEX_PREFIXES) {
            let v = unquote(&tok[len..]);
            if !v.is_empty() {
                let p = Pattern::new(v);
                if !p.is_valid() {
                    self.warnings.push(trf!("التعبير النمطي «{v}» غير صحيح، فلن يطابق أي شيء.", "The regular expression “{v}” is not valid, so it matches nothing."));
                }
                self.regexes.push(p);
            }
        } else if let Some(len) = prefix_len(tok, &WORD_PREFIXES) {
            let mut alts = Vec::new();
            for w in unquote(&tok[len..]).split('|').map(normalize).filter(|w| !w.trim().is_empty()) {
                push_unique(&mut alts, w);
            }
            if !alts.is_empty() {
                self.words.push(Term { alts });
            }
        } else {
            return false;
        }
        true
    }

    /// True when nothing that narrows the results was typed: no terms,
    /// excludes, extensions or other filters. `dirs_only` alone does not
    /// count, so an empty box never lists the whole disk.
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
            && self.exts.is_empty()
            && self.excludes.is_empty()
            && self.size.is_none()
            && self.mtime.is_none()
            && !self.has_text_filters()
    }

    /// True when the query has something to look for inside a text: terms,
    /// regular expressions, whole words or proximity searches.
    pub fn has_text_criteria(&self) -> bool {
        !self.terms.is_empty() || self.has_text_filters()
    }

    fn has_text_filters(&self) -> bool {
        !self.regexes.is_empty() || !self.words.is_empty() || !self.near.is_empty()
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

    /// Size and date filters. Folders have no size, so `size:` drops them.
    fn meta_ok(&self, e: &Entry) -> bool {
        self.size.is_none_or(|(lo, hi)| !e.is_dir && (lo..=hi).contains(&e.size))
            && self.mtime.is_none_or(|(from, to)| (from..to).contains(&e.mtime))
    }

    /// Type, folder, size and date filters on an entry.
    fn kind_ok(&self, e: &Entry) -> bool {
        (!self.dirs_only || e.is_dir) && self.ext_ok(e) && self.meta_ok(e)
    }

    /// Regex, whole-word and proximity filters on an entry's name.
    fn name_ok(&self, e: &Entry) -> bool {
        self.words.iter().all(|w| w.matches_word(&e.norm_name))
            && self.near.iter().all(|n| n.find(&e.norm_name).is_some())
            && self.regexes.iter().all(|r| r.is_match(e.name()))
    }

    /// Every filter on an entry's name: type, folder, size, date, excluded
    /// words, regex, whole word and proximity.
    pub(crate) fn entry_ok(&self, e: &Entry) -> bool {
        self.kind_ok(e) && !self.excludes.iter().any(|x| e.norm_name.contains(x.as_str())) && self.name_ok(e)
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
///
/// A `fuzzy:` or `root:` query goes to [`crate::fuzzy::fuzzy_search_names`].
/// With `franco:`, names matching an earlier (more likely) transliteration
/// of the first term come first among equally good matches.
pub fn search_names(entries: &[Entry], q: &Query, scope: &str) -> Vec<u32> {
    if q.fuzzy || q.root {
        return crate::fuzzy::fuzzy_search_names(entries, q, scope);
    }
    if q.is_empty() {
        return Vec::new();
    }
    let scope_lower = scope.to_lowercase();
    let mut hits: Vec<(u8, u8, u32, u32)> = entries
        .par_iter()
        .enumerate()
        .filter(|(_, e)| q.terms.iter().all(|t| t.matches(&e.norm_name)))
        .filter(|(_, e)| q.entry_ok(e) && in_scope(&e.path, &scope_lower))
        .map(|(i, e)| {
            let alt = if q.franco { alt_rank(q, &e.norm_name) } else { 0 };
            (name_rank(q, &e.norm_name), alt, e.norm_name.chars().count() as u32, i as u32)
        })
        .collect();
    hits.par_sort_unstable();
    hits.into_iter().map(|(_, _, _, i)| i).collect()
}

/// How `name` matches the first term: 0 is the whole name, 1 its start, 2 elsewhere.
pub(crate) fn name_rank(q: &Query, name: &str) -> u8 {
    match q.terms.first() {
        Some(t) if t.alts.iter().any(|a| name == a) => 0,
        Some(t) if t.alts.iter().any(|a| name.starts_with(a.as_str())) => 1,
        _ => 2,
    }
}

/// Position of the first alternative of the first term found in `name`.
fn alt_rank(q: &Query, name: &str) -> u8 {
    let alts = q.terms.first().map_or(&[][..], |t| &t.alts[..]);
    alts.iter().position(|a| name.contains(a.as_str())).map_or(u8::MAX, |p| p.min(254) as u8)
}

/// One content-search result.
pub struct ContentHit {
    pub index: u32,
    /// A short piece of the document text around the first match.
    pub snippet: String,
    /// How many matches of the query the document holds.
    pub count: u32,
    /// Up to [`MAX_LINES`] matching lines: (1-based line number, the line
    /// cut to about 200 characters around its first match).
    pub lines: Vec<(u32, String)>,
}

/// Most matching lines kept per document in [`ContentHit::lines`].
pub const MAX_LINES: usize = 50;

#[derive(Default)]
pub struct ContentProgress {
    pub total: AtomicUsize,
    pub checked: AtomicUsize,
    pub cancel: AtomicBool,
    pub done: AtomicBool,
    /// Documents whose text came from the content index instead of the file.
    pub from_cache: AtomicUsize,
}

/// Searches inside every supported document in scope. Hits are pushed into
/// `hits` as they are found so the UI can show them while the search runs.
/// With a `cache`, stored texts are used instead of re-reading unchanged
/// documents, and newly extracted ones are added to it.
pub fn search_content(
    entries: &[Entry],
    q: &Query,
    scope: &str,
    progress: &ContentProgress,
    hits: &Mutex<Vec<ContentHit>>,
    cache: Option<&ContentCache>,
) {
    let scope_lower = scope.to_lowercase();
    let candidates: Vec<u32> = entries
        .par_iter()
        .enumerate()
        .filter(|(_, e)| content_cache::is_document(e) && q.kind_ok(e))
        .filter(|(_, e)| in_scope(&e.path, &scope_lower))
        .map(|(i, _)| i as u32)
        .collect();
    progress.total.store(candidates.len(), Ordering::Relaxed);

    if !q.has_text_criteria() {
        progress.done.store(true, Ordering::Relaxed);
        return;
    }

    // The background warm-up pauses while this runs.
    let _searching = cache.map(ContentCache::begin_search);
    // A dedicated pool keeps the global one free for instant filename search.
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get().saturating_sub(1).max(1));
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).thread_name(|i| format!("content-{i}")).build();
    let run = || {
        candidates.par_iter().for_each(|&i| {
            if progress.cancel.load(Ordering::Relaxed) {
                return;
            }
            let entry = &entries[i as usize];
            // Keyed by the file on disk, not the index entry, which may be stale.
            let (text, from_cache) = content_cache::text_of(cache, &entry.path);
            if from_cache {
                progress.from_cache.fetch_add(1, Ordering::Relaxed);
            }
            // A bug matching one document (reported by the panic hook)
            // skips that document instead of ending the whole search.
            if let Some(text) = text
                && let Some(hit) = crate::crash_log::catch(|| content_hit(i, &text, q)).flatten()
                && let Ok(mut h) = hits.lock()
            {
                h.push(hit);
            }
            progress.checked.fetch_add(1, Ordering::Relaxed);
        })
    };
    match pool {
        Ok(pool) => pool.install(run),
        Err(_) => run(),
    }
    if let Some(c) = cache {
        let _ = c.flush();
    }
    progress.done.store(true, Ordering::Relaxed);
}

/// Where `q` matches `text` (whose normalised form is `norm`, with `map`
/// from [`normalize_with_map`]): sorted byte offsets in `text`. With `all`
/// false only the first match of each part of the query is listed, which is
/// enough for a snippet and stops early. `None` when the text doesn't match
/// every term and filter, contains an excluded word, or the query has
/// nothing to look for.
fn text_matches(text: &str, norm: &str, map: &[u32], q: &Query, all: bool) -> Option<Vec<usize>> {
    if q.excludes.iter().any(|x| norm.contains(x.as_str())) {
        return None;
    }
    // Offsets into `norm`, and into `text` for regex matches.
    let mut at: Vec<usize> = Vec::new();
    let mut src: Vec<usize> = Vec::new();
    for (t, whole) in q.terms.iter().map(|t| (t, false)).chain(q.words.iter().map(|t| (t, true))) {
        if all {
            let spans = t.spans(norm, whole);
            if spans.is_empty() {
                return None;
            }
            at.extend(spans.iter().map(|&(s, _)| s));
        } else if whole {
            at.push(t.alts.iter().filter_map(|a| word_matches(norm, a).next()).min()?);
        } else {
            at.push(t.find_in(norm)?);
        }
    }
    for n in &q.near {
        if all {
            let hits = n.all(norm);
            if hits.is_empty() {
                return None;
            }
            at.extend(hits);
        } else {
            at.push(n.find(norm)?);
        }
    }
    for r in &q.regexes {
        let re = r.re.as_ref()?;
        let first = re.find(text)?.start();
        let before = src.len();
        if all {
            src.extend(r.ranges(text).map(|(s, _)| s));
        }
        if src.len() == before {
            // Only empty matches (`^`), or the first match is enough.
            src.push(first);
        }
    }
    src.extend(at.into_iter().map(|p| map[p] as usize));
    if src.is_empty() {
        return None;
    }
    src.sort_unstable();
    src.dedup();
    Some(src)
}

/// If `text` matches every term and filter and contains no excluded word,
/// returns a snippet around the earliest match.
#[cfg_attr(not(test), allow(dead_code))] // for the UI and the CLI; not called there yet
pub fn match_text(text: &str, q: &Query) -> Option<String> {
    let (norm, map) = normalize_with_map(text);
    let at = text_matches(text, &norm, &map, q, false)?;
    Some(snippet_at(text, at[0]))
}

/// Counts the matches of `q` in `text` and returns them with up to `max`
/// matching lines (1-based line number, the line around its first match).
/// `(0, [])` when the text doesn't match the query.
#[cfg_attr(not(test), allow(dead_code))] // for the UI and the CLI; not called there yet
pub fn all_matches(text: &str, q: &Query, max: usize) -> (u32, Vec<(u32, String)>) {
    let (norm, map) = normalize_with_map(text);
    match text_matches(text, &norm, &map, q, true) {
        Some(at) => (u32::try_from(at.len()).unwrap_or(u32::MAX), matching_lines(text, &at, max)),
        None => (0, Vec::new()),
    }
}

/// A content hit for entry `index` if `text` matches, normalising the text
/// only once. Documents that don't match stop at the cheap first-match check.
fn content_hit(index: u32, text: &str, q: &Query) -> Option<ContentHit> {
    if let Some(m) = extract::split_members(text) {
        return members::archive_hit(index, &m, q); // an archive: per member
    }
    let (norm, map) = normalize_with_map(text);
    let first = text_matches(text, &norm, &map, q, false)?[0];
    let at = text_matches(text, &norm, &map, q, true).unwrap_or_else(|| vec![first]);
    Some(ContentHit {
        index,
        snippet: snippet_at(text, first),
        count: u32::try_from(at.len()).unwrap_or(u32::MAX),
        lines: matching_lines(text, &at, MAX_LINES),
    })
}

/// The first `max` distinct lines holding a match at one of the sorted
/// byte offsets `at`, each cut to about 200 characters around that match.
fn matching_lines(text: &str, at: &[usize], max: usize) -> Vec<(u32, String)> {
    let mut lines = Vec::new();
    let (mut line_no, mut counted, mut line_end) = (1u32, 0, 0);
    for &p in at {
        if lines.len() >= max {
            break;
        }
        if p < line_end {
            continue; // another match in a line already listed
        }
        line_no += text.as_bytes()[counted..p].iter().filter(|&&b| b == b'\n').count() as u32;
        counted = p;
        let start = text[..p].rfind('\n').map_or(0, |i| i + 1);
        line_end = text[p..].find('\n').map_or(text.len(), |i| p + i);
        lines.push((line_no, excerpt(&text[start..line_end], p - start, 60, 140)));
    }
    lines
}

/// About 40 characters either side of byte offset `pos`, on one line.
fn snippet_at(text: &str, pos: usize) -> String {
    excerpt(text, pos, 40, 80)
}

/// `before` characters before byte offset `pos` and `after` from it, with
/// white space collapsed and `…` where text was cut.
fn excerpt(text: &str, pos: usize, before: usize, after: usize) -> String {
    let start = text[..pos].char_indices().rev().nth(before).map_or(0, |(i, _)| i);
    let end = text[pos..].char_indices().nth(after).map_or(text.len(), |(i, _)| pos + i);
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
/// covered by any occurrence of any alternative of any term, whole word,
/// proximity word or regex, sorted and merged. Diacritics right after a match
/// are included so the highlighted word stays whole. Excluded words are not
/// highlighted.
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
    for t in q.words.iter() {
        ranges.extend(t.spans(&norm, true).into_iter().map(|(s, e)| (map[s] as usize, src_end(e - 1))));
    }
    for t in q.near.iter().flat_map(|n| &n.words) {
        ranges.extend(t.spans(&norm, false).into_iter().map(|(s, e)| (map[s] as usize, src_end(e - 1))));
    }
    for r in &q.regexes {
        ranges.extend(r.ranges(name));
    }
    if q.fuzzy || q.root {
        // The word a typo-tolerant or root match found.
        ranges.extend(crate::fuzzy::word_spans(&norm, q).into_iter().map(|(s, e)| (map[s] as usize, src_end(e - 1))));
    }
    merge_spans(ranges)
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
        for input in ["ملف 12", "مشروع", "ملف .txt", "ملف size:>50kb", "size:<1kb", "ملف dm:>1970-01-05"] {
            let q = Query::parse(input);
            let t = std::time::Instant::now();
            let n = search_names(&entries, &q, "").len();
            println!("search {input}: {n} hits in {:?}", t.elapsed());
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
            search_content(&entries, &Query::parse(q), "", &progress, &hits, None);
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
        // Size filters narrow the candidates; regex alone searches the text.
        assert_eq!(run("تقرير size:>100b", false).0, 0);
        assert_eq!(run("تقرير حجم:<1ك", false).2.len(), 1);
        assert_eq!(run("regex:السنو.", false).2.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn content_search_uses_the_cache() {
        let dir = std::env::temp_dir().join(format!("bahth_content_cache_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let files = [("تقرير.txt", "تقرير المبيعات السنوي\nالمبيعات ارتفعت"), ("اخر.txt", "لا شيء هنا"), ("فارغ.txt", "  ")];
        let entries: Vec<Entry> = files
            .iter()
            .map(|(name, text)| {
                let p = dir.join(name);
                std::fs::write(&p, text).unwrap();
                Entry { size: text.len() as u64, mtime: 1000, ..entry(p.to_str().unwrap(), false) }
            })
            .collect();
        let set_mtime = |name: &str, secs: u64| {
            let f = std::fs::File::options().write(true).open(dir.join(name)).unwrap();
            f.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs)).unwrap();
        };
        for (name, _) in &files {
            set_mtime(name, 1000);
        }
        let cache = ContentCache::open(&dir.join("store")).unwrap();
        let run = |entries: &[Entry], q: &str| {
            let progress = ContentProgress::default();
            let hits = Mutex::new(Vec::new());
            search_content(entries, &Query::parse(q), "", &progress, &hits, Some(&cache));
            assert!(!cache.searching());
            let mut hits: Vec<(u32, u32, usize)> =
                hits.into_inner().unwrap().into_iter().map(|h| (h.index, h.count, h.lines.len())).collect();
            hits.sort_unstable();
            (progress.from_cache.load(Ordering::Relaxed), progress.checked.load(Ordering::Relaxed), hits)
        };
        // First run extracts every document (the empty one is stored as "no text").
        assert_eq!(run(&entries, "المبيعات"), (0, 3, vec![(0, 2, 2)]));
        assert_eq!(cache.stats().documents, 3);
        assert_eq!(run(&entries, "المبيعات"), (3, 3, vec![(0, 2, 2)]));
        // The store was saved at the end of the search.
        assert_eq!(ContentCache::open(&dir.join("store")).unwrap().stats().documents, 3);
        // The files change on disk but their index entries don't (auto-index
        // off, a missed event): the cache is keyed by the disk, so they are
        // read again and the new text is found.
        for (name, _) in &files {
            std::fs::write(dir.join(name), "المبيعات في كل مكان").unwrap();
            set_mtime(name, 2000);
        }
        assert_eq!(run(&entries, "المبيعات"), (0, 3, vec![(0, 1, 1), (1, 1, 1), (2, 1, 1)]));
        assert_eq!(run(&entries, "المبيعات").0, 3);
        // A same-size edit just now: read again, and not stored while the
        // whole-second mtime couldn't tell it from a further edit.
        std::fs::write(dir.join(files[1].0), "الأرباح في كل مكان!!").unwrap();
        assert_eq!(std::fs::metadata(dir.join(files[1].0)).unwrap().len(), "المبيعات في كل مكان".len() as u64);
        assert_eq!(run(&entries, "الأرباح"), (2, 3, vec![(1, 1, 1)]));
        assert_eq!(run(&entries, "الأرباح").0, 2);
        drop(cache);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // ------------------------------------------------- size and date filters

    /// 2024-05-15 (a Wednesday) 10:00 UTC.
    const NOW: i64 = 1_715_767_200;

    fn at(input: &str) -> Query {
        Query::parse_at(input, NOW, 0)
    }

    fn utc(y: i64, m: u32, d: u32) -> i64 {
        days_from_civil(y, m, d) * DAY
    }

    #[test]
    fn civil_dates_round_trip() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(utc(2024, 5, 15) + 10 * 3600, NOW);
        for z in [-800_000, -1, 0, 59, 60, 11_016, 19_858, 2_000_000] {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z);
        }
        assert_eq!(civil_from_days(utc(2024, 2, 29) / DAY), (2024, 2, 29));
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
    }

    #[test]
    fn parse_sizes() {
        assert_eq!(at("size:>10mb").size, Some((10 * MB + 1, u64::MAX)));
        assert_eq!(at("size:>=2gb").size, Some((2 * GB, u64::MAX)));
        assert_eq!(at("size:<500kb").size, Some((0, 500 * KB - 1)));
        assert_eq!(at("size:<=1k").size, Some((0, KB)));
        assert_eq!(at("SIZE:1mb..5mb").size, Some((MB, 5 * MB)));
        assert_eq!(at("size:1.5mb").size, Some((MB * 3 / 2, MB * 3 / 2)));
        assert_eq!(at("size:..1kb").size, Some((0, KB)));
        assert_eq!(at("size:10").size, Some((10, 10)));
        // Arabic prefix, units and digits.
        assert_eq!(at("حجم:>10م").size, Some((10 * MB + 1, u64::MAX)));
        assert_eq!(at("حجم:<٥٠٠ك").size, Some((0, 500 * KB - 1)));
        assert_eq!(at("حجم:٢ج..٣ج").size, Some((2 * GB, 3 * GB)));
        assert_eq!(at("حجم:١٫٥م").size, Some((MB * 3 / 2, MB * 3 / 2)));
        // Named sizes.
        assert_eq!(at("size:empty").size, Some((0, 0)));
        assert_eq!(at("حجم:فارغ").size, Some((0, 0)));
        assert_eq!(at("size:tiny").size, Some((1, 10 * KB)));
        assert_eq!(at("حجم:صغير").size, at("size:small").size);
        assert_eq!(at("حجم:كبير").size, Some((MB + 1, 16 * MB)));
        assert_eq!(at("حجم:\"كبير جدا\"").size, at("size:huge").size);
        assert_eq!(at("size:huge").size, Some((16 * MB + 1, 128 * MB)));
        assert_eq!(at("size:gigantic").size, Some((128 * MB + 1, u64::MAX)));
        // Two filters intersect.
        assert_eq!(at("size:>1mb size:<2mb").size, Some((MB + 1, 2 * MB - 1)));
        // A size filter alone narrows the results.
        assert!(!at("size:>1mb").is_empty());
        assert!(at("size:>1mb").warnings.is_empty());
    }

    #[test]
    fn bad_filter_values_become_words() {
        for input in ["size:>10xb", "size:5mb..1mb", "size:<0", "size:abc", "حجم:..", "dm:2024-13", "dm:soon", "تاريخ:2024-02-30"] {
            let q = at(input);
            assert_eq!(q.size, None, "{input}");
            assert_eq!(q.mtime, None, "{input}");
            assert_eq!(q.warnings.len(), 1, "{input}");
            assert_eq!(alts(&q), vec![vec![normalize(input)]], "{input}");
        }
        assert!(at("size:abc").warnings[0].contains("الحجم"));
        assert!(at("dm:soon").warnings[0].contains("التاريخ"));
        // An empty value is ignored, like `ext:`.
        let q = at("size: dm: regex: ww:");
        assert!(q.is_empty() && q.warnings.is_empty());
    }

    #[test]
    fn parse_calendar_dates() {
        let day = |y, m, d| (utc(y, m, d), utc(y, m, d) + DAY);
        assert_eq!(at("dm:today").mtime, Some(day(2024, 5, 15)));
        assert_eq!(at("dm:yesterday").mtime, Some(day(2024, 5, 14)));
        assert_eq!(at("تاريخ:اليوم").mtime, at("dm:today").mtime);
        assert_eq!(at("تاريخ:أمس").mtime, at("dm:yesterday").mtime);
        assert_eq!(at("تاريخ:امس").mtime, at("dm:yesterday").mtime);
        // Weeks start on Saturday: 2024-05-11.
        assert_eq!(at("dm:thisweek").mtime, Some((utc(2024, 5, 11), utc(2024, 5, 18))));
        assert_eq!(at("dm:lastweek").mtime, Some((utc(2024, 5, 4), utc(2024, 5, 11))));
        assert_eq!(at("تاريخ:هذا_الأسبوع").mtime, at("dm:thisweek").mtime);
        assert_eq!(at("dm:thismonth").mtime, Some((utc(2024, 5, 1), utc(2024, 6, 1))));
        assert_eq!(at("dm:lastmonth").mtime, Some((utc(2024, 4, 1), utc(2024, 5, 1))));
        assert_eq!(at("تاريخ:هذا_الشهر").mtime, at("dm:thismonth").mtime);
        assert_eq!(at("dm:thisyear").mtime, Some((utc(2024, 1, 1), utc(2025, 1, 1))));
        assert_eq!(at("تاريخ:هذه_السنة").mtime, at("dm:thisyear").mtime);
        assert_eq!(at("تاريخ:هذه_السنه").mtime, at("dm:thisyear").mtime);
        assert_eq!(at("dm:lastyear").mtime, Some((utc(2023, 1, 1), utc(2024, 1, 1))));
        // Quoted Arabic words may use a space instead of `_`.
        assert_eq!(at("تاريخ:\"هذا الشهر\" عقد").mtime, at("dm:thismonth").mtime);
        assert_eq!(alts(&at("تاريخ:\"هذا الشهر\" عقد")), vec![vec!["عقد"]]);
    }

    #[test]
    fn parse_dates_in_local_time() {
        // 01:00 on the 16th in UTC+3 is still the 15th in UTC.
        let late = utc(2024, 5, 15) + 22 * 3600;
        let q = Query::parse_at("dm:today", late, 3 * 3600);
        assert_eq!(q.mtime, Some((utc(2024, 5, 16) - 3 * 3600, utc(2024, 5, 17) - 3 * 3600)));
        let q = Query::parse_at("dm:2024-05-12", NOW, -5 * 3600);
        assert_eq!(q.mtime, Some((utc(2024, 5, 12) + 5 * 3600, utc(2024, 5, 13) + 5 * 3600)));
        // The global offset is what `parse` uses.
        set_utc_offset_secs(7200);
        assert_eq!(utc_offset_secs(), 7200);
        let (from, to) = Query::parse("dm:today").mtime.unwrap();
        assert_eq!((to - from, (from + 7200) % DAY), (DAY, 0));
        set_utc_offset_secs(0);
    }

    #[test]
    fn parse_explicit_dates() {
        assert_eq!(at("dm:2024").mtime, Some((utc(2024, 1, 1), utc(2025, 1, 1))));
        assert_eq!(at("dm:2024-05").mtime, Some((utc(2024, 5, 1), utc(2024, 6, 1))));
        assert_eq!(at("dm:2024-12").mtime, Some((utc(2024, 12, 1), utc(2025, 1, 1))));
        assert_eq!(at("dm:2024-05-12").mtime, Some((utc(2024, 5, 12), utc(2024, 5, 13))));
        assert_eq!(at("dm:2024/5/2").mtime, Some((utc(2024, 5, 2), utc(2024, 5, 3))));
        assert_eq!(at("dm:>2024-01-01").mtime, Some((utc(2024, 1, 2), i64::MAX)));
        assert_eq!(at("dm:>=2024-01-01").mtime, Some((utc(2024, 1, 1), i64::MAX)));
        assert_eq!(at("dm:<2023").mtime, Some((i64::MIN, utc(2023, 1, 1))));
        assert_eq!(at("dm:<=2023").mtime, Some((i64::MIN, utc(2024, 1, 1))));
        assert_eq!(at("dm:2024-01..2024-03").mtime, Some((utc(2024, 1, 1), utc(2024, 4, 1))));
        assert_eq!(at("dm:2024-02-28..").mtime, Some((utc(2024, 2, 28), i64::MAX)));
        assert_eq!(at("dm:2024-02-29").mtime, Some((utc(2024, 2, 29), utc(2024, 3, 1))));
        // Arabic-Indic digits.
        assert_eq!(at("تاريخ:٢٠٢٤-٠٥").mtime, at("dm:2024-05").mtime);
        assert_eq!(at("dm:>٢٠٢٤-٠١-٠١").mtime, at("dm:>2024-01-01").mtime);
        // Two filters intersect.
        assert_eq!(at("dm:2024 dm:<2024-03").mtime, Some((utc(2024, 1, 1), utc(2024, 3, 1))));
    }

    #[test]
    fn parse_recent_periods() {
        assert_eq!(at("dm:7d").mtime, Some((NOW - 7 * DAY, i64::MAX)));
        assert_eq!(at("dm:3w").mtime, Some((NOW - 21 * DAY, i64::MAX)));
        assert_eq!(at("dm:2m").mtime, Some((utc(2024, 3, 15) + 10 * 3600, i64::MAX)));
        assert_eq!(at("dm:1y").mtime, Some((utc(2023, 5, 15) + 10 * 3600, i64::MAX)));
        assert_eq!(at("dm:٧d").mtime, at("dm:7d").mtime);
        assert_eq!(at("dm:<7d").mtime, Some((i64::MIN, NOW - 7 * DAY)));
        // Month arithmetic clamps the day: 31 March minus one month is 29 February.
        let q = Query::parse_at("dm:1m", utc(2024, 3, 31), 0);
        assert_eq!(q.mtime, Some((utc(2024, 2, 29), i64::MAX)));
    }

    #[test]
    fn name_search_with_size_and_date() {
        let entries = vec![
            sized(r"C:\a\تقرير صغير.pdf", false, 2 * KB, utc(2024, 5, 15) + 100),
            sized(r"C:\a\تقرير كبير.pdf", false, 20 * MB, utc(2024, 5, 14)),
            sized(r"C:\a\تقرير", true, 0, utc(2024, 5, 15)),
            sized(r"C:\a\فارغ.txt", false, 0, utc(2023, 1, 1)),
        ];
        let find = |s: &str| search_names(&entries, &at(s), "");
        assert_eq!(find("تقرير size:>10mb"), vec![1]);
        assert_eq!(find("تقرير حجم:<10ك"), vec![0]);
        // Folders have no size.
        assert_eq!(find("size:<1gb").len(), 3);
        assert_eq!(find("size:empty"), vec![3]);
        // Dates apply to folders too.
        assert_eq!(find("تقرير dm:today"), vec![2, 0]);
        assert_eq!(find("تاريخ:أمس"), vec![1]);
        assert_eq!(find("dm:<2024"), vec![3]);
        assert_eq!(find("dm:2024 size:>1kb ext:pdf").len(), 2);
    }

    // ------------------------------------------------- regex and whole word

    #[test]
    fn parse_regex_and_words() {
        let q = at(r"regex:^IMG_\d+\.jpg$ re:a|b");
        assert_eq!(q.regexes.iter().map(|r| r.source.as_str()).collect::<Vec<_>>(), vec![r"^IMG_\d+\.jpg$", "a|b"]);
        assert!(q.terms.is_empty() && !q.is_empty() && q.has_text_criteria());
        let q = at("regex:(abc");
        assert_eq!(q.regexes.len(), 1);
        assert_eq!(q.warnings.len(), 1);
        assert!(q.warnings[0].contains("(abc"));
        // A quoted value keeps its spaces.
        assert_eq!(at(r#"regex:"a b" c"#).regexes[0].source, "a b");
        assert_eq!(alts(&at(r#"regex:"a b" c"#)), vec![vec!["c"]]);
        let q = at("ww:أحمد كلمة:عقد|إيجار");
        let words: Vec<Vec<&str>> = q.words.iter().map(|t| t.alts.iter().map(String::as_str).collect()).collect();
        assert_eq!(words, vec![vec!["احمد"], vec!["عقد", "ايجار"]]);
        assert!(q.terms.is_empty() && !q.is_empty());
        // A huge pattern hits the size limit instead of eating memory.
        assert!(!Pattern::new(r"\w{1000}\w{1000}\w{1000}").is_valid());
    }

    #[test]
    fn whole_words() {
        assert_eq!(word_matches("عقد العقد عقد_2 عقدة", "عقد").collect::<Vec<_>>(), vec![0, "عقد العقد ".len()]);
        assert!(word_matches("abc", "b").next().is_none());
        assert_eq!(split_words("  a-bc_د.ه ").collect::<Vec<_>>(), vec![(2, "a"), (4, "bc"), (7, "د"), (10, "ه")]);
        assert!(split_words(" .. ").next().is_none());
    }

    #[test]
    fn name_search_with_regex_and_words() {
        let entries = vec![
            entry(r"C:\a\IMG_2041.JPG", false),
            entry(r"C:\a\img-x.jpg", false),
            entry(r"C:\a\عقد إيجار.docx", false),
            entry(r"C:\a\العقود.docx", false),
            entry(r"C:\a\عقد_2024.pdf", false),
        ];
        let find = |s: &str| search_names(&entries, &at(s), "");
        assert_eq!(find(r"regex:^img_\d+\.jpg$"), vec![0]);
        assert_eq!(find("re:jpg$ !x"), vec![0]);
        // Regex sees the original name: the hamza is not folded.
        assert_eq!(find("re:إيجار"), vec![2]);
        assert!(find("re:ايجار").is_empty());
        assert!(find("regex:(").is_empty());
        assert_eq!(find("ww:عقد"), vec![4, 2]); // shorter name first
        assert_eq!(find("كلمة:أيجار"), vec![2]);
        assert!(find("ww:عق").is_empty());
        assert_eq!(find("ww:2024 ext:pdf"), vec![4]);
    }

    #[test]
    fn content_regex_and_words() {
        let text = "رقم الفاتورة INV-2024-0042 بتاريخ اليوم.\nالعقدة ليست عقداً.";
        let s = match_text(text, &at(r"regex:inv-\d{4}-\d+")).unwrap();
        assert!(s.contains("INV-2024-0042"), "{s}");
        assert!(match_text(text, &at(r"regex:^العقدة")).is_some());
        assert!(match_text(text, &at("regex:[")).is_none());
        assert!(match_text(text, &at("ww:فاتوره")).is_none());
        assert!(match_text(text, &at("ww:الفاتوره")).is_some());
        assert!(match_text(text, &at("ww:عقدا")).is_some());
        assert!(match_text(text, &at("ww:عقد")).is_none());
    }

    #[test]
    fn highlight_regex_words_and_near() {
        assert_eq!(highlighted("IMG_2041.JPG", r"re:\d+"), vec!["2041"]);
        assert_eq!(highlighted("عقد العقد.pdf", "ww:عقد"), vec!["عقد"]);
        assert_eq!(highlighted("عَقْد إيجار", "ww:عقد"), vec!["عَقْد"]);
        assert_eq!(highlighted("a ab", "re:a ww:ab"), vec!["a", "ab"]);
        assert_eq!(highlighted("عقد شقة إيجار", r#""عقد ايجار"~3"#), vec!["عقد", "إيجار"]);
        // Empty regex matches are not highlighted.
        assert!(highlighted("abc", "re:^").is_empty());
    }

    // ------------------------------------------------------------ proximity

    #[test]
    fn parse_proximity() {
        let q = at(r#""عقد الإيجار"~5 تقرير"#);
        assert_eq!(q.near.len(), 1);
        assert_eq!(q.near[0].dist, 5);
        let words: Vec<Vec<&str>> = q.near[0].words.iter().map(|t| t.alts.iter().map(String::as_str).collect()).collect();
        assert_eq!(words, vec![vec!["عقد"], vec!["الايجار", "ايجار"]]);
        assert_eq!(alts(&q), vec![vec!["تقرير"]]);
        assert_eq!(at(r#""a b"~٣"#).near[0].dist, 3);
        // One word is a plain phrase; `~` without digits stays a word.
        assert_eq!(alts(&at(r#""عقد"~5"#)), vec![vec!["عقد"]]);
        let q = at(r#""a b"~x"#);
        assert!(q.near.is_empty());
        assert_eq!(alts(&q), vec![vec!["a b"], vec!["~x"]]);
        assert_eq!(alts(&at(r#""a b"~5x"#)), vec![vec!["a b"], vec!["~5x"]]);
    }

    #[test]
    fn proximity_matching() {
        let q = at(r#""عقد إيجار"~3"#);
        assert!(match_text("هذا عقد شقة سكنية إيجار سنوي", &q).is_some());
        assert!(match_text("الإيجار الشهري لهذا العقد", &q).is_some()); // any order
        assert!(match_text("عقد واحد اثنان ثلاثة أربعة إيجار", &q).is_none());
        // A far pair doesn't count but a later close one does.
        let s = match_text("عقد ١ ٢ ٣ ٤ ٥ نص طويل هنا ثم عقد إيجار", &q).unwrap();
        assert!(s.starts_with("…") || s.contains("عقد إيجار"), "{s}");
        let near = &q.near[0];
        assert_eq!(near.find("x عقد ايجار"), Some(2));
        assert_eq!(near.find("عقد 1 2 3 4 5 ايجار 6 عقد"), Some("عقد 1 2 3 4 5 ".len()));
        // Three words: all within the distance of each other.
        let q3 = at(r#""a b c"~2"#);
        assert!(match_text("c x a b", &q3).is_none());
        assert!(match_text("c a b", &q3).is_some());
        // Name search uses the file name's words.
        let entries = vec![entry(r"C:\عقد شقة إيجار.pdf", false), entry(r"C:\عقد 1 2 3 4 إيجار.pdf", false)];
        assert_eq!(search_names(&entries, &q, ""), vec![0]);
    }

    // --------------------------------------------------- all matches in text

    #[test]
    fn all_matches_counts_and_lines() {
        let text = "سطر أول فيه التقرير\n\nالتقرير والتقرير الثاني\r\nلا شيء\nتقرير أخير";
        let (count, lines) = all_matches(text, &at("التقرير"), 50);
        // "التقرير" and its bare form "تقرير" overlap and count once.
        assert_eq!(count, 4);
        assert_eq!(
            lines,
            vec![
                (1, "سطر أول فيه التقرير".to_string()),
                (3, "التقرير والتقرير الثاني".to_string()),
                (5, "تقرير أخير".to_string())
            ]
        );
        let (count, lines) = all_matches(text, &at("التقرير"), 2);
        assert_eq!((count, lines.len()), (4, 2));
        assert_eq!(all_matches(text, &at("غير موجود"), 50), (0, vec![]));
        assert_eq!(all_matches(text, &at("تقرير !شيء"), 50), (0, vec![]));
        // Every term must be present; all are counted.
        assert_eq!(all_matches(text, &at("تقرير ثاني"), 50).0, 5);
        assert_eq!(all_matches(text, &at("تقرير مفقود"), 50).0, 0);
        // Regex, whole word and proximity matches count too.
        assert_eq!(all_matches(text, &at("re:تقرير"), 50).0, 4);
        assert_eq!(all_matches(text, &at("ww:تقرير"), 50), (1, vec![(5, "تقرير أخير".to_string())]));
        assert_eq!(all_matches(text, &at(r#""التقرير الثاني"~1"#), 50).0, 2);
    }

    #[test]
    fn all_matches_long_lines() {
        let long = format!("{}مفتاح{}", "أ".repeat(500), "ب".repeat(500));
        let text = format!("x\n{long}\n{}", "مفتاح ".repeat(3000));
        let (count, lines) = all_matches(&text, &at("مفتاح"), 50);
        assert_eq!(count, 3001);
        assert_eq!(lines.len(), 2);
        let (n, line) = &lines[0];
        assert_eq!(*n, 2);
        assert!(line.starts_with('…') && line.ends_with('…') && line.contains("مفتاح"), "{line}");
        assert!(line.chars().count() <= 203, "{}", line.chars().count());
        assert_eq!(lines[1].0, 3);
    }

    #[test]
    fn content_hit_fields() {
        let text = "عقد الإيجار\nبند ثانٍ\nنهاية العقد";
        let hit = content_hit(7, text, &at("عقد")).unwrap();
        assert_eq!(hit.index, 7);
        assert_eq!(hit.snippet, match_text(text, &at("عقد")).unwrap());
        assert_eq!(hit.count, 2);
        assert_eq!(hit.lines, vec![(1, "عقد الإيجار".to_string()), (3, "نهاية العقد".to_string())]);
        assert!(content_hit(0, text, &at("مفقود")).is_none());
        assert!(content_hit(0, text, &at("ext:pdf")).is_none());
    }
}
