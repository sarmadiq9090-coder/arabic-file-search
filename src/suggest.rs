//! What the search shows besides results (0.9): the favourite and recently
//! opened files while the box is empty, and what to try instead when a
//! search finds nothing ("did you mean …", search inside documents, all
//! drives, all file types, the web).
//!
//! Everything here is platform independent and unit-tested on Linux; the
//! popup (`src/ui/quick.rs`) and the main window (`src/ui/empty_state.rs`)
//! only draw it and apply the chosen suggestion.

use rayon::prelude::*;
use std::collections::HashMap;

use crate::index::Entry;
use crate::normalize::normalize;
use crate::quick::{Group, Icon, Row, Target};
use crate::search::{self, Query, in_scope, split_words};

/// At most this many recently opened files are listed.
pub const RECENT_ROWS: usize = 8;
/// At most this many favourites are listed while the box is empty.
pub const FAVOURITE_ROWS: usize = 6;
/// At most this many remembered files are checked for existence to fill the
/// recent list (each check may touch the disk).
pub const MAX_PROBES: usize = 24;
/// Queries of more words than this get no "did you mean".
const MAX_TOKENS: usize = 6;
/// Words longer than this (characters) get no spelling suggestion.
const MAX_WORD: usize = 24;

// ------------------------------------------------------------------ empty box

/// The first `limit` of `paths` (newest first) that still exist, as
/// (path, what `probe` says about it: `None` = gone). At most `max_probes`
/// paths are probed, so a list of deleted files can't make it slow.
pub fn pick_recent<'a, T>(
    paths: impl IntoIterator<Item = &'a str>,
    limit: usize,
    max_probes: usize,
    mut probe: impl FnMut(&str) -> Option<T>,
) -> Vec<(String, T)> {
    paths
        .into_iter()
        .take(max_probes)
        .filter_map(|p| probe(p).map(|t| (p.to_string(), t)))
        .take(limit)
        .collect()
}

/// The drive letter of a `C:\...` path, upper case; `None` for UNC paths,
/// relative paths and anything else.
pub fn drive_letter(path: &str) -> Option<char> {
    let mut chars = path.chars();
    let letter = chars.next().filter(char::is_ascii_alphabetic)?;
    (chars.next() == Some(':') && matches!(chars.next(), Some('\\' | '/'))).then(|| letter.to_ascii_uppercase())
}

/// The list shown while the search box is empty: the favourites, then the
/// recently opened files that are not favourites, each section capped.
pub fn empty_rows(pins: Vec<Row>, recent: Vec<Row>) -> Vec<Row> {
    let mut rows: Vec<Row> = pins.into_iter().take(FAVOURITE_ROWS).map(|r| Row { group: Group::Pinned, ..r }).collect();
    let pinned = |t: &Target| {
        rows.iter().any(|p| p.target.pin_target().zip(t.pin_target()).is_some_and(|(a, b)| a.eq_ignore_ascii_case(b)))
    };
    let recent: Vec<Row> =
        recent.into_iter().filter(|r| !pinned(&r.target)).take(RECENT_ROWS).map(|r| Row { group: Group::Recent, ..r }).collect();
    rows.extend(recent);
    rows
}

// ------------------------------------------------------------------ suggestions

/// Something to try when a search found nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Suggestion {
    /// Search for this text instead ("did you mean").
    Query(String),
    /// Drop the file-type filter.
    AllKinds,
    /// Search all drives instead of the chosen folder.
    AllDrives,
    /// Search inside documents too.
    Contents,
    /// Search the web for the query.
    Web,
}

impl Suggestion {
    /// What the suggestion says, in the interface language.
    pub fn label(&self) -> String {
        match self {
            Suggestion::Query(t) => {
                let t = shorten(t, 40);
                trf!("هل تقصد «{}»؟", "Did you mean “{}”?", t)
            }
            Suggestion::AllKinds => tr!("ابحث في كل أنواع الملفات", "Search all file types").to_string(),
            Suggestion::AllDrives => tr!("ابحث في كل الأقراص", "Search all drives").to_string(),
            Suggestion::Contents => tr!("ابحث داخل محتوى الملفات", "Search inside file contents").to_string(),
            Suggestion::Web => tr!("ابحث في الويب", "Search the web").to_string(),
        }
    }

    /// Its glyph (Segoe Fluent / MDL2 code point).
    pub fn glyph(&self) -> char {
        match self {
            Suggestion::Query(_) => crate::quick::glyph::SEARCH,
            Suggestion::AllKinds => '\u{E71C}',
            Suggestion::AllDrives => '\u{EDA2}',
            Suggestion::Contents => '\u{E8A5}',
            Suggestion::Web => crate::quick::glyph::GLOBE,
        }
    }
}

/// `text` cut to `max` characters, with "…" when cut.
fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max.saturating_sub(1)).collect();
    s.push('…');
    s
}

/// A web search for `query`.
pub fn web_url(query: &str) -> String {
    crate::launcher::web::google_url(query.trim())
}

/// What the search that found nothing was like.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Situation {
    /// A text that finds something ([`did_you_mean`]).
    pub did_you_mean: Option<String>,
    /// A file-type filter was chosen.
    pub kind_filtered: bool,
    /// The search was limited to a folder.
    pub scoped: bool,
    /// Searching inside documents is off and could find this query.
    pub contents: bool,
    /// Offer a web search (the popup has its own web rows).
    pub web: bool,
}

/// The suggestions for `query`, which found nothing, best first: a better
/// spelling, then a wider search (all types, all drives, inside documents),
/// then the web.
pub fn suggestions(query: &str, s: &Situation) -> Vec<Suggestion> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let mut v = Vec::new();
    if let Some(t) = s.did_you_mean.as_deref().map(str::trim)
        && !t.is_empty()
        && normalize(t) != normalize(query)
    {
        v.push(Suggestion::Query(t.to_string()));
    }
    if s.kind_filtered {
        v.push(Suggestion::AllKinds);
    }
    if s.scoped {
        v.push(Suggestion::AllDrives);
    }
    if s.contents {
        v.push(Suggestion::Contents);
    }
    if s.web {
        v.push(Suggestion::Web);
    }
    v
}

/// The suggestions as rows of the quick-search list.
pub fn rows(list: &[Suggestion]) -> Vec<Row> {
    list.iter()
        .map(|s| Row {
            group: Group::Suggest,
            title: s.label(),
            subtitle: String::new(),
            target: Target::Apply(s.clone()),
            icon: Icon::Glyph(s.glyph()),
        })
        .collect()
}

/// Whether the quick-search list shows no result: only web rows and the
/// "more" rows (feedback, open in the window).
pub fn nothing_found(rows: &[Row]) -> bool {
    rows.iter().all(|r| matches!(r.group, Group::Web | Group::More))
}

/// Puts the suggestion rows first, before the web and "more" rows.
pub fn insert_rows(rows: &mut Vec<Row>, list: &[Suggestion]) {
    let at = rows.iter().position(|r| matches!(r.group, Group::Web | Group::More)).unwrap_or(rows.len());
    rows.splice(at..at, self::rows(list));
}

// ------------------------------------------------------------------ did you mean

/// A word typed as is (letters and digits only), not a filter (`ext:pdf`,
/// `!old`, `*.txt`, `"a b"`).
fn is_plain(token: &str) -> bool {
    !token.is_empty() && token.chars().all(char::is_alphanumeric)
}

/// For a name search of `text` that found nothing (even with the keyboard
/// layout, Arabizi and typo fallbacks): a text that finds something.
///
/// - Several words: the query without the one word no name has with the
///   others (the last typed tried first).
/// - One word: the nearest word of the names in reach (`scope` and the
///   query's filters), a little farther than the typo fallback goes, also
///   for the word re-typed in the other keyboard layout.
///
/// `parse` turns a text into a [`Query`] as the search does.
pub fn did_you_mean(entries: &[Entry], text: &str, scope: &str, parse: &(impl Fn(&str) -> Query + Sync)) -> Option<String> {
    let q = parse(text);
    if q.terms.is_empty() {
        return None;
    }
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.len() > MAX_TOKENS {
        return None;
    }
    let plain: Vec<usize> = (0..tokens.len()).filter(|&i| is_plain(tokens[i])).collect();
    match plain.as_slice() {
        [] => None,
        [i] => {
            let scope_lower = scope.to_lowercase();
            let word = closest_word(entries, tokens[*i], &|e: &Entry| q.entry_ok(e) && in_scope(&e.path, &scope_lower))?;
            Some(replace_token(&tokens, *i, Some(&word))).filter(|t| finds(entries, &parse(t), scope))
        }
        _ => plain.iter().rev().find_map(|&i| {
            let rest = replace_token(&tokens, i, None);
            finds(entries, &parse(&rest), scope).then_some(rest)
        }),
    }
}

/// Whether a name search for `q` finds something.
fn finds(entries: &[Entry], q: &Query, scope: &str) -> bool {
    !q.terms.is_empty() && !search::search_names(entries, q, scope).is_empty()
}

/// `tokens` joined by spaces, with token `i` replaced by `with` (or left out).
fn replace_token(tokens: &[&str], i: usize, with: Option<&str>) -> String {
    tokens
        .iter()
        .enumerate()
        .filter_map(|(j, t)| if j == i { with } else { Some(*t) })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Most edits allowed between a word of `len` characters and a suggestion:
/// one more than the typo fallback allows ([`crate::fuzzy::max_edits`]).
fn allowed_edits(len: usize) -> usize {
    match len {
        0..=2 => 0,
        3 => 1,
        4..=7 => 2,
        _ => 3,
    }
}

/// The best word of the names that pass `ok` for `token` (or for it in the
/// other keyboard layout): the fewest edits, then the most names, then the
/// alphabetical first. Never `token` itself.
fn closest_word(entries: &[Entry], token: &str, ok: &(impl Fn(&Entry) -> bool + Sync)) -> Option<String> {
    let own = normalize(token);
    let mut targets: Vec<Vec<char>> = vec![own.chars().collect()];
    if let Some(alt) = crate::layout::convert_layout(token).map(|a| normalize(&a))
        && is_plain(&alt)
        && alt != own
    {
        targets.push(alt.chars().collect());
    }
    targets.retain(|t| (2..=MAX_WORD).contains(&t.len()) && !t.iter().all(char::is_ascii_digit));
    if targets.is_empty() {
        return None;
    }
    let limits: Vec<usize> = targets.iter().map(|t| allowed_edits(t.len())).collect();
    let own_chars: Vec<char> = own.chars().collect();
    // Per target: the most edits still accepted (the fewest found so far, or
    // the limit while none is found) and the words found at it, counted.
    type Best = Vec<(usize, HashMap<String, u32>)>;
    let fresh = || -> (Best, Vec<char>) { (limits.iter().map(|&l| (l, HashMap::new())).collect(), Vec::new()) };
    let (best, _) = entries
        .par_iter()
        .filter(|e| ok(e))
        .fold(fresh, |(mut best, mut chars), e| {
            for (_, w) in split_words(&e.norm_name) {
                chars.clear();
                chars.extend(w.chars());
                let have = letters(&chars);
                for (t, (d_best, words)) in targets.iter().zip(best.iter_mut()) {
                    // Each letter of the target the word lacks takes an edit.
                    if chars.len().abs_diff(t.len()) > *d_best || t.iter().filter(|&&c| have & letter_bit(c) == 0).count() > *d_best {
                        continue;
                    }
                    let Some(d) = distance(t, &chars, *d_best) else { continue };
                    if d == 0 && *t == own_chars {
                        continue;
                    }
                    if d < *d_best {
                        *d_best = d;
                        words.clear();
                    }
                    match words.get_mut(w) {
                        Some(n) => *n += 1,
                        None => {
                            words.insert(w.to_string(), 1);
                        }
                    }
                }
            }
            (best, chars)
        })
        .reduce(fresh, |(a, c), (b, _)| (merge(a, b), c));
    best.into_iter()
        .filter(|(_, words)| !words.is_empty())
        .enumerate()
        .map(|(i, (d, words))| {
            let (word, n) = words.into_iter().max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0))).unwrap_or_default();
            (d, i, std::cmp::Reverse(n), word)
        })
        .min()
        .map(|(.., w)| w)
}

/// A bit standing for the letter `c` (letters may share one: the test that
/// uses it only ever lets too much through).
fn letter_bit(c: char) -> u128 {
    1 << (c as u32 % 128)
}

/// The [`letter_bit`]s of `chars`.
fn letters(chars: &[char]) -> u128 {
    chars.iter().fold(0, |m, &c| m | letter_bit(c))
}

/// Joins two per-target bests: the fewer edits wins, equal ones add up.
fn merge(a: Vec<(usize, HashMap<String, u32>)>, b: Vec<(usize, HashMap<String, u32>)>) -> Vec<(usize, HashMap<String, u32>)> {
    a.into_iter()
        .zip(b)
        .map(|((da, mut wa), (db, wb))| match da.cmp(&db) {
            std::cmp::Ordering::Less => (da, wa),
            std::cmp::Ordering::Greater => (db, wb),
            std::cmp::Ordering::Equal => {
                for (w, n) in wb {
                    *wa.entry(w).or_insert(0) += n;
                }
                (da, wa)
            }
        })
        .collect()
}

/// Damerau (optimal string alignment) distance between `a` and `b` in whole
/// edits, if it is at most `max`.
fn distance(a: &[char], b: &[char], max: usize) -> Option<usize> {
    if a.len().abs_diff(b.len()) > max {
        return None;
    }
    let n = b.len();
    let mut prev2: Vec<usize> = vec![0; n + 1];
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut cur: Vec<usize> = vec![0; n + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        let mut row_min = cur[0];
        for j in 1..=n {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut d = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d = d.min(prev2[j - 2] + 1);
            }
            cur[j] = d;
            row_min = row_min.min(d);
        }
        if row_min > max {
            return None;
        }
        std::mem::swap(&mut prev2, &mut prev);
        std::mem::swap(&mut prev, &mut cur);
    }
    Some(prev[n]).filter(|&d| d <= max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launcher::Action;

    fn entry(path: &str) -> Entry {
        let name = path.rsplit(['\\', '/']).next().unwrap();
        Entry { path: path.into(), size: 1, mtime: 0, is_dir: false, norm_name: normalize(name).into() }
    }

    fn entries(paths: &[&str]) -> Vec<Entry> {
        paths.iter().map(|p| entry(p)).collect()
    }

    fn parse(t: &str) -> Query {
        Query::parse(t)
    }

    fn dym(es: &[Entry], text: &str, scope: &str) -> Option<String> {
        did_you_mean(es, text, scope, &parse)
    }

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn edit_distance() {
        assert_eq!(distance(&chars("report"), &chars("report"), 2), Some(0));
        assert_eq!(distance(&chars("reprot"), &chars("report"), 2), Some(1));
        assert_eq!(distance(&chars("rport"), &chars("report"), 2), Some(1));
        assert_eq!(distance(&chars("rpt"), &chars("report"), 2), None);
        assert_eq!(distance(&chars("abcd"), &chars("wxyz"), 3), None);
        assert_eq!(distance(&chars("abcd"), &chars("wxyz"), 4), Some(4));
        assert_eq!(distance(&chars("تقرير"), &chars("تقارير"), 1), Some(1));
        assert_eq!(distance(&chars(""), &chars("ab"), 2), Some(2));
    }

    #[test]
    fn spelling_of_one_word() {
        let es = entries(&[
            r"C:\docs\budget 2024.xlsx",
            r"C:\docs\budget final.xlsx",
            r"C:\docs\budgie.png",
            r"C:\docs\التقرير السنوي.docx",
            r"D:\other\invoice.pdf",
        ]);
        // Two edits from "budget", beyond the one typo the fallback allows.
        assert_eq!(dym(&es, "bdugte", "").as_deref(), Some("budget"));
        // The more common of two equally near words wins.
        assert_eq!(dym(&es, "budgxx", "").as_deref(), Some("budget"));
        // Filters are kept as typed, and limit what is suggested.
        assert_eq!(dym(&es, "bdugte ext:xlsx", "").as_deref(), Some("budget ext:xlsx"));
        assert_eq!(dym(&es, "budgxx ext:png", "").as_deref(), Some("budgie ext:png"));
        // The other terms must still match: no name has "qq" or "zz" too.
        assert_eq!(dym(&es, "bdugte qq|zz", ""), None);
        assert_eq!(dym(&es, "bdugte qq|final", "").as_deref(), Some("budget qq|final"));
        // Only words in the folder searched.
        assert_eq!(dym(&es, "invoise", r"D:\other").as_deref(), Some("invoice"));
        assert_eq!(dym(&es, "invoise", r"C:\docs"), None);
        // Arabic, normalised.
        assert_eq!(dym(&es, "السنةي", "").as_deref(), Some("السنوي"));
        // Nothing near enough, too short, numbers, filters only.
        assert_eq!(dym(&es, "zzzzzzzz", ""), None);
        assert_eq!(dym(&es, "q", ""), None);
        assert_eq!(dym(&es, "2023", ""), None);
        assert_eq!(dym(&es, "ext:docx", ""), None);
        assert_eq!(dym(&es, "", ""), None);
        assert_eq!(dym(&[], "budget", ""), None);
    }

    #[test]
    fn spelling_in_the_other_layout() {
        // "hgjrvdv" is "التقرير" typed on the English layout; "hgjrvdx" has a
        // typo on top, so the layout fallback alone finds nothing.
        let es = entries(&[r"C:\التقرير.docx", r"C:\other.txt"]);
        assert_eq!(dym(&es, "hgjrvdx", "").as_deref(), Some("التقرير"));
    }

    #[test]
    fn a_word_too_many() {
        let es = entries(&[r"C:\annual report 2024.pdf", r"C:\notes.txt"]);
        assert_eq!(dym(&es, "annual report zebra", "").as_deref(), Some("annual report"));
        // The last word typed goes first.
        assert_eq!(dym(&es, "zebra report", "").as_deref(), Some("report"));
        // Filters stay.
        assert_eq!(dym(&es, "report zebra ext:pdf", "").as_deref(), Some("report ext:pdf"));
        assert_eq!(dym(&es, "zebra lion", ""), None);
    }

    #[test]
    fn suggestion_list_and_order() {
        let all = Situation { did_you_mean: Some("budget".into()), kind_filtered: true, scoped: true, contents: true, web: true };
        assert_eq!(
            suggestions(" bdugte ", &all),
            vec![Suggestion::Query("budget".into()), Suggestion::AllKinds, Suggestion::AllDrives, Suggestion::Contents, Suggestion::Web]
        );
        // The same text (as compared for search) is no suggestion.
        let same = Situation { did_you_mean: Some("Budget".into()), ..Situation::default() };
        assert!(suggestions("budget", &same).is_empty());
        assert!(suggestions("  ", &all).is_empty());
        let web = Situation { web: true, ..Situation::default() };
        assert_eq!(suggestions("x", &web), vec![Suggestion::Web]);
        for s in suggestions("x", &all) {
            assert!(!s.label().is_empty());
        }
        assert!(Suggestion::Query("a".repeat(100)).label().chars().count() < 60);
        assert_eq!(web_url(" a b&c "), "https://www.google.com/search?q=a%20b%26c");
    }

    #[test]
    fn suggestion_rows_go_before_web_and_more() {
        let web = Row { group: Group::Web, ..Row::file(r"C:\w", false) };
        let more = Row { group: Group::More, target: Target::OpenInMain("q".into()), ..Row::file(r"C:\m", false) };
        let mut list = vec![web.clone(), more.clone()];
        assert!(nothing_found(&list));
        insert_rows(&mut list, &[Suggestion::Query("a".into()), Suggestion::Contents]);
        let groups: Vec<Group> = list.iter().map(|r| r.group).collect();
        assert_eq!(groups, vec![Group::Suggest, Group::Suggest, Group::Web, Group::More]);
        assert_eq!(list[1].target, Target::Apply(Suggestion::Contents));
        assert!(!nothing_found(&list));
        assert!(nothing_found(&[]));
        assert!(!nothing_found(&[Row::file(r"C:\f", false)]));
    }

    #[test]
    fn recent_files_checked_lazily() {
        let paths = ["a", "gone1", "b", "gone2", "c", "d"];
        let mut probed = Vec::new();
        let got = pick_recent(paths, 2, MAX_PROBES, |p| {
            probed.push(p.to_string());
            (!p.starts_with("gone")).then_some(p.len())
        });
        assert_eq!(got, vec![("a".to_string(), 1), ("b".to_string(), 1)]);
        // Stops as soon as it has enough.
        assert_eq!(probed, vec!["a", "gone1", "b"]);
        // And never probes more than allowed.
        let got = pick_recent(["gone1", "gone2", "a"], 5, 2, |p| (!p.starts_with("gone")).then_some(()));
        assert!(got.is_empty());
    }

    #[test]
    fn drive_letters() {
        assert_eq!(drive_letter(r"c:\x\a.txt"), Some('C'));
        assert_eq!(drive_letter("D:/a"), Some('D'));
        for p in [r"\\server\share\a", "//server/a", "a.txt", "C:", "C:a", "1:\\a", ""] {
            assert_eq!(drive_letter(p), None, "{p}");
        }
    }

    #[test]
    fn empty_box_lists_favourites_then_recent() {
        let pin_app = Row {
            group: Group::Pinned,
            title: "Word".into(),
            subtitle: String::new(),
            target: Target::Launcher(Action::Launch { target: r"C:\Word.lnk".into(), args: String::new() }),
            icon: Icon::Glyph('x'),
        };
        let pin_file = Row::file(r"C:\docs\a.txt", false);
        let recent: Vec<Row> = (0..12).map(|i| Row::file(&format!(r"C:\docs\r{i}.txt"), false)).collect();
        let mut with_dup = vec![Row::file(r"C:\DOCS\A.TXT", false)];
        with_dup.extend(recent);
        let rows = empty_rows(vec![pin_app, pin_file], with_dup);
        let groups: Vec<Group> = rows.iter().map(|r| r.group).collect();
        assert_eq!(groups[..2], [Group::Pinned, Group::Pinned]);
        assert!(groups[2..].iter().all(|g| *g == Group::Recent));
        // The pinned file is not listed again; the recent list is capped.
        assert_eq!(rows.len(), 2 + RECENT_ROWS);
        assert_eq!(rows[2].title, "r0.txt");
        let many: Vec<Row> = (0..10).map(|i| Row::file(&format!(r"C:\p{i}"), false)).collect();
        assert_eq!(empty_rows(many, Vec::new()).len(), FAVOURITE_ROWS);
        assert!(empty_rows(Vec::new(), Vec::new()).is_empty());
    }
}
