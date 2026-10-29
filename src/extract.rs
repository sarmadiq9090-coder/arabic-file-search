//! Plain-text extraction from documents for content search.
//!
//! Every public entry point is defensive: failures, unsupported formats and
//! panics inside third-party parsers all turn into `None`. Arabic text is a
//! first-class concern: legacy files fall back to windows-1256, RTF honours
//! `\ansicpg` / font charsets, and Arabic presentation forms (common in PDFs)
//! are folded back to their base letters so searches for normal text match.

// Only the Windows UI and the tests use this module.

use std::borrow::Cow;
use std::fs::File;
use std::io::{Read, Seek};
use std::ops::Range;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;

use calamine::{Data, Reader, Sheets};
use encoding_rs::{Encoding, UTF_16BE, UTF_16LE, WINDOWS_1252, WINDOWS_1256};
use quick_xml::events::Event;

mod bidi_fix;
mod epub;
mod mail;

/// Files larger than this are never opened.
pub const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024;
/// Plain-text formats are read only up to this many bytes.
const MAX_TEXT_READ: u64 = 20 * 1024 * 1024;
/// Upper bound for a single decompressed zip member (zip-bomb guard).
const MAX_ZIP_ENTRY: u64 = 64 * 1024 * 1024;
/// Upper bound (in bytes) for text produced from spreadsheets / zip documents.
const MAX_OUTPUT: usize = 64 * 1024 * 1024;

const TEXT_EXTS: &[&str] = &[
    "txt", "md", "csv", "tsv", "log", "json", "xml", "html", "htm", "ini", "cfg", "yaml", "yml",
    "srt", "rs", "py", "js", "ts", "c", "cpp", "h", "cs", "java", "sql", "bat", "ps1", "php",
    "css",
];

const OTHER_EXTS: &[&str] = &[
    "docx", "docm", "pptx", "odt", "odp", "ods", "xlsx", "xlsm", "xlsb", "xls", "pdf", "rtf",
    "doc", "eml", "msg", "epub",
];

/// Returns true if the extension (lowercase, no dot) is supported for content search.
pub fn is_supported(ext: &str) -> bool {
    TEXT_EXTS.contains(&ext) || OTHER_EXTS.contains(&ext)
}

/// Extract plain text from the file at `path`. Returns None on any failure/unsupported.
///
/// Never panics: the whole extraction runs inside `catch_unwind`, and files
/// larger than 100 MB are skipped.
pub fn extract_text(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if !is_supported(&ext) {
        return None;
    }
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_SIZE {
        return None;
    }

    let text = panic::catch_unwind(AssertUnwindSafe(|| extract_by_ext(path, &ext)))
        .ok()
        .flatten()?;
    let text = normalize_arabic_forms(&text).into_owned();
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn extract_by_ext(path: &Path, ext: &str) -> Option<String> {
    match ext {
        "html" | "htm" => Some(strip_html(&read_text_file(path)?)),
        e if TEXT_EXTS.contains(&e) => read_text_file(path),
        "docx" | "docm" => extract_docx(path),
        "pptx" => extract_pptx(path),
        "odt" | "odp" => extract_odf(path),
        "xlsx" | "xlsm" | "xlsb" | "xls" | "ods" => extract_spreadsheet(path, ext),
        "pdf" => extract_pdf(path),
        "rtf" => Some(rtf_to_text(&std::fs::read(path).ok()?)),
        "doc" => Some(extract_doc_heuristic(&std::fs::read(path).ok()?)),
        "eml" => mail::extract_eml(path),
        "msg" => mail::extract_msg(path),
        "epub" => epub::extract_epub(path),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Plain text
// ---------------------------------------------------------------------------

/// Reads at most `cap` bytes from `r`.
fn read_capped(r: impl Read, cap: u64) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    r.take(cap).read_to_end(&mut buf).ok()?;
    Some(buf)
}

fn read_text_file(path: &Path) -> Option<String> {
    Some(decode_bytes(&read_capped(File::open(path).ok()?, MAX_TEXT_READ)?))
}

/// Decode bytes of unknown encoding: BOMs first, then a BOM-less UTF-16
/// guess, then UTF-8, finally windows-1256 (the Arabic Windows code page).
fn decode_bytes(buf: &[u8]) -> String {
    if let Some(rest) = buf.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if let Some(rest) = buf.strip_prefix(&[0xFF, 0xFE]) {
        return UTF_16LE.decode_without_bom_handling(rest).0.into_owned();
    }
    if let Some(rest) = buf.strip_prefix(&[0xFE, 0xFF]) {
        return UTF_16BE.decode_without_bom_handling(rest).0.into_owned();
    }
    // Must precede the UTF-8 check: ASCII-range UTF-16 is also valid UTF-8.
    if let Some(enc) = guess_utf16_without_bom(buf) {
        return enc.decode_without_bom_handling(buf).0.into_owned();
    }
    match std::str::from_utf8(buf) {
        Ok(s) => return s.to_owned(),
        // Only an incomplete sequence at the very end (e.g. the 20 MB read cap
        // cut a character in half): keep the valid prefix.
        Err(e) if e.error_len().is_none() => {
            return String::from_utf8_lossy(&buf[..e.valid_up_to()]).into_owned();
        }
        Err(_) => {}
    }
    WINDOWS_1256.decode_without_bom_handling(buf).0.into_owned()
}

/// Detects BOM-less UTF-16 by the share of zero bytes in even/odd positions.
fn guess_utf16_without_bom(buf: &[u8]) -> Option<&'static Encoding> {
    let sample = &buf[..buf.len().min(4096)];
    if sample.len() < 8 {
        return None;
    }
    let pairs = sample.len() / 2;
    let (mut even0, mut odd0) = (0usize, 0usize);
    for pair in sample.chunks_exact(2) {
        even0 += (pair[0] == 0) as usize;
        odd0 += (pair[1] == 0) as usize;
    }
    // ASCII-heavy UTF-16 has ~100% zeros on one side; Arabic UTF-16 has the
    // high byte 0x06 instead, so also accept a dominant 0x06 high byte.
    let odd06 = sample.chunks_exact(2).filter(|p| p[1] == 0x06).count();
    let even06 = sample.chunks_exact(2).filter(|p| p[0] == 0x06).count();
    if (odd0 + odd06) * 10 >= pairs * 7 && even0 * 10 < pairs {
        Some(UTF_16LE)
    } else if (even0 + even06) * 10 >= pairs * 7 && odd0 * 10 < pairs {
        Some(UTF_16BE)
    } else {
        None
    }
}

/// Very crude HTML-to-text: drops tags, comments, `<script>`/`<style>` bodies
/// and decodes common entities.
fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let lower = html.to_ascii_lowercase(); // same byte offsets as `html`
    let mut i = 0;
    while i < html.len() {
        let rest = &html[i..];
        if rest.starts_with("<!--") {
            i += rest.find("-->").map_or(rest.len(), |p| p + 3);
        } else if rest.starts_with('<') {
            let lrest = &lower[i..];
            let skip_to = ["script", "style"].iter().find_map(|tag| {
                let open = format!("<{tag}");
                let next = lrest.as_bytes().get(open.len()).copied();
                (lrest.starts_with(&open)
                    && matches!(next, Some(b'>' | b' ' | b'\t' | b'\n' | b'\r')))
                .then(|| lrest.find(&format!("</{tag}")).unwrap_or(lrest.len()))
            });
            if let Some(off) = skip_to {
                i += off;
                if i >= html.len() {
                    break;
                }
            }
            let rest = &html[i..];
            i += rest.find('>').map_or(rest.len(), |p| p + 1);
            out.push(' ');
        } else {
            let end = rest.find('<').unwrap_or(rest.len());
            decode_html_entities(&rest[..end], &mut out);
            i += end;
        }
    }
    out
}

fn decode_html_entities(s: &str, out: &mut String) {
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let semi = rest[..rest.len().min(12)].find(';');
        let decoded = semi.and_then(|semi| {
            let name = &rest[1..semi];
            let ch = if let Some(num) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X"))
            {
                u32::from_str_radix(num, 16).ok().and_then(char::from_u32)
            } else if let Some(num) = name.strip_prefix('#') {
                num.parse::<u32>().ok().and_then(char::from_u32)
            } else {
                match name {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some(' '),
                    _ => None,
                }
            };
            ch.map(|c| (c, semi + 1))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
}

// ---------------------------------------------------------------------------
// Zip-based documents (OOXML and ODF)
// ---------------------------------------------------------------------------

type Zip = zip::ZipArchive<File>;

fn open_zip(path: &Path) -> Option<Zip> {
    zip::ZipArchive::new(File::open(path).ok()?).ok()
}

/// Reads one zip member as text (size-capped). Missing members yield None.
fn read_zip_entry<R: Read + Seek>(zip: &mut zip::ZipArchive<R>, name: &str) -> Option<String> {
    let entry = zip.by_name(name).ok()?;
    Some(decode_bytes(&read_capped(entry, MAX_ZIP_ENTRY)?))
}

/// Names of members whose name is `{prefix}{N}.xml`, sorted by N.
fn numbered_entries<R: Read + Seek>(zip: &zip::ZipArchive<R>, prefix: &str) -> Vec<String> {
    let mut found: Vec<(u32, String)> = zip
        .file_names()
        .filter_map(|n| {
            let n = n.ok()?;
            let num = n.strip_prefix(prefix)?.strip_suffix(".xml")?.parse().ok()?;
            Some((num, n.into_owned()))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, n)| n).collect()
}

/// Which text nodes of an XML part carry document text.
#[derive(Clone, Copy, PartialEq)]
enum XmlFlavor {
    /// OOXML: only text inside `<w:t>` / `<a:t>` elements counts.
    Ooxml,
    /// ODF: all character data inside `<office:body>` counts.
    Odf,
}

/// Pulls the text out of an OOXML/ODF XML part, appending to `out`.
/// Paragraph ends become newlines, table cells tabs, so words never fuse.
fn xml_to_text(xml: &str, flavor: XmlFlavor, out: &mut String) {
    let mut reader = quick_xml::Reader::from_str(xml);
    // Local names of currently open elements.
    let mut stack: Vec<String> = Vec::new();
    let collecting = |stack: &[String]| match flavor {
        XmlFlavor::Ooxml => stack.last().is_some_and(|n| n == "t"),
        XmlFlavor::Odf => {
            stack.iter().any(|n| n == "body")
                && !stack
                    .iter()
                    .any(|n| n == "annotation" || n == "tracked-changes")
        }
    };

    loop {
        if out.len() > MAX_OUTPUT {
            break;
        }
        let event = match reader.read_event() {
            Ok(Event::Eof) | Err(_) => break,
            Ok(ev) => ev,
        };
        match event {
            Event::Start(e) => {
                let name = e.local_name().as_ref().to_owned();
                stack.push(name);
            }
            Event::Empty(e) => {
                let name = e.local_name();
                let parent = stack.last().map(String::as_str);
                match (name.as_ref(), flavor) {
                    // `w:tab` inside `w:tabs` is a tab-stop definition, not a tab.
                    ("tab", _) if parent != Some("tabs") => out.push('\t'),
                    ("br" | "cr" | "line-break", _) => out.push('\n'),
                    ("p" | "h", _) => out.push('\n'),
                    ("s", XmlFlavor::Odf) => {
                        let count = e
                            .attributes()
                            .flatten()
                            .find(|a| a.key.local_name().as_ref() == "c")
                            .and_then(|a| a.value.parse::<usize>().ok())
                            .unwrap_or(1)
                            .min(64);
                        out.extend(std::iter::repeat_n(' ', count));
                    }
                    _ => {}
                }
            }
            Event::End(_) => {
                if let Some(name) = stack.pop() {
                    match name.as_str() {
                        "p" | "h" | "tr" | "table-row" => out.push('\n'),
                        "tc" | "table-cell" => out.push('\t'),
                        _ => {}
                    }
                }
            }
            Event::Text(t) if collecting(&stack) => out.push_str(&t.xml10_content()),
            Event::CData(t) if collecting(&stack) => out.push_str(&t.xml10_content()),
            Event::GeneralRef(r) if collecting(&stack) => {
                if r.is_char_ref() {
                    if let Ok(Some(c)) = r.resolve_char_ref() {
                        out.push(c);
                    }
                } else if let Some(s) = quick_xml::escape::resolve_xml_entity(&r.xml10_content()) {
                    out.push_str(s);
                }
            }
            _ => {}
        }
    }
}

fn extract_docx(path: &Path) -> Option<String> {
    let mut zip = open_zip(path)?;
    let main = read_zip_entry(&mut zip, "word/document.xml")?;
    let mut out = String::new();
    xml_to_text(&main, XmlFlavor::Ooxml, &mut out);

    // Secondary parts: headers, footers, notes, comments.
    let mut extra = numbered_entries(&zip, "word/header");
    extra.extend(numbered_entries(&zip, "word/footer"));
    extra.extend(
        [
            "word/footnotes.xml",
            "word/endnotes.xml",
            "word/comments.xml",
        ]
        .map(String::from),
    );
    for name in extra {
        if let Some(xml) = read_zip_entry(&mut zip, &name) {
            out.push('\n');
            xml_to_text(&xml, XmlFlavor::Ooxml, &mut out);
        }
    }
    Some(out)
}

fn extract_pptx(path: &Path) -> Option<String> {
    let mut zip = open_zip(path)?;
    let slides = numbered_entries(&zip, "ppt/slides/slide");
    if slides.is_empty() {
        return None;
    }
    let notes = numbered_entries(&zip, "ppt/notesSlides/notesSlide");
    let mut out = String::new();
    for name in slides.iter().chain(&notes) {
        if let Some(xml) = read_zip_entry(&mut zip, name) {
            xml_to_text(&xml, XmlFlavor::Ooxml, &mut out);
            out.push('\n');
        }
    }
    Some(out)
}

fn extract_odf(path: &Path) -> Option<String> {
    let mut zip = open_zip(path)?;
    let xml = read_zip_entry(&mut zip, "content.xml")?;
    let mut out = String::new();
    xml_to_text(&xml, XmlFlavor::Odf, &mut out);
    Some(out)
}

// ---------------------------------------------------------------------------
// Spreadsheets (calamine)
// ---------------------------------------------------------------------------

fn extract_spreadsheet(path: &Path, ext: &str) -> Option<String> {
    use calamine::{Ods, Xls, Xlsb, Xlsx, open_workbook};
    // Dispatch on our lowercase extension (calamine's auto-detection is
    // case-sensitive, so "REPORT.XLSX" would go through slow guessing).
    let mut book: Sheets<_> = match ext {
        "xlsx" | "xlsm" => Sheets::Xlsx(open_workbook::<Xlsx<_>, _>(path).ok()?),
        "xlsb" => Sheets::Xlsb(open_workbook::<Xlsb<_>, _>(path).ok()?),
        "xls" => Sheets::Xls(open_workbook::<Xls<_>, _>(path).ok()?),
        "ods" => Sheets::Ods(open_workbook::<Ods<_>, _>(path).ok()?),
        _ => return None,
    };

    let mut out = String::new();
    let sheet_count = book.sheet_names().len();
    for i in 0..sheet_count {
        let Some(Ok(range)) = book.worksheet_range_at(i) else {
            continue;
        };
        for row in range.rows() {
            let mut first = true;
            for cell in row {
                if matches!(cell, Data::Empty) {
                    continue;
                }
                if !first {
                    out.push('\t');
                }
                first = false;
                match cell {
                    Data::String(s) => out.push_str(s),
                    other => out.push_str(&other.to_string()),
                }
            }
            if !first {
                out.push('\n');
            }
            if out.len() > MAX_OUTPUT {
                return Some(out);
            }
        }
        out.push('\n');
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// PDF
// ---------------------------------------------------------------------------

fn extract_pdf(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    // pdf-extract is known to panic on malformed files; contain it here even
    // though `extract_text` has an outer guard as well. (It does not print to
    // stdout in normal operation; in a GUI-subsystem build there is no console
    // to print to anyway.)
    let text = panic::catch_unwind(AssertUnwindSafe(|| {
        pdf_extract::extract_text_from_mem(&bytes)
    }))
    .ok()?
    .ok()?;
    // Before `extract_text` folds presentation forms: their shapes are the
    // best evidence of visual (reversed) Arabic line order.
    Some(bidi_fix::fix_visual_order(text))
}

// ---------------------------------------------------------------------------
// RTF
// ---------------------------------------------------------------------------

/// Group-scoped parser state.
#[derive(Clone, Copy)]
struct RtfGroup {
    /// Inside a destination whose text is ignored.
    skip: bool,
    /// Inside `\fonttbl` (still parsed, to learn font charsets).
    font_table: bool,
    /// Number of fallback chars after `\uN` (`\ucN`).
    uc: usize,
    /// Current font number (`\fN`) for the body text.
    font: Option<i32>,
}

/// Destinations whose content is not document text.
const RTF_SKIP_DESTINATIONS: &[&str] = &[
    "fonttbl",
    "colortbl",
    "stylesheet",
    "info",
    "pict",
    "listtable",
    "listoverridetable",
    "revtbl",
    "rsidtbl",
    "themedata",
    "colorschememapping",
    "datastore",
    "latentstyles",
    "xmlnstbl",
    "generator",
    "filetbl",
    "objdata",
    "fldinst",
    "pgdsctbl",
    "listtext",
    "pntext",
    "pntxta",
    "pntxtb",
    "bkmkstart",
    "bkmkend",
    "xmlopen",
    "mmathPr",
    "falt",
    "panose",
    "fontemb",
    "fontfile",
    "author",
    "operator",
    "title",
    "subject",
    "keywords",
    "comment",
    "doccomm",
    "company",
    "category",
    "manager",
    "hlinkbase",
    "userprops",
    "template",
    "nonshppict",
    "shpinst",
    "sp",
    "sn",
    "sv",
    "blipuid",
    "protusertbl",
    "wgrffmtfilter",
    "defchp",
    "defpap",
];

/// Windows code page for an RTF `\fcharsetN` value.
fn charset_to_codepage(charset: i32) -> Option<u16> {
    Some(match charset {
        77 => 10000,
        128 => 932,
        129 => 949,
        134 => 936,
        136 => 950,
        161 => 1253,
        162 => 1254,
        163 => 1258,
        177 => 1255,
        178..=180 => 1256,
        186 => 1257,
        204 => 1251,
        222 => 874,
        238 => 1250,
        _ => return None, // 0 = ANSI, 1 = default, 2 = symbol: use \ansicpg
    })
}

fn codepage_encoding(cp: u16) -> &'static Encoding {
    let label = match cp {
        932 => "shift_jis".to_owned(),
        936 => "gbk".to_owned(),
        949 => "euc-kr".to_owned(),
        950 => "big5".to_owned(),
        10000 => "macintosh".to_owned(),
        874 => "windows-874".to_owned(),
        cp => format!("windows-{cp}"),
    };
    Encoding::for_label(label.as_bytes()).unwrap_or(WINDOWS_1252)
}

/// Accumulates RTF output, buffering raw code-page bytes and UTF-16 units so
/// multi-byte sequences and surrogate pairs decode correctly.
struct RtfOut {
    text: String,
    bytes: Vec<u8>,
    bytes_enc: &'static Encoding,
    units: Vec<u16>,
}

impl RtfOut {
    fn push_byte(&mut self, b: u8, enc: &'static Encoding) {
        if !self.bytes.is_empty() && self.bytes_enc != enc {
            self.flush();
        }
        self.flush_units();
        self.bytes_enc = enc;
        self.bytes.push(b);
    }

    fn push_unit(&mut self, u: u16) {
        self.flush_bytes();
        self.units.push(u);
    }

    fn push_char(&mut self, c: char) {
        self.flush();
        self.text.push(c);
    }

    fn flush_bytes(&mut self) {
        if !self.bytes.is_empty() {
            let (s, _) = self.bytes_enc.decode_without_bom_handling(&self.bytes);
            self.text.push_str(&s);
            self.bytes.clear();
        }
    }

    fn flush_units(&mut self) {
        if !self.units.is_empty() {
            self.text.extend(
                char::decode_utf16(self.units.drain(..))
                    .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER)),
            );
        }
    }

    fn flush(&mut self) {
        self.flush_bytes();
        self.flush_units();
    }
}

/// Lexes the RTF control word whose first letter is `data[i]`: the letters,
/// an optional signed number and one optional delimiting space. Returns the
/// word, its parameter (clamped to i32) and the position after it.
fn rtf_control_word(data: &[u8], mut i: usize) -> (&str, Option<i32>, usize) {
    let start = i;
    while i < data.len() && data[i].is_ascii_alphabetic() {
        i += 1;
    }
    let word = std::str::from_utf8(&data[start..i]).unwrap_or("");
    let num_start = i;
    if i < data.len() && data[i] == b'-' {
        i += 1;
    }
    while i < data.len() && data[i].is_ascii_digit() {
        i += 1;
    }
    let param = std::str::from_utf8(&data[num_start..i])
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .map(|n| n.clamp(i32::MIN as i64, i32::MAX as i64) as i32);
    if i < data.len() && data[i] == b' ' {
        i += 1;
    }
    (word, param, i)
}

/// The character a symbol control word (`\par`, `\emdash`…) stands for.
fn rtf_word_char(word: &str) -> Option<char> {
    Some(match word {
        "par" | "line" | "sect" | "page" | "row" => '\n',
        "tab" | "cell" => '\t',
        "emdash" => '\u{2014}',
        "endash" => '\u{2013}',
        "lquote" => '\u{2018}',
        "rquote" => '\u{2019}',
        "ldblquote" => '\u{201C}',
        "rdblquote" => '\u{201D}',
        "bullet" => '\u{2022}',
        "emspace" | "enspace" | "qmspace" => ' ',
        "rtlmark" => '\u{200F}',
        "ltrmark" => '\u{200E}',
        _ => return None,
    })
}

fn rtf_to_text(data: &[u8]) -> String {
    let mut out = RtfOut {
        text: String::new(),
        bytes: Vec::new(),
        bytes_enc: WINDOWS_1252,
        units: Vec::new(),
    };
    let mut ansi_cp: u16 = 1252;
    // Font number -> code page, learned from \fonttbl.
    let mut font_cp: Vec<(i32, u16)> = Vec::new();
    let mut table_font: Option<i32> = None;

    let mut state = RtfGroup {
        skip: false,
        font_table: false,
        uc: 1,
        font: None,
    };
    let mut stack: Vec<RtfGroup> = Vec::new();
    // True right after '{' (a following `\*` or destination word applies to this group).
    let mut group_start = false;
    // Fallback characters still to skip after a `\uN`.
    let mut pending_skip = 0usize;

    let current_enc = |state: &RtfGroup, font_cp: &[(i32, u16)], ansi_cp: u16| {
        let cp = state
            .font
            .and_then(|f| font_cp.iter().find(|(n, _)| *n == f).map(|(_, cp)| *cp))
            .unwrap_or(ansi_cp);
        codepage_encoding(cp)
    };

    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        match b {
            b'{' => {
                stack.push(state);
                group_start = true;
                pending_skip = 0;
                i += 1;
                continue;
            }
            b'}' => {
                if let Some(prev) = stack.pop() {
                    state = prev;
                }
                pending_skip = 0;
                i += 1;
                group_start = false;
                continue;
            }
            b'\\' => {
                i += 1;
                let Some(&c) = data.get(i) else { break };
                if c.is_ascii_alphabetic() {
                    let (word, param, next) = rtf_control_word(data, i);
                    i = next;
                    let was_group_start = std::mem::take(&mut group_start);

                    if word == "bin" {
                        // Raw binary payload: skip it entirely.
                        i = i.saturating_add(param.unwrap_or(0).max(0) as usize);
                        continue;
                    }
                    if word == "fonttbl" {
                        state.skip = true;
                        state.font_table = true;
                        continue;
                    }
                    if state.font_table {
                        let cp = match word {
                            "f" => {
                                table_font = param;
                                None
                            }
                            "fcharset" => param.and_then(charset_to_codepage),
                            "cpg" => param.and_then(|p| u16::try_from(p).ok()),
                            _ => None,
                        };
                        if let (Some(f), Some(cp)) = (table_font, cp) {
                            font_cp.retain(|(n, _)| *n != f);
                            font_cp.push((f, cp));
                        }
                        continue;
                    }
                    if was_group_start && RTF_SKIP_DESTINATIONS.contains(&word) {
                        state.skip = true;
                        continue;
                    }
                    match word {
                        "ansicpg" => {
                            if let Some(cp) = param.and_then(|p| u16::try_from(p).ok()) {
                                ansi_cp = cp;
                            }
                        }
                        "uc" => state.uc = param.unwrap_or(1).clamp(0, 16) as usize,
                        "f" => state.font = param,
                        _ if state.skip => {}
                        "u" => {
                            if let Some(n) = param {
                                let unit = if n < 0 { (n + 65536) as u16 } else { n as u16 };
                                out.push_unit(unit);
                                pending_skip = state.uc;
                            }
                            continue;
                        }
                        w => {
                            if let Some(ch) = rtf_word_char(w) {
                                out.push_char(ch);
                            }
                        }
                    }
                    continue;
                }
                // Control symbol.
                i += 1;
                group_start = false;
                match c {
                    b'*' => {
                        // `{\*\dest ...}`: unknown/optional destination, skip it.
                        state.skip = true;
                    }
                    b'\'' => {
                        let hex = data.get(i..i + 2).and_then(|h| std::str::from_utf8(h).ok());
                        let byte = hex.and_then(|h| u8::from_str_radix(h, 16).ok());
                        if byte.is_some() {
                            i += 2;
                        }
                        if pending_skip > 0 {
                            pending_skip -= 1;
                        } else if let (Some(byte), false) = (byte, state.skip) {
                            let enc = current_enc(&state, &font_cp, ansi_cp);
                            out.push_byte(byte, enc);
                        }
                    }
                    _ if state.skip => {}
                    _ if pending_skip > 0 => pending_skip -= 1,
                    b'\\' | b'{' | b'}' => out.push_char(c as char),
                    b'~' => out.push_char('\u{00A0}'),
                    b'_' => out.push_char('\u{2011}'),
                    b'\n' | b'\r' => out.push_char('\n'), // `\<newline>` == \par
                    _ => {} // \- optional hyphen, \: index subentry, ...
                }
            }
            b'\r' | b'\n' => {
                i += 1;
            }
            _ => {
                i += 1;
                group_start = false;
                if pending_skip > 0 {
                    pending_skip -= 1;
                } else if !state.skip {
                    if b < 0x80 {
                        out.push_char(b as char);
                    } else {
                        // Non-conforming writers emit raw 8-bit text.
                        let enc = current_enc(&state, &font_cp, ansi_cp);
                        out.push_byte(b, enc);
                    }
                }
            }
        }
    }
    out.flush();
    out.text
}

// ---------------------------------------------------------------------------
// Legacy Word .doc (heuristic)
// ---------------------------------------------------------------------------

fn is_doc_text_unit(u: u16) -> bool {
    matches!(u,
        0x09 | 0x0D | 0x0B
        | 0x20..=0x7E
        | 0xA0..=0xFF
        | 0x0600..=0x06FF   // Arabic
        | 0x0750..=0x077F   // Arabic supplement
        | 0x2000..=0x206F   // general punctuation (incl. RLM/LRM)
        | 0xFB50..=0xFDFF   // Arabic presentation forms A
        | 0xFE70..=0xFEFF) // Arabic presentation forms B
}

/// Maximal runs of consecutive items for which `keep` holds, as index ranges.
fn runs_where<T: Copy>(items: &[T], keep: impl Fn(T) -> bool) -> impl Iterator<Item = Range<usize>> {
    let mut pos = 0;
    std::iter::from_fn(move || {
        let start = pos + items.get(pos..)?.iter().position(|&x| keep(x))?;
        let len = items[start..].iter().position(|&x| !keep(x)).unwrap_or(items.len() - start);
        pos = start + len;
        Some(start..pos)
    })
}

/// Best-effort text recovery from a binary Word 97-2003 file: collects runs of
/// UTF-16LE text and runs of printable 8-bit text (decoded as windows-1256).
fn extract_doc_heuristic(data: &[u8]) -> String {
    let mut runs: Vec<(usize, String)> = Vec::new();

    // UTF-16LE runs (Word stores Unicode text pieces this way, word-aligned).
    let units: Vec<u16> = data
        .chunks_exact(2)
        .map(|p| u16::from_le_bytes([p[0], p[1]]))
        .collect();
    for Range { start, end } in runs_where(&units, is_doc_text_unit) {
        let run = &units[start..end];
        let letters = run
            .iter()
            .filter_map(|&u| char::from_u32(u as u32))
            .filter(|c| c.is_alphanumeric())
            .count();
        if run.len() >= 4 && letters >= 2 {
            let s: String = char::decode_utf16(run.iter().copied())
                .map(|r| match r {
                    Ok('\r' | '\u{0B}') => '\n',
                    Ok(c) => c,
                    Err(_) => ' ',
                })
                .collect();
            runs.push((start * 2, s));
        }
    }

    // Single-byte runs (Word "compressed" pieces), decoded as windows-1256.
    let is_byte_text = |b: u8| matches!(b, 0x09 | 0x0D | 0x20..=0x7E | 0x80..=0xFF);
    for Range { start, end } in runs_where(data, is_byte_text) {
        if end - start >= 5 {
            let (s, _) = WINDOWS_1256.decode_without_bom_handling(&data[start..end]);
            let total = s.chars().count();
            let letters = s.chars().filter(|c| c.is_alphabetic()).count();
            let good = s
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '\r')
                .count();
            // Reject binary noise: require real letters and mostly text-like chars.
            if letters >= 3 && good * 10 >= total * 8 {
                runs.push((start, s.replace('\r', "\n")));
            }
        }
    }

    runs.sort_by_key(|(pos, _)| *pos);
    let mut out = String::new();
    for (_, s) in runs {
        let s = s.trim();
        if !s.is_empty() {
            out.push_str(s);
            out.push('\n');
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Arabic presentation forms
// ---------------------------------------------------------------------------

/// Base letters for U+FE80..=U+FEF4, in code point order.
const FE80_LETTERS: &str = "ءآأؤإئابةتثجحخدذرزسشصضطظعغفقكلمنهوىي";
/// Number of contextual forms (isolated/final/initial/medial) per letter above.
const FE80_FORM_COUNTS: [u8; 36] = [
    1, 2, 2, 2, 2, 4, 2, 4, 2, 4, 4, 4, 4, 4, 2, 2, 2, 2, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    4, 2, 2, 4,
];

/// For a code point in U+FE80..=U+FEF4: (index of its letter in
/// [`FE80_LETTERS`], form index 0 isolated / 1 final / 2 initial / 3 medial).
fn fe80_form(cp: u32) -> Option<(usize, usize)> {
    let mut offset = cp.checked_sub(0xFE80)?;
    for (k, &n) in FE80_FORM_COUNTS.iter().enumerate() {
        if offset < n as u32 {
            return Some((k, offset as usize));
        }
        offset -= n as u32;
    }
    None
}

/// Maps one Arabic presentation-form character to its base text.
fn presentation_form_base(c: char) -> Option<&'static str> {
    let cp = c as u32;
    match cp {
        0xFE70..=0xFE7F => {
            // Harakat: isolated / medial pairs.
            const MARKS: [&str; 8] = [
                "\u{064B}", "\u{064C}", "\u{064D}", "\u{064E}", "\u{064F}", "\u{0650}", "\u{0651}",
                "\u{0652}",
            ];
            let idx = match cp {
                0xFE70 | 0xFE71 => 0,
                0xFE72 => 1,
                0xFE74 => 2,
                _ => (cp - 0xFE76) as usize / 2 + 3,
            };
            if cp == 0xFE73 || cp == 0xFE75 {
                None
            } else {
                MARKS.get(idx).copied()
            }
        }
        0xFE80..=0xFEF4 => {
            let (k, _) = fe80_form(cp)?;
            // Slice the k-th letter out (all are 2-byte UTF-8).
            FE80_LETTERS.get(k * 2..k * 2 + 2)
        }
        0xFEF5 | 0xFEF6 => Some("لآ"),
        0xFEF7 | 0xFEF8 => Some("لأ"),
        0xFEF9 | 0xFEFA => Some("لإ"),
        0xFEFB | 0xFEFC => Some("لا"),
        0xFDF2 => Some("الله"),
        _ => None,
    }
}

/// Replaces Arabic presentation forms (as emitted by many PDF producers) with
/// ordinary Arabic letters. Borrows the input when nothing needs changing.
fn normalize_arabic_forms(s: &str) -> Cow<'_, str> {
    if !s.chars().any(|c| presentation_form_base(c).is_some()) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match presentation_form_base(c) {
            Some(base) => out.push_str(base),
            None => out.push(c),
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bahth_extract_tests_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    fn write_file(name: &str, bytes: &[u8]) -> PathBuf {
        let p = temp_path(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    fn write_zip(name: &str, entries: &[(&str, &str)]) -> PathBuf {
        let p = temp_path(name);
        let mut zw = zip::ZipWriter::new(File::create(&p).unwrap());
        for (n, content) in entries {
            zw.start_file(*n, zip::write::SimpleFileOptions::default())
                .unwrap();
            zw.write_all(content.as_bytes()).unwrap();
        }
        zw.finish().unwrap();
        p
    }

    #[test]
    fn supported_extensions() {
        for e in [
            "txt", "docx", "pdf", "rtf", "doc", "xlsx", "ods", "pptx", "css", "eml", "msg", "epub",
        ] {
            assert!(is_supported(e), "{e}");
        }
        assert!(!is_supported("exe"));
        assert!(!is_supported("DOCX")); // callers pass lowercase
    }

    #[test]
    fn text_utf8_bom_and_plain() {
        let p = write_file("a.txt", "مرحبا بالعالم".as_bytes());
        assert!(extract_text(&p).unwrap().contains("مرحبا"));
        let mut b = vec![0xEF, 0xBB, 0xBF];
        b.extend_from_slice("تقرير".as_bytes());
        let p = write_file("b.md", &b);
        assert_eq!(extract_text(&p).unwrap(), "تقرير");
    }

    #[test]
    fn text_utf16_le_be() {
        let s = "تقرير سنوي report";
        let mut le = vec![0xFF, 0xFE];
        le.extend(s.encode_utf16().flat_map(|u| u.to_le_bytes()));
        assert_eq!(extract_text(&write_file("le.txt", &le)).unwrap(), s);
        let mut be = vec![0xFE, 0xFF];
        be.extend(s.encode_utf16().flat_map(|u| u.to_be_bytes()));
        assert_eq!(extract_text(&write_file("be.csv", &be)).unwrap(), s);
        // BOM-less UTF-16LE
        let raw: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(extract_text(&write_file("nobom.log", &raw)).unwrap(), s);
    }

    #[test]
    fn text_cp1256_fallback() {
        let (bytes, _, _) = WINDOWS_1256.encode("مرحبا تقرير");
        let p = write_file("ar.txt", &bytes);
        assert_eq!(extract_text(&p).unwrap(), "مرحبا تقرير");
    }

    #[test]
    fn utf8_truncated_tail_keeps_prefix() {
        let mut b = "مرحبا".as_bytes().to_vec();
        b.push(0xD8); // half of an Arabic letter
        assert_eq!(decode_bytes(&b), "مرحبا");
    }

    #[test]
    fn html_stripping() {
        let html = "<html><head><style>p{}</style><script>var x='<b>';</script></head>\
                    <body><p>مرحبا</p><p>&amp; &#1578;&#x642;</p><!-- c --></body></html>";
        let t = extract_text(&write_file("x.html", html.as_bytes())).unwrap();
        assert!(t.contains("مرحبا"));
        assert!(t.contains("& تق"));
        assert!(!t.contains("var x"));
        assert!(!t.contains("p{}"));
    }

    #[test]
    fn docx_paragraphs_headers() {
        let doc = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:pPr><w:tabs><w:tab w:val="left" w:pos="100"/></w:tabs></w:pPr><w:r><w:t>مرحبا</w:t></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">تقرير</w:t></w:r><w:r><w:tab/><w:t>A&amp;B</w:t></w:r><w:r><w:instrText>HYPERLINK</w:instrText></w:r></w:p>
</w:body></w:document>"#;
        let header = r#"<w:hdr xmlns:w="x"><w:p><w:r><w:t>ترويسة</w:t></w:r></w:p></w:hdr>"#;
        let p = write_zip(
            "t.docx",
            &[("word/document.xml", doc), ("word/header1.xml", header)],
        );
        let t = extract_text(&p).unwrap();
        assert!(t.contains("مرحبا\n"), "{t:?}");
        assert!(t.contains("تقرير\tA&B"), "{t:?}");
        assert!(!t.contains("مرحباتقرير"));
        assert!(!t.contains("HYPERLINK"));
        assert!(t.contains("ترويسة"));
    }

    #[test]
    fn pptx_slide_order() {
        let slide = |s: &str| {
            format!(
                r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>{s}</a:t></a:r></a:p><a:p><a:r><a:t>نهاية</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#
            )
        };
        let (s1, s2, s10) = (slide("مرحبا"), slide("تقرير"), slide("عاشر"));
        let p = write_zip(
            "t.pptx",
            &[
                ("ppt/slides/slide10.xml", &s10),
                ("ppt/slides/slide2.xml", &s2),
                ("ppt/slides/slide1.xml", &s1),
            ],
        );
        let t = extract_text(&p).unwrap();
        let (a, b, c) = (
            t.find("مرحبا").unwrap(),
            t.find("تقرير").unwrap(),
            t.find("عاشر").unwrap(),
        );
        assert!(a < b && b < c, "{t:?}");
        assert!(!t.contains("مرحبانهاية"));
    }

    #[test]
    fn odt_content() {
        let content = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="o" xmlns:text="t" xmlns:style="s">
<office:automatic-styles><style:style style:name="P1"/></office:automatic-styles>
<office:body><office:text><text:h>تقرير</text:h><text:p>مرحبا<text:s text:c="2"/>بكم<text:tab/>x<text:line-break/>y</text:p><text:p>آخر</text:p></office:text></office:body>
</office:document-content>"#;
        let p = write_zip(
            "t.odt",
            &[
                ("mimetype", "application/vnd.oasis.opendocument.text"),
                ("content.xml", content),
            ],
        );
        let t = extract_text(&p).unwrap();
        assert!(t.contains("تقرير\n"), "{t:?}");
        assert!(t.contains("مرحبا  بكم\tx\ny\n"), "{t:?}");
        assert!(t.contains("آخر"));
    }

    #[test]
    fn xlsx_inline_strings() {
        let ct = r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#;
        let rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
        let wb = r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="ورقة1" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
        let wbrels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;
        let sheet = r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>مرحبا</t></is></c><c r="B1"><v>42</v></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>تقرير</t></is></c></row></sheetData></worksheet>"#;
        let p = write_zip(
            "t.xlsx",
            &[
                ("[Content_Types].xml", ct),
                ("_rels/.rels", rels),
                ("xl/workbook.xml", wb),
                ("xl/_rels/workbook.xml.rels", wbrels),
                ("xl/worksheets/sheet1.xml", sheet),
            ],
        );
        let t = extract_text(&p).unwrap();
        assert!(t.contains("مرحبا\t42"), "{t:?}");
        assert!(t.contains("تقرير"), "{t:?}");
    }

    #[test]
    fn rtf_ansicpg1256_and_unicode() {
        let (cp, _, _) = WINDOWS_1256.encode("مرحبا");
        let hex: String = cp.iter().map(|b| format!("\\'{b:02x}")).collect();
        let uni: String = "تقرير"
            .encode_utf16()
            .map(|u| format!("\\u{}?", u as i16))
            .collect();
        let rtf = format!(
            "{{\\rtf1\\ansi\\ansicpg1256\\deff0{{\\fonttbl{{\\f0\\fcharset178 Arial;}}}}\
             {{\\colortbl;\\red0\\green0\\blue0;}}{{\\*\\generator Msftedit;}}{{\\info{{\\title secret}}}}\
             \\pard\\f0 {hex}\\par {uni}\\tab x \\{{y\\}} \\\\z\\line\\uc2\\u-1580\\'3f\\'3fend}}"
        );
        let t = extract_text(&write_file("t.rtf", rtf.as_bytes())).unwrap();
        assert!(t.contains("مرحبا\n"), "{t:?}");
        assert!(t.contains("تقرير\tx {y} \\z\n"), "{t:?}");
        assert!(t.contains("\u{F9D4}end"), "{t:?}"); // \u-1580 == U+F9D4, two fallback chars skipped
        assert!(!t.contains("Arial"));
        assert!(!t.contains("secret"));
        assert!(!t.contains("Msftedit"));
        assert!(!t.contains('?'));
    }

    #[test]
    fn rtf_font_charset_overrides_ansicpg() {
        // Word often writes \ansicpg1252 but tags Arabic fonts with \fcharset178.
        let rtf = "{\\rtf1\\ansi\\ansicpg1252{\\fonttbl{\\f0 Times;}{\\f1\\fcharset178 Arial;}}\
                   \\f0 caf\\'e9 {\\f1 \\'e3\\'d1\\'cd\\'c8\\'c7}}";
        let t = rtf_to_text(rtf.as_bytes());
        assert!(t.contains("café"), "{t:?}");
        assert!(t.contains("مرحبا"), "{t:?}");
    }

    #[test]
    fn doc_heuristic() {
        let mut data: Vec<u8> = (0..600u32).map(|i| (i * 7 % 5) as u8).collect();
        data.extend(
            "مرحبا بالعالم\rتقرير"
                .encode_utf16()
                .flat_map(|u| u.to_le_bytes()),
        );
        data.extend([0u8, 1, 2, 3]);
        let (cp, _, _) = WINDOWS_1256.encode("نص قديم للبحث");
        data.extend(cp.iter());
        data.extend([0u8; 16]);
        let p = write_file("t.doc", &data);
        let t = extract_text(&p).unwrap();
        assert!(t.contains("مرحبا بالعالم"), "{t:?}");
        assert!(t.contains("تقرير"), "{t:?}");
        assert!(t.contains("نص قديم للبحث"), "{t:?}");
    }

    #[test]
    fn presentation_forms_normalized() {
        // "مرحبا" written with presentation forms: meem-initial, reh-final,
        // hah-initial, beh-medial, alef-final.
        let pf = "\u{FEE3}\u{FEAE}\u{FEA3}\u{FE92}\u{FE8E} \u{FEFB}";
        assert_eq!(normalize_arabic_forms(pf), "مرحبا لا");
        assert_eq!(
            normalize_arabic_forms("\u{FEF2}\u{FE80}\u{FE71}"),
            "يء\u{064B}"
        );
        assert!(matches!(
            normalize_arabic_forms("plain مرحبا"),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn fe80_table_covers_its_range() {
        let total: u32 = FE80_FORM_COUNTS.iter().map(|&n| n as u32).sum();
        assert_eq!(total, 0xFEF5 - 0xFE80);
        assert_eq!(FE80_LETTERS.chars().count(), FE80_FORM_COUNTS.len());
        assert_eq!(fe80_form(0xFE80), Some((0, 0)));
        assert_eq!(fe80_form(0xFEF4), Some((35, 3)));
        assert_eq!(fe80_form(0xFE7F), None);
        assert_eq!(fe80_form(0xFEF5), None);
    }

    #[test]
    fn runs_of_matching_items() {
        let runs = |s: &str| runs_where(s.as_bytes(), |b| b != b' ').collect::<Vec<_>>();
        assert_eq!(runs("ab  c d"), vec![0..2, 4..5, 6..7]);
        assert_eq!(runs("  "), vec![]);
        assert_eq!(runs(""), vec![]);
        assert_eq!(runs("abc"), vec![0..3]);
    }

    #[test]
    fn rtf_lexing() {
        assert_eq!(rtf_control_word(b"\\u-1576 x", 1), ("u", Some(-1576), 8));
        assert_eq!(rtf_control_word(b"par99999999999x", 0), ("par", Some(i32::MAX), 14));
        assert_eq!(rtf_control_word(b"b", 0), ("b", None, 1));
        assert_eq!(rtf_word_char("rdblquote"), Some('\u{201D}'));
        assert_eq!(rtf_word_char("fonttbl"), None);
    }

    /// Builds a minimal valid one-page PDF with correct xref offsets.
    fn tiny_pdf(text: &str) -> Vec<u8> {
        let stream = format!("BT /F1 24 Tf 72 720 Td ({text}) Tj ET");
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_owned(),
            format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_owned(),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
        }
        let xref = pdf.len();
        pdf.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
        for off in offsets {
            pdf.extend(format!("{off:010} 00000 n \n").bytes());
        }
        pdf.extend(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objs.len() + 1
            )
            .bytes(),
        );
        pdf
    }

    #[test]
    fn pdf_simple_and_garbage() {
        let p = write_file("t.pdf", &tiny_pdf("Annual Report"));
        let t = extract_text(&p).unwrap_or_default();
        assert!(t.contains("Annual") && t.contains("Report"), "{t:?}");
        // Corrupt PDFs must not panic.
        let p = write_file("bad.pdf", b"%PDF-1.4\n1 0 obj << /Type /Catalog /Pages 9 0 R >> endobj\ntrailer << /Root 1 0 R >>\n%%EOF");
        let _ = extract_text(&p);
        let p = write_file("bad2.pdf", &[0xFFu8; 300]);
        assert!(extract_text(&p).is_none());
    }

    #[test]
    fn failures_return_none() {
        assert!(extract_text(Path::new("/nonexistent/file.txt")).is_none());
        assert!(extract_text(&write_file("x.exe", b"MZ")).is_none());
        assert!(extract_text(&write_file("broken.docx", b"not a zip")).is_none());
        assert!(extract_text(&write_file("broken.xlsx", b"not a zip")).is_none());
        assert!(extract_text(&write_file("empty.txt", b"")).is_none());
        assert!(extract_text(&write_file("broken.msg", b"not a cfb")).is_none());
        assert!(extract_text(&write_file("broken.epub", b"not a zip")).is_none());
        assert!(extract_text(&write_file("broken.eml", b"\x00\x01\x02")).is_none());
    }

    #[test]
    fn eml_and_epub_from_disk() {
        let eml = write_file(
            "m.eml",
            b"Subject: =?utf-8?B?2YXYsdit2KjYpw==?=\r\n\r\nbody text\r\n",
        );
        let text = extract_text(&eml).unwrap();
        assert!(
            text.contains("مرحبا") && text.contains("body text"),
            "{text}"
        );
        let epub = write_zip("b.epub", &[("ch.xhtml", "<p>كتاب</p>")]);
        assert!(extract_text(&epub).unwrap().contains("كتاب"));
    }
}
