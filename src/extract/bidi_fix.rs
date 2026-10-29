//! Repair of Arabic text that a PDF stored in visual (display) order.
//!
//! Many PDF producers write Arabic glyphs in the order they are painted, left
//! to right, so every extracted line comes out reversed ("يف" instead of
//! "في") and content search finds nothing. Two signals detect this:
//! - presentation forms (per line): in logical order a word starts with an
//!   initial form and ends with a final form; visual order flips that;
//! - frequent words and the definite article (whole document): `في`, `من`,
//!   `ال…` versus their reversed spellings `يف`, `نم`, `…لا`.
//!
//! Lines judged visual are rewritten in logical order. When the evidence is
//! mixed, the repaired lines are appended after the original text so either
//! orientation matches (snippets still come from the original, which is first).
//!
//! Detection must run before presentation forms are folded to base letters,
//! which is why this is applied inside `extract_pdf`.

use super::{fe80_form, normalize_arabic_forms};

/// Frequent words whose reversed spelling is not itself a common word.
/// (Ambiguous pairs such as مع/عم, لك/كل, قد/دق, لم/مل and palindromes like
/// هذه are deliberately left out.)
const COMMON_WORDS: &[&str] = &[
    "في", "من", "على", "إلى", "الى", "عن", "أن", "إن", "التي", "الذي", "الله", "هذا", "ذلك", "كان",
    "كانت", "عند", "بعد", "حتى", "بين",
];

/// Share of votes (in quarters) one orientation needs to be called confidently.
const CONFIDENT_QUARTERS: u32 = 3;
/// Minimum document-level word votes before visual order is assumed outright.
const MIN_DOC_VOTES: u32 = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Shape {
    Isolated,
    Final,
    Initial,
    Medial,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Order {
    Logical,
    Visual,
    Unsure,
}

/// Evidence for each orientation.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct Votes {
    logical: u32,
    visual: u32,
}

impl Votes {
    fn add(&mut self, o: Votes) {
        self.logical += o.logical;
        self.visual += o.visual;
    }

    /// Visual needs >= 75 % of the votes and at least `min` of them; logical
    /// wins outright with <= 25 % visual votes (or none). Anything else is unsure.
    fn decide(self, min: u32) -> Order {
        let total = self.logical + self.visual;
        if self.visual * 4 <= total * (4 - CONFIDENT_QUARTERS) {
            Order::Logical
        } else if self.visual * 4 >= total * CONFIDENT_QUARTERS && self.visual >= min {
            Order::Visual
        } else {
            Order::Unsure
        }
    }
}

/// Arabic-script letters and marks (Arabic-Indic digits and Arabic punctuation
/// excluded: they are numbers / neutrals for bidi purposes).
fn is_arabic(c: char) -> bool {
    matches!(c as u32,
        0x0610..=0x061A | 0x0620..=0x065F | 0x066E..=0x06D3 | 0x06D5..=0x06EF | 0x06FA..=0x06FF
        | 0x0750..=0x077F | 0x08A0..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFC)
}

/// Combining marks that belong to the preceding letter.
fn is_mark(c: char) -> bool {
    matches!(c as u32,
        0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06DC | 0x06DF..=0x06E4
        | 0x06E7 | 0x06E8 | 0x06EA..=0x06ED | 0x08D3..=0x08FF)
}

/// Characters that form left-to-right runs (Latin, digits incl. Arabic-Indic).
fn is_ltr(c: char) -> bool {
    c.is_alphanumeric() && !is_arabic(c)
}

/// Symbols that bidi keeps attached to an adjacent number.
fn is_terminator(s: &str) -> bool {
    matches!(
        s,
        "%" | "\u{066A}" | "\u{2030}" | "$" | "\u{20AC}" | "\u{00A3}" | "\u{00B0}" | "#"
    )
}

/// Contextual shape of a presentation-form letter (U+FE80..=U+FEFC only).
fn shape(c: char) -> Option<Shape> {
    const SHAPES: [Shape; 4] = [Shape::Isolated, Shape::Final, Shape::Initial, Shape::Medial];
    let cp = c as u32;
    match cp {
        0xFE80..=0xFEF4 => fe80_form(cp).map(|(_, form)| SHAPES[form]),
        // Lam-alef ligatures: isolated / final pairs.
        0xFEF5..=0xFEFC => Some(SHAPES[((cp - 0xFEF5) % 2) as usize]),
        _ => None,
    }
}

/// Votes from presentation-form shapes at the edges of each Arabic word.
fn form_votes(line: &str) -> Votes {
    let mut v = Votes::default();
    for word in line.split(|c: char| !is_arabic(c)) {
        let mut shapes = word.chars().filter_map(shape);
        let (Some(first), Some(last)) = (shapes.next(), shapes.next_back()) else {
            continue; // fewer than two shaped letters: no evidence
        };
        match first {
            Shape::Initial => v.logical += 1,
            Shape::Final => v.visual += 1,
            _ => {}
        }
        match last {
            Shape::Final => v.logical += 1,
            Shape::Initial => v.visual += 1,
            _ => {}
        }
    }
    v
}

/// Votes from frequent words and the definite article (base letters expected).
fn word_votes(text: &str) -> Votes {
    let mut v = Votes::default();
    for raw in text.split(|c: char| !is_arabic(c)) {
        if raw.is_empty() {
            continue;
        }
        let word: String = raw
            .chars()
            .filter(|&c| !is_mark(c) && c != '\u{0640}')
            .collect();
        for w in COMMON_WORDS {
            if word == *w {
                v.logical += 1;
            } else if word.chars().eq(w.chars().rev()) {
                v.visual += 1;
            }
        }
        if word.chars().count() >= 4 {
            if word.starts_with("ال") || word.starts_with("وال") {
                v.logical += 1;
            }
            if word.ends_with("لا") || word.ends_with("لاو") {
                v.visual += 1;
            }
        }
    }
    v
}

/// Rewrites one visual-order line in logical order: the sequence of
/// letter clusters is reversed, then left-to-right runs (Latin words,
/// numbers, and the neutrals between them) are turned back, and brackets
/// outside those runs are mirrored. Surrounding whitespace stays in place.
fn fix_line(line: &str) -> String {
    let core = line.trim();
    let lead = &line[..line.len() - line.trim_start().len()];
    let trail = &line[lead.len() + core.len()..];

    // Letter + following combining marks form one cluster.
    let mut clusters: Vec<&str> = Vec::new();
    let mut start = 0;
    for (i, c) in core.char_indices() {
        if i > 0 && !is_mark(c) {
            clusters.push(&core[start..i]);
            start = i;
        }
    }
    if !core.is_empty() {
        clusters.push(&core[start..]);
    }
    clusters.reverse();

    #[derive(PartialEq)]
    enum Kind {
        Ltr,
        Rtl,
        Neutral,
    }
    let kind = |s: &str| match s.chars().next() {
        Some(c) if is_ltr(c) => Kind::Ltr,
        Some(c) if is_arabic(c) => Kind::Rtl,
        _ => Kind::Neutral,
    };
    let kinds: Vec<Kind> = clusters.iter().map(|s| kind(s)).collect();

    let mut in_ltr = vec![false; clusters.len()];
    let mut i = 0;
    while i < clusters.len() {
        if kinds[i] != Kind::Ltr {
            i += 1;
            continue;
        }
        let mut end = i;
        let mut j = i + 1;
        while j < clusters.len() && kinds[j] != Kind::Rtl {
            if kinds[j] == Kind::Ltr {
                end = j;
            }
            j += 1;
        }
        // Number terminators (`%`, currency…) stick to the run they touch.
        while i > 0 && kinds[i - 1] == Kind::Neutral && is_terminator(clusters[i - 1]) {
            i -= 1;
        }
        while end + 1 < clusters.len()
            && kinds[end + 1] == Kind::Neutral
            && is_terminator(clusters[end + 1])
        {
            end += 1;
        }
        clusters[i..=end].reverse();
        in_ltr[i..=end].iter_mut().for_each(|f| *f = true);
        i = end + 1;
    }

    let mut out = String::with_capacity(line.len());
    out.push_str(lead);
    for (s, ltr) in clusters.iter().zip(in_ltr) {
        let mirrored = match *s {
            "(" => ")",
            ")" => "(",
            "[" => "]",
            "]" => "[",
            "{" => "}",
            "}" => "{",
            "<" => ">",
            ">" => "<",
            "«" => "»",
            "»" => "«",
            other => other,
        };
        out.push_str(if ltr { s } else { mirrored });
    }
    out.push_str(trail);
    out
}

/// Detects visual-order Arabic in PDF-extracted `text` and returns it in
/// logical order (see the module docs). Text without Arabic is returned as is.
pub(super) fn fix_visual_order(text: String) -> String {
    if !text.chars().any(is_arabic) {
        return text;
    }

    // Per-line shape evidence plus whole-document word evidence.
    let lines: Vec<&str> = text.split('\n').collect();
    let line_forms: Vec<Votes> = lines.iter().map(|l| form_votes(l)).collect();
    let mut doc = word_votes(&normalize_arabic_forms(&text));
    line_forms.iter().for_each(|v| doc.add(*v));
    let doc_order = doc.decide(MIN_DOC_VOTES);

    let mut out = String::with_capacity(text.len());
    let mut extra = String::new();
    let mut changed = false;
    for (i, (line, forms)) in lines.iter().zip(&line_forms).enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if !line.chars().any(is_arabic) {
            out.push_str(line);
            continue;
        }
        let order = match forms.decide(1) {
            Order::Unsure if doc_order != Order::Unsure => doc_order,
            Order::Logical if *forms == Votes::default() => doc_order,
            o => o,
        };
        match order {
            Order::Logical => out.push_str(line),
            Order::Visual => {
                out.push_str(&fix_line(line));
                changed = true;
            }
            Order::Unsure => {
                out.push_str(line);
                extra.push('\n');
                extra.push_str(&fix_line(line));
                changed = true;
            }
        }
    }
    if !changed {
        return text;
    }
    if !extra.is_empty() {
        out.push('\n');
        out.push_str(&extra);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rev(s: &str) -> String {
        s.chars().rev().collect()
    }

    #[test]
    fn common_words_are_not_palindromes() {
        for w in COMMON_WORDS {
            assert_ne!(*w, rev(w), "{w}");
        }
    }

    #[test]
    fn fix_line_keeps_numbers_and_latin_ltr() {
        assert_eq!(fix_line("2024 يونسلا ريرقتلا يف"), "في التقرير السنوي 2024");
        assert_eq!(
            fix_line("  PDF Annual :ريرقتلا\r"),
            "  التقرير: PDF Annual\r"
        );
        assert_eq!(fix_line("(ةكرشلا) 3.5%"), "3.5% (الشركة)");
        // Marks stay after their letter.
        assert_eq!(fix_line(&rev_clusters("كَتَبَ")), "كَتَبَ");
    }

    /// Visual-order rendering of a logical Arabic-only string.
    fn rev_clusters(s: &str) -> String {
        let mut clusters: Vec<String> = Vec::new();
        for c in s.chars() {
            match clusters.last_mut() {
                Some(last) if is_mark(c) => last.push(c),
                _ => clusters.push(c.to_string()),
            }
        }
        clusters.reverse();
        clusters.concat()
    }

    #[test]
    fn word_based_detection_fixes_visual_document() {
        let visual = "2024 يونسلا ريرقتلا يف\nةكرشلا نم ريرقت\nEnglish line";
        assert_eq!(
            fix_visual_order(visual.into()),
            "في التقرير السنوي 2024\nتقرير من الشركة\nEnglish line"
        );
    }

    #[test]
    fn logical_text_is_untouched() {
        let logical = "في التقرير السنوي 2024\nتقرير من الشركة على الموقع";
        assert_eq!(fix_visual_order(logical.into()), logical);
        assert_eq!(fix_visual_order("plain text".into()), "plain text");
        // Arabic without any cue: assumed logical.
        assert_eq!(fix_visual_order("سلام".into()), "سلام");
    }

    #[test]
    fn presentation_forms_detect_per_line() {
        // "مرحبا بكم" shaped: ﻣﺮﺣﺒﺎ ﺑﻜﻢ (logical).
        let logical = "\u{FEE3}\u{FEAE}\u{FEA3}\u{FE92}\u{FE8E} \u{FE91}\u{FEDC}\u{FEE2}";
        assert_eq!(
            form_votes(logical),
            Votes {
                logical: 4,
                visual: 0
            }
        );
        assert_eq!(fix_visual_order(logical.into()), logical);

        let visual = rev(logical);
        assert_eq!(
            form_votes(&visual),
            Votes {
                logical: 0,
                visual: 4
            }
        );
        let fixed = fix_visual_order(visual);
        assert_eq!(fixed, logical);
        assert_eq!(normalize_arabic_forms(&fixed), "مرحبا بكم");

        // Lam-alef ligature survives reversal as one glyph: "لا شيء" visual.
        let la = "\u{FEFB} \u{FEB7}\u{FEF4}\u{FE80}";
        let fixed = fix_visual_order(rev(la));
        assert_eq!(normalize_arabic_forms(&fixed), "لا شيء");
    }

    #[test]
    fn mixed_evidence_appends_fixed_copy() {
        // One logical cue (في) and one visual cue (نم): unsure.
        let text = "في نم";
        let out = fix_visual_order(text.into());
        assert!(out.starts_with("في نم\n"), "{out}");
        assert!(out.ends_with("من يف"), "{out}");
    }

    #[test]
    fn votes_thresholds() {
        let v = |logical, visual| Votes { logical, visual };
        assert_eq!(v(0, 0).decide(3), Order::Logical);
        assert_eq!(v(3, 1).decide(3), Order::Logical);
        assert_eq!(v(1, 3).decide(3), Order::Visual);
        assert_eq!(v(0, 2).decide(3), Order::Unsure);
        assert_eq!(v(0, 1).decide(1), Order::Visual);
        assert_eq!(v(2, 2).decide(1), Order::Unsure);
    }
}
