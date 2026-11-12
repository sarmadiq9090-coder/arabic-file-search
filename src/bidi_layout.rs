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

/// Put before a right-to-left name's extension in the results list: a
/// right-to-left mark ends the base (its digits included), and a
/// left-to-right mark starts ".xlsx" as one left-to-right run. Plain marks,
/// not isolates (LRI…PDI), so every Windows draws them the same.
pub const EXT_MARKS: &str = "\u{200F}\u{200E}";

/// The byte index of the dot that starts `name`'s extension, when the name
/// has one worth setting apart: a base that is not empty or blank, and
/// after its last dot 1 to 16 letters, digits, `_`, `-` or `~`, none of
/// them right to left. Folders have none.
pub fn extension_at(name: &str, is_dir: bool) -> Option<usize> {
    let dot = name.rfind('.')?;
    let (base, ext) = (&name[..dot], &name[dot + 1..]);
    let ext_ok = (1..=16).contains(&ext.chars().count()) && ext.chars().all(|c| (c.is_alphanumeric() || matches!(c, '_' | '-' | '~')) && !is_rtl(c));
    (!is_dir && !base.trim().is_empty() && ext_ok).then_some(dot)
}

/// Whether `c` is a right-to-left letter or mark (Hebrew, Arabic, Syriac,
/// Thaana, NKo… and the Arabic presentation forms).
fn is_rtl(c: char) -> bool {
    matches!(c, '\u{0590}'..='\u{08FF}' | '\u{FB1D}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}')
}

/// A file name as the classic results list shows it. Laid out as one piece
/// in a right-to-left line, «تقرير المبيعات ٢٠٢٦.xlsx» reads "xlsx.٢٠٢٦ …":
/// the extension's letters at the far left and the dot between them and the
/// number. When the base starts with a right-to-left letter, [`EXT_MARKS`]
/// go before the dot, so ".xlsx" stays one left-to-right piece at the end
/// of the name as it reads (on its left), apart from the number. Names that
/// start left to right are unchanged. Returns the text and where the marks
/// went, to move the match ranges after them ([`shift_ranges`]). Display
/// only: never a name to rename or open.
pub fn list_name(name: &str, is_dir: bool) -> (String, Option<usize>) {
    let at = extension_at(name, is_dir).filter(|&dot| {
        name[..dot].chars().find(|c| c.is_alphabetic()).is_some_and(is_rtl)
    });
    match at {
        Some(dot) => (format!("{}{EXT_MARKS}{}", &name[..dot], &name[dot..]), Some(dot)),
        None => (name.to_string(), None),
    }
}

/// Byte ranges into a name, moved to the same characters in its
/// [`list_name`] text (`at` is where the marks went).
pub fn shift_ranges(ranges: Vec<(usize, usize)>, at: Option<usize>) -> Vec<(usize, usize)> {
    let Some(at) = at else { return ranges };
    let n = EXT_MARKS.len();
    let shift = |b: usize| if b >= at { b + n } else { b };
    // A range ending at the dot ends before the marks; one starting there starts after them.
    ranges.into_iter().map(|(s, e)| (shift(s), if e > at { e + n } else { e })).collect()
}

/// The name the user typed in the list's rename box: [`EXT_MARKS`], which
/// the box may have been given with the shown text, taken out again unless
/// the file's own name has them.
pub fn typed_name(typed: &str, old_name: &str) -> String {
    if old_name.contains(EXT_MARKS) { typed.to_string() } else { typed.replace(EXT_MARKS, "") }
}

/// A path for display, kept left to right even when folder names are Arabic
/// and the text around it is right to left, so «\» stays between the
/// folders it separates: the whole path is a left-to-right embedding
/// (LRE … PDF) with left-to-right marks around every separator.
///
/// Not an isolate (LRI … PDI): the lists, labels and the quick popup are
/// drawn by GDI, which shows U+2066 and U+2069 as dotted boxes around the
/// path. LRE, PDF and LRM are drawn as nothing by every Windows.
///
/// `plain_ok`: the text around the path is left to right (the English
/// interface), so a path with no right-to-left letters is returned as it is.
/// Display only: never a path to open or copy.
pub fn ltr_path(path: &str, plain_ok: bool) -> String {
    if plain_ok && !path.chars().any(is_rtl) {
        return path.to_owned();
    }
    let mut s = String::with_capacity(path.len() + 24);
    s.push_str("\u{202A}\u{200E}");
    for c in path.chars() {
        if c == '\\' {
            s.push_str("\u{200E}\\\u{200E}");
        } else {
            s.push(c);
        }
    }
    s.push_str("\u{200E}\u{202C}");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_paths_have_no_isolates_for_gdi_to_draw_as_boxes() {
        // GDI draws LRI, RLI, FSI and PDI (U+2066–U+2069) as dotted boxes.
        let boxed = |s: &str| s.chars().any(|c| ('\u{2066}'..='\u{2069}').contains(&c));
        for path in ["C:\\المستندات\\تقرير.pdf", "C:\\Users\\me\\report.pdf", "", "\\\\server\\مشاركة\\ملف ٢٠٢٦.xlsx", "D:\\"] {
            for plain_ok in [false, true] {
                let shown = ltr_path(path, plain_ok);
                assert!(!boxed(&shown), "{shown:?}");
            }
        }
    }

    #[test]
    fn display_paths_are_left_to_right_embeddings() {
        let shown = ltr_path("C:\\المستندات\\تقرير.pdf", false);
        assert_eq!(shown, "\u{202A}\u{200E}C:\u{200E}\\\u{200E}المستندات\u{200E}\\\u{200E}تقرير.pdf\u{200E}\u{202C}");
        // The embedding is closed, so it cannot reorder what follows it.
        assert_eq!(shown.matches('\u{202A}').count(), shown.matches('\u{202C}').count());
        // The English window: only a path with right-to-left letters needs it.
        assert_eq!(ltr_path("C:\\Users\\me\\report.pdf", true), "C:\\Users\\me\\report.pdf");
        assert!(ltr_path("C:\\Users\\me\\تقرير.pdf", true).starts_with('\u{202A}'));
        assert!(ltr_path("C:\\Users\\me\\report.pdf", false).starts_with('\u{202A}'));
    }

    #[test]
    fn display_paths_read_left_to_right_in_a_right_to_left_line() {
        // In a right-to-left line (the Arabic window) the parts must come in
        // order from the left: C:, «\», «المستندات», «\», the file.
        let path = "C:\\المستندات\\تقرير.pdf";
        // Where a piece of `text` is drawn: its leftmost and rightmost cell.
        fn cells(text: &str, piece: &str, from: usize) -> (i32, i32) {
            let spans = visual_spans(text, &vec![1; text.chars().count()], true).unwrap();
            let byte = from + text[from..].find(piece).unwrap();
            let first = text[..byte].chars().count();
            let xs = spans[first..first + piece.chars().count()].iter().map(|s| s.0);
            (xs.clone().min().unwrap(), xs.max().unwrap())
        }
        let shown = ltr_path(path, false);
        let second = shown.rfind('\\').unwrap();
        let order = [cells(&shown, "C:", 0), cells(&shown, "\\", 0), cells(&shown, "المستندات", 0), cells(&shown, "\\", second), cells(&shown, "تقرير.pdf", 0)];
        for pair in order.windows(2) {
            assert!(pair[0].1 < pair[1].0, "{order:?}");
        }
        // Without the marks the same line reads the other way round.
        assert!(cells(path, "تقرير", 0).1 < cells(path, "C:", 0).0);
    }

    #[test]
    fn display_paths_keep_their_text() {
        const PIECES: &[&str] = &["a", "ب", "تقرير", "\u{064E}", "\u{0640}", "\u{200F}", "\u{200E}", "👨‍👩‍👧", "\\", ",", "\"", " ", "١", ".", "\u{FFFD}", "\0"];
        let mut x = 0x1234_5678_9ABC_DEF1u64;
        let mut next = |m: usize| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % m as u64) as usize
        };
        for _ in 0..3000 {
            let s: String = (0..next(10)).map(|_| PIECES[next(PIECES.len())]).collect();
            for plain_ok in [false, true] {
                let shown = ltr_path(&s, plain_ok);
                if shown == s {
                    assert!(plain_ok && !s.chars().any(is_rtl), "{s:?}");
                    continue;
                }
                // Only the embedding around it and left-to-right marks are added.
                let inner = shown.strip_prefix('\u{202A}').and_then(|t| t.strip_suffix('\u{202C}')).unwrap_or_else(|| panic!("{shown:?}"));
                assert_eq!(inner.replace('\u{200E}', ""), s.replace('\u{200E}', ""), "{s:?}");
            }
        }
    }

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

    #[test]
    fn extensions_set_apart() {
        assert_eq!(extension_at("تقرير.pdf", false), Some(10));
        assert_eq!(extension_at("a.tar.gz", false), Some(5));
        assert_eq!(extension_at("report.pdf", true), None);
        for none in ["README", ".gitignore", "  .txt", "د. أحمد", "ملف.نص", "a.", "a.b c", "a.verylongextension12"] {
            assert_eq!(extension_at(none, false), None, "{none}");
        }
    }

    #[test]
    fn right_to_left_names_keep_the_extension_whole() {
        // The bug: one piece of text, the dot between "xlsx" and the number.
        assert_eq!(order("تقرير المبيعات ٢٠٢٦.xlsx", true), "xlsx.٢٠٢٦ تاعيبملا ريرقت");
        // With the marks: ".xlsx" whole, on the left, apart from the number.
        let (shown, at) = list_name("تقرير المبيعات ٢٠٢٦.xlsx", false);
        assert_eq!(at, Some("تقرير المبيعات ٢٠٢٦".len()));
        assert_eq!(shown, "تقرير المبيعات ٢٠٢٦\u{200F}\u{200E}.xlsx");
        let visible = |s: String| s.replace(['\u{200F}', '\u{200E}'], "");
        assert_eq!(visible(order(&shown, true)), ".xlsx٢٠٢٦ تاعيبملا ريرقت");
        assert_eq!(visible(order(&list_name("تقرير 2026.xlsx", false).0, true)), ".xlsx2026 ريرقت");
        assert_eq!(visible(order(&list_name("التقرير النهائي.pdf", false).0, true)), ".pdfيئاهنلا ريرقتلا");
        // The first letter decides, not a number before it.
        assert_eq!(visible(order(&list_name("2026 تقرير.pdf", false).0, true)), ".pdfريرقت 2026");
        // In the English (left-to-right) list it reads as before.
        assert_eq!(visible(order(&list_name("تقرير.pdf", false).0, false)), "ريرقت.pdf");
        assert_eq!(visible(order(&list_name("تقرير ٢٠٢٦.pdf", false).0, false)), order("تقرير ٢٠٢٦.pdf", false));
    }

    #[test]
    fn other_names_unchanged() {
        for name in ["report 2026.xlsx", "Q3 تقرير.pdf", "مجلد.2026", "README", "ملف.نص"] {
            let is_dir = name == "مجلد.2026";
            assert_eq!(list_name(name, is_dir), (name.to_string(), None), "{name}");
        }
        assert_eq!(order(&list_name("report 2026.xlsx", false).0, true), "report 2026.xlsx");
    }

    #[test]
    fn match_ranges_follow_the_marks() {
        let name = "تقرير 2026.xlsx";
        let (shown, at) = list_name(name, false);
        let dot = name.find('.').unwrap();
        // "تقرير", "2026.x" across the dot, and "xlsx".
        let ranges = vec![(0, "تقرير".len()), (dot - 4, dot + 2), (dot + 1, name.len())];
        let moved = shift_ranges(ranges.clone(), at);
        assert_eq!(&shown[moved[0].0..moved[0].1], "تقرير");
        assert_eq!(shown[moved[1].0..moved[1].1].replace(EXT_MARKS, ""), "2026.x");
        assert_eq!(&shown[moved[2].0..moved[2].1], "xlsx");
        // A range ending at the dot stays before the marks.
        assert_eq!(shift_ranges(vec![(0, dot)], at), vec![(0, dot)]);
        assert_eq!(shift_ranges(ranges.clone(), None), ranges);
        // Every moved range is on character boundaries of the shown text.
        assert!(moved.iter().all(|&(s, e)| shown.is_char_boundary(s) && shown.is_char_boundary(e)));
    }

    #[test]
    fn rename_box_text_loses_the_marks() {
        let (shown, _) = list_name("تقرير 2026.xlsx", false);
        assert_eq!(typed_name(&shown, "تقرير 2026.xlsx"), "تقرير 2026.xlsx");
        assert_eq!(typed_name(&shown.replace("2026", "2027"), "تقرير 2026.xlsx"), "تقرير 2027.xlsx");
        // A name that really has them keeps them.
        let odd = format!("a{EXT_MARKS}.txt");
        assert_eq!(typed_name(&odd, &odd), odd);
    }
}
