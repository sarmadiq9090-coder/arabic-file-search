//! WinRT classes are activated only when Windows has them registered.
//!
//! When a class is not registered, the `windows` crate (windows-core 0.62,
//! `factory_cache.rs`) falls back to loading a DLL named after the class's
//! namespace (`Windows.Media.Ocr.dll`, ...) with
//! `LOAD_LIBRARY_SEARCH_DEFAULT_DIRS`, which includes the program's own
//! folder: a file of that name next to `bahth.exe` in Downloads would be
//! loaded. So every class the OCR uses is looked up under
//! `HKLM\SOFTWARE\Microsoft\WindowsRuntime\ActivatableClassId` first, and
//! OCR counts as unavailable (the existing "no OCR" path) when one is missing.
//!
//! Free of Win32, so it is tested on every platform.

/// Every WinRT class `ocr.rs` activates or calls statics of.
pub const OCR_CLASSES: &[&str] = &[
    "Windows.Media.Ocr.OcrEngine",
    "Windows.Globalization.Language",
    "Windows.Graphics.Imaging.BitmapDecoder",
    "Windows.Graphics.Imaging.BitmapTransform",
    "Windows.Graphics.Imaging.SoftwareBitmap",
    "Windows.Storage.Streams.DataWriter",
    "Windows.Storage.Streams.InMemoryRandomAccessStream",
];

/// The registry key of a WinRT class, under `HKEY_LOCAL_MACHINE`.
pub fn class_key(class: &str) -> String {
    format!("SOFTWARE\\Microsoft\\WindowsRuntime\\ActivatableClassId\\{class}")
}

/// The first of `classes` whose key `exists` does not find, if any.
pub fn first_missing<'a>(classes: &[&'a str], exists: impl Fn(&str) -> bool) -> Option<&'a str> {
    classes.iter().copied().find(|c| !exists(&class_key(c)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocr_runs_only_with_every_class_registered() {
        assert_eq!(class_key("Windows.Media.Ocr.OcrEngine"), r"SOFTWARE\Microsoft\WindowsRuntime\ActivatableClassId\Windows.Media.Ocr.OcrEngine");
        assert_eq!(first_missing(OCR_CLASSES, |_| true), None);
        // Windows N without the media features: no OCR engine registered.
        let no_ocr = |k: &str| !k.ends_with("Windows.Media.Ocr.OcrEngine");
        assert_eq!(first_missing(OCR_CLASSES, no_ocr), Some("Windows.Media.Ocr.OcrEngine"));
        assert_eq!(first_missing(OCR_CLASSES, |_| false), Some("Windows.Media.Ocr.OcrEngine"), "the OCR engine is checked first");
        let no_streams = |k: &str| !k.contains("Storage.Streams");
        assert_eq!(first_missing(OCR_CLASSES, no_streams), Some("Windows.Storage.Streams.DataWriter"));
    }
}
