//! Text normalisation used for matching, so that Arabic queries find words
//! regardless of hamza forms, tashkeel (diacritics), tatweel, presentation
//! forms, Persian/Urdu letter variants, or letter case.

// Only the Windows UI and the tests use this module.
#![cfg(any(windows, test))]

use unicode_normalization::char::decompose_compatible;

/// Maps a single character to its normalised form. `None` means the
/// character is dropped entirely (diacritics, tatweel, bidi marks).
#[inline]
fn map_char(c: char) -> Option<char> {
    match c {
        // Tashkeel (harakat), superscript alef, Quranic marks and tatweel.
        '\u{064B}'..='\u{065F}' | '\u{0670}' | '\u{06D6}'..='\u{06ED}' | '\u{0640}' => None,
        // Invisible direction / joiner marks.
        '\u{061C}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}' => None,
        // All alef forms to bare alef.
        'أ' | 'إ' | 'آ' | 'ٱ' | 'ٲ' | 'ٳ' => Some('ا'),
        'ة' => Some('ه'),
        'ى' | 'ئ' | 'ی' => Some('ي'),
        'ؤ' => Some('و'),
        'ک' | 'ڪ' => Some('ك'),
        // Persian/Urdu heh forms.
        'ە' | 'ہ' | 'ھ' | 'ۀ' => Some('ه'),
        // Arabic-Indic and Persian digits to ASCII digits.
        '\u{0660}'..='\u{0669}' => char::from_u32(c as u32 - 0x0660 + '0' as u32),
        '\u{06F0}'..='\u{06F9}' => char::from_u32(c as u32 - 0x06F0 + '0' as u32),
        _ => Some(c),
    }
}

/// Arabic presentation forms (contextual shapes and ligatures), which some
/// PDFs and old software store instead of the base letters.
#[inline]
fn is_presentation_form(c: char) -> bool {
    matches!(c, '\u{FB50}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFE}')
}

/// Pushes the mapped, lower-cased form of a single character.
#[inline]
fn push_mapped(out: &mut String, c: char) {
    if let Some(m) = map_char(c) {
        if m.is_ascii() {
            out.push(m.to_ascii_lowercase());
        } else {
            out.extend(m.to_lowercase());
        }
    }
}

/// Pushes the normalised (lower-cased) form of `c` into `out`.
/// Returns how many bytes were appended.
#[inline]
fn push_norm(out: &mut String, c: char) -> usize {
    let before = out.len();
    if is_presentation_form(c) {
        // Compatibility decomposition (as NFKC does), e.g. ﻻ → لا, ﷲ → الله.
        // Isolated harakat forms decompose to a space plus the mark: drop them.
        let mut first = true;
        let mut drop = false;
        decompose_compatible(c, |d| {
            if first && d == ' ' {
                drop = true;
            }
            first = false;
            if !drop {
                push_mapped(out, d);
            }
        });
    } else {
        push_mapped(out, c);
    }
    out.len() - before
}

/// Normalises a string for matching.
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        push_norm(&mut out, c);
    }
    out
}

/// Normalises a string and also returns, for every byte of the output, the
/// byte offset in `s` of the character it came from. Used to cut a readable
/// snippet out of the original text around a match.
pub fn normalize_with_map(s: &str) -> (String, Vec<u32>) {
    let mut out = String::with_capacity(s.len());
    let mut map = Vec::with_capacity(s.len());
    for (i, c) in s.char_indices() {
        let n = push_norm(&mut out, c);
        map.extend(std::iter::repeat_n(i as u32, n));
    }
    (out, map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arabic_variants_match() {
        assert_eq!(normalize("أحمد"), normalize("احمد"));
        assert_eq!(normalize("إسلام"), "اسلام");
        assert_eq!(normalize("مَدْرَسَة"), "مدرسه");
        assert_eq!(normalize("مستشفى"), normalize("مستشفي"));
        assert_eq!(normalize("كـــتاب"), "كتاب");
        assert_eq!(normalize("تقرير ٢٠٢٤"), "تقرير 2024");
    }

    #[test]
    fn latin_case_insensitive() {
        assert_eq!(normalize("Report.PDF"), "report.pdf");
    }

    #[test]
    fn map_points_back_to_source() {
        let src = "الـتَّقرير";
        let (n, map) = normalize_with_map(src);
        assert_eq!(n, "التقرير");
        assert_eq!(map.len(), n.len());
        let pos = n.find("تقرير").unwrap();
        assert!(src[map[pos] as usize..].starts_with('ت'));
    }

    #[test]
    fn presentation_forms_to_base_letters() {
        // "التقرير" written with contextual shapes, as some PDFs store it.
        assert_eq!(normalize("\u{FE8D}\u{FEDF}\u{FE98}\u{FED8}\u{FEAE}\u{FEF3}\u{FEAE}"), "التقرير");
        assert_eq!(normalize("\u{FEFB}"), "لا");
        assert_eq!(normalize("\u{FEF7}\u{FEF9}\u{FEF5}"), "لالالا");
        assert_eq!(normalize("\u{FDF2}"), "الله");
        assert_eq!(normalize("\u{FB8E}"), "ك"); // keheh
        assert_eq!(normalize("\u{FE70}\u{FE7C}"), ""); // isolated harakat
    }

    #[test]
    fn presentation_form_map_is_per_byte() {
        let src = "a\u{FEFB}\u{FDF2}b";
        let (n, map) = normalize_with_map(src);
        assert_eq!(n, "aلااللهb");
        assert_eq!(map.len(), n.len());
        let lig = src.find('\u{FEFB}').unwrap() as u32;
        let allah = src.find('\u{FDF2}').unwrap() as u32;
        let b = n.find('b').unwrap();
        assert_eq!(map[1..5], [lig; 4]);
        assert!(map[5..b].iter().all(|&m| m == allah));
        assert_eq!(map[b] as usize, src.len() - 1);
    }

    #[test]
    fn persian_urdu_folding() {
        assert_eq!(normalize("ە ہ ھ ۀ"), "ه ه ه ه");
        assert_eq!(normalize("ڪتاب"), "كتاب");
        assert_eq!(normalize("کتابی"), "كتابي");
        assert_eq!(normalize("ملف\u{061C}ات"), "ملفات");
    }

    const SAMPLES: &[&str] = &[
        "",
        "Report.PDF",
        "الـتَّقرير السنويّ ٢٠٢٤",
        "إسلام أحمد مستشفى مدرسة",
        "\u{FE8D}\u{FEDF}\u{FE98}\u{FED8}\u{FEAE}\u{FEF3}\u{FEAE} \u{FEFB}\u{FDF2}",
        "کتابی ۱۲۳ \u{200F}x\u{202B}y",
        "ÄÖÜ ß İ",
    ];

    #[test]
    fn normalize_is_idempotent() {
        for s in SAMPLES {
            let once = normalize(s);
            assert_eq!(normalize(&once), once, "{s}");
        }
    }

    #[test]
    fn map_agrees_with_normalize() {
        for s in SAMPLES {
            let (n, map) = normalize_with_map(s);
            assert_eq!(n, normalize(s), "{s}");
            assert_eq!(map.len(), n.len(), "{s}");
            // Offsets never go backwards and always start a source character.
            assert!(map.windows(2).all(|w| w[0] <= w[1]), "{s}");
            assert!(map.iter().all(|&m| s.is_char_boundary(m as usize) && (m as usize) < s.len()), "{s}");
        }
    }
}
