//! E-mail formats: RFC 822 / MIME `.eml` (hand-written parser) and Outlook
//! `.msg` (CFB compound file, via the `cfb` crate).
//!
//! Only searchable text is kept: the Subject / From / To headers and the
//! message body (text/plain parts, text/html as a fallback). Non-text
//! attachments are skipped.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE};

use super::{
    MAX_FILE_SIZE, MAX_OUTPUT, MAX_TEXT_READ, codepage_encoding, decode_bytes, read_capped,
    strip_html,
};

/// Nested multipart / message/rfc822 depth limit.
const MAX_DEPTH: u32 = 16;
/// Upper bound for the number of MIME parts visited.
const MAX_PARTS: usize = 4096;

// ---------------------------------------------------------------------------
// EML
// ---------------------------------------------------------------------------

pub(super) fn extract_eml(path: &Path) -> Option<String> {
    eml_to_text(&read_capped(File::open(path).ok()?, MAX_FILE_SIZE)?)
}

/// Appends `v` trimmed plus a newline, unless it is blank.
fn push_line(out: &mut String, v: &str) {
    let v = v.trim();
    if !v.is_empty() {
        out.push_str(v);
        out.push('\n');
    }
}

/// Decodes with `enc` when known, else by guessing (see `decode_bytes`).
fn decode_with(bytes: &[u8], enc: Option<&'static Encoding>) -> String {
    match enc {
        Some(enc) => enc.decode_without_bom_handling(bytes).0.into_owned(),
        None => decode_bytes(bytes),
    }
}

pub(super) fn eml_to_text(data: &[u8]) -> Option<String> {
    eml_to_text_at(data, 0)
}

/// `depth` counts enclosing messages and multiparts, so forwarded messages
/// nested inside each other stay within MAX_DEPTH.
fn eml_to_text_at(data: &[u8], depth: u32) -> Option<String> {
    if depth > MAX_DEPTH {
        return None;
    }
    let (headers, _) = split_message(data);
    // Something that is not a mail at all (no header lines) is rejected.
    if headers.is_empty() {
        return None;
    }
    let mut out = String::new();
    for name in ["subject", "from", "to"] {
        if let Some(v) = header_value(&headers, name) {
            push_line(&mut out, &decode_header_value(v));
        }
    }
    let mut walk = Walk::default();
    walk.part(data, depth, None);
    let body = if walk.plain.is_empty() {
        walk.html
    } else {
        walk.plain
    };
    for chunk in body {
        if out.len() > MAX_OUTPUT {
            break;
        }
        out.push('\n');
        out.push_str(&chunk);
    }
    Some(out)
}

#[derive(Default)]
struct Walk {
    plain: Vec<String>,
    html: Vec<String>,
    parts: usize,
    bytes: usize,
}

impl Walk {
    /// Processes one MIME entity (headers + body). `default_ct` is the
    /// implied content type (`message/rfc822` children of multipart/digest).
    fn part(&mut self, data: &[u8], depth: u32, default_ct: Option<&str>) {
        self.parts += 1;
        if depth > MAX_DEPTH || self.parts > MAX_PARTS || self.bytes > MAX_OUTPUT {
            return;
        }
        let (headers, body) = split_message(data);
        let ct_raw = header_value(&headers, "content-type");
        let (mime, params) = match ct_raw {
            Some(v) => parse_content_type(&String::from_utf8_lossy(v)),
            None => (default_ct.unwrap_or("text/plain").to_owned(), Vec::new()),
        };
        let param = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());

        if let Some(sub) = mime.strip_prefix("multipart/") {
            let Some(boundary) = param("boundary").filter(|b| !b.is_empty()) else {
                return;
            };
            let child_default = (sub == "digest").then_some("message/rfc822");
            for child in split_multipart(body, boundary.as_bytes()) {
                self.part(child, depth + 1, child_default);
            }
            return;
        }

        let cte = header_value(&headers, "content-transfer-encoding")
            .map(|v| String::from_utf8_lossy(v).trim().to_ascii_lowercase())
            .unwrap_or_default();
        let decoded = match cte.as_str() {
            "base64" => decode_base64(body),
            "quoted-printable" => decode_qp(body),
            _ => body.to_vec(),
        };

        if mime == "message/rfc822" || mime == "message/global" {
            // Forwarded message: its own headers and body are text too.
            if let Some(text) = eml_to_text_at(&decoded, depth + 1) {
                self.bytes += text.len();
                self.plain.push(text);
            }
            return;
        }
        let is_plain = mime == "text/plain";
        let is_html = mime == "text/html";
        if !is_plain && !is_html {
            return; // images, pdf, octet-stream, calendar, ...
        }
        let text = decode_charset(&decoded, param("charset"));
        let text = if is_html { strip_html(&text) } else { text };
        self.bytes += text.len();
        if is_plain {
            self.plain.push(text);
        } else {
            self.html.push(text);
        }
    }
}

/// Splits an entity into (header lines, body). The body starts after the
/// first empty line; without one everything is headers.
fn split_message(data: &[u8]) -> (Vec<(String, Vec<u8>)>, &[u8]) {
    let mut headers: Vec<(String, Vec<u8>)> = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let end = data[pos..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(data.len(), |p| pos + p);
        let next = (end + 1).min(data.len());
        let mut line = &data[pos..end];
        if let Some(l) = line.strip_suffix(b"\r") {
            line = l;
        }
        if line.is_empty() {
            return (headers, &data[next..]);
        }
        if headers.is_empty() && pos == 0 && line.starts_with(b"From ") {
            // mbox "From " separator line.
            pos = next;
            continue;
        }
        if matches!(line[0], b' ' | b'\t') {
            // Folded continuation of the previous header.
            if let Some((_, v)) = headers.last_mut() {
                v.push(b' ');
                v.extend_from_slice(trim_ascii(line));
            }
        } else if let Some(colon) = line.iter().position(|&b| b == b':') {
            let name = trim_ascii(&line[..colon]);
            if name.is_empty() || !name.iter().all(|b| (33..=126).contains(b)) {
                // Not a header line: treat the rest as body.
                return (headers, &data[pos..]);
            }
            let name = String::from_utf8_lossy(name).to_ascii_lowercase();
            headers.push((name, trim_ascii(&line[colon + 1..]).to_vec()));
        } else if headers.is_empty() {
            // Garbage before any header: not a MIME entity.
            return (headers, data);
        } else {
            return (headers, &data[pos..]);
        }
        pos = next;
    }
    (headers, &data[data.len()..])
}

fn trim_ascii(b: &[u8]) -> &[u8] {
    let start = b
        .iter()
        .position(|c| !c.is_ascii_whitespace())
        .unwrap_or(b.len());
    let end = b
        .iter()
        .rposition(|c| !c.is_ascii_whitespace())
        .map_or(start, |p| p + 1);
    &b[start..end.max(start)]
}

fn header_value<'a>(headers: &'a [(String, Vec<u8>)], name: &str) -> Option<&'a [u8]> {
    headers
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_slice())
}

/// `type/subtype; k=v; k="v"` -> (lowercase mime, [(lowercase key, value)]).
fn parse_content_type(v: &str) -> (String, Vec<(String, String)>) {
    let mut params = Vec::new();
    let mut parts = v.splitn(2, ';');
    let mime = parts.next().unwrap_or("").trim().to_ascii_lowercase();
    let mut rest = parts.next().unwrap_or("");
    loop {
        rest = rest.trim_start_matches(|c: char| c == ';' || c.is_whitespace());
        if rest.is_empty() {
            break;
        }
        let Some(eq) = rest.find('=') else { break };
        let key = rest[..eq].trim().to_ascii_lowercase();
        rest = rest[eq + 1..].trim_start();
        let value;
        if let Some(q) = rest.strip_prefix('"') {
            let mut val = String::new();
            let mut chars = q.char_indices();
            let mut consumed = q.len();
            while let Some((i, c)) = chars.next() {
                match c {
                    '\\' => {
                        if let Some((_, n)) = chars.next() {
                            val.push(n);
                        }
                    }
                    '"' => {
                        consumed = i + 1;
                        break;
                    }
                    c => val.push(c),
                }
            }
            value = val;
            rest = &q[consumed.min(q.len())..];
        } else {
            let end = rest.find(';').unwrap_or(rest.len());
            value = rest[..end].trim().to_owned();
            rest = &rest[end..];
        }
        // RFC 2231 `charset*=utf-8''...`: keep the charset part only.
        let (key, value) = match key.strip_suffix('*') {
            Some(k) => (
                k.to_owned(),
                value.split('\'').next().unwrap_or("").to_owned(),
            ),
            None => (key, value),
        };
        params.push((key, value));
    }
    (mime, params)
}

/// Bodies of the parts of a multipart body delimited by `--boundary` lines.
fn split_multipart<'a>(body: &'a [u8], boundary: &[u8]) -> Vec<&'a [u8]> {
    let mut delim = b"--".to_vec();
    delim.extend_from_slice(boundary);
    // Offsets of delimiter lines (start of line).
    let mut marks = Vec::new();
    let mut pos = 0;
    while let Some(off) = find(&body[pos..], &delim) {
        let at = pos + off;
        if at == 0 || body[at - 1] == b'\n' {
            marks.push(at);
        }
        pos = at + delim.len();
    }
    let mut parts = Vec::new();
    for (i, &m) in marks.iter().enumerate() {
        let after = m + delim.len();
        if body[after..].starts_with(b"--") {
            break; // close delimiter
        }
        let start = body[after..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(body.len(), |p| after + p + 1);
        let mut end = marks.get(i + 1).copied().unwrap_or(body.len());
        // The CRLF before a delimiter belongs to the delimiter.
        if end > start && body[end - 1] == b'\n' && marks.get(i + 1).is_some() {
            end -= 1;
            if end > start && body[end - 1] == b'\r' {
                end -= 1;
            }
        }
        parts.push(&body[start.min(end)..end]);
    }
    parts
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    let first = needle[0];
    let last = hay.len() - needle.len();
    let mut i = 0;
    while i <= last {
        i += hay[i..=last].iter().position(|&b| b == first)?;
        if hay[i..].starts_with(needle) {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn charset_encoding(charset: &str) -> Option<&'static Encoding> {
    let cs = charset.trim().trim_matches('"');
    // RFC 2231 language suffix: `utf-8*ar`.
    let cs = cs.split('*').next().unwrap_or(cs);
    Encoding::for_label(cs.as_bytes()).or_else(|| {
        // `cp1256`, `windows1256`, `ms-1256` and friends.
        let digits: String = cs.chars().filter(char::is_ascii_digit).collect();
        let cp: u16 = digits.parse().ok()?;
        codepage_to_encoding(cp)
    })
}

fn decode_charset(bytes: &[u8], charset: Option<&str>) -> String {
    decode_with(bytes, charset.and_then(charset_encoding))
}

/// Decodes a raw header value: 8-bit bytes via `decode_bytes`, then any
/// RFC 2047 encoded-words. Adjacent encoded-words in the same charset are
/// joined before decoding so multi-byte characters split across them survive.
pub(super) fn decode_header_value(raw: &[u8]) -> String {
    let s = decode_bytes(raw);
    let mut out = String::with_capacity(s.len());
    let mut pending: Option<(&'static Encoding, Vec<u8>)> = None;
    let mut rest = s.as_str();
    let flush = |pending: &mut Option<(&'static Encoding, Vec<u8>)>, out: &mut String| {
        if let Some((enc, bytes)) = pending.take() {
            out.push_str(&enc.decode_without_bom_handling(&bytes).0);
        }
    };
    while let Some(start) = rest.find("=?") {
        let Some((word_len, enc, bytes)) = parse_encoded_word(&rest[start..]) else {
            flush(&mut pending, &mut out);
            out.push_str(&rest[..start + 2]);
            rest = &rest[start + 2..];
            continue;
        };
        let between = &rest[..start];
        // Whitespace between two encoded-words is dropped (RFC 2047 §6.2).
        if !(pending.is_some() && between.trim().is_empty()) {
            flush(&mut pending, &mut out);
            out.push_str(between);
        }
        match &mut pending {
            Some((penc, pbytes)) if *penc == enc => pbytes.extend_from_slice(&bytes),
            _ => {
                flush(&mut pending, &mut out);
                pending = Some((enc, bytes));
            }
        }
        rest = &rest[start + word_len..];
    }
    flush(&mut pending, &mut out);
    out.push_str(rest);
    out
}

/// Parses `=?charset?B|Q?text?=` at the start of `s`.
/// Returns (length consumed, encoding, decoded bytes).
fn parse_encoded_word(s: &str) -> Option<(usize, &'static Encoding, Vec<u8>)> {
    let inner = s.strip_prefix("=?")?;
    let q1 = inner.find('?')?;
    let charset = &inner[..q1];
    let after_cs = &inner[q1 + 1..];
    let q2 = after_cs.find('?')?;
    let kind = &after_cs[..q2];
    let payload_start = &after_cs[q2 + 1..];
    let end = payload_start.find("?=")?;
    let payload = &payload_start[..end];
    if charset.is_empty() || payload.contains(' ') {
        return None;
    }
    let enc = charset_encoding(charset).unwrap_or(UTF_8);
    let bytes = match kind {
        "B" | "b" => decode_base64(payload.as_bytes()),
        "Q" | "q" => {
            let replaced: Vec<u8> = payload
                .bytes()
                .map(|b| if b == b'_' { b' ' } else { b })
                .collect();
            decode_qp(&replaced)
        }
        _ => return None,
    };
    let consumed = 2 + q1 + 1 + q2 + 1 + end + 2;
    Some((consumed, enc, bytes))
}

/// Lenient base64: ignores whitespace and junk, stops at padding.
fn decode_base64(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for &c in input {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => {
                // Padding ends a group; anything after (rare concatenation)
                // restarts cleanly.
                acc = 0;
                bits = 0;
                continue;
            }
            _ => continue,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

fn decode_qp(input: &[u8]) -> Vec<u8> {
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let b = input[i];
        if b == b'=' {
            let rest = &input[i + 1..];
            if rest.starts_with(b"\r\n") {
                i += 3;
                continue;
            }
            if rest.starts_with(b"\n") {
                i += 2;
                continue;
            }
            if let (Some(&h), Some(&l)) = (rest.first(), rest.get(1))
                && let (Some(h), Some(l)) = (hex(h), hex(l))
            {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(b);
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// MSG (Outlook)
// ---------------------------------------------------------------------------

/// Windows code page -> encoding, for codes encoding_rs knows; None otherwise.
fn codepage_to_encoding(cp: u16) -> Option<&'static Encoding> {
    let label = match cp {
        65001 => return Some(UTF_8),
        1200 => return Some(UTF_16LE),
        1201 => return Some(UTF_16BE),
        874 | 932 | 936 | 949 | 950 | 1250..=1258 | 10000 => return Some(codepage_encoding(cp)),
        20127 | 28591 => "windows-1252",
        28592..=28606 => return Encoding::for_label(format!("iso-8859-{}", cp - 28590).as_bytes()),
        20866 => "koi8-r",
        21866 => "koi8-u",
        50220..=50222 => "iso-2022-jp",
        51932 | 20932 => "euc-jp",
        51949 => "euc-kr",
        52936 | 54936 => "gb18030",
        _ => return None,
    };
    Encoding::for_label(label.as_bytes())
}

pub(super) fn extract_msg(path: &Path) -> Option<String> {
    let comp = cfb::CompoundFile::open(File::open(path).ok()?).ok()?;
    msg_to_text(comp)
}

fn msg_to_text<F: Read + std::io::Seek>(mut comp: cfb::CompoundFile<F>) -> Option<String> {
    // Top-level streams, keyed by uppercase name (CFB names are
    // case-insensitive and writers differ in hex digit case).
    let names: Vec<(String, std::path::PathBuf)> = comp
        .read_root_storage()
        .filter(|e| e.is_stream())
        .map(|e| (e.name().to_ascii_uppercase(), e.path().to_owned()))
        .collect();
    if !names.iter().any(|(n, _)| n.starts_with("__SUBSTG1.0_")) {
        return None;
    }
    // Read every stream we may need up front (each size-capped).
    let mut wanted = vec![
        "__PROPERTIES_VERSION1.0".to_owned(),
        "__SUBSTG1.0_10130102".to_owned(),
    ];
    for id in ["0037", "0C1A", "0E04", "1000", "1013"] {
        wanted.push(format!("__SUBSTG1.0_{id}001F"));
        wanted.push(format!("__SUBSTG1.0_{id}001E"));
    }
    let mut streams: Vec<(String, Vec<u8>)> = Vec::new();
    for (name, p) in &names {
        if !wanted.contains(name) {
            continue;
        }
        if let Ok(s) = comp.open_stream(p)
            && let Some(buf) = read_capped(s, MAX_TEXT_READ)
        {
            streams.push((name.clone(), buf));
        }
    }
    let read = |name: &str| -> Option<&[u8]> {
        streams
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, b)| b.as_slice())
    };

    // PR_MESSAGE_CODEPAGE (3FFD), else PR_INTERNET_CPID (3FDE), from the
    // fixed-size property stream (32-byte header, 16-byte entries).
    let find_long = |id: u32| {
        let entries = read("__PROPERTIES_VERSION1.0")?.get(32..)?;
        entries.chunks_exact(16).find_map(|e| {
            let tag = u32::from_le_bytes(e[0..4].try_into().ok()?);
            (tag == (id << 16 | 0x0003)).then(|| u32::from_le_bytes(e[8..12].try_into().unwrap_or_default()))
        })
    };
    let to_enc = |cp: Option<u32>| cp.and_then(|cp| u16::try_from(cp).ok()).and_then(codepage_to_encoding);
    // 8-bit string properties use PR_MESSAGE_CODEPAGE; the binary HTML body
    // uses PR_INTERNET_CPID (MS-OXCMSG). Each falls back to the other.
    let enc8 = to_enc(find_long(0x3FFD).or_else(|| find_long(0x3FDE)));
    let enc_html = to_enc(find_long(0x3FDE)).or(enc8);
    let trim_nul = |s: String| s.trim_end_matches('\0').to_owned();

    let prop = |id: &str| -> Option<String> {
        if let Some(b) = read(&format!("__SUBSTG1.0_{id}001F")) {
            return Some(trim_nul(
                UTF_16LE.decode_without_bom_handling(b).0.into_owned(),
            ));
        }
        read(&format!("__SUBSTG1.0_{id}001E")).map(|b| trim_nul(decode_with(b, enc8)))
    };

    let mut out = String::new();
    for id in ["0037", "0C1A", "0E04"] {
        if let Some(v) = prop(id) {
            push_line(&mut out, &v);
        }
    }
    let body = prop("1000").filter(|b| !b.trim().is_empty()).or_else(|| {
        // HTML body: usually PT_BINARY (0102), sometimes a string property.
        let html = prop("1013").or_else(|| {
            read("__SUBSTG1.0_10130102").map(|b| match enc_html {
                Some(enc) => enc.decode(b).0.into_owned(),
                None => decode_bytes(b),
            })
        })?;
        Some(strip_html(&html))
    });
    if let Some(body) = body {
        out.push('\n');
        out.push_str(&body);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use encoding_rs::WINDOWS_1256;
    use std::io::{Cursor, Write};

    fn b64(data: &[u8]) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut s = String::new();
        for c in data.chunks(3) {
            let n = (c[0] as u32) << 16
                | (*c.get(1).unwrap_or(&0) as u32) << 8
                | *c.get(2).unwrap_or(&0) as u32;
            for i in 0..4 {
                if i <= c.len() {
                    s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
                } else {
                    s.push('=');
                }
            }
        }
        s
    }

    fn qp(s: &str) -> String {
        let mut out = String::new();
        for b in s.bytes() {
            if b.is_ascii_alphanumeric() || b == b' ' {
                out.push(b as char);
            } else {
                out.push_str(&format!("={b:02X}"));
            }
        }
        out
    }

    #[test]
    fn eml_arabic_subject_and_qp_body() {
        let subject = WINDOWS_1256.encode("تقرير الميزانية").0;
        let body = "مرحبا بكم في الاجتماع";
        let eml = format!(
            "From: =?UTF-8?Q?=D8=B3=D8=A7=D9=84=D9=85?= <salem@example.com>\r\n\
             To: team@example.com\r\n\
             Subject: =?windows-1256?B?{}?=\r\n\
             MIME-Version: 1.0\r\n\
             Content-Type: multipart/mixed; boundary=\"outer\"\r\n\
             \r\n\
             preamble\r\n\
             --outer\r\n\
             Content-Type: multipart/alternative;\r\n\tboundary=inner\r\n\
             \r\n\
             --inner\r\n\
             Content-Type: text/plain; charset=utf-8\r\n\
             Content-Transfer-Encoding: quoted-printable\r\n\
             \r\n\
             {}=\r\n end\r\n\
             --inner\r\n\
             Content-Type: text/html; charset=utf-8\r\n\
             \r\n\
             <p>HTMLONLY</p>\r\n\
             --inner--\r\n\
             --outer\r\n\
             Content-Type: image/png\r\n\
             Content-Transfer-Encoding: base64\r\n\
             \r\n\
             iVBORw0KGgo=\r\n\
             --outer--\r\n",
            b64(&subject),
            qp(body),
        );
        let text = eml_to_text(eml.as_bytes()).unwrap();
        assert!(text.contains("تقرير الميزانية"), "{text}");
        assert!(text.contains("سالم"), "{text}");
        assert!(text.contains("team@example.com"), "{text}");
        assert!(text.contains("مرحبا بكم في الاجتماع end"), "{text}");
        assert!(!text.contains("HTMLONLY"), "{text}");
        assert!(!text.contains("iVBOR"), "{text}");
    }

    #[test]
    fn eml_html_fallback_and_8bit() {
        let mut eml = b"Subject: plain 8bit \xd5\xc8\xc7\xcd\r\nContent-Type: text/html; charset=windows-1256\r\nContent-Transfer-Encoding: base64\r\n\r\n".to_vec();
        let html = WINDOWS_1256
            .encode("<html><body><b>عاجل</b> &amp; مهم</body></html>")
            .0;
        eml.extend_from_slice(b64(&html).as_bytes());
        let text = eml_to_text(&eml).unwrap();
        assert!(text.contains("صباح"), "{text}");
        assert!(text.contains("عاجل") && text.contains("& مهم"), "{text}");
        assert!(!text.contains("<b>"));
    }

    #[test]
    fn encoded_words() {
        assert_eq!(
            decode_header_value(b"=?utf-8?q?a_b?= =?utf-8?b?w6k=?= c"),
            "a bé c"
        );
        // Split multi-byte char across two words.
        assert_eq!(
            decode_header_value(b"=?utf-8?b?2A==?= =?utf-8?b?pw==?="),
            "ا"
        );
        assert_eq!(decode_header_value(b"broken =?x"), "broken =?x");
    }

    #[test]
    fn eml_garbage() {
        assert!(eml_to_text(b"").is_none());
        assert!(eml_to_text(b"\x00\x01 no headers here").is_none());
        // Unterminated multipart must not panic.
        let t = eml_to_text(
            b"Content-Type: multipart/mixed; boundary=x\n\n--x\nContent-Type: text/plain\n\nhi",
        );
        assert!(t.unwrap().contains("hi"));
    }

    fn build_msg(streams: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut comp = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        for (name, data) in streams {
            let mut s = comp.create_stream(format!("/{name}")).unwrap();
            s.write_all(data).unwrap();
        }
        comp.flush().unwrap();
        comp.into_inner().into_inner()
    }

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    #[test]
    fn msg_unicode_and_codepage() {
        let mut props = vec![0u8; 32];
        props.extend_from_slice(&0x3FFD_0003u32.to_le_bytes());
        props.extend_from_slice(&[0; 4]);
        props.extend_from_slice(&1256u32.to_le_bytes());
        props.extend_from_slice(&[0; 4]);
        let bytes = build_msg(&[
            ("__substg1.0_0037001F", utf16("موضوع الرسالة")),
            ("__substg1.0_0C1A001F", utf16("أحمد")),
            ("__substg1.0_0E04001F", utf16("فريق العمل")),
            (
                "__substg1.0_1000001E",
                WINDOWS_1256.encode("نص الرسالة\0").0.into_owned(),
            ),
            ("__properties_version1.0", props),
        ]);
        let comp = cfb::CompoundFile::open(Cursor::new(bytes)).unwrap();
        let text = msg_to_text(comp).unwrap();
        for w in ["موضوع الرسالة", "أحمد", "فريق العمل", "نص الرسالة"]
        {
            assert!(text.contains(w), "{w}: {text}");
        }
        assert!(!text.contains('\0'));
    }

    #[test]
    fn msg_html_fallback() {
        let bytes = build_msg(&[
            ("__substg1.0_0037001F", utf16("Hello")),
            ("__substg1.0_10130102", b"<p>Body &amp; more</p>".to_vec()),
        ]);
        let comp = cfb::CompoundFile::open(Cursor::new(bytes)).unwrap();
        let text = msg_to_text(comp).unwrap();
        assert!(
            text.contains("Hello") && text.contains("Body & more"),
            "{text}"
        );
        // A compound file that is not a message (e.g. a .doc) is rejected.
        let bytes = build_msg(&[("WordDocument", vec![1, 2, 3])]);
        assert!(msg_to_text(cfb::CompoundFile::open(Cursor::new(bytes)).unwrap()).is_none());
    }
}
