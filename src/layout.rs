//! Fixes text typed with the wrong keyboard layout: "hgjrvdv" typed on an
//! English layout becomes "التقرير" (standard Arabic 101 layout), and back.
//! Also Persian, Central Kurdish (Sorani), Russian, Ukrainian, Turkish Q and
//! French AZERTY, each against English (US QWERTY), when that keyboard is
//! installed on the PC (all of them in the tests). Used only as a fallback
//! when the query as typed finds nothing.

// Only the Windows UI and the tests use this module.

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

/// Windows Persian (KBDFA): unshifted keys.
const PERSIAN: &[(char, &str)] = &[
    ('q', "ض"), ('w', "ص"), ('e', "ث"), ('r', "ق"), ('t', "ف"), ('y', "غ"), ('u', "ع"), ('i', "ه"), ('o', "خ"), ('p', "ح"),
    ('[', "ج"), (']', "چ"), ('a', "ش"), ('s', "س"), ('d', "ی"), ('f', "ب"), ('g', "ل"), ('h', "ا"), ('j', "ت"), ('k', "ن"),
    ('l', "م"), (';', "ک"), ('\'', "گ"), ('\\', "پ"), ('z', "ظ"), ('x', "ط"), ('c', "ز"), ('v', "ر"), ('b', "ذ"), ('n', "د"),
    ('m', "ئ"), (',', "و"),
];
/// Windows Persian: the shifted keys that give letters.
const PERSIAN_SHIFTED: &[(char, &str)] =
    &[('Z', "ة"), ('X', "ي"), ('C', "ژ"), ('V', "ؤ"), ('B', "إ"), ('N', "أ"), ('M', "ء"), ('H', "آ"), ('G', "ۀ")];

/// Windows Central Kurdish (KBDKURD): unshifted keys, close to the Latin
/// letters' sounds.
const KURDISH: &[(char, &str)] = &[
    ('q', "ق"), ('w', "و"), ('e', "ە"), ('r', "ر"), ('t', "ت"), ('y', "ی"), ('u', "ئ"), ('i', "ح"), ('o', "ۆ"), ('p', "پ"),
    ('a', "ا"), ('s', "س"), ('d', "د"), ('f', "ف"), ('g', "گ"), ('h', "ه"), ('j', "ژ"), ('k', "ک"), ('l', "ل"), ('z', "ز"),
    ('x', "خ"), ('c', "ج"), ('v', "ڤ"), ('b', "ب"), ('n', "ن"), ('m', "م"),
];
/// Windows Central Kurdish: the shifted keys that give letters (R is ڕ).
const KURDISH_SHIFTED: &[(char, &str)] = &[
    ('E', "ي"), ('R', "ڕ"), ('T', "ط"), ('Y', "ێ"), ('U', "ء"), ('I', "ع"), ('O', "ؤ"), ('P', "ث"), ('A', "آ"), ('S', "ش"),
    ('D', "ذ"), ('F', "إ"), ('G', "غ"), ('J', "أ"), ('K', "ك"), ('L', "ڵ"), ('Z', "ض"), ('X', "ص"), ('C', "چ"), ('V', "ظ"),
    ('B', "ى"), ('N', "ة"),
];

/// Windows Russian (ЙЦУКЕН): unshifted keys.
const RUSSIAN: &[(char, &str)] = &[
    ('`', "ё"), ('q', "й"), ('w', "ц"), ('e', "у"), ('r', "к"), ('t', "е"), ('y', "н"), ('u', "г"), ('i', "ш"), ('o', "щ"),
    ('p', "з"), ('[', "х"), (']', "ъ"), ('a', "ф"), ('s', "ы"), ('d', "в"), ('f', "а"), ('g', "п"), ('h', "р"), ('j', "о"),
    ('k', "л"), ('l', "д"), (';', "ж"), ('\'', "э"), ('z', "я"), ('x', "ч"), ('c', "с"), ('v', "м"), ('b', "и"), ('n', "т"),
    ('m', "ь"), (',', "б"), ('.', "ю"),
];

/// Windows Ukrainian: Russian's keys with і, ї and є for ы, ъ and э.
const UKRAINIAN: &[(char, &str)] = &[
    ('`', "ё"), ('q', "й"), ('w', "ц"), ('e', "у"), ('r', "к"), ('t', "е"), ('y', "н"), ('u', "г"), ('i', "ш"), ('o', "щ"),
    ('p', "з"), ('[', "х"), (']', "ї"), ('a', "ф"), ('s', "і"), ('d', "в"), ('f', "а"), ('g', "п"), ('h', "р"), ('j', "о"),
    ('k', "л"), ('l', "д"), (';', "ж"), ('\'', "є"), ('z', "я"), ('x', "ч"), ('c', "с"), ('v', "м"), ('b', "и"), ('n', "т"),
    ('m', "ь"), (',', "б"), ('.', "ю"),
];

/// Windows Turkish Q: the keys that differ from US QWERTY. Only US → Turkish
/// (";eker" is "şeker"): Turkish letters typed for English ones fold into
/// them anyway (ı is i, ş is s).
const TURKISH: &[(char, &str)] =
    &[('i', "ı"), ('[', "ğ"), (']', "ü"), (';', "ş"), ('\'', "i"), (',', "ö"), ('.', "ç")];

/// Windows French AZERTY: the keys that differ from US QWERTY, and the
/// unshifted digit keys that give letters (é is 2), used only AZERTY → US.
const AZERTY: &[(char, &str)] = &[
    ('q', "a"), ('w', "z"), ('a', "q"), ('z', "w"), (';', "m"), ('m', ","), (',', ";"), ('.', ":"), ('/', "!"), ('\'', "ù"),
];
const AZERTY_DIGITS: &[(char, &str)] = &[('2', "é"), ('7', "è"), ('9', "ç"), ('0', "à")];

/// A keyboard layout against US QWERTY.
struct Layout {
    /// Unshifted keys: (US key, what this layout types there).
    keys: &'static [(char, &'static str)],
    /// Shifted keys that give letters (US key with Shift, output).
    shifted: &'static [(char, &'static str)],
    /// US text → this layout.
    to_layout: bool,
    /// In US text, a capital means the shifted key (Kurdish R is ڕ, S is ش:
    /// Sorani needs Shift for common letters); otherwise case is ignored.
    shift_case: bool,
    /// This layout's text → US.
    to_latin: bool,
    /// Its letters are Latin: text typed on it may hold ASCII letters too.
    latin: bool,
    /// The Windows language ids (low word of an HKL) that use it; none:
    /// always on (Arabic).
    #[cfg_attr(not(windows), allow(dead_code))]
    langs: &'static [u16],
}

const LAYOUTS: &[Layout] = &[
    Layout { keys: KEYS, shifted: SHIFTED, to_layout: true, shift_case: false, to_latin: true, latin: false, langs: &[] },
    Layout { keys: PERSIAN, shifted: PERSIAN_SHIFTED, to_layout: true, shift_case: false, to_latin: true, latin: false, langs: &[0x0429] },
    Layout { keys: KURDISH, shifted: KURDISH_SHIFTED, to_layout: true, shift_case: true, to_latin: true, latin: false, langs: &[0x0492] },
    Layout { keys: RUSSIAN, shifted: &[], to_layout: true, shift_case: false, to_latin: true, latin: false, langs: &[0x0419] },
    Layout { keys: UKRAINIAN, shifted: &[], to_layout: true, shift_case: false, to_latin: true, latin: false, langs: &[0x0422] },
    Layout { keys: TURKISH, shifted: &[], to_layout: true, shift_case: false, to_latin: false, latin: true, langs: &[0x041F] },
    Layout {
        keys: AZERTY,
        shifted: AZERTY_DIGITS,
        to_layout: true,
        shift_case: false,
        to_latin: true,
        latin: true,
        langs: &[0x040C, 0x080C, 0x180C],
    },
];

/// Every layout (one bit each, in [`LAYOUTS`] order).
const ALL: u32 = (1 << LAYOUTS.len()) - 1;

/// The layouts whose keyboard is installed on this PC (Arabic always), read
/// once. Every layout outside Windows.
fn installed() -> u32 {
    #[cfg(windows)]
    {
        static MASK: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
        *MASK.get_or_init(|| {
            use winapi::um::winuser::GetKeyboardLayoutList;
            let mut hkls = [std::ptr::null_mut(); 64];
            // SAFETY: the buffer holds 64 handles and its length says so.
            let n = unsafe { GetKeyboardLayoutList(hkls.len() as i32, hkls.as_mut_ptr()) };
            let langs: Vec<u16> = hkls[..n.clamp(0, 64) as usize].iter().map(|&h| (h as usize & 0xFFFF) as u16).collect();
            if langs.is_empty() {
                return ALL;
            }
            LAYOUTS
                .iter()
                .enumerate()
                .filter(|(_, l)| l.langs.is_empty() || l.langs.iter().any(|id| langs.contains(id)))
                .fold(0, |m, (i, _)| m | 1 << i)
        })
    }
    #[cfg(not(windows))]
    ALL
}

/// Tokens that are search filters, not words, and are never converted:
/// every filter the query parser knows (`size:>1mb`, `dm:today`, `ext:`,
/// `مجلد:`…). A `.ext` token counts when it is the last one or names a known
/// file type (`.pdf`), so that `.` typed for ز elsewhere is still converted.
fn is_filter(tok: &str, last: bool) -> bool {
    let lower = tok.to_ascii_lowercase();
    use crate::search::KINDS;
    let known = |e: &str| KINDS.iter().any(|(_, exts)| exts.contains(&e));
    crate::search::is_filter_token(tok)
        || lower
            .strip_prefix('.')
            .is_some_and(|e| !e.is_empty() && e.chars().all(|c| c.is_ascii_alphanumeric()) && (last || known(e)))
}

/// US text → `layout`.
fn to_layout(layout: &Layout, tok: &str) -> String {
    tok.chars()
        .map(|c| {
            let shifted = layout.shifted.iter().find(|(l, _)| layout.shift_case && *l == c && c.is_ascii_uppercase());
            let k = c.to_ascii_lowercase();
            shifted.or_else(|| layout.keys.iter().find(|(l, _)| *l == k)).map_or_else(|| c.to_string(), |(_, a)| a.to_string())
        })
        .collect()
}

/// `layout`'s text → US, matching two-letter keys (لا, لأ…) before single
/// letters. Lower-cased first (Cyrillic capitals are the same keys).
fn to_latin(layout: &Layout, tok: &str) -> String {
    let lower = tok.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut rest = lower.as_str();
    while let Some(c) = rest.chars().next() {
        let hit = layout.keys.iter().chain(layout.shifted).filter(|(_, a)| rest.starts_with(a)).max_by_key(|(_, a)| a.len());
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

/// True when `c` (lower-cased) is a letter `layout` types.
fn types_letter(layout: &Layout, c: char) -> bool {
    c.to_lowercase().all(|c| layout.keys.iter().chain(layout.shifted).any(|(_, a)| a.contains(c)))
}

/// [`to_layout`] or [`to_latin`].
type Convert = fn(&Layout, &str) -> String;

/// Every conversion of `input` to or from US QWERTY in the layouts of
/// `mask` (bits in [`LAYOUTS`] order), in that order, each normalised
/// differently from `input` and from the ones before. Filter tokens
/// (`ext:`, `*.`, `folder:`, `مجلد:`, a final `.pdf`) are kept as they are;
/// tokens are rejoined with single spaces. A conversion needs at least 2
/// letters, all Latin (to a layout) or all typed by the layout (from it).
fn candidates(input: &str, mask: u32) -> Vec<String> {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    let n = tokens.len();
    let words = || tokens.iter().enumerate().filter(|&(i, t)| !is_filter(t, i + 1 == n)).map(|(_, t)| *t);
    let letters: Vec<char> = words().flat_map(str::chars).filter(|c| c.is_alphabetic()).collect();
    let mut out: Vec<String> = Vec::new();
    if letters.len() < 2 {
        return out;
    }
    let latin = letters.iter().all(char::is_ascii_alphabetic);
    let mut seen = vec![crate::normalize::normalize(&tokens.join(" "))];
    for (i, layout) in LAYOUTS.iter().enumerate() {
        if mask & 1 << i == 0 {
            continue;
        }
        let from = layout.to_latin && letters.iter().all(|&c| layout.latin && c.is_ascii_alphabetic() || types_letter(layout, c));
        let convert: [Option<Convert>; 2] = [(layout.to_layout && latin).then_some(to_layout), from.then_some(to_latin)];
        for f in convert.into_iter().flatten() {
            let conv: Vec<String> =
                tokens.iter().enumerate().map(|(i, t)| if is_filter(t, i + 1 == n) { t.to_string() } else { f(layout, t) }).collect();
            let conv = conv.join(" ");
            let norm = crate::normalize::normalize(&conv);
            if !seen.contains(&norm) {
                seen.push(norm);
                out.push(conv);
            }
        }
    }
    out
}

/// Every conversion of `input` typed with the wrong keyboard layout, among
/// the layouts installed on this PC: Arabic first, then Persian, Central
/// Kurdish, Russian, Ukrainian, Turkish Q and French AZERTY. Empty when
/// there is nothing to convert.
pub fn layout_candidates(input: &str) -> Vec<String> {
    candidates(input, installed())
}

/// The first of [`layout_candidates`]: `input` typed in the wrong layout
/// (all its letters Latin, or all of one layout, and at least 2 of them),
/// converted. Filter tokens are kept; tokens are rejoined with single spaces.
/// Returns `None` when there is nothing to convert.
pub fn convert_layout(input: &str) -> Option<String> {
    layout_candidates(input).into_iter().next()
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
        assert_eq!(convert_layout("ڨيديو"), None); // letter not on any layout
    }

    fn all(s: &str) -> Vec<String> {
        candidates(s, ALL)
    }

    #[test]
    fn persian_layout() {
        // پدر and گل: keys Arabic has no letter for.
        assert!(all("\\nv").contains(&"پدر".to_string()));
        assert!(all("'gd").contains(&"گلی".to_string()));
        // English typed on Persian: ی is not on the Arabic layout.
        assert_eq!(all("یخل")[0], "dog"); // then the Kurdish reading, «yxl»
        // A Persian word also typed the Arabic way is listed once.
        let c = all("sghl");
        assert_eq!(c[0], "سلام");
        assert_eq!(c.iter().filter(|c| crate::normalize::normalize(c) == "سلام").count(), 1, "{c:?}");
    }

    #[test]
    fn kurdish_layout() {
        // Sorani keys follow the Latin sounds; Shift gives ڕ, ش, ڵ…
        assert!(all("Roj").contains(&"ڕۆژ".to_string()));
        assert!(all("slaw").contains(&"سلاو".to_string()));
        assert!(all("Sar").contains(&"شار".to_string()));
        assert!(all("سلاو").contains(&"slaw".to_string()));
        assert!(all("ڕۆژ").contains(&"Roj".to_string()));
    }

    #[test]
    fn russian_and_ukrainian_layouts() {
        assert!(all("ghbdtn").contains(&"привет".to_string()));
        assert!(all("Ghbdtn").contains(&"привет".to_string()));
        assert!(all("ghbdsn").contains(&"привіт".to_string())); // і is on s
        assert_eq!(all("руддщ"), ["hello"]);
        assert_eq!(all("Руддщ Цщкдв"), ["hello world"]);
        assert_eq!(all("ghbdsn ext:pdf").iter().filter(|c| c.ends_with(" ext:pdf")).count(), all("ghbdsn ext:pdf").len());
    }

    #[test]
    fn turkish_and_azerty_layouts() {
        // Turkish letters on the US punctuation keys.
        assert!(all(";eker").contains(&"şeker".to_string()));
        assert!(all("g]zel").contains(&"güzel".to_string()));
        // AZERTY: US text meant for AZERTY, and AZERTY text meant for US.
        assert!(all(";qison").contains(&"maison".to_string()));
        assert!(all("zindoz").contains(&"window".to_string()));
        assert!(all("rqpport é0é4").contains(&"rapport 2024".to_string()));
    }

    #[test]
    fn only_installed_layouts() {
        // Arabic alone (the first bit): as before.
        assert_eq!(candidates("ghbdtn", 1), ["لالايفى"]);
        assert!(candidates("руддщ", 1).is_empty());
        assert_eq!(candidates("hgjrvdv", 1), ["التقرير"]);
        // Every candidate keeps the tokens and differs from the input.
        for s in ["ghbdtn", "Roj", ";qison", "руддщ", "سلاو"] {
            for c in all(s) {
                assert_eq!(c.split(' ').count(), s.split_whitespace().count(), "{s}");
                assert_ne!(crate::normalize::normalize(&c), crate::normalize::normalize(s), "{s}");
            }
        }
    }

    #[test]
    fn layouts_round_trip() {
        for (i, layout) in LAYOUTS.iter().enumerate().filter(|(_, l)| l.to_latin) {
            for (key, out) in layout.keys {
                if key.is_ascii_alphabetic() {
                    let typed = format!("{key}{key}");
                    let there = to_layout(layout, &typed);
                    assert_eq!(to_latin(layout, &there), typed.to_lowercase(), "layout {i}: {key} {out}");
                }
            }
        }
    }

    #[test]
    fn known_extension_anywhere_is_kept() {
        assert_eq!(convert_layout(".pdf hgjrvdv").as_deref(), Some(".pdf التقرير"));
    }

    fn round_trip(s: &str) -> Option<String> {
        convert_layout(&convert_layout(s)?)
    }

    #[test]
    fn round_trips() {
        // Every unshifted key (g is never followed by h: "gh" reads back as لا → b).
        let all_keys = "qwertyuiop[]asdfgjklh;'zxcvbnm,./`";
        assert_eq!(round_trip(all_keys).as_deref(), Some(all_keys));
        for s in ["hgjrvdv ext:pdf", "folder: lgtJ ,'kd", "b lhg", "hgjrvdv .pdf"] {
            assert_eq!(round_trip(s).as_deref(), Some(s.to_lowercase().as_str()), "{s}");
        }
        // Shifted keys (أ إ آ لأ…) read back unshifted, so only plain letters round-trip.
        for s in ["التقرير", "لا ملف", "عقد ايجار", "مجلد: ظرف"] {
            assert_eq!(round_trip(s).as_deref(), Some(s), "{s}");
        }
    }

    #[test]
    fn random_input_never_panics() {
        let pieces = [
            "h", "g", "J", "b", "T", "[", ";", "'", ".", "/", "`", "ا", "ل", "لا", "لأ", "أ", "ى", "ة", "ؤ", "ء", " ", "  ",
            "ext:", "*.", ".pdf", "folder:", "مجلد:", "\u{064E}", "\u{0640}", "😀", "\u{200D}", "\u{0301}", "ڤ", "İ", "1",
            "\t", "\u{FEFB}", "\u{0}",
        ];
        let mut s: u64 = 0xA5A5_5A5A_1234_4321;
        let mut next = |n: usize| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s % n as u64) as usize
        };
        for _ in 0..30_000 {
            let input: String = (0..next(10)).map(|_| pieces[next(pieces.len())]).collect();
            if let Some(out) = convert_layout(&input) {
                assert_ne!(out, input.split_whitespace().collect::<Vec<_>>().join(" "));
                assert_eq!(out.split(' ').count(), input.split_whitespace().count(), "{input:?}");
            }
        }
    }

    #[test]
    fn converted_text_matches_after_normalising() {
        use crate::normalize::normalize;
        assert_eq!(normalize(&convert_layout("hgjrvdv").unwrap()), normalize("التقرير"));
        assert_eq!(normalize(&convert_layout("lsjatn").unwrap()), normalize("مستشفى"));
    }

}
