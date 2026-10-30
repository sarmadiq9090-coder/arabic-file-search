//! Typo-tolerant name search (`تقريب:`, and the last fallback when nothing
//! matches), root search (`جذر:`), and the chain of fallbacks tried when a
//! name search finds nothing: keyboard layout, then Arabizi, then typos.
//!
//! A term of 4–7 letters may match a word of the name (or the start of one)
//! with one edit, a term of 8 or more with two; shorter terms stay exact.
//! Edits are Damerau-style (insert, delete, substitute, swap two neighbours)
//! on normalised text, and letters Arabic writers often confuse (ض/ظ، ذ/ز،
//! ث/س، ص/س، ط/ت، ق/ك، ح/ه and the hamza seats) cost half an edit.

// Only the Windows UI and the tests use this module.

use rayon::prelude::*;
use std::sync::LazyLock;

use crate::index::Entry;
use crate::search::{self, FRANCO_PREFIXES, FUZZY_PREFIXES, Query, Term, in_scope, split_words};
use crate::{franco, layout, root};

/// Costs in half edits, so that a confusion is 1 and the sums stay integers.
const EDIT: u8 = 2;
const CONFUSION: u8 = 1;

/// Letter pairs that cost half an edit (normalised forms: ؤ is و, ئ is ي, أ is ا).
const CONFUSABLE: &[(char, char)] = &[
    ('ض', 'ظ'),
    ('ذ', 'ز'),
    ('ث', 'س'),
    ('ط', 'ت'),
    ('ص', 'س'),
    ('ق', 'ك'),
    ('ح', 'ه'),
    ('ء', 'و'),
    ('ء', 'ي'),
    ('ء', 'ا'),
];

fn sub_cost(a: char, b: char) -> u8 {
    if a == b {
        0
    } else if is_arabic(a) && is_arabic(b) && name_bits(a) & bit(b) != 0 {
        // Arabic letters have a bit each, and name_bits adds the partners'.
        CONFUSION
    } else {
        EDIT
    }
}

fn is_arabic(c: char) -> bool {
    ('\u{0620}'..='\u{064A}').contains(&c)
}

/// Most whole edits allowed for a term of `len` characters.
pub fn max_edits(len: usize) -> u8 {
    match len {
        0..=3 => 0,
        4..=7 => 1,
        _ => 2,
    }
}

/// The bit of a character for the shared-letter pre-filter.
fn bit(c: char) -> u128 {
    let n = match c {
        '\u{0620}'..='\u{064A}' => c as u32 - 0x620,
        'a'..='z' => 43 + (c as u32 - 'a' as u32),
        '0'..='9' => 69 + (c as u32 - '0' as u32),
        _ => 79 + c as u32 % 49,
    };
    1 << n
}

/// The bits of the characters of a term.
fn mask(s: &str) -> u128 {
    s.chars().fold(0, |m, c| m | bit(c))
}

/// [`name_bits`] of the Arabic letters U+0620..=U+064A.
static ARABIC_BITS: LazyLock<[u128; 43]> = LazyLock::new(|| {
    std::array::from_fn(|i| {
        let c = char::from_u32(0x620 + i as u32).unwrap_or(' ');
        CONFUSABLE.iter().fold(bit(c), |m, &(x, y)| match () {
            _ if c == x => m | bit(y),
            _ if c == y => m | bit(x),
            _ => m,
        })
    })
});

/// The bits a character of a name provides: its own and those of the letters
/// it is confused with, so that a confusion never fails the pre-filter.
fn name_bits(c: char) -> u128 {
    match c {
        '\u{0620}'..='\u{064A}' => ARABIC_BITS[(c as u32 - 0x620) as usize],
        c => bit(c),
    }
}

/// [`search::ARTICLES`] as characters.
const ARTICLE_CHARS: [&[char]; 6] =
    [&['و', 'ا', 'ل'], &['ب', 'ا', 'ل'], &['ك', 'ا', 'ل'], &['ف', 'ا', 'ل'], &['ل', 'ل'], &['ا', 'ل']];

/// Longest term matched with typos; longer ones stay exact.
const MAX_TERM: usize = 48;

/// Damerau (optimal string alignment) distance in half edits between `t`
/// and the closest prefix of `w`, if it is at most `max` (at most 2 whole
/// edits). Only the band of cells within `max` of the diagonal is worked out.
fn prefix_distance(t: &[char], w: &[char], max: u8) -> Option<u8> {
    const W: usize = MAX_TERM + 4;
    let k = usize::from(max / EDIT);
    let m = t.len();
    if m > MAX_TERM || k > 2 || w.len() + k < m {
        return None;
    }
    let w = &w[..w.len().min(m + k)];
    let n = w.len();
    let mut rows = [[u8::MAX; W]; 3];
    for (j, d) in rows[1].iter_mut().enumerate().take(k.min(n) + 1) {
        *d = j as u8 * EDIT;
    }
    // rows[p2], rows[p1], rows[c]: rows i-2, i-1 and i.
    let (mut p2, mut p1, mut c) = (0, 1, 2);
    for i in 1..=m {
        rows[c] = [u8::MAX; W];
        if i <= k {
            rows[c][0] = i as u8 * EDIT;
        }
        let mut row_min = rows[c][0];
        for j in i.saturating_sub(k).max(1)..=(i + k).min(n) {
            let (prev, cur) = (&rows[p1], &rows[c]);
            let mut d = prev[j].saturating_add(EDIT).min(cur[j - 1].saturating_add(EDIT));
            d = d.min(prev[j - 1].saturating_add(sub_cost(t[i - 1], w[j - 1])));
            if i > 1 && j > 1 && t[i - 1] == w[j - 2] && t[i - 2] == w[j - 1] {
                d = d.min(rows[p2][j - 2].saturating_add(EDIT));
            }
            rows[c][j] = d;
            row_min = row_min.min(d);
        }
        if row_min > max {
            return None;
        }
        (p2, p1, c) = (p1, c, p2);
    }
    rows[p1][..=n].iter().copied().min().filter(|&d| d <= max)
}

/// A term alternative prepared for typo-tolerant matching.
struct Alt {
    chars: Vec<char>,
    /// Largest distance, in half edits.
    max: u8,
    mask: u128,
    /// Fewest bits of `mask` a matching text must have.
    need: u32,
}

impl Alt {
    fn new(a: &str) -> Option<Alt> {
        let chars: Vec<char> = a.chars().collect();
        let edits = max_edits(chars.len());
        let mask = mask(a);
        // Each whole edit removes at most one letter of the term (a confusion keeps its bit).
        let need = mask.count_ones().saturating_sub(u32::from(edits));
        (edits > 0).then_some(Alt { chars, max: edits * EDIT, mask, need })
    }

    /// The shared-letter test: false when a text with letters `m` can't match.
    fn possible(&self, m: u128) -> bool {
        (self.mask & m).count_ones() >= self.need
    }
}

/// How one term matches a name beyond containing one of its alternatives.
enum Matcher {
    /// Only exactly.
    Exact,
    Fuzzy(Vec<Alt>),
    /// Roots of the Arabic alternatives.
    Root(Vec<String>),
}

impl Matcher {
    fn new(t: &Term, q: &Query) -> Matcher {
        if q.root {
            let roots: Vec<String> = t.alts.iter().filter_map(|a| root::root(a)).collect();
            if !roots.is_empty() {
                return Matcher::Root(roots);
            }
        }
        if !q.fuzzy {
            return Matcher::Exact;
        }
        let alts: Vec<Alt> = t
            .alts
            .iter()
            .filter(|a| !a.contains(char::is_whitespace) && !a.bytes().all(|b| b.is_ascii_digit()))
            .filter(|a| a.chars().count() <= MAX_TERM)
            .filter_map(|a| Alt::new(a))
            .collect();
        if alts.is_empty() { Matcher::Exact } else { Matcher::Fuzzy(alts) }
    }

    /// Cost (in half edits) of the best word of the name loaded in `s`, if one matches.
    fn cost(&self, s: &mut Scratch) -> Option<u8> {
        match self {
            Matcher::Exact => None,
            Matcher::Fuzzy(alts) => {
                if !alts.iter().any(|a| a.possible(s.mask)) {
                    return None;
                }
                let mut best: Option<u8> = None;
                for &(start, end, bits) in &s.words {
                    let word = &s.chars[start..end];
                    let bare = ARTICLE_CHARS.iter().find(|p| word.starts_with(p)).map(|p| &word[p.len()..]);
                    let bare = bare.filter(|w| w.len() >= 3);
                    for a in alts.iter().filter(|a| a.possible(bits)) {
                        for form in std::iter::once(word).chain(bare) {
                            if let Some(d) = prefix_distance(&a.chars, form, a.max) {
                                best = Some(best.map_or(d, |b| b.min(d)));
                            }
                        }
                    }
                }
                best
            }
            Matcher::Root(roots) => s
                .words
                .iter()
                .any(|&(start, end, _)| {
                    let word = &s.chars[start..end];
                    roots.iter().any(|r| {
                        let mut letters = word.iter();
                        r.chars().all(|c| letters.any(|&l| l == c))
                            && root::root(&word.iter().collect::<String>()).as_ref() == Some(r)
                    })
                })
                .then_some(1),
        }
    }
}

/// Same as `char::is_alphanumeric` (what [`split_words`] uses), much faster
/// for Arabic letters, which are all alphanumeric.
fn is_word_char(c: char) -> bool {
    match c {
        _ if c.is_ascii() => c.is_ascii_alphanumeric(),
        '\u{0621}'..='\u{064A}' => true,
        _ => c.is_alphanumeric(),
    }
}

/// Per-thread buffer: the name being checked, split into words.
#[derive(Default)]
struct Scratch {
    chars: Vec<char>,
    /// (start, end, [`name_bits`] of the letters) of each word in `chars`.
    words: Vec<(usize, usize, u128)>,
    /// All the words' bits.
    mask: u128,
}

impl Scratch {
    /// Decodes `name` and splits it into words as [`split_words`] does.
    fn load(&mut self, name: &str) {
        self.chars.clear();
        self.words.clear();
        self.mask = 0;
        let mut word: Option<(usize, u128)> = None;
        for c in name.chars() {
            let i = self.chars.len();
            self.chars.push(c);
            if is_word_char(c) {
                let (_, bits) = word.get_or_insert((i, 0));
                *bits |= name_bits(c);
            } else if let Some((start, bits)) = word.take() {
                self.words.push((start, i, bits));
                self.mask |= bits;
            }
        }
        if let Some((start, bits)) = word {
            self.words.push((start, self.chars.len(), bits));
            self.mask |= bits;
        }
    }
}

/// The total cost of `name` against all terms, if every term matches it.
fn name_cost(q: &Query, matchers: &[Matcher], name: &str, s: &mut Scratch) -> Option<u32> {
    let terms = || q.terms.iter().zip(matchers);
    // Exact terms first: they are cheap and reject most names.
    if terms().any(|(t, m)| matches!(m, Matcher::Exact) && !t.matches(name)) {
        return None;
    }
    let mut total = 0;
    let mut loaded = false;
    for (t, m) in terms().filter(|(_, m)| !matches!(m, Matcher::Exact)) {
        if t.matches(name) {
            continue;
        }
        if !loaded {
            s.load(name);
            loaded = true;
        }
        total += u32::from(m.cost(s)?);
    }
    Some(total)
}

/// Like [`search::search_names`], but each term may also match a word with
/// a typo (`fuzzy:`) or the same root (`root:`), as set on `q`. Ranked by
/// total distance, then as [`search::search_names`] ranks.
pub fn fuzzy_search_names(entries: &[Entry], q: &Query, scope: &str) -> Vec<u32> {
    if q.is_empty() {
        return Vec::new();
    }
    let scope_lower = scope.to_lowercase();
    let matchers: Vec<Matcher> = q.terms.iter().map(|t| Matcher::new(t, q)).collect();
    let mut hits: Vec<(u32, u8, u32, u32)> = entries
        .par_iter()
        .enumerate()
        .filter(|(_, e)| q.entry_ok(e) && in_scope(&e.path, &scope_lower))
        .map_init(Scratch::default, |s, (i, e)| {
            let cost = name_cost(q, &matchers, &e.norm_name, s)?;
            Some((cost, search::name_rank(q, &e.norm_name), e.norm_name.chars().count() as u32, i as u32))
        })
        .flatten()
        .collect();
    hits.par_sort_unstable();
    hits.into_iter().map(|(.., i)| i).collect()
}

/// Byte ranges of `norm` (a normalised name) of the words that a term
/// matches only with a typo or by root, for underlining.
pub fn word_spans(norm: &str, q: &Query) -> Vec<(usize, usize)> {
    let mut s = Scratch::default();
    let mut spans = Vec::new();
    for t in q.terms.iter().filter(|t| !t.matches(norm)) {
        let m = Matcher::new(t, q);
        for (off, w) in split_words(norm) {
            s.load(w);
            if m.cost(&mut s).is_some() {
                spans.push((off, off + w.len()));
            }
        }
    }
    spans
}

/// Which fallback found the results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackKind {
    /// The query re-typed in the other keyboard layout.
    Layout,
    /// Latin words read as Arabizi.
    Franco,
    /// Words with typos.
    Fuzzy,
}

/// How a fallback search found its results.
#[derive(Debug, Clone, PartialEq)]
pub struct Fallback {
    pub kind: FallbackKind,
    /// The query that found them: parse it to underline the matches.
    pub query: String,
    /// What was searched for, to show the user.
    pub shown: String,
}

impl Fallback {
    /// The start of the status line for `typed`, the query as typed.
    pub fn status(&self, typed: &str) -> String {
        match self.kind {
            FallbackKind::Fuzzy => trf!(
                "لا نتائج مطابقة تماماً؛ عرض نتائج مقاربة لـ «{}»",
                "No exact results; showing close matches for “{}”",
                typed.trim()
            ),
            _ => trf!("لا نتائج لـ «{}»، عرض نتائج «{}»", "No results for “{}”; showing results for “{}”", typed.trim(), self.shown),
        }
    }
}

/// For a name search of `text` that found nothing: tries the query typed in
/// the other keyboard layout, then (for Latin words) as Arabizi, then with
/// typos allowed, and returns the first hits found with how they were found.
/// `parse` turns a query text into a [`Query`] (with the type filter applied).
pub fn name_fallback(
    entries: &[Entry],
    text: &str,
    scope: &str,
    parse: &impl Fn(&str) -> Query,
) -> Option<(Vec<u32>, Fallback)> {
    let q = parse(text);
    if q.is_empty() {
        return None;
    }
    let found = |hits: Vec<u32>, kind, query: String, shown: String| {
        (!hits.is_empty()).then_some((hits, Fallback { kind, query, shown }))
    };
    if let Some(alt) = layout::convert_layout(text) {
        let hits = search::search_names(entries, &parse(&alt), scope);
        if let Some(r) = found(hits, FallbackKind::Layout, alt.clone(), alt) {
            return Some(r);
        }
    }
    if !q.franco && franco::worth_trying(&q) {
        let query = format!("{} {text}", FRANCO_PREFIXES[1]);
        let fq = parse(&query);
        let hits = search::search_names(entries, &fq, scope);
        let shown = hits.first().map(|&i| franco_shown(&fq, &entries[i as usize].norm_name)).unwrap_or_default();
        if let Some(r) = found(hits, FallbackKind::Franco, query, shown) {
            return Some(r);
        }
    }
    let fuzzy_terms = q.terms.iter().any(|t| t.alts.iter().any(|a| max_edits(a.chars().count()) > 0));
    if !q.fuzzy && !q.root && fuzzy_terms {
        let query = format!("{} {text}", FUZZY_PREFIXES[1]);
        let hits = search::search_names(entries, &parse(&query), scope);
        if let Some(r) = found(hits, FallbackKind::Fuzzy, query, text.trim().to_string()) {
            return Some(r);
        }
    }
    None
}

/// The Arabic spelling of each term that `name` (the best hit) contains.
fn franco_shown(q: &Query, name: &str) -> String {
    let pick = |t: &Term| {
        let arabic = || t.alts.iter().filter(|a| !a.is_ascii());
        arabic().find(|a| name.contains(a.as_str())).or_else(|| arabic().next()).unwrap_or(&t.alts[0]).clone()
    };
    q.terms.iter().map(pick).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normalize::normalize;

    fn entry(path: &str) -> Entry {
        let name = path.rsplit(['\\', '/']).next().unwrap();
        Entry { path: path.into(), size: 1, mtime: 0, is_dir: false, norm_name: normalize(name).into() }
    }

    fn entries(names: &[&str]) -> Vec<Entry> {
        names.iter().map(|n| entry(&format!(r"C:\d\{n}"))).collect()
    }

    fn found<'a>(es: &'a [Entry], q: &str) -> Vec<&'a str> {
        fuzzy_search_names(es, &Query::parse(&format!("fuzzy: {q}")), "").into_iter().map(|i| es[i as usize].name()).collect()
    }

    fn dist(a: &str, b: &str) -> Option<u8> {
        let (a, b): (Vec<char>, Vec<char>) = (normalize(a).chars().collect(), normalize(b).chars().collect());
        prefix_distance(&a, &b, 4)
    }

    #[test]
    fn article_chars_agree() {
        let chars: Vec<String> = ARTICLE_CHARS.iter().map(|a| a.iter().collect()).collect();
        assert_eq!(chars, search::ARTICLES);
    }

    #[test]
    fn prefilter_keeps_confusions() {
        for (a, b) in CONFUSABLE {
            assert_ne!(name_bits(*a) & bit(*b), 0, "{a}{b}");
            assert_ne!(name_bits(*b) & bit(*a), 0, "{a}{b}");
        }
        assert_eq!(name_bits('ب'), bit('ب'));
        assert_eq!(name_bits('x'), bit('x'));
    }

    #[test]
    fn word_chars_agree() {
        for c in ('\u{0}'..='\u{07FF}').chain('\u{FB50}'..='\u{FEFF}') {
            assert_eq!(is_word_char(c), c.is_alphanumeric(), "{c:?}");
        }
    }

    #[test]
    fn confusion_table() {
        let letters = ('\u{0620}'..='\u{064A}').chain(['a', 'z', '0', ' ']);
        for x in letters.clone() {
            for y in letters.clone() {
                let listed = CONFUSABLE.iter().any(|&p| p == (x, y) || p == (y, x));
                let want = if x == y { 0 } else if listed { CONFUSION } else { EDIT };
                assert_eq!(sub_cost(x, y), want, "{x}{y}");
            }
        }
    }

    #[test]
    fn distances() {
        assert_eq!(dist("report", "report"), Some(0));
        assert_eq!(dist("reprot", "report"), Some(2)); // a swap is one edit
        assert_eq!(dist("repot", "report"), Some(2));
        assert_eq!(dist("reeport", "report"), Some(2));
        assert_eq!(dist("rexort", "report"), Some(2));
        assert_eq!(dist("rxpxrt", "report"), Some(4));
        assert_eq!(dist("rxxxrt", "report"), None);
        // The rest of the word is free: the term may be its start.
        assert_eq!(dist("reprot", "reporting"), Some(2));
        assert_eq!(dist("report", "rep"), None);
    }

    #[test]
    fn arabic_confusions_cost_half() {
        assert_eq!(dist("ضابط", "ظابط"), Some(1));
        assert_eq!(dist("زكرى", "ذكرى"), Some(1));
        assert_eq!(dist("سورة", "ثورة"), Some(1));
        assert_eq!(dist("صورة", "سورة"), Some(1));
        assert_eq!(dist("تالب", "طالب"), Some(1));
        assert_eq!(dist("كلب", "قلب"), Some(1));
        assert_eq!(dist("هسن", "حسن"), Some(1));
        assert_eq!(dist("مسءول", "مسؤول"), Some(1));
        assert_eq!(dist("قاءمة", "قائمة"), Some(1));
        assert_eq!(dist("سأل", "ساءل"), Some(2));
        // Unrelated letters cost a whole edit.
        assert_eq!(dist("كلب", "جلب"), Some(2));
        // Two confusions fit in one edit.
        assert_eq!(dist("ظابت", "ضابط"), Some(2));
    }

    #[test]
    fn thresholds_by_length() {
        assert_eq!(max_edits(3), 0);
        assert_eq!(max_edits(4), 1);
        assert_eq!(max_edits(7), 1);
        assert_eq!(max_edits(8), 2);
        let es = entries(&["report.pdf", "presentation.pptx", "cat.txt", "التقرير السنوي.docx", "مسؤول.txt"]);
        assert_eq!(found(&es, "reprot"), vec!["report.pdf"]);
        assert_eq!(found(&es, "rexorx"), Vec::<&str>::new()); // two edits on 6 letters
        assert_eq!(found(&es, "presentatoin"), vec!["presentation.pptx"]);
        assert_eq!(found(&es, "prisentatoin"), vec!["presentation.pptx"]); // two edits on 12
        assert_eq!(found(&es, "prisentatoan"), Vec::<&str>::new()); // three
        assert_eq!(found(&es, "تقريز"), vec!["التقرير السنوي.docx"]); // the article is optional
        assert_eq!(found(&es, "السنوى تقرر"), vec!["التقرير السنوي.docx"]);
        assert_eq!(found(&es, "مسءول"), vec!["مسؤول.txt"]);
    }

    #[test]
    fn short_terms_stay_exact() {
        let es = entries(&["cat.txt", "car.txt", "باب.txt", "2024.txt"]);
        assert_eq!(found(&es, "cat"), vec!["cat.txt"]);
        assert_eq!(found(&es, "cut"), Vec::<&str>::new());
        assert_eq!(found(&es, "بات"), Vec::<&str>::new());
        assert_eq!(found(&es, "2025"), Vec::<&str>::new()); // numbers are never fuzzy
        assert_eq!(found(&es, "ca"), vec!["cat.txt", "car.txt"]);
    }

    #[test]
    fn ranked_by_distance_then_usual_order() {
        let es = entries(&["xx ظابط.txt", "ضابطة.txt", "ضابط.txt", "صابط.txt", "زابط.txt"]);
        // Exact first, then the confusion (half), then a whole edit.
        assert_eq!(found(&es, "ضابط"), vec!["ضابط.txt", "ضابطة.txt", "xx ظابط.txt", "صابط.txt", "زابط.txt"]);
    }

    #[test]
    fn filters_still_apply() {
        let es = entries(&["report.pdf", "report.docx", "old report.pdf"]);
        let q = Query::parse("fuzzy: reprot ext:pdf !old");
        let hits: Vec<&str> = fuzzy_search_names(&es, &q, "").into_iter().map(|i| es[i as usize].name()).collect();
        assert_eq!(hits, vec!["report.pdf"]);
        // Through search_names too.
        assert_eq!(search::search_names(&es, &q, "").len(), 1);
    }

    #[test]
    fn root_search() {
        let es = entries(&["مكتبة الجامعة.pdf", "الكاتب.docx", "كتابات قديمة.txt", "كبت.txt", "تقرير.pdf"]);
        let q = Query::parse("جذر: كتاب");
        let hits: Vec<&str> = search::search_names(&es, &q, "").into_iter().map(|i| es[i as usize].name()).collect();
        assert_eq!(hits, vec!["كتابات قديمة.txt", "الكاتب.docx", "مكتبة الجامعة.pdf"]);
        assert_eq!(highlighted("مكتبة الجامعة.pdf", "جذر: كتاب"), vec!["مكتبة"]);
    }

    fn highlighted<'a>(name: &'a str, q: &str) -> Vec<&'a str> {
        search::match_ranges(name, &Query::parse(q)).into_iter().map(|(s, e)| &name[s..e]).collect()
    }

    #[test]
    fn underlines_the_matched_word() {
        assert_eq!(highlighted("التقرير السنوي.pdf", "تقريب: تقريز"), vec!["التقرير"]);
        assert_eq!(highlighted("my report 2024.pdf", "fuzzy: reprot 2024"), vec!["report", "2024"]);
        assert_eq!(highlighted("ظابط.txt", "fuzzy: ضابط"), vec!["ظابط"]);
        assert!(highlighted("report.pdf", "reprot").is_empty()); // not a fuzzy query
    }

    fn fallback<'a>(es: &'a [Entry], text: &str) -> Option<(Vec<&'a str>, FallbackKind, String)> {
        let (hits, fb) = name_fallback(es, text, "", &|t: &str| Query::parse(t))?;
        Some((hits.into_iter().map(|i| es[i as usize].name()).collect(), fb.kind, fb.shown))
    }

    #[test]
    fn fallback_order() {
        let es = entries(&["التقرير.pdf", "محمد علي.docx", "عبد الله.txt", "report.pdf", "حاجة.txt"]);
        // Wrong keyboard layout comes first.
        assert_eq!(fallback(&es, "hgjrvdv ext:docx").map(|r| r.1), None);
        let (hits, kind, shown) = fallback(&es, "hgjrvdv").unwrap();
        assert_eq!((hits, kind, shown.as_str()), (vec!["التقرير.pdf"], FallbackKind::Layout, "التقرير"));
        // Then Arabizi.
        let (hits, kind, shown) = fallback(&es, "mohammed").unwrap();
        assert_eq!((hits, kind, shown.as_str()), (vec!["محمد علي.docx"], FallbackKind::Franco, "محمد"));
        let (hits, kind, _) = fallback(&es, "3abd").unwrap();
        assert_eq!((hits, kind), (vec!["عبد الله.txt"], FallbackKind::Franco));
        assert_eq!(fallback(&es, "7aga").unwrap().0, vec!["حاجة.txt"]);
        // Then typos.
        let (hits, kind, shown) = fallback(&es, "reprot").unwrap();
        assert_eq!((hits, kind, shown.as_str()), (vec!["report.pdf"], FallbackKind::Fuzzy, "reprot"));
        let (hits, kind, _) = fallback(&es, "تقريز").unwrap();
        assert_eq!((hits, kind), (vec!["التقرير.pdf"], FallbackKind::Fuzzy));
        // Nothing close.
        assert_eq!(fallback(&es, "xyzzyq"), None);
        assert_eq!(fallback(&es, "zzz"), None);
        assert_eq!(fallback(&es, ""), None);
    }

    #[test]
    fn fallback_status_lines() {
        let fb = Fallback { kind: FallbackKind::Fuzzy, query: "تقريب: reprot".into(), shown: "reprot".into() };
        assert_eq!(fb.status(" reprot "), "لا نتائج مطابقة تماماً؛ عرض نتائج مقاربة لـ «reprot»");
        let fb = Fallback { kind: FallbackKind::Franco, query: "فرانكو: 3abd".into(), shown: "عبد".into() };
        assert_eq!(fb.status("3abd"), "لا نتائج لـ «3abd»، عرض نتائج «عبد»");
    }

    #[test]
    fn franco_prefix() {
        let es = entries(&["محمد.docx", "تقرير المبيعات.xlsx", "mohammed cv.pdf", "منى.txt"]);
        let names = |q: &str| -> Vec<&str> {
            search::search_names(&es, &Query::parse(q), "").into_iter().map(|i| es[i as usize].name()).collect()
        };
        assert_eq!(names("franco: mohammed"), vec!["mohammed cv.pdf", "محمد.docx"]);
        assert_eq!(names("فرانكو: ta2rer ext:xlsx"), vec!["تقرير المبيعات.xlsx"]);
        assert_eq!(names("فرانكو:taqrer"), vec!["تقرير المبيعات.xlsx"]);
        assert_eq!(names("ta2rer"), Vec::<&str>::new()); // only with the prefix
        let q = Query::parse("franco: 3abd 2024");
        assert_eq!(q.terms[1].alts, vec!["2024"]);
        assert!(q.franco && q.terms[0].alts.contains(&"عبد".to_string()));
        assert!(Query::parse("franco: 3abd").terms[0].alts.len() <= 1 + franco::MAX_CANDIDATES * 2);
    }

    #[test]
    #[ignore = "benchmark: cargo test --release -- --ignored --nocapture"]
    fn fuzzy_two_million() {
        let words = ["تقرير", "مشروع", "فاتورة", "report", "invoice", "صورة", "ملف", "budget", "الميزانية", "عقد"];
        let es: Vec<Entry> = (0..2_000_000u32)
            .map(|i| {
                let (a, b) = (words[i as usize % 10], words[(i as usize / 10) % 10]);
                entry(&format!(r"C:\Users\Saif\Documents\مشروع {}\{a} {b} {}.txt", i % 977, i.wrapping_mul(2654435761)))
            })
            .collect();
        for q in ["report", "ملف"] {
            let t = std::time::Instant::now();
            let n = search::search_names(&es, &Query::parse(q), "").len();
            println!("exact {q}: {n} hits in {:?}", t.elapsed());
        }
        for q in ["reprot", "فاتوره", "تقريز", "الميزانيه", "invoce budgte", "ظورة", "xyzzyq", "presentatoin"] {
            let t = std::time::Instant::now();
            let n = found(&es, q).len();
            println!("fuzzy {q}: {n} hits in {:?}", t.elapsed());
        }
    }
}
