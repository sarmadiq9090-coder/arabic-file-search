//! The user-interface language.
//!
//! Every user-visible string is written in place as both texts, `tr!("…", "…")`
//! for a fixed text and `trf!("… {x}", "… {x}")` for a `format!`, so the
//! Arabic and English originals sit next to each other. The other languages
//! are translations of the English text, one file each in `i18n/<code>.json`
//! (embedded at build time, see `build.rs`):
//!
//! ```json
//! { "meta": { "name_native": "Français", "name_en": "French", "rtl": false,
//!             "script": "Latn", "font_hint": "", "status": "machine-translated, needs review" },
//!   "rust": { "<English text of a tr!/trf!>": "<translation>" },
//!   "js":   { "<key of app.js's T table>": "<translation>" } }
//! ```
//!
//! In such a language `tr!` gives the translation of its English text and
//! `trf!` fills the translated template ([`fill`]); a text without a
//! translation, or a template whose placeholders don't match the English
//! one, stays English. Adding a language is one file in `i18n/` plus one
//! row in [`LOCALES`]; a row without a file is not offered.
//!
//! The language is chosen once per process: the user's choice saved in the
//! registry (`HKCU\Software\BahthSari`, value `Language` = a code of
//! [`LOCALES`]), otherwise the Windows display language (a language of the
//! table when its file is there, English otherwise). Changing it takes
//! effect at the next start.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

/// Which of the two original texts a language starts from: Arabic, or
/// English (every other language, translated from the English text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Ar,
    En,
}

impl Lang {
    /// The code saved in the registry.
    pub fn code(self) -> &'static str {
        match self {
            Lang::Ar => "ar",
            Lang::En => "en",
        }
    }

    #[cfg(test)]
    pub fn from_code(code: &str) -> Option<Lang> {
        match code.trim().to_ascii_lowercase().as_str() {
            "ar" => Some(Lang::Ar),
            "en" => Some(Lang::En),
            _ => None,
        }
    }

    /// The text for a Windows LANGID: Arabic for any Arabic locale
    /// (primary language 0x01), English for everything else.
    #[cfg(test)]
    pub fn from_langid(langid: u16) -> Lang {
        if langid & 0x3ff == 0x01 { Lang::Ar } else { Lang::En }
    }

    /// English for `true`, Arabic for `false`: for code that is handed the
    /// language as a flag (`english: bool`), so it can still [`Lang::pick`]
    /// or `trf!(in …)` and get the translations.
    pub fn from_english(english: bool) -> Lang {
        if english { Lang::En } else { Lang::Ar }
    }

    /// `ar` in Arabic, `en` in English; in a translated language the
    /// translation of `en` when it is a text that has one.
    pub fn pick<T: Text>(self, ar: T, en: T) -> T {
        match self {
            Lang::Ar => ar,
            Lang::En => en.localized(),
        }
    }
}

/// A language of the interface.
#[derive(Debug, PartialEq, Eq)]
pub struct Locale {
    /// BCP 47 code: saved in the registry, the page's `lang`, the file name.
    pub code: &'static str,
    /// The name in the language itself, as the language pickers show it.
    pub native: &'static str,
    pub english: &'static str,
    pub rtl: bool,
    /// ISO 15924 script (the page picks its font by `lang`).
    pub script: &'static str,
    /// Windows languages it is picked for on a first start: a primary
    /// language (LANGID & 0x3ff) or, above 0x3ff, one exact LANGID (tried
    /// first, so zh-TW can be Traditional while the rest of 0x04 is not).
    pub langids: &'static [u16],
    /// The original text it starts from.
    pub base: Lang,
    /// A machine translation, labelled as one in the pickers.
    pub machine: bool,
}

/// A translated language's row: (code, native name, English name,
/// right to left, script, Windows languages).
const fn tl(code: &'static str, native: &'static str, english: &'static str, rtl: bool, script: &'static str, langids: &'static [u16]) -> Locale {
    Locale { code, native, english, rtl, script, langids, base: Lang::En, machine: true }
}

pub const AR: Locale = Locale { code: "ar", native: "العربية", english: "Arabic", rtl: true, script: "Arab", langids: &[0x01], base: Lang::Ar, machine: false };
pub const EN: Locale = Locale { code: "en", native: "English", english: "English", rtl: false, script: "Latn", langids: &[0x09], base: Lang::En, machine: false };

/// Every language the program knows. A translated one is offered only
/// when its `i18n/<code>.json` was there at build time ([`available`]).
pub static LOCALES: &[Locale] = &[
    AR,
    EN,
    tl("ckb", "کوردی (سۆرانی)", "Kurdish (Sorani)", true, "Arab", &[0x92]),
    tl("kmr", "Kurdî (Kurmancî)", "Kurdish (Kurmanji)", false, "Latn", &[]),
    tl("fa", "فارسی", "Persian", true, "Arab", &[0x29]),
    tl("ur", "اردو", "Urdu", true, "Arab", &[0x20]),
    tl("ps", "پښتو", "Pashto", true, "Arab", &[0x63]),
    tl("ug", "ئۇيغۇرچە", "Uyghur", true, "Arab", &[0x80]),
    tl("syr", "ܣܘܪܝܝܐ", "Syriac (Assyrian Neo-Aramaic)", true, "Syrc", &[0x5A]),
    tl("tr", "Türkçe", "Turkish", false, "Latn", &[0x1F]),
    tl("az", "Azərbaycanca", "Azerbaijani", false, "Latn", &[0x2C]),
    tl("uz", "Oʻzbekcha", "Uzbek", false, "Latn", &[0x43]),
    tl("fr", "Français", "French", false, "Latn", &[0x0C]),
    tl("es", "Español", "Spanish", false, "Latn", &[0x0A]),
    tl("pt", "Português (Brasil)", "Portuguese (Brazil)", false, "Latn", &[0x16]),
    tl("de", "Deutsch", "German", false, "Latn", &[0x07]),
    tl("ru", "Русский", "Russian", false, "Cyrl", &[0x19]),
    tl("uk", "Українська", "Ukrainian", false, "Cyrl", &[0x22]),
    tl("zgh", "ⵜⴰⵎⴰⵣⵉⵖⵜ", "Standard Moroccan Tamazight", false, "Tfng", &[0x5F]),
    tl("sw", "Kiswahili", "Swahili", false, "Latn", &[0x41]),
    tl("ha", "Hausa", "Hausa", false, "Latn", &[0x68]),
    tl("so", "Soomaali", "Somali", false, "Latn", &[0x77]),
    tl("hi", "हिन्दी", "Hindi", false, "Deva", &[0x39]),
    tl("mr", "मराठी", "Marathi", false, "Deva", &[0x4E]),
    tl("bn", "বাংলা", "Bengali", false, "Beng", &[0x45]),
    tl("pa", "ਪੰਜਾਬੀ", "Punjabi (Gurmukhi)", false, "Guru", &[0x46]),
    tl("gu", "ગુજરાતી", "Gujarati", false, "Gujr", &[0x47]),
    tl("or", "ଓଡ଼ିଆ", "Odia", false, "Orya", &[0x48]),
    tl("ta", "தமிழ்", "Tamil", false, "Taml", &[0x49]),
    tl("te", "తెలుగు", "Telugu", false, "Telu", &[0x4A]),
    tl("kn", "ಕನ್ನಡ", "Kannada", false, "Knda", &[0x4B]),
    tl("ml", "മലയാളം", "Malayalam", false, "Mlym", &[0x4C]),
    tl("id", "Bahasa Indonesia", "Indonesian", false, "Latn", &[0x21]),
    tl("ms", "Bahasa Melayu", "Malay", false, "Latn", &[0x3E]),
    tl("vi", "Tiếng Việt", "Vietnamese", false, "Latn", &[0x2A]),
    tl("th", "ไทย", "Thai", false, "Thai", &[0x1E]),
    // zh-TW, zh-HK, zh-MO and the neutral zh-Hant; every other Chinese is Simplified.
    tl("zh-Hant", "繁體中文", "Chinese (Traditional)", false, "Hant", &[0x0404, 0x0C04, 0x1404, 0x7C04]),
    tl("zh-Hans", "简体中文", "Chinese (Simplified)", false, "Hans", &[0x04]),
    tl("ja", "日本語", "Japanese", false, "Jpan", &[0x11]),
    tl("ko", "한국어", "Korean", false, "Kore", &[0x12]),
];

/// The embedded language files: (code, contents), written by `build.rs`
/// for every `i18n/<code>.json` but `source.json`.
const FILES: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/i18n_files.rs"));

fn file(code: &str) -> Option<&'static str> {
    FILES.iter().find(|(c, _)| *c == code).map(|(_, f)| *f)
}

impl Locale {
    /// Arabic and English always; a translation when its file is embedded.
    pub fn available(&self) -> bool {
        !self.machine || file(self.code).is_some()
    }

    /// The available language with this code (any letter case).
    pub fn find(code: &str) -> Option<&'static Locale> {
        let code = code.trim();
        LOCALES.iter().find(|l| l.code.eq_ignore_ascii_case(code) && l.available())
    }

    /// The available language for a Windows LANGID, if any.
    pub fn for_langid(langid: u16) -> Option<&'static Locale> {
        let primary = langid & 0x3ff;
        let usable = || LOCALES.iter().filter(|l| l.available());
        usable().find(|l| l.langids.iter().any(|&id| id > 0x3ff && id == langid)).or_else(|| usable().find(|l| l.langids.contains(&primary)))
    }

    /// Arabic or English.
    pub fn of(lang: Lang) -> &'static Locale {
        match lang {
            Lang::Ar => &LOCALES[0],
            Lang::En => &LOCALES[1],
        }
    }
}

/// The languages to offer, in the pickers' order: Arabic and English, then
/// the others by native name (which groups them by script).
pub fn available() -> Vec<&'static Locale> {
    let mut v: Vec<&'static Locale> = LOCALES.iter().filter(|l| l.available()).collect();
    v[2..].sort_by_key(|l| l.native.to_lowercase());
    v
}

const UNSET: u8 = 0;
/// 1 + the index in [`LOCALES`] of this process's language.
static CURRENT: AtomicU8 = AtomicU8::new(UNSET);

/// The language of this process, detected on first use.
pub fn locale() -> &'static Locale {
    match CURRENT.load(Ordering::Relaxed) {
        UNSET => {
            let l = detect();
            let i = LOCALES.iter().position(|x| std::ptr::eq(x, l)).unwrap_or(1);
            CURRENT.store(i as u8 + 1, Ordering::Relaxed);
            &LOCALES[i]
        }
        n => &LOCALES[(n - 1) as usize],
    }
}

/// The original text this process's language starts from.
pub fn current() -> Lang {
    locale().base
}

/// Whether the English texts are used (every language but Arabic; a
/// translated language translates them). For the direction ask [`is_rtl`].
pub fn is_en() -> bool {
    current() == Lang::En
}

/// The interface reads right to left (Arabic, Persian, Urdu, Syriac…).
pub fn is_rtl() -> bool {
    locale().rtl
}

/// `ar` or `en` by the current language (`en` translated when the language
/// has a translation of it). Usually written `tr!(ar, en)`.
pub fn tr<T: Text>(ar: T, en: T) -> T {
    current().pick(ar, en)
}

/// `flags` (right-to-left window styles) in a right-to-left language, 0
/// otherwise.
pub fn rtl_only(flags: u32) -> u32 {
    if is_rtl() { flags } else { 0 }
}

/// A value `tr!` and [`Lang::pick`] can give: a text is translated, any
/// other value is the English one as it is.
pub trait Text {
    fn localized(self) -> Self;
}

impl<'a> Text for &'a str {
    fn localized(self) -> &'a str {
        translation(self).unwrap_or(self)
    }
}

impl Text for String {
    fn localized(self) -> String {
        translation(&self).map_or(self, str::to_string)
    }
}

macro_rules! plain_text {
    ($($t:ty),*) => { $(impl Text for $t { fn localized(self) -> Self { self } })* };
}
plain_text!(u8, u16, u32, u64, usize, i8, i16, i32, i64, isize, bool, char);

/// The current language's translations, read from its file once.
struct Table {
    rust: HashMap<String, String>,
    /// The `js` part, for the page.
    js: serde_json::Value,
}

static TABLE: OnceLock<Option<Table>> = OnceLock::new();

fn table() -> Option<&'static Table> {
    TABLE.get_or_init(|| load(locale())).as_ref()
}

/// The translations of a translated language; `None` for Arabic, English
/// or a file that does not parse (the language then reads English).
fn load(l: &Locale) -> Option<Table> {
    if !l.machine {
        return None;
    }
    let mut v: serde_json::Value = serde_json::from_str(file(l.code)?).ok()?;
    let rust = match v.get_mut("rust").map(serde_json::Value::take) {
        Some(serde_json::Value::Object(m)) => m
            .into_iter()
            .filter_map(|(k, v)| match v {
                serde_json::Value::String(s) if !s.trim().is_empty() => Some((k, s)),
                _ => None,
            })
            .collect(),
        _ => HashMap::new(),
    };
    let js = match v.get_mut("js").map(serde_json::Value::take) {
        Some(o @ serde_json::Value::Object(_)) => o,
        _ => serde_json::Value::Object(Default::default()),
    };
    Some(Table { rust, js })
}

/// The translation of the English text `en`, in a translated language.
fn translation(en: &str) -> Option<&'static str> {
    if !locale().machine {
        return None;
    }
    table()?.rust.get(en).map(String::as_str)
}

/// The English template `en` filled with `values` (its placeholders in
/// order, formatted) in language `l`, which need not be this process's:
/// `l`'s translation when its file has one, the English text otherwise. For
/// the few texts worded in a language the user just chose.
#[cfg(any(windows, test))]
pub fn format_in(l: &Locale, en: &str, values: &[String]) -> String {
    let t = if !l.machine {
        None
    } else if std::ptr::eq(l, locale()) {
        translation(en).map(str::to_string)
    } else {
        load(l).and_then(|t| t.rust.get(en).cloned())
    };
    fill(t.as_deref().unwrap_or(en), en, values)
}

/// The translated template of a `trf!` (used by its expansion).
#[doc(hidden)]
pub fn template(en: &'static str) -> Option<&'static str> {
    translation(en)
}

/// Where a placeholder's value comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Arg<'a> {
    /// `{}` (the next one) or `{1}`.
    Pos(usize),
    /// `{name}`.
    Named(&'a str),
}

/// A template's parts: text, and placeholders (where from, format spec).
#[derive(Debug, PartialEq, Eq)]
enum Part<'a> {
    Text(&'a str),
    /// `{{` or `}}`.
    Brace(char),
    Place(Arg<'a>, &'a str),
}

/// A `format!` template taken apart; `None` when it is not one (a lone
/// brace, an unclosed placeholder).
fn parse(t: &str) -> Option<Vec<Part<'_>>> {
    let mut out = Vec::new();
    let (mut rest, mut next) = (t, 0);
    while let Some(i) = rest.find(['{', '}']) {
        if i > 0 {
            out.push(Part::Text(&rest[..i]));
        }
        let b = rest.as_bytes()[i];
        rest = &rest[i + 1..];
        if rest.as_bytes().first() == Some(&b) {
            out.push(Part::Brace(b as char));
            rest = &rest[1..];
            continue;
        }
        if b == b'}' {
            return None;
        }
        let end = rest.find('}')?;
        let inner = &rest[..end];
        if inner.contains('{') {
            return None;
        }
        rest = &rest[end + 1..];
        let (name, spec) = inner.split_once(':').unwrap_or((inner, ""));
        let name = name.trim();
        let arg = if name.is_empty() {
            next += 1;
            Arg::Pos(next - 1)
        } else if let Ok(n) = name.parse() {
            Arg::Pos(n)
        } else {
            Arg::Named(name)
        };
        out.push(Part::Place(arg, spec));
    }
    if !rest.is_empty() {
        out.push(Part::Text(rest));
    }
    Some(out)
}

/// The placeholders of a template: (where from, spec), each once, sorted.
fn places<'a>(parts: &[Part<'a>]) -> Vec<(Arg<'a>, &'a str)> {
    let mut v: Vec<(Arg<'a>, &'a str)> = parts.iter().filter_map(|p| if let Part::Place(a, s) = p { Some((a.clone(), *s)) } else { None }).collect();
    v.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    v.dedup();
    v
}

/// Whether translation `t` has exactly the placeholders of English
/// template `en` (each at least once, in any order, nothing else).
#[cfg(test)]
pub fn same_placeholders(en: &str, t: &str) -> bool {
    match (parse(en), parse(t)) {
        (Some(e), Some(t)) => places(&e) == places(&t),
        _ => false,
    }
}

/// `trf!` in a translated language: template `t` (the translation of
/// `en`) with the placeholders' texts, `values` being the English
/// template's placeholders formatted one by one, in its order. A
/// translation with other placeholders than the English one gives the
/// English text.
#[doc(hidden)]
pub fn fill(t: &str, en: &str, values: &[String]) -> String {
    let Some(en_parts) = parse(en) else { return values.concat() };
    let known: Vec<(Arg<'_>, &str, &str)> = en_parts
        .iter()
        .filter_map(|p| if let Part::Place(a, s) = p { Some((a.clone(), *s)) } else { None })
        .zip(values)
        .map(|((a, s), v)| (a, s, v.as_str()))
        .collect();
    let render = |parts: &[Part<'_>]| {
        let mut s = String::new();
        for p in parts {
            match p {
                Part::Text(x) => s.push_str(x),
                Part::Brace(c) => s.push(*c),
                Part::Place(a, spec) => s.push_str(known.iter().find(|(ka, ks, _)| ka == a && ks == spec)?.2),
            }
        }
        Some(s)
    };
    match parse(t) {
        Some(tp) if places(&tp) == places(&en_parts) => render(&tp),
        _ => None,
    }
    .or_else(|| render(&en_parts))
    .unwrap_or_else(|| values.concat())
}

/// What the page needs of the language, as JSON for its data block:
/// its code, direction, the translations of app.js's texts (`{}` for
/// Arabic and English, which app.js has itself) and the languages to offer.
pub fn page_data(l: &'static Locale) -> String {
    let js = if std::ptr::eq(l, locale()) { table().map(|t| t.js.clone()) } else { load(l).map(|t| t.js) };
    let langs: Vec<serde_json::Value> = available()
        .into_iter()
        .map(|l| serde_json::json!({ "code": l.code, "native": l.native, "english": l.english, "rtl": l.rtl, "machine": l.machine }))
        .collect();
    let data = serde_json::json!({
        "code": l.code,
        "rtl": l.rtl,
        "js": js.unwrap_or_else(|| serde_json::json!({})),
        "langs": langs,
    });
    // Inside <script type="application/json">: no "</script>" can appear.
    data.to_string().replace('<', "\\u003c")
}

/// English names of the file-type presets, in the order of
/// [`crate::search::KINDS`] (whose labels are the Arabic names).
pub const KIND_LABELS_EN: [&str; 8] =
    ["All types", "Documents", "Pictures", "Audio", "Video", "Archives", "Programs", "Folders"];

/// The saved choice, else the Windows display language. Copies that were
/// installed before English existed keep Arabic instead of switching on an
/// update. The first detection is saved so it stays the same later.
fn detect() -> &'static Locale {
    // Tests check the Arabic texts whatever the machine's language, and
    // must not write the choice to the registry.
    if cfg!(test) {
        return &LOCALES[0];
    }
    if let Some(code) = saved() {
        match Locale::find(&code) {
            Some(l) => return l,
            // A language this build has no file for (a later or an earlier
            // version had one): English, and the choice stays saved.
            None if !code.trim().is_empty() => return &LOCALES[1],
            None => {}
        }
    }
    let l = if installed_before() { &LOCALES[0] } else { system() };
    #[cfg(windows)]
    win::save(l.code);
    l
}

/// Whether an index or settings from an earlier version exist.
#[cfg(windows)]
fn installed_before() -> bool {
    let Some(dir) = std::env::var_os("LOCALAPPDATA") else { return false };
    data_from_before_english(&std::path::Path::new(&dir).join("BahthSari"))
}

#[cfg(not(windows))]
fn installed_before() -> bool {
    false
}

/// The settings key only versions with English write (0.10.1's one-time
/// switch to the new look; 1.0 still writes it, `Settings::make_new_look_only`).
const ENGLISH_ERA_KEY: &str = "new_ui_default";

/// Whether `dir` holds data of a copy installed before English existed:
/// an index, a history, or a settings file. A settings file with
/// [`ENGLISH_ERA_KEY`] was written by a version that already saves the
/// language it detected, so it does not count (the registry choice was
/// removed, or the settings were written before the first detection).
fn data_from_before_english(dir: &std::path::Path) -> bool {
    if ["index.bin", "history.json"].iter().any(|f| dir.join(f).exists()) {
        return true;
    }
    match std::fs::read(dir.join("settings.txt")) {
        Ok(bytes) => !String::from_utf8_lossy(&bytes)
            .lines()
            .any(|l| l.split_once('=').is_some_and(|(k, _)| k.trim() == ENGLISH_ERA_KEY)),
        Err(_) => false,
    }
}

/// `tr!("نص", "text")`: the text in the current language.
macro_rules! tr {
    ($ar:expr, $en:expr $(,)?) => {
        $crate::i18n::tr($ar, $en)
    };
}

/// `trf!("… {x}", "… {x}", args…)`: `format!` with the template of the
/// current language (in a translated one, the translation of the English
/// template; see `i18n-macros`). The arguments are evaluated once.
///
/// `trf!(in lang; "…", "…", args…)` takes the language from `lang` (a
/// [`Lang`]) instead, for code that is handed its language: Arabic, or
/// the English template translated as above. The `format!` counterpart of
/// [`Lang::pick`].
macro_rules! trf {
    (in $lang:expr; $ar:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        ::bahth_i18n_macros::trf_impl!(lang ($lang) ($ar) ($en) $(($arg))*)
    };
    ($ar:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        ::bahth_i18n_macros::trf_impl!(($ar) ($en) $(($arg))*)
    };
}

#[cfg(windows)]
mod win {
    use winapi::shared::minwindef::DWORD;
    use winapi::shared::winerror::ERROR_SUCCESS;
    use winapi::um::winnt::REG_SZ;
    use winapi::um::winreg::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW, RegSetKeyValueW};

    const KEY: &str = "Software\\BahthSari";
    const VALUE: &str = "Language";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetUserDefaultUILanguage() -> u16;
    }

    pub fn system() -> &'static super::Locale {
        super::Locale::for_langid(unsafe { GetUserDefaultUILanguage() }).unwrap_or(&super::LOCALES[1])
    }

    /// The saved code, as it is.
    pub fn saved() -> Option<String> {
        let (key, value) = (wide(KEY), wide(VALUE));
        let mut buf = [0u16; 32];
        let mut len = std::mem::size_of_val(&buf) as DWORD;
        let r = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut len,
            )
        };
        if r != ERROR_SUCCESS as i32 {
            return None;
        }
        let n = (len as usize / 2).min(buf.len());
        Some(String::from_utf16_lossy(&buf[..n]).replace('\0', ""))
    }

    pub fn save(code: &str) -> bool {
        let (key, value, data) = (wide(KEY), wide(VALUE), wide(code));
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as DWORD,
            ) == ERROR_SUCCESS as i32
        }
    }
}

#[cfg(windows)]
use win::{saved, system};

/// Saves the language for the next start (this process keeps its own).
#[cfg(windows)]
pub fn save_locale(l: &Locale) -> bool {
    win::save(l.code)
}

// Off Windows only the tests run: they keep the original Arabic texts.
#[cfg(not(windows))]
fn saved() -> Option<String> {
    None
}

#[cfg(not(windows))]
fn system() -> &'static Locale {
    &LOCALES[0]
}

#[cfg(test)]
#[path = "i18n_check.rs"]
mod check;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip() {
        for l in [Lang::Ar, Lang::En] {
            assert_eq!(Lang::from_code(l.code()), Some(l));
        }
        assert_eq!(Lang::from_code(" EN "), Some(Lang::En));
        assert_eq!(Lang::from_code("fr"), None);
        assert_eq!(Lang::from_code(""), None);
    }

    #[test]
    fn arabic_locales_are_arabic() {
        // ar-SA, ar-EG, ar-AE and the neutral Arabic LANGID.
        for id in [0x0401, 0x0c01, 0x3801, 0x0001] {
            assert_eq!(Lang::from_langid(id), Lang::Ar, "{id:#x}");
            assert_eq!(Locale::for_langid(id).map(|l| l.code), Some("ar"), "{id:#x}");
        }
        // en-US, fr-FR, fa-IR (Persian is not Arabic), ur-PK.
        for id in [0x0409, 0x040c, 0x0429, 0x0420] {
            assert_eq!(Lang::from_langid(id), Lang::En, "{id:#x}");
        }
        assert_eq!(Locale::for_langid(0x0409).map(|l| l.code), Some("en"));
    }

    #[test]
    fn table_is_sound() {
        let mut codes: Vec<&str> = LOCALES.iter().map(|l| l.code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), LOCALES.len(), "a code twice");
        assert_eq!((LOCALES[0].code, LOCALES[1].code), ("ar", "en"));
        assert!(LOCALES.iter().all(|l| l.code.len() <= 16), "the registry reads up to 31 characters");
        assert!(LOCALES[2..].iter().all(|l| l.machine && l.base == Lang::En));
        // Each Windows language goes to one row only (exact LANGIDs first).
        let mut ids: Vec<u16> = LOCALES.iter().flat_map(|l| l.langids.iter().copied()).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "a Windows language in two rows");
        // Hebrew is not offered.
        assert!(LOCALES.iter().all(|l| l.code != "he" && !l.langids.contains(&0x0D)));
        // Every embedded file has its row.
        for (code, _) in FILES {
            assert!(LOCALES.iter().any(|l| l.code == *code), "i18n/{code}.json has no row in LOCALES");
        }
    }

    #[test]
    fn languages_are_detected_only_when_embedded() {
        // Chinese: Taiwan, Hong Kong and Macao read Traditional.
        let code = |id| Locale::for_langid(id).map(|l| l.code);
        for (id, want) in [(0x0404u16, "zh-Hant"), (0x0C04, "zh-Hant"), (0x1404, "zh-Hant"), (0x0804, "zh-Hans"), (0x1004, "zh-Hans"), (0x0429, "fa"), (0x0492, "ckb"), (0x041F, "tr"), (0x040C, "fr"), (0x0C0C, "fr"), (0x0416, "pt"), (0x0816, "pt"), (0x105F, "zgh")] {
            let row = LOCALES.iter().find(|l| l.code == want).unwrap();
            assert_eq!(code(id), if row.available() { Some(want) } else { None }, "{id:#x}");
        }
        // Not a language of the table: nothing (English is used).
        assert_eq!(code(0x040D), None);
        assert!(Locale::find("xx").is_none());
        assert_eq!(Locale::find(" AR ").map(|l| l.code), Some("ar"));
        let list = available();
        assert_eq!((list[0].code, list[1].code), ("ar", "en"));
        assert!(list.iter().all(|l| l.available()));
    }

    #[test]
    fn pick_chooses_by_language() {
        assert_eq!(Lang::Ar.pick("بحث", "Search"), "بحث");
        assert_eq!(Lang::En.pick("بحث", "Search"), "Search");
        assert_eq!(Lang::En.pick(0x0040_0000u32, 0), 0);
    }

    #[test]
    fn every_kind_has_an_english_name() {
        assert_eq!(KIND_LABELS_EN.len(), crate::search::KINDS.len());
        assert_eq!(KIND_LABELS_EN[crate::search::KIND_FOLDERS], "Folders");
    }

    #[test]
    fn only_data_from_before_english_keeps_arabic() {
        let dir = std::env::temp_dir().join(format!("bahth-i18n-{}", std::process::id()));
        let _tmp = crate::test_temp::TempPath::at(dir.clone());
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A new user: nothing there yet.
        assert!(!data_from_before_english(&dir));
        // Settings written by 0.10.1 or later (the new-look marker): not old.
        std::fs::write(dir.join("settings.txt"), "new_ui=0\nnew_ui_default=1\n").unwrap();
        assert!(!data_from_before_english(&dir));
        // Settings of an older version, without the marker: old.
        std::fs::write(dir.join("settings.txt"), "width=900\nnew_ui=0\n").unwrap();
        assert!(data_from_before_english(&dir));
        std::fs::remove_file(dir.join("settings.txt")).unwrap();
        // An index or a history alone: old.
        for f in ["index.bin", "history.json"] {
            std::fs::write(dir.join(f), b"x").unwrap();
            assert!(data_from_before_english(&dir), "{f}");
            std::fs::remove_file(dir.join(f)).unwrap();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn helpers_follow_the_current_language() {
        // Tests never change the language, which off Windows is Arabic.
        assert_eq!(current(), Lang::Ar);
        assert!(!is_en());
        assert!(is_rtl());
        assert_eq!(tr!("بحث", "Search"), "بحث");
        let n = 3;
        assert_eq!(trf!("وجدت {n} نتيجة", "Found {n} results"), "وجدت 3 نتيجة");
        assert_eq!(trf!("{} من {}", "{} of {}", 1, 2), "1 من 2");
        assert_eq!(rtl_only(0x0040_0000), 0x0040_0000);
        // Arguments are evaluated once.
        let mut calls = 0;
        let mut next = || {
            calls += 1;
            calls
        };
        let x = 0;
        assert_eq!(trf!("{:?} {x}", "{:?} {x}", next()).len(), 3);
        assert_eq!(calls, 1);
        // With a given language: that one, whatever the process's.
        assert_eq!(trf!(in Lang::En; "وجدت {n} نتيجة", "Found {n} results"), "Found 3 results");
        assert_eq!(trf!(in Lang::from_english(false); "{} من {}", "{} of {}", 1, 2), "1 من 2");
        // The language and the arguments are evaluated once.
        let (mut langs, mut args) = (0, 0);
        let mut lang = || {
            langs += 1;
            Lang::En
        };
        let mut arg = || {
            args += 1;
            args
        };
        assert_eq!(trf!(in lang(); "{:?} {x}", "{x} {:?}", arg()), "0 1");
        assert_eq!((langs, args), (1, 1));
        assert_eq!(Lang::from_english(true).pick("بحث", "Search"), "Search");
    }

    #[test]
    fn translated_templates_are_filled() {
        let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // Named, positional, reordered, with specs.
        assert_eq!(fill("{n} résultats", "Found {n} results", &v(&["3"])), "3 résultats");
        assert_eq!(fill("{1} / {0}", "{} of {}", &v(&["1", "2"])), "2 / 1");
        assert_eq!(fill("{} sur {}", "{} of {}", &v(&["1", "2"])), "1 sur 2");
        assert_eq!(fill("{:.1} Go ({e:?})", "{:.1} GB ({e:?})", &v(&["1.5", "\"x\""])), "1.5 Go (\"x\")");
        assert_eq!(fill("{{{n}}} fois {n}", "{{{n}}} times", &v(&["2"])), "{2} fois 2");
        // A placeholder missing, added, renamed or with another spec, or a
        // broken template: the English text.
        for bad in ["résultats", "{n} {m} résultats", "{x} résultats", "{n:?} résultats", "{n résultats", "} {n}"] {
            assert_eq!(fill(bad, "Found {n} results", &v(&["3"])), "Found 3 results", "{bad}");
        }
        assert!(same_placeholders("{} of {}", "{1} de {0}"));
        // In a language that is not this process's: English without a translation.
        assert_eq!(format_in(&EN, "In {}.\n\nNow?", &v(&["Français"])), "In Français.\n\nNow?");
        if let Some(fr) = Locale::find("fr") {
            assert!(!format_in(fr, "Found {n} results", &v(&["3"])).is_empty());
        }
        assert!(!same_placeholders("{} of {}", "{}"));
        assert!(same_placeholders("a {{b}}", "x {{y}}"));
    }

    #[test]
    fn the_page_gets_its_language() {
        let d: serde_json::Value = serde_json::from_str(&page_data(&LOCALES[0])).unwrap();
        assert_eq!(d["code"], "ar");
        assert_eq!(d["rtl"], true);
        assert_eq!(d["js"], serde_json::json!({}));
        assert_eq!(d["langs"][1]["code"], "en");
        assert_eq!(d["langs"][1]["machine"], false);
        assert!(!page_data(&LOCALES[1]).contains('<'));
    }
}
