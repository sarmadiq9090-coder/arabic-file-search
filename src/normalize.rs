//! Text normalisation used for matching, so that Arabic queries find words
//! regardless of hamza forms, tashkeel (diacritics), tatweel, presentation
//! forms, Persian/Urdu letter variants, or letter case.
//!
//! Hamza seats: ؤ is و and ئ is ي ([`normalize`]), so a word typed without
//! the hamza («مسوول», «مسايل», «سوال») finds it. The other spellings of a
//! seat are extra forms (`seat_forms`): «مسئول» is also «مسوول» (ئ before
//! و), «رئيس» and «رءيس» are also «ريس» (the seat before ي dropped), and a
//! hamza on the line between letters («رءيس», «رءوف») is also its carrier.
//! A file name keeps them after its normalised name in [`name_key`], and a
//! query adds them to its alternatives, so «مسئول», «مسؤول» and «مسوول» all
//! find each other while و and ي stay different letters everywhere else.
//! A final ئ is also يء (شاطئ = شاطيء). Document text is kept as it is and
//! has no extra forms, so a search inside documents adds to the query the
//! spellings its words may have there ([`document_forms`]).

// Only the Windows UI and the tests use this module.

use unicode_normalization::char::decompose_compatible;

/// Maps a single character to its normalised form. `None` means the
/// character is dropped entirely (diacritics, tatweel, bidi marks).
#[inline]
fn map_char(c: char) -> Option<char> {
    match c {
        // Tashkeel (harakat), superscript alef, Quranic marks and tatweel.
        '\u{064B}'..='\u{065F}' | '\u{0670}' | '\u{06D6}'..='\u{06ED}' | '\u{0640}' => None,
        // Invisible direction / joiner marks, and the name key's separator.
        FORM_SEP | SPELLING_SEP | '\u{061C}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}' => None,
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

/// Separates a name's extra spellings in its [`name_key`]. Windows file
/// names cannot hold it, and normalisation drops it from queries.
pub const FORM_SEP: char = '\u{1}';
/// Starts the name as spelled (ؤ and ئ kept) at the end of a [`name_key`].
/// It never matches more than the normalised name: queries hold no ؤ or ئ.
pub const SPELLING_SEP: char = '\u{2}';

/// Hamza above as a combining mark (decomposed text, e.g. names from macOS,
/// and the decomposition of the presentation forms of ؤ and ئ).
const HAMZA_ABOVE: char = '\u{0654}';

/// Pushes the mapped, lower-cased form of a single character. With
/// `seats`, ؤ and ئ (also as و/ي/ى plus a combining hamza) stay as they are.
#[inline]
fn push_mapped(out: &mut String, c: char, seats: bool) {
    if seats && matches!(c, 'ؤ' | 'ئ') {
        out.push(c);
    } else if seats && c == HAMZA_ABOVE {
        // Same byte length, so a map stays valid.
        if out.ends_with('و') {
            out.pop();
            out.push('ؤ');
        } else if out.ends_with('ي') {
            out.pop();
            out.push('ئ');
        }
    } else if let Some(m) = map_char(c) {
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
fn push_norm(out: &mut String, c: char, seats: bool) -> usize {
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
                push_mapped(out, d, seats);
            }
        });
    } else {
        push_mapped(out, c, seats);
    }
    out.len() - before
}

/// Normalises a string for matching.
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        push_norm(&mut out, c, false);
    }
    out
}

/// Like [`normalize`], but ؤ and ئ keep their seat: the spelling as typed.
/// Same length and character boundaries as [`normalize`] (ؤ/و and ئ/ي are
/// two bytes each).
pub fn normalize_spelling(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        push_norm(&mut out, c, true);
    }
    out
}

fn is_arabic_letter(c: Option<char>) -> bool {
    matches!(c, Some('\u{0620}'..='\u{064A}' | '\u{066E}'..='\u{06D3}'))
}

/// How a hamza seat in a spelling may also be written: `(carrier, alt, drop)`.
/// ئ is ي, also و before و (مسئول = مسؤول) and nothing before ي (رئيس = ريس);
/// at the end of a word it is also يء (شاطئ = شاطيء, مبادئ = مباديء, شئ =
/// شيء), the other way being a plain substring already. ء between two
/// letters is also و before و (رءوف = رؤوف), ي elsewhere (رءيس = رئيس, قراءة
/// = قرائة), and nothing before ي. ؤ is و. In a `query` ؤ before و is also ي
/// and ئ before ة also ء, to reach a text that spells it ئ or ء (شؤون finds
/// شئون and قرائة finds قراءة in a document, whose words have no extra
/// forms). None for every other character.
fn seat(c: char, prev: Option<char>, next: Option<char>, query: bool) -> Option<(char, &'static str, bool)> {
    let alt = |carrier| match next {
        Some('و') => (carrier, "و", false),
        Some('ي') => (carrier, "ي", true),
        _ => (carrier, "ي", false),
    };
    match c {
        'ؤ' if query && next == Some('و') => Some(('و', "ي", false)),
        'ؤ' => Some(('و', "و", false)),
        'ئ' if query && next == Some('ه') => Some(('ي', "ء", false)),
        'ئ' if is_arabic_letter(prev) && !is_arabic_letter(next) => Some(('ي', "يء", false)),
        'ئ' => Some(alt('ي')),
        'ء' if is_arabic_letter(prev) && is_arabic_letter(next) => Some(alt('ء')),
        _ => None,
    }
}

/// The other spellings of a [`normalize_spelling`] text with hamza seats,
/// each with a map like `map` (one source offset per byte), never equal to
/// its normalised form: every seat written its other way, then also the
/// seats before ي dropped. Empty when it has none.
fn forms_mapped(spelling: &str, map: &[u32], query: bool) -> Vec<(String, Vec<u32>)> {
    if !spelling.contains(['ؤ', 'ئ', 'ء']) {
        return Vec::new();
    }
    let chars: Vec<(usize, char)> = spelling.char_indices().collect();
    let mut carrier = String::with_capacity(spelling.len());
    let mut alt = (String::with_capacity(spelling.len()), Vec::with_capacity(map.len()));
    let mut dropped = (String::with_capacity(spelling.len()), Vec::with_capacity(map.len()));
    for (k, &(i, c)) in chars.iter().enumerate() {
        let bytes = &map[i.min(map.len())..(i + c.len_utf8()).min(map.len())];
        let prev = k.checked_sub(1).map(|p| chars[p].1);
        let next = chars.get(k + 1).map(|n| n.1);
        let Some((plain, other, drop)) = seat(c, prev, next, query) else {
            carrier.push(c);
            for f in [&mut alt, &mut dropped] {
                f.0.push(c);
                f.1.extend_from_slice(bytes);
            }
            continue;
        };
        carrier.push(plain);
        // Every byte of a written seat maps to the seat's own source offset.
        let src = bytes.first().copied().unwrap_or(0);
        let fs: &mut [&mut (String, Vec<u32>)] = if drop { &mut [&mut alt] } else { &mut [&mut alt, &mut dropped] };
        for f in fs {
            f.0.push_str(other);
            f.1.extend(std::iter::repeat_n(src, other.len()));
        }
    }
    let mut out: Vec<(String, Vec<u32>)> = Vec::new();
    for f in [alt, dropped] {
        if f.0 != carrier && !out.iter().any(|o| o.0 == f.0) {
            out.push(f);
        }
    }
    out
}

/// Most extra spellings [`document_forms`] gives one query alternative.
pub const MAX_DOCUMENT_FORMS: usize = 4;

/// The spellings a normalised query alternative may have in a document
/// (a document's words have no extra forms, unlike a [`name_key`]):
/// `(forms, ends)`, `ends` being forms that only match where a word ends.
/// A final ء after ي may be the seat ئ: «شيء» is also «شي» (شئ) and
/// «مباديء» «مبادي» (مبادئ), but only at the end of a word, or «شيء»
/// would find «شيخ».
///
/// Each change on its own, then all of them, at most [`MAX_DOCUMENT_FORMS`]
/// forms: never one per combination.
pub fn document_forms(norm: &str) -> (Vec<String>, Vec<String>) {
    let (mut forms, mut ends) = (Vec::new(), Vec::new());
    if !norm.contains('ء') {
        return (forms, ends);
    }
    let chars: Vec<char> = norm.chars().collect();
    // (char index, replacement) of every place a seat may be.
    let mut edits: Vec<(usize, &'static str)> = Vec::new();
    // The letter's place in its word.
    let mut at = 0usize;
    for (k, &c) in chars.iter().enumerate() {
        let prev = k.checked_sub(1).map(|p| chars[p]);
        let next = chars.get(k + 1).copied();
        if !is_arabic_letter(Some(c)) {
            at = 0;
            continue;
        }
        if c == 'ء' && prev == Some('ي') && at >= 2 && !is_arabic_letter(next) {
            edits.push((k, ""));
        }
        at += 1;
    }
    let all: Vec<usize> = (0..edits.len()).collect();
    let singles = (0..edits.len()).map(|e| vec![e]);
    let subsets = singles.chain((edits.len() > 1).then_some(all));
    for subset in subsets {
        if forms.len() + ends.len() >= MAX_DOCUMENT_FORMS {
            break;
        }
        let mut f = String::with_capacity(norm.len() + 2);
        let mut ends_word = false;
        for (k, &c) in chars.iter().enumerate() {
            match subset.iter().map(|&e| edits[e]).find(|&(i, _)| i == k) {
                Some((_, to)) => {
                    f.push_str(to);
                    ends_word |= to.is_empty() && k + 1 == chars.len();
                }
                None => f.push(c),
            }
        }
        let list = if ends_word { &mut ends } else { &mut forms };
        if f != norm && !list.contains(&f) {
            list.push(f);
        }
    }
    (forms, ends)
}

/// The other spellings of a name's hamza seats (see the module notes), as
/// normalised text. Empty when it has none. (A name's are built in
/// [`name_key`]; this is the same list, for the tests.)
#[cfg(test)]
pub fn seat_forms(s: &str) -> Vec<String> {
    forms_of(s, false)
}

/// The other spellings of a query's hamza seats: those of `seat_forms`,
/// and ؤ before و also as ي (for documents, see [`seat`]).
pub fn query_forms(s: &str) -> Vec<String> {
    forms_of(s, true)
}

fn forms_of(s: &str, query: bool) -> Vec<String> {
    let spelling = normalize_spelling(s);
    let map = vec![0; spelling.len()];
    forms_mapped(&spelling, &map, query).into_iter().map(|f| f.0).collect()
}

/// What a file name is matched against: its normalised form, then each of
/// its `seat_forms` after a [`FORM_SEP`], then, when it has ؤ or ئ, its
/// [`normalize_spelling`] after a [`SPELLING_SEP`] (for ranking the name
/// spelled as typed first, without normalising it again for every search).
/// Names without a hamza seat are just their normalised form; [`primary`]
/// gives it back.
pub fn name_key(name: &str) -> String {
    // One pass: the spelling, which is the normalised name unless it has a seat.
    let mut spelling = String::with_capacity(name.len());
    let mut seats = false;
    for c in name.chars() {
        seats |= matches!(c, 'ؤ' | 'ئ' | 'ء' | HAMZA_ABOVE) || is_presentation_form(c);
        push_norm(&mut spelling, c, true);
    }
    if !seats || !has_seat_bytes(&spelling) {
        return spelling;
    }
    let mut key: String = spelling
        .chars()
        .map(|c| match c {
            'ؤ' => 'و',
            'ئ' => 'ي',
            c => c,
        })
        .collect();
    let plain = key.len();
    for (f, _) in forms_mapped(&spelling, &vec![0; spelling.len()], false) {
        key.push(FORM_SEP);
        key.push_str(&f);
    }
    if spelling != key[..plain] {
        key.push(SPELLING_SEP);
        key.push_str(&spelling);
    }
    key
}

/// Where the normalised name ends in a [`name_key`] (the first separator),
/// if it has more. A byte scan: both separators are single bytes.
#[inline]
pub fn key_extras_at(key: &str) -> Option<usize> {
    key.bytes().position(|b| b == 1 || b == 2)
}

/// The spellings of the name in a [`name_key`] that queries match: the
/// normalised name and its other seat spellings (not the name as spelled).
pub fn key_forms(key: &str) -> impl Iterator<Item = &str> {
    key.split(SPELLING_SEP).next().unwrap_or_default().split(FORM_SEP)
}

/// The name as spelled in a [`name_key`]: its [`normalize_spelling`] (which
/// is the normalised name when it has no ؤ or ئ).
pub fn key_spelling(key: &str) -> &str {
    match key.rfind(SPELLING_SEP) {
        Some(i) => &key[i + 1..],
        None => primary(key),
    }
}

/// True when a normalised text holds ء, ؤ or ئ (D8 A1, D8 A4, D8 A6 in
/// UTF-8): a byte scan, cheap enough for every name and every hit.
pub fn has_seat_bytes(s: &str) -> bool {
    s.as_bytes().windows(2).any(|w| w[0] == 0xD8 && matches!(w[1], 0xA1 | 0xA4 | 0xA6))
}

/// True when a raw text may have a hamza seat once normalised: one of the
/// seat letters, a combining hamza (D9 94) or a presentation form (EF …).
fn may_have_seats(s: &str) -> bool {
    has_seat_bytes(s) || s.as_bytes().windows(2).any(|w| w == [0xD9, 0x94] || w[0] == 0xEF)
}

/// The normalised name in a [`name_key`] (the part before any extra form).
pub fn primary(key: &str) -> &str {
    &key[..key_extras_at(key).unwrap_or(key.len())]
}

/// The normalised text and its other seat spellings, each with the source
/// offset of every byte (see [`normalize_with_map`]): for underlining what
/// matched in a name whatever spelling the query used.
pub fn forms_with_map(s: &str) -> Vec<(String, Vec<u32>)> {
    let (norm, map) = normalize_with_map(s);
    let mut out = Vec::new();
    if may_have_seats(s) {
        let mut spelling = String::with_capacity(s.len());
        for c in s.chars() {
            push_norm(&mut spelling, c, true);
        }
        out = forms_mapped(&spelling, &map, false);
    }
    out.insert(0, (norm, map));
    out
}

/// Normalises a string and also returns, for every byte of the output, the
/// byte offset in `s` of the character it came from. Used to cut a readable
/// snippet out of the original text around a match.
pub fn normalize_with_map(s: &str) -> (String, Vec<u32>) {
    let mut out = String::with_capacity(s.len());
    let mut map = Vec::with_capacity(s.len());
    for (i, c) in s.char_indices() {
        let n = push_norm(&mut out, c, false);
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
    fn hamza_seats_and_their_other_spellings() {
        // The carrier: a word typed without the hamza is the same word.
        assert_eq!(normalize("مسؤول"), "مسوول");
        assert_eq!(normalize("مسائل"), normalize("مسايل"));
        assert_eq!(normalize("سؤال"), normalize("سوال"));
        assert_eq!(normalize("بيئة"), normalize("بيية"));
        assert_eq!(normalize("مسئول"), "مسيول");
        // The other spellings: ئ before و, a seat before ي, ء between letters.
        assert_eq!(seat_forms("مسئول"), ["مسوول"]);
        assert_eq!(seat_forms("شئون"), ["شوون"]);
        assert_eq!(seat_forms("رئيس"), ["ريس"]);
        assert_eq!(seat_forms("رءيس"), ["رييس", "ريس"]);
        assert_eq!(seat_forms("رءوف"), ["رووف"]);
        assert_eq!(query_forms("مسؤول"), ["مسيول"]);
        assert_eq!(query_forms("مسئول"), ["مسوول"]);
        assert!(query_forms("سؤال").is_empty());
        for none in ["مسؤول", "سؤال", "مسائل", "بيئة", "قائمة", "سماء", "شيء", "ءاب", "تقرير", "report"] {
            assert!(seat_forms(none).is_empty(), "{none}");
        }
        // Diacritics and tatweel do not hide the next letter.
        assert_eq!(seat_forms("مَسْئُـول"), ["مسوول"]);
        // Combining hamza (decomposed names) and the presentation forms.
        assert_eq!(normalize_spelling("مسي\u{0654}ول"), "مسئول");
        assert_eq!(normalize_spelling("مسو\u{0654}ول"), "مسؤول");
        assert_eq!(seat_forms("مسي\u{0654}ول"), ["مسوول"]);
        assert_eq!(normalize_spelling("\u{FE85}\u{FE8B}"), "ؤئ");
        // The key holds the name, then its other spellings.
        assert_eq!(name_key("مسئول.txt"), "مسيول.txt\u{1}مسوول.txt\u{2}مسئول.txt");
        assert_eq!(primary(&name_key("مسئول.txt")), "مسيول.txt");
        assert_eq!(key_spelling(&name_key("مسئول.txt")), "مسئول.txt");
        assert_eq!(name_key("مسؤول"), "مسوول\u{2}مسؤول");
        assert_eq!(name_key("رءيس"), "رءيس\u{1}رييس\u{1}ريس");
        assert_eq!(key_spelling(&name_key("رءيس")), "رءيس");
        assert_eq!(name_key("سماء"), "سماء");
        assert_eq!(name_key("Report.PDF"), "report.pdf");
        assert_eq!(normalize("a\u{1}b\u{2}"), "ab");
    }

    #[test]
    fn word_final_seats() {
        // A final ئ is also يء (and the other way round is a plain part of it).
        assert_eq!(seat_forms("شاطئ"), ["شاطيء"]);
        assert_eq!(seat_forms("مبادئ عامة"), ["مباديء عامه"]);
        assert_eq!(seat_forms("شئ.txt"), ["شيء.txt"]);
        assert_eq!(query_forms("شاطئ"), ["شاطيء"]);
        for none in ["شاطيء", "شيء", "ئ", "سماء", "قارئة"] {
            assert!(seat_forms(none).is_empty(), "{none}");
        }
        assert_eq!(name_key("شاطئ.txt"), "شاطي.txt\u{1}شاطيء.txt\u{2}شاطئ.txt");
        // ءة / ئة: a query with ئ before ة also has ء (قرائة finds قراءة).
        assert_eq!(seat_forms("قراءة"), ["قرايه"]);
        assert_eq!(query_forms("قرائة"), ["قراءه"]);
        assert!(seat_forms("قرائة").is_empty());
        let forms = forms_with_map("شئ هنا");
        assert_eq!(forms[1].0, "شيء هنا");
        assert_eq!(forms[1].1, [0, 0, 2, 2, 2, 2, 4, 5, 5, 7, 7, 9, 9]);
    }

    #[test]
    fn document_forms_of_a_query() {
        let df = |s: &str| document_forms(&normalize(s));
        let none = (Vec::<String>::new(), Vec::<String>::new());
        assert_eq!(df("شيء"), (vec![], vec!["شي".to_string()]));
        assert_eq!(df("مباديء"), (vec![], vec!["مبادي".to_string()]));
        // Inside a phrase the next character ends the word already.
        assert_eq!(df("شيء جديد"), (vec!["شي جديد".to_string()], vec![]));
        // Each change, then all of them.
        assert_eq!(df("شيء وشيء"), (vec!["شي وشيء".to_string()], vec!["شيء وشي".to_string(), "شي وشي".to_string()]));
        for w in ["سماء", "شيئا", "تقرير", "report", ""] {
            assert_eq!(df(w), none, "{w}");
        }
        let long = "شيء ".repeat(100);
        let (f, e) = document_forms(&long);
        assert_eq!(f.len() + e.len(), MAX_DOCUMENT_FORMS);
    }

    #[test]
    fn forms_map_back_to_source() {
        let src = "مسئول ورئيس";
        let forms = forms_with_map(src);
        assert_eq!(forms.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), ["مسيول ورييس", "مسوول ورييس", "مسوول وريس"]);
        for (n, map) in &forms {
            assert_eq!(map.len(), n.len());
            assert!(map.windows(2).all(|w| w[0] <= w[1]));
        }
        let (n, map) = &forms[2];
        let at = n.find("ريس").unwrap();
        assert!(src[map[at] as usize..].starts_with("رئيس"));
        assert_eq!(forms_with_map("تقرير").len(), 1);
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

    /// Random strings from the code point ranges normalisation treats
    /// specially (and any scalar value now and then), same on every run.
    fn random_strings(seed: u64, count: usize) -> Vec<String> {
        const RANGES: &[(u32, u32)] = &[
            (0x20, 0x7E),
            (0xC0, 0x24F),
            (0x300, 0x36F),
            (0x390, 0x3CF),
            (0x600, 0x6FF),
            (0x200B, 0x206F),
            (0xFB00, 0xFB06),
            (0xFB50, 0xFDFF),
            (0xFE70, 0xFEFF),
            (0x1F468, 0x1F469),
            (0x200D, 0x200D),
        ];
        let mut s = seed;
        let mut next = |n: u32| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s % u64::from(n)) as u32
        };
        (0..count)
            .map(|_| {
                (0..next(24))
                    .map(|_| {
                        let c = if next(40) == 0 {
                            next(0x11_0000)
                        } else {
                            let (lo, hi) = RANGES[next(RANGES.len() as u32) as usize];
                            lo + next(hi - lo + 1)
                        };
                        char::from_u32(c).unwrap_or('\u{FFFD}')
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn random_text_forms_invariants() {
        for s in random_strings(0x5EED_0002, 20_000) {
            let key = name_key(&s);
            assert_eq!(primary(&key), normalize(&s), "{s:?}");
            assert_eq!(key_spelling(&key), normalize_spelling(&s), "{s:?}");
            assert_eq!(normalize_spelling(&s).len(), normalize(&s).len(), "{s:?}");
            let forms = forms_with_map(&s);
            let matched = key.split(SPELLING_SEP).next().unwrap();
            assert_eq!(matched, forms.iter().map(|f| f.0.as_str()).collect::<Vec<_>>().join("\u{1}"), "{s:?}");
            for (n, map) in &forms {
                assert_eq!(map.len(), n.len(), "{s:?}");
                assert!(map.windows(2).all(|w| w[0] <= w[1]), "{s:?}");
                assert!(n.char_indices().all(|(i, _)| s.is_char_boundary(map[i] as usize)), "{s:?}");
            }
        }
    }

    #[test]
    fn random_text_map_invariants() {
        for s in random_strings(0x5EED_0001, 20_000) {
            let (n, map) = normalize_with_map(&s);
            assert_eq!(n, normalize(&s), "{s:?}");
            assert_eq!(map.len(), n.len(), "{s:?}");
            assert!(map.windows(2).all(|w| w[0] <= w[1]), "{s:?}");
            assert!(map.iter().all(|&m| s.is_char_boundary(m as usize) && (m as usize) < s.len()), "{s:?}");
            // Every normalised character starts at a byte that maps to a source character start.
            assert!(n.char_indices().all(|(i, _)| s.is_char_boundary(map[i] as usize)), "{s:?}");
            // Nothing that normalisation drops survives it.
            assert!(!n.contains(['\u{0640}', '\u{064E}', '\u{200F}', '\u{FEFF}']), "{s:?}");
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
