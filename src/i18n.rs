//! The user-interface language: Arabic (right-to-left) or English.
//!
//! Every user-visible string is written in place as both texts, `tr!("…", "…")`
//! for a fixed text and `trf!("… {x}", "… {x}")` for a `format!`, so the
//! translation sits next to the original. The language is chosen once per
//! process: the user's choice saved in the registry
//! (`HKCU\Software\BahthSari`, value `Language` = `ar` / `en`), otherwise
//! the Windows display language (Arabic → Arabic, anything else → English).
//! Changing it takes effect at the next start.

use std::sync::atomic::{AtomicU8, Ordering};

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

    pub fn from_code(code: &str) -> Option<Lang> {
        match code.trim().to_ascii_lowercase().as_str() {
            "ar" => Some(Lang::Ar),
            "en" => Some(Lang::En),
            _ => None,
        }
    }

    /// The language for a Windows LANGID: Arabic for any Arabic locale
    /// (primary language 0x01), English for everything else.
    pub fn from_langid(langid: u16) -> Lang {
        if langid & 0x3ff == 0x01 { Lang::Ar } else { Lang::En }
    }

    /// `ar` in Arabic, `en` in English.
    pub fn pick<T>(self, ar: T, en: T) -> T {
        match self {
            Lang::Ar => ar,
            Lang::En => en,
        }
    }
}

const UNSET: u8 = 0;
static CURRENT: AtomicU8 = AtomicU8::new(UNSET);

fn encode(lang: Lang) -> u8 {
    match lang {
        Lang::Ar => 1,
        Lang::En => 2,
    }
}

/// The language of this process, detected on first use.
pub fn current() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::Ar,
        2 => Lang::En,
        _ => {
            let lang = detect();
            CURRENT.store(encode(lang), Ordering::Relaxed);
            lang
        }
    }
}

pub fn is_en() -> bool {
    current() == Lang::En
}

/// `ar` or `en` by the current language. Usually written `tr!(ar, en)`.
pub fn tr<T>(ar: T, en: T) -> T {
    current().pick(ar, en)
}

/// `flags` (right-to-left window styles) in Arabic, 0 in English.
pub fn rtl_only(flags: u32) -> u32 {
    tr(flags, 0)
}

/// English names of the file-type presets, in the order of
/// [`crate::search::KINDS`] (whose labels are the Arabic names).
pub const KIND_LABELS_EN: [&str; 8] =
    ["All types", "Documents", "Pictures", "Audio", "Video", "Archives", "Programs", "Folders"];

/// The saved choice, else the Windows display language.
fn detect() -> Lang {
    saved().unwrap_or_else(system)
}

/// `tr!("نص", "text")`: the text in the current language.
macro_rules! tr {
    ($ar:expr, $en:expr $(,)?) => {
        $crate::i18n::tr($ar, $en)
    };
}

/// `trf!("… {x}", "… {x}", args…)`: `format!` with the template of the
/// current language. Only the chosen template's arguments are evaluated.
macro_rules! trf {
    ($ar:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        if $crate::i18n::is_en() { format!($en $(, $arg)*) } else { format!($ar $(, $arg)*) }
    };
}

#[cfg(windows)]
mod win {
    use super::Lang;
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

    pub fn system() -> Lang {
        Lang::from_langid(unsafe { GetUserDefaultUILanguage() })
    }

    pub fn saved() -> Option<Lang> {
        let (key, value) = (wide(KEY), wide(VALUE));
        let mut buf = [0u16; 16];
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
        Lang::from_code(&String::from_utf16_lossy(&buf[..n]).replace('\0', ""))
    }

    pub fn save(lang: Lang) -> bool {
        let (key, value, data) = (wide(KEY), wide(VALUE), wide(lang.code()));
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
pub fn save(lang: Lang) -> bool {
    win::save(lang)
}

// Off Windows only the tests run: they keep the original Arabic texts.
#[cfg(not(windows))]
fn saved() -> Option<Lang> {
    None
}

#[cfg(not(windows))]
fn system() -> Lang {
    Lang::Ar
}

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
        }
        // en-US, fr-FR, fa-IR (Persian is not Arabic), ur-PK.
        for id in [0x0409, 0x040c, 0x0429, 0x0420] {
            assert_eq!(Lang::from_langid(id), Lang::En, "{id:#x}");
        }
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
    fn helpers_follow_the_current_language() {
        // Tests never change the language, which off Windows is Arabic.
        assert_eq!(current(), Lang::Ar);
        assert!(!is_en());
        assert_eq!(tr!("بحث", "Search"), "بحث");
        let n = 3;
        assert_eq!(trf!("وجدت {n} نتيجة", "Found {n} results"), "وجدت 3 نتيجة");
        assert_eq!(trf!("{} من {}", "{} of {}", 1, 2), "1 من 2");
        assert_eq!(rtl_only(0x0040_0000), 0x0040_0000);
    }
}
