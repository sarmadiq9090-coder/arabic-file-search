//! Documents inside archives (zip, 7z, tar, tar.gz / tgz) for content search.
//!
//! Every member whose extension is a supported document type is read into
//! memory (nothing is written to disk) and its text extracted. Archives can
//! be hostile, so reading is bounded: at most [`MAX_MEMBERS`] entries, each
//! member at most [`MAX_MEMBER_SIZE`] uncompressed, at most
//! [`MAX_ARCHIVE_TOTAL`] decompressed per archive, no member that expands
//! more than [`MAX_RATIO`] times, and no archive inside an archive. Office
//! files (docx, xlsx, epub…) are zips too, but they are documents: they are
//! never treated as archives, only read as documents when found inside one.
//!
//! For the search, [`extract_archive`] packs the members into one string:
//! `MEMBER_MARK name \n text \n` per member, which [`split_members`] takes
//! apart again. That keeps `extract_text` (and anything that stores its
//! output) returning a single `String` per file.

use std::borrow::Cow;
use std::fs::File;
use std::io::{self, Cursor, Read};
use std::path::Path;

use calamine::{Ods, Reader, Sheets, Xls, Xlsb, Xlsx};
use encoding_rs::WINDOWS_1256;

use super::{
    MAX_OUTPUT, MAX_TEXT_READ, OTHER_EXTS, TEXT_EXTS, bidi_fix, decode_bytes, docx_text, epub,
    extract_doc_heuristic, mail, normalize_arabic_forms, odf_text, pptx_text, rtf_to_text,
    sheets_text, strip_html,
};

/// At most this many entries of an archive are looked at.
pub const MAX_MEMBERS: usize = 2_000;
/// Members larger than this (uncompressed) are skipped.
pub const MAX_MEMBER_SIZE: u64 = 50 * 1024 * 1024;
/// At most this much is decompressed from one archive.
pub const MAX_ARCHIVE_TOTAL: u64 = 300 * 1024 * 1024;
/// Members that expand more than this many times are skipped (zip bombs).
/// For 7z and tar.gz, whose members share one compressed stream, the bound
/// applies to the archive as a whole.
pub const MAX_RATIO: u64 = 200;
/// Data below this size is too small to hurt, whatever its ratio (a short
/// run of repeated characters easily compresses 200 times).
const RATIO_FLOOR: u64 = 1024 * 1024;

/// Extensions that make a file an archive. `gz` counts only as `.tar.gz`.
const ARCHIVE_EXTS: &[&str] = &["zip", "7z", "tar", "tgz", "gz"];

/// Starts each member in the text from [`extract_archive`]: a control
/// character followed by a Unicode noncharacter, neither of which can come
/// out of a document (they are removed from member texts).
const MEMBER_MARK: &str = "\u{1F}\u{FDD0}";

/// True for the (lowercase, dot-less) extensions of the archive formats
/// searched. `gz` is included because the extension check sees only the last
/// extension; [`extract_archive`] then reads only names ending in `.tar.gz`.
pub fn is_archive_ext(ext: &str) -> bool {
    ARCHIVE_EXTS.contains(&ext)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Zip,
    SevenZ,
    Tar,
    TarGz,
}

fn kind_of(path: &Path) -> Option<Kind> {
    let name = path.file_name()?.to_str()?.to_lowercase();
    let ext = name.rsplit_once('.')?.1;
    Some(match ext {
        "zip" => Kind::Zip,
        "7z" => Kind::SevenZ,
        "tar" => Kind::Tar,
        "tgz" => Kind::TarGz,
        "gz" if name.ends_with(".tar.gz") => Kind::TarGz,
        _ => return None,
    })
}

/// Bounds for reading one archive (the constants above; smaller in tests).
#[derive(Clone, Copy)]
struct Limits {
    members: usize,
    member_size: u64,
    total: u64,
    ratio: u64,
}

const LIMITS: Limits = Limits {
    members: MAX_MEMBERS,
    member_size: MAX_MEMBER_SIZE,
    total: MAX_ARCHIVE_TOTAL,
    ratio: MAX_RATIO,
};

/// `(member path, text)` for every document inside the archive at `path`
/// that yields text, in archive order. Member paths are cleaned for display
/// (`تقارير\تقرير.docx`). Empty for anything that isn't a readable archive.
/// Never panics.
#[cfg_attr(not(test), allow(dead_code))] // the search goes through `extract_archive`
pub fn archive_texts(path: &Path) -> Vec<(String, String)> {
    archive_texts_with(path, &LIMITS)
        .into_iter()
        .map(|(n, t)| (n, normalize_arabic_forms(&t).into_owned()))
        .collect()
}

fn archive_texts_with(path: &Path, lim: &Limits) -> Vec<(String, String)> {
    let Some(kind) = kind_of(path) else { return Vec::new() };
    let Ok(file) = File::open(path) else { return Vec::new() };
    let len = file.metadata().map_or(0, |m| m.len());
    let mut col = Collector::new(lim, len);
    // Third-party decoders on hostile input: contain any panic, keep what
    // was collected before it.
    let _ = crate::crash_log::quietly(|| match kind {
        Kind::Zip => read_zip(file, &mut col),
        Kind::SevenZ => read_7z(file, &mut col),
        Kind::Tar => read_tar(Capped::new(file, col.stream_cap), &mut col),
        Kind::TarGz => read_tar(Capped::new(flate2::read::GzDecoder::new(file), col.stream_cap), &mut col),
    });
    col.out
}

/// The archive's documents as one text for content search: each member as
/// `MEMBER_MARK name \n text \n`. None when no member yields text.
pub(super) fn extract_archive(path: &Path) -> Option<String> {
    let members = archive_texts_with(path, &LIMITS);
    if members.is_empty() {
        return None;
    }
    let mut out = String::new();
    for (name, text) in members {
        out.push_str(MEMBER_MARK);
        out.push_str(&name);
        out.push('\n');
        out.push_str(&text);
        out.push('\n');
    }
    Some(out)
}

/// Takes apart a text made by [`extract_archive`]: `(member path, text)`
/// per member. None for the text of an ordinary document.
pub fn split_members(text: &str) -> Option<Vec<(&str, &str)>> {
    let rest = text.strip_prefix(MEMBER_MARK)?;
    Some(
        rest.split(MEMBER_MARK)
            .map(|m| m.split_once('\n').unwrap_or((m, "")))
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// Bookkeeping shared by the formats
// ---------------------------------------------------------------------------

struct Collector<'a> {
    lim: &'a Limits,
    out: Vec<(String, String)>,
    /// Entries looked at so far.
    seen: usize,
    /// Bytes of members read so far.
    total: u64,
    /// Text collected so far.
    out_len: usize,
    /// How much a shared compressed stream (tar.gz, 7z) may expand to.
    stream_cap: u64,
}

impl<'a> Collector<'a> {
    fn new(lim: &'a Limits, archive_len: u64) -> Self {
        let stream_cap = archive_len.saturating_mul(lim.ratio).max(RATIO_FLOOR).min(lim.total);
        Collector { lim, out: Vec::new(), seen: 0, total: 0, out_len: 0, stream_cap }
    }

    /// Counts one more entry; false once the archive should not be read on.
    fn next_entry(&mut self) -> bool {
        if self.seen >= self.lim.members || self.total >= self.lim.total || self.out_len >= MAX_OUTPUT {
            return false;
        }
        self.seen += 1;
        true
    }

    /// The cleaned name and extension of a member worth reading, if its
    /// declared size is within bounds.
    fn wanted(&self, raw_name: &str, size: u64) -> Option<(String, String)> {
        if size == 0 || size > self.lim.member_size {
            return None;
        }
        let name = display_name(raw_name);
        let ext = document_ext(&name)?;
        Some((name, ext))
    }

    /// Reads a member (bounded) and keeps its text.
    fn add(&mut self, name: String, ext: &str, r: impl Read) {
        let room = self.lim.member_size.min(self.lim.total - self.total);
        let mut data = Vec::new();
        let read = r.take(room + 1).read_to_end(&mut data);
        self.total += data.len() as u64;
        if read.is_err() || data.len() as u64 > room {
            return;
        }
        if let Some(text) = member_text(ext, data) {
            let text = text.replace(MEMBER_MARK, " ").replace('\u{FDD0}', "");
            if !text.trim().is_empty() {
                self.out_len += text.len();
                self.out.push((name, text));
            }
        }
    }
}

/// A reader that fails once `left` bytes have come through: bounds how much
/// a compressed tar stream is expanded, skipped members included.
struct Capped<R> {
    inner: R,
    left: u64,
}

impl<R> Capped<R> {
    fn new(inner: R, cap: u64) -> Self {
        Capped { inner, left: cap }
    }
}

impl<R: Read> Read for Capped<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 {
            return Err(io::Error::other("archive expands too much"));
        }
        let n = buf.len().min(usize::try_from(self.left).unwrap_or(usize::MAX));
        let got = self.inner.read(&mut buf[..n])?;
        self.left -= got as u64;
        Ok(got)
    }
}

// ---------------------------------------------------------------------------
// Member names
// ---------------------------------------------------------------------------

/// The member path as shown to the user: `\` separators, no root, drive,
/// `.` or `..` parts, no control or bidi-override characters, at most 200
/// characters. Nothing is ever written to this path.
fn display_name(raw: &str) -> String {
    let clean = |c: &char| {
        !c.is_control()
            && !matches!(*c, '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FDD0}'..='\u{FDEF}')
    };
    let parts: Vec<String> = raw
        .split(['/', '\\'])
        .map(|p| p.chars().filter(clean).collect::<String>())
        .filter(|p| !p.trim().is_empty() && p != "." && p != "..")
        .enumerate()
        .filter(|(i, p)| !(*i == 0 && p.len() == 2 && p.ends_with(':')))
        .map(|(_, p)| p)
        .collect();
    let joined = parts.join("\\");
    match joined.char_indices().nth(200) {
        Some((i, _)) => format!("{}…", &joined[..i]),
        None => joined,
    }
}

/// The lowercase extension of a member that is a supported document, and
/// not an archive (no nesting) or macOS resource-fork junk.
fn document_ext(name: &str) -> Option<String> {
    if name.split('\\').any(|p| p.eq_ignore_ascii_case("__MACOSX")) {
        return None;
    }
    let file = name.rsplit('\\').next()?;
    let (stem, ext) = file.rsplit_once('.')?;
    let ext = ext.to_ascii_lowercase();
    let doc = TEXT_EXTS.contains(&ext.as_str()) || OTHER_EXTS.contains(&ext.as_str());
    (!stem.is_empty() && doc && !is_archive_ext(&ext)).then_some(ext)
}

/// Decodes a member name stored without a declared encoding: UTF-8 when the
/// bytes are valid UTF-8 (the zip UTF-8 flag, and what most tools write),
/// otherwise the Arabic DOS code page 720 (Windows' built-in zip on Arabic
/// systems) or windows-1256, whichever reads as more Arabic, and `cp437`
/// (the zip default, already decoded by the caller) when neither does.
fn decode_name<'a>(raw: &'a [u8], cp437: Cow<'a, str>) -> Cow<'a, str> {
    if let Ok(s) = std::str::from_utf8(raw) {
        return Cow::Borrowed(s);
    }
    let dos: String = raw.iter().map(|&b| cp720(b)).collect();
    let (win, _) = WINDOWS_1256.decode_without_bom_handling(raw);
    let (s_dos, s_win) = (arabic_score(&dos), arabic_score(&win));
    if s_dos <= 0 && s_win <= 0 {
        cp437
    } else if s_dos > s_win {
        Cow::Owned(dos)
    } else {
        Cow::Owned(win.into_owned())
    }
}

/// Arabic letters minus box-drawing, control and undecodable characters.
fn arabic_score(s: &str) -> i32 {
    s.chars()
        .map(|c| match c {
            '\u{0600}'..='\u{06FF}' => 1,
            '\u{2500}'..='\u{25FF}' | '\u{FFFD}' => -2,
            c if c.is_control() => -2,
            _ => 0,
        })
        .sum()
}

/// One byte of code page 720 (Arabic, "Transparent ASMO").
fn cp720(b: u8) -> char {
    const HIGH: [char; 128] = [
        // 0x80
        '\u{FFFD}', '\u{FFFD}', 'é', 'â', '\u{FFFD}', 'à', '\u{FFFD}', 'ç', 'ê', 'ë', 'è', 'ï', 'î', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}',
        // 0x90
        '\u{FFFD}', '\u{0651}', '\u{0652}', 'ô', '¤', '\u{0640}', 'û', 'ù', '\u{0621}', '\u{0622}', '\u{0623}', '\u{0624}', '£', '\u{0625}', '\u{0626}', '\u{0627}',
        // 0xA0
        '\u{0628}', '\u{0629}', '\u{062A}', '\u{062B}', '\u{062C}', '\u{062D}', '\u{062E}', '\u{062F}', '\u{0630}', '\u{0631}', '\u{0632}', '\u{0633}', '\u{0634}', '\u{0635}', '«', '»',
        // 0xB0
        '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', '╕', '╣', '║', '╗', '╝', '╜', '╛', '┐',
        // 0xC0
        '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦', '╠', '═', '╬', '╧',
        // 0xD0
        '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐', '▀',
        // 0xE0
        '\u{0636}', '\u{0637}', '\u{0638}', '\u{0639}', '\u{063A}', '\u{0641}', 'µ', '\u{0642}', '\u{0643}', '\u{0644}', '\u{0645}', '\u{0646}', '\u{0647}', '\u{0648}', '\u{0649}', '\u{064A}',
        // 0xF0
        '≡', '\u{064B}', '\u{064C}', '\u{064D}', '\u{064E}', '\u{064F}', '\u{0650}', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', '\u{00A0}',
    ];
    if b < 0x80 { b as char } else { HIGH[usize::from(b - 0x80)] }
}

// ---------------------------------------------------------------------------
// Formats
// ---------------------------------------------------------------------------

fn read_zip(file: File, col: &mut Collector) {
    let Ok(mut zip) = zip::ZipArchive::new(file) else { return };
    for i in 0..zip.len() {
        if !col.next_entry() {
            break;
        }
        let Ok(f) = zip.by_index(i) else { continue }; // e.g. encrypted
        if !f.is_file() || f.encrypted() {
            continue;
        }
        let (size, packed) = (f.size(), f.compressed_size());
        if size > RATIO_FLOOR && size > packed.saturating_mul(col.lim.ratio) {
            continue; // zip bomb
        }
        let raw_name = decode_name(f.name_raw(), f.name().unwrap_or_default()).into_owned();
        if let Some((name, ext)) = col.wanted(&raw_name, size) {
            col.add(name, &ext, f);
        }
    }
}

fn read_tar(stream: impl Read, col: &mut Collector) {
    let mut tar = tar::Archive::new(stream);
    let Ok(entries) = tar.entries() else { return };
    // An error (a corrupt header, or the stream hitting its cap) ends the
    // walk; skipped members are read through, and so count against the cap.
    for entry in entries {
        let Ok(e) = entry else { break };
        if !col.next_entry() {
            break;
        }
        if !e.header().entry_type().is_file() {
            continue;
        }
        let raw = e.path_bytes();
        let raw_name = decode_name(&raw, String::from_utf8_lossy(&raw)).into_owned();
        if let Some((name, ext)) = col.wanted(&raw_name, e.size()) {
            col.add(name, &ext, e);
        }
    }
}

fn read_7z(file: File, col: &mut Collector) {
    use sevenz_rust2::{ArchiveReader, Password};
    let Ok(mut zr) = ArchiveReader::new(file, Password::empty()) else { return };
    // In a solid archive members share one stream: a skipped member must
    // still be decoded to reach the next, so all of it counts against the cap.
    let solid = zr.archive().is_solid;
    let mut decoded: u64 = 0;
    let cap = col.stream_cap;
    let _ = zr.for_each_entries(|entry, r| {
        if !col.next_entry() || decoded >= cap {
            return Ok(false);
        }
        if entry.is_directory() || !entry.has_stream() {
            return Ok(true);
        }
        let size = entry.size();
        decoded = decoded.saturating_add(size);
        let fits = decoded <= cap;
        match col.wanted(entry.name(), size) {
            Some((name, ext)) if fits => col.add(name, &ext, r),
            _ if solid => {
                if !fits {
                    return Ok(false);
                }
                io::copy(&mut r.take(size), &mut io::sink())?;
            }
            _ => {}
        }
        Ok(true)
    });
}

// ---------------------------------------------------------------------------
// Text of one member, from memory
// ---------------------------------------------------------------------------

/// The text of a member with extension `ext` (a supported document type),
/// like `extract_by_ext` but from bytes. Never panics.
fn member_text(ext: &str, data: Vec<u8>) -> Option<String> {
    crate::crash_log::quietly(|| member_text_inner(ext, data)).ok().flatten()
}

fn member_text_inner(ext: &str, mut data: Vec<u8>) -> Option<String> {
    let zip = |data: Vec<u8>| zip::ZipArchive::new(Cursor::new(data)).ok();
    match ext {
        "html" | "htm" => Some(strip_html(&decode_bytes(&data))),
        e if TEXT_EXTS.contains(&e) => {
            data.truncate(usize::try_from(MAX_TEXT_READ).unwrap_or(usize::MAX));
            Some(decode_bytes(&data))
        }
        "docx" | "docm" => docx_text(zip(data)?),
        "pptx" => pptx_text(zip(data)?),
        "odt" | "odp" => odf_text(zip(data)?),
        "xlsx" | "xlsm" => sheets_text(Sheets::Xlsx(Xlsx::new(Cursor::new(data)).ok()?)),
        "xlsb" => sheets_text(Sheets::Xlsb(Xlsb::new(Cursor::new(data)).ok()?)),
        "xls" => sheets_text(Sheets::Xls(Xls::new(Cursor::new(data)).ok()?)),
        "ods" => sheets_text(Sheets::Ods(Ods::new(Cursor::new(data)).ok()?)),
        "pdf" => {
            let text = pdf_extract::extract_text_from_mem(&data).ok()?;
            Some(bidi_fix::fix_visual_order(text))
        }
        "rtf" => Some(rtf_to_text(&data)),
        "doc" => Some(extract_doc_heuristic(&data)),
        "eml" => mail::eml_to_text(&data),
        "msg" => mail::msg_to_text(cfb::CompoundFile::open(Cursor::new(data)).ok()?),
        "epub" => epub::epub_to_text(&mut zip(data)?),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use zip::write::SimpleFileOptions;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bahth_archive_tests_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    fn write(name: &str, bytes: &[u8]) -> PathBuf {
        let p = temp_path(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zw = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (n, data) in entries {
            if n.ends_with('/') {
                zw.add_directory(*n, SimpleFileOptions::default()).unwrap();
            } else {
                zw.start_file(*n, SimpleFileOptions::default()).unwrap();
                zw.write_all(data).unwrap();
            }
        }
        zw.finish().unwrap().into_inner()
    }

    fn docx_bytes(text: &str) -> Vec<u8> {
        let xml = format!(
            r#"<w:document xmlns:w="w"><w:body><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:body></w:document>"#
        );
        zip_bytes(&[("word/document.xml", xml.as_bytes())])
    }

    fn names(v: &[(String, String)]) -> Vec<&str> {
        v.iter().map(|(n, _)| n.as_str()).collect()
    }

    #[test]
    fn archive_extensions() {
        for e in ["zip", "7z", "tar", "tgz", "gz"] {
            assert!(is_archive_ext(e), "{e}");
            assert!(super::super::is_supported(e), "{e}");
        }
        for e in ["docx", "xlsx", "pptx", "odt", "epub", "rar", "txt"] {
            assert!(!is_archive_ext(e), "{e}");
        }
        let k = |n: &str| kind_of(Path::new(n));
        assert_eq!(k("a.ZIP"), Some(Kind::Zip));
        assert_eq!(k("a.7z"), Some(Kind::SevenZ));
        assert_eq!(k("a.tar"), Some(Kind::Tar));
        assert_eq!(k("a.tgz"), Some(Kind::TarGz));
        assert_eq!(k("نسخة.Tar.Gz"), Some(Kind::TarGz));
        assert_eq!(k("log.gz"), None);
        assert_eq!(k("a.docx"), None);
    }

    #[test]
    fn docx_and_txt_inside_zip() {
        let z = zip_bytes(&[
            ("تقارير/", b""),
            ("تقارير/تقرير.docx", &docx_bytes("تقرير المبيعات السنوي")),
            ("ملاحظات.txt", "المبيعات ارتفعت".as_bytes()),
            ("صورة.png", b"\x89PNG not a document"),
        ]);
        let p = write("docs.zip", &z);
        let v = archive_texts(&p);
        assert_eq!(names(&v), ["تقارير\\تقرير.docx", "ملاحظات.txt"]);
        assert!(v[0].1.contains("تقرير المبيعات السنوي"));
        assert_eq!(v[1].1, "المبيعات ارتفعت");

        // Through extract_text, as the content search sees it.
        let text = super::super::extract_text(&p).unwrap();
        let members = split_members(&text).unwrap();
        assert_eq!(members.len(), 2);
        assert_eq!(members[0].0, "تقارير\\تقرير.docx");
        assert!(members[0].1.contains("المبيعات"));
        assert_eq!(members[1], ("ملاحظات.txt", "المبيعات ارتفعت\n"));
    }

    #[test]
    fn office_files_are_not_archives() {
        let p = write("plain.docx", &docx_bytes("نص عادي"));
        let text = super::super::extract_text(&p).unwrap();
        assert!(split_members(&text).is_none());
        assert!(archive_texts(&p).is_empty());
    }

    #[test]
    fn nested_archives_skipped() {
        let inner = zip_bytes(&[("داخلي.txt", "سر دفين".as_bytes())]);
        let z = zip_bytes(&[
            ("inner.zip", &inner),
            ("inner.tar.gz", b"whatever"),
            ("a.7z", b"7z"),
            ("نص.txt", "ظاهر".as_bytes()),
        ]);
        let v = archive_texts(&write("nested.zip", &z));
        assert_eq!(names(&v), ["نص.txt"]);
    }

    #[test]
    fn zip_bomb_ratio_skipped() {
        let spaces = vec![b' '; 8 * 1024 * 1024];
        let z = zip_bytes(&[("bomb.txt", &spaces), ("ok.txt", b"fine text")]);
        assert!(z.len() < 100_000, "highly compressible");
        let v = archive_texts(&write("bomb.zip", &z));
        assert_eq!(names(&v), ["ok.txt"]);
        // Small members are never ratio-checked.
        let small = zip_bytes(&[("small.txt", &[b'a'; 100_000])]);
        assert_eq!(archive_texts(&write("small.zip", &small)).len(), 1);
    }

    #[test]
    fn member_and_total_limits() {
        let entries: Vec<(String, Vec<u8>)> =
            (0..2_005).map(|i| (format!("f{i}.txt"), format!("نص {i}").into_bytes())).collect();
        let refs: Vec<(&str, &[u8])> = entries.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
        let p = write("many.zip", &zip_bytes(&refs));
        let v = archive_texts(&p);
        assert_eq!(v.len(), MAX_MEMBERS);
        assert_eq!(v.last().unwrap().0, "f1999.txt");

        // Smaller limits for the size bounds.
        let lim = Limits { members: 100, member_size: 1000, total: 2500, ratio: MAX_RATIO };
        let big = vec![b'x'; 1500];
        let mid = vec![b'y'; 900];
        let z = zip_bytes(&[("big.txt", &big), ("a.txt", &mid), ("b.txt", &mid), ("c.txt", &mid), ("d.txt", &mid)]);
        let v = archive_texts_with(&write("limits.zip", &z), &lim);
        // big is over the member size; after a and b the total has room
        // for only 700 bytes, so c is dropped and reading stops.
        assert_eq!(names(&v), ["a.txt", "b.txt"]);
        let lim = Limits { members: 2, ..lim };
        assert_eq!(names(&archive_texts_with(&write("limits2.zip", &z), &lim)), ["a.txt"]);
    }

    /// A stored zip written by hand, to control the raw name bytes.
    fn raw_zip(name: &[u8], data: &[u8]) -> Vec<u8> {
        let crc = {
            let mut c = !0u32;
            for &b in data {
                c ^= u32::from(b);
                for _ in 0..8 {
                    c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
                }
            }
            !c
        };
        let (n, d) = (name.len() as u16, data.len() as u32);
        let mut out = Vec::new();
        let common = |v: &mut Vec<u8>| {
            v.extend(0u16.to_le_bytes()); // flags: no UTF-8 bit
            v.extend(0u16.to_le_bytes()); // stored
            v.extend(0u32.to_le_bytes()); // time, date
            v.extend(crc.to_le_bytes());
            v.extend(d.to_le_bytes());
            v.extend(d.to_le_bytes());
            v.extend(n.to_le_bytes());
            v.extend(0u16.to_le_bytes()); // extra
        };
        out.extend(0x0403_4b50u32.to_le_bytes());
        out.extend(20u16.to_le_bytes());
        common(&mut out);
        out.extend(name);
        out.extend(data);
        let cd = out.len() as u32;
        out.extend(0x0201_4b50u32.to_le_bytes());
        out.extend(20u16.to_le_bytes());
        out.extend(20u16.to_le_bytes());
        common(&mut out);
        out.extend([0u8; 14]); // comment len, disk, attributes, local header offset 0
        out.extend(name);
        let cd_len = out.len() as u32 - cd;
        out.extend(0x0605_4b50u32.to_le_bytes());
        out.extend([0u8; 4]);
        out.extend(1u16.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(cd_len.to_le_bytes());
        out.extend(cd.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out
    }

    #[test]
    fn arabic_member_names() {
        // UTF-8 (written by the zip crate with the UTF-8 flag).
        let z = zip_bytes(&[("مجلد/عقد.txt", b"text")]);
        assert_eq!(names(&archive_texts(&write("utf8.zip", &z))), ["مجلد\\عقد.txt"]);
        // CP720, as Windows' own zip writes on Arabic systems: "تقرير".
        let z = raw_zip(b"\xA2\xE7\xA9\xEF\xA9.txt", b"text");
        assert_eq!(names(&archive_texts(&write("cp720.zip", &z))), ["تقرير.txt"]);
        // windows-1256: "تقرير".
        let z = raw_zip(b"\xCA\xDE\xD1\xED\xD1.txt", b"text");
        assert_eq!(names(&archive_texts(&write("cp1256.zip", &z))), ["تقرير.txt"]);
        // Western names stay CP437.
        let z = raw_zip(b"caf\x82.txt", b"text");
        assert_eq!(names(&archive_texts(&write("cp437.zip", &z))), ["café.txt"]);
    }

    #[test]
    fn names_cleaned_for_display() {
        assert_eq!(display_name("../../etc/passwd.txt"), "etc\\passwd.txt");
        assert_eq!(display_name("/abs//./a.txt"), "abs\\a.txt");
        assert_eq!(display_name("C:\\Windows\\x.txt"), "Windows\\x.txt");
        assert_eq!(display_name("a\u{202E}txt.exe\n.txt"), "atxt.exe.txt");
        assert_eq!(display_name(&"ب".repeat(300)).chars().count(), 201);
        assert_eq!(document_ext("__MACOSX\\._a.txt"), None);
        assert_eq!(document_ext(".txt"), None);
        assert_eq!(document_ext("A.TXT").as_deref(), Some("txt"));
    }

    fn tar_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        for (n, data) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, n, *data).unwrap();
        }
        b.into_inner().unwrap()
    }

    #[test]
    fn tar_and_tar_gz() {
        let t = tar_bytes(&[
            ("مستندات/تقرير.docx", &docx_bytes("أرباح الربع")),
            ("نص.txt", "المبيعات".as_bytes()),
            ("inner.zip", &zip_bytes(&[("x.txt", b"hidden")])),
        ]);
        let v = archive_texts(&write("a.tar", &t));
        assert_eq!(names(&v), ["مستندات\\تقرير.docx", "نص.txt"]);
        assert!(v[0].1.contains("أرباح الربع"));

        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&t).unwrap();
        let gz = gz.finish().unwrap();
        for n in ["a.tar.gz", "a.tgz"] {
            assert_eq!(names(&archive_texts(&write(n, &gz))), ["مستندات\\تقرير.docx", "نص.txt"], "{n}");
        }
        // A .gz that is not a tarball is ignored.
        assert!(super::super::extract_text(&write("log.gz", &gz)).is_none());
    }

    #[test]
    fn tar_gz_bomb_stops() {
        // 20 MB of spaces in one member: the stream may expand only 200 times
        // the archive (at least 1 MB), so the walk stops inside the bomb.
        let t = tar_bytes(&[("bomb.txt", &vec![b' '; 20 * 1024 * 1024]), ("after.txt", b"after")]);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        gz.write_all(&t).unwrap();
        let gz = gz.finish().unwrap();
        assert!((gz.len() as u64) * MAX_RATIO < 20 * 1024 * 1024);
        assert!(archive_texts(&write("bomb.tar.gz", &gz)).is_empty());
    }

    fn sevenz(path: &Path, entries: &[(&str, &[u8])], solid: bool) {
        use sevenz_rust2::{ArchiveEntry, ArchiveWriter, SourceReader};
        let mut w = ArchiveWriter::create(path).unwrap();
        if solid {
            let es = entries.iter().map(|(n, _)| ArchiveEntry::new_file(n)).collect();
            let rs = entries.iter().map(|(_, d)| SourceReader::new(*d)).collect();
            w.push_archive_entries(es, rs).unwrap();
        } else {
            for (n, d) in entries {
                w.push_archive_entry(ArchiveEntry::new_file(n), Some(*d)).unwrap();
            }
        }
        w.finish().unwrap();
    }

    #[test]
    fn seven_zip() {
        let docx = docx_bytes("الميزانية");
        let entries: [(&str, &[u8]); 4] = [
            ("تقارير/ميزانية.docx", &docx),
            ("big.txt", &[b'z'; 3000]),
            ("nested.zip", b"PK"),
            ("ملاحظة.txt", "المبيعات".as_bytes()),
        ];
        for solid in [false, true] {
            let p = temp_path(&format!("a{solid}.7z"));
            sevenz(&p, &entries, solid);
            let v = archive_texts(&p);
            assert_eq!(names(&v), ["تقارير\\ميزانية.docx", "big.txt", "ملاحظة.txt"], "solid {solid}");
            assert!(v[0].1.contains("الميزانية"));
            assert_eq!(v[2].1, "المبيعات");
            // A member over the size limit is skipped, the ones after it
            // (decoded through, in a solid block) still read.
            let lim = Limits { member_size: 2000, ..LIMITS };
            assert_eq!(names(&archive_texts_with(&p, &lim)), ["تقارير\\ميزانية.docx", "ملاحظة.txt"], "solid {solid}");
        }
    }

    #[test]
    fn garbage_and_missing() {
        assert!(archive_texts(&write("junk.zip", b"not a zip")).is_empty());
        assert!(archive_texts(&write("junk.7z", b"7z\xBC\xAF\x27\x1C junk")).is_empty());
        assert!(archive_texts(&write("junk.tar.gz", b"\x1f\x8b junk")).is_empty());
        assert!(archive_texts(Path::new("/no/such/file.zip")).is_empty());
        assert!(super::super::extract_text(&write("empty.zip", &zip_bytes(&[]))).is_none());
    }

    #[test]
    fn mark_cannot_be_forged() {
        let evil = format!("a{MEMBER_MARK}fake.txt\nb");
        let p = write("forge.zip", &zip_bytes(&[("x.txt", evil.as_bytes())]));
        let text = super::super::extract_text(&p).unwrap();
        let m = split_members(&text).unwrap();
        assert_eq!(m, [("x.txt", "a fake.txt\nb\n")]);
        assert!(split_members("plain text").is_none());
    }
}
