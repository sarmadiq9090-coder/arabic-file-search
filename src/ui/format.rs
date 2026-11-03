//! Formatting numbers, sizes, paths and CSV cells for display.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::index::Entry;

/// Show numbers with Arabic-Indic digits.
pub(super) static ARABIC_DIGITS: AtomicBool = AtomicBool::new(false);

/// Keeps a path reading left-to-right even when folder names are Arabic: the
/// whole path is a left-to-right isolate (LRI … PDI), so the right-to-left
/// text around it (the Arabic window, a label) cannot move its ends, such as
/// a trailing `\`, to the other side; and left-to-right marks around every
/// separator keep Arabic folder names in path order. Display only: never
/// pass the result to the shell or the clipboard.
pub(super) fn ltr_path(path: &str) -> String {
    // In the left-to-right (English) window a path with no right-to-left
    // letters already reads correctly.
    if crate::i18n::is_en() && !path.chars().any(|c| matches!(c, '\u{0590}'..='\u{08FF}' | '\u{FB1D}'..='\u{FEFC}')) {
        return path.to_owned();
    }
    let mut s = String::with_capacity(path.len() + 16);
    s.push('\u{2066}');
    for c in path.chars() {
        if c == '\\' {
            s.push_str("\u{200E}\\\u{200E}");
        } else {
            s.push(c);
        }
    }
    s.push('\u{2069}');
    s
}

/// Reads a NUL-terminated UTF-16 string.
///
/// # Safety
/// `p` must point to a NUL-terminated UTF-16 buffer.
pub(super) unsafe fn wide_to_string(p: *const u16) -> String {
    let mut n = 0;
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, n) })
}

/// Swaps ASCII digits for Arabic-Indic ones when the user asked for them.
pub(super) fn digits(s: String) -> String {
    if !ARABIC_DIGITS.load(Ordering::Relaxed) {
        return s;
    }
    s.chars()
        .map(|c| if c.is_ascii_digit() { char::from_u32(c as u32 - '0' as u32 + 0x0660).unwrap_or(c) } else { c })
        .collect()
}

pub(super) fn fmt_size(e: &Entry) -> String {
    if e.is_dir {
        return tr!("مجلد", "Folder").into();
    }
    fmt_bytes(e.size)
}

/// A byte count as "12.3 MB".
pub(super) fn fmt_bytes(size: u64) -> String {
    let b = size as f64;
    digits(if b < 1024.0 {
        trf!("{} بايت", "{} bytes", size)
    } else if b < 1024.0 * 1024.0 {
        trf!("{:.1} ك.ب", "{:.1} KB", b / 1024.0)
    } else if b < 1024.0 * 1024.0 * 1024.0 {
        trf!("{:.1} م.ب", "{:.1} MB", b / 1024.0 / 1024.0)
    } else {
        trf!("{:.2} ج.ب", "{:.2} GB", b / 1024.0 / 1024.0 / 1024.0)
    })
}

pub(super) fn fmt_num(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    digits(out)
}

/// Quotes a CSV cell, and stops Excel from reading a cell that starts with
/// `= + - @` as a formula by prefixing an apostrophe.
pub(super) fn csv_field(s: &str) -> String {
    let s = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) { format!("'{s}") } else { s.to_string() };
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s }
}

/// "قبل 5 دقائق" with the right Arabic plural for the count.
pub(super) fn minutes_ago(n: i64) -> String {
    match n {
        0 => tr!("الآن", "just now").into(),
        1 => tr!("قبل دقيقة", "1 minute ago").into(),
        2 => tr!("قبل دقيقتين", "2 minutes ago").into(),
        3..=10 => trf!("قبل {n} دقائق", "{n} minutes ago"),
        _ => trf!("قبل {n} دقيقة", "{n} minutes ago"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Awkward pieces of names and paths: Arabic with marks and tatweel,
    /// bidi marks, emoji with joiners, separators, quotes and formula signs.
    const PIECES: &[&str] = &[
        "a", "ب", "تقرير", "\u{064E}", "\u{0651}", "\u{0640}", "\u{200F}", "\u{200E}", "👨‍👩‍👧", "\\", ",", "\"",
        "\r", "\n", "\t", "=", "+", "-", "@", " ", "١", "\u{FFFD}", "\0",
    ];

    fn random_strings(seed: u64, n: usize) -> Vec<String> {
        let mut x = seed;
        let mut next = |m: usize| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % m as u64) as usize
        };
        (0..n)
            .map(|_| {
                let len = next(10);
                (0..len).map(|_| PIECES[next(PIECES.len())]).collect()
            })
            .collect()
    }

    #[test]
    fn numbers_get_thousands_separators() {
        // Arabic-Indic digits are off unless the user turns them on.
        assert_eq!(fmt_num(0), "0");
        assert_eq!(fmt_num(999), "999");
        assert_eq!(fmt_num(1000), "1,000");
        assert_eq!(fmt_num(1_234_567), "1,234,567");
        assert_eq!(fmt_num(usize::MAX).replace(',', ""), usize::MAX.to_string());
    }

    #[test]
    fn csv_cells() {
        assert_eq!(csv_field(""), "");
        assert_eq!(csv_field("تقرير.pdf"), "تقرير.pdf");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\", ok"), "\"say \"\"hi\"\", ok\"");
        assert_eq!(csv_field("=1+1"), "'=1+1");
        assert_eq!(csv_field("-x"), "'-x");
        assert_eq!(csv_field("\r\n"), "\"'\r\n\"");
    }

    #[test]
    fn csv_cells_of_random_text_stay_one_cell() {
        for s in random_strings(0x2545_F491_4F6C_DD1D, 3000) {
            let c = csv_field(&s);
            let inner = if c.starts_with('"') && c.len() >= 2 && c.ends_with('"') {
                let body = &c[1..c.len() - 1];
                // Every inner quote doubled, so the cell can't end early.
                assert!(!body.replace("\"\"", "").contains('"'), "{s:?} -> {c:?}");
                body.replace("\"\"", "\"")
            } else {
                assert!(!c.contains([',', '"', '\n', '\r']), "{s:?} -> {c:?}");
                c.clone()
            };
            // The text is kept, behind an apostrophe when Excel would read a formula.
            let guarded = s.starts_with(['=', '+', '-', '@', '\t', '\r']);
            assert_eq!(inner, if guarded { format!("'{s}") } else { s.clone() }, "{s:?} -> {c:?}");
        }
    }

    #[test]
    fn display_paths_keep_their_text() {
        for s in random_strings(0x1234_5678_9ABC_DEF1, 3000) {
            let shown = ltr_path(&s);
            // Only the isolate and left-to-right marks are added.
            let plain = |t: &str| t.replace(['\u{200E}', '\u{2066}', '\u{2069}'], "");
            assert_eq!(plain(&shown), plain(&s), "{s:?}");
            assert!(shown.starts_with('\u{2066}') && shown.ends_with('\u{2069}'), "{s:?}");
        }
    }

    #[test]
    fn display_paths_are_one_left_to_right_isolate() {
        // Tests run in Arabic (i18n::detect).
        assert_eq!(ltr_path("C:\\العمل\\"), "\u{2066}C:\u{200E}\\\u{200E}العمل\u{200E}\\\u{200E}\u{2069}");
    }

    #[test]
    fn wide_strings_read_up_to_the_nul() {
        let buf: Vec<u16> = "ملف 👨‍👩‍👧.txt".encode_utf16().chain([0, b'x' as u16, 0]).collect();
        assert_eq!(unsafe { wide_to_string(buf.as_ptr()) }, "ملف 👨‍👩‍👧.txt");
        assert_eq!(unsafe { wide_to_string([0u16].as_ptr()) }, "");
        // A lone surrogate (possible in an NTFS name) reads as U+FFFD.
        assert_eq!(unsafe { wide_to_string([0xD800u16, b'a' as u16, 0].as_ptr()) }, "\u{FFFD}a");
    }
}
