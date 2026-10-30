//! Franco-Arabic (Arabizi): Latin letters and digits used to write Arabic,
//! `3abd` → عبد, `7aga` → حاجة, `ta2rer` → تقرير. A word becomes a short,
//! ranked list of normalised Arabic spellings that are searched as
//! alternatives.

// Only the Windows UI and the tests use this module.

/// Most spellings made for one word.
pub const MAX_CANDIDATES: usize = 16;

/// Partial spellings kept after each letter; more than [`MAX_CANDIDATES`]
/// so that paths which later merge into the same word don't crowd others out.
const BEAM: usize = 64;

/// Digits that stand for Arabic letters (`3` ع, `6` ط, `7` ح…). Any other digit
/// means the word is not Arabizi (`file2024`).
const LETTER_DIGITS: &[u8] = b"2356789";

/// Fewest Arabic letters a spelling must have: one letter would match
/// almost every Arabic name.
const MIN_LEN: usize = 2;

/// True when `word` (normalised: lower-case ASCII) looks like an Arabizi
/// word: letters, possibly with letter digits, at least two letters, and
/// no more digits than letters.
pub fn is_arabizi(word: &str) -> bool {
    let b = word.as_bytes();
    let letters = b.iter().filter(|c| c.is_ascii_lowercase()).count();
    let digits = b.iter().filter(|c| c.is_ascii_digit()).count();
    letters >= 2
        && digits <= letters
        && b.iter().all(|c| c.is_ascii_lowercase() || LETTER_DIGITS.contains(c))
}

/// One piece of the word and its Arabic options with a cost each (lower is
/// more likely). An empty option drops the piece (a short vowel).
type Options = &'static [(&'static str, u8)];

fn is_vowel(c: u8) -> bool {
    matches!(c, b'a' | b'e' | b'i' | b'o' | b'u')
}

/// Options for a vowel (or a pair such as `aa`, `ee`, `ou` when `long`) at
/// the start, the end or inside the word.
fn vowel(c: u8, long: bool, start: bool, end: bool) -> Options {
    match (c, long, start, end) {
        // Initial vowels carry an alef (أ/إ), or stand for ع (omar, ali).
        (b'a', true, true, _) => &[("ا", 0), ("عا", 1)],
        (_, _, true, _) => &[("ا", 0), ("ع", 1)],
        (b'a', true, ..) => &[("ا", 0)],
        (b'e' | b'i', true, ..) => &[("ي", 0)],
        (_, true, ..) => &[("و", 0)],
        // Final a is usually ة (normalised to ه) or alef: 7aga, dunya.
        (b'a', _, _, true) => &[("ه", 0), ("ا", 1), ("", 2), ("ي", 2)],
        (b'e', _, _, true) => &[("", 0), ("ه", 1), ("ي", 1)],
        (b'i', _, _, true) => &[("ي", 0)],
        (_, _, _, true) => &[("و", 0), ("", 1)],
        // Short vowels inside the word are usually not written.
        (b'a', ..) => &[("", 0), ("ا", 1)],
        (b'e', ..) => &[("", 0), ("ي", 1)],
        (b'i', ..) => &[("ي", 0), ("", 1)],
        (b'o', ..) => &[("", 0), ("و", 1)],
        _ => &[("و", 0), ("", 1)],
    }
}

/// Options for a digraph (`sh`, `kh`…) starting the bytes, if there is one.
fn digraph(b: &[u8]) -> Option<Options> {
    Some(match b.get(..2)? {
        b"sh" | b"ch" => &[("ش", 0)],
        b"kh" => &[("خ", 0)],
        b"gh" => &[("غ", 0)],
        b"th" => &[("ث", 0), ("ذ", 1)],
        b"dh" => &[("ذ", 0), ("ض", 1)],
        _ => return None,
    })
}

/// Options for a consonant or letter digit; `start` and `end` matter for `2` (hamza) and `h`.
fn consonant(c: u8, start: bool, end: bool) -> Option<Options> {
    Some(match c {
        b'2' if start => &[("ا", 0), ("ق", 1), ("ء", 2)],
        b'2' if end => &[("ء", 0), ("ق", 1)],
        // Hamza inside a word sits on alef, waw or ya, which normalise to those letters.
        b'2' => &[("ء", 0), ("ق", 1), ("ا", 1), ("و", 2), ("ي", 2)],
        b'3' => &[("ع", 0)],
        b'5' => &[("خ", 0)],
        b'6' => &[("ط", 0)],
        b'7' => &[("ح", 0)],
        b'8' => &[("ق", 0), ("غ", 1)],
        b'9' => &[("ق", 0), ("ص", 1)],
        b'b' | b'p' => &[("ب", 0)],
        b't' => &[("ت", 0), ("ط", 1)],
        b'd' => &[("د", 0), ("ض", 1)],
        b'r' => &[("ر", 0)],
        b'z' => &[("ز", 0), ("ظ", 1)],
        b's' => &[("س", 0), ("ص", 1)],
        b'f' | b'v' => &[("ف", 0)],
        b'q' => &[("ق", 0)],
        b'k' => &[("ك", 0), ("ق", 1)],
        b'l' => &[("ل", 0)],
        b'm' => &[("م", 0)],
        b'n' => &[("ن", 0)],
        // Inside a word h is more often ح (ahmed, mohammed); at the ends ه (hala, allah).
        b'h' if !start && !end => &[("ح", 0), ("ه", 1)],
        b'h' => &[("ه", 0), ("ح", 1)],
        b'w' => &[("و", 0)],
        b'y' => &[("ي", 0)],
        b'j' => &[("ج", 0)],
        b'g' => &[("ج", 0), ("غ", 1)],
        b'c' => &[("ك", 0), ("س", 1)],
        b'x' => &[("كس", 0)],
        _ => return None,
    })
}

/// Splits `word` into pieces with their options. `None` when a character
/// has no Arabic reading.
fn pieces(word: &str) -> Option<Vec<Options>> {
    let b = word.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let start = i == 0;
        if let Some(opts) = digraph(&b[i..]) {
            out.push(opts);
            i += 2;
            continue;
        }
        let c = b[i];
        if is_vowel(c) {
            let next = b.get(i + 1).copied();
            let long = next.is_some_and(|n| n == c || matches!((c, n), (b'o', b'u') | (b'e', b'i') | (b'i', b'e')));
            let len = if long { 2 } else { 1 };
            out.push(vowel(c, long, start, i + len == b.len()));
            i += len;
            continue;
        }
        // A doubled consonant (shadda) is written once: mohammed.
        let mut run = 1;
        while b.get(i + run) == Some(&c) {
            run += 1;
        }
        out.push(consonant(c, start, i + run == b.len())?);
        i += run;
    }
    Some(out)
}

/// Up to [`MAX_CANDIDATES`] normalised Arabic spellings of the Arabizi
/// `word`, most likely first. Empty when `word` is not Arabizi.
pub fn candidates(word: &str) -> Vec<String> {
    if !is_arabizi(word) {
        return Vec::new();
    }
    let Some(pieces) = pieces(word) else { return Vec::new() };
    let mut beam: Vec<(u16, String)> = vec![(0, String::new())];
    for opts in pieces {
        let mut next = Vec::with_capacity(beam.len() * opts.len());
        for (cost, s) in &beam {
            for &(o, c) in opts {
                next.push((cost + u16::from(c), format!("{s}{o}")));
            }
        }
        // Stable: equal costs keep the order of the options.
        next.sort_by_key(|(c, _)| *c);
        next.truncate(BEAM);
        beam = next;
    }
    let mut out: Vec<String> = Vec::new();
    for (_, s) in beam {
        if s.chars().count() >= MIN_LEN && !out.contains(&s) {
            out.push(s);
            if out.len() == MAX_CANDIDATES {
                break;
            }
        }
    }
    out
}

/// True when every term of `q` is written in Latin letters and digits and at
/// least one word of three letters or more looks like Arabizi: the query
/// may be Arabic typed in Latin letters.
pub fn worth_trying(q: &crate::search::Query) -> bool {
    !q.terms.is_empty()
        && q.terms.iter().flat_map(|t| &t.alts).all(|a| a.is_ascii())
        && q.terms
            .iter()
            .flat_map(|t| &t.alts)
            .any(|a| is_arabizi(a) && a.bytes().filter(u8::is_ascii_lowercase).count() >= 3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normalize::normalize;

    fn has(word: &str, arabic: &str) -> bool {
        candidates(word).contains(&normalize(arabic))
    }

    #[test]
    fn known_words() {
        assert!(has("mohammed", "محمد"));
        assert!(has("mohamed", "محمد"));
        assert!(has("3abd", "عبد"));
        assert!(has("7aga", "حاجة"));
        assert!(has("ahmad", "أحمد"));
        assert!(has("ahmed", "أحمد"));
        assert!(has("taqrer", "تقرير"));
        assert!(has("ta2rer", "تقرير"));
        assert!(has("taqrir", "تقرير"));
        assert!(has("sha3er", "شاعر"));
        assert!(has("khaled", "خالد"));
        assert!(has("ghada", "غادة"));
        assert!(has("omar", "عمر"));
        assert!(has("fatma", "فاطمة"));
        assert!(has("so2al", "سؤال"));
        assert!(has("9ora", "صورة"));
        assert!(has("ma3loomat", "معلومات"));
    }

    #[test]
    fn likely_spellings_first() {
        assert_eq!(candidates("3abd")[0], "عبد");
        assert_eq!(candidates("7aga")[0], "حجه");
        assert_eq!(candidates("taqrir")[0], "تقرير");
        let c = candidates("mohammed");
        assert_eq!(c[0], "محمد", "{c:?}");
        assert_eq!(candidates("ahmad")[0], "احمد");
    }

    #[test]
    fn bounded_and_normalised() {
        for w in ["mohammed", "3abdelra7man", "ta2reer", "abcdefghijklmnop", "shkhghthdh"] {
            let c = candidates(w);
            assert!(!c.is_empty() && c.len() <= MAX_CANDIDATES, "{w}: {c:?}");
            assert!(c.iter().all(|s| normalize(s) == *s && s.chars().count() >= MIN_LEN), "{w}: {c:?}");
            let mut d = c.clone();
            d.dedup();
            assert_eq!(c, d);
        }
    }

    #[test]
    fn not_arabizi() {
        assert!(candidates("2024").is_empty());
        assert!(candidates("file2024").is_empty()); // 0, 1 and 4 are not letters
        assert!(candidates("a").is_empty());
        assert!(candidates("a77").is_empty()); // more digits than letters
        assert!(candidates("report.pdf").is_empty());
        assert!(candidates("تقرير").is_empty());
        assert!(is_arabizi("ta2rer") && is_arabizi("3abd") && !is_arabizi("x1"));
    }

    #[test]
    fn worth_trying_only_latin_words() {
        use crate::search::Query;
        assert!(worth_trying(&Query::parse("mohammed")));
        assert!(worth_trying(&Query::parse("3abd ext:pdf")));
        assert!(!worth_trying(&Query::parse("محمد")));
        assert!(!worth_trying(&Query::parse("mohammed محمد")));
        assert!(!worth_trying(&Query::parse("ab")));
        assert!(!worth_trying(&Query::parse("2024")));
        assert!(!worth_trying(&Query::parse("ext:pdf")));
    }
}
