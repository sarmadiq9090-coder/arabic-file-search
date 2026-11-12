//! Find and replace across plain-text files, with a backup and an Undo.
//!
//! Only the plain-text formats that content search reads as-is are touched
//! (see [`crate::extract::is_plain_text`]); documents such as docx or pdf
//! never are. Safety comes first:
//! - The encoding is detected (UTF-8 with or without BOM, UTF-16 LE/BE with
//!   a BOM, else windows-1256) and kept, BOM included. A file is only
//!   edited when decoding and encoding it again gives back the very same
//!   bytes, so nothing but the matches can change; line endings and every
//!   other byte stay as they were.
//! - Files over 20 MB, read-only files, links and anything binary are
//!   skipped and reported.
//! - Before a file is written its original bytes are copied into the
//!   operation's backup folder, and the folder's `index.json` records it.
//!   The new content goes to a temporary file in the same folder that then
//!   replaces the original in one step (`ReplaceFileW` on Windows, which
//!   keeps the original's attributes, ACL and alternate streams; else a
//!   rename). Hard links to the original are not kept: they go on pointing
//!   at the old content.
//! - Undo puts a file back only while it still holds exactly what the
//!   replace wrote (checked by SHA-256), so later edits are never lost.

// Parts are only used by the Windows UI.
#![cfg_attr(not(windows), allow(dead_code))]

use std::borrow::Cow;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use encoding_rs::{UTF_16BE, UTF_16LE, WINDOWS_1256};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Bigger files are skipped.
pub const MAX_FILE_SIZE: u64 = 20 * 1024 * 1024;
/// Before/after lines shown per file in the preview.
const SAMPLE_LINES: usize = 3;
/// Characters of a sample line shown around its first match.
const SAMPLE_CHARS: usize = 140;
/// Older backup folders are deleted.
const KEEP_OPERATIONS: usize = 10;
const INDEX_FILE: &str = "index.json";
const UNDONE_FILE: &str = "index.undone.json";

// ---------------------------------------------------------------- encodings

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Windows1256,
}

impl TextEncoding {
    pub fn label(self) -> &'static str {
        match self {
            TextEncoding::Utf8 => "UTF-8",
            TextEncoding::Utf8Bom => "UTF-8 BOM",
            TextEncoding::Utf16Le => "UTF-16 LE",
            TextEncoding::Utf16Be => "UTF-16 BE",
            TextEncoding::Windows1256 => "Windows-1256",
        }
    }
}

/// Decodes a text file's bytes. None for anything that doesn't decode
/// cleanly (malformed UTF-16, zero bytes without a UTF-16 BOM, …).
pub fn decode(bytes: &[u8]) -> Option<(String, TextEncoding)> {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return Some((std::str::from_utf8(rest).ok()?.to_owned(), TextEncoding::Utf8Bom));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let text = UTF_16LE.decode_without_bom_handling_and_without_replacement(rest)?;
        return Some((text.into_owned(), TextEncoding::Utf16Le));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let text = UTF_16BE.decode_without_bom_handling_and_without_replacement(rest)?;
        return Some((text.into_owned(), TextEncoding::Utf16Be));
    }
    // A zero byte: binary, or UTF-16 without a BOM. Neither is safe to edit.
    if bytes.contains(&0) {
        return None;
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Some((s.to_owned(), TextEncoding::Utf8));
    }
    let text = WINDOWS_1256.decode_without_bom_handling_and_without_replacement(bytes)?;
    Some((text.into_owned(), TextEncoding::Windows1256))
}

/// Encodes text back, BOM included. None when a character has no place in
/// the encoding (e.g. an emoji in a windows-1256 file).
pub fn encode(text: &str, enc: TextEncoding) -> Option<Vec<u8>> {
    Some(match enc {
        TextEncoding::Utf8 => text.as_bytes().to_vec(),
        TextEncoding::Utf8Bom => [&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat(),
        TextEncoding::Utf16Le => {
            let mut out = vec![0xFF, 0xFE];
            out.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
            out
        }
        TextEncoding::Utf16Be => {
            let mut out = vec![0xFE, 0xFF];
            out.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
            out
        }
        TextEncoding::Windows1256 => {
            let (bytes, _, had_errors) = WINDOWS_1256.encode(text);
            if had_errors {
                return None;
            }
            bytes.into_owned()
        }
    })
}

// ---------------------------------------------------------------- matching

/// What to find and what to put instead.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub find: String,
    pub replace: String,
    pub ignore_case: bool,
    /// `find` is a regular expression and `replace` may use `$1`, `${name}`.
    pub regex: bool,
}

/// The replacement rewritten so that the regex crate reads it exactly as a
/// person means it, or Err with a message for the user.
///
/// The crate reads a group name as long as letters, digits and `_` follow
/// the `$`, so `$1em` is the group "1em" — which does not exist, and an
/// unknown group expands to nothing: the matched text would silently
/// vanish. Here `$` and digits name the numbered group (the longest run of
/// those digits that is a group of the pattern; what follows is text), so
/// `$1em` is `${1}em` and `$1_new` is `${1}_new`. A name that starts with a
/// letter or `_` must be a named group of the pattern, and `${…}` an
/// existing group; anything else is refused. `$$` is a `$` sign, and a `$`
/// not followed by a name stays a `$`.
fn group_template(replace: &str, re: &Regex) -> Result<String, String> {
    let groups = re.captures_len();
    let named: Vec<&str> = re.capture_names().flatten().collect();
    let exists = |g: &str| match g.parse::<usize>() {
        Ok(n) => g.bytes().all(|b| b.is_ascii_digit()) && n < groups,
        Err(_) => named.contains(&g),
    };
    let missing = |typed: &str| {
        trf!(
            "المجموعة «{}» غير موجودة في التعبير النمطي، فلن يُكتب مكانها شيء. صحّحها، أو اكتب «$$» لكتابة علامة $ نفسها.",
            "The group “{}” is not in the regular expression, so nothing would be written in its place. Fix it, or write “$$” for a $ sign itself.",
            typed
        )
    };
    let is_name = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut out = String::with_capacity(replace.len() + 8);
    let mut rest = replace;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        if let Some(tail) = after.strip_prefix('$') {
            out.push_str("$$");
            rest = tail;
        } else if let Some(braced) = after.strip_prefix('{').and_then(|b| Some((b, b.find('}')?))) {
            let (name, tail) = (&braced.0[..braced.1], &braced.0[braced.1 + 1..]);
            if !exists(name) {
                return Err(missing(&format!("${{{name}}}")));
            }
            out.push_str(&format!("${{{name}}}"));
            rest = tail;
        } else {
            let end = after.find(|c: char| !is_name(c)).unwrap_or(after.len());
            let name = &after[..end];
            if name.is_empty() {
                // `$` before a space, a sign or the end: the crate keeps it.
                out.push_str("$$");
            } else if name.starts_with(|c: char| c.is_ascii_digit()) {
                let digits = name.find(|c: char| !c.is_ascii_digit()).unwrap_or(name.len());
                let Some(n) = (1..=digits).rev().find(|&n| exists(&name[..n])) else {
                    return Err(missing(&format!("${}", &name[..digits])));
                };
                out.push_str(&format!("${{{}}}", &name[..n]));
                out.push_str(&name[n..]);
            } else if exists(name) {
                out.push_str(&format!("${{{name}}}"));
            } else {
                return Err(missing(&format!("${name}")));
            }
            rest = &after[end..];
        }
    }
    out.push_str(rest);
    Ok(out)
}

pub struct Matcher {
    re: Regex,
    replace: String,
    expand: bool,
}

impl Matcher {
    /// Err with a message for the user when `find` is empty or not a valid
    /// regular expression.
    pub fn new(o: &Options) -> Result<Matcher, String> {
        if o.find.is_empty() {
            return Err(tr!("اكتب النص المراد البحث عنه.", "Type the text to find.").to_string());
        }
        let pattern = if o.regex { o.find.clone() } else { regex::escape(&o.find) };
        let re = RegexBuilder::new(&pattern)
            .case_insensitive(o.ignore_case)
            .build()
            .map_err(|e| trf!("التعبير النمطي غير صالح: {e}", "Invalid regular expression: {e}"))?;
        let replace = if o.regex { group_template(&o.replace, &re)? } else { o.replace.clone() };
        Ok(Matcher { re, replace, expand: o.regex })
    }

    /// Non-empty matches only: a pattern like `a*` must not insert text
    /// between every two characters.
    fn matches<'t>(&'t self, text: &'t str) -> impl Iterator<Item = regex::Captures<'t>> + 't {
        self.re.captures_iter(text).filter(|c| c.get(0).is_some_and(|m| !m.is_empty()))
    }

    pub fn count(&self, text: &str) -> usize {
        self.matches(text).count()
    }

    /// The text with every match replaced, and how many there were.
    pub fn replace_all<'t>(&self, text: &'t str) -> (Cow<'t, str>, usize) {
        let mut out = String::new();
        let mut last = 0;
        let mut n = 0;
        for caps in self.matches(text) {
            let m = caps.get(0).expect("group 0 always matches");
            out.push_str(&text[last..m.start()]);
            if self.expand {
                caps.expand(&self.replace, &mut out);
            } else {
                out.push_str(&self.replace);
            }
            last = m.end();
            n += 1;
        }
        if n == 0 {
            return (Cow::Borrowed(text), 0);
        }
        out.push_str(&text[last..]);
        (Cow::Owned(out), n)
    }
}

// ---------------------------------------------------------------- files

/// Why a file was left alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Skip {
    /// Not one of the plain-text extensions.
    NotText,
    TooBig,
    ReadOnly,
    /// A symbolic link or something other than a regular file.
    NotRegular,
    /// Zero bytes or malformed text: binary or an unknown encoding.
    Binary,
    /// Decoding and re-encoding would not give back the same bytes.
    Lossy,
    /// The replacement has characters the file's encoding can't hold.
    Unencodable,
    /// The file changed while it was being processed.
    Changed,
    Io(String),
}

impl Skip {
    pub fn reason(&self) -> String {
        match self {
            Skip::NotText => tr!("ليس ملفاً نصياً عادياً", "Not a plain text file").into(),
            Skip::TooBig => tr!("أكبر من ٢٠ ميغابايت", "Larger than 20 MB").into(),
            Skip::ReadOnly => tr!("للقراءة فقط", "Read-only").into(),
            Skip::NotRegular => tr!("ليس ملفاً عادياً (اختصار أو رابط)", "Not a regular file (link)").into(),
            Skip::Binary => tr!("ترميز غير معروف أو محتوى ثنائي", "Unknown encoding or binary content").into(),
            Skip::Lossy => tr!("لا يمكن حفظه دون تغيير بايتات أخرى", "Can't be saved without changing other bytes").into(),
            Skip::Unencodable => {
                tr!("نص الاستبدال يحوي أحرفاً لا يدعمها ترميز الملف", "The replacement has characters the file's encoding can't hold").into()
            }
            Skip::Changed => tr!("تغيّر الملف أثناء العمل", "The file changed meanwhile").into(),
            Skip::Io(e) => e.clone(),
        }
    }
}

/// A file read and checked, ready to be edited.
struct Loaded {
    bytes: Vec<u8>,
    text: String,
    enc: TextEncoding,
    stamp: (u64, Option<SystemTime>),
}

fn io_err(e: std::io::Error) -> Skip {
    Skip::Io(e.to_string())
}

fn file_stamp(meta: &fs::Metadata) -> (u64, Option<SystemTime>) {
    (meta.len(), meta.modified().ok())
}

fn is_plain_text_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| crate::extract::is_plain_text(&e.to_ascii_lowercase()))
}

fn load(path: &Path) -> Result<Loaded, Skip> {
    if !is_plain_text_path(path) {
        return Err(Skip::NotText);
    }
    let meta = fs::symlink_metadata(path).map_err(io_err)?;
    if !meta.file_type().is_file() {
        return Err(Skip::NotRegular);
    }
    if meta.len() > MAX_FILE_SIZE {
        return Err(Skip::TooBig);
    }
    if meta.permissions().readonly() {
        return Err(Skip::ReadOnly);
    }
    let bytes = fs::read(path).map_err(io_err)?;
    if bytes.len() as u64 > MAX_FILE_SIZE {
        return Err(Skip::TooBig);
    }
    let (text, enc) = decode(&bytes).ok_or(Skip::Binary)?;
    if encode(&text, enc).as_deref() != Some(&bytes[..]) {
        return Err(Skip::Lossy);
    }
    Ok(Loaded { bytes, text, enc, stamp: file_stamp(&meta) })
}

/// One line before and after the replace, for the preview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sample {
    /// 1-based.
    pub line: usize,
    pub before: String,
    pub after: String,
}

#[derive(Clone, Debug)]
pub struct FilePreview {
    pub count: usize,
    pub encoding: TextEncoding,
    pub samples: Vec<Sample>,
}

#[derive(Clone, Debug)]
pub enum Outcome {
    Matches(FilePreview),
    NoMatch,
    Skipped(Skip),
}

/// `s` cut to about [`SAMPLE_CHARS`] characters, starting a little before
/// the character at byte `at`.
fn clip(s: &str, at: usize) -> String {
    // `at` comes from the original line; in the replaced copy it may fall
    // inside a letter, so step back to the start of that letter.
    let mut at = at.min(s.len());
    while !s.is_char_boundary(at) {
        at -= 1;
    }
    let lead = s[..at].chars().count().saturating_sub(30);
    let mut out: String = if lead > 0 { "…".into() } else { String::new() };
    let mut chars = s.chars().skip(lead);
    out.extend(chars.by_ref().take(SAMPLE_CHARS));
    if chars.next().is_some() {
        out.push('…');
    }
    out.replace('\t', " ")
}

fn samples(text: &str, m: &Matcher) -> Vec<Sample> {
    let mut out: Vec<Sample> = Vec::new();
    let mut last_start = usize::MAX;
    let (mut counted_to, mut line_no) = (0, 1);
    for caps in m.matches(text) {
        let start = caps.get(0).expect("group 0").start();
        let ls = text[..start].rfind('\n').map_or(0, |i| i + 1);
        if ls == last_start {
            continue;
        }
        if out.len() == SAMPLE_LINES {
            break;
        }
        last_start = ls;
        line_no += text[counted_to..ls].matches('\n').count();
        counted_to = ls;
        let le = text[ls..].find('\n').map_or(text.len(), |i| ls + i);
        let line = text[ls..le].trim_end_matches('\r');
        let (after, _) = m.replace_all(line);
        let at = start - ls;
        out.push(Sample { line: line_no, before: clip(line, at), after: clip(&after, at) });
    }
    out
}

/// Counts the matches in one file, with a few before/after lines.
pub fn preview_file(path: &Path, m: &Matcher) -> Outcome {
    let f = match load(path) {
        Ok(f) => f,
        Err(s) => return Outcome::Skipped(s),
    };
    let count = m.count(&f.text);
    if count == 0 {
        return Outcome::NoMatch;
    }
    let (after, _) = m.replace_all(&f.text);
    if encode(&after, f.enc).is_none() {
        return Outcome::Skipped(Skip::Unencodable);
    }
    Outcome::Matches(FilePreview { count, encoding: f.enc, samples: samples(&f.text, m) })
}

// ---------------------------------------------------------------- backups

/// One operation's record in its backup folder.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OpIndex {
    pub version: u32,
    /// Unix seconds.
    pub created: i64,
    pub find: String,
    pub replace: String,
    pub files: Vec<OpFile>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpFile {
    pub path: String,
    /// File name of the copy inside the backup folder.
    pub backup: String,
    pub original_sha256: String,
    pub new_sha256: String,
    pub count: usize,
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Writes `bytes` to a temporary file next to `path`, then renames it over
/// `path`, so the file is either the old or the new content, never half.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.bahth-tmp", std::process::id()));
    let result = (|| {
        let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        if let Ok(meta) = fs::metadata(path) {
            let _ = fs::set_permissions(&tmp, meta.permissions());
        }
        replace_with(&tmp, path)
    })();
    if result.is_err() {
        // `ReplaceFileW` can fail half-way with the original already gone
        // (ERROR_UNABLE_TO_MOVE_REPLACEMENT); then the temporary file holds
        // the only copy of the content and must stay.
        if fs::symlink_metadata(path).is_ok() || !tmp.exists() {
            let _ = fs::remove_file(&tmp);
        }
    }
    result
}

/// Puts `tmp` in the place of `path`. On Windows `ReplaceFileW` keeps what
/// belongs to the file being replaced (Hidden/System and other attributes,
/// its ACL, object id, alternate data streams), which a rename would swap
/// for the temp file's. It falls back to a rename where that fails (no file
/// there yet, file systems or shares without support): with no backup name
/// given, every failure of `ReplaceFileW` leaves `tmp` in place. Hard links
/// to the old file are lost either way.
fn replace_with(tmp: &Path, path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use winapi::um::winbase::REPLACEFILE_IGNORE_MERGE_ERRORS;
        // Declared here: winapi 0.3's binding leaves out the BOOL result.
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn ReplaceFileW(
                replaced: *const u16,
                replacement: *const u16,
                backup: *const u16,
                flags: u32,
                exclude: *mut std::ffi::c_void,
                reserved: *mut std::ffi::c_void,
            ) -> i32;
        }
        let wide = |p: &Path| p.as_os_str().encode_wide().chain([0]).collect::<Vec<u16>>();
        let (to, from) = (wide(path), wide(tmp));
        if path.exists() {
            // SAFETY: both strings are NUL-terminated and outlive the call;
            // the optional pointers are null.
            let ok = unsafe {
                ReplaceFileW(
                    to.as_ptr(),
                    from.as_ptr(),
                    std::ptr::null(),
                    REPLACEFILE_IGNORE_MERGE_ERRORS,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if ok != 0 {
                return Ok(());
            }
        }
    }
    fs::rename(tmp, path)
}

fn write_index(dir: &Path, index: &OpIndex) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(index).map_err(std::io::Error::other)?;
    write_atomic(&dir.join(INDEX_FILE), &json)
}

/// `YYYYMMDD-HHMMSS` of Unix seconds shifted by `offset` seconds.
fn stamp(secs: i64, offset: i32) -> String {
    let t = secs.saturating_add(offset as i64);
    let (days, rem) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    // Civil date from a day number (H. Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", rem / 3600, rem / 60 % 60, rem % 60)
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// A backup folder's name: `s<sequence>-<UTC time>Z`. The sequence, one
/// more than the highest one already there, orders the operations, so a
/// clock or daylight-saving change can't make an older one look newest.
fn op_dir_name(seq: u64, secs: i64) -> String {
    format!("s{seq:08}-{}Z", stamp(secs, 0))
}

/// Where a backup folder sorts: folders of the old `YYYYMMDD-HHMMSS[-NNN]`
/// (local time) format, by name, come before every sequenced one. `None`
/// for a folder that is not a backup.
fn op_dir_key(name: &str) -> Option<(Option<u64>, String)> {
    if let Some(rest) = name.strip_prefix('s') {
        let (seq, time) = rest.split_once('-')?;
        if !seq.bytes().all(|b| b.is_ascii_digit()) || !time.ends_with('Z') {
            return None;
        }
        return Some((Some(seq.parse().ok()?), String::new()));
    }
    (name.len() >= 15 && name.chars().all(|c| c.is_ascii_digit() || c == '-')).then(|| (None, name.to_string()))
}

/// A new, empty backup folder under `root` (see [`op_dir_name`]).
fn new_op_dir(root: &Path, secs: i64) -> std::io::Result<PathBuf> {
    fs::create_dir_all(root)?;
    let mut seq = op_dir_keys(root).into_iter().filter_map(|(k, _)| k.0).max().unwrap_or(0);
    for _ in 0..1000 {
        // A folder named with the largest number must not overflow.
        seq = seq.checked_add(1).ok_or_else(|| std::io::Error::other("no free backup folder name"))?;
        let dir = root.join(op_dir_name(seq, secs));
        match fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("no free backup folder name"))
}

/// Backup folders under `root` with their sort keys, unordered.
fn op_dir_keys(root: &Path) -> Vec<((Option<u64>, String), PathBuf)> {
    fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| Some((op_dir_key(&e.file_name().to_string_lossy())?, e.path())))
        .collect()
}

/// Backup folders under `root`, newest first.
fn op_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs = op_dir_keys(root);
    dirs.sort_by(|a, b| b.0.cmp(&a.0));
    dirs.into_iter().map(|(_, p)| p).collect()
}

fn prune(root: &Path) {
    for old in op_dirs(root).into_iter().skip(KEEP_OPERATIONS) {
        let _ = fs::remove_dir_all(old);
    }
}

/// The newest operation, if it can still be undone.
pub fn last_operation(root: &Path) -> Option<(PathBuf, OpIndex)> {
    let dir = op_dirs(root).into_iter().next()?;
    let index = serde_json::from_slice(&fs::read(dir.join(INDEX_FILE)).ok()?).ok()?;
    Some((dir, index))
}

// ---------------------------------------------------------------- apply & undo

#[derive(Debug, Default)]
pub struct ApplyReport {
    /// The backup folder, when any file was changed.
    pub dir: Option<PathBuf>,
    /// (path, replacements).
    pub done: Vec<(String, usize)>,
    pub skipped: Vec<(String, Skip)>,
    /// Files without a match any more.
    pub unchanged: usize,
}

/// Replaces in every file of `paths`, backing each one up under `root`
/// first. Each file is read and checked again, as it may have changed since
/// the preview. `progress` is called with the number of files done.
pub fn apply(paths: &[String], m: &Matcher, opts: &Options, root: &Path, progress: &dyn Fn(usize)) -> Result<ApplyReport, String> {
    let created = now_secs();
    let dir = new_op_dir(root, created)
        .map_err(|e| trf!("تعذر إنشاء مجلد النسخ الاحتياطية: {e}", "Could not create the backup folder: {e}"))?;
    let mut index = OpIndex { version: 1, created, find: opts.find.clone(), replace: opts.replace.clone(), files: Vec::new() };
    let mut report = ApplyReport::default();
    let mut seen = std::collections::HashSet::new();
    for (i, p) in paths.iter().enumerate() {
        progress(i);
        // The same file twice would be replaced twice (and backed up
        // half-done the second time).
        if !seen.insert(p.as_str()) {
            continue;
        }
        match apply_one(Path::new(p), m, &dir, &mut index) {
            Ok(0) => report.unchanged += 1,
            Ok(n) => report.done.push((p.clone(), n)),
            Err(s) => report.skipped.push((p.clone(), s)),
        }
    }
    progress(paths.len());
    if index.files.is_empty() {
        let _ = fs::remove_dir_all(&dir);
    } else {
        report.dir = Some(dir);
        prune(root);
    }
    Ok(report)
}

fn apply_one(path: &Path, m: &Matcher, dir: &Path, index: &mut OpIndex) -> Result<usize, Skip> {
    let f = load(path)?;
    let (after, n) = m.replace_all(&f.text);
    if n == 0 {
        return Ok(0);
    }
    let new_bytes = encode(&after, f.enc).ok_or(Skip::Unencodable)?;
    // Back up first, and record it, so an interrupted run can be undone.
    let backup = format!("{:04}.bak", index.files.len() + 1);
    let mut b = fs::File::create(dir.join(&backup)).map_err(io_err)?;
    b.write_all(&f.bytes).and_then(|_| b.sync_all()).map_err(io_err)?;
    drop(b);
    index.files.push(OpFile {
        path: path.to_string_lossy().into_owned(),
        backup: backup.clone(),
        original_sha256: sha256_hex(&f.bytes),
        new_sha256: sha256_hex(&new_bytes),
        count: n,
    });
    let forget = |index: &mut OpIndex| {
        index.files.pop();
        let _ = fs::remove_file(dir.join(&backup));
        let _ = write_index(dir, index);
    };
    if let Err(e) = write_index(dir, index) {
        forget(index);
        return Err(io_err(e));
    }
    let same = fs::symlink_metadata(path).is_ok_and(|meta| file_stamp(&meta) == f.stamp);
    if !same {
        forget(index);
        return Err(Skip::Changed);
    }
    if let Err(e) = write_atomic(path, &new_bytes) {
        // The backup is only dropped while the original is still intact;
        // otherwise it is kept, and recorded, so nothing is lost.
        if fs::read(path).is_ok_and(|b| b == f.bytes) {
            forget(index);
        }
        return Err(io_err(e));
    }
    Ok(n)
}

#[derive(Debug, Default)]
pub struct UndoReport {
    pub restored: Vec<String>,
    /// Already back to the original content.
    pub already: Vec<String>,
    /// Changed since the replace (or gone): left as they are.
    pub conflicts: Vec<String>,
    pub failed: Vec<(String, String)>,
}

/// Puts back the files of the operation in `dir`, then marks it undone.
pub fn undo(dir: &Path) -> Result<UndoReport, String> {
    let index: OpIndex = fs::read(dir.join(INDEX_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| tr!("لا توجد عملية استبدال يمكن التراجع عنها.", "There is no replace to undo.").to_string())?;
    let mut report = UndoReport::default();
    // Newest first: if one file was changed twice, the later change is
    // undone before the earlier one, back to the very first content.
    for f in index.files.iter().rev() {
        let path = Path::new(&f.path);
        // Backups are plain names inside the folder, never a path elsewhere.
        if f.backup.is_empty() || f.backup.contains(['/', '\\', ':']) || f.backup.starts_with('.') {
            report.failed.push((f.path.clone(), tr!("النسخة الاحتياطية تالفة", "The backup is damaged").into()));
            continue;
        }
        let Ok(current) = fs::read(path) else {
            report.conflicts.push(f.path.clone());
            continue;
        };
        let hash = sha256_hex(&current);
        if hash == f.original_sha256 {
            report.already.push(f.path.clone());
            continue;
        }
        if hash != f.new_sha256 {
            report.conflicts.push(f.path.clone());
            continue;
        }
        let backup = match fs::read(dir.join(&f.backup)) {
            Ok(b) if sha256_hex(&b) == f.original_sha256 => b,
            Ok(_) => {
                report.failed.push((f.path.clone(), tr!("النسخة الاحتياطية تالفة", "The backup is damaged").into()));
                continue;
            }
            Err(e) => {
                report.failed.push((f.path.clone(), e.to_string()));
                continue;
            }
        };
        match write_atomic(path, &backup) {
            Ok(()) => report.restored.push(f.path.clone()),
            Err(e) => report.failed.push((f.path.clone(), e.to_string())),
        }
    }
    // Failures may be retried (a file locked by another program); conflicts never will succeed.
    if report.failed.is_empty() {
        fs::rename(dir.join(INDEX_FILE), dir.join(UNDONE_FILE)).map_err(|e| e.to_string())?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(find: &str, replace: &str) -> Options {
        Options { find: find.into(), replace: replace.into(), ..Default::default() }
    }

    fn matcher(o: &Options) -> Matcher {
        Matcher::new(o).unwrap()
    }

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(name: &str) -> TempDir {
            let d = std::env::temp_dir().join(format!("bahth_replace_{name}_{}", std::process::id()));
            let _ = fs::remove_dir_all(&d);
            fs::create_dir_all(&d).unwrap();
            TempDir(d)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn encodings_round_trip_and_keep_bom() {
        let text = "سطر أول\r\nsecond line\n";
        for enc in [TextEncoding::Utf8, TextEncoding::Utf8Bom, TextEncoding::Utf16Le, TextEncoding::Utf16Be, TextEncoding::Windows1256] {
            let bytes = encode(text, enc).unwrap();
            assert_eq!(decode(&bytes), Some((text.to_string(), enc)), "{enc:?}");
        }
        assert_eq!(&encode("a", TextEncoding::Utf8Bom).unwrap()[..3], &[0xEF, 0xBB, 0xBF]);
        assert_eq!(encode("a", TextEncoding::Utf16Le).unwrap(), [0xFF, 0xFE, b'a', 0]);
        assert_eq!(encode("a", TextEncoding::Utf16Be).unwrap(), [0xFE, 0xFF, 0, b'a']);
    }

    #[test]
    fn detection() {
        // "تقرير" in windows-1256 is not valid UTF-8.
        let cp = WINDOWS_1256.encode("تقرير").0.into_owned();
        assert_eq!(decode(&cp).unwrap().1, TextEncoding::Windows1256);
        assert_eq!(decode("تقرير".as_bytes()).unwrap().1, TextEncoding::Utf8);
        // Zero bytes without a BOM, odd-length UTF-16, a lone surrogate: refused.
        assert_eq!(decode(b"a\0b\0"), None);
        assert_eq!(decode(&[0xFF, 0xFE, b'a']), None);
        assert_eq!(decode(&[0xFF, 0xFE, 0x00, 0xD8]), None);
        // UTF-8 BOM followed by invalid UTF-8.
        assert_eq!(decode(&[0xEF, 0xBB, 0xBF, 0xFF]), None);
    }

    #[test]
    fn unencodable_in_1256() {
        assert_eq!(encode("😀", TextEncoding::Windows1256), None);
        assert!(encode("😀", TextEncoding::Utf16Le).is_some());
    }

    #[test]
    fn literal_case_and_regex() {
        let (out, n) = matcher(&opts("a.b", "X")).replace_all("a.b axb A.B");
        assert_eq!((out.as_ref(), n), ("X axb A.B", 1));
        let o = Options { ignore_case: true, ..opts("a.b", "X") };
        assert_eq!(matcher(&o).replace_all("a.b axb A.B").0, "X axb X");
        // Literal mode: `$1` in the replacement is plain text.
        assert_eq!(matcher(&opts("x", "$1")).replace_all("x").0, "$1");
        let o = Options { regex: true, ..opts(r"(\d+)-(\d+)", "$2-$1") };
        assert_eq!(matcher(&o).replace_all("10-20 and 3-4").0, "20-10 and 4-3");
        // Empty matches are ignored rather than inserting everywhere.
        let o = Options { regex: true, ..opts("z*", "-") };
        assert_eq!(matcher(&o).replace_all("abzzc"), (Cow::Owned::<str>("ab-c".into()), 1));
        assert!(Matcher::new(&Options { regex: true, ..opts("(", "") }).is_err());
        assert!(Matcher::new(&opts("", "x")).is_err());
        // Arabic, unchanged when nothing matches.
        let (out, n) = matcher(&opts("قديم", "جديد")).replace_all("لا شيء");
        assert!(matches!(out, Cow::Borrowed(_)) && n == 0);
    }

    /// `$1em` was read as the group named "1em", which does not exist, so
    /// the match was replaced by nothing: text silently deleted.
    #[test]
    fn group_numbers_followed_by_letters_keep_the_letters() {
        let re = |find: &str, replace: &str| Matcher::new(&Options { regex: true, ..opts(find, replace) });
        let run = |find: &str, replace: &str, text: &str| re(find, replace).unwrap().replace_all(text).0.into_owned();
        assert_eq!(run(r"(\d+)px", "$1em", "12px"), "12em");
        assert_eq!(run(r"(\w+)\.txt", "$1_new.txt", "a.txt"), "a_new.txt");
        assert_eq!(run(r"(\w+)", "${1}x", "ab"), "abx");
        assert_eq!(run(r"(\d)(\d)", "$2$1", "12"), "21");
        assert_eq!(run(r"(\d)", "$10", "7"), "70");
        assert_eq!(run(r"(\d+)", "$$$1", "5"), "$5");
        assert_eq!(run(r"(\d+)", "$", "5"), "$");
        assert_eq!(run(r"(\d+)", "$ and $.", "5"), "$ and $.");
        assert_eq!(run(r"(?P<n>\d+)", "${n}em", "5"), "5em");
        assert_eq!(run(r"(?P<n>\d+)", "$n.", "5"), "5.");
        assert_eq!(run(r"(?P<n>\d+)", "<$0>", "5"), "<5>");
        assert_eq!(run("ب(\\d)", "$1ت", "ب3"), "3ت");
        // Groups that do not exist are refused with a message, never
        // replaced by nothing.
        for bad in ["$2", "${2}", "$nx", "${nope}", "$n_x"] {
            let e = re(r"(?P<n>\d+)", bad).err().unwrap_or_else(|| panic!("{bad} accepted"));
            assert!(e.contains('$'), "{bad}: {e}");
        }
        // Literal mode is untouched.
        assert_eq!(matcher(&opts("x", "$1em")).replace_all("x").0, "$1em");
    }

    #[test]
    fn samples_show_lines() {
        let text = "one\r\nfoo here\r\nthree foo foo\nfour\nfoo\nfoo\n";
        let s = samples(text, &matcher(&opts("foo", "bar")));
        assert_eq!(s.len(), SAMPLE_LINES);
        assert_eq!(s[0], Sample { line: 2, before: "foo here".into(), after: "bar here".into() });
        assert_eq!(s[1], Sample { line: 3, before: "three foo foo".into(), after: "three bar bar".into() });
        assert_eq!(s[2].line, 5);
        let long = format!("{}foo{}", "x".repeat(200), "y".repeat(200));
        let s = samples(&long, &matcher(&opts("foo", "bar")));
        assert!(s[0].before.starts_with('…') && s[0].before.contains("foo") && s[0].before.ends_with('…'));
        assert!(s[0].after.contains("bar"));
    }

    /// `^` matches at the start of the line on its own but not inside the
    /// whole text, so the line's replaced copy has a longer text before the
    /// sampled match, and the match's byte offset fell inside a letter.
    #[test]
    fn sample_offsets_inside_a_replaced_letter() {
        let o = Options { regex: true, ..opts("^x|y", "ببب") };
        let s = samples("q\nxy", &matcher(&o));
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].line, s[0].before.as_str()), (2, "xy"));
        assert_eq!(s[0].after, "بببببب");
        assert_eq!(clip("ب", 1), "ب");
        assert_eq!(clip("ab", 99), "ab");
    }

    /// Random texts, patterns and replacements: no panic, and the samples
    /// and counts agree with the replace.
    #[test]
    fn random_matching_never_panics() {
        struct Rng(u64);
        impl Rng {
            fn below(&mut self, n: usize) -> usize {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 7;
                self.0 ^= self.0 << 17;
                (self.0 % n as u64) as usize
            }
        }
        let pieces = ["a", "b", "x", "ب", "ة", "\u{0651}", "\u{0640}", "\u{200F}", "😀", "\n", "\r\n", " ", "\t", "$", "İ"];
        let patterns = ["^x|y", "x$|b", "(?m)^a", r"\bب", "a*", "ب+", "(a)(b)?", "$", "^", ".", r"\s", "(?i)i", "😀|x"];
        let replacements = ["", "ببب", "$1", "${2}x", "😀😀", "\r\n", "$0$0"];
        let mut rng = Rng(0xDEAD_BEEF);
        for _ in 0..3_000 {
            let text: String = (0..rng.below(30)).map(|_| pieces[rng.below(pieces.len())]).collect();
            let regex = rng.below(2) == 0;
            let find = if regex { patterns[rng.below(patterns.len())].to_string() } else { pieces[rng.below(pieces.len())].to_string() };
            let o = Options { find, replace: replacements[rng.below(replacements.len())].into(), ignore_case: rng.below(2) == 0, regex };
            let Ok(m) = Matcher::new(&o) else { continue };
            let r = std::panic::catch_unwind(|| {
                let (after, n) = m.replace_all(&text);
                assert_eq!(n, m.count(&text));
                let s = samples(&text, &m);
                assert!(s.len() <= SAMPLE_LINES && (n == 0) == s.is_empty());
                for enc in [TextEncoding::Utf8Bom, TextEncoding::Utf16Le, TextEncoding::Utf16Be] {
                    let b = encode(&after, enc).expect("Unicode encodings hold everything");
                    assert_eq!(decode(&b), Some((after.to_string(), enc)));
                }
                if let Some(b) = encode(&after, TextEncoding::Windows1256) {
                    let _ = decode(&b);
                }
            });
            assert!(r.is_ok(), "panic for {o:?} on {text:?}");
        }
    }

    #[test]
    fn same_file_twice_is_replaced_once_and_undone_fully() {
        let t = TempDir::new("twice");
        let p = t.0.join("a.txt");
        fs::write(&p, "a").unwrap();
        let ps = p.to_string_lossy().into_owned();
        let o = opts("a", "aa");
        let report = apply(&[ps.clone(), ps.clone()], &matcher(&o), &o, &t.0.join("bk"), &|_| {}).unwrap();
        assert_eq!(report.done, [(ps.clone(), 1)]);
        assert_eq!(fs::read(&p).unwrap(), b"aa");
        assert!(undo(&report.dir.unwrap()).unwrap().conflicts.is_empty());
        assert_eq!(fs::read(&p).unwrap(), b"a");
    }

    /// An operation that changed one file twice (two records, as an older
    /// version could write) is undone newest first, back to the original.
    #[test]
    fn undo_goes_newest_first() {
        let t = TempDir::new("undo_order");
        let p = t.0.join("a.txt");
        let ps = p.to_string_lossy().into_owned();
        let dir = t.0.join("op");
        fs::create_dir_all(&dir).unwrap();
        let (v0, v1, v2) = (b"v0".as_slice(), b"v1".as_slice(), b"v2".as_slice());
        fs::write(dir.join("0001.bak"), v0).unwrap();
        fs::write(dir.join("0002.bak"), v1).unwrap();
        fs::write(&p, v2).unwrap();
        let rec = |backup: &str, from: &[u8], to: &[u8]| OpFile {
            path: ps.clone(),
            backup: backup.into(),
            original_sha256: sha256_hex(from),
            new_sha256: sha256_hex(to),
            count: 1,
        };
        let index = OpIndex { version: 1, files: vec![rec("0001.bak", v0, v1), rec("0002.bak", v1, v2)], ..Default::default() };
        write_index(&dir, &index).unwrap();
        let u = undo(&dir).unwrap();
        assert!(u.conflicts.is_empty() && u.failed.is_empty(), "{u:?}");
        assert_eq!(fs::read(&p).unwrap(), v0);
    }

    /// A damaged index pointing a backup outside its folder is refused.
    #[test]
    fn undo_refuses_backup_paths() {
        let t = TempDir::new("undo_paths");
        let p = t.0.join("a.txt");
        fs::write(&p, "new").unwrap();
        let outside = t.0.join("outside.txt");
        fs::write(&outside, "old").unwrap();
        let dir = t.0.join("op");
        fs::create_dir_all(&dir).unwrap();
        for backup in ["../outside.txt", outside.to_string_lossy().as_ref(), "", ".."] {
            let index = OpIndex {
                version: 1,
                files: vec![OpFile {
                    path: p.to_string_lossy().into_owned(),
                    backup: backup.into(),
                    original_sha256: sha256_hex(b"old"),
                    new_sha256: sha256_hex(b"new"),
                    count: 1,
                }],
                ..Default::default()
            };
            write_index(&dir, &index).unwrap();
            let u = undo(&dir).unwrap();
            assert_eq!(u.failed.len(), 1, "{backup}");
            assert_eq!(fs::read(&p).unwrap(), b"new");
        }
    }

    #[test]
    fn backup_sequence_at_the_top_does_not_overflow() {
        let t = TempDir::new("seq_max");
        fs::create_dir_all(t.0.join(format!("s{}-19700101-000000Z", u64::MAX))).unwrap();
        assert!(new_op_dir(&t.0, 0).is_err());
        assert!(!stamp(i64::MAX, i32::MAX).is_empty());
        let _ = stamp(i64::MIN, i32::MIN);
    }

    #[test]
    fn stamps() {
        assert_eq!(stamp(0, 0), "19700101-000000");
        // 2026-10-30 12:54:56 UTC, shown at UTC+3.
        assert_eq!(stamp(1_793_364_896, 3 * 3600), "20261030-155456");
    }

    #[test]
    fn apply_keeps_encoding_and_undo_restores() {
        let t = TempDir::new("apply");
        let root = t.0.join("backups");
        let files = [
            ("a.txt", encode("قديم\r\nold قديم\r\n", TextEncoding::Utf8Bom).unwrap()),
            ("b.csv", encode("x,قديم\ny\n", TextEncoding::Utf16Le).unwrap()),
            ("c.ini", encode("[قديم]\r\n", TextEncoding::Windows1256).unwrap()),
            ("d.md", encode("nothing here\n", TextEncoding::Utf8).unwrap()),
            ("e.docx", b"PK\x03\x04 \xd9\x82\xd8\xaf\xd9\x8a\xd9\x85".to_vec()),
            ("f.txt", b"bin\0\xff".to_vec()),
        ];
        let mut paths = Vec::new();
        for (name, bytes) in &files {
            let p = t.0.join(name);
            fs::write(&p, bytes).unwrap();
            paths.push(p.to_string_lossy().into_owned());
        }
        let o = opts("قديم", "جديد");
        let m = matcher(&o);
        assert!(matches!(preview_file(Path::new(&paths[0]), &m), Outcome::Matches(FilePreview { count: 2, encoding: TextEncoding::Utf8Bom, .. })));
        assert!(matches!(preview_file(Path::new(&paths[3]), &m), Outcome::NoMatch));
        assert!(matches!(preview_file(Path::new(&paths[4]), &m), Outcome::Skipped(Skip::NotText)));
        assert!(matches!(preview_file(Path::new(&paths[5]), &m), Outcome::Skipped(Skip::Binary)));

        let report = apply(&paths, &m, &o, &root, &|_| {}).unwrap();
        assert_eq!(report.done.len(), 3);
        assert_eq!(report.unchanged, 1);
        assert_eq!(report.skipped.len(), 2);
        let read = |i: usize| fs::read(&paths[i]).unwrap();
        assert_eq!(read(0), encode("جديد\r\nold جديد\r\n", TextEncoding::Utf8Bom).unwrap());
        assert_eq!(read(1), encode("x,جديد\ny\n", TextEncoding::Utf16Le).unwrap());
        assert_eq!(read(2), encode("[جديد]\r\n", TextEncoding::Windows1256).unwrap());
        assert_eq!(read(4), files[4].1);
        // No temporary files left behind.
        assert_eq!(fs::read_dir(&t.0).unwrap().count(), files.len() + 1);

        // A file edited after the replace is not overwritten by Undo.
        fs::write(&paths[2], b"edited later").unwrap();
        let (dir, index) = last_operation(&root).unwrap();
        assert_eq!(index.files.len(), 3);
        let u = undo(&dir).unwrap();
        assert_eq!(u.restored.len(), 2);
        assert_eq!(u.conflicts, [paths[2].clone()]);
        assert_eq!(read(0), files[0].1);
        assert_eq!(read(1), files[1].1);
        assert_eq!(read(2), b"edited later");
        // Undone: nothing left to undo.
        assert!(last_operation(&root).is_none());
        assert!(undo(&dir).is_err());
    }

    #[test]
    fn skips_unsafe_files() {
        let t = TempDir::new("skips");
        let m = matcher(&opts("a", "😀"));
        let p = t.0.join("cp.txt");
        fs::write(&p, WINDOWS_1256.encode("a تقرير").0).unwrap();
        assert!(matches!(preview_file(&p, &m), Outcome::Skipped(Skip::Unencodable)));
        let big = t.0.join("big.log");
        fs::File::create(&big).unwrap().set_len(MAX_FILE_SIZE + 1).unwrap();
        assert!(matches!(preview_file(&big, &m), Outcome::Skipped(Skip::TooBig)));
        let ro = t.0.join("ro.txt");
        fs::write(&ro, "a").unwrap();
        let mut perm = fs::metadata(&ro).unwrap().permissions();
        perm.set_readonly(true);
        fs::set_permissions(&ro, perm.clone()).unwrap();
        assert!(matches!(preview_file(&ro, &m), Outcome::Skipped(Skip::ReadOnly)));
        let o = opts("a", "b");
        let report = apply(&[ro.to_string_lossy().into_owned()], &matcher(&o), &o, &t.0.join("bk"), &|_| {}).unwrap();
        assert_eq!(report.skipped[0].1, Skip::ReadOnly);
        assert!(report.dir.is_none());
        assert_eq!(fs::read(&ro).unwrap(), b"a");
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        fs::set_permissions(&ro, perm).unwrap();
    }

    #[test]
    fn write_atomic_replaces_and_creates() {
        let t = TempDir::new("atomic");
        let p = t.0.join("ملف.txt");
        write_atomic(&p, b"one").unwrap();
        write_atomic(&p, b"two").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"two");
        assert_eq!(fs::read_dir(&t.0).unwrap().count(), 1);
    }

    /// The replaced file keeps its Hidden and System attributes.
    #[cfg(windows)]
    #[test]
    fn write_atomic_keeps_attributes() {
        use std::os::windows::ffi::OsStrExt;
        use winapi::um::fileapi::{GetFileAttributesW, SetFileAttributesW};
        use winapi::um::winnt::{FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_SYSTEM};
        let t = TempDir::new("attrs");
        let p = t.0.join("مخفي.txt");
        fs::write(&p, "old").unwrap();
        let w: Vec<u16> = p.as_os_str().encode_wide().chain([0]).collect();
        let flags = FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM;
        // SAFETY: `w` is a NUL-terminated path.
        assert_ne!(unsafe { SetFileAttributesW(w.as_ptr(), flags) }, 0);
        write_atomic(&p, b"new").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"new");
        // SAFETY: as above.
        let attrs = unsafe { GetFileAttributesW(w.as_ptr()) };
        assert_eq!(attrs & flags, flags);
        // SAFETY: as above.
        unsafe { SetFileAttributesW(w.as_ptr(), FILE_ATTRIBUTE_NORMAL) };
    }

    #[test]
    fn backups_are_ordered_by_sequence_not_clock() {
        let t = TempDir::new("seq");
        // Old-format folders are still listed, older than any new one.
        fs::create_dir_all(t.0.join("20991231-235959")).unwrap();
        fs::create_dir_all(t.0.join("20260101-000000-002")).unwrap();
        let a = new_op_dir(&t.0, 1_793_364_896).unwrap();
        assert!(a.ends_with("s00000001-20261030-125456Z"));
        // The clock goes back an hour: the next one is still the newest.
        let b = new_op_dir(&t.0, 1_793_364_896 - 3600).unwrap();
        assert!(b.ends_with("s00000002-20261030-115456Z"));
        let c = new_op_dir(&t.0, 0).unwrap();
        assert_eq!(op_dirs(&t.0), [c.clone(), b, a, t.0.join("20991231-235959"), t.0.join("20260101-000000-002")]);
        fs::write(c.join(INDEX_FILE), serde_json::to_vec(&OpIndex::default()).unwrap()).unwrap();
        assert_eq!(last_operation(&t.0).unwrap().0, c);
        // Sequences past 8 digits still sort by number, and gaps are fine.
        fs::create_dir_all(t.0.join("s123456789-19700101-000000Z")).unwrap();
        assert!(new_op_dir(&t.0, 0).unwrap().ends_with("s123456790-19700101-000000Z"));
        for bad in ["s-1Z", "sx-20260101Z", "s1-2026", "seq", "s12"] {
            assert_eq!(op_dir_key(bad), None, "{bad}");
        }
    }

    #[test]
    fn keeps_only_recent_backups() {
        let t = TempDir::new("prune");
        for i in 0..KEEP_OPERATIONS + 3 {
            fs::create_dir_all(t.0.join(format!("20260101-0000{i:02}"))).unwrap();
        }
        fs::create_dir_all(t.0.join("other")).unwrap();
        prune(&t.0);
        let dirs = op_dirs(&t.0);
        assert_eq!(dirs.len(), KEEP_OPERATIONS);
        assert!(dirs[0].ends_with(format!("20260101-0000{:02}", KEEP_OPERATIONS + 2)));
        assert!(t.0.join("other").exists());
        // New folders, whatever their time, push out the old-format ones first.
        let newest = new_op_dir(&t.0, 0).unwrap();
        prune(&t.0);
        let dirs = op_dirs(&t.0);
        assert_eq!(dirs.len(), KEEP_OPERATIONS);
        assert_eq!(dirs[0], newest);
        assert!(dirs[1].ends_with(format!("20260101-0000{:02}", KEEP_OPERATIONS + 2)));
        assert!(!t.0.join("20260101-000003").exists());
    }
}
