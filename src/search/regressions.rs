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
    Entry { path: path.into(), size: 1, mtime: 0, is_dir, norm_name: crate::normalize::name_key(name).into() }
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
    // A lone `*` is `*.*`, not the letter `*` (which no name has).
    for q in ["*", "**", "*.*", "ext:*"] {
        let q = at(q);
        assert!(q.is_empty() && q.any_name, "{q:?}");
    }
    assert_eq!(names("تقرير *"), names("تقرير"));
    assert_eq!(names("* .gitignore"), names(".gitignore"));
    assert!(!at("تقرير").any_name && !at("ext:pdf").any_name && !at("\"*\"").any_name);
    assert_eq!(at("\"*\"").terms[0].alts, ["*"]);
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
    let _tmp = crate::test_temp::TempPath::at(dir.clone());
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

// ------------------------------------------------ dm: and daylight saving

/// A zone one hour east of UTC in winter and two in summer (daylight saving
/// from 31 March to 27 October 2024, 01:00 UTC, as in central Europe).
fn cet(t: i64) -> i64 {
    let summer = days_from_civil(2024, 3, 31) * DAY + 3600..days_from_civil(2024, 10, 27) * DAY + 3600;
    if summer.contains(&t) { 7200 } else { 3600 }
}

fn day_utc(y: i64, m: u32, d: u32) -> i64 {
    days_from_civil(y, m, d) * DAY
}

/// `dm:` used the one offset read at start-up for every date: in summer
/// (+2), `dm:2024-01-15` started at 22:00 UTC instead of 23:00, so a file the
/// Date column shows on 15 January late in the evening was not found.
#[test]
fn dates_use_the_offset_of_their_own_day() {
    // NOW is 15 May (summer, +2).
    let q = |s: &str| Query::parse_zoned(s, NOW, &cet).mtime.unwrap();
    assert_eq!(q("dm:2024-01-15"), (day_utc(2024, 1, 15) - 3600, day_utc(2024, 1, 16) - 3600));
    let late_on_the_15th = day_utc(2024, 1, 15) + 22 * 3600 + 1800; // 23:30 local
    let (from, to) = q("dm:2024-01-15");
    assert!((from..to).contains(&late_on_the_15th));
    // Summer dates and today keep the summer offset.
    assert_eq!(q("dm:2024-05-12"), (day_utc(2024, 5, 12) - 7200, day_utc(2024, 5, 13) - 7200));
    assert_eq!(q("dm:today"), (day_utc(2024, 5, 15) - 7200, day_utc(2024, 5, 16) - 7200));
    // A range across the change: each end with its own offset; the change day is 23 hours long.
    assert_eq!(q("dm:2024-03"), (day_utc(2024, 3, 1) - 3600, day_utc(2024, 4, 1) - 7200));
    assert_eq!(q("dm:2024-03-31"), (day_utc(2024, 3, 31) - 3600, day_utc(2024, 4, 1) - 7200));
    let (from, to) = q("dm:2024-10-27");
    assert_eq!(to - from, 25 * 3600);
    // "Today" follows the clock at parse time: in January the winter offset.
    let winter_now = day_utc(2024, 1, 20) + 23 * 3600 + 1800; // 00:30 on the 21st, local
    let (from, _) = Query::parse_zoned("dm:today", winter_now, &cet).mtime.unwrap();
    assert_eq!(from, day_utc(2024, 1, 21) - 3600);
    // `1m` a month back from summer lands in winter: same local time of day.
    let (from, _) = Query::parse_zoned("dm:2m", NOW, &cet).mtime.unwrap();
    assert_eq!(from, day_utc(2024, 3, 15) + 12 * 3600 - 3600);
    // A fixed offset is unchanged.
    assert_eq!(at("dm:2024-01-15").mtime, Some((day_utc(2024, 1, 15), day_utc(2024, 1, 16))));
}

// ------------------------------------------------- Arabic filter prefixes

/// Arabic filter prefixes were compared byte for byte: `كلمه:` (ه for ة),
/// `تأريخ:` (with hamza) or `مُجلد:` became plain search words.
#[test]
fn arabic_prefixes_are_compared_normalised() {
    let words = |q: &Query| q.words.iter().map(|t| t.alts.clone()).collect::<Vec<_>>();
    assert_eq!(words(&at("كلمه:عقد")), vec![vec!["عقد".to_string()]]);
    assert_eq!(words(&at("كلمة:عقد")), words(&at("كلمه:عقد")));
    assert!(at("كلمه:عقد").terms.is_empty());
    assert_eq!(at("تأريخ:2024").mtime, at("dm:2024").mtime);
    assert_eq!(at("تـاريخ:اليوم").mtime, at("dm:today").mtime);
    assert_eq!(at("حَجم:>1م").size, at("size:>1mb").size);
    let q = at("مُجلد:صور");
    assert!(q.dirs_only);
    assert_eq!(q.terms[0].alts, vec!["صور"]);
    assert!(at("نِظام: x").show_system);
    assert!(at("تقريب:تقرير").fuzzy && at("جذر:كتب").root && at("فرانكو:3abd").franco);
    // The value starts right after the prefix, whatever its spelling.
    assert_eq!(at("كلمه:\"عقد ايجار\"").words[0].alts, vec!["عقد ايجار"]);
    assert_eq!(at(r#"تأريخ:"هذا الشهر" عقد"#).mtime, at("dm:thismonth").mtime);
    // Not a prefix: a different word, or the prefix inside a word.
    assert!(at("كلمات:عقد").words.is_empty());
    assert!(at("الكلمة:عقد").words.is_empty());
    assert!(at("كلم:عقد").words.is_empty());
    assert!(!at("صور:مجلد:").dirs_only);
    // Latin prefixes ignore case as before.
    assert_eq!(at("SIZE:>1mb").size, at("size:>1mb").size);
}

// ------------------------------------------ relative dates with comparisons

/// `dm:>7d` was refused (and searched as text) and `dm:<=7d` matched every
/// file. A period is the moment that long ago: `>`/`>=` after it, `<`/`<=`
/// before it, `..` between two.
#[test]
fn relative_dates_with_comparisons() {
    let week_ago = NOW - 7 * DAY;
    for s in ["dm:7d", "dm:>7d", "dm:>=7d", "dm:=7d", "تاريخ:>7d"] {
        assert_eq!(at(s).mtime, Some((week_ago, i64::MAX)), "{s}");
        assert!(at(s).warnings.is_empty() && at(s).terms.is_empty(), "{s}");
    }
    for s in ["dm:<7d", "dm:<=7d"] {
        assert_eq!(at(s).mtime, Some((i64::MIN, week_ago)), "{s}");
    }
    assert_eq!(at("dm:30d..7d").mtime, Some((NOW - 30 * DAY, week_ago)));
    assert_eq!(at("dm:7d..").mtime, Some((week_ago, i64::MAX)));
    assert_eq!(at("dm:..7d").mtime, Some((i64::MIN, week_ago)));
    assert_eq!(at("dm:2024-01..7d").mtime, Some((day_utc(2024, 1, 1), week_ago)));
    assert_eq!(at("dm:>1m").mtime, at("dm:1m").mtime);
    // The wrong way round is still refused.
    assert!(at("dm:7d..30d").mtime.is_none());
    // Calendar dates are unchanged.
    assert_eq!(at("dm:>2024-01-01").mtime, Some((day_utc(2024, 1, 2), i64::MAX)));
    assert_eq!(at("dm:<=2023").mtime, Some((i64::MIN, day_utc(2024, 1, 1))));
}

// --------------------------------------------------- folder scope and case

/// The scope check cut the path at the lower-cased scope's length in bytes;
/// lower case can change it (`İ` → `i̇`), so a folder such as
/// `C:\Users\İbrahim` never matched its own files.
#[test]
fn scope_with_letters_whose_lower_case_is_longer() {
    let scope = |s: &str| s.to_lowercase();
    let ibrahim = scope(r"C:\Users\İbrahim");
    assert!(in_scope(r"C:\Users\İbrahim\a.txt", &ibrahim));
    assert!(in_scope(r"C:\Users\İbrahim", &ibrahim));
    assert!(in_scope(r"c:\users\İBRAHIM\x\y.pdf", &ibrahim));
    assert!(in_scope(r"C:\Users\İbrahim\a.txt", &scope(r"C:\Users\İbrahim\")));
    assert!(!in_scope(r"C:\Users\İbrahim2\a.txt", &ibrahim));
    assert!(!in_scope(r"C:\Users\Ibrahim\a.txt", &ibrahim), "İ is not I");
    assert!(!in_scope(r"C:\Users\İb", &ibrahim));
    // Other letters whose case forms differ in length, and a final sigma.
    assert!(in_scope(r"D:\ẞTRASSE\a", &scope(r"D:\ẞtrasse")));
    assert!(in_scope(r"D:\Ⱥ\a", &scope(r"D:\ⱥ")));
    assert!(in_scope(r"D:\ΟΔΟΣ\a", &scope(r"D:\ΟΔΟΣ")));
    // Name search with such a scope.
    let entries = vec![entry(r"C:\Users\İbrahim\تقرير.pdf", false), entry(r"C:\Users\Other\تقرير.pdf", false)];
    assert_eq!(search_names(&entries, &at("تقرير"), r"C:\Users\İbrahim"), vec![0]);
    // Arabic and ASCII scopes as before.
    assert!(in_scope(r"C:\المستندات\a", &scope(r"C:\المستندات")));
    assert!(!in_scope(r"C:\Docs2\a.txt", r"c:\docs"));
}

// ------------------------------------- wrong keyboard layout and filters

/// The wrong-layout fallback kept only `ext:`, `*.` and `folder:` as they
/// were: `hgjrvdv size:>1mb` became `التقرير سهئث:>1ةلا` and found nothing.
#[test]
fn layout_fallback_keeps_every_filter() {
    use crate::layout::convert_layout;
    assert_eq!(convert_layout("hgjrvdv size:>1mb").as_deref(), Some("التقرير size:>1mb"));
    assert_eq!(convert_layout("hgjrvdv dm:today ww:x").as_deref(), Some("التقرير dm:today ww:x"));
    assert_eq!(convert_layout("hgjrvdv regex:^a sys: fuzzy: ext:pdf *.doc").as_deref(), Some("التقرير regex:^a sys: fuzzy: ext:pdf *.doc"));
    assert_eq!(convert_layout("hgjrvdv حجم:>1م تاريخ:اليوم").as_deref(), Some("التقرير حجم:>1م تاريخ:اليوم"));
    // The other way round: Arabic typed for Latin, filters kept.
    assert_eq!(convert_layout("اثممخ size:>1mb").as_deref(), Some("hello size:>1mb"));
    // A filter alone has nothing to convert; words are converted as before.
    assert_eq!(convert_layout("size:>1mb"), None);
    assert_eq!(convert_layout("hgjrvdv").as_deref(), Some("التقرير"));
    // And the converted query keeps its filter when parsed.
    let q = at(&convert_layout("hgjrvdv size:>1mb").unwrap());
    assert_eq!(q.size, at("size:>1mb").size);
    assert_eq!(q.terms[0].alts[0], "التقرير");
}

// ----------------------------------------- fuzzy: and root: inside documents

/// `fuzzy:` and `root:` changed nothing in a search inside documents and
/// said nothing: the words were looked for as typed. Now the search says so.
#[test]
fn fuzzy_and_root_say_they_do_not_apply_inside_documents() {
    for s in ["fuzzy: تقرير", "root:كتب", "تقريب:تقرير", "جذر: كتب"] {
        let mut q = at(s);
        assert!(q.warnings.is_empty(), "{s}");
        q.note_document_search();
        assert_eq!(q.warnings.len(), 1, "{s}");
        assert!(q.warnings[0].contains("fuzzy:") && q.warnings[0].contains("root:"), "{s}");
    }
    // The words are still searched as typed.
    let mut q = at("fuzzy: تقرير");
    q.note_document_search();
    assert!(match_text("هذا تقرير", &q).is_some());
    assert!(match_text("هذا تقريير", &q).is_none());
    // Nothing to say for other queries.
    let mut q = at("تقرير franco:");
    q.note_document_search();
    assert!(q.warnings.is_empty());
}

// ------------------------------------------------ hamza seats: مسئول / مسؤول

/// «مسئول» did not find «مسؤول» (nor the other way round): ئ became ي and ؤ
/// became و, so the two usual spellings of one word never met. Each seat now
/// also matches its other spellings, and a word typed without the hamza
/// (as many people type) still matches directly, both ways.
#[test]
fn hamza_seat_spellings_find_each_other() {
    let entries = vec![
        entry(r"C:\docs\مسؤول المبيعات.docx", false), // 0
        entry(r"C:\docs\مسئول الحسابات.xlsx", false), // 1
        entry(r"C:\docs\مسوول المخزن.txt", false),    // 2 without the hamza
        entry(r"C:\docs\شؤون الموظفين", true),        // 3
        entry(r"C:\docs\شئون الطلاب.pdf", false),     // 4
        entry(r"C:\docs\شوون عامة.txt", false),       // 5
        entry(r"C:\docs\رئيس القسم.txt", false),      // 6
        entry(r"C:\docs\رءيس.txt", false),            // 7
        entry(r"C:\docs\ريس.txt", false),             // 8
        entry(r"C:\docs\سؤال 1.txt", false),          // 9
        entry(r"C:\docs\سوال 2.txt", false),          // 10
        entry(r"C:\docs\مسائل.txt", false),           // 11
        entry(r"C:\docs\مسايل.txt", false),           // 12
        entry(r"C:\docs\بيئة العمل.txt", false),      // 13
        entry(r"C:\docs\بيية.txt", false),            // 14
        entry(r"C:\docs\مسيول.txt", false),           // 15 a bare ي is not a seat
        entry(r"C:\docs\سيال.txt", false),            // 16
    ];
    let find = |s: &str| {
        let mut r = search_names(&entries, &at(s), "");
        r.sort_unstable();
        r
    };
    assert_eq!(find("مسوول"), vec![0, 1, 2]);
    // With a seat typed, و and ي may differ there: مسئول without the hamza is مسيول.
    for q in ["مسؤول", "مسئول", "المسؤول", "مَسْؤُول"] {
        assert_eq!(find(q), vec![0, 1, 2, 15], "{q}");
    }
    for q in ["شؤون", "شئون", "شوون"] {
        assert_eq!(find(q), vec![3, 4, 5], "{q}");
    }
    for q in ["رئيس", "رءيس", "ريس"] {
        assert_eq!(find(q), vec![6, 7, 8], "{q}");
    }
    for q in ["سؤال", "سوال"] {
        assert_eq!(find(q), vec![9, 10], "{q}");
    }
    for q in ["مسائل", "مسايل"] {
        assert_eq!(find(q), vec![11, 12], "{q}");
    }
    for q in ["بيئة", "بيية", "بيئه"] {
        assert_eq!(find(q), vec![13, 14], "{q}");
    }
    // و and ي stay different letters where no hamza is involved.
    assert_eq!(find("مسيول"), vec![1, 15]); // مسئول is مسيول without the hamza
    assert_eq!(find("سيال"), vec![16]);
    // Inside documents, a query with a seat finds the other spellings.
    let text = "اجتماع مع مسؤول المبيعات غداً";
    assert_eq!(match_text(text, &at("مسئول")).as_deref(), Some(text));
    assert!(match_text("شئون الطلاب", &at("شؤون")).is_some());
    assert!(match_text("مسوول المخزن", &at("مسئول")).is_some());
    // Excluding a spelling excludes the names spelled another way with the
    // seat (their keys hold it). An exclusion has no lossy spellings, so the
    // names typed without the hamza stay (مسوول, شوون, ريس) unless typed too.
    assert_eq!(find("txt -مسئول -رئيس -سؤال -مسائل -بيئة -شئون -سيال -مسيول"), vec![2, 5, 8]);
    assert_eq!(find("txt -مسوول -ريس -سوال -مسايل -بيية -شوون -سيال -مسيول"), Vec::<u32>::new());
}

/// The name spelled as typed comes before the other spellings when they
/// match equally well (whole name, then start of the name, as before).
#[test]
fn hamza_seat_spelling_as_typed_ranks_first() {
    let entries = vec![
        entry(r"C:\a\مسئول.txt", false),   // 0
        entry(r"C:\a\مسؤول.txt", false),   // 1
        entry(r"C:\a\مسئولية.txt", false), // 2
        entry(r"C:\a\مسؤولية.txt", false), // 3
        entry(r"C:\a\مسوول.txt", false),   // 4
    ];
    let find = |s: &str| search_names(&entries, &at(s), "");
    // As typed first, then the other spellings, shorter names first.
    assert_eq!(find("مسؤول"), vec![1, 3, 0, 4, 2]);
    // «مسؤولية» holds «مسوول» only inside a longer word, and that spelling
    // of «مسئول» is lossy: it matches whole words only (see Term::word_alts).
    assert_eq!(find("مسئول"), vec![0, 2, 1, 4]);
    assert_eq!(find("مسوول"), vec![4, 0, 1, 2, 3]);
    // A whole-name match still comes first, whatever its spelling.
    assert_eq!(find("مسئول.txt"), vec![0, 1, 4]);
    assert_eq!(find("مسوول.txt"), vec![4, 0, 1]);
    // Diacritics and the article do not stop a name counting as typed.
    assert_eq!(find("المسؤول"), vec![1, 3, 0, 4, 2]);
    assert_eq!(find("مَسْئُول"), vec![0, 2, 1, 4]);
    // رئيس / ريس.
    let entries = vec![entry(r"C:\a\رئيس.txt", false), entry(r"C:\a\ريس.txt", false)];
    assert_eq!(search_names(&entries, &at("ريس"), ""), vec![1, 0]);
    assert_eq!(search_names(&entries, &at("رئيس"), ""), vec![0, 1]);
}

/// The underline covers the word in the name whichever spelling found it.
#[test]
fn hamza_seat_matches_are_underlined() {
    let name = "مسئول ورئيس.txt";
    let marked = |q: &str| match_ranges(name, &at(q)).into_iter().map(|(s, e)| &name[s..e]).collect::<Vec<_>>();
    assert_eq!(marked("مسوول"), ["مسئول"]);
    assert_eq!(marked("مسؤول"), ["مسئول"]);
    assert_eq!(marked("ريس"), ["رئيس"]);
    assert_eq!(marked("رءيس"), ["رئيس"]);
}

/// The other spellings of a name serve `ww:` and proximity searches too,
/// one spelling at a time, and names with a seat keep their place when no
/// term ranks them (filters only, Arabizi).
#[test]
fn hamza_seats_in_filters_and_order() {
    let entries = vec![
        entry(r"C:\a\تقرير مسؤول.txt", false),         // 0
        entry(r"C:\a\مسوول المخزن.txt", false),        // 1
        entry(r"C:\a\مسؤولية.txt", false),             // 2
        entry(r"C:\a\مسئول a b c d رييس", true),      // 3
        entry(r"C:\a\abcdefghijklmnop.txt", false),    // 4
    ];
    let find = |s: &str| {
        let mut r = search_names(&entries, &at(s), "");
        r.sort_unstable();
        r
    };
    for q in ["ww:مسئول", "ww:مسؤول", "ww:مسوول"] {
        assert_eq!(find(q), vec![0, 1, 3], "{q}");
    }
    assert_eq!(find(r#""المخزن مسئول"~1"#), vec![1]);
    // The last word of one spelling is not next to the first of the next.
    assert_eq!(find(r#""رييس مسوول"~1"#), Vec::<u32>::new());
    assert_eq!(find(r#""رييس مسوول"~6"#), vec![3]);
    // Shorter names first, as before, whether or not they have a seat.
    assert_eq!(search_names(&entries, &at("ext:txt"), ""), vec![2, 0, 1, 4]);
    let marked = |name: &str, q: &str| match_ranges(name, &at(q)).into_iter().map(|(s, e)| name[s..e].to_string()).collect::<Vec<_>>();
    assert_eq!(marked("مسئول.txt", "ww:مسوول"), ["مسئول"]);
    assert_eq!(marked("مسئول.txt", r#""مسؤول txt"~2"#), ["مسئول", "txt"]);
}

// ------------------------------------------- word-final hamza: شاطئ / شاطيء

/// «شاطيء» did not find «شاطئ»: a final ء after ي was kept, so only the
/// other way round («شاطئ» is شاطي, a part of شاطيء) matched. A final ئ is
/// now also يء, in names and in queries, and inside documents a final يء
/// typed in the query also finds ئ (at the end of a word only: «شيء» must
/// not find «شيخ»). The spelling as typed still comes first.
#[test]
fn word_final_hamza_spellings_find_each_other() {
    let entries = vec![
        entry(r"C:\a\شاطئ.txt", false),       // 0
        entry(r"C:\a\شاطيء.txt", false),      // 1
        entry(r"C:\a\مبادئ عامة.docx", false), // 2
        entry(r"C:\a\مباديء.docx", false),    // 3
        entry(r"C:\a\شئ.txt", false),         // 4
        entry(r"C:\a\شيء.txt", false),        // 5
        entry(r"C:\a\شيخ.txt", false),        // 6
        entry(r"C:\a\قراءة.txt", false),      // 7
        entry(r"C:\a\قرائة.txt", false),      // 8
    ];
    let find = |s: &str| {
        let mut r = search_names(&entries, &at(s), "");
        r.sort_unstable();
        r
    };
    for q in ["شاطئ", "شاطيء", "شاطئ.txt", "شاطيء.txt"] {
        assert_eq!(find(q), vec![0, 1], "{q}");
    }
    for q in ["مبادئ", "مباديء"] {
        assert_eq!(find(q), vec![2, 3], "{q}");
    }
    for q in ["شيء", "شيء.txt"] {
        assert_eq!(find(q), vec![4, 5], "{q}");
    }
    // «شئ» is «شي» once normalised, a part of «شيخ» too (as before).
    assert_eq!(find("شئ"), vec![4, 5, 6]);
    for q in ["قراءة", "قرائة"] {
        assert_eq!(find(q), vec![7, 8], "{q}");
    }
    // The spelling as typed first.
    assert_eq!(search_names(&entries, &at("شاطيء"), ""), vec![1, 0]);
    assert_eq!(search_names(&entries, &at("شاطئ"), ""), vec![0, 1]);
    assert_eq!(search_names(&entries, &at("شيء"), ""), vec![5, 4]);
    // Inside documents, both ways.
    for (text, q) in [("على الشاطئ صباحاً", "شاطيء"), ("على الشاطيء صباحاً", "شاطئ"), ("لا شئ هنا", "شيء"), ("لا شيء هنا", "شئ")] {
        assert!(match_text(text, &at(q)).is_some(), "{q} in {text}");
    }
    assert!(match_text("هذه مبادئ.", &at("مباديء")).is_some());
    assert!(match_text("قراءة الكتاب", &at("قرائة")).is_some());
    assert!(match_text("قرائة الكتاب", &at("قراءة")).is_some());
    assert!(match_text("رأي الشيخ", &at("شيء")).is_none());
    assert_eq!(all_matches("شئ ثم شيء ثم شيخ", &at("شيء"), 5).0, 2);
    // The underline covers the name whichever spelling found it.
    let marked = |name: &str, q: &str| match_ranges(name, &at(q)).into_iter().map(|(s, e)| name[s..e].to_string()).collect::<Vec<_>>();
    assert_eq!(marked("شاطئ.txt", "شاطيء"), ["شاطئ"]);
    assert_eq!(marked("لا شئ هنا", "شيء"), ["شئ"]);
}

// ------------------------------------- a query without the hamza, in documents

/// A query typed without the hamza («مسوول», «ريس», «شوون») found «مسئول»,
/// «رئيس» and «شئون» in file names (their keys hold those spellings) but not
/// inside documents, whose text is kept as it is. A search inside documents
/// now adds those spellings to the query.
#[test]
fn query_without_hamza_finds_the_seat_in_documents() {
    let text = "اجتماع مسئول الحسابات مع رئيس القسم حول شئون الموظفين وسؤال المدير ومسؤول المخزن";
    for (q, word) in [
        ("مسوول", "مسئول"),
        ("المسوول الحسابات", "مسئول"),
        ("ريس", "رئيس"),
        ("الريس", "رئيس"),
        ("شوون", "شئون"),
        ("سوال", "وسؤال"),
        ("مسيول", "مسئول"),
    ] {
        let s = match_text(text, &at(q)).unwrap_or_else(|| panic!("{q}"));
        assert!(s.contains(word), "{q}: {s}");
    }
    // Every occurrence counts, whatever its spelling.
    assert_eq!(all_matches(text, &at("مسوول"), 50).0, 2);
    assert_eq!(all_matches("رئيس ورييس وريس", &at("ريس"), 50).0, 3);
    // ww:, proximity and exclusions too.
    assert!(match_text(text, &at("ww:ريس")).is_some());
    assert!(match_text(text, &at(r#""ريس القسم"~1"#)).is_some());
    // An exclusion is matched as typed only: «ريس» is not in «رئيس».
    assert!(match_text(text, &at("القسم -ريس")).is_some());
    assert!(match_text(text, &at("القسم -رئيس")).is_none());
    // و and ي stay different letters where no hamza can be.
    for w in ["سيارة", "بيوت", "تقرير", "كبير", "مدير", "سؤال", "report"] {
        assert!(crate::normalize::document_forms(&normalize(w)).is_empty(), "{w}");
    }
    assert!(match_text("سيارة جديدة", &at("سوارة")).is_none());
    // The same lines are listed when a result is opened.
    let hit = content_hit(0, text, &at("ريس").for_documents()).unwrap();
    assert_eq!(hit.count, 1);
    // A search over documents uses them.
    let (hits, _) = search_many(3, text, "مسوول", HitLimits::DEFAULT);
    assert_eq!(hits.len(), 3);
    assert!(hits.iter().all(|h| h.count == 2));
}

/// The document spellings stay few whatever the query: each change on its
/// own, then all of them, never one per combination.
#[test]
fn document_forms_are_capped() {
    let q = at("مسوولين شوونهم ريس وو شيء").for_documents();
    for t in &q.terms {
        assert!(t.alts.len() + t.word_alts.len() <= 2 + 1 + MAX_TERM_DOCUMENT_FORMS, "{t:?}");
    }
    let many = "مسوول ".repeat(40);
    assert!(crate::normalize::document_forms(&many).len() <= crate::normalize::MAX_DOCUMENT_FORMS);
    // Names are not searched with them: name search stays as it was.
    assert_eq!(at("ريس").terms[0].alts, ["ريس"]);
    assert!(at("ريس").terms[0].word_alts.is_empty());
}

// ------------------------------- lossy hamza spellings match whole words only

/// «رئيس» also searched for «ريس» (its seat dropped) as a plain substring, so
/// it found «إدريس», «باريس» and «تدريس»; «نهائي» ranked «نهاية» first (it
/// starts with «نهاي»), «جزئي» found «جزيرة», «-رئيس» hid every name with
/// «ريس» in it, and inside documents «نهائي» found every «نهاية». The other
/// spellings of a seat now match only as whole words (after an optional
/// article), and exclusions use the word as typed only.
#[test]
fn lossy_hamza_spellings_match_whole_words_only() {
    let entries = vec![
        entry(r"C:\a\رئيس القسم.txt", false),         // 0
        entry(r"C:\a\إدريس.txt", false),              // 1
        entry(r"C:\a\باريس 2024.jpg", false),         // 2
        entry(r"C:\a\تدريس الرياضيات.docx", false),   // 3
        entry(r"C:\a\ريس.txt", false),                // 4
        entry(r"C:\a\الريس الجديد.txt", false),       // 5
        entry(r"C:\a\تقرير نهائي.pdf", false),        // 6
        entry(r"C:\a\نهاية العام.docx", false),       // 7
        entry(r"C:\a\جزيرة.png", false),              // 8
        entry(r"C:\a\تقرير جزئي.pdf", false),         // 9
        entry(r"C:\a\ريسيتال.txt", false),            // 10
    ];
    let ranked = |s: &str| search_names(&entries, &at(s), "");
    let find = |s: &str| {
        let mut r = ranked(s);
        r.sort_unstable();
        r
    };
    for q in ["رئيس", "الرئيس", "رَئِيس", "\"رئيس\"", "ww:رئيس", "رءيس"] {
        assert_eq!(find(q), vec![0, 4, 5], "{q}");
    }
    // The spelling as typed first.
    assert_eq!(ranked("رئيس")[0], 0);
    assert_eq!(find("نهائي"), vec![6]);
    assert_eq!(find("نهائي.pdf"), vec![6]);
    assert_eq!(find("جزئي"), vec![9]);
    // Excluding «رئيس» does not hide «ريس» inside other words.
    assert_eq!(find("txt -رئيس"), vec![1, 4, 5, 10]);
    assert_eq!(find("jpg -رئيس"), vec![2]);
    assert_eq!(find("docx -رئيس"), vec![3, 7]);
    // Proximity words too.
    assert_eq!(find(r#""رئيس القسم"~1"#), vec![0]);
    assert_eq!(find(r#""رئيس 2024"~1"#), Vec::<u32>::new());
    // A phrase: its other spelling as whole words too (an article may come first).
    assert_eq!(find("\"رئيس الجديد\""), vec![5]);
    assert_eq!(find("\"الرئيس الجديد\""), vec![5]);
    assert_eq!(find("\"يس الجديد\""), vec![5]);
    assert_eq!(find("\"رئيس 2024\""), Vec::<u32>::new());
    // The query as typed still matches anywhere (as before).
    assert_eq!(find("ريس"), vec![0, 1, 2, 3, 4, 5, 10]);
    // Underlined: the word only.
    let marked = |name: &str, q: &str| match_ranges(name, &at(q)).into_iter().map(|(s, e)| name[s..e].to_string()).collect::<Vec<_>>();
    assert_eq!(marked("باريس وريس.txt", "رئيس"), ["ريس"]);
    assert!(marked("إدريس.txt", "رئيس").is_empty());

    // Inside documents.
    let text = "سافر إدريس إلى باريس لتدريس الرياضيات في نهاية العام وزار جزيرة";
    for q in ["رئيس", "نهائي", "جزئي", "\"رئيس\"", "ww:نهائي", r#""نهائي العام"~1"#] {
        assert!(match_text(text, &at(q)).is_none(), "{q}");
    }
    assert!(match_text(text, &at("سافر -رئيس")).is_some());
    assert!(match_text(text, &at("سافر -نهائي")).is_some());
    assert_eq!(all_matches("نهاية ونهائي والنهائي ونهايات", &at("نهائي"), 9).0, 2);
    assert!(match_text("التقرير النهائي للعام", &at("نهائي")).is_some());
    assert!(match_text("قال الريس كلمته", &at("رئيس")).is_some());
    assert!(match_text("حضر الرئيس", &at("ريس")).is_some());
    assert!(match_text("في نهاية العام", &at("نهاي")).is_some()); // as typed: anywhere
}

/// The owner's list: each pair is one word, typed either way, as the query
/// and as the file name (also with the article and with tashkeel), and inside
/// documents (where the word is matched whole).
#[test]
fn hamza_spellings_owner_list() {
    let groups: &[&[&str]] = &[
        &["رئيس", "ريس"],
        &["مسؤول", "مسئول", "مسوول"],
        &["شؤون", "شئون", "شوون"],
        &["سؤال", "سوال"],
        &["مسائل", "مسايل"],
        &["شاطئ", "شاطيء"],
        &["شيء", "شئ"],
    ];
    // A fatha after the first letter.
    let voweled = |w: &str| {
        let mut c = w.chars();
        let first = c.next().unwrap();
        format!("{first}\u{064E}{}", c.as_str())
    };
    for group in groups {
        for &name in group.iter() {
            for name in [name.to_string(), format!("ال{name}"), voweled(name)] {
                let entries = vec![entry(&format!(r"C:\a\{name}.txt"), false), entry(r"C:\a\تقرير.txt", false)];
                for &q in group.iter() {
                    for q in [q.to_string(), format!("ال{q}"), voweled(q)] {
                        if q.starts_with("ال") && !name.starts_with("ال") {
                            continue; // «الرئيس» needs the article in the name
                        }
                        assert_eq!(search_names(&entries, &at(&q), ""), vec![0], "{q} → {name}");
                        for doc in [format!("حضر {name} اليوم"), format!("حضر و{name}.")] {
                            if q.starts_with("ال") && !doc.contains(" ال") {
                                continue;
                            }
                            assert!(match_text(&doc, &at(&q)).is_some(), "{q} in «{doc}»");
                        }
                    }
                }
            }
        }
    }
}
