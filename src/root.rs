//! Light Arabic root extraction (in the spirit of the ISRI stemmer) for
//! `جذر:` searches: كتاب، كاتب، مكتبة، مكتوب، يكتبون all give كتب. It strips
//! the article, common suffixes and verb prefixes, then a few word patterns.
//! It is a heuristic, not a dictionary: some words get no root or a wrong one.

// Only the Windows UI and the tests use this module.

use crate::search::ARTICLES;

/// Suffixes (normalised: ة is ه), longest first.
const SUFFIXES: [&str; 17] =
    ["كما", "هما", "تين", "تان", "ات", "ون", "ين", "ان", "ها", "هم", "هن", "كم", "نا", "يه", "يا", "ه", "ي"];

/// Letters of the Arabic alphabet, after normalisation.
fn is_arabic_letter(c: char) -> bool {
    matches!(c, '\u{0621}'..='\u{063A}' | '\u{0641}'..='\u{064A}')
}

fn is_weak(c: char) -> bool {
    matches!(c, 'ا' | 'و' | 'ي')
}

/// The 3-letter root of a normalised Arabic word, if one is found.
pub fn root(word: &str) -> Option<String> {
    let mut w: Vec<char> = word.chars().collect();
    if w.len() < 3 || !w.iter().all(|&c| is_arabic_letter(c)) {
        return None;
    }
    strip(&mut w, &ARTICLES, true);
    strip(&mut w, &SUFFIXES, false);
    // استفعل and its forms: استخدم، مستخدم، استخدام.
    if w.len() >= 6 && (w.starts_with(&['ا', 'س', 'ت']) || w.starts_with(&['م', 'س', 'ت'])) {
        w.drain(..3);
    }
    pattern(&w).map(|r| r.iter().collect())
}

/// Removes the first of `list` that `w` starts (or ends) with, keeping at least 3 letters.
fn strip(w: &mut Vec<char>, list: &[&str], front: bool) {
    for p in list {
        let p: Vec<char> = p.chars().collect();
        let hit = if front { w.starts_with(&p) } else { w.ends_with(&p) };
        if hit && w.len() >= p.len() + 3 {
            if front {
                w.drain(..p.len());
            } else {
                w.truncate(w.len() - p.len());
            }
            return;
        }
    }
}

/// The root of a stem of 3 to 6 letters by its pattern.
fn pattern(w: &[char]) -> Option<[char; 3]> {
    let pick = |a: usize, b: usize, c: usize| Some([w[a], w[b], w[c]]);
    match w.len() {
        3 => pick(0, 1, 2),
        4 => {
            if is_weak(w[2]) {
                pick(0, 1, 3) // فعال فعيل فعول: كتاب كبير رسول
            } else if matches!(w[1], 'ا' | 'و') {
                pick(0, 2, 3) // فاعل فوعل: كاتب
            } else if matches!(w[0], 'م' | 'ا' | 'ت' | 'ي' | 'ن') {
                pick(1, 2, 3) // مفعل أفعل تفعل يفعل: مكتب أكبر يكتب
            } else {
                None
            }
        }
        5 => match (w[0], w[1], w[2], w[3]) {
            ('م', _, _, 'و' | 'ي') | ('ت', _, _, 'ي') => pick(1, 2, 4), // مفعول تفعيل: مكتوب تقرير
            ('م' | 'ت', _, 'ا', _) | ('ا', _, 'ت', _) => pick(1, 3, 4), // مفاعل تفاعل افتعل: اجتمع
            ('ا', 'ن', ..) => pick(2, 3, 4),                             // انفعل: انكسر
            (_, 'ا', _, 'ي' | 'و') => pick(0, 2, 4),                     // فاعيل فاعول
            ('م' | 'ت' | 'ي' | 'ن' | 'ا', ..) => pattern(&w[1..]),
            _ => None,
        },
        6 => match (w[0], w[2], w[4]) {
            ('ت' | 'م', 'ا', 'ي') => pick(1, 3, 5), // تفاعيل مفاعيل: تقارير
            ('ا', 'ت', 'ا') => pick(1, 3, 5),         // افتعال: اجتماع
            ('م' | 'ت' | 'ي' | 'ن' | 'ا', ..) => pattern(&w[1..]),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normalize::normalize;

    fn r(w: &str) -> Option<String> {
        root(&normalize(w))
    }

    #[test]
    fn family_of_words() {
        for w in ["كتب", "كتاب", "كاتب", "مكتب", "مكتبة", "مكتوب", "الكتب", "كتابات", "يكتبون", "الكتاب", "مكاتب"] {
            assert_eq!(r(w).as_deref(), Some("كتب"), "{w}");
        }
        for w in ["تقرير", "تقارير", "التقرير", "مقرر"] {
            assert_eq!(r(w).as_deref(), Some("قرر"), "{w}");
        }
        for w in ["استخدام", "مستخدم", "استخدم"] {
            assert_eq!(r(w).as_deref(), Some("خدم"), "{w}");
        }
        assert_eq!(r("اجتماع").as_deref(), Some("جمع"));
        assert_eq!(r("اجتمع").as_deref(), Some("جمع"));
        assert_eq!(r("رسول").as_deref(), Some("رسل"));
        assert_eq!(r("معلومات").as_deref(), Some("علم"));
    }

    #[test]
    fn no_root() {
        assert_eq!(r("report"), None);
        assert_eq!(r("في"), None);
        assert_eq!(r("2024"), None);
        assert_eq!(r("تقرير2024"), None);
    }
}
