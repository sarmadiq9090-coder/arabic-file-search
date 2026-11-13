//! Fuzz-style robustness tests for every extractor: no input, however
//! truncated, corrupt or hostile, may panic in this crate's code, and no
//! panic (ours or a third-party parser's) may escape `extract_text`.
//!
//! For each format a small valid sample is built in the test, then fed as:
//! empty bytes, random bytes, the sample truncated at many offsets, and the
//! sample with random byte flips (for zip-based formats also with flips in
//! the XML inside an intact zip). A panic hook records where every panic on
//! the test thread happened, including panics that `catch_unwind` inside the
//! extractors swallows, so a bug in our own parsers cannot hide behind the
//! outer guard. Third-party panics are allowed (they are contained), but
//! counted and printed (`cargo test fuzz -- --nocapture`).

use std::cell::RefCell;
use std::io::{Cursor, Write};
use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;

use super::*;

/// A small xorshift generator: the same cases on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn bytes(&mut self, max: usize) -> Vec<u8> {
        (0..self.below(max)).map(|_| self.next() as u8).collect()
    }
}

// ---------------------------------------------------------------------------
// Panic bookkeeping
// ---------------------------------------------------------------------------

thread_local! {
    /// Locations (`file:line`) and messages of panics on this thread while recording.
    static RECORD: RefCell<Option<Vec<(String, String)>>> = const { RefCell::new(None) };
}

fn install_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let recorded = RECORD
                .try_with(|r| {
                    let Ok(mut r) = r.try_borrow_mut() else { return false };
                    let Some(v) = r.as_mut() else { return false };
                    let loc = info.location().map_or(String::new(), |l| format!("{}:{}", l.file(), l.line()));
                    let msg = info
                        .payload()
                        .downcast_ref::<&str>()
                        .map(|s| s.to_string())
                        .or_else(|| info.payload().downcast_ref::<String>().cloned())
                        .unwrap_or_default();
                    v.push((loc, msg));
                    true
                })
                .unwrap_or(false);
            if !recorded {
                previous(info);
            }
        }));
    });
}

/// What happened while running one input.
struct Outcome {
    /// Every panic, caught or not: (location, message).
    panics: Vec<(String, String)>,
    /// The panic unwound out of `f`.
    escaped: bool,
}

impl Outcome {
    /// Panics located in this crate's own sources.
    fn ours(&self) -> Vec<&(String, String)> {
        self.panics.iter().filter(|(loc, _)| loc.starts_with("src/") || loc.starts_with("src\\")).collect()
    }
}

fn run(f: impl FnOnce()) -> Outcome {
    install_hook();
    RECORD.with(|r| *r.borrow_mut() = Some(Vec::new()));
    let escaped = panic::catch_unwind(AssertUnwindSafe(f)).is_err();
    let panics = RECORD.with(|r| r.borrow_mut().take()).unwrap_or_default();
    Outcome { panics, escaped }
}

/// Tally of contained third-party panics, printed at the end of a test.
#[derive(Default)]
struct Tally {
    inputs: usize,
    foreign: Vec<(String, String)>,
}

impl Tally {
    fn report(&self, what: &str) {
        let mut locs: Vec<&str> = self.foreign.iter().map(|(l, _)| l.as_str()).collect();
        locs.sort_unstable();
        locs.dedup();
        eprintln!("{what}: {} inputs, {} contained third-party panics at {locs:?}", self.inputs, self.foreign.len());
    }
}

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

/// Empty, random, truncated and bit-flipped versions of `sample`.
fn variants(sample: &[u8], rng: &mut Rng, flips: usize) -> Vec<Vec<u8>> {
    let mut v = vec![Vec::new()];
    for _ in 0..12 {
        v.push(rng.bytes(700));
    }
    // Random bytes behind the sample's first bytes (its magic number).
    for _ in 0..6 {
        let mut b = sample[..sample.len().min(8)].to_vec();
        b.extend(rng.bytes(400));
        v.push(b);
    }
    let n = sample.len();
    let step = (n / 160).max(1);
    let mut cut = 1;
    while cut < n {
        v.push(sample[..cut].to_vec());
        cut += if cut < 64 { 1 } else { step };
    }
    for _ in 0..flips {
        let mut b = sample.to_vec();
        for _ in 0..1 + rng.below(8) {
            let i = rng.below(n);
            b[i] = match rng.below(4) {
                0 => 0,
                1 => 0xFF,
                2 => b[i] ^ (1 << rng.below(8)),
                _ => rng.next() as u8,
            };
        }
        v.push(b);
    }
    // A chunk removed from / duplicated inside the middle.
    for _ in 0..8 {
        let (a, len) = (rng.below(n), rng.below(64));
        let mut b = sample.to_vec();
        let end = (a + len).min(n);
        if rng.below(2) == 0 {
            b.drain(a..end);
        } else {
            let chunk = b[a..end].to_vec();
            b.splice(a..a, chunk);
        }
        v.push(b);
    }
    v
}

/// `name` in a folder of its own, removed when the guard is dropped.
fn temp_path(name: &str) -> crate::test_temp::TempPath {
    crate::test_temp::TempPath::in_own_dir("bahth_fuzz_tests", name)
}

/// Feeds `data` to the extractors for `ext`: the byte-level dispatcher with
/// no guard around it, and `extract_text` on a file. Fails on any panic in
/// our code and on any panic escaping `extract_text`.
fn check(ext: &str, data: &[u8], tally: &mut Tally, file_name: &str) {
    tally.inputs += 1;
    let label = || format!("{ext} input of {} bytes: {:?}", data.len(), &data[..data.len().min(80)]);

    // In-memory path (archive members), no catch_unwind of ours around it.
    let mem = run(|| {
        let _ = archive::member_text_inner(ext, data.to_vec());
    });
    // From disk, the way the search calls it.
    let path = temp_path(file_name);
    std::fs::write(&path, data).unwrap();
    let disk = run(|| {
        let _ = extract_text(&path);
    });
    assert!(!disk.escaped, "panic escaped extract_text for {}: {:?}", label(), disk.panics);
    for o in [&mem, &disk] {
        let ours = o.ours();
        assert!(ours.is_empty(), "panic in our code for {}: {ours:?}", label());
        tally.foreign.extend(o.panics.iter().cloned());
    }
}

fn fuzz(ext: &str, sample: &[u8], seed: u64, flips: usize) {
    let mut rng = Rng(seed);
    let mut tally = Tally::default();
    let file_name = format!("fuzz_{seed:x}.{ext}");
    for data in variants(sample, &mut rng, flips) {
        check(ext, &data, &mut tally, &file_name);
    }
    tally.report(ext);
}

pub(super) fn zip_of(entries: &[(&str, &[u8])], stored: bool) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = if stored {
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)
    } else {
        zip::write::SimpleFileOptions::default()
    };
    for (n, data) in entries {
        zw.start_file(*n, opts).unwrap();
        zw.write_all(data).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

/// Fuzzes a zip-based format twice: flips in the zip bytes (deflated), and
/// flips/truncations of each XML member inside an intact (stored) zip.
fn fuzz_zip_doc(ext: &str, entries: &[(&str, &str)], seed: u64) {
    let raw: Vec<(&str, &[u8])> = entries.iter().map(|(n, d)| (*n, d.as_bytes())).collect();
    fuzz(ext, &zip_of(&raw, false), seed, 80);
    fuzz(ext, &zip_of(&raw, true), seed ^ 0x55, 40);

    let mut rng = Rng(seed ^ 0xABCD);
    let mut tally = Tally::default();
    let file_name = format!("fuzz_inner_{seed:x}.{ext}");
    for (k, (_, content)) in entries.iter().enumerate() {
        for data in variants(content.as_bytes(), &mut rng, 25) {
            let mut members = raw.clone();
            members[k].1 = &data;
            check(ext, &zip_of(&members, true), &mut tally, &file_name);
        }
    }
    tally.report(&format!("{ext} (inner XML)"));
}

// ---------------------------------------------------------------------------
// Samples
// ---------------------------------------------------------------------------

const DOCX_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="w"><w:body><w:p><w:pPr><w:tabs><w:tab w:val="left"/></w:tabs></w:pPr><w:r><w:t>مرحبا &amp; &#1578;</w:t></w:r><w:r><w:tab/><w:br/><w:t xml:space="preserve"> تقرير </w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>خلية</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>"#;
const HEADER_XML: &str = r#"<w:hdr xmlns:w="w"><w:p><w:r><w:t>ترويسة</w:t></w:r></w:p></w:hdr>"#;
const SLIDE_XML: &str = r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>شريحة</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#;
const ODT_XML: &str = r#"<?xml version="1.0"?><office:document-content xmlns:office="o" xmlns:text="t"><office:body><office:text><text:h>عنوان</text:h><text:p>نص<text:s text:c="99999999999999999999"/>x<text:tab/><text:line-break/></text:p><office:annotation><text:p>no</text:p></office:annotation></office:text></office:body></office:document-content>"#;
const CONTAINER_XML: &str = r#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/c%20o.opf"/></rootfiles></container>"#;
const OPF_XML: &str = r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf"><manifest><item id="a" href="../OEBPS/t/ch1.xhtml#x" media-type="application/xhtml+xml"/><item id="b" href="/t/%D9%81.xhtml"/></manifest><spine><itemref idref="a"/><itemref idref="b"/><itemref idref="a"/></spine></package>"#;
pub(super) const XLSX_CT: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#;
pub(super) const XLSX_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
pub(super) const XLSX_WB: &str = r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="ورقة1" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
pub(super) const XLSX_WBRELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;
pub(super) const XLSX_SHEET: &str = r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><dimension ref="A1:B2"/><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>مرحبا</t></is></c><c r="B1"><v>42</v></c></row><row r="2"><c r="XFD1048576" t="inlineStr"><is><t>تقرير</t></is></c></row></sheetData></worksheet>"#;
const ODS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:spreadsheet><table:table table:name="ورقة"><table:table-row><table:table-cell table:number-columns-repeated="3"><text:p>خلية</text:p></table:table-cell></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#;

pub(super) fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

fn rtf_sample() -> Vec<u8> {
    let (cp, _, _) = WINDOWS_1256.encode("مرحبا");
    let hex: String = cp.iter().map(|b| format!("\\'{b:02x}")).collect();
    format!(
        "{{\\rtf1\\ansi\\ansicpg1256{{\\fonttbl{{\\f0\\fcharset178 Arial;}}{{\\f1\\cpg932 MS;}}}}{{\\*\\generator x;}}\
         \\pard\\f0 {hex}\\par\\uc2\\u-1580\\'3f\\'3f\\u55357?\\u56832?\\f1\\'82\\'a0{{\\pict\\bin4 \\}}{{}}x}}\\tab\\bin99999999999 end}}"
    )
    .into_bytes()
}

fn eml_sample() -> Vec<u8> {
    let inner = "Subject: =?windows-1256?Q?=E3=D1=CD=C8=C7?=\r\nContent-Type: text/plain; charset=cp1256\r\n\r\ninner body\r\n";
    format!(
        "From mbox line\r\nSubject: =?utf-8?B?2YXYsdit2KjYpw==?= =?utf-8?Q?_=D8=AA?=\r\nFrom: a@b\r\n To: folded\r\n\
         Content-Type: multipart/mixed; boundary=\"XX\"; charset*=utf-8''ar\r\n\r\npreamble\r\n--XX\r\n\
         Content-Type: text/plain; charset=\"utf-8\"\r\nContent-Transfer-Encoding: base64\r\n\r\n2YXYsdit2KjYpw==\r\n--XX\r\n\
         Content-Type: text/html\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n<p>a=3Db=\r\nc &amp; =D9</p>\r\n--XX\r\n\
         Content-Type: message/rfc822\r\n\r\n{inner}--XX\r\nContent-Type: multipart/digest; boundary=YY\r\n\r\n--YY\r\n\r\n{inner}--YY--\r\n--XX--\r\n"
    )
    .into_bytes()
}

pub(super) fn cfb_of(streams: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut comp = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    for (name, data) in streams {
        let mut s = comp.create_stream(name).unwrap();
        s.write_all(data).unwrap();
    }
    comp.flush().unwrap();
    comp.into_inner().into_inner()
}

fn msg_sample() -> Vec<u8> {
    let mut props = vec![0u8; 32];
    for (tag, value) in [(0x3FFD_0003u32, 1256u32), (0x3FDE_0003, 65001)] {
        props.extend(tag.to_le_bytes());
        props.extend(6u32.to_le_bytes());
        props.extend(value.to_le_bytes());
        props.extend(0u32.to_le_bytes());
    }
    let (subject8, _, _) = WINDOWS_1256.encode("موضوع");
    cfb_of(&[
        ("__properties_version1.0", props),
        ("__substg1.0_0037001E", subject8.into_owned()),
        ("__substg1.0_0C1A001F", utf16le("مرسل\0")),
        ("__substg1.0_1000001F", utf16le("")),
        ("__substg1.0_10130102", b"<p>&#1605;\xE3</p>".to_vec()),
    ])
}

pub(super) fn biff_record(kind: u16, data: &[u8]) -> Vec<u8> {
    let mut r = kind.to_le_bytes().to_vec();
    r.extend((data.len() as u16).to_le_bytes());
    r.extend(data);
    r
}

fn xls_sample() -> Vec<u8> {
    xls_with((0, 1, 0, 1), &[(0, 0, "خلية")])
}

/// A one-sheet BIFF8 workbook with the given `DIMENSIONS` (first row, row
/// after the last, first column, column after the last) and LABEL cells.
pub(super) fn xls_with(dims: (u32, u32, u16, u16), cells: &[(u16, u16, &str)]) -> Vec<u8> {
    let bof = |dt: u16| {
        let mut d = 0x0600u16.to_le_bytes().to_vec();
        d.extend(dt.to_le_bytes());
        d.extend([0u8; 12]);
        biff_record(0x0809, &d)
    };
    let mut globals = bof(0x0005);
    let sheet_name: Vec<u8> = {
        let mut n = vec![3u8, 1]; // 3 chars, UTF-16
        n.extend(utf16le("ورق"));
        n
    };
    let boundsheet_len = 4 + 6 + sheet_name.len();
    let codepage = biff_record(0x0042, &1200u16.to_le_bytes()); // UTF-16, as BIFF8 requires
    let eof = biff_record(0x000A, &[]);
    let sheet_pos = (globals.len() + codepage.len() + boundsheet_len + eof.len()) as u32;
    globals.extend(codepage);
    let mut bs = sheet_pos.to_le_bytes().to_vec();
    bs.extend([0u8, 0]);
    bs.extend(&sheet_name);
    globals.extend(biff_record(0x0085, &bs));
    globals.extend(&eof);
    let mut sheet = bof(0x0010);
    let mut dim = dims.0.to_le_bytes().to_vec();
    dim.extend(dims.1.to_le_bytes());
    dim.extend(dims.2.to_le_bytes());
    dim.extend(dims.3.to_le_bytes());
    dim.extend(0u16.to_le_bytes());
    sheet.extend(biff_record(0x0200, &dim));
    for (row, col, text) in cells {
        // LABEL: row, column, xf, then the string (UTF-16).
        let mut label = row.to_le_bytes().to_vec();
        label.extend(col.to_le_bytes());
        label.extend(0u16.to_le_bytes());
        label.extend((text.chars().count() as u16).to_le_bytes());
        label.push(1);
        label.extend(utf16le(text));
        sheet.extend(biff_record(0x0204, &label));
    }
    sheet.extend(&eof);
    globals.extend(sheet);
    cfb_of(&[("Workbook", globals)])
}

fn doc_sample() -> Vec<u8> {
    let mut data = cfb_of(&[("WordDocument", utf16le("مرحبا بالعالم\rتقرير"))]);
    let (cp, _, _) = WINDOWS_1256.encode("نص قديم للبحث");
    data.extend(cp.iter());
    data
}

// ---------------------------------------------------------------------------
// Tests: one per format family
// ---------------------------------------------------------------------------

#[test]
fn samples_are_valid() {
    // The fuzzing starts from files the extractors really read.
    let docx = zip_of(&[("word/document.xml", DOCX_XML.as_bytes())], false);
    let xlsx = zip_of(
        &[
            ("[Content_Types].xml", XLSX_CT.as_bytes()),
            ("_rels/.rels", XLSX_RELS.as_bytes()),
            ("xl/workbook.xml", XLSX_WB.as_bytes()),
            ("xl/_rels/workbook.xml.rels", XLSX_WBRELS.as_bytes()),
            ("xl/worksheets/sheet1.xml", XLSX_SHEET.as_bytes()),
        ],
        false,
    );
    let ods = zip_of(&[("mimetype", b"application/vnd.oasis.opendocument.spreadsheet"), ("content.xml", ODS_XML.as_bytes())], false);
    let epub = zip_of(
        &[
            ("META-INF/container.xml", CONTAINER_XML.as_bytes()),
            ("OEBPS/c o.opf", OPF_XML.as_bytes()),
            ("OEBPS/t/ch1.xhtml", "<p>الفصل</p>".as_bytes()),
        ],
        false,
    );
    for (ext, data, want) in [
        ("rtf", rtf_sample(), "مرحبا"),
        ("eml", eml_sample(), "inner body"),
        ("msg", msg_sample(), "موضوع"),
        ("xls", xls_sample(), "خلية"),
        ("doc", doc_sample(), "تقرير"),
        ("docx", docx, "مرحبا & ت"),
        ("xlsx", xlsx, "مرحبا\t42"),
        ("ods", ods, "خلية"),
        ("epub", epub, "الفصل"),
        ("pdf", tests::tiny_pdf("Annual Report"), "Annual"),
    ] {
        let text = archive::member_text_inner(ext, data).unwrap_or_default();
        assert!(text.contains(want), "{ext}: {text:?}");
    }
}

#[test]
fn fuzz_plain_text_encodings() {
    let ar = "مرحبا بالعالم report ‘quote’ 😀‍👩";
    let mut le_bom = vec![0xFF, 0xFE];
    le_bom.extend(utf16le(ar));
    le_bom.push(b'x'); // odd length
    let mut be_bom = vec![0xFE, 0xFF];
    be_bom.extend(ar.encode_utf16().flat_map(|u| u.to_be_bytes()));
    be_bom.push(0xD8); // odd length, half a surrogate
    let mut nobom = utf16le(ar);
    nobom.push(0); // odd length, BOM-less
    let mut utf8_bom = vec![0xEF, 0xBB, 0xBF];
    utf8_bom.extend(ar.as_bytes());
    let (cp1256, _, _) = WINDOWS_1256.encode("مرحبا تقرير");
    for (i, sample) in [ar.as_bytes().to_vec(), le_bom, be_bom, nobom, utf8_bom, cp1256.into_owned()]
        .iter()
        .enumerate()
    {
        fuzz("txt", sample, 0x1000 + i as u64, 40);
        // Every decoder path directly, on every prefix.
        for cut in 0..=sample.len() {
            let _ = decode_bytes(&sample[..cut]);
        }
    }
}

#[test]
fn fuzz_visual_order_and_presentation_forms() {
    // What PDFs give: presentation forms, marks, tatweel, digits, brackets,
    // number terminators, RTL marks and line breaks in any order.
    const PIECES: &[&str] = &[
        "\u{FEE3}", "\u{FEAE}", "\u{FEA3}", "\u{FE92}", "\u{FE8E}", "\u{FEFB}", "\u{FEF5}", "\u{FDF2}", "\u{FE70}", "\u{FE73}",
        "\u{FE80}", "\u{FEF4}", "يف", "نم", "لا", "ال", "في", "\u{064E}", "\u{0651}", "\u{0640}", "١٢", "42", "%", "\u{066A}",
        "$", "(", ")", "[", "«", "»", "abc", " ", "\n", "\t", "\u{200F}", "\u{200E}", "😀", "\u{0}", "\r\n",
    ];
    let mut rng = Rng(0xC000);
    for _ in 0..4000 {
        let s: String = (0..rng.below(24)).map(|_| PIECES[rng.below(PIECES.len())]).collect();
        let o = run(|| {
            let fixed = bidi_fix::fix_visual_order(s.clone());
            let _ = normalize_arabic_forms(&fixed);
            assert!(fixed.len() >= s.len());
        });
        assert!(o.panics.is_empty(), "fix_visual_order({s:?}): {:?}", o.panics);
    }
}

#[test]
fn fuzz_html() {
    let html = "<html><head><style>p{}</style><SCRIPT>var x='<b>';</SCRIPT></head><body><p>مرحبا &amp;&#1578;&#x642;&#xD800;&#99999999999;&nbsp‘&</p><!-- c --><script\n>";
    fuzz("html", html.as_bytes(), 0x2000, 80);
    let mut rng = Rng(0x2001);
    const PIECES: &[&str] = &["<", ">", "&", ";", "#", "x", "<!--", "-->", "<script>", "</script", "<style ", "ب", "‘", "😀", "&#", "amp", "\u{0}"];
    for _ in 0..3000 {
        let s: String = (0..rng.below(30)).map(|_| PIECES[rng.below(PIECES.len())]).collect();
        let o = run(|| {
            let _ = strip_html(&s);
        });
        assert!(o.panics.is_empty(), "strip_html({s:?}): {:?}", o.panics);
    }
}

#[test]
fn fuzz_rtf() {
    fuzz("rtf", &rtf_sample(), 0x3000, 150);
    // Pathological nesting and control words.
    for data in [
        "{".repeat(200_000).into_bytes(),
        "}".repeat(1000).into_bytes(),
        "\\".repeat(1001).into_bytes(),
        b"{\\rtf1\\u-99999999999\\uc-5\\ucx\\'\\'z\\'".to_vec(),
        b"{\\rtf1\\ansicpg99999\\f-2147483648\\cpg-1 \\'e3}".to_vec(),
        b"{\\fonttbl{\\f1\\cpg65535;}}\\f1\\'e3".to_vec(),
    ] {
        let o = run(|| {
            let _ = rtf_to_text(&data);
        });
        assert!(o.panics.is_empty(), "{:?}", o.panics);
    }
}

#[test]
fn fuzz_legacy_doc() {
    fuzz("doc", &doc_sample(), 0x4000, 100);
}

#[test]
fn fuzz_eml() {
    fuzz("eml", &eml_sample(), 0x5000, 200);
    // Deeply nested multiparts and forwarded messages.
    let mut nested = String::from("x");
    for i in 0..300 {
        nested = format!("Content-Type: multipart/mixed; boundary=b{i}\r\n\r\n--b{i}\r\n{nested}\r\n--b{i}--\r\n");
    }
    let mut fwd = String::from("Subject: x\r\n\r\nbody");
    for _ in 0..300 {
        fwd = format!("Subject: y\r\nContent-Type: message/rfc822\r\n\r\n{fwd}");
    }
    for data in [nested, fwd, "Subject: =?utf-8?B?".to_owned(), "a:".repeat(5000)] {
        let o = run(|| {
            let _ = mail::eml_to_text(data.as_bytes());
        });
        assert!(o.panics.is_empty(), "{:?}", o.panics);
    }
    let mut rng = Rng(0x5001);
    const PIECES: &[&str] = &["=?", "?=", "?", "utf-8", "B", "Q", "=", "_", "2Y", "=D9", " ", "windows-1256", "ب", "\r\n", "\t"];
    for _ in 0..3000 {
        let s: String = (0..rng.below(20)).map(|_| PIECES[rng.below(PIECES.len())]).collect();
        let o = run(|| {
            let _ = mail::decode_header_value(s.as_bytes());
        });
        assert!(o.panics.is_empty(), "decode_header_value({s:?}): {:?}", o.panics);
    }
}

#[test]
fn fuzz_msg() {
    fuzz("msg", &msg_sample(), 0x6000, 200);
}

#[test]
fn fuzz_xls() {
    fuzz("xls", &xls_sample(), 0x7000, 100);
    fuzz("xlsb", &[0x50, 0x4B, 0x03, 0x04], 0x7001, 4);
}

#[test]
fn fuzz_docx() {
    fuzz_zip_doc("docx", &[("word/document.xml", DOCX_XML), ("word/header1.xml", HEADER_XML)], 0x8000);
}

#[test]
fn fuzz_pptx() {
    fuzz_zip_doc("pptx", &[("ppt/slides/slide1.xml", SLIDE_XML), ("ppt/notesSlides/notesSlide1.xml", SLIDE_XML)], 0x8100);
}

#[test]
fn fuzz_odt() {
    fuzz_zip_doc("odt", &[("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", ODT_XML)], 0x8200);
}

#[test]
fn fuzz_xlsx() {
    fuzz_zip_doc(
        "xlsx",
        &[
            ("[Content_Types].xml", XLSX_CT),
            ("_rels/.rels", XLSX_RELS),
            ("xl/workbook.xml", XLSX_WB),
            ("xl/_rels/workbook.xml.rels", XLSX_WBRELS),
            ("xl/worksheets/sheet1.xml", XLSX_SHEET),
        ],
        0x8300,
    );
}

#[test]
fn fuzz_ods() {
    fuzz_zip_doc("ods", &[("mimetype", "application/vnd.oasis.opendocument.spreadsheet"), ("content.xml", ODS_XML)], 0x8400);
}

#[test]
fn fuzz_epub() {
    fuzz_zip_doc(
        "epub",
        &[
            ("META-INF/container.xml", CONTAINER_XML),
            ("OEBPS/c o.opf", OPF_XML),
            ("OEBPS/t/ch1.xhtml", "<html><body><p>الفصل &amp; الأول</p></body></html>"),
            ("t/ف.xhtml", "<p>ثان"),
        ],
        0x8500,
    );
}

#[test]
fn fuzz_pdf() {
    fuzz("pdf", &tests::tiny_pdf("Annual Report"), 0x9000, 250);
}

#[test]
fn hostile_pdfs_do_not_crash() {
    // Each must come back (None or text) without a panic escaping and
    // without overflowing the stack of an ordinary 2 MB thread.
    let mut cases: Vec<Vec<u8>> = Vec::new();
    // Deeply nested arrays / dictionaries in an object.
    for open in ["[", "<<"] {
        let mut pdf = tests::tiny_pdf("x");
        let pos = pdf.windows(6).position(|w| w == b"5 0 obj").unwrap_or(0);
        let deep = format!("6 0 obj\n{}\nendobj\n", open.repeat(100_000));
        pdf.splice(pos..pos, deep.into_bytes());
        cases.push(pdf);
    }
    // A page tree whose Kids point back at itself.
    cases.push(tests::tiny_pdf("x").replace_all(b"/Kids [3 0 R]", b"/Kids [2 0 R]"));
    // A Parent loop and a self-referencing Contents.
    cases.push(tests::tiny_pdf("x").replace_all(b"/Contents 4 0 R", b"/Contents 3 0 R"));
    // A content stream with deeply nested q / BT and a huge declared length.
    cases.push(tests::tiny_pdf(&"(".repeat(50_000)));
    cases.push(tests::tiny_pdf("x").replace_all(b"/Length ", b"/Length 99999999999"));
    for (i, pdf) in cases.into_iter().enumerate() {
        let path = temp_path(&format!("hostile{i}.pdf"));
        std::fs::write(&path, &pdf).unwrap();
        let handle = std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(move || {
                let o = run(|| {
                    let _ = extract_text(&path);
                });
                (o.escaped, o.ours().into_iter().cloned().collect::<Vec<_>>())
            })
            .unwrap();
        let (escaped, ours) = handle.join().expect("extraction thread died");
        assert!(!escaped && ours.is_empty(), "case {i}: {ours:?}");
    }
}

/// `replace_all` for byte vectors (test helper).
trait ReplaceAll {
    fn replace_all(self, from: &[u8], to: &[u8]) -> Vec<u8>;
}

impl ReplaceAll for Vec<u8> {
    fn replace_all(self, from: &[u8], to: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.len());
        let mut i = 0;
        while i < self.len() {
            if self[i..].starts_with(from) {
                out.extend_from_slice(to);
                i += from.len();
            } else {
                out.push(self[i]);
                i += 1;
            }
        }
        out
    }
}

fn archive_docx() -> Vec<u8> {
    zip_of(&[("word/document.xml", DOCX_XML.as_bytes())], false)
}

#[test]
fn fuzz_zip_archive() {
    let docx = archive_docx();
    let members: Vec<(&str, &[u8])> = vec![
        ("مجلد/تقرير.docx", &docx),
        ("../../evil/ملاحظات.txt", "المبيعات".as_bytes()),
        ("inner.zip", &docx),
        ("C:\\x\u{202E}.rtf", b"{\\rtf1 hi}"),
    ];
    fuzz("zip", &zip_of(&members, false), 0xA000, 100);
}

#[test]
fn fuzz_tar_archives() {
    let docx = archive_docx();
    let mut tar = tar::Builder::new(Vec::new());
    for (n, data) in [("مجلد/تقرير.docx", docx.as_slice()), ("ملاحظات.txt", "المبيعات".as_bytes())] {
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        tar.append_data(&mut h, n, data).unwrap();
    }
    let tar = tar.into_inner().unwrap();
    fuzz("tar", &tar, 0xA100, 50);
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&tar).unwrap();
    fuzz("tgz", &gz.finish().unwrap(), 0xA200, 50);
}

#[test]
fn fuzz_7z_archive() {
    let docx = archive_docx();
    let path = temp_path("sample.7z");
    {
        use sevenz_rust2::{ArchiveEntry, ArchiveWriter};
        let mut w = ArchiveWriter::create(&path).unwrap();
        w.push_archive_entry(ArchiveEntry::new_file("تقرير.docx"), Some(docx.as_slice())).unwrap();
        w.push_archive_entry(ArchiveEntry::new_file("نص.txt"), Some("المبيعات".as_bytes())).unwrap();
        w.finish().unwrap();
    }
    fuzz("7z", &std::fs::read(&path).unwrap(), 0xA300, 80);
}

#[test]
fn files_that_vanish_or_are_not_files() {
    let dir = temp_path("a_dir.txt");
    std::fs::create_dir_all(&dir).unwrap();
    assert!(extract_text(&dir).is_none());
    assert!(extract_text(&temp_path("missing.docx")).is_none());
    assert!(extract_text(Path::new("")).is_none());
    assert!(extract_text(Path::new(".txt")).is_none());
}

/// A one-page PDF showing a Flate-compressed grayscale image with a PNG
/// predictor (the path `pdf_images` decodes itself).
fn image_pdf() -> Vec<u8> {
    use lopdf::{Document, Object, Stream, dictionary};
    let (w, h) = (40usize, 36usize);
    let mut rows = Vec::new();
    for y in 0..h {
        rows.push((y % 5) as u8); // every PNG filter type
        rows.extend((0..w).map(|x| (x * y) as u8));
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&rows).unwrap();
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let img = doc.add_object(Stream::new(
        dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => w as i64, "Height" => h as i64,
        "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8, "Filter" => "FlateDecode",
        "DecodeParms" => dictionary! { "Predictor" => 15, "Columns" => w as i64 } },
        z.finish().unwrap(),
    ));
    let content = doc.add_object(Stream::new(dictionary! {}, b"q 100 0 0 100 0 0 cm /Im0 Do Q".to_vec()));
    let page = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id, "Contents" => content,
        "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
        "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => img } },
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![Object::Reference(page)], "Count" => 1 }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

#[test]
fn fuzz_pdf_page_images() {
    // The scanned-PDF path (OCR on Windows): lopdf, then our image decoding.
    let sample = image_pdf();
    let doc = lopdf::Document::load_mem(&sample).unwrap();
    assert_eq!(pdf_images::page_images(&doc, pdf_images::MAX_PAGES).len(), 1);
    let mut rng = Rng(0xB000);
    let mut tally = Tally::default();
    for data in variants(&sample, &mut rng, 250) {
        tally.inputs += 1;
        let o = run(|| {
            if let Ok(doc) = lopdf::Document::load_mem(&data) {
                let _ = pdf_images::page_count(&doc);
                let _ = pdf_images::page_images(&doc, pdf_images::MAX_PAGES);
            }
        });
        let ours = o.ours();
        assert!(ours.is_empty(), "panic in our code for a PDF of {} bytes: {ours:?}", data.len());
        tally.foreign.extend(o.panics);
    }
    tally.report("pdf page images");
}
