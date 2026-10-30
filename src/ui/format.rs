//! Formatting numbers, sizes, paths and CSV cells for display.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::index::Entry;

/// Show numbers with Arabic-Indic digits.
pub(super) static ARABIC_DIGITS: AtomicBool = AtomicBool::new(false);

/// Keeps a path reading left-to-right even when folder names are Arabic, by
/// putting left-to-right marks around every separator. Display only: never
/// pass the result to the shell or the clipboard.
pub(super) fn ltr_path(path: &str) -> String {
    let mut s = String::with_capacity(path.len() + 16);
    s.push('\u{200E}');
    for c in path.chars() {
        if c == '\\' {
            s.push_str("\u{200E}\\\u{200E}");
        } else {
            s.push(c);
        }
    }
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
    let b = e.size as f64;
    digits(if b < 1024.0 {
        trf!("{} بايت", "{} bytes", e.size)
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
