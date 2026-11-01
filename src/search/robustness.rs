//! Matching never panics, whatever the text and the query: a panic in a
//! content search loses the search, and one while drawing the list (name
//! highlighting) would end the program. Random texts and queries built from
//! the pieces that make byte offsets tricky: Arabic letters that
//! normalisation folds or drops, ligatures that expand, letters whose lower
//! case is longer, line breaks, and the archive member markers.

use super::*;

const PIECES: &[&str] = &[
    "a", "B", "rtf", "eml", "ب", "ة", "أ", "ال", "تقرير", "\u{064E}", "\u{0651}", "\u{0640}", "\u{FEFB}", "\u{FDF2}",
    "\u{FE70}", "\u{FE8D}", "ﷺ", " ", "\n", "\r\n", "\u{1F}\u{FDD0}", "İ", "ß", "ẞ", "Σ", "ﬃ", "ǅ", "\u{200F}", "١",
    "x.txt\n", "\t", "\"", "~", "|", "-", ":", "*", "\\par ", "=?utf-8?", "\u{FFFD}",
];
const PREFIXES: &[&str] = &["", "\"", "ww:", "re:", "-", "folder:", "fuzzy:", "root:", "ext:rtf ", "\"a b\"~2 "];

/// A small xorshift generator: the same cases on every run.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }

    fn pieces(&mut self, max: usize) -> String {
        (0..self.below(max)).map(|_| PIECES[self.below(PIECES.len())]).collect()
    }
}

fn check(text: &str, query: &str) {
    let q = Query::parse_at(query, 1_700_000_000, 0);
    let r = std::panic::catch_unwind(|| {
        let _ = content_hit(0, text, &q);
        let _ = match_ranges(text, &q);
        let _ = all_matches(text, &q, 5);
        let _ = match_text(text, &q);
    });
    assert!(r.is_ok(), "panic for text {text:?} and query {query:?}");
}

#[test]
fn random_texts_and_queries() {
    let mut rng = Rng(0x1234_5678);
    for _ in 0..20_000 {
        let text = rng.pieces(30);
        let mut query = String::new();
        for _ in 0..1 + rng.below(4) {
            query.push_str(PREFIXES[rng.below(PREFIXES.len())]);
            query.push_str(&rng.pieces(4));
            query.push(' ');
        }
        check(&text, &query);
    }
}

/// Code points from the ranges where normalisation, case mapping and word
/// splitting behave unusually, plus any scalar value now and then.
const RANGES: &[(u32, u32)] = &[
    (0x20, 0x7E),
    (0x00, 0x1F),
    (0xC0, 0x24F),     // Latin with case pairs, ǅ
    (0x300, 0x36F),    // combining marks
    (0x370, 0x3FF),    // Greek (final sigma)
    (0x600, 0x6FF),    // Arabic, harakat, tatweel, Persian digits
    (0x200B, 0x206F),  // zero-width and bidi marks
    (0xFB00, 0xFB06),  // Latin ligatures
    (0xFB50, 0xFDFF),  // Arabic presentation forms A
    (0xFE70, 0xFEFF),  // Arabic presentation forms B, BOM
    (0x1F300, 0x1F64F), // emoji
    (0x1F3FB, 0x1F3FF), // skin tones
    (0x200D, 0x200D),  // ZWJ
];

impl Rng {
    fn char(&mut self) -> char {
        if self.below(50) == 0 {
            // Any scalar value.
            loop {
                if let Some(c) = char::from_u32(self.below(0x11_0000) as u32) {
                    return c;
                }
            }
        }
        let (lo, hi) = RANGES[self.below(RANGES.len())];
        char::from_u32(lo + self.below((hi - lo + 1) as usize) as u32).unwrap_or('?')
    }

    fn unicode(&mut self, max: usize) -> String {
        (0..self.below(max)).map(|_| self.char()).collect()
    }
}

/// Highlight ranges must be sorted, disjoint, inside the name and on char
/// boundaries: the UI slices the name with them.
fn check_ranges(name: &str, q: &Query) {
    let r = match_ranges(name, q);
    for &(s, e) in &r {
        assert!(s < e && e <= name.len(), "{name:?} {r:?}");
        assert!(name.is_char_boundary(s) && name.is_char_boundary(e), "{name:?} {r:?}");
    }
    assert!(r.windows(2).all(|w| w[0].1 < w[1].0), "{name:?} {r:?}");
}

#[test]
fn random_unicode_texts_and_queries() {
    let mut rng = Rng(0xDEAD_BEEF_1234);
    for _ in 0..1_500 {
        let text = rng.unicode(40);
        let mut query = String::new();
        for _ in 0..1 + rng.below(3) {
            query.push_str(PREFIXES[rng.below(PREFIXES.len())]);
            query.push_str(&rng.unicode(6));
            query.push(' ');
        }
        check(&text, &query);
        let q = Query::parse_at(&query, 1_700_000_000, 0);
        check_ranges(&text, &q);
        // A query made from a piece of the text itself always has something to find.
        let cut: String = text.chars().skip(rng.below(10)).take(1 + rng.below(6)).collect();
        for prefix in ["", "ww:", "fuzzy:", "root:", "franco:", "re:"] {
            let q = Query::parse_at(&format!("{prefix}{cut}"), 1_700_000_000, 0);
            check(&text, &format!("{prefix}{cut}"));
            check_ranges(&text, &q);
        }
    }
}

#[test]
fn random_filter_values() {
    const FILTERS: &[&str] = &[
        "size:", "حجم:", "dm:", "تاريخ:", "datemodified:", "regex:", "re:", "ww:", "كلمة:", "ext:", "*.", ".", "folder:",
        "مجلد:", "fuzzy:", "root:", "جذر:", "franco:", "!", "-", "\"", "|",
    ];
    const BITS: &[&str] = &[
        ">", "<", ">=", "<=", "=", "..", "-", "/", ".", "1", "0", "٣", "۹", "99999999999999999999", "1e9", "mb", "kb", "م",
        "today", "اليوم", "hذا_الشهر", "2024", "13", "02-30", "7d", "w", "m", "y", "\"", "~", "~5", "|", "(", ")", "[", "*",
        "+", "{9999}", "\\", "^", "$", "?", "\u{064E}", "\u{FDF2}", "İ", " ", "٫",
    ];
    let mut rng = Rng(0xFEED_F00D);
    for _ in 0..6_000 {
        let mut query = String::new();
        for _ in 0..1 + rng.below(3) {
            query.push_str(FILTERS[rng.below(FILTERS.len())]);
            for _ in 0..rng.below(5) {
                query.push_str(BITS[rng.below(BITS.len())]);
            }
            query.push(' ');
        }
        let now = [0, 1_700_000_000, -1, i64::from(i32::MAX) * 4][rng.below(4)];
        let off = [0, 14 * 3600, -12 * 3600, 3600][rng.below(4)];
        let q = Query::parse_at(&query, now, off);
        let name = rng.pieces(10);
        check(&name, &query);
        check_ranges(&name, &q);
    }
}

#[test]
fn pathological_regexes_are_refused_or_fast() {
    let text = "a".repeat(20_000) + "تقرير" + &"ب".repeat(20_000);
    for p in [
        r"(a*)*b",
        r"(a|a)*b",
        r"(a+)+$",
        r"(?:a{1,100}){1,100}",
        r"a{100000}",
        r"(((((((((a*)*)*)*)*)*)*)*)*)*c",
        r"\w{200}x",
        r"(?s).{0,5000}x",
        r"[\p{Arabic}]{1000}",
        r"^.*$",
        r"",
        r"(",
        r"\",
        r"[z-a]",
    ] {
        let t = std::time::Instant::now();
        let q = Query::parse_at(&format!("re:\"{p}\""), 0, 0);
        assert_eq!(q.regexes.len(), usize::from(!p.is_empty()), "{p}");
        check(&text, &format!("re:\"{p}\""));
        check_ranges(&text[..2000], &q);
        assert!(t.elapsed() < std::time::Duration::from_secs(5), "{p}: {:?}", t.elapsed());
    }
    // A deeply nested pattern is rejected instead of overflowing the stack.
    let deep = "(".repeat(10_000) + "a" + &")".repeat(10_000);
    assert!(!Query::parse_at(&format!("re:{deep}"), 0, 0).regexes[0].is_valid());
}

#[test]
fn proximity_and_highlight_edge_cases() {
    for (text, query) in [
        ("تقرير", r#""تقرير تقرير"~1"#),
        ("a a a a", r#""a a a a a"~2"#),
        ("", r#""a b"~10000"#),
        ("a", r#""a b"~99999999999999999999"#),
        ("\u{0640}\u{0640}", r#""ـ ـ"~1"#),
        ("\u{FDF2}\u{FDF2}", r#""الله الله"~1"#),
        ("İ İ", r#""i i"~1"#),
        ("👩‍👩‍👧 👩‍👩‍👧", r#""👩‍👩‍👧 x"~1"#),
    ] {
        check(text, query);
        check_ranges(text, &Query::parse_at(query, 0, 0));
    }
    let long = "كلمة ".repeat(5_000) + "عقد ايجار";
    let q = Query::parse_at(&format!("\"{}\"~3", "عقد ".repeat(300)), 0, 0);
    check(&long, &format!("\"{}\"~3", "عقد ".repeat(300)));
    assert!(match_text(&long, &q).is_none());
}

#[test]
fn edge_cases() {
    for (text, query) in [
        ("", "rtf"),
        ("\u{064E}\u{0651}", "ب"),
        ("\u{1F}\u{FDD0}", "eml"),
        ("\u{1F}\u{FDD0}a.rtf", "a"),
        ("\u{1F}\u{FDD0}a.rtf\n\u{1F}\u{FDD0}", "rtf"),
        ("\u{1F}\u{FDD0}a.eml\nتقرير\u{1F}\u{FDD0}ب.rtf\nتقرير", "تقرير"),
        ("İİİ", "i"),
        ("\u{FDF2}\n\u{FDF2}", "الله"),
        ("ﷺ", "صلى"),
        ("x\r\n\r\nتقرير\r\n", "re:$"),
        ("x\r\n\r\nتقرير\r\n", "re:^"),
        ("تقرير المبيعات", "\"تقرير المبيعات\"~0"),
    ] {
        check(text, query);
    }
}
