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
