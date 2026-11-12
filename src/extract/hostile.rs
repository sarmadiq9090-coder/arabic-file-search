//! Files that used to END THE PROCESS (stack overflow, failed allocation),
//! which no `catch_unwind` can stop: each is extracted in a child process,
//! so a regression fails one test instead of killing the whole test run.
//!
//! The child is this same test executable, started again with only
//! [`child_entry`] selected and the file to read in `BAHTH_TEST_EXTRACT`. It
//! extracts on a thread with the stack of the app's worker threads (2 MB,
//! the Rust default the warm-up and the search pool use), under a counting
//! allocator that refuses to go past `BAHTH_TEST_MEM_CAP` bytes (so a memory
//! bomb aborts the child the way a real out-of-memory would, without taking
//! the machine with it), and prints what came out and the peak heap use.

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use super::*;

// ---------------------------------------------------------------------------
// Counting allocator (test builds only)
// ---------------------------------------------------------------------------

/// The system allocator, counting live and peak bytes, with an optional cap.
/// Not the global allocator itself: `crate::test_alloc::Counting` (the one
/// test builds install, in `main.rs`) passes every request through this.
pub(crate) struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static CAP: AtomicUsize = AtomicUsize::new(usize::MAX);

/// Accounts for `add` more bytes; false (nothing counted) past the cap.
fn take(add: usize) -> bool {
    let live = LIVE.fetch_add(add, Ordering::Relaxed).saturating_add(add);
    if live > CAP.load(Ordering::Relaxed) {
        LIVE.fetch_sub(add, Ordering::Relaxed);
        return false;
    }
    PEAK.fetch_max(live, Ordering::Relaxed);
    true
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if !take(l.size()) {
            return std::ptr::null_mut();
        }
        let p = unsafe { System.alloc(l) };
        if p.is_null() {
            LIVE.fetch_sub(l.size(), Ordering::Relaxed);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        if !take(l.size()) {
            return std::ptr::null_mut();
        }
        let p = unsafe { System.alloc_zeroed(l) };
        if p.is_null() {
            LIVE.fetch_sub(l.size(), Ordering::Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let grow = new.saturating_sub(l.size());
        if grow > 0 && !take(grow) {
            return std::ptr::null_mut();
        }
        let q = unsafe { System.realloc(p, l, new) };
        if q.is_null() {
            LIVE.fetch_sub(grow, Ordering::Relaxed);
        } else if new < l.size() {
            LIVE.fetch_sub(l.size() - new, Ordering::Relaxed);
        }
        q
    }
}

/// Bytes allocated right now, and the most there ever were.
pub(crate) fn heap() -> (usize, usize) {
    (LIVE.load(Ordering::Relaxed), PEAK.load(Ordering::Relaxed))
}

// ---------------------------------------------------------------------------
// The child process
// ---------------------------------------------------------------------------

/// What a child extraction did.
#[derive(Debug)]
pub(crate) struct Child {
    /// It ended by itself with exit code 0.
    pub ok: bool,
    /// How it ended (`exit status: 0`, `signal: 6 (SIGABRT)`, `timed out`…).
    pub status: String,
    /// Length of the text, `Some(None)` for "no text", `None` when it died first.
    pub text: Option<Option<usize>>,
    /// Peak heap growth during the extraction, in bytes.
    pub peak: Option<usize>,
    pub elapsed: Duration,
    pub stderr: String,
    /// With a store: how many files the last run died in (`recovered`).
    pub recovered: Option<usize>,
}

const MB: usize = 1024 * 1024;

/// Extracts `path` in a child process whose heap may grow to `mem_cap`
/// bytes, killing it after `timeout`. `env` adds variables for the child.
pub(crate) fn extract_in_child(path: &Path, mem_cap: usize, timeout: Duration, env: &[(&str, &str)]) -> Child {
    let exe = std::env::current_exe().unwrap();
    let mut cmd = Command::new(exe);
    cmd.args(["--exact", "extract::hostile::child_entry", "--nocapture", "--test-threads", "1"])
        .env("BAHTH_TEST_EXTRACT", path)
        .env("BAHTH_TEST_MEM_CAP", mem_cap.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let started = Instant::now();
    let mut child = cmd.spawn().unwrap();
    // The output is a few lines: reading it after the wait cannot block the child.
    let status = loop {
        match child.try_wait().unwrap() {
            Some(s) => break Some(s),
            None if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    let elapsed = started.elapsed();
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let field = |key: &str| stdout.lines().find_map(|l| l.strip_prefix(key).map(|v| v.trim().to_owned()));
    Child {
        ok: status.is_some_and(|s| s.success()),
        status: status.map_or("timed out".into(), |s| s.to_string()),
        text: field("CHILD-TEXT").map(|v| v.parse().ok()),
        peak: field("CHILD-PEAK").and_then(|v| v.parse().ok()),
        elapsed,
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        recovered: field("CHILD-RECOVERED").and_then(|v| v.parse().ok()),
    }
}

/// The child's side: does nothing in an ordinary test run.
#[test]
fn child_entry() {
    let Some(path) = std::env::var_os("BAHTH_TEST_EXTRACT") else { return };
    let cap: usize = std::env::var("BAHTH_TEST_MEM_CAP").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    let path = PathBuf::from(path);
    let worker = std::thread::Builder::new()
        .name("content-test".into())
        .stack_size(2 * MB)
        .spawn(move || {
            let (base, _) = heap();
            PEAK.store(base, Ordering::Relaxed);
            CAP.store(base.saturating_add(cap), Ordering::Relaxed);
            // With a store, the way the warm-up and the content search read.
            let text = match std::env::var_os("BAHTH_TEST_CACHE_DIR") {
                Some(dir) => {
                    let cache = crate::content_cache::ContentCache::open(Path::new(&dir)).unwrap();
                    println!("\nCHILD-RECOVERED {}", cache.recovered().len());
                    crate::content_cache::text_of(Some(&cache), path.to_str().unwrap()).0
                }
                None => extract_text(&path),
            };
            CAP.store(usize::MAX, Ordering::Relaxed);
            (text.map(|t| t.len()), heap().1.saturating_sub(base))
        })
        .unwrap();
    let (text, peak) = worker.join().unwrap();
    let mut out = std::io::stdout().lock();
    writeln!(out, "\nCHILD-TEXT {}", text.map_or("none".into(), |n| n.to_string())).unwrap();
    writeln!(out, "CHILD-PEAK {peak}").unwrap();
}

fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bahth_hostile_tests_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

/// Runs the child on `bytes` saved as `name` with the default limits: 512 MB
/// of heap, 60 seconds.
fn child(name: &str, bytes: &[u8], env: &[(&str, &str)]) -> Child {
    let c = extract_in_child(&temp_file(name, bytes), 512 * MB, Duration::from_secs(60), env);
    eprintln!(
        "{name}{env:?} ({} bytes): {}, text {:?}, peak {:?} KB, {} ms",
        bytes.len(),
        c.status,
        c.text,
        c.peak.map(|p| p / 1024),
        c.elapsed.as_millis()
    );
    c
}

#[test]
fn child_harness_reports_text() {
    let c = child("plain.txt", "نص عادي".as_bytes(), &[]);
    assert!(c.ok, "{c:?}");
    assert_eq!(c.text, Some(Some("نص عادي".len())));
    assert!(c.peak.is_some_and(|p| p < 16 * MB), "{c:?}");
}

// ---------------------------------------------------------------------------
// PDF: recursion without end
// ---------------------------------------------------------------------------

/// A PDF made of `objects` (object N is `objects[N - 1]`), with a correct
/// cross-reference table. Object 1 is the catalog.
pub(crate) fn pdf_of(objects: &[String]) -> Vec<u8> {
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
    }
    let xref = pdf.len();
    pdf.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
    for off in offsets {
        pdf.extend(format!("{off:010} 00000 n \n").bytes());
    }
    pdf.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).bytes());
    pdf
}

/// A stream object with `dict` entries and `content`.
pub(crate) fn pdf_stream(dict: &str, content: &str) -> String {
    format!("<< {dict} /Length {} >>\nstream\n{content}\nendstream", content.len())
}

/// Turns the PDF check off in the child (see `extract::unchecked`).
const UNCHECKED_PDF: (&str, &str) = ("BAHTH_TEST_UNCHECKED_PDF", "1");
/// Turns the spreadsheet checks off in the child.
const UNCHECKED_SHEETS: (&str, &str) = ("BAHTH_TEST_UNCHECKED_SHEETS", "1");

const CATALOG: &str = "<< /Type /Catalog /Pages 2 0 R >>";
const PAGES: &str = "<< /Type /Pages /Kids [3 0 R] /Count 1 >>";
const FONT: &str = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>";
const FORM: &str = "/Type /XObject /Subtype /Form /BBox [0 0 612 792]";

/// A one-page PDF: the page has `resources` and `content` (object 4);
/// `more` become objects 6 and up (object 5 is the font /F1).
pub(crate) fn one_page_pdf(resources: &str, content: &str, more: &[String]) -> Vec<u8> {
    let mut objects = vec![
        CATALOG.to_owned(),
        PAGES.to_owned(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << {resources} >> >>"),
        pdf_stream("", content),
        FONT.to_owned(),
    ];
    objects.extend(more.iter().cloned());
    pdf_of(&objects)
}

/// The reviewer's file: a form whose content is `/X Do`, with no resources
/// of its own, so it finds itself again in the page's `/XObject <</X 6 0 R>>`.
pub(crate) fn pdf_form_calling_itself() -> Vec<u8> {
    one_page_pdf("/XObject << /X 6 0 R >>", "/X Do", &[pdf_stream(FORM, "/X Do")])
}

/// Two forms calling each other through their own resources.
fn pdf_two_forms_in_a_loop() -> Vec<u8> {
    one_page_pdf(
        "/XObject << /A 6 0 R >>",
        "/A Do",
        &[
            pdf_stream(&format!("{FORM} /Resources << /XObject << /B 7 0 R >> >>"), "/B Do"),
            pdf_stream(&format!("{FORM} /Resources << /XObject << /A 6 0 R >> >>"), "/A Do"),
        ],
    )
}

/// `depth` forms, each drawing the next: no loop, only a very deep chain.
/// The innermost one shows `text`.
pub(crate) fn pdf_form_chain(depth: usize, text: &str) -> Vec<u8> {
    let mut forms = Vec::new();
    for i in 0..depth {
        let id = 6 + i;
        forms.push(if i + 1 == depth {
            pdf_stream(&format!("{FORM} /Resources << /Font << /F1 5 0 R >> >>"), &format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET"))
        } else {
            pdf_stream(&format!("{FORM} /Resources << /XObject << /N {} 0 R >> >>", id + 1), "/N Do")
        });
    }
    one_page_pdf("/XObject << /N 6 0 R >>", "/N Do", &forms)
}

/// `depth` forms, each drawing the next one `fan` times: `fan^depth` draws.
fn pdf_form_fan(depth: usize, fan: usize) -> Vec<u8> {
    let mut forms = Vec::new();
    for i in 0..depth {
        let id = 6 + i;
        forms.push(if i + 1 == depth {
            pdf_stream(FORM, "0 0 m")
        } else {
            pdf_stream(&format!("{FORM} /Resources << /XObject << /N {} 0 R >> >>", id + 1), &"/N Do ".repeat(fan))
        });
    }
    one_page_pdf("/XObject << /N 6 0 R >>", "/N Do", &forms)
}

/// A page tree node that is its own parent, and no MediaBox anywhere.
fn pdf_parent_loop() -> Vec<u8> {
    pdf_of(&[
        CATALOG.to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /Parent 2 0 R >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_owned(),
        pdf_stream("", "BT ET"),
    ])
}

/// A page whose `/Parent` chain is `depth` dictionaries long, none of them
/// with a MediaBox.
fn pdf_parent_chain(depth: usize) -> Vec<u8> {
    let mut objects = vec![
        CATALOG.to_owned(),
        PAGES.to_owned(),
        "<< /Type /Page /Parent 5 0 R /Contents 4 0 R >>".to_owned(),
        pdf_stream("", "BT ET"),
    ];
    for i in 0..depth {
        objects.push(if i + 1 == depth { "<< /Type /Pages >>".to_owned() } else { format!("<< /Type /Pages /Parent {} 0 R >>", 6 + i) });
    }
    pdf_of(&objects)
}

#[test]
fn pdf_form_recursion_does_not_kill_the_process() {
    // pdf-extract runs every `Do` as a nested call with no depth limit: a
    // form that draws itself (or a loop, or a chain of 20,000 forms) ran the
    // 2 MB stack out, which ends the process whatever catches panics.
    let c = child("self-unchecked.pdf", &pdf_form_calling_itself(), &[UNCHECKED_PDF]);
    assert!(!c.ok && c.stderr.contains("overflowed its stack"), "without the check: {c:?}");
    for (name, pdf) in [
        ("self.pdf", pdf_form_calling_itself()),
        ("loop.pdf", pdf_two_forms_in_a_loop()),
        ("chain.pdf", pdf_form_chain(20_000, "deep")),
    ] {
        let c = child(name, &pdf, &[]);
        assert!(c.ok, "{name}: {c:?}");
        assert_eq!(c.text, Some(None), "{name}: {c:?}");
    }
    // The same inside an archive (members are read from memory).
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zw.start_file("a/self.pdf", zip::write::SimpleFileOptions::default()).unwrap();
    zw.write_all(&pdf_form_calling_itself()).unwrap();
    zw.start_file("after.txt", zip::write::SimpleFileOptions::default()).unwrap();
    zw.write_all("بعد الملف".as_bytes()).unwrap();
    let c = child("self.zip", &zw.finish().unwrap().into_inner(), &[]);
    assert!(c.ok && matches!(c.text, Some(Some(_))), "{c:?}");
}

#[test]
fn pdf_form_fan_out_does_not_hang() {
    // 40 forms each drawing the next twice: 2^40 draws, days of work.
    let c = child("fan.pdf", &pdf_form_fan(40, 2), &[]);
    assert!(c.ok, "{c:?}");
    assert_eq!(c.text, Some(None), "{c:?}");
    assert!(c.elapsed < Duration::from_secs(30), "{c:?}");
}

#[test]
fn pdf_parent_loop_does_not_kill_or_hang_the_process() {
    // pdf-extract looks for an inherited MediaBox up the /Parent chain with
    // no limit: a node that is its own parent never ends.
    let c = child("parent-loop-unchecked.pdf", &pdf_parent_loop(), &[UNCHECKED_PDF]);
    // Debug builds overflow the stack; optimised builds may turn the recursion
    // into a loop that never ends. Either way the unchecked read never returns.
    assert!(!c.ok && (c.stderr.contains("overflowed its stack") || c.status == "timed out"), "without the check: {c:?}");
    for (name, pdf) in [("parent-loop.pdf", pdf_parent_loop()), ("parent-chain.pdf", pdf_parent_chain(200_000))] {
        let c = child(name, &pdf, &[]);
        assert!(c.ok, "{name}: {c:?}");
        assert_eq!(c.text, Some(None), "{name}: {c:?}");
    }
}

#[test]
fn crash_loop_guard_skips_the_file_the_last_run_died_in() {
    // The reviewer's PDF with the check off stands for the next unknown
    // parser bug: the first run dies inside it, the second skips it.
    let pdf = temp_file("loop-start.pdf", &pdf_form_calling_itself());
    let store = std::env::temp_dir().join(format!("bahth_hostile_store_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&store);
    let env = [UNCHECKED_PDF, ("BAHTH_TEST_CACHE_DIR", store.to_str().unwrap())];
    let run = || extract_in_child(&pdf, 512 * MB, Duration::from_secs(60), &env);
    let first = run();
    assert!(!first.ok && first.stderr.contains("overflowed its stack"), "{first:?}");
    let second = run();
    assert!(second.ok && second.text == Some(None) && second.recovered == Some(1), "{second:?}");
    // Reported once; still skipped while the file stays as it is.
    let third = run();
    assert!(third.ok && third.text == Some(None) && third.recovered == Some(0), "{third:?}");
    let _ = std::fs::remove_dir_all(&store);
}

// ---------------------------------------------------------------------------
// Spreadsheets: allocations calamine cannot survive
// ---------------------------------------------------------------------------

/// A one-sheet xlsx (the shared fuzz sample) with `sst` as its shared strings.
fn xlsx_with_shared_strings(sst: &str) -> Vec<u8> {
    use super::fuzz::{XLSX_CT, XLSX_RELS, XLSX_SHEET, XLSX_WB, XLSX_WBRELS, zip_of};
    zip_of(
        &[
            ("[Content_Types].xml", XLSX_CT.as_bytes()),
            ("_rels/.rels", XLSX_RELS.as_bytes()),
            ("xl/workbook.xml", XLSX_WB.as_bytes()),
            ("xl/_rels/workbook.xml.rels", XLSX_WBRELS.as_bytes()),
            ("xl/worksheets/sheet1.xml", XLSX_SHEET.as_bytes()),
            ("xl/sharedStrings.xml", sst.as_bytes()),
        ],
        false,
    )
}

#[test]
fn xlsx_huge_unique_count_does_not_kill_the_process() {
    // calamine reserves `uniqueCount` strings (24 bytes each) before reading
    // one: 10^13 in a file of a few hundred bytes is a failed allocation,
    // which aborts.
    let bomb = xlsx_with_shared_strings(r#"<sst xmlns="x" uniqueCount="10000000000000"><si><t>a</t></si></sst>"#);
    // The unchecked abort needs 64-bit sizes: on a 32-bit build calamine cannot
    // even express the reservation, so only the guarded read is checked there.
    if cfg!(target_pointer_width = "64") {
        let c = child("count-unchecked.xlsx", &bomb, &[UNCHECKED_SHEETS]);
        assert!(!c.ok && c.text.is_none(), "without the check: {c:?}");
    }
    let c = child("count.xlsx", &bomb, &[]);
    assert!(c.ok && c.text == Some(None), "{c:?}");
    // An honest count, and none at all, still read.
    for sst in [
        r#"<sst xmlns="x" count="2" uniqueCount="1"><si><t>مشترك</t></si></sst>"#,
        r#"<sst xmlns="x"><si><t>مشترك</t></si></sst>"#,
    ] {
        let p = temp_file("honest.xlsx", &xlsx_with_shared_strings(sst));
        assert!(extract_text(&p).is_some_and(|t| t.contains("مرحبا")), "{sst}");
    }
    // Spellings another reader of the tag could take for a smaller number.
    for tag in [
        r#"<sst uniqueCount = "10000000000000">"#,
        r#"<sst uniqueCount='10000000000000'>"#,
        r#"<sst uniqueCount="+1">"#,
        r#"<sst uniqueCount="1" uniqueCount="10000000000000">"#,
        r#"<sst uniqueCount="9999999999999999999999">"#,
        r#"<sst uniqueCount>"#,
    ] {
        assert!(!declared_counts_ok(tag.as_bytes(), 1000), "{tag}");
    }
    assert!(declared_counts_ok(br#"<sst count="999999999999" uniqueCount="7">"#, 1000));
}

/// An .xls whose shared string table holds one string of `chars` Latin-1
/// characters, and whose sheet has `cells` LabelSst records all at A1
/// pointing at it.
fn xls_shared_string_bomb(chars: usize, cells: usize) -> Vec<u8> {
    use super::fuzz::{biff_record, utf16le};
    let bof = |dt: u16| {
        let mut d = 0x0600u16.to_le_bytes().to_vec();
        d.extend(dt.to_le_bytes());
        d.extend([0u8; 12]);
        biff_record(0x0809, &d)
    };
    // SST: totals, then cch, flags (0: one byte per character), the text,
    // continued in CONTINUE records that each start with a flags byte.
    let mut sst = Vec::new();
    let mut first = (cells as u32).to_le_bytes().to_vec();
    first.extend(1u32.to_le_bytes());
    first.extend((chars as u16).to_le_bytes());
    first.push(0);
    let text = vec![b'x'; chars];
    let (head, mut rest) = text.split_at(chars.min(8224 - first.len()));
    first.extend(head);
    sst.extend(biff_record(0x00FC, &first));
    while !rest.is_empty() {
        let (part, more) = rest.split_at(rest.len().min(8223));
        let mut cont = vec![0u8];
        cont.extend(part);
        sst.extend(biff_record(0x003C, &cont));
        rest = more;
    }
    let mut name = vec![3u8, 1];
    name.extend(utf16le("ورق"));
    let codepage = biff_record(0x0042, &1200u16.to_le_bytes());
    let eof = biff_record(0x000A, &[]);
    let mut globals = bof(0x0005);
    let sheet_pos = (globals.len() + codepage.len() + sst.len() + 4 + 6 + name.len() + eof.len()) as u32;
    globals.extend(codepage);
    globals.extend(sst);
    let mut bs = sheet_pos.to_le_bytes().to_vec();
    bs.extend([0u8, 0]);
    bs.extend(&name);
    globals.extend(biff_record(0x0085, &bs));
    globals.extend(&eof);
    let mut sheet = bof(0x0010);
    let mut dim = 0u32.to_le_bytes().to_vec();
    dim.extend(1u32.to_le_bytes());
    dim.extend(0u16.to_le_bytes());
    dim.extend(1u16.to_le_bytes());
    dim.extend(0u16.to_le_bytes());
    sheet.extend(biff_record(0x0200, &dim));
    // LabelSst: row, column, xf, string 0.
    let label = biff_record(0x00FD, &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    for _ in 0..cells {
        sheet.extend(&label);
    }
    sheet.extend(&eof);
    globals.extend(sheet);
    // Version 3, as Excel writes (calamine cannot read a large version-4 file).
    let mut comp = cfb::CompoundFile::create_with_version(cfb::Version::V3, std::io::Cursor::new(Vec::new())).unwrap();
    comp.create_stream("Workbook").unwrap().write_all(&globals).unwrap();
    comp.flush().unwrap();
    comp.into_inner().into_inner()
}

#[test]
fn xls_shared_string_copies_are_bounded() {
    // One 32,000-character string, 20,000 cells using it, all at A1: a 1x1
    // sheet as far as the cell check goes, but calamine clones the string
    // into every cell (640 MB here; a 2 MB file came to about 9 GB).
    let bomb = xls_shared_string_bomb(32_000, 20_000);
    assert!(bomb.len() < 400_000, "{}", bomb.len());
    let c = child("sst-unchecked.xls", &bomb, &[UNCHECKED_SHEETS]);
    assert!(!c.ok && c.text.is_none(), "without the check: {c:?}");
    // Checked: the raw-bytes fallback, in little memory.
    let c = child("sst.xls", &bomb, &[]);
    assert!(c.ok && c.peak.is_some_and(|p| p < 64 * MB), "{c:?}");
    // The sizes are those calamine reads (the string spans 4 records).
    let stream = cfb::CompoundFile::open(std::io::Cursor::new(&bomb))
        .and_then(|mut f| {
            let mut v = Vec::new();
            std::io::Read::read_to_end(&mut f.open_stream("Workbook")?, &mut v)?;
            Ok(v)
        })
        .unwrap();
    let recs: Vec<(u16, &[u8])> = biff_records(&stream).collect();
    let at = recs.iter().position(|r| r.0 == 0x00FC).unwrap();
    let cont: Vec<&[u8]> = recs[at + 1..].iter().take_while(|r| r.0 == 0x003C).map(|r| r.1).collect();
    assert_eq!(cont.len(), 3);
    assert_eq!(xls_shared_string_sizes(recs[at].1, &cont, true), Some(vec![64_000]));
    // A few cells sharing it are fine, and read by calamine.
    let small = xls_shared_string_bomb(32_000, 100);
    let t = spreadsheet_text(std::io::Cursor::new(&small), "xls").unwrap();
    assert_eq!(t.len(), 32_000 + 2, "one cell at A1, the last copy wins");
}
