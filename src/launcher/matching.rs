//! Matching and scoring of apps and settings pages for [`super::Catalog::query`].
//!
//! Names, aliases, titles and keywords are normalised once ([`Index::build`]),
//! with the byte offsets where their words start, so a query only compares
//! strings. Tiers, best first: the whole name, the start of the name, the
//! start of a word, every query word starting a word somewhere, anywhere in
//! the name. Then, for what that missed: the query in the other keyboard
//! layout, Arabic spellings of Latin names by sound (exact, then with one
//! sound off), Arabizi spellings, and finally one or two typos.

use super::Context;
use super::apps::App;
use super::phonetic;
use super::settings_pages::Page;
use crate::normalize::normalize;
use crate::{franco, fuzzy, layout};

const EXACT: u32 = 1000;
const PREFIX: u32 = 900;
const WORD_START: u32 = 800;
const ALL_WORDS_EXACT: u32 = 700;
const ALL_WORDS: u32 = 600;
const SUBSTRING: u32 = 500;
const WORD_SUBSTRING: u32 = 350;
const SOUNDS_LIKE: u32 = 450;
/// The name's own words said in Arabic, four sound classes at least
/// ("نتفلكس"): as good as the start of the name, so it counts as a strong
/// match ([`super::STRONG_SCORE`]).
const SOUNDS_LIKE_EXACT: u32 = 860;
/// Above a web shortcut ([`super::web::SHORTCUT_SCORE`]): "جوجل بلاي" is
/// the app rather than a Google search for "بلاي".
const SOUNDS_LIKE_WORDS: u32 = 920;
/// One sound class off ("ديسكود" for Discord): below every exact sound match.
const SOUNDS_LIKE_SLIP: u32 = 400;
const TYPO: u32 = 300;

/// What a piece of text is, which sets how much a match in it counts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Field {
    Name,
    Alias,
    TitleAr,
    TitleEn,
    Keyword,
    Section,
}

impl Field {
    /// Weight in percent; the title in the interface language counts most.
    fn weight(self, english: bool) -> u32 {
        match self {
            Field::Name => 100,
            Field::Alias => 95,
            Field::TitleAr if !english => 100,
            Field::TitleEn if english => 100,
            Field::TitleAr | Field::TitleEn => 92,
            Field::Keyword => 85,
            Field::Section => 40,
        }
    }

    /// Names worth matching with typos (not keywords or sections).
    fn is_name(self) -> bool {
        matches!(self, Field::Name | Field::Alias | Field::TitleAr | Field::TitleEn)
    }
}

/// The bit of a character for the shared-letter pre-filter: Arabic letters,
/// Latin letters and digits have their own, anything else shares the rest.
fn bit(c: char) -> u128 {
    let n = match c {
        '\u{0620}'..='\u{064A}' => c as u32 - 0x620,
        'a'..='z' => 43 + (c as u32 - 'a' as u32),
        '0'..='9' => 69 + (c as u32 - '0' as u32),
        _ => 79 + c as u32 % 49,
    };
    1 << n
}

fn mask(s: &str) -> u128 {
    s.chars().filter(|c| *c != ' ').fold(0, |m, c| m | bit(c))
}

/// `w` without a leading Arabic article, when two letters or more remain.
fn strip_article(w: &str) -> Option<&str> {
    w.strip_prefix("ال").filter(|r| r.chars().count() >= 2)
}

/// Whether the bits of `need` are all in `have`.
fn has(have: u128, need: u128) -> bool {
    have & need == need
}

/// One normalised text and the offsets where its words start, including the
/// offset after an Arabic article ("الشاشه" also starts a word at "شاشه").
#[derive(Debug)]
struct Text {
    field: Field,
    text: String,
    /// Byte offsets of the word starts in `text`.
    starts: Vec<u32>,
    /// The same starts as character offsets into `chars`, for typos.
    char_starts: Vec<u32>,
    chars: Vec<char>,
    mask: u128,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
}

impl Text {
    fn new(field: Field, raw: &str) -> Text {
        let text = normalize(raw).split_whitespace().collect::<Vec<_>>().join(" ");
        let mut starts = Vec::new();
        let mut char_starts = Vec::new();
        let mut prev_word = false;
        for (ci, (i, c)) in text.char_indices().enumerate() {
            let w = is_word_char(c);
            if w && !prev_word {
                starts.push(i as u32);
                char_starts.push(ci as u32);
                if let Some(after) = text[i..].strip_prefix("ال")
                    && after.chars().take(2).filter(|c| is_word_char(*c)).count() == 2
                {
                    starts.push((i + "ال".len()) as u32);
                    char_starts.push(ci as u32 + 2);
                }
            }
            prev_word = w;
        }
        let chars = text.chars().collect();
        let mask = mask(&text);
        Text { field, text, starts, char_starts, chars, mask }
    }

    /// Whether `w` ends a word when it is found at byte `at`.
    fn ends_word(&self, at: usize, w: &str) -> bool {
        !self.text[at + w.len()..].chars().next().is_some_and(is_word_char)
    }

    /// Score of `w` (with mask `wmask`) against the words of this text: a
    /// whole word, the start of a word, or (for words of 3+ letters) anywhere.
    fn word_score(&self, w: &str, wmask: u128) -> u32 {
        if !has(self.mask, wmask) {
            return 0;
        }
        let mut best = 0;
        for &s in &self.starts {
            let s = s as usize;
            if self.text[s..].starts_with(w) {
                if self.ends_word(s, w) {
                    return ALL_WORDS_EXACT;
                }
                best = ALL_WORDS;
            }
        }
        if best == 0 && w.chars().count() >= 3 && self.text.contains(w) {
            best = WORD_SUBSTRING;
        }
        best
    }

    /// Score of the whole query `q` (with mask `qmask`) against this text.
    fn full_score(&self, q: &str, qmask: u128) -> u32 {
        if !has(self.mask, qmask) {
            0
        } else if self.text == q {
            EXACT
        } else if self.text.strip_prefix("ال") == Some(q) {
            // "شاشه" is the whole of "الشاشه".
            EXACT * 97 / 100
        } else if self.text.starts_with(q) {
            PREFIX
        } else if self.starts.iter().any(|&s| self.text[s as usize..].starts_with(q)) {
            WORD_START
        } else if q.chars().count() >= 2 && self.text.contains(q) {
            SUBSTRING
        } else {
            0
        }
    }
}

/// The texts of one app or page.
#[derive(Debug, Default)]
struct Entry {
    texts: Vec<Text>,
    /// Percent the entry's scores count: classic tools come just after the
    /// settings page that does the same.
    percent: u32,
    /// All the texts' characters, to skip the entry at once.
    mask: u128,
    /// Sound skeletons of the Latin name's words, for Arabic spellings.
    sounds: Vec<String>,
}

impl Entry {
    fn new(texts: Vec<Text>, percent: u32, latin_name: &str) -> Entry {
        let mask = texts.iter().fold(0, |m, t| m | t.mask);
        Entry { texts, percent, mask, sounds: phonetic::name_sounds(latin_name) }
    }
}

/// Normalised texts of a catalog's apps and settings, in the same order.
#[derive(Debug, Default)]
pub struct Index {
    apps: Vec<Entry>,
    settings: Vec<Entry>,
    /// Names the index was built from, to notice a catalog changed after.
    app_names: Vec<String>,
    setting_uris: Vec<&'static str>,
}

/// Words that only say what kind of thing is wanted ("إعدادات الشاشة",
/// "برنامج الرسام"): any app or page satisfies them, unless the query is
/// nothing but such words.
const SETTING_WORDS: &[&str] = &["اعدادات", "الاعدادات", "اعداد", "اعدادت", "settings", "setting", "ضبط", "خيارات", "options"];
const APP_WORDS: &[&str] =
    &["برنامج", "البرنامج", "تطبيق", "التطبيق", "app", "program", "افتح", "فتح", "open", "شغل", "تشغيل", "run", "launch"];

/// A normalised query and its words.
struct Query {
    full: String,
    words: Vec<String>,
}

impl Query {
    fn new(raw: &str) -> Query {
        let full = normalize(raw).split_whitespace().collect::<Vec<_>>().join(" ");
        let words = full.split(' ').filter(|w| !w.is_empty()).map(str::to_string).collect();
        Query { full, words }
    }
}

/// One word that must match somewhere: itself and its mask, and the same
/// without the Arabic article.
struct Word<'a> {
    word: &'a str,
    mask: u128,
    bare: Option<(&'a str, u128)>,
}

/// A query prepared for one kind of entry (apps or settings), whose
/// `kind_words` any entry of the kind satisfies.
struct Prepared<'a> {
    /// (text, mask, percent it counts): the whole query, then without the
    /// kind words ("اعدادات الشاشه" → "الشاشه") and without the Arabic
    /// article ("الواي فاي" → "واي فاي").
    fulls: Vec<(String, u128, u32)>,
    words: Vec<Word<'a>>,
    /// Letters every match needs, to skip entries at once.
    required: u128,
}

impl<'a> Prepared<'a> {
    fn new(q: &'a Query, kind_words: &[&str]) -> Prepared<'a> {
        let all_kind = q.words.iter().all(|w| kind_words.contains(&w.as_str()));
        let rest: Vec<&'a str> =
            q.words.iter().map(String::as_str).filter(|w| all_kind || !kind_words.contains(w)).collect();
        let mut fulls = vec![(q.full.clone(), mask(&q.full), 100)];
        if !rest.is_empty() && rest.len() < q.words.len() {
            let r = rest.join(" ");
            fulls.push((r.clone(), mask(&r), 97));
        }
        let bare: Vec<&str> = rest.iter().map(|w| strip_article(w).unwrap_or(w)).collect();
        if bare != rest && !bare.is_empty() {
            let b = bare.join(" ");
            fulls.push((b.clone(), mask(&b), 95));
        }
        let words: Vec<Word<'a>> = rest
            .iter()
            .map(|w| Word { word: w, mask: mask(w), bare: strip_article(w).map(|b| (b, mask(b))) })
            .collect();
        // A word matches as itself or without its article: the bare letters are needed.
        let required = words.iter().fold(0, |m, w| m | w.bare.map_or(w.mask, |(_, bm)| bm));
        Prepared { fulls, words, required }
    }

    /// Score of the query against `entry`, 0 when it does not match.
    fn score(&self, entry: &Entry, english: bool) -> u32 {
        if self.fulls[0].0.is_empty() || !has(entry.mask, self.required) {
            return 0;
        }
        let mut best = 0;
        for t in &entry.texts {
            let w = t.field.weight(english);
            for (full, m, percent) in &self.fulls {
                let s = t.full_score(full, *m);
                if s > 0 {
                    best = best.max(s * w / 100 * percent / 100);
                }
            }
        }
        // Every word somewhere, in any of the texts.
        let mut words_score = u32::MAX;
        for w in &self.words {
            let mut word_best = 0;
            for t in &entry.texts {
                let mut s = t.word_score(w.word, w.mask);
                if s == 0
                    && let Some((b, bm)) = w.bare
                {
                    s = t.word_score(b, bm) * 95 / 100;
                }
                word_best = word_best.max(s * t.field.weight(english) / 100);
            }
            words_score = words_score.min(word_best);
            if words_score == 0 {
                break;
            }
        }
        if words_score != u32::MAX {
            best = best.max(words_score);
        }
        best * entry.percent / 100
    }
}

/// Longest word matched with typos.
const MAX_TYPO_WORD: usize = 48;

/// Damerau (optimal string alignment) distance between `t` and the closest
/// prefix of `w`, if at most `k` (at most 2).
fn prefix_typos(t: &[char], w: &[char], k: usize) -> bool {
    const N: usize = MAX_TYPO_WORD + 4;
    let m = t.len();
    if m == 0 || m > MAX_TYPO_WORD || k > 2 || w.len() + k < m {
        return false;
    }
    let w = &w[..w.len().min(m + k)];
    let n = w.len();
    // Quick reject: with at most k edits, one of the first k+1 letters of `t`
    // is kept and lands within the first 2k+2 letters of `w`.
    let head = &w[..n.min(2 * k + 2)];
    if !t[..m.min(k + 1)].iter().any(|c| head.contains(c)) {
        return false;
    }
    // Rows i-2, i-1 and i of the distances between t[..i] and w[..j].
    let mut prev2 = [0usize; N];
    let mut prev = [0usize; N];
    let mut cur = [0usize; N];
    for (j, d) in prev.iter_mut().enumerate().take(n + 1) {
        *d = j;
    }
    for i in 1..=m {
        cur[0] = i;
        for j in 1..=n {
            let cost = usize::from(t[i - 1] != w[j - 1]);
            let mut d = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && t[i - 1] == w[j - 2] && t[i - 2] == w[j - 1] {
                d = d.min(prev2[j - 2] + 1);
            }
            cur[j] = d;
        }
        prev2 = prev;
        prev = cur;
    }
    prev[..=n].iter().any(|&d| d <= k)
}

/// Typo score: every query word (chars, mask, edits allowed) starts a word
/// of a name of the entry with at most that many edits.
fn typo_score(words: &[(Vec<char>, u128, usize)], entry: &Entry, english: bool) -> u32 {
    let mut total = u32::MAX;
    for (wc, wm, k) in words {
        // Each letter the entry lacks costs an edit at least.
        if (wm & !entry.mask).count_ones() as usize > *k {
            return 0;
        }
        let mut best = 0;
        for t in entry.texts.iter().filter(|t| t.field.is_name()) {
            if (wm & !t.mask).count_ones() as usize > *k {
                continue;
            }
            if t.char_starts.iter().any(|&s| prefix_typos(wc, &t.chars[s as usize..], *k)) {
                best = best.max(TYPO * t.field.weight(english) / 100);
            }
        }
        if best == 0 {
            return 0;
        }
        total = total.min(best);
    }
    if total == u32::MAX { 0 } else { total * entry.percent / 100 }
}

impl Index {
    pub fn build(apps: &[App], settings: &[Page]) -> Index {
        let apps_idx = apps
            .iter()
            .map(|a| {
                let mut texts = vec![Text::new(Field::Name, &a.name)];
                texts.extend(a.aliases.iter().map(|al| Text::new(Field::Alias, al)));
                Entry::new(texts, 100, &a.name)
            })
            .collect();
        let settings_idx = settings
            .iter()
            .map(|p| {
                let mut texts = vec![
                    Text::new(Field::TitleAr, p.title_ar),
                    Text::new(Field::TitleEn, p.title_en),
                    Text::new(Field::Section, p.section_ar),
                    Text::new(Field::Section, p.section_en),
                ];
                texts.extend(p.keywords.iter().map(|k| Text::new(Field::Keyword, k)));
                Entry::new(texts, if p.is_settings_uri() { 100 } else { 98 }, "")
            })
            .collect();
        Index {
            apps: apps_idx,
            settings: settings_idx,
            app_names: apps.iter().map(|a| a.name.clone()).collect(),
            setting_uris: settings.iter().map(|p| p.uri).collect(),
        }
    }

    /// Whether this index was built from exactly these apps and settings.
    pub fn matches(&self, apps: &[App], settings: &[Page]) -> bool {
        self.app_names.len() == apps.len()
            && self.setting_uris.len() == settings.len()
            && self.app_names.iter().zip(apps).all(|(n, a)| *n == a.name)
            && self.setting_uris.iter().zip(settings).all(|(u, p)| *u == p.uri)
    }
}

/// (position, score) of the entries that match.
pub type Hits = Vec<(usize, u32)>;

impl Index {
    /// Scores of the matching apps and settings pages (index, score),
    /// settings filtered by the Windows build.
    pub fn score(&self, raw: &str, ctx: &Context, settings: &[Page]) -> (Hits, Hits) {
        let english = ctx.english;
        let shown = |i: usize| settings.get(i).is_some_and(|p| p.available(ctx.windows_build));
        let mut app_scores = vec![0u32; self.apps.len()];
        let mut set_scores = vec![0u32; self.settings.len()];

        let pass = |q: &Query, percent: u32, app_scores: &mut [u32], set_scores: &mut [u32]| {
            let p = Prepared::new(q, APP_WORDS);
            for (e, s) in self.apps.iter().zip(app_scores.iter_mut()) {
                *s = (*s).max(p.score(e, english) * percent / 100);
            }
            let p = Prepared::new(q, SETTING_WORDS);
            for (i, (e, s)) in self.settings.iter().zip(set_scores.iter_mut()).enumerate() {
                if shown(i) {
                    *s = (*s).max(p.score(e, english) * percent / 100);
                }
            }
        };

        let q = Query::new(raw);
        pass(&q, 100, &mut app_scores, &mut set_scores);
        // Typed with the other keyboard layout: "ؤاقخةث" is "chrome". Converted
        // before normalising, which would merge the letters the keys stand for.
        if let Some(conv) = layout::convert_layout(raw) {
            pass(&Query::new(&conv), 75, &mut app_scores, &mut set_scores);
        }
        // Arabic letters for a Latin app name: "كوكل بلي" is "Google Play".
        // Not for settings pages, which have Arabic titles of their own.
        if let Some(qs) = phonetic::query_sounds(&q.full) {
            for (e, s) in self.apps.iter().zip(app_scores.iter_mut()) {
                let base = if phonetic::matches(&qs, &e.sounds) {
                    if phonetic::is_whole_words(&qs, &e.sounds) {
                        SOUNDS_LIKE_WORDS
                    } else if phonetic::is_exact_words(&qs, &e.sounds) {
                        SOUNDS_LIKE_EXACT
                    } else {
                        SOUNDS_LIKE
                    }
                } else if *s < SOUNDS_LIKE_SLIP && phonetic::matches_tolerant(&qs, &e.sounds) {
                    SOUNDS_LIKE_SLIP
                } else {
                    0
                };
                *s = (*s).max(base * e.percent / 100);
            }
        }
        let none = |a: &[u32], s: &[u32]| a.iter().chain(s).all(|&x| x == 0);
        // Arabizi ("kroom" → كروم), only for one Latin word and when nothing matched.
        if none(&app_scores, &set_scores) && q.words.len() == 1 && q.full.is_ascii() && franco::is_arabizi(&q.full) {
            for cand in franco::candidates(&q.full) {
                pass(&Query::new(&cand), 60, &mut app_scores, &mut set_scores);
            }
        }
        // Typos, last.
        let typo_words: Vec<(Vec<char>, u128, usize)> = q
            .words
            .iter()
            .map(|w| {
                let wc: Vec<char> = w.chars().collect();
                let k = usize::from(fuzzy::max_edits(wc.len()));
                (wc, mask(w), k)
            })
            .collect();
        let typos_ok =
            !typo_words.is_empty() && typo_words.iter().all(|(wc, _, k)| *k > 0 && wc.len() <= MAX_TYPO_WORD);
        if typos_ok && none(&app_scores, &set_scores) {
            for (e, s) in self.apps.iter().zip(app_scores.iter_mut()) {
                *s = typo_score(&typo_words, e, english);
            }
            for (i, (e, s)) in self.settings.iter().zip(set_scores.iter_mut()).enumerate() {
                if shown(i) {
                    *s = typo_score(&typo_words, e, english);
                }
            }
        }
        let hits = |v: Vec<u32>| v.into_iter().enumerate().filter(|&(_, s)| s > 0).collect();
        (hits(app_scores), hits(set_scores))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_starts_include_after_article() {
        let t = Text::new(Field::Name, "إعدادات الشاشة");
        assert_eq!(t.text, "اعدادات الشاشه");
        let words: Vec<&str> = t.starts.iter().map(|&s| &t.text[s as usize..]).collect();
        assert_eq!(words, ["اعدادات الشاشه", "الشاشه", "شاشه"]);
    }

    #[test]
    fn typo_distance() {
        let c = |s: &str| s.chars().collect::<Vec<_>>();
        assert!(prefix_typos(&c("chrmoe"), &c("chrome"), 1));
        assert!(prefix_typos(&c("spotfy"), &c("spotify"), 1));
        assert!(prefix_typos(&c("telegr"), &c("telegram"), 0));
        assert!(!prefix_typos(&c("zzzz"), &c("chrome"), 1));
    }

    #[test]
    fn tiers() {
        let t = Text::new(Field::Name, "Google Chrome");
        let full = |q: &str| t.full_score(q, mask(q));
        assert_eq!(full("google chrome"), EXACT);
        assert_eq!(full("goo"), PREFIX);
        assert_eq!(full("chr"), WORD_START);
        assert_eq!(full("hrom"), SUBSTRING);
        assert_eq!(full("x"), 0);
        assert_eq!(t.word_score("chrome", mask("chrome")), ALL_WORDS_EXACT);
        assert_eq!(t.word_score("chro", mask("chro")), ALL_WORDS);
    }
}
