//! Regression tests for search bugs found in review, one group per bug.

use super::*;

/// 2024-05-15 (a Wednesday) 10:00 UTC.
const NOW: i64 = 1_715_767_200;

fn at(input: &str) -> Query {
    Query::parse_at(input, NOW, 0)
}

// ------------------------------------------------ regex: `$` and CRLF lines

/// `regex:…$` inside a document whose lines end in `\r\n` (most Windows
/// text files): `$` must match before the `\r`, as it does before `\n`.
#[test]
fn regex_end_of_line_matches_crlf_lines() {
    let unix = "رقم الفاتورة INV-2024\nالسطر الثاني\n";
    let windows = "رقم الفاتورة INV-2024\r\nالسطر الثاني\r\n";
    for text in [unix, windows] {
        assert!(match_text(text, &at(r"regex:inv-\d+$")).is_some(), "{text:?}");
        assert!(match_text(text, &at("regex:الثاني$")).is_some(), "{text:?}");
        assert_eq!(all_matches(text, &at(r"regex:\S$"), 50).0, 2, "{text:?}");
        // `^` and a whole-line pattern.
        assert!(match_text(text, &at("regex:^السطر")).is_some(), "{text:?}");
        assert!(match_text(text, &at(r#"regex:"^السطر الثاني$""#)).is_some(), "{text:?}");
        // Not the end of a line.
        assert!(match_text(text, &at("regex:الفاتورة$")).is_none(), "{text:?}");
    }
    // `.` stops at the line break either way, so a match never holds a stray `\r`.
    let hit = content_hit(0, windows, &at("regex:INV.*")).unwrap();
    assert_eq!(hit.lines, vec![(1, "رقم الفاتورة INV-2024".to_string())]);
    assert_eq!(match_ranges("a\r\nb", &at("regex:a.*")), vec![(0, 1)]);
    // File names have no line breaks: nothing changes there.
    let e = Entry { path: r"C:\a\IMG_1.JPG".into(), size: 1, mtime: 0, is_dir: false, norm_name: "img_1.jpg".into() };
    assert!(at(r"regex:\.jpg$").entry_ok(&e));
}
