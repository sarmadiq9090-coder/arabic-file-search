//! Fixes text typed with the wrong keyboard layout: "hgjrvdv" typed on an
//! English layout becomes "التقرير" (standard Arabic 101 layout), and back.

/// Unshifted keys of the Arabic (101) layout: (Latin key, Arabic output).
const KEYS: &[(char, &str)] = &[
    ('q', "ض"),
    ('w', "ص"),
    ('e', "ث"),
    ('r', "ق"),
    ('t', "ف"),
    ('y', "غ"),
    ('u', "ع"),
    ('i', "ه"),
    ('o', "خ"),
    ('p', "ح"),
    ('[', "ج"),
    (']', "د"),
    ('a', "ش"),
    ('s', "س"),
    ('d', "ي"),
    ('f', "ب"),
    ('g', "ل"),
    ('h', "ا"),
    ('j', "ت"),
    ('k', "ن"),
    ('l', "م"),
    (';', "ك"),
    ('\'', "ط"),
    ('z', "ئ"),
    ('x', "ء"),
    ('c', "ؤ"),
    ('v', "ر"),
    ('b', "لا"),
    ('n', "ى"),
    ('m', "ة"),
    (',', "و"),
    ('.', "ز"),
    ('/', "ظ"),
    ('`', "ذ"),
];

/// Shifted keys that produce Arabic letters, used only for Arabic → Latin.
const SHIFTED: &[(char, &str)] = &[('T', "لإ"), ('G', "لأ"), ('B', "لآ"), ('Y', "إ"), ('H', "أ"), ('N', "آ")];

/// Tokens that are search filters, not words, and are never converted. A
/// `.ext` token counts when it is the last one or names a known file type
/// (`.pdf`), so that `.` typed for ز elsewhere is still converted.
fn is_filter(tok: &str, last: bool) -> bool {
    let lower = tok.to_ascii_lowercase();
    let known = |e: &str| crate::search::KINDS.iter().any(|(_, exts)| exts.contains(&e));
    ["ext:", "*.", "folder:", "مجلد:"].iter().any(|p| lower.starts_with(p))
        || lower
            .strip_prefix('.')
            .is_some_and(|e| !e.is_empty() && e.chars().all(|c| c.is_ascii_alphanumeric()) && (last || known(e)))
}

fn latin_to_arabic(tok: &str) -> String {
    tok.chars()
        .map(|c| {
            let k = c.to_ascii_lowercase();
            KEYS.iter().find(|(l, _)| *l == k).map_or_else(|| c.to_string(), |(_, a)| a.to_string())
        })
        .collect()
}

/// Arabic → Latin, matching two-letter keys (لا, لأ…) before single letters.
fn arabic_to_latin(tok: &str) -> String {
    let mut out = String::with_capacity(tok.len());
    let mut rest = tok;
    while let Some(c) = rest.chars().next() {
        let hit = KEYS.iter().chain(SHIFTED).filter(|(_, a)| rest.starts_with(a)).max_by_key(|(_, a)| a.len());
        match hit {
            Some((l, a)) => {
                out.push(*l);
                rest = &rest[a.len()..];
            }
            None => {
                out.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    out
}

fn is_arabic_key_letter(c: char) -> bool {
    KEYS.iter().chain(SHIFTED).any(|(_, a)| a.contains(c))
}

/// If `input` looks like it was typed in the wrong layout (all its letters
/// Latin, or all Arabic, and at least 2 of them), returns it converted to the
/// other layout. Filter tokens (`ext:`, `*.`, `folder:`, `مجلد:`, a final
/// `.pdf`) are kept as they are; tokens are rejoined with single spaces.
/// Returns `None` when there is nothing to convert.
pub fn convert_layout(input: &str) -> Option<String> {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    let n = tokens.len();
    let words = || tokens.iter().enumerate().filter(|&(i, t)| !is_filter(t, i + 1 == n)).map(|(_, t)| *t);
    let letters = || words().flat_map(str::chars).filter(|c| c.is_alphabetic());

    let to_arabic = if letters().all(|c| c.is_ascii_alphabetic()) {
        true
    } else if letters().all(is_arabic_key_letter) {
        false
    } else {
        return None;
    };
    if letters().count() < 2 {
        return None;
    }
    let out: Vec<String> = tokens
        .iter()
        .enumerate()
        .map(|(i, t)| match () {
            _ if is_filter(t, i + 1 == n) => t.to_string(),
            _ if to_arabic => latin_to_arabic(t),
            _ => arabic_to_latin(t),
        })
        .collect();
    let out = out.join(" ");
    (out != tokens.join(" ")).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_to_arabic_words() {
        assert_eq!(convert_layout("hgjrvdv").as_deref(), Some("التقرير"));
        assert_eq!(convert_layout("HGJRVDV").as_deref(), Some("التقرير"));
        assert_eq!(convert_layout("ugn").as_deref(), Some("على"));
        assert_eq!(convert_layout("bh").as_deref(), Some("لاا"));
        assert_eq!(convert_layout("lgtJ ,'kd").as_deref(), Some("ملفت وطني"));
    }

    #[test]
    fn filters_survive() {
        assert_eq!(convert_layout("hgjrvdv ext:pdf").as_deref(), Some("التقرير ext:pdf"));
        assert_eq!(convert_layout("hgjrvdv   *.docx").as_deref(), Some("التقرير *.docx"));
        assert_eq!(convert_layout("folder: hgjrvdv .pdf").as_deref(), Some("folder: التقرير .pdf"));
        assert_eq!(convert_layout(r#""hgjrvdv" | lgt !2024"#).as_deref(), Some(r#""التقرير" | ملف !2024"#));
    }

    #[test]
    fn arabic_to_latin_words() {
        assert_eq!(convert_layout("ي]ث").as_deref(), Some("d]e"));
        assert_eq!(convert_layout("اثممخ").as_deref(), Some("hello"));
        assert!(convert_layout("لاhي").is_none()); // mixed scripts
        assert_eq!(convert_layout("لاخخنس").as_deref(), Some("books"));
        assert_eq!(convert_layout("مجلد: لاخخنس").as_deref(), Some("مجلد: books"));
        assert_eq!(convert_layout("أثممخ").as_deref(), Some("Hello"));
    }

    #[test]
    fn nothing_to_convert() {
        assert_eq!(convert_layout(""), None);
        assert_eq!(convert_layout("a"), None);
        assert_eq!(convert_layout("ext:pdf"), None);
        assert_eq!(convert_layout("2024 - 01"), None);
        assert_eq!(convert_layout("abc تقرير"), None);
        assert_eq!(convert_layout("ڤيديو"), None); // letter not on the layout
    }

    #[test]
    fn known_extension_anywhere_is_kept() {
        assert_eq!(convert_layout(".pdf hgjrvdv").as_deref(), Some(".pdf التقرير"));
    }

}
