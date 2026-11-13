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

use unicode_bidi::{BidiClass, BidiInfo, Level, bidi_class};

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
/// of the name as it reads (on its left), apart from the number. Its phone
/// numbers, dates and versions are kept in the order they were typed
/// ([`digit_marks`]; `rtl`: the list reads right to left). Names that start
/// left to right and have no such number are unchanged. Returns the text
/// and the marks put in, to move the match ranges after them
/// ([`shift_ranges`]). Display only: never a name to rename or open.
pub fn list_name(name: &str, is_dir: bool, rtl: bool) -> (String, Marks) {
    let at = extension_at(name, is_dir).filter(|&dot| {
        name[..dot].chars().find(|c| c.is_alphabetic()).is_some_and(is_rtl)
    });
    let mut marks = digit_marks(name, rtl);
    if let Some(dot) = at {
        // After a run's closing mark at the same place (the sort is stable).
        marks.push((dot, EXT_MARKS));
        marks.sort_by_key(|m| m.0);
    }
    (insert_marks(name, &marks), marks)
}

/// Invisible marks put into a text for display: each is inserted before the
/// character at its byte position in the original text, in order.
pub type Marks = Vec<(usize, &'static str)>;

/// `text` with `marks` put in.
pub fn insert_marks(text: &str, marks: &[(usize, &str)]) -> String {
    let mut out = String::with_capacity(text.len() + marks.iter().map(|m| m.1.len()).sum::<usize>());
    let mut from = 0;
    for &(at, mark) in marks {
        let at = at.clamp(from, text.len());
        out.push_str(&text[from..at]);
        out.push_str(mark);
        from = at;
    }
    out.push_str(&text[from..]);
    out
}

/// Byte ranges into a text, moved to the same characters in it with
/// `marks` put in ([`insert_marks`], [`list_name`]). A range starts after
/// the marks at its start and ends before the marks at its end, so it never
/// covers a mark on its edge.
pub fn shift_ranges(ranges: Vec<(usize, usize)>, marks: &[(usize, &str)]) -> Vec<(usize, usize)> {
    if marks.is_empty() {
        return ranges;
    }
    let before = |b: usize, inclusive: bool| -> usize { marks.iter().filter(|m| if inclusive { m.0 <= b } else { m.0 < b }).map(|m| m.1.len()).sum() };
    ranges.into_iter().map(|(s, e)| (s + before(s, true), e + before(e, false))).collect()
}

/// The name the user typed in the list's rename box: the marks
/// [`list_name`] puts in, which the box may have been given with the shown
/// text, taken out again unless the file's own name has such marks.
pub fn typed_name(typed: &str, old_name: &str) -> String {
    if !old_name.contains([LRM, RLM]) {
        typed.replace([LRM, RLM], "")
    } else if old_name.contains(EXT_MARKS) {
        typed.to_string()
    } else {
        typed.replace(EXT_MARKS, "")
    }
}

const LRM: char = '\u{200E}';
const RLM: char = '\u{200F}';

/// A digit of a number: European (0–9), Arabic-Indic (٠–٩) or Eastern
/// Arabic-Indic (۰–۹).
fn is_digit(c: char) -> bool {
    c.is_ascii_digit() || ('\u{0660}'..='\u{0669}').contains(&c) || ('\u{06F0}'..='\u{06F9}').contains(&c)
}

/// What joins the groups of digits of a phone number (0770-123-4567), a
/// date (2026-11-12, 12/11/2026), a version (2.0.1), a time (10:30) or a
/// range (2024–2026). Besides the ASCII ones: the hyphens U+2010 and U+2011
/// (non-breaking), the figure dash U+2012, the en dash U+2013 (the usual
/// range dash, which Word puts in by itself), the minus sign U+2212 and the
/// division slash U+2215. The same list as `DIGIT_RUN` in
/// `webui/assets/app.js`.
fn is_joiner(c: char) -> bool {
    matches!(c, '-' | '/' | '.' | ':' | ',' | '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2212}' | '\u{2215}')
}

/// Byte ranges of the digit runs of `text`: two or more groups of digits,
/// each joined to the next by one [`is_joiner`] and nothing else (no space).
/// A lone number is not a run.
pub fn digit_runs(text: &str) -> Vec<(usize, usize)> {
    let cs: Vec<(usize, char)> = text.char_indices().collect();
    let at = |i: usize| cs.get(i).map_or(text.len(), |c| c.0);
    let digit = |i: usize| cs.get(i).is_some_and(|c| is_digit(c.1));
    let mut runs = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        if !digit(i) {
            i += 1;
            continue;
        }
        let start = i;
        let mut groups = 0;
        loop {
            while digit(i) {
                i += 1;
            }
            groups += 1;
            if cs.get(i).is_some_and(|c| is_joiner(c.1)) && digit(i + 1) {
                i += 1;
            } else {
                break;
            }
        }
        if groups > 1 {
            runs.push((at(start), at(i)));
        }
    }
    runs
}

/// The marks that keep every digit run of `text` ([`digit_runs`]) in the
/// order it was typed, when shown in a line whose reading order is right to
/// left (`rtl`) or left to right.
///
/// The bidi algorithm takes digits after an Arabic letter for Arabic
/// numbers (rule W2), as ٠–٩ always are, and only a common separator
/// (/ . : ,) joins Arabic numbers (W4), never "-". So in «هاتف 0770-123-4567»
/// each group is a piece of its own and a right-to-left line shows
/// "4567-123-0770"; «بتاريخ 2026-11-12» shows "12-11-2026". Such a run gets a
/// left-to-right mark (LRM) before it, on both sides of each separator and
/// after it:
/// strong left-to-right text all through, so the run is one left-to-right
/// piece, whatever the line's direction. In a right-to-left line, when the
/// text before the run reads right to left, a right-to-left mark (RLM) after
/// it gives what follows that context back (a number after it stays apart
/// from the run).
///
/// Marks, not isolates (LRI … PDI): GDI draws U+2066–U+2069 as boxes (see
/// [`ltr_path`]), and LRM and RLM are drawn as nothing by every Windows.
/// Runs that already read in order (any run after a Latin letter, or
/// "12/11/2026" after an Arabic one) get nothing. With no strong letter
/// before the run, a right-to-left line counts as Arabic: Windows takes the
/// digits of an Arabic line for Arabic ones there too (see
/// `quick::embed_quoted`).
pub fn digit_marks(text: &str, rtl: bool) -> Marks {
    let mut marks = Vec::new();
    for (s, e) in digit_runs(text) {
        let before = text[..s].chars().rev().map(bidi_class).find(|c| matches!(c, BidiClass::L | BidiClass::R | BidiClass::AL));
        // In a left-to-right line the mark would pull the space after the
        // run into the right-to-left words after it, gluing them to the run.
        let close = rtl && before.is_none_or(|c| c != BidiClass::L);
        let run = &text[s..e];
        // W2, W4 and W7. `arabic`: digits 0–9 count as Arabic numbers;
        // `after_l`: a left-to-right letter (or the start of a left-to-right
        // line) comes first, so European digits become left-to-right letters
        // (W7) and any separator between them follows (N1). Otherwise
        // European digits are joined only by a European separator (-, the
        // minus sign) or a common one (/ . : ,), W4, and Arabic numbers only
        // by a common one.
        // The dashes U+2010–U+2013 and the division slash are neither: in
        // «2024–2026» after an Arabic word the en dash split the range in two
        // and the line showed "2026–2024".
        let in_order = |arabic: bool, after_l: bool| {
            let an: Vec<bool> = run.chars().filter(|&c| is_digit(c)).map(|c| arabic || bidi_class(c) == BidiClass::AN).collect();
            let joined_by = |ok: &dyn Fn(BidiClass) -> bool| run.chars().filter(|&c| is_joiner(c)).all(|c| ok(bidi_class(c)));
            (an.iter().all(|&a| !a) && (after_l || joined_by(&|k| matches!(k, BidiClass::ES | BidiClass::CS))))
                || (an.iter().all(|&a| a) && joined_by(&|k| k == BidiClass::CS))
        };
        let fine = match before {
            Some(c) => in_order(c == BidiClass::AL, c == BidiClass::L),
            // The algorithm's European digits, and Windows' Arabic ones.
            None => in_order(false, !rtl) && (!rtl || in_order(true, false)),
        };
        if fine {
            continue;
        }
        marks.push((s, "\u{200E}"));
        for (i, c) in run.char_indices() {
            if is_joiner(c) {
                marks.push((s + i, "\u{200E}"));
                marks.push((s + i + c.len_utf8(), "\u{200E}"));
            }
        }
        // Also after it: a last group of Arabic numbers would join what follows.
        marks.push((e, "\u{200E}"));
        if close {
            marks.push((e, "\u{200F}"));
        }
    }
    marks
}

/// `text` with its digit runs kept in the order they were typed
/// ([`digit_marks`]). Display only: never a name or a text to open, copy or
/// search with.
pub fn ordered_digits(text: &str, rtl: bool) -> String {
    let marks = digit_marks(text, rtl);
    if marks.is_empty() { text.to_string() } else { insert_marks(text, &marks) }
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
    // Each folder name starts after a left-to-right mark, inside a
    // left-to-right embedding: its digit runs as in a left-to-right line.
    for (i, part) in path.split('\\').enumerate() {
        if i > 0 {
            s.push_str("\u{200E}\\\u{200E}");
        }
        s.push_str(&ordered_digits(part, false));
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
        const PIECES: &[&str] = &["a", "ب", "تقرير", "\u{064E}", "\u{0640}", "\u{200F}", "\u{200E}", "👨‍👩‍👧", "\\", ",", "\"", " ", "١", ".", "-", "2", "\u{FFFD}", "\0"];
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
                // Only the embedding around it and left-to-right marks (also
                // around a digit run such as "١-١") are added.
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
        let (shown, at) = list_name("تقرير المبيعات ٢٠٢٦.xlsx", false, true);
        assert_eq!(at, vec![("تقرير المبيعات ٢٠٢٦".len(), EXT_MARKS)]);
        assert_eq!(shown, "تقرير المبيعات ٢٠٢٦\u{200F}\u{200E}.xlsx");
        let visible = |s: String| s.replace(['\u{200F}', '\u{200E}'], "");
        assert_eq!(visible(order(&shown, true)), ".xlsx٢٠٢٦ تاعيبملا ريرقت");
        assert_eq!(visible(order(&list_name("تقرير 2026.xlsx", false, true).0, true)), ".xlsx2026 ريرقت");
        assert_eq!(visible(order(&list_name("التقرير النهائي.pdf", false, true).0, true)), ".pdfيئاهنلا ريرقتلا");
        // The first letter decides, not a number before it.
        assert_eq!(visible(order(&list_name("2026 تقرير.pdf", false, true).0, true)), ".pdfريرقت 2026");
        // In the English (left-to-right) list it reads as before.
        assert_eq!(visible(order(&list_name("تقرير.pdf", false, false).0, false)), "ريرقت.pdf");
        assert_eq!(visible(order(&list_name("تقرير ٢٠٢٦.pdf", false, false).0, false)), order("تقرير ٢٠٢٦.pdf", false));
    }

    #[test]
    fn other_names_unchanged() {
        for name in ["report 2026.xlsx", "Q3 تقرير.pdf", "مجلد.2026", "README", "ملف.نص"] {
            let is_dir = name == "مجلد.2026";
            for rtl in [true, false] {
                assert_eq!(list_name(name, is_dir, rtl), (name.to_string(), Vec::new()), "{name}");
            }
        }
        assert_eq!(order(&list_name("report 2026.xlsx", false, true).0, true), "report 2026.xlsx");
    }

    #[test]
    fn match_ranges_follow_the_marks() {
        let name = "تقرير 2026.xlsx";
        let (shown, at) = list_name(name, false, true);
        let dot = name.find('.').unwrap();
        // "تقرير", "2026.x" across the dot, and "xlsx".
        let ranges = vec![(0, "تقرير".len()), (dot - 4, dot + 2), (dot + 1, name.len())];
        let moved = shift_ranges(ranges.clone(), &at);
        assert_eq!(&shown[moved[0].0..moved[0].1], "تقرير");
        assert_eq!(shown[moved[1].0..moved[1].1].replace(EXT_MARKS, ""), "2026.x");
        assert_eq!(&shown[moved[2].0..moved[2].1], "xlsx");
        // A range ending at the dot stays before the marks.
        assert_eq!(shift_ranges(vec![(0, dot)], &at), vec![(0, dot)]);
        assert_eq!(shift_ranges(ranges.clone(), &[]), ranges);
        // Every moved range is on character boundaries of the shown text.
        assert!(moved.iter().all(|&(s, e)| shown.is_char_boundary(s) && shown.is_char_boundary(e)));
    }

    #[test]
    fn rename_box_text_loses_the_marks() {
        let (shown, _) = list_name("تقرير 2026.xlsx", false, true);
        assert_eq!(typed_name(&shown, "تقرير 2026.xlsx"), "تقرير 2026.xlsx");
        assert_eq!(typed_name(&shown.replace("2026", "2027"), "تقرير 2026.xlsx"), "تقرير 2027.xlsx");
        // A name that really has them keeps them.
        let odd = format!("a{EXT_MARKS}.txt");
        assert_eq!(typed_name(&odd, &odd), odd);
    }

    // ------------------------------------------------ digit runs

    /// The example from a real Arabic Windows 11, where the list showed the
    /// phone as 4567-123-0770 and the date as 12-11-2026.
    const INVOICE: &str = "فاتورة رقم 1234 بتاريخ 2026-11-12 من شركة Microsoft هاتف 0770-123-4567.txt";

    /// The left-to-right screen order of `text`'s characters with every
    /// invisible mark dropped: a run reads in order when it appears in this
    /// string as typed.
    fn seen(text: &str, rtl: bool) -> String {
        order(text, rtl).replace([LRM, RLM, '\u{202A}', '\u{202C}'], "")
    }

    fn runs_of(text: &str) -> Vec<&str> {
        digit_runs(text).into_iter().map(|(s, e)| &text[s..e]).collect()
    }

    #[test]
    fn digit_runs_are_numbers_joined_by_one_separator() {
        assert_eq!(runs_of(INVOICE), ["2026-11-12", "0770-123-4567"]);
        assert_eq!(runs_of("هاتف 0770-123-4567"), ["0770-123-4567"]);
        assert_eq!(runs_of("بتاريخ 12/11/2026"), ["12/11/2026"]);
        assert_eq!(runs_of("الإصدار 2.0.1.zip"), ["2.0.1"]);
        assert_eq!(runs_of("الساعة 10:30"), ["10:30"]);
        assert_eq!(runs_of("المبلغ 1,250,000"), ["1,250,000"]);
        assert_eq!(runs_of("تاريخ ٢٠٢٦-١١-١٢"), ["٢٠٢٦-١١-١٢"]);
        assert_eq!(runs_of("تاريخ ۲۰۲۶/۱۱/۱۲"), ["۲۰۲۶/۱۱/۱۲"]);
        assert_eq!(runs_of("Invoice 2026-11-12 v2.0.1"), ["2026-11-12", "2.0.1"]);
        assert_eq!(runs_of("2026-11-12"), ["2026-11-12"]);
        assert_eq!(runs_of("a2026-11-12b"), ["2026-11-12"]);
        // Ranges and dates with the typographic dashes and slashes.
        assert_eq!(runs_of("التقرير السنوي 2024\u{2013}2026.pdf"), ["2024\u{2013}2026"]);
        for j in ['\u{2010}', '\u{2011}', '\u{2012}', '\u{2013}', '\u{2212}', '\u{2215}'] {
            let run = format!("12{j}11{j}2026");
            assert_eq!(runs_of(&format!("تاريخ {run} من")), [run.as_str()], "U+{:04X}", j as u32);
        }
        // Not joiners: the em dash, a dash with spaces, two dashes.
        for none in ["2024\u{2014}2026", "2024 \u{2013} 2026", "2024\u{2013}\u{2013}2026", "\u{2013}5", "5\u{2212}"] {
            assert_eq!(runs_of(none), Vec::<&str>::new(), "{none}");
        }
        // Not runs: no digits, a lone number, a separator with no digits on
        // one side, two separators, a space, the extension's dot.
        for none in ["", "تقرير", "report.pdf", "7", "ملف 1234", "-5", "5-", "1--2", "1- 2", "1 2", "2026.xlsx", "تقرير 2026.xlsx", "1+2", "١٢٣"] {
            assert_eq!(runs_of(none), Vec::<&str>::new(), "{none}");
        }
    }

    #[test]
    fn digit_runs_read_as_typed_in_the_arabic_list() {
        // The bug, as the bidi algorithm lays it out.
        assert!(seen(INVOICE, true).contains("4567-123-0770"));
        assert!(seen(INVOICE, true).contains("12-11-2026"));
        let shown = ordered_digits(INVOICE, true);
        assert!(seen(&shown, true).contains("0770-123-4567"), "{}", seen(&shown, true));
        assert!(seen(&shown, true).contains("2026-11-12"));
        // The words keep their places: «فاتورة» still starts the line on the right.
        assert!(seen(&shown, true).ends_with("ةروتاف"));
        // The same name in the list, its extension set apart.
        let (listed, _) = list_name(INVOICE, false, true);
        let line = seen(&listed, true);
        assert!(line.starts_with(".txt0770-123-4567 فتاه"), "{line}");
        assert!(line.contains("2026-11-12"));
        assert_eq!(listed.replace([LRM, RLM], ""), INVOICE);
    }

    #[test]
    fn digit_runs_read_as_typed_in_the_english_list() {
        assert!(seen(INVOICE, false).contains("12-11-2026"));
        let shown = ordered_digits(INVOICE, false);
        assert!(seen(&shown, false).contains("2026-11-12"), "{}", seen(&shown, false));
        assert!(seen(&shown, false).contains("0770-123-4567"));
        let (listed, _) = list_name(INVOICE, false, false);
        assert!(seen(&listed, false).contains("0770-123-4567"));
        // English names are left alone.
        for name in ["Invoice 2026-11-12.pdf", "Call 0770-123-4567.txt", "v2.0.1 notes 10:30.md", "12/11/2026"] {
            assert_eq!(ordered_digits(name, false), name);
            assert_eq!(ordered_digits(name, true), if name.starts_with(|c: char| c.is_ascii_digit()) { ordered_digits(name, true) } else { name.to_string() });
            assert_eq!(list_name(name, false, false).1, Vec::new(), "{name}");
        }
    }

    #[test]
    fn each_kind_of_run_reads_as_typed() {
        for run in [
            "0770-123-4567", "2026-11-12", "12/11/2026", "2.0.1", "10:30", "1,250", "٢٠٢٦-١١-١٢", "١٢/١١/٢٠٢٦", "۲۰۲۶-۱۱-۱۲", "2026-١١-12", "1/2-3:4.5,6",
            "2024\u{2013}2026", "2024\u{2010}2026", "0770\u{2011}123\u{2011}4567", "2024\u{2012}2026", "5\u{2212}3", "12\u{2215}11\u{2215}2026", "٢٠٢٤\u{2013}٢٠٢٦", "۲۰۲۴\u{2013}۲۰۲۶",
            "2024\u{2013}11-12",
        ] {
            for text in [format!("بتاريخ {run} من"), format!("Report {run} done"), format!("{run} تقرير"), format!("تقرير {run}"), run.to_string(), format!("שלום {run} עולם")] {
                for rtl in [true, false] {
                    let shown = ordered_digits(&text, rtl);
                    assert!(seen(&shown, rtl).contains(run), "{text:?} rtl={rtl}: {:?}", seen(&shown, rtl));
                    assert_eq!(shown.replace([LRM, RLM], ""), text);
                }
            }
        }
    }

    /// The round-2 PC test: «2024–2026» with an EN DASH still showed as
    /// "2026–2024" in both looks. The dash is a neutral (ON), not a
    /// separator, so even European digits are not joined by it in a
    /// right-to-left line.
    #[test]
    fn a_range_with_an_en_dash_reads_as_typed() {
        let range = "2024\u{2013}2026";
        let reversed = "2026\u{2013}2024";
        for text in ["التقرير السنوي 2024\u{2013}2026.pdf", "2024\u{2013}2026 تقرير", "שנים 2024\u{2013}2026"] {
            assert!(seen(text, true).contains(reversed), "{text}: the bug");
            assert!(!digit_marks(text, true).is_empty(), "{text}");
            assert!(seen(&ordered_digits(text, true), true).contains(range), "{text}: {:?}", seen(&ordered_digits(text, true), true));
            let (listed, _) = list_name(text, false, true);
            assert!(seen(&listed, true).contains(range), "{text}");
        }
        // The English list with an Arabic name, too.
        let text = "التقرير السنوي 2024\u{2013}2026.pdf";
        assert!(seen(text, false).contains(reversed), "the bug in the English list");
        assert!(seen(&ordered_digits(text, false), false).contains(range));
        // After a Latin word the digits are left-to-right letters (W7): nothing to do.
        assert_eq!(digit_marks("Report 2024\u{2013}2026", true), Vec::new());
        assert_eq!(digit_marks("Report 2024\u{2013}2026", false), Vec::new());
        assert_eq!(digit_marks(range, false), Vec::new());
        // With no letter before it, a right-to-left line needs the marks.
        assert!(seen(range, true).contains(reversed));
        assert!(seen(&ordered_digits(range, true), true).contains(range));
        // A path shows it in order as well.
        let path = "C:\\التقارير\\التقرير السنوي 2024\u{2013}2026.pdf";
        assert!(seen(&ltr_path(path, false), true).contains(range));
    }

    /// The new look finds the runs with `DIGIT_RUN` in app.js: the same
    /// joiners, the same runs.
    #[test]
    fn the_new_look_joins_digits_with_the_same_characters() {
        let js = include_str!("webui/assets/app.js");
        let line = js.lines().find(|l| l.contains("const DIGIT_RUN")).expect("DIGIT_RUN in app.js");
        assert!(line.contains(r"(?:[-/.:,\u2010-\u2013\u2212\u2215][0-9\u0660-\u0669\u06F0-\u06F9]+)+"), "{line}");
        let in_js = |c: char| c.is_ascii() && "-/.:,".contains(c) || ('\u{2010}'..='\u{2013}').contains(&c) || c == '\u{2212}' || c == '\u{2215}';
        for c in (0u32..0x3000).filter_map(char::from_u32) {
            assert_eq!(is_joiner(c), in_js(c), "U+{:04X}", c as u32);
        }
    }

    #[test]
    fn runs_that_already_read_in_order_get_no_marks() {
        // After a Latin letter the digits are European: every separator joins them.
        assert_eq!(digit_marks("Microsoft 0770-123-4567", true), Vec::new());
        assert_eq!(digit_marks("Microsoft 0770-123-4567", false), Vec::new());
        // After an Arabic letter they are Arabic numbers, which / . : , join.
        for text in ["بتاريخ 12/11/2026", "الإصدار 2.0.1", "الساعة 10:30", "المبلغ 1,250,000", "تاريخ ١٢/١١/٢٠٢٦"] {
            assert_eq!(digit_marks(text, true), Vec::new(), "{text}");
            assert_eq!(digit_marks(text, false), Vec::new(), "{text}");
            assert!(seen(text, true).contains(&text[digit_runs(text)[0].0..]), "{text}");
        }
        // A lone number, or none.
        for text in ["ملف 1234", "ملف ١٢٣٤", "تقرير", "", "7"] {
            assert_eq!(digit_marks(text, true), Vec::new(), "{text}");
        }
    }

    #[test]
    fn arabic_indic_digits_follow_the_same_rule() {
        // ٠–٩ are Arabic numbers wherever they are: "-" never joins them,
        // even after a Latin word; "/" does.
        for text in ["تاريخ ٢٠٢٦-١١-١٢", "Date ٢٠٢٦-١١-١٢", "٢٠٢٦-١١-١٢"] {
            for rtl in [true, false] {
                assert!(!seen(text, rtl).contains("٢٠٢٦-١١-١٢"), "{text}: the bug");
                assert!(!digit_marks(text, rtl).is_empty(), "{text}");
                assert!(seen(&ordered_digits(text, rtl), rtl).contains("٢٠٢٦-١١-١٢"), "{text} rtl={rtl}");
            }
        }
        assert_eq!(digit_marks("Date ١٢/١١/٢٠٢٦", true), Vec::new());
        // Eastern Arabic-Indic digits (۰–۹) are European ones, as 0–9.
        assert_eq!(digit_marks("Date ۲۰۲۶-۱۱-۱۲", true), Vec::new());
        assert!(!digit_marks("تاريخ ۲۰۲۶-۱۱-۱۲", true).is_empty());
    }

    #[test]
    fn the_marks_are_left_to_right_marks_around_separators() {
        let text = "هاتف 0770-123-4567";
        let s = "هاتف ".len();
        assert_eq!(
            digit_marks(text, true),
            vec![(s, "\u{200E}"), (s + 4, "\u{200E}"), (s + 5, "\u{200E}"), (s + 8, "\u{200E}"), (s + 9, "\u{200E}"), (text.len(), "\u{200E}"), (text.len(), "\u{200F}")]
        );
        assert_eq!(ordered_digits(text, true), "هاتف \u{200E}0770\u{200E}-\u{200E}123\u{200E}-\u{200E}4567\u{200E}\u{200F}");
        // No isolates, which GDI draws as boxes.
        assert!(!ordered_digits(INVOICE, true).chars().any(|c| ('\u{2066}'..='\u{2069}').contains(&c)));
        // A run at the start of a name: marked in the Arabic list (Windows
        // takes those digits for Arabic ones there), not in the English one.
        assert_eq!(ordered_digits("2026-11-12 تقرير", true), "\u{200E}2026\u{200E}-\u{200E}11\u{200E}-\u{200E}12\u{200E}\u{200F} تقرير");
        assert_eq!(ordered_digits("2026-11-12 تقرير", false), "2026-11-12 تقرير");
        // At the end, before the extension, the run's closing mark comes
        // before the extension's marks.
        let (listed, marks) = list_name("هاتف 0770-123-4567.txt", false, true);
        assert!(listed.ends_with("4567\u{200E}\u{200F}\u{200F}\u{200E}.txt"), "{listed:?}");
        assert!(marks.windows(2).all(|w| w[0].0 <= w[1].0));
    }

    #[test]
    fn spaces_stay_around_a_run() {
        for rtl in [true, false] {
            let line = seen(&ordered_digits(INVOICE, rtl), rtl);
            assert!(line.contains(" 2026-11-12 ") && line.contains(if rtl { "0770-123-4567 " } else { " 0770-123-4567" }), "rtl={rtl}: {line}");
            let line = seen(&ordered_digits("تاريخ ٢٠٢٦-١١-١٢ من 5 ملفات", rtl), rtl);
            assert!(line.contains(" ٢٠٢٦-١١-١٢ "), "rtl={rtl}: {line}");
        }
        // No closing mark in the English list.
        assert!(!ordered_digits(INVOICE, false).contains(RLM));
    }

    #[test]
    fn a_number_after_a_run_stays_apart_from_it() {
        // «بتاريخ 2026-11-12 5 ملفات»: in the Arabic line the 5 comes after
        // the date as it reads, on its left, not glued to its end.
        let text = "بتاريخ 2026-11-12 5 ملفات";
        let shown = ordered_digits(text, true);
        let line = seen(&shown, true);
        assert!(line.contains("5 2026-11-12"), "{line}");
        assert_eq!(seen(text, true).find('5') < seen(text, true).find("2026"), line.find('5') < line.find("2026"));
    }

    #[test]
    fn highlight_ranges_follow_the_digit_marks() {
        let (shown, marks) = list_name(INVOICE, false, true);
        let find = |p: &str| {
            let s = INVOICE.find(p).unwrap();
            (s, s + p.len())
        };
        // Inside a run, across its start and end, a whole run, the
        // extension, and a word the marks don't touch.
        let pieces = ["123-45", "0770", "4567", "4567.t", "هاتف 0770", "2026-11-12", "-11-", "txt", "Microsoft", "1234"];
        let ranges: Vec<(usize, usize)> = pieces.iter().map(|p| find(p)).collect();
        let moved = shift_ranges(ranges, &marks);
        for (p, &(s, e)) in pieces.iter().zip(&moved) {
            assert!(shown.is_char_boundary(s) && shown.is_char_boundary(e), "{p}");
            let got = &shown[s..e];
            assert_eq!(got.replace([LRM, RLM], ""), *p);
            // A range never starts or ends on a mark.
            assert!(!got.starts_with([LRM, RLM]) && !got.ends_with([LRM, RLM]), "{p}: {got:?}");
        }
        // No marks, nothing moves.
        assert_eq!(shift_ranges(vec![(0, 3)], &[]), vec![(0, 3)]);
        // Marks at the same place both move the start.
        assert_eq!(shift_ranges(vec![(2, 4), (0, 2)], &[(2, "\u{200F}"), (2, EXT_MARKS)]), vec![(11, 13), (0, 2)]);
    }

    #[test]
    fn names_for_rename_copy_and_search_are_untouched() {
        // The marks are display only: taken out, the name is the name.
        let (listed, _) = list_name(INVOICE, false, true);
        assert_ne!(listed, INVOICE);
        assert_eq!(typed_name(&listed, INVOICE), INVOICE);
        assert_eq!(typed_name(&ordered_digits(INVOICE, true), INVOICE), INVOICE);
        // A name with its own marks keeps them.
        let own = "ملف\u{200F} 1-2";
        assert_eq!(typed_name(own, own), own);
    }

    #[test]
    fn display_paths_keep_digit_runs_in_order() {
        let path = "C:\\الفواتير\\فاتورة 2026-11-12 هاتف 0770-123-4567.txt";
        for plain_ok in [false, true] {
            let shown = ltr_path(path, plain_ok);
            let line = seen(&shown, true);
            assert!(line.contains("2026-11-12") && line.contains("0770-123-4567"), "{line}");
            assert!(line.starts_with("C:\\"), "{line}");
            assert_eq!(shown.replace([LRM, RLM, '\u{202A}', '\u{202C}'], ""), path);
            // The English window.
            assert!(seen(&shown, false).contains("0770-123-4567"));
        }
        // A folder with a date after a Latin name needs nothing.
        assert_eq!(ltr_path("C:\\Backup 2026-11-12\\a.txt", true), "C:\\Backup 2026-11-12\\a.txt");
    }

    #[test]
    fn random_text_keeps_every_run_in_order() {
        const PIECES: &[&str] = &[
            "ب", "تقرير", "a", "Word", "ש", "1", "23", "٤", "٥٦", "۷", "-", "/", ".", ":", ",", " ", "%", "+", "#", "\u{200F}", "\u{200E}", "\u{0651}", "😀",
            "\u{2010}", "\u{2011}", "\u{2012}", "\u{2013}", "\u{2014}", "\u{2212}", "\u{2215}",
        ];
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = |m: usize| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % m as u64) as usize
        };
        for _ in 0..4000 {
            let text: String = (0..next(12)).map(|_| PIECES[next(PIECES.len())]).collect();
            for rtl in [true, false] {
                let marks = digit_marks(&text, rtl);
                let shown = insert_marks(&text, &marks);
                assert_eq!(shown.replace([LRM, RLM], ""), text.replace([LRM, RLM], ""), "{text:?}");
                let n = shown.chars().count();
                let spans = visual_spans(&shown, &vec![1; n], rtl).unwrap();
                let char_at = |b: usize| shown[..b].chars().count();
                for (s, e) in digit_runs(&text) {
                    // The run's characters, where they are in the shown text, left to right on screen.
                    let xs: Vec<i32> = text[s..e].char_indices().map(|(i, _)| spans[char_at(shift_ranges(vec![(s + i, s + i)], &marks)[0].0)].0).collect();
                    assert!(xs.windows(2).all(|w| w[0] < w[1]), "{text:?} rtl={rtl}: run {:?} at {xs:?}", &text[s..e]);
                }
            }
        }
    }
}
