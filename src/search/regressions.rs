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

// ------------------------------- the selection when document rows are sorted

/// When a search inside documents ends its rows are put in order. The
/// selection must go with its file: left on its row number, Enter or Delete
/// acted on whichever file took that row.
#[test]
fn sorting_document_rows_keeps_the_selected_file() {
    let entries = dot_entries();
    // Rows in the order the search found them, each with its own payload.
    let found = || vec![(5u32, "e"), (9, "j"), (0, "a"), (12, "m")];
    let path = |rows: &[(u32, &str)], i: usize| entries[rows[i].0 as usize].path.clone();
    for key in [SortKey::Relevance, SortKey::Name, SortKey::Path, SortKey::Size, SortKey::Date] {
        for desc in [false, true] {
            for selected in 0..4 {
                let mut rows = found();
                let before = path(&rows, selected);
                let now = sort_rows(&entries, &mut rows, |r| r.0, key, desc, Some(selected)).unwrap();
                assert_eq!(path(&rows, now), before, "{key:?} {desc} {selected}");
                // Every row is still there, with its own payload.
                let mut all = rows.clone();
                all.sort_unstable();
                assert_eq!(all, vec![(0, "a"), (5, "e"), (9, "j"), (12, "m")]);
            }
        }
    }
    // Path order (best match for documents): the row found second is last but one…
    let mut rows = found();
    assert_eq!(sort_rows(&entries, &mut rows, |r| r.0, SortKey::Relevance, false, Some(1)), Some(2));
    assert_eq!(rows, vec![(0, "a"), (5, "e"), (9, "j"), (12, "m")]);
    // …and its old row number now shows another file (what Enter used to open).
    assert_ne!(rows[1], found()[1]);
    // Nothing selected, or a row that is not there: nothing selected after.
    assert_eq!(sort_rows(&entries, &mut found(), |r| r.0, SortKey::Name, false, None), None);
    assert_eq!(sort_rows(&entries, &mut found(), |r| r.0, SortKey::Name, false, Some(9)), None);
    assert_eq!(sort_rows(&entries, &mut Vec::<(u32, &str)>::new(), |r| r.0, SortKey::Name, true, Some(0)), None);
}

// ------------------------------------- memory kept by a search inside documents

/// Runs [`search_documents`] over `n` in-memory documents, all with `text`.
fn search_many(n: usize, text: &str, q: &str, limits: HitLimits) -> (Vec<ContentHit>, ContentProgress) {
    let entries: Vec<Entry> = (0..n).map(|i| Entry { size: 10, ..entry(&format!(r"C:\d\doc {i}.txt"), false) }).collect();
    let progress = ContentProgress::default();
    let hits = Mutex::new(Vec::new());
    let text_of = |_: &Entry| (Some(text.to_string()), false);
    let mut q = at(q);
    q.show_system = true;
    search_documents(&entries, &q, "", &progress, &hits, limits, &text_of);
    (hits.into_inner().unwrap(), progress)
}

/// A common word («في», `a`) found in tens of thousands of documents kept 50
/// lines of up to 200 characters for every one of them: gigabytes, more than
/// the 32-bit program can have. Now only the first [`LINED_HITS`] results keep
/// lines, and at most [`MAX_CONTENT_HITS`] are listed.
#[test]
fn document_search_memory_is_bounded() {
    // 50,000 small documents (12 short lines each), all containing the word.
    let small: String = (1..=12).map(|i| format!("السطر {i} فيه كلمة في الملف\n")).collect();
    let (hits, progress) = search_many(50_000, &small, "في", HitLimits::DEFAULT);
    let total: usize = hits.iter().map(ContentHit::retained_bytes).sum();
    println!("50,000 small documents: {} hits keep {} bytes", hits.len(), total);
    assert_eq!(hits.len(), 50_000);
    assert!(!progress.capped.load(Ordering::Relaxed));
    assert_eq!(hits.iter().filter(|h| !h.lines.is_empty()).count(), LINED_HITS);
    assert!(hits.iter().all(|h| h.count == 24 && !h.snippet.is_empty()));
    // Before: 70,150,000 bytes (1,403 per hit). Now about 12 MB.
    assert!(total < 15 << 20, "{total} bytes");

    // The worst case per hit: 50 long lines. Before, 34 KB for every result.
    let long: String = (1..=80).map(|i| format!("{} في {} {i}\n", "كلمة ".repeat(30), "نص ".repeat(40))).collect();
    let (hits, _) = search_many(40, &long, "في", HitLimits { lined: 10, hits: 1_000 });
    let lined = hits.iter().filter(|h| !h.lines.is_empty()).map(ContentHit::retained_bytes).max().unwrap();
    let bare = hits.iter().filter(|h| h.lines.is_empty()).map(ContentHit::retained_bytes).max().unwrap();
    println!("one long document: {lined} bytes with its lines, {bare} without");
    assert!(lined > 30_000 && bare < 1_000, "{lined} {bare}");
    // So a whole search keeps at most (LINED_HITS × 40 KB) + (MAX_CONTENT_HITS × 1 KB), about 135 MB,
    // where 50,000 such documents used to keep 1.6 GB.
    const { assert!(LINED_HITS * 40_000 + MAX_CONTENT_HITS * 1_000 < 160 << 20) };
}

/// The search stops at the result limit and says so; the lines of a result
/// listed without them are read again, the same as they would have been.
#[test]
fn document_search_caps_results_and_reads_lines_again() {
    let text = "عقد الإيجار\nبند ثانٍ\nنهاية العقد";
    let (hits, progress) = search_many(500, text, "عقد", HitLimits { lined: 3, hits: 10 });
    assert_eq!(hits.len(), 10);
    assert!(progress.capped.load(Ordering::Relaxed));
    assert_eq!(hits.iter().filter(|h| !h.lines.is_empty()).count(), 3);
    assert!(hits.iter().all(|h| h.count == 2));
    let (hits, progress) = search_many(10, text, "عقد", HitLimits { lined: 3, hits: 10 });
    assert_eq!(hits.len(), 10);
    assert!(!progress.capped.load(Ordering::Relaxed), "exactly the limit is not more");

    // What "Show all matches" gets for a result without its lines.
    let dir = std::env::temp_dir().join(format!("bahth_rereads_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("عقد.txt");
    std::fs::write(&p, text).unwrap();
    let path = p.to_str().unwrap();
    let full = content_hit(0, text, &at("عقد")).unwrap();
    assert_eq!(document_matches(path, &at("عقد"), None), Some((full.count, full.lines)));
    assert_eq!(document_matches(path, &at("مفقود"), None), None);
    std::fs::remove_file(&p).unwrap();
    assert_eq!(document_matches(path, &at("عقد"), None), None, "gone since");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `Near::hits` kept every word of the text that matched (a one-letter word
/// in a big document: millions of entries); counting stops at MAX_SPANS like
/// every other part of a query, and finding the first window keeps only the
/// words within the distance.
#[test]
fn proximity_search_memory_is_bounded() {
    let text = "a ".repeat(MAX_SPANS + 10);
    let near = &at(r#""a zzz"~5"#).near[0];
    let (found, most) = near.find_window(&text);
    assert_eq!(found, None);
    assert!(most <= 6, "{most}");
    assert!(near.all(&text).is_empty());
    let both = format!("{text} zzz");
    assert_eq!(near.find_window(&both).0, Some(2 * (MAX_SPANS + 10 - 5)));
    let near = &at(r#""a a"~2"#).near[0];
    assert_eq!(near.all(&text).len(), MAX_SPANS);
    assert_eq!(all_matches(&text, &at(r#""a a"~2"#), 1).0, MAX_SPANS as u32);
}
