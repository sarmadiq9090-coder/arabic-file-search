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
//!
//! The other spellings of a query ([`query_forms`], [`document_forms`]) are
//! lossy: «ريس», the seat of «رئيس» dropped, is also the end of «إدريس» and
//! «باريس», and «نهاي» («نهائي») the start of «نهاية». So a search matches
//! them only as whole words (after an optional article), never inside
//! another word, and never for an excluded word; the query as typed keeps
//! matching anywhere (see `search::Term`).
//!
//! Letters of Persian, Iraqi and Kurdish spelling are their Arabic letter
//! everywhere ([`normalize`]): ی ێ → ي, ک گ → ك, ۆ → و, ە → ه, چ → ج, پ →
//! ب, ڤ → ف, ژ → ز, ڕ → ر, ڵ → ل. Arabic spelling writes these sounds with
//! that letter (بيتزا, فيديو, كراج), so each pair is one word spelled two
//! ways. گ is also the Iraqi ق (گال = قال): a query's گ is also ق, as a
//! whole word only ([`query_forms`]). So are the Urdu, Pashto, Sindhi and
//! Uyghur letters that are a base letter with dots or a mark (ٹ ټ → ت, ں ڼ →
//! ن, ې → ي, ۇ ۋ → و…); letters that are not (Urdu ے, Pashto ښ ږ ځ څ) stay.
//!
//! Other alphabets, as search engines fold them: Latin and Greek without
//! accents (é → e, ş → s, İ ı → i, ß → ss, ά → α, final ς → σ), Cyrillic ё →
//! е and ґ → г (й stays), Syriac without its vowel points, full-width and
//! half-width forms as NFKC has them, katakana as hiragana, Korean and the
//! Indic scripts composed (NFC: decomposed names from macOS match), without
//! ZWJ/ZWNJ or nukta (क़ → क, ਖ਼ → ਖ), and Malayalam chillu letters as the
//! consonant and virama. Chinese Simplified and Traditional stay different.
//! The letters folded into another letter of their alphabet (ş, پ, a nukta)
//! are kept in the spelling as typed ([`normalize_spelling`]), so names
//! spelled as the query is come first.

// Only the Windows UI and the tests use this module.

use unicode_normalization::char::{compose, decompose_canonical, decompose_compatible};

/// What a character becomes in the normalised text, before lower-casing.
enum Fold {
    Drop,
    One(char),
    Two(char, char),
}

/// The fold of a single non-ASCII character (see the module notes).
#[inline]
fn fold(c: char) -> Fold {
    use Fold::{Drop, One, Two};
    match c {
        // Tashkeel (harakat), superscript alef, Quranic marks, tatweel and
        // the Kazakh/Uyghur high hamza.
        '\u{064B}'..='\u{065F}' | '\u{0670}' | '\u{06D6}'..='\u{06ED}' | '\u{0640}' | '\u{0674}' => Drop,
        // Invisible direction / joiner marks (ZWNJ and ZWJ too, which Indic
        // and Persian text holds or not as it was typed).
        '\u{061C}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}' => Drop,
        // All alef forms to bare alef.
        'أ' | 'إ' | 'آ' | 'ٱ' | 'ٲ' | 'ٳ' => One('ا'),
        'ة' | 'ۃ' => One('ه'),
        // Yeh forms: Farsi yeh, Kurdish ێ (ê), Pashto ې and ۍ.
        'ى' | 'ئ' | 'ی' | 'ێ' | 'ې' | 'ۍ' => One('ي'),
        // Waw forms: Kurdish and Uyghur ۆ ۇ ۈ ۋ, and the rarer ones.
        'ؤ' | 'ۆ' | 'ۇ' | 'ۈ' | 'ۉ' | 'ۊ' | 'ۋ' | 'ۅ' | 'ۏ' => One('و'),
        // Keheh, swash kaf, gaf (Iraqi, Persian and Kurdish g; Arabic
        // spelling writes it ك: كراج, كوكل) and Sindhi ڳ. A query's گ is
        // also ق as a whole word ([`query_forms`]).
        'ک' | 'ڪ' | 'گ' | 'ڳ' => One('ك'),
        // Persian/Urdu heh forms (ە is also the Kurdish vowel e).
        'ە' | 'ہ' | 'ھ' | 'ۀ' => One('ه'),
        // The other Persian, Iraqi, Kurdish, Urdu, Pashto and Sindhi
        // letters that are a base letter with extra dots or a mark, written
        // in Arabic with that letter: چ (ch, چاي/جاي), پ (p, پيتزا/بيتزا),
        // ڤ (v, ڤيديو/فيديو), ژ (zh), Kurdish ڕ and ڵ, the retroflex ٹ ڈ ڑ
        // (Urdu; ټ ډ ړ in Pashto, ٽ ڊ ڙ in Sindhi), noon ghunna ں and the
        // retroflex noons, and the Sindhi aspirates and implosives.
        'چ' | 'ڄ' | 'ڇ' => One('ج'),
        'پ' | 'ٻ' | 'ڀ' => One('ب'),
        'ڤ' | 'ڦ' => One('ف'),
        'ژ' => One('ز'),
        'ڕ' | 'ڑ' | 'ړ' | 'ڙ' => One('ر'),
        'ڵ' => One('ل'),
        'ٹ' | 'ټ' | 'ٽ' | 'ٿ' | 'ٺ' => One('ت'),
        'ڈ' | 'ډ' | 'ڊ' | 'ڌ' | 'ڍ' => One('د'),
        'ں' | 'ڻ' | 'ڼ' | '\u{0768}' => One('ن'),
        // Urdu yeh barree with hamza is yeh barree (ے stays: بچے is not بچی).
        'ۓ' => One('ے'),
        // Arabic-Indic and Persian digits to ASCII digits, and the Arabic
        // decimal and thousands separators (١٢٫٥, ١٬٠٠٠) to `.` and `,`.
        '\u{0660}'..='\u{0669}' => One(char::from_u32(c as u32 - 0x0660 + '0' as u32).unwrap_or(c)),
        '\u{06F0}'..='\u{06F9}' => One(char::from_u32(c as u32 - 0x06F0 + '0' as u32).unwrap_or(c)),
        '\u{066B}' => One('.'),
        '\u{066C}' => One(','),
        '\u{0600}'..='\u{06FF}' => One(c),
        // Latin and Greek: the letter without its accents (é → e, ş → s,
        // İ → i, ά → α), the letters that are not a base letter plus a mark,
        // and the final sigma.
        'ß' | 'ẞ' => Two('s', 's'),
        'æ' | 'Æ' => Two('a', 'e'),
        'œ' | 'Œ' => Two('o', 'e'),
        'ø' | 'Ø' => One('o'),
        'đ' | 'Đ' => One('d'),
        'ł' | 'Ł' => One('l'),
        'ħ' | 'Ħ' => One('h'),
        'ı' => One('i'),
        'ς' => One('σ'),
        '\u{00C0}'..='\u{024F}' | '\u{1E00}'..='\u{1EFF}' | '\u{0386}'..='\u{03CE}' | '\u{1F00}'..='\u{1FFF}' => match base_letter(c) {
            // ǿ is ø with an accent: folded again.
            b if b != c => fold(b),
            _ => One(c),
        },
        // Accents as combining marks (decomposed text, names from macOS).
        '\u{0300}'..='\u{036F}' => Drop,
        // Cyrillic: ё is е (often written so), the accented е and и, and
        // Ukrainian ґ (written г for decades). й stays: мой is not мои.
        'ё' | 'Ё' | 'ѐ' | 'Ѐ' => One('е'),
        'ѝ' | 'Ѝ' => One('и'),
        'ґ' | 'Ґ' => One('г'),
        // Syriac vowel points and marks, the abbreviation mark, and the
        // final semkath (its only letter with a separate final form).
        '\u{070F}' | '\u{0730}'..='\u{074A}' => Drop,
        '\u{0724}' => One('\u{0723}'),
        // Indic nukta (Devanagari, Bengali, Gurmukhi, Gujarati, Odia,
        // Telugu, Kannada): the letter without it (क़ → क, ਖ਼ → ਖ).
        _ if is_nukta(c) => Drop,
        _ if is_nukta_letter(c) => One(base_letter(c)),
        // Malayalam chillu letters are the consonant and virama (the same
        // letters were once typed with a ZWJ, which is dropped), and so is
        // Bengali khanda ta.
        '\u{0D7A}' => Two('\u{0D23}', '\u{0D4D}'),
        '\u{0D7B}' => Two('\u{0D28}', '\u{0D4D}'),
        '\u{0D7C}' => Two('\u{0D30}', '\u{0D4D}'),
        '\u{0D7D}' => Two('\u{0D32}', '\u{0D4D}'),
        '\u{0D7E}' => Two('\u{0D33}', '\u{0D4D}'),
        '\u{0D7F}' => Two('\u{0D15}', '\u{0D4D}'),
        '\u{0D54}' => Two('\u{0D2E}', '\u{0D4D}'),
        '\u{0D55}' => Two('\u{0D2F}', '\u{0D4D}'),
        '\u{0D56}' => Two('\u{0D34}', '\u{0D4D}'),
        '\u{09CE}' => Two('\u{09A4}', '\u{09CD}'),
        // Katakana is hiragana (カタカナ = かたかな); the prolonged sound
        // mark ー stays.
        '\u{30A1}'..='\u{30F6}' => One(char::from_u32(c as u32 - 0x60).unwrap_or(c)),
        'ヽ' => One('ゝ'),
        'ヾ' => One('ゞ'),
        // The ideographic space.
        '\u{3000}' => One(' '),
        _ => One(c),
    }
}

/// A precomposed letter's base letter (é → e): the first character of its
/// canonical decomposition.
#[inline]
fn base_letter(c: char) -> char {
    let mut base = None;
    decompose_canonical(c, |d| {
        base.get_or_insert(d);
    });
    base.unwrap_or(c)
}

/// The Indic nukta signs.
#[inline]
fn is_nukta(c: char) -> bool {
    matches!(c, '\u{093C}' | '\u{09BC}' | '\u{0A3C}' | '\u{0ABC}' | '\u{0B3C}' | '\u{0C3C}' | '\u{0CBC}')
}

/// Indic letters that are a letter plus a nukta (ऩ ऱ ऴ क़…य़, ड़ ढ़ য়, ਲ਼ ਸ਼ ਖ਼…ਫ਼, ଡ଼ ଢ଼).
#[inline]
fn is_nukta_letter(c: char) -> bool {
    matches!(
        c,
        '\u{0929}' | '\u{0931}' | '\u{0934}' | '\u{0958}'..='\u{095F}' | '\u{09DC}' | '\u{09DD}' | '\u{09DF}' | '\u{0A33}' | '\u{0A36}'
            | '\u{0A59}'..='\u{0A5B}' | '\u{0A5E}' | '\u{0B5C}' | '\u{0B5D}'
    )
}

/// Letters that the normalised text folds into a different letter of their
/// own alphabet (ş into s, پ into ب), which the spelling as typed
/// keeps ([`normalize_spelling`]), so that names spelled as the query is rank
/// first. Not the forms of one letter (ی and ي, final σ, katakana and
/// hiragana, a chillu and its consonant), which are the same letter, nor
/// the Cyrillic ё, ѐ, ѝ and ґ (which «и + ̆ = й» would compose differently).
#[inline]
fn spelled_letter(c: char) -> bool {
    match c {
        // The Arabic alphabet (most names): none.
        '\u{0621}'..='\u{064A}' => false,
        'گ' | 'چ' | 'پ' | 'ڤ' | 'ژ' | 'ڕ' | 'ڵ' | 'ێ' | 'ۆ' | 'ې' | 'ۍ' | 'ۇ' | 'ۈ' | 'ۉ' | 'ۊ' | 'ۋ' | 'ۅ' | 'ۏ' | 'ٹ' | 'ټ' | 'ٽ' | 'ٿ'
        | 'ٺ' | 'ڈ' | 'ډ' | 'ڊ' | 'ڌ' | 'ڍ' | 'ڑ' | 'ړ' | 'ڙ' | 'ں' | 'ڻ' | 'ڼ' | '\u{0768}' | 'ٻ' | 'ڀ' | 'ڄ' | 'ڇ' | 'ڳ' | 'ڦ' => true,
        'ς' => false,
        '\u{00C0}'..='\u{024F}' | '\u{1E00}'..='\u{1EFF}' | '\u{0386}'..='\u{03CE}' | '\u{1F00}'..='\u{1FFF}' => true,
        _ => is_nukta(c) || is_nukta_letter(c),
    }
}

/// Characters a normalised text composes with the one before it (NFC):
/// the kana voicing marks (ｶﾞ is ガ), the Hangul vowels and final
/// consonants (decomposed Korean names, e.g. from macOS), and the second
/// part of the Indic two-part vowel signs (ে + া is ো). The accents are
/// [`push_accent`]'s.
#[inline]
fn composes(c: char) -> bool {
    matches!(
        c,
        '\u{3099}' | '\u{309A}' | '\u{1161}'..='\u{1175}' | '\u{11A8}'..='\u{11C2}' | '\u{09BE}' | '\u{09D7}' | '\u{0B3E}' | '\u{0B56}'
            | '\u{0B57}' | '\u{0BBE}' | '\u{0BD7}' | '\u{0C56}' | '\u{0CC2}' | '\u{0CD5}' | '\u{0CD6}' | '\u{0D3E}' | '\u{0D57}'
    )
}

/// An accent as a combining mark (U+0300–U+036F, decomposed text such as
/// names from macOS): composed with the letter before it, then folded as
/// that letter would be, so «e + ́» is «e» (the spelling as typed keeps «é»)
/// and «и + ̆» is «й». Dropped when nothing composes.
#[inline]
fn push_accent(out: &mut String, c: char, mode: Mode) {
    let Some(prev) = out.chars().next_back() else { return };
    let Some(k) = compose(prev, c) else { return };
    let mut composed = String::new();
    if mode == Mode::Typed && spelled_letter(k) {
        push_lower(&mut composed, k);
    } else {
        push_fold(&mut composed, k);
    }
    // Never shorter, so a map stays valid.
    if composed.len() >= prev.len_utf8() {
        out.truncate(out.len() - prev.len_utf8());
        out.push_str(&composed);
    }
}

/// Replaces the last character of `out` with its composition with `c`, if
/// it has one (never shorter, so a map stays valid).
#[inline]
fn compose_last(out: &mut String, c: char) -> bool {
    let Some(prev) = out.chars().next_back() else { return false };
    match compose(prev, c) {
        Some(k) if k.len_utf8() >= prev.len_utf8() => {
            out.truncate(out.len() - prev.len_utf8());
            out.push(k);
            true
        }
        _ => false,
    }
}

/// Compatibility forms decomposed as NFKC does: Latin ligatures (ﬁ),
/// Arabic presentation forms (contextual shapes and ligatures, which some
/// PDFs and old software store instead of the base letters), the CJK
/// compatibility ideographs, and the full-width and half-width forms
/// (ＡＢＣ１２３ is ABC123, ｶﾀｶﾅ is カタカナ).
#[inline]
fn is_compat_form(c: char) -> bool {
    matches!(c, '\u{F900}'..='\u{FAFF}' | '\u{FB00}'..='\u{FB06}' | '\u{FB50}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFE}' | '\u{FF01}'..='\u{FFEE}')
}

/// Separates a name's extra spellings in its [`name_key`]. Windows file
/// names cannot hold it, and normalisation drops it from queries.
pub const FORM_SEP: char = '\u{1}';
/// Starts the name as spelled ([`normalize_spelling`]) at the end of a
/// [`name_key`]. It never matches more than the normalised name: queries
/// are matched against the normalised forms only.
pub const SPELLING_SEP: char = '\u{2}';

/// Gaf, kept like ؤ and ئ in the text the other spellings of a query are
/// made from: it is ك in the normalised text, and a query's گ is also ق
/// (Iraqi گال = قال, گلب = قلب), matched only as a whole word
/// ([`query_forms`]).
const GAF: char = 'گ';

/// Hamza above as a combining mark (decomposed text, e.g. names from macOS,
/// and the decomposition of the presentation forms of ؤ and ئ).
const HAMZA_ABOVE: char = '\u{0654}';

/// How [`push_norm`] normalises.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Every fold: [`normalize`].
    Plain,
    /// As `Plain`, but ؤ, ئ (also as و/ي/ى plus a combining hamza) and گ
    /// stay: the same bytes as `Plain` (each is two bytes, as و, ي and ك).
    /// The text the other spellings of a seat are made from.
    Seats,
    /// As `Seats`, and the [`spelled_letter`]s stay too, composed: the
    /// spelling as typed ([`normalize_spelling`]), for ranking only.
    Typed,
}

#[inline]
fn push_lower(out: &mut String, c: char) {
    if c.is_ascii() {
        out.push(c.to_ascii_lowercase());
    } else {
        out.extend(c.to_lowercase());
    }
}

/// Pushes the normalised, lower-cased form of a single character.
#[inline]
fn push_mapped(out: &mut String, c: char, mode: Mode) {
    if c.is_ascii() {
        if c != FORM_SEP && c != SPELLING_SEP {
            out.push(c.to_ascii_lowercase());
        }
        return;
    }
    if mode == Mode::Plain && ('\u{0600}'..='\u{06FF}').contains(&c) {
        // Arabic: no accent or composition to handle.
        push_fold(out, c);
        return;
    }
    if mode != Mode::Plain {
        if matches!(c, 'ؤ' | 'ئ' | GAF) {
            out.push(c);
            return;
        }
        if c == HAMZA_ABOVE {
            // Same byte length, so a map stays valid.
            if out.ends_with('و') {
                out.pop();
                out.push('ؤ');
            } else if out.ends_with('ي') {
                out.pop();
                out.push('ئ');
            }
            return;
        }
        if mode == Mode::Typed && spelled_letter(c) {
            match c {
                'İ' => out.push('i'),
                _ if is_nukta_letter(c) => decompose_canonical(c, |d| out.push(d)),
                _ => push_lower(out, c),
            }
            return;
        }
    }
    if matches!(c, '\u{0300}'..='\u{036F}') {
        push_accent(out, c, mode);
    } else if !(composes(c) && compose_last(out, c)) {
        push_fold(out, c);
    }
}

/// Pushes the lower-cased [`fold`] of `c`.
#[inline]
fn push_fold(out: &mut String, c: char) {
    match fold(c) {
        Fold::Drop => {}
        Fold::One(m) => push_lower(out, m),
        Fold::Two(a, b) => {
            push_lower(out, a);
            push_lower(out, b);
        }
    }
}

/// Pushes the normalised (lower-cased) form of `c` into `out`.
/// Returns how many bytes were appended.
#[inline]
fn push_norm(out: &mut String, c: char, mode: Mode) -> usize {
    let before = out.len();
    if is_compat_form(c) {
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
                push_mapped(out, d, mode);
            }
        });
    } else {
        push_mapped(out, c, mode);
    }
    out.len() - before
}

/// Pushes the [`normalize`]d form of the one character `c` onto `out`
/// (`normalize` of a text is these pieces one after another). It may
/// replace the last character it pushed before (a composition), so the
/// caller keeps building in the same buffer.
#[inline]
pub fn push_normalized(out: &mut String, c: char) {
    push_norm(out, c, Mode::Plain);
}

fn normalize_mode(s: &str, mode: Mode) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        push_norm(&mut out, c, mode);
    }
    out
}

/// Bumped whenever what [`normalize`] gives for some text changes (a new
/// fold, a letter mapped differently): the content index's word index holds
/// normalised words and is then built again from the stored texts. A test
/// in `content_words` notices a change that forgot to bump it.
pub const VERSION: u32 = 1;

/// Normalises a string for matching.
pub fn normalize(s: &str) -> String {
    normalize_mode(s, Mode::Plain)
}

/// The spelling as typed, for ranking: like [`normalize`] (case, tashkeel,
/// the forms of one letter), but ؤ and ئ keep their seat and the letters of
/// other alphabets that [`normalize`] folds into another letter stay as they
/// are (گ, پ, ş, é, a nukta). Never matched against: it may differ in
/// length from [`normalize`].
pub fn normalize_spelling(s: &str) -> String {
    normalize_mode(s, Mode::Typed)
}

/// [`normalize`] with ؤ, ئ and گ kept (`Mode::Seats`): same bytes.
fn seat_spelling(s: &str) -> String {
    normalize_mode(s, Mode::Seats)
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

/// The other spellings of a [`seat_spelling`] text with hamza seats,
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
            let c = if c == GAF { 'ك' } else { c };
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

/// The spellings a normalised query alternative may have in a document when
/// the query left a hamza out (a document's words have no extra forms, unlike
/// a [`name_key`]). Like every other spelling of a seat they are lossy, so a
/// search matches them only as whole words (see `search::Term::word_alts`).
/// Where a seat may have been typed as its carrier:
/// - و before و may be ئ: «مسوول» is also «مسيول» (مسئول), «شوون» «شيون» (شئون);
/// - a seat before ي may have been left out, in the second letter of a word
///   (also after ال), when a consonant follows: «ريس» is also «رييس» (رئيس),
///   «ضيل» «ضييل» (ضئيل);
/// - a final ء after ي may be the seat ئ: «شيء» is also «شي» (شئ) and
///   «مباديء» «مبادي» (مبادئ).
///
/// Each change on its own, then all of them, at most [`MAX_DOCUMENT_FORMS`]
/// forms: never one per combination.
pub fn document_forms(norm: &str) -> Vec<String> {
    let mut forms = Vec::new();
    if !norm.contains(['و', 'ي', 'ء']) {
        return forms;
    }
    let chars: Vec<char> = norm.chars().collect();
    // (char index, replacement) of every place a seat may be.
    let mut edits: Vec<(usize, &'static str)> = Vec::new();
    // The letter's place in its word, and whether the word starts with ال.
    let (mut at, mut al) = (0usize, false);
    for (k, &c) in chars.iter().enumerate() {
        let prev = k.checked_sub(1).map(|p| chars[p]);
        let next = chars.get(k + 1).copied();
        if !is_arabic_letter(Some(c)) {
            at = 0;
            continue;
        }
        if at == 0 {
            al = c == 'ا' && next == Some('ل');
        }
        let letters_after = chars[k + 1..].iter().take_while(|&&n| is_arabic_letter(Some(n))).count();
        match c {
            'و' if next == Some('و') && is_arabic_letter(prev) && prev != Some('و') => edits.push((k, "ي")),
            'ي' if (at == 1 || al && at == 3)
                && letters_after >= 1
                && !matches!(next, Some('ا' | 'و' | 'ي' | 'ى' | 'ء'))
                && prev != Some('ي') =>
            {
                edits.push((k, "يي"))
            }
            'ء' if prev == Some('ي') && at >= 2 && !is_arabic_letter(next) => edits.push((k, "")),
            _ => {}
        }
        at += 1;
    }
    let all: Vec<usize> = (0..edits.len()).collect();
    let singles = (0..edits.len()).map(|e| vec![e]);
    let subsets = singles.chain((edits.len() > 1).then_some(all));
    for subset in subsets {
        if forms.len() >= MAX_DOCUMENT_FORMS {
            break;
        }
        let mut f = String::with_capacity(norm.len() + 2);
        for (k, &c) in chars.iter().enumerate() {
            match subset.iter().map(|&e| edits[e]).find(|&(i, _)| i == k) {
                Some((_, to)) => f.push_str(to),
                None => f.push(c),
            }
        }
        if f != norm && !forms.contains(&f) {
            forms.push(f);
        }
    }
    forms
}

/// The other spellings of a name's hamza seats (see the module notes), as
/// normalised text. Empty when it has none. (A name's are built in
/// [`name_key`]; this is the same list, for the tests.)
#[cfg(test)]
pub fn seat_forms(s: &str) -> Vec<String> {
    spelling_forms(&seat_spelling(s), false)
}

/// The other spellings of a query: those of its hamza seats (as in
/// `seat_forms`, and ؤ before و also as ي for documents, see [`seat`]),
/// then, when it has گ, the query with ق for it and that one's seat
/// spellings. All are lossy (گول is the Iraqi قول, but also «goal»), so
/// they match only as whole words. The other way round, a typed ق never
/// finds گ: ق is far more common, and گ is ك in every normalised text, so
/// it would also find ك (قلب would find كلب).
pub fn query_forms(s: &str) -> Vec<String> {
    let spelling = seat_spelling(s);
    let mut out = spelling_forms(&spelling, true);
    if spelling.contains(GAF) {
        let qaf = spelling.replace(GAF, "ق");
        for f in std::iter::once(plain(&qaf)).chain(spelling_forms(&qaf, true)) {
            if !out.contains(&f) {
                out.push(f);
            }
        }
    }
    out
}

/// A [`seat_spelling`] text as [`normalize`] would give it.
fn plain(spelling: &str) -> String {
    spelling
        .chars()
        .map(|c| match c {
            'ؤ' => 'و',
            'ئ' => 'ي',
            GAF => 'ك',
            c => c,
        })
        .collect()
}

fn spelling_forms(spelling: &str, query: bool) -> Vec<String> {
    forms_mapped(spelling, &vec![0; spelling.len()], query).into_iter().map(|f| f.0).collect()
}

/// What a file name is matched against: its normalised form, then each of
/// its `seat_forms` after a [`FORM_SEP`], then, when it differs, its
/// [`normalize_spelling`] after a [`SPELLING_SEP`] (for ranking the name
/// spelled as typed first, without normalising it again for every search).
/// Names without a hamza seat or a [`spelled_letter`] are just their
/// normalised form; [`primary`] gives it back.
pub fn name_key(name: &str) -> String {
    // One pass: the seat spelling, which is the normalised name unless it
    // has a seat or گ.
    let mut spelling = String::with_capacity(name.len());
    let (mut seats, mut typed) = (false, false);
    for c in name.chars() {
        if !c.is_ascii() {
            let compat = is_compat_form(c);
            seats |= matches!(c, 'ؤ' | 'ئ' | 'ء' | GAF | HAMZA_ABOVE) || compat;
            typed |= compat || spelled_letter(c) || matches!(c, '\u{0300}'..='\u{036F}');
        }
        push_norm(&mut spelling, c, Mode::Seats);
    }
    if !typed && (!seats || !(has_seat_bytes(&spelling) || spelling.contains(GAF))) {
        return spelling;
    }
    let mut key = plain(&spelling);
    let plain = key.len();
    for (f, _) in forms_mapped(&spelling, &vec![0; spelling.len()], false) {
        key.push(FORM_SEP);
        key.push_str(&f);
    }
    // Without a letter to keep, the spelling as typed is the seat spelling.
    let typed = if typed { normalize_spelling(name) } else { spelling };
    if typed != key[..plain] {
        key.push(SPELLING_SEP);
        key.push_str(&typed);
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
/// is the normalised name when it has no ؤ, ئ or [`spelled_letter`]).
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
        out = forms_mapped(&seat_spelling(s), &map, false);
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
        let n = push_norm(&mut out, c, Mode::Plain);
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
    fn document_forms_of_a_query_without_hamza() {
        let df = |s: &str| document_forms(&normalize(s));
        let v = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(df("مسوول"), v(&["مسيول"]));
        assert_eq!(df("شوون"), v(&["شيون"]));
        assert_eq!(df("ريس"), v(&["رييس"]));
        assert_eq!(df("الريس"), v(&["الرييس"]));
        assert_eq!(df("ضيل"), v(&["ضييل"]));
        assert_eq!(df("شيء"), v(&["شي"]));
        assert_eq!(df("مباديء"), v(&["مبادي"]));
        assert_eq!(df("شيء جديد"), v(&["شي جديد"]));
        // Each change, then all of them.
        assert_eq!(df("مسوول ريس"), v(&["مسيول ريس", "مسوول رييس", "مسيول رييس"]));
        assert_eq!(df("ريس شيء"), v(&["رييس شيء", "ريس شي", "رييس شي"]));
        for w in ["سيارة", "بيوت", "رييس", "تقرير", "مدير", "وو", "report", ""] {
            assert!(df(w).is_empty(), "{w}");
        }
        let long = "مسوول ".repeat(100);
        assert_eq!(document_forms(&long).len(), MAX_DOCUMENT_FORMS);
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

    #[test]
    fn persian_iraqi_kurdish_letters_fold() {
        assert_eq!(normalize("گ چ پ ڤ ژ"), "ك ج ب ف ز");
        assert_eq!(normalize("ی ێ ک ۆ ە ڕ ڵ"), "ي ي ك و ه ر ل");
        assert_eq!(normalize("گراج پيتزا ڤيديو چاي"), "كراج بيتزا فيديو جاي");
        assert_eq!(normalize("سلێمانی ڕۆژ دڵ"), "سليماني روز دل");
        // Presentation forms: پ, چ, ژ, ڤ and گ in their initial shapes.
        assert_eq!(normalize("\u{FB58}\u{FB7C}\u{FB8A}\u{FB6C}\u{FB94}"), "بجزفك");
        // The spelling keeps گ (same length), so names spelled with it rank first.
        assert_eq!(normalize_spelling("گاع"), "گاع");
        assert_eq!(normalize_spelling("گاع").len(), normalize("گاع").len());
        assert_eq!(name_key("گاع.txt"), "كاع.txt\u{2}گاع.txt");
        assert_eq!(primary(&name_key("گاع.txt")), "كاع.txt");
        assert_eq!(name_key("گلب مسئول"), "كلب مسيول\u{1}كلب مسوول\u{2}گلب مسئول");
        assert_eq!(name_key("چاي.txt"), "جاي.txt\u{2}چاي.txt");
        assert_eq!(name_key("جاي.txt"), "جاي.txt");
        // A query's گ is also ق (lossy: whole words only), never the other way.
        assert_eq!(query_forms("گاع"), ["قاع"]);
        assert_eq!(query_forms("الگاع"), ["القاع"]);
        assert_eq!(query_forms("گال مسئول"), ["كال مسوول", "قال مسيول", "قال مسوول"]);
        for none in ["قاع", "كاع", "چاي", "پيتزا"] {
            assert!(query_forms(none).is_empty(), "{none}");
        }
        assert!(seat_forms("گاع").is_empty());
    }

    #[test]
    fn latin_greek_cyrillic_letters_fold() {
        assert_eq!(normalize("Café Ünïcode ÇĞŞÖÜ Ñ âêîôû"), "cafe unicode cgsou n aeiou");
        assert_eq!(normalize("Straße ẞ Æsop Œuvre Øre Đak Łódź Ħal"), "strasse ss aesop oeuvre ore dak lodz hal");
        // Turkish: İ, I, ı and i all match i; Kurmanji ê î û ç ş; Vietnamese.
        assert_eq!(normalize("İSTANBUL Istanbul ıspanak"), "istanbul istanbul ispanak");
        assert_eq!(normalize("Kurmancî: êîûçş"), "kurmanci: eiucs");
        assert_eq!(normalize("Tiếng Việt ǿ"), "tieng viet o");
        // Decomposed accents (names from macOS) are the same.
        assert_eq!(normalize("Cafe\u{301} I\u{307}"), "cafe i");
        // Greek: no tonos or dialytika, final sigma is sigma.
        assert_eq!(normalize("Αθήνα ΐ ΟΔΟΣ οδός"), "αθηνα ι οδοσ οδοσ");
        // Cyrillic: ё is е, ґ is г, й stays й (decomposed too).
        assert_eq!(normalize("Ёлка ёж ґанок"), "елка еж ганок");
        assert_eq!(normalize("мой и\u{306}"), "мой й");
        assert_ne!(normalize("мой"), normalize("мои"));
        // The spelling as typed keeps the letters, composed.
        assert_eq!(normalize_spelling("Café"), "café");
        assert_eq!(normalize_spelling("Cafe\u{301}"), "café");
        assert_eq!(normalize_spelling("İstanbul ıspanak Ş"), "istanbul ıspanak ş");
        assert_eq!(normalize_spelling("Ёлка"), "елка");
        assert_eq!(name_key("Café.txt"), "cafe.txt\u{2}café.txt");
        assert_eq!(name_key("Cafe.txt"), "cafe.txt");
        assert_eq!(name_key("Öğrenci.xlsx"), "ogrenci.xlsx\u{2}öğrenci.xlsx");
        assert_eq!(name_key("Ёлка.png"), "елка.png");
        assert!(query_forms("café").is_empty());
    }

    #[test]
    fn other_arabic_script_letters_fold() {
        // Urdu, Pashto, Sindhi, Uyghur letters that are a base letter with dots or a mark.
        assert_eq!(normalize("ٹ ڈ ڑ ں ۃ"), "ت د ر ن ه");
        assert_eq!(normalize("ټ ډ ړ ڼ ې ۍ"), "ت د ر ن ي ي");
        assert_eq!(normalize("ٻ ڀ ڄ ڇ ڊ ڌ ڍ ڙ ٽ ٿ ٺ ڳ ڦ ڻ \u{0768}"), "ب ب ج ج د د د ر ت ت ت ك ف ن ن");
        assert_eq!(normalize("ۇ ۈ ۉ ۊ ۋ ۅ ۏ"), "و و و و و و و");
        assert_eq!(normalize("ۓ ٴ"), "ے ");
        // Distinct letters stay: Urdu ے (بچے is not بچی), Pashto ښ ږ ځ څ, Sindhi ڃ.
        assert_ne!(normalize("بچے"), normalize("بچی"));
        assert_eq!(normalize("ښ ږ ځ څ ڃ"), "ښ ږ ځ څ ڃ");
        // Urdu: the word as typed in Arabic letters.
        assert_eq!(normalize("ٹوکری"), normalize("توكري"));
        assert_eq!(normalize("پاکستان"), "باكستان");
        assert_eq!(normalize_spelling("ٹوکری"), "ٹوكري");
    }

    #[test]
    fn syriac_marks_and_final_letters() {
        // ܫܠܵܡܵܐ with its vowel points is ܫܠܡܐ; the final semkath is semkath.
        assert_eq!(normalize("\u{072B}\u{0720}\u{0735}\u{0721}\u{0735}\u{0710}"), "\u{072B}\u{0720}\u{0721}\u{0710}");
        assert_eq!(normalize("\u{0724}\u{0741}\u{070F}"), "\u{0723}");
    }

    #[test]
    fn east_asian_forms_fold() {
        // Full-width Latin, digits, punctuation and space.
        assert_eq!(normalize("ＲＥＰＯＲＴ　２０２４！"), "report 2024!");
        assert_eq!(normalize("\u{FB01}le"), "file");
        // Half-width katakana (with its voicing marks) and katakana are hiragana.
        assert_eq!(normalize("ｶﾞｷﾞ ﾊﾟ"), "がぎ ぱ");
        assert_eq!(normalize("カタカナ"), normalize("かたかな"));
        assert_eq!(normalize("コーヒー"), "こーひー"); // the prolonged sound mark stays
        assert_eq!(normalize("ガ"), normalize("カ\u{3099}"));
        assert_eq!(normalize("ヴ"), "ゔ");
        assert_eq!(normalize("ヷ"), "ヷ"); // no hiragana
        // Korean decomposed (NFD) is composed.
        assert_eq!(normalize("\u{1112}\u{1161}\u{11AB}\u{1100}\u{116E}\u{11A8}"), "한국");
        assert_eq!(normalize("한국어"), "한국어");
        // Chinese, Thai and Tifinagh are as they are.
        for s in ["东京 東京", "ภาษาไทย", "ⵜⴰⵎⴰⵣⵉⵖⵜ"] {
            assert_eq!(normalize(s), s);
        }
    }

    #[test]
    fn indic_scripts_fold() {
        // Devanagari: nukta letters (precomposed or not) are the letter; ZWJ/ZWNJ go.
        assert_eq!(normalize("\u{0958}िला"), "किला");
        assert_eq!(normalize("क\u{093C}िला"), "किला");
        assert_eq!(normalize("ज़िंदगी"), normalize("जिंदगी"));
        assert_eq!(normalize("क्\u{200D}ष क्\u{200C}ष"), "क्ष क्ष");
        assert_eq!(normalize("\u{0929}"), "न");
        // Bengali: য় ড় ঢ় are য ড ঢ; two-part vowels composed; khanda ta.
        assert_eq!(normalize("\u{09DF}"), "\u{09AF}");
        assert_eq!(normalize("\u{09AF}\u{09BC}"), "\u{09AF}");
        assert_eq!(normalize("\u{0995}\u{09C7}\u{09BE}"), "\u{0995}\u{09CB}");
        assert_eq!(normalize("\u{0995}\u{09C7}\u{09D7}"), "\u{0995}\u{09CC}");
        assert_eq!(normalize("\u{09CE}"), "\u{09A4}\u{09CD}");
        assert_eq!(normalize("\u{09A4}\u{09CD}\u{200D}"), "\u{09A4}\u{09CD}");
        // Gurmukhi: ਖ਼ ਗ਼ ਜ਼ ਫ਼ ਲ਼ ਸ਼ are ਖ ਗ ਜ ਫ ਲ ਸ.
        assert_eq!(normalize("\u{0A59}\u{0A5A}\u{0A5B}\u{0A5E}\u{0A33}\u{0A36}"), "\u{0A16}\u{0A17}\u{0A1C}\u{0A2B}\u{0A32}\u{0A38}");
        assert_eq!(normalize("\u{0A16}\u{0A3C}"), "\u{0A16}");
        // Gujarati: nukta and joiners.
        assert_eq!(normalize("\u{0A9C}\u{0ABC}\u{0ACD}\u{200D}"), "\u{0A9C}\u{0ACD}");
        // Odia: ଡ଼ ଢ଼, and ୋ ୈ ୌ from their parts.
        assert_eq!(normalize("\u{0B5C}\u{0B5D}"), "\u{0B21}\u{0B22}");
        assert_eq!(normalize("\u{0B15}\u{0B47}\u{0B3E} \u{0B15}\u{0B47}\u{0B56} \u{0B15}\u{0B47}\u{0B57}"), "\u{0B15}\u{0B4B} \u{0B15}\u{0B48} \u{0B15}\u{0B4C}");
        // Tamil: ொ ோ ௌ and ஔ from their parts.
        assert_eq!(normalize("\u{0B95}\u{0BC6}\u{0BBE}"), "\u{0B95}\u{0BCA}");
        assert_eq!(normalize("\u{0B95}\u{0BC7}\u{0BBE}"), "\u{0B95}\u{0BCB}");
        assert_eq!(normalize("\u{0B95}\u{0BC6}\u{0BD7}"), "\u{0B95}\u{0BCC}");
        assert_eq!(normalize("\u{0B92}\u{0BD7}"), "\u{0B94}");
        // Telugu: ై from its parts.
        assert_eq!(normalize("\u{0C15}\u{0C46}\u{0C56}"), "\u{0C15}\u{0C48}");
        // Kannada: ೀ ೇ ೈ ೊ ೋ from their parts.
        assert_eq!(normalize("\u{0C95}\u{0CBF}\u{0CD5}"), "\u{0C95}\u{0CC0}");
        assert_eq!(normalize("\u{0C95}\u{0CC6}\u{0CC2}\u{0CD5}"), "\u{0C95}\u{0CCB}");
        assert_eq!(normalize("\u{0C95}\u{0CC6}\u{0CD6}"), "\u{0C95}\u{0CC8}");
        // Malayalam: chillu letters are consonant + virama (as typed with a ZWJ once); ൊ ോ ൌ.
        assert_eq!(normalize("അവ\u{0D7B}"), "അവ\u{0D28}\u{0D4D}");
        assert_eq!(normalize("അവ\u{0D28}\u{0D4D}\u{200D}"), normalize("അവ\u{0D7B}"));
        for (chillu, base) in [('\u{0D7A}', '\u{0D23}'), ('\u{0D7C}', '\u{0D30}'), ('\u{0D7D}', '\u{0D32}'), ('\u{0D7E}', '\u{0D33}'), ('\u{0D7F}', '\u{0D15}')] {
            assert_eq!(normalize(&chillu.to_string()), format!("{base}\u{0D4D}"));
        }
        assert_eq!(normalize("\u{0D15}\u{0D46}\u{0D3E}"), "\u{0D15}\u{0D4A}");
        assert_eq!(normalize("\u{0D15}\u{0D47}\u{0D3E}"), "\u{0D15}\u{0D4B}");
        assert_eq!(normalize("\u{0D15}\u{0D46}\u{0D57}"), "\u{0D15}\u{0D4C}");
        // The spelling as typed keeps the nukta (decomposed, as NFC has it).
        assert_eq!(normalize_spelling("\u{0958}िला"), "क\u{093C}िला");
        assert_eq!(normalize_spelling("क\u{093C}िला"), "क\u{093C}िला");
        assert_eq!(name_key("\u{0958}िला.pdf"), "किला.pdf\u{2}क\u{093C}िला.pdf");
    }

    #[test]
    fn arabic_and_english_unchanged() {
        for (s, n) in [
            ("التقرير السنوي ٢٠٢٤", "التقرير السنوي 2024"),
            ("إسلام أحمد مدرسة مستشفى", "اسلام احمد مدرسه مستشفي"),
            ("قلم كلم قال كال", "قلم كلم قال كال"),
            ("The Quick Brown Fox_v2.TXT", "the quick brown fox_v2.txt"),
        ] {
            assert_eq!(normalize(s), n, "{s}");
            assert_eq!(normalize_spelling(s), n, "{s}");
            assert_eq!(name_key(s), n, "{s}");
        }
    }

    #[test]
    fn arabic_number_separators_fold() {
        assert_eq!(normalize("١٢٫٥"), "12.5");
        assert_eq!(normalize("١٬٢٥٠٫٧٥"), "1,250.75");
        assert_eq!(normalize("۱۲٫۵"), "12.5");
        let (n, map) = normalize_with_map("س ١٬٢");
        assert_eq!(n, "س 1,2");
        assert_eq!(map, [0, 0, 2, 3, 5, 7]);
    }

    const SAMPLES: &[&str] = &[
        "",
        "Report.PDF",
        "الـتَّقرير السنويّ ٢٠٢٤",
        "إسلام أحمد مستشفى مدرسة",
        "\u{FE8D}\u{FEDF}\u{FE98}\u{FED8}\u{FEAE}\u{FEF3}\u{FEAE} \u{FEFB}\u{FDF2}",
        "کتابی ۱۲۳ \u{200F}x\u{202B}y",
        "گلب مسئول ڕۆژ چاي ١٬٢٥٠٫٧٥",
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
            // Latin Extended Additional, Cyrillic, Syriac, Indic, Thai,
            // Hangul jamo and syllables, kana, CJK, full/half width.
            (0x1E00, 0x1EFF),
            (0x0400, 0x045F),
            (0x0700, 0x074F),
            (0x0900, 0x0D7F),
            (0x0E00, 0x0E7F),
            (0x1100, 0x11FF),
            (0xAC00, 0xAC40),
            (0x3040, 0x30FF),
            (0x4E00, 0x4E20),
            (0xFF00, 0xFFEF),
            (0x0750, 0x077F),
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
            assert_eq!(seat_spelling(&s).len(), normalize(&s).len(), "{s:?}");
            assert_eq!(normalize(&normalize_spelling(&s)), normalize(&s), "{s:?}");
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

    /// Folding cost: the name keys of two million names (all ASCII, all
    /// Arabic, mixed) as an index load builds them, and a long document.
    #[test]
    #[ignore = "benchmark: cargo test --release -- --ignored --nocapture"]
    fn normalize_two_million() {
        let sets: [(&str, &[&str]); 3] = [
            ("ascii", &["Report", "invoice_2024", "IMG_0042.JPG", "budget final", "notes.txt"]),
            ("arabic", &["تقرير", "الميزانية", "فاتورة ٢٠٢٤", "مشروع", "صورة.jpg"]),
            ("mixed", &["تقرير report", "مسئول", "گراج", "café", "東京"]),
        ];
        for (label, words) in sets {
            let names: Vec<String> =
                (0..2_000_000usize).map(|i| format!("{} {} {}", words[i % 5], words[(i / 5) % 5], i % 977)).collect();
            let t = std::time::Instant::now();
            let bytes: usize = names.iter().map(|n| name_key(n).len()).sum();
            println!("name_key {label}: {:?} ({bytes} bytes)", t.elapsed());
        }
        let text = "التقرير السنوي للميزانية Annual budget report 2024 ".repeat(20_000);
        let t = std::time::Instant::now();
        let n = normalize_with_map(&text).0.len();
        println!("normalize_with_map {} KB: {:?} ({n})", text.len() / 1024, t.elapsed());
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
