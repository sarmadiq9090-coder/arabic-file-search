//! Where each character of a one-line label ends up on screen.
//!
//! The results list draws a file name with `DrawText`, which reorders mixed
//! Arabic/Latin text with the Unicode bidi algorithm using the reading order
//! of the control: right-to-left in the Arabic interface. Uniscribe's
//! `ScriptStringAnalyse` measures every character correctly, but on real
//! Windows it does not place the runs the way the list does: for
//! "تقرير 2026.xlsx" in a right-to-left list it puts ".xlsx" at the right end
//! of the string, while the list shows "xlsx.2026 تقرير". The highlight
//! underline was then shifted by the width of the extension.
//!
//! So [`visual_spans`] takes only the advance width of each character from
//! Uniscribe and does the ordering itself, with the same algorithm and the
//! same paragraph direction as the list. It is plain Rust, so it is tested
//! on every platform.

use unicode_bidi::{BidiInfo, Level};

/// The horizontal extent `[left, right)` of each character of `text`, in
/// logical (string) order, measured from the visual left edge of the line.
///
/// `widths` holds the advance width of each `char` of `text`, in the same
/// order. `rtl` is the paragraph direction: true when the text is drawn with
/// a right-to-left reading order.
///
/// Returns `None` when `widths` does not match `text` or the text holds more
/// than one paragraph (a file name never does), so the caller can fall back.
pub fn visual_spans(text: &str, widths: &[i32], rtl: bool) -> Option<Vec<(i32, i32)>> {
    let starts: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
    if starts.len() != widths.len() {
        return None;
    }
    if text.is_empty() {
        return Some(Vec::new());
    }
    let level = if rtl { Level::rtl() } else { Level::ltr() };
    let info = BidiInfo::new(text, Some(level));
    let [para] = info.paragraphs.as_slice() else { return None };
    let (levels, runs) = info.visual_runs(para, para.range.clone());

    let mut spans = vec![(0, 0); widths.len()];
    let mut x = 0i32;
    let mut place = |i: usize| {
        let w = widths[i].max(0);
        spans[i] = (x, x.saturating_add(w));
        x = x.saturating_add(w);
    };
    // Runs come in visual order, left to right; a right-to-left run shows
    // its characters in reverse.
    for run in runs {
        let first = starts.partition_point(|&b| b < run.start);
        let end = starts.partition_point(|&b| b < run.end);
        if levels.get(run.start).is_some_and(|l| l.is_rtl()) {
            (first..end).rev().for_each(&mut place);
        } else {
            (first..end).for_each(&mut place);
        }
    }
    Some(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One unit of width per char, so positions read as visual indices.
    fn order(text: &str, rtl: bool) -> String {
        let n = text.chars().count();
        let spans = visual_spans(text, &vec![1; n], rtl).unwrap();
        let mut visual = vec![' '; n];
        for (c, (l, r)) in text.chars().zip(spans) {
            assert_eq!(r - l, 1);
            visual[l as usize] = c;
        }
        visual.into_iter().collect()
    }

    #[test]
    fn latin_left_to_right() {
        assert_eq!(order("report 2026.xlsx", false), "report 2026.xlsx");
        // A Latin name in the right-to-left list keeps its order.
        assert_eq!(order("report.pdf", true), "report.pdf");
    }

    #[test]
    fn arabic_name_with_extension_in_rtl_list() {
        // What the list shows on real Windows (checked on a screenshot):
        // the extension at the left, the Arabic first letter at the right.
        assert_eq!(order("تقرير 2026.xlsx", true), "xlsx.2026 ريرقت");
        assert_eq!(order("التقرير النهائي.pdf", true), "pdf.يئاهنلا ريرقتلا");
    }

    #[test]
    fn arabic_name_in_ltr_list() {
        assert_eq!(order("تقرير.pdf", false), "ريرقت.pdf");
    }

    #[test]
    fn first_arabic_letter_is_rightmost() {
        let text = "تقرير المبيعات 2026.xlsx";
        let widths: Vec<i32> = text.chars().map(|c| if c.is_ascii() { 7 } else { 5 }).collect();
        let total: i32 = widths.iter().sum();
        let spans = visual_spans(text, &widths, true).unwrap();
        assert_eq!(spans[0], (total - 5, total));
        // ".xlsx" is the leftmost thing on the line.
        let dot = text.chars().position(|c| c == '.').unwrap();
        assert_eq!(spans[dot + 1], (0, 7)); // 'x'
        // Every pixel is used exactly once.
        let mut sorted = spans.clone();
        sorted.sort();
        let mut x = 0;
        for (l, r) in sorted {
            assert_eq!(l, x);
            x = r;
        }
        assert_eq!(x, total);
    }

    #[test]
    fn surrogates_and_marks() {
        let text = "مُلف 😀.txt";
        let n = text.chars().count();
        let spans = visual_spans(text, &vec![3; n], true).unwrap();
        assert_eq!(spans.len(), n);
        assert!(spans.iter().all(|&(l, r)| r - l == 3 && l >= 0 && r <= 3 * n as i32));
    }

    #[test]
    fn mismatch_and_odd_input() {
        assert_eq!(visual_spans("abc", &[1, 1], false), None);
        assert_eq!(visual_spans("", &[], true), Some(Vec::new()));
        // Two paragraphs: not a file name, let the caller fall back.
        assert_eq!(visual_spans("a\u{2029}b", &[1, 1, 1], true), None);
        // Negative widths never move anything backwards.
        assert_eq!(visual_spans("ab", &[-4, 2], false), Some(vec![(0, 0), (0, 2)]));
        // Huge widths saturate instead of overflowing.
        let s = visual_spans("ab", &[i32::MAX, i32::MAX], false).unwrap();
        assert_eq!(s[1], (i32::MAX, i32::MAX));
    }
}
