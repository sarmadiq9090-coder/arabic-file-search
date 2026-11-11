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

// ------------------------------------------------- names starting with a dot

fn entry(path: &str, is_dir: bool) -> Entry {
    let name = path.rsplit(['\\', '/']).next().unwrap();
    Entry { path: path.into(), size: 1, mtime: 0, is_dir, norm_name: normalize(name).into() }
}

fn dot_entries() -> Vec<Entry> {
    vec![
        entry(r"C:\repo\.gitignore", false),                                  // 0
        entry(r"C:\repo\.git", true),                                         // 1
        entry(r"C:\repo\.env", false),                                        // 2
        entry(r"C:\repo\.ENV.local", false),                                  // 3
        entry(r"C:\Users\a\.ssh", true),                                      // 4
        entry(r"C:\Ref\.NETFramework", true),                                 // 5
        entry(r"C:\dl\Microsoft .NET Framework 4.8 Setup.exe", false),        // 6
        entry(r"C:\Windows\Microsoft.NET", true),                             // 7
        entry(r"C:\repo\prod.env", false),                                    // 8
        entry(r"C:\repo\environment.txt", false),                             // 9
        entry(r"C:\repo\readme.gitignore.bak", false),                        // 10
        entry(r"C:\docs\تقرير.pdf", false),                                   // 11
        entry(r"C:\docs\framework notes.net", false),                         // 12
        entry(r"C:\docs\بلا امتداد", false),                                  // 13
    ]
}

fn names(q: &str) -> Vec<u32> {
    let mut hits = search_names(&dot_entries(), &at(q), "");
    hits.sort_unstable();
    hits
}

/// `.gitignore`, `.env`, `.ssh`: a name that is all "extension" has none
/// for the index, and the `.word` filter then found nothing.
#[test]
fn dot_names_are_found() {
    assert_eq!(names(".gitignore"), vec![0]);
    assert_eq!(names(".GitIgnore"), vec![0]);
    // The file `.env`, its variants, and files whose extension is `env`.
    assert_eq!(names(".env"), vec![2, 3, 8]);
    // A folder too.
    assert_eq!(names(".ssh"), vec![4]);
    assert_eq!(names(".ssh folder:"), vec![4]);
    assert_eq!(names("مجلد: .git"), vec![1]);
    // `.git` starts `.gitignore` as well, like any typed part of a name.
    assert_eq!(names(".git"), vec![0, 1]);
    // A dot word with more words: «.NET Framework».
    assert_eq!(names(".net framework"), vec![5, 6, 12]);
    assert_eq!(names("framework .net"), vec![5, 6, 12]);
    // Only at the start of a word: not «Microsoft.NET», not `.gitignore.bak`.
    assert_eq!(names(".net"), vec![5, 6, 12]);
    // The usual extension filter is unchanged.
    assert_eq!(names(".pdf"), vec![11]);
    assert_eq!(names("تقرير .pdf"), vec![11]);
    assert!(names(".docx").is_empty());
    assert!(names(".bak .env").len() == 4); // either type
}

/// `ext:` and `*.` name an extension exactly: the file or folder called
/// `.gitignore` has it (as in Explorer and `dir *.gitignore`), `.gitignore`
/// does not have the extension `git`.
#[test]
fn explicit_extension_filters_and_dot_names() {
    assert_eq!(names("ext:gitignore"), vec![0]);
    assert_eq!(names("*.gitignore"), vec![0]);
    assert_eq!(names("*.ssh"), vec![4]);
    assert_eq!(names("ext:git"), vec![1]);
    assert_eq!(names("*.env"), vec![2, 8]);
    assert_eq!(names("ext:net"), vec![12]);
}

/// `ext:.pdf` is `ext:pdf`, and `*.*` is every type, not the extension `*`.
#[test]
fn extension_filter_spellings() {
    assert_eq!(at("ext:.pdf").exts, vec!["pdf"]);
    assert_eq!(at("EXT:.PDF,.docx;*.txt").exts, vec!["pdf", "docx", "txt"]);
    assert_eq!(names("ext:.pdf"), vec![11]);
    assert_eq!(names("ext:.env"), vec![2, 8]);
    // Every type: no filter at all.
    assert!(at("*.*").exts.is_empty() && at("*.*").is_empty());
    assert!(at("ext:*").is_empty() && at("ext:.").is_empty() && at("ext:*.").is_empty());
    assert_eq!(names("تقرير *.*"), names("تقرير"));
    assert_eq!(names("*.* framework"), vec![5, 6, 12]);
    // …and it ends a pending `|` like any filter.
    assert_eq!(at("a | *.* b").terms.len(), 2);
    assert!(at("*.* .pdf").warnings.is_empty());
}

/// A type picked in the list still limits a dot-name search to that type.
#[test]
fn dot_names_and_the_type_filter() {
    let with_kind = |q: &str, kind: usize| {
        let mut q = at(q);
        q.apply_kind(kind);
        let mut hits = search_names(&dot_entries(), &q, "");
        hits.sort_unstable();
        hits
    };
    assert_eq!(with_kind(".env", 0), vec![2, 3, 8]);
    assert!(with_kind(".env", 1).is_empty()); // documents
    assert!(with_kind(".net framework", 6).is_empty()); // programs: `net` is not one
    assert_eq!(with_kind(".ssh", KIND_FOLDERS), vec![4]);
    assert_eq!(with_kind(".pdf", 1), vec![11]);
}
