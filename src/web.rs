//! Search from a phone over the home network: a small, read-only HTTP server.
//!
//! Off unless the user turns it on (Options ▸ "Search from phone"). Safety
//! comes first:
//! - it listens only on private LAN addresses (10/8, 172.16/12, 192.168/16,
//!   169.254/16) and 127.0.0.1, and drops any connection whose peer or local
//!   address is not private;
//! - every request needs the secret token, either as the `?t=` of the link
//!   shown in the window or as a cookie set after typing the PIN; both are
//!   made by the OS random generator and compared in constant time, and
//!   wrong guesses lock the PIN out for a growing time;
//! - the `Host` header must be an IP address or `localhost` (no DNS rebinding);
//! - it only searches names and lists results; downloading a result is a
//!   separate switch, off by default, and only serves a path that is in the
//!   index, looked up exactly (never a path built from the request);
//! - every text that goes into a page is HTML-escaped, and pages are sent
//!   with a strict Content-Security-Policy and no caching;
//! - connections, request sizes and time are limited.
//!
//! Plain `std::net` and a minimal HTTP/1.1 parser: one request per
//! connection (`Connection: close`), no chunked bodies.

use std::fmt::Write as _;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::index::{Entry, Index};
use crate::search::{Query, search_names};

#[cfg_attr(not(windows), allow(dead_code))]
pub const DEFAULT_PORT: u16 = 8765;
/// Connections handled at the same time; more are closed at once.
const MAX_CONNECTIONS: usize = 16;
/// Request line and headers together.
const MAX_HEAD: usize = 8 * 1024;
/// Only the PIN form has a body.
const MAX_BODY: usize = 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(10);
/// The whole request must arrive within this (slow senders are cut off).
const REQUEST_DEADLINE: Duration = Duration::from_secs(15);
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESULTS: usize = 200;
/// Longest query, in characters.
const MAX_QUERY: usize = 300;
const COOKIE: &str = "bahth_t";
/// Wrong PINs or tokens allowed before a lock-out.
const MAX_FAILURES: u32 = 5;
const FIRST_LOCKOUT: Duration = Duration::from_secs(30);
const MAX_LOCKOUT: Duration = Duration::from_secs(3600);
/// Failures older than this are forgotten.
const FAILURE_MEMORY: Duration = Duration::from_secs(600);

// ---------------------------------------------------------------- secrets

/// Fills `buf` from the operating system's random generator.
#[cfg(windows)]
pub fn random_bytes(buf: &mut [u8]) -> bool {
    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptGenRandom(alg: *mut std::ffi::c_void, buf: *mut u8, len: u32, flags: u32) -> i32;
    }
    const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 2;
    buf.chunks_mut(u32::MAX as usize).all(|c| unsafe {
        BCryptGenRandom(std::ptr::null_mut(), c.as_mut_ptr(), c.len() as u32, BCRYPT_USE_SYSTEM_PREFERRED_RNG) == 0
    })
}

#[cfg(not(windows))]
pub fn random_bytes(buf: &mut [u8]) -> bool {
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(buf)).is_ok()
}

/// A new 128-bit token as 32 lowercase hex digits; `None` if the OS
/// generator failed (the server then refuses to start).
pub fn new_token() -> Option<String> {
    let mut b = [0u8; 16];
    random_bytes(&mut b).then(|| b.iter().fold(String::new(), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    }))
}

/// A new 8-digit PIN, uniformly drawn (rejection sampling).
pub fn new_pin() -> Option<String> {
    const LIMIT: u32 = 4_200_000_000; // 42 × 10^8, below u32::MAX
    for _ in 0..64 {
        let mut b = [0u8; 4];
        if !random_bytes(&mut b) {
            return None;
        }
        let n = u32::from_le_bytes(b);
        if n < LIMIT {
            return Some(format!("{:08}", n % 100_000_000));
        }
    }
    None
}

pub fn valid_token(t: &str) -> bool {
    t.len() == 32 && t.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn valid_pin(p: &str) -> bool {
    p.len() == 8 && p.bytes().all(|b| b.is_ascii_digit())
}

/// The PIN as shown: `1234 5678`.
pub fn pin_display(p: &str) -> String {
    if p.len() == 8 && p.is_ascii() { format!("{} {}", &p[..4], &p[4..]) } else { p.to_string() }
}

/// Equal without returning early, so the time taken does not leak how many
/// leading characters matched. (Lengths are not secret.)
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ---------------------------------------------------------------- addresses

fn private_v4(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_private() || ip.is_link_local()
}

/// Loopback, private (RFC 1918) or link-local; for IPv6 also unique-local
/// (fc00::/7) and link-local (fe80::/10), and IPv4-mapped private addresses.
pub fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => private_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return private_v4(v4);
            }
            let s0 = v6.segments()[0];
            v6.is_loopback() || s0 & 0xffc0 == 0xfe80 || s0 & 0xfe00 == 0xfc00
        }
    }
}

/// A connection is served only when both ends are on a private network.
pub fn accept_ok(peer: IpAddr, local: IpAddr) -> bool {
    is_private(peer) && is_private(local)
}

/// This computer's private IPv4 addresses, found by asking the routing table
/// which local address would reach a few typical home-network and Internet
/// addresses (a UDP "connect" sends nothing).
pub fn lan_addresses() -> Vec<Ipv4Addr> {
    let probes = ["192.168.1.1:9", "192.168.0.1:9", "10.0.0.1:9", "172.16.0.1:9", "8.8.8.8:9"];
    let mut out: Vec<Ipv4Addr> = Vec::new();
    for p in probes {
        let Ok(s) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) else { continue };
        if s.connect(p).is_err() {
            continue;
        }
        if let Ok(SocketAddr::V4(a)) = s.local_addr() {
            let ip = *a.ip();
            if private_v4(ip) && !ip.is_loopback() && !out.contains(&ip) {
                out.push(ip);
            }
        }
    }
    out
}

/// `Host` must name an IP address or `localhost` (with an optional port), so
/// a web site that points its own name at this computer gets nothing.
pub fn host_ok(host: Option<&str>) -> bool {
    let Some(h) = host.map(str::trim) else { return false };
    let name = if let Some(rest) = h.strip_prefix('[') {
        // [v6]:port
        let Some((inner, after)) = rest.split_once(']') else { return false };
        if !(after.is_empty() || after.strip_prefix(':').is_some_and(|p| p.parse::<u16>().is_ok())) {
            return false;
        }
        return inner.parse::<std::net::Ipv6Addr>().is_ok();
    } else {
        match h.rsplit_once(':') {
            Some((n, p)) if p.parse::<u16>().is_ok() => n,
            Some(_) => return false,
            None => h,
        }
    };
    name.eq_ignore_ascii_case("localhost") || name.parse::<Ipv4Addr>().is_ok()
}

// ---------------------------------------------------------------- HTTP parsing

#[derive(Debug, PartialEq, Eq)]
pub enum HttpError {
    /// Malformed request: 400.
    Bad,
    /// Headers too large: 431.
    HeadTooLarge,
    /// Body too large: 413.
    BodyTooLarge,
    /// Too slow or the connection broke: just close.
    Io,
}

#[derive(Debug, Default)]
pub struct Request {
    pub method: String,
    /// Decoded path, e.g. `/` or `/f`.
    pub path: String,
    /// Decoded query parameters, in order.
    pub query: Vec<(String, String)>,
    /// Header names in lower case.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    pub fn param(&self, name: &str) -> Option<&str> {
        self.query.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    pub fn cookie(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .filter(|(n, _)| n == "cookie")
            .flat_map(|(_, v)| v.split(';'))
            .filter_map(|c| c.trim().split_once('='))
            .find(|(n, _)| n.trim() == name)
            .map(|(_, v)| v.trim())
    }

    /// `Content-Length`, or 0. Several different values are an error.
    fn content_length(&self) -> Result<usize, HttpError> {
        let mut len: Option<usize> = None;
        for (_, v) in self.headers.iter().filter(|(n, _)| n == "content-length") {
            let n: usize = v.trim().parse().map_err(|_| HttpError::Bad)?;
            if len.is_some_and(|l| l != n) {
                return Err(HttpError::Bad);
            }
            len = Some(n);
        }
        Ok(len.unwrap_or(0))
    }
}

/// Decodes `%XX` (and `+` as a space in forms and queries). Bad escapes stay
/// as they are; the bytes are read as UTF-8, invalid ones replaced.
pub fn percent_decode(s: &str, plus_is_space: bool) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
        match b[i] {
            b'%' if i + 2 < b.len() => {
                match (hex(b[i + 1]), hex(b[i + 2])) {
                    (Some(h), Some(l)) => {
                        out.push(h << 4 | l);
                        i += 3;
                        continue;
                    }
                    _ => out.push(b'%'),
                }
            }
            b'+' if plus_is_space => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encodes everything but the unreserved characters.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// `a=1&b=2` into decoded pairs.
pub fn parse_form(s: &str) -> Vec<(String, String)> {
    s.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (percent_decode(k, true), percent_decode(v, true))
        })
        .collect()
}

/// Parses the request line and headers (everything before the blank line).
pub fn parse_head(head: &[u8]) -> Result<Request, HttpError> {
    if head.len() > MAX_HEAD {
        return Err(HttpError::HeadTooLarge);
    }
    let head = std::str::from_utf8(head).map_err(|_| HttpError::Bad)?;
    let mut lines = head.split("\r\n");
    let line = lines.next().ok_or(HttpError::Bad)?;
    let mut parts = line.split(' ');
    let (Some(method), Some(target), Some(version), None) = (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(HttpError::Bad);
    };
    if method.is_empty() || !method.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err(HttpError::Bad);
    }
    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        return Err(HttpError::Bad);
    }
    // Origin form only: "/path?query". No absolute URLs, no "*".
    if !target.starts_with('/') || target.bytes().any(|b| b <= b' ' || b == 0x7f || b == b'#') {
        return Err(HttpError::Bad);
    }
    let (raw_path, raw_query) = target.split_once('?').unwrap_or((target, ""));
    let path = percent_decode(raw_path, false);
    let mut headers = Vec::new();
    for l in lines {
        if l.is_empty() {
            continue;
        }
        let (name, value) = l.split_once(':').ok_or(HttpError::Bad)?;
        // No folded lines, no spaces before the colon, no control characters.
        if name.is_empty()
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
            || value.bytes().any(|b| (b < b' ' && b != b'\t') || b == 0x7f)
        {
            return Err(HttpError::Bad);
        }
        headers.push((name.to_ascii_lowercase(), value.trim().to_string()));
        if headers.len() > 64 {
            return Err(HttpError::HeadTooLarge);
        }
    }
    let req = Request { method: method.to_string(), path, query: parse_form(raw_query), headers, body: Vec::new() };
    // Chunked bodies are refused (no request smuggling, nothing needs them).
    if req.header("transfer-encoding").is_some() {
        return Err(HttpError::Bad);
    }
    Ok(req)
}

fn find_blank_line(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Reads one request: the head up to [`MAX_HEAD`], then a body up to
/// [`MAX_BODY`], all within [`REQUEST_DEADLINE`].
pub fn read_request(r: &mut impl Read) -> Result<Request, HttpError> {
    let start = Instant::now();
    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    let end = loop {
        if let Some(i) = find_blank_line(&buf) {
            break i;
        }
        if buf.len() > MAX_HEAD {
            return Err(HttpError::HeadTooLarge);
        }
        if start.elapsed() > REQUEST_DEADLINE {
            return Err(HttpError::Io);
        }
        let n = r.read(&mut chunk).map_err(|_| HttpError::Io)?;
        if n == 0 {
            return Err(if buf.is_empty() { HttpError::Io } else { HttpError::Bad });
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let mut req = parse_head(&buf[..end])?;
    let len = req.content_length()?;
    if len > MAX_BODY {
        return Err(HttpError::BodyTooLarge);
    }
    let mut body = buf[end + 4..].to_vec();
    while body.len() < len {
        if start.elapsed() > REQUEST_DEADLINE {
            return Err(HttpError::Io);
        }
        let n = r.read(&mut chunk).map_err(|_| HttpError::Io)?;
        if n == 0 {
            return Err(HttpError::Bad);
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(len);
    req.body = body;
    Ok(req)
}

// ---------------------------------------------------------------- responses

pub enum Body {
    Bytes(Vec<u8>),
    File(std::fs::File, u64),
}

pub struct Response {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Body,
}

impl Response {
    fn html(status: u16, page: String) -> Response {
        Response {
            status,
            headers: vec![("Content-Type", "text/html; charset=utf-8".into())],
            body: Body::Bytes(page.into_bytes()),
        }
    }

    fn text(status: u16, text: &str) -> Response {
        Response {
            status,
            headers: vec![("Content-Type", "text/plain; charset=utf-8".into())],
            body: Body::Bytes(text.as_bytes().to_vec()),
        }
    }

    fn redirect(location: String, cookie: Option<String>) -> Response {
        let mut headers = vec![("Location", location)];
        if let Some(c) = cookie {
            headers.push(("Set-Cookie", c));
        }
        Response { status: 303, headers, body: Body::Bytes(Vec::new()) }
    }

    #[cfg(test)]
    pub fn body_bytes(&self) -> &[u8] {
        match &self.body {
            Body::Bytes(b) => b,
            Body::File(..) => &[],
        }
    }

    #[cfg(test)]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        303 => "See Other",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        413 => "Content Too Large",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

const SECURITY_HEADERS: &[(&str, &str)] = &[
    ("Cache-Control", "no-store"),
    ("X-Content-Type-Options", "nosniff"),
    ("X-Frame-Options", "DENY"),
    ("Referrer-Policy", "no-referrer"),
    (
        "Content-Security-Policy",
        "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'",
    ),
    ("Connection", "close"),
];

fn write_response(w: &mut impl Write, resp: Response, head_only: bool, stop: &AtomicBool) -> io::Result<()> {
    let len = match &resp.body {
        Body::Bytes(b) => b.len() as u64,
        Body::File(_, n) => *n,
    };
    let mut head = format!("HTTP/1.1 {} {}\r\n", resp.status, reason(resp.status));
    for (n, v) in SECURITY_HEADERS {
        let _ = write!(head, "{n}: {v}\r\n");
    }
    for (n, v) in &resp.headers {
        // Values are ours, but never let a line break through.
        let v: String = v.chars().filter(|c| !c.is_control()).collect();
        let _ = write!(head, "{n}: {v}\r\n");
    }
    let _ = write!(head, "Content-Length: {len}\r\n\r\n");
    w.write_all(head.as_bytes())?;
    if head_only {
        return w.flush();
    }
    match resp.body {
        Body::Bytes(b) => w.write_all(&b)?,
        Body::File(f, n) => {
            let mut f = f.take(n);
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                if stop.load(Ordering::Relaxed) {
                    return Err(io::Error::other("server stopped"));
                }
                let k = f.read(&mut buf)?;
                if k == 0 {
                    break;
                }
                w.write_all(&buf[..k])?;
            }
        }
    }
    w.flush()
}

// ---------------------------------------------------------------- HTML

/// Escapes text for HTML content and quoted attributes.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

const STYLE: &str = "body{font-family:system-ui,'Segoe UI',Tahoma,sans-serif;margin:0;background:#f6f7f9;color:#1b1f24}\
header{background:#1e5aa8;color:#fff;padding:12px 16px;font-size:18px}\
main{padding:12px 16px;max-width:760px;margin:auto}\
form{display:flex;gap:8px;margin:8px 0 12px}\
input{flex:1;font-size:17px;padding:10px;border:1px solid #b9c0ca;border-radius:8px;min-width:0}\
button{font-size:17px;padding:10px 16px;border:0;border-radius:8px;background:#1e5aa8;color:#fff}\
ul{list-style:none;padding:0;margin:0}\
li{background:#fff;border-radius:8px;padding:10px 12px;margin-bottom:8px;box-shadow:0 1px 2px #0002;overflow-wrap:anywhere}\
.n{font-weight:600;font-size:16px}.n a{color:#1e5aa8}\
.f{color:#555;font-size:13px;margin-top:2px}.m{color:#777;font-size:13px;margin-top:2px}\
.e{color:#b00020}.i{color:#555}\
@media (prefers-color-scheme:dark){body{background:#15181c;color:#e6e8eb}li{background:#22262b}\
.f,.i{color:#aab}.m{color:#99a}input{background:#22262b;color:#e6e8eb;border-color:#444}.n a{color:#8ab4f8}}";

fn page(title: &str, body: &str) -> String {
    let lang = crate::i18n::current();
    let (code, dir) = (lang.code(), lang.pick("rtl", "ltr"));
    format!(
        "<!doctype html><html lang=\"{code}\" dir=\"{dir}\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<meta name=\"referrer\" content=\"no-referrer\"><title>{}</title><style>{STYLE}</style></head>\
<body><header>{}</header><main>{body}</main></body></html>",
        esc(title),
        esc(tr!("بحث سريع", "Bahth Sari'"))
    )
}

fn login_page(message: Option<&str>) -> String {
    let mut body = String::new();
    if let Some(m) = message {
        let _ = write!(body, "<p class=\"e\">{}</p>", esc(m));
    }
    let _ = write!(
        body,
        "<p>{}</p><form method=\"post\" action=\"/login\">\
<input name=\"pin\" inputmode=\"numeric\" autocomplete=\"off\" maxlength=\"16\" placeholder=\"{}\" autofocus>\
<button type=\"submit\">{}</button></form>",
        esc(tr!(
            "اكتب الرمز السري الظاهر في نافذة «البحث من الهاتف» على الحاسوب.",
            "Type the PIN shown in the \"Search from phone\" window on the computer."
        )),
        esc(tr!("الرمز السري", "PIN")),
        esc(tr!("دخول", "Enter")),
    );
    page(tr!("الرمز السري", "PIN"), &body)
}

fn message_page(text: &str) -> String {
    page(tr!("بحث سريع", "Bahth Sari'"), &format!("<p class=\"e\">{}</p>", esc(text)))
}

fn fmt_size(e: &Entry) -> String {
    if e.is_dir {
        return tr!("مجلد", "Folder").into();
    }
    let b = e.size as f64;
    if b < 1024.0 {
        trf!("{} بايت", "{} bytes", e.size)
    } else if b < 1024.0 * 1024.0 {
        trf!("{:.1} ك.ب", "{:.1} KB", b / 1024.0)
    } else if b < 1024.0 * 1024.0 * 1024.0 {
        trf!("{:.1} م.ب", "{:.1} MB", b / 1024.0 / 1024.0)
    } else {
        trf!("{:.2} ج.ب", "{:.2} GB", b / 1024.0 / 1024.0 / 1024.0)
    }
}

/// Local `YYYY-MM-DD HH:MM`, empty when unknown.
fn fmt_date(mtime: i64) -> String {
    if mtime <= 0 {
        return String::new();
    }
    let iso = crate::cli::iso_time(mtime + crate::search::utc_offset_secs() as i64);
    iso.get(..16).map(|s| s.replace('T', " ")).unwrap_or_default()
}

fn folder_of(path: &str) -> &str {
    let t = path.trim_end_matches(['\\', '/']);
    match t.rfind(['\\', '/']) {
        Some(0) => &t[..1],
        Some(i) if t[..i].ends_with(':') => &t[..=i],
        Some(i) => &t[..i],
        None => "",
    }
}

fn search_page(q: &str, index: &Index, downloads: bool) -> String {
    let mut body = String::new();
    let _ = write!(
        body,
        "<form method=\"get\" action=\"/\"><input name=\"q\" type=\"search\" value=\"{}\" placeholder=\"{}\" \
autocomplete=\"off\" autofocus><button type=\"submit\">{}</button></form>",
        esc(q),
        esc(tr!("اسم الملف أو كلمة…", "File name or a word…")),
        esc(tr!("بحث", "Search")),
    );
    let query = Query::parse(q);
    if q.trim().is_empty() || query.is_empty() {
        let _ = write!(
            body,
            "<p class=\"i\">{}</p>",
            esc(tr!(
                "ابحث في أسماء ملفات هذا الحاسوب. يمكن استخدام نفس صيغ البحث: ext:pdf و dm:today و size:>10mb…",
                "Search the names of this PC's files. The same syntax works: ext:pdf, dm:today, size:>10mb…"
            ))
        );
        return page(tr!("بحث سريع", "Bahth Sari'"), &body);
    }
    if index.entries.is_empty() {
        let _ = write!(body, "<p class=\"e\">{}</p>", esc(tr!("الفهرس غير جاهز بعد. حاول بعد قليل.", "The index is not ready yet. Try again shortly.")));
        return page(q, &body);
    }
    let hits = search_names(&index.entries, &query, "");
    let total = hits.len();
    let shown = total.min(MAX_RESULTS);
    let summary = if total > shown {
        trf!("{total} نتيجة (تظهر أول {shown})", "{total} results (showing the first {shown})")
    } else {
        trf!("{total} نتيجة", "{total} results")
    };
    let _ = write!(body, "<p class=\"i\">{}</p><ul>", esc(&summary));
    for e in hits.iter().take(shown).filter_map(|&i| index.entries.get(i as usize)) {
        let name = esc(e.name());
        let name = if downloads && !e.is_dir {
            format!("<a href=\"/f?p={}\">{name}</a>", percent_encode(&e.path))
        } else {
            name
        };
        let _ = write!(
            body,
            "<li><div class=\"n\"><bdi>{name}</bdi></div><div class=\"f\"><bdi dir=\"ltr\">{}</bdi></div>\
<div class=\"m\">{} · <bdi dir=\"ltr\">{}</bdi></div></li>",
            esc(folder_of(&e.path)),
            esc(&fmt_size(e)),
            esc(&fmt_date(e.mtime)),
        );
    }
    body.push_str("</ul>");
    page(q, &body)
}

// ---------------------------------------------------------------- auth and routing

/// Lock-out after repeated wrong PINs or tokens. One counter for the whole
/// server, so changing address does not buy more guesses.
#[derive(Default)]
pub struct Limiter {
    failures: u32,
    strikes: u32,
    last_failure: Option<Instant>,
    locked_until: Option<Instant>,
}

impl Limiter {
    /// `Err(remaining)` while locked out.
    pub fn check(&mut self, now: Instant) -> Result<(), Duration> {
        if let Some(until) = self.locked_until {
            if now < until {
                return Err(until - now);
            }
            self.locked_until = None;
        }
        if self.last_failure.is_some_and(|t| now.duration_since(t) > FAILURE_MEMORY) {
            self.failures = 0;
            self.strikes = 0;
            self.last_failure = None;
        }
        Ok(())
    }

    pub fn fail(&mut self, now: Instant) {
        self.failures += 1;
        self.last_failure = Some(now);
        if self.failures >= MAX_FAILURES {
            self.failures = 0;
            let lock = FIRST_LOCKOUT.saturating_mul(1u32 << self.strikes.min(16)).min(MAX_LOCKOUT);
            self.strikes += 1;
            self.locked_until = Some(now + lock);
        }
    }

    pub fn success(&mut self) {
        *self = Limiter::default();
    }
}

#[derive(Clone)]
struct Secrets {
    token: String,
    pin: String,
}

/// What the connection threads share with the UI.
pub struct Shared {
    secrets: RwLock<Secrets>,
    pub allow_download: AtomicBool,
    index: RwLock<Arc<Index>>,
    limiter: Mutex<Limiter>,
}

impl Shared {
    pub fn new(token: String, pin: String, allow_download: bool) -> Shared {
        Shared {
            secrets: RwLock::new(Secrets { token, pin }),
            allow_download: AtomicBool::new(allow_download),
            index: RwLock::new(Arc::default()),
            limiter: Mutex::new(Limiter::default()),
        }
    }

    /// New secrets: every phone has to log in again.
    pub fn set_secrets(&self, token: String, pin: String) {
        *self.secrets.write().unwrap_or_else(|e| e.into_inner()) = Secrets { token, pin };
    }

    pub fn index(&self) -> Arc<Index> {
        self.index.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_index(&self, index: Arc<Index>) {
        *self.index.write().unwrap_or_else(|e| e.into_inner()) = index;
    }

    fn secrets(&self) -> Secrets {
        self.secrets.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn limiter(&self) -> std::sync::MutexGuard<'_, Limiter> {
        self.limiter.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn session_cookie(token: &str) -> String {
    format!("{COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000")
}

fn locked_response(wait: Duration) -> Response {
    let mins = wait.as_secs().div_ceil(60).max(1);
    let mut r = Response::html(
        429,
        login_page(Some(&trf!(
            "محاولات خاطئة كثيرة. حاول بعد {mins} دقيقة.",
            "Too many wrong attempts. Try again in {mins} min."
        ))),
    );
    r.headers.push(("Retry-After", wait.as_secs().max(1).to_string()));
    r
}

/// Where to go after a `?t=` login: the same page without the token. Only
/// our own two pages, so it can never redirect elsewhere.
fn after_login(req: &Request) -> String {
    let base = if req.path == "/f" { "/f" } else { "/" };
    let rest: Vec<String> = req
        .query
        .iter()
        .filter(|(k, _)| k != "t")
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
        .collect();
    if rest.is_empty() { base.to_string() } else { format!("{base}?{}", rest.join("&")) }
}

/// Answers one request. `peer` has already passed [`accept_ok`]; it is
/// checked again so this function is safe on its own.
pub fn handle(req: &Request, peer: IpAddr, shared: &Shared, now: Instant) -> Response {
    if !is_private(peer) {
        return Response::text(403, "Forbidden");
    }
    if !host_ok(req.header("host")) {
        return Response::text(400, "Bad Host");
    }
    let is_get = req.method == "GET" || req.method == "HEAD";
    let is_login = req.path == "/login" && req.method == "POST";
    if !is_get && !is_login {
        let mut r = Response::text(405, "Method Not Allowed");
        r.headers.push(("Allow", "GET, HEAD, POST".into()));
        return r;
    }
    let secrets = shared.secrets();

    if is_login {
        if let Err(wait) = shared.limiter().check(now) {
            return locked_response(wait);
        }
        let form = parse_form(&String::from_utf8_lossy(&req.body));
        let typed: String = form
            .iter()
            .find(|(k, _)| k == "pin")
            .map(|(_, v)| v.chars().filter(|c| !c.is_whitespace()).map(arabic_digit_to_ascii).collect())
            .unwrap_or_default();
        if ct_eq(typed.as_bytes(), secrets.pin.as_bytes()) {
            shared.limiter().success();
            return Response::redirect("/".into(), Some(session_cookie(&secrets.token)));
        }
        shared.limiter().fail(now);
        return Response::html(401, login_page(Some(tr!("الرمز السري غير صحيح.", "Wrong PIN."))));
    }

    if let Some(t) = req.param("t") {
        if let Err(wait) = shared.limiter().check(now) {
            return locked_response(wait);
        }
        if ct_eq(t.as_bytes(), secrets.token.as_bytes()) {
            shared.limiter().success();
            return Response::redirect(after_login(req), Some(session_cookie(&secrets.token)));
        }
        shared.limiter().fail(now);
        return Response::html(401, login_page(Some(tr!("الرابط قديم أو غير صحيح.", "The link is old or wrong."))));
    }

    let authed = req.cookie(COOKIE).is_some_and(|c| ct_eq(c.as_bytes(), secrets.token.as_bytes()));
    if !authed {
        return Response::html(401, login_page(None));
    }

    match req.path.as_str() {
        "/" => {
            let q: String = req.param("q").unwrap_or("").chars().take(MAX_QUERY).collect();
            let downloads = shared.allow_download.load(Ordering::Relaxed);
            Response::html(200, search_page(&q, &shared.index(), downloads))
        }
        "/f" => download(req.param("p").unwrap_or(""), shared),
        "/favicon.ico" => Response::text(404, "Not Found"),
        _ => Response::html(404, message_page(tr!("الصفحة غير موجودة.", "Page not found."))),
    }
}

fn arabic_digit_to_ascii(c: char) -> char {
    match c {
        '\u{0660}'..='\u{0669}' => char::from(b'0' + (c as u32 - 0x0660) as u8),
        '\u{06F0}'..='\u{06F9}' => char::from(b'0' + (c as u32 - 0x06F0) as u8),
        c => c,
    }
}

/// Sends a result file: only when allowed, only a file the index has at
/// exactly this path, and only if it is still a regular file.
fn download(path: &str, shared: &Shared) -> Response {
    if !shared.allow_download.load(Ordering::Relaxed) {
        return Response::html(403, message_page(tr!("تنزيل الملفات غير مفعّل على الحاسوب.", "Downloading files is turned off on the computer.")));
    }
    let not_found = || Response::html(404, message_page(tr!("الملف غير موجود في الفهرس.", "The file is not in the index.")));
    if path.is_empty() || path.contains('\0') {
        return not_found();
    }
    let index = shared.index();
    let Some(entry) = index.find(path).and_then(|i| index.entries.get(i)) else { return not_found() };
    if entry.is_dir {
        return not_found();
    }
    let opened = std::fs::File::open(&*entry.path).and_then(|f| {
        let m = f.metadata()?;
        if m.is_file() { Ok((f, m.len())) } else { Err(io::Error::other("not a file")) }
    });
    let Ok((file, len)) = opened else {
        return Response::html(404, message_page(tr!("تعذر فتح الملف.", "Could not open the file.")));
    };
    let name = entry.name();
    let ascii: String = name.chars().map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' { c } else { '_' }).collect();
    Response {
        status: 200,
        headers: vec![
            ("Content-Type", "application/octet-stream".into()),
            ("Content-Disposition", format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{}", percent_encode(name))),
        ],
        body: Body::File(file, len),
    }
}

// ---------------------------------------------------------------- server

fn error_response(e: &HttpError) -> Option<Response> {
    match e {
        HttpError::Bad => Some(Response::text(400, "Bad Request")),
        HttpError::HeadTooLarge => Some(Response::text(431, "Request Header Fields Too Large")),
        HttpError::BodyTooLarge => Some(Response::text(413, "Content Too Large")),
        HttpError::Io => None,
    }
}

fn serve_connection(mut stream: TcpStream, peer: IpAddr, shared: &Shared, stop: &AtomicBool) {
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let _ = stream.set_nodelay(true);
    match read_request(&mut stream) {
        Ok(req) => {
            let head_only = req.method == "HEAD";
            let resp = handle(&req, peer, shared, Instant::now());
            let _ = write_response(&mut stream, resp, head_only, stop);
        }
        Err(e) => {
            if let Some(resp) = error_response(&e) {
                let _ = write_response(&mut stream, resp, false, stop);
            }
        }
    }
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

/// Decrements the connection count when a connection thread ends.
struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn accept_loop(listener: TcpListener, shared: Arc<Shared>, stop: Arc<AtomicBool>, active: Arc<AtomicUsize>) {
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, peer)) => {
                // Accepted sockets inherit non-blocking mode on Windows.
                let _ = stream.set_nonblocking(false);
                let local = stream.local_addr().map(|a| a.ip()).unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
                if !accept_ok(peer.ip(), local) {
                    continue; // dropped: closes the connection
                }
                if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                    active.fetch_sub(1, Ordering::SeqCst);
                    continue;
                }
                let slot = Slot(active.clone());
                let (shared, stop) = (shared.clone(), stop.clone());
                let spawned = std::thread::Builder::new().name("web-conn".into()).spawn(move || {
                    let _slot = slot;
                    serve_connection(stream, peer.ip(), &shared, &stop);
                });
                if spawned.is_err() {
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// A running server; stops when dropped.
pub struct Server {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
    /// The addresses it listens on, LAN ones first.
    pub addrs: Vec<SocketAddr>,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl Server {
    /// Listens on `port` of every private LAN address and of 127.0.0.1.
    /// Fails if none of them can be bound (e.g. the port is taken).
    pub fn start(port: u16, shared: Arc<Shared>) -> io::Result<Server> {
        let mut ips: Vec<Ipv4Addr> = lan_addresses();
        ips.push(Ipv4Addr::LOCALHOST);
        Server::start_on(&ips, port, shared)
    }

    pub fn start_on(ips: &[Ipv4Addr], port: u16, shared: Arc<Shared>) -> io::Result<Server> {
        let stop = Arc::new(AtomicBool::new(false));
        let active = Arc::new(AtomicUsize::new(0));
        let mut threads = Vec::new();
        let mut addrs = Vec::new();
        let mut last_err = None;
        for &ip in ips {
            if !private_v4(ip) {
                continue;
            }
            let listener = match TcpListener::bind((ip, port)).and_then(|l| l.set_nonblocking(true).map(|_| l)) {
                Ok(l) => l,
                Err(e) => {
                    last_err = Some(e);
                    continue;
                }
            };
            if let Ok(a) = listener.local_addr() {
                addrs.push(a);
            }
            let (shared, stop, active) = (shared.clone(), stop.clone(), active.clone());
            match std::thread::Builder::new()
                .name("web-accept".into())
                .spawn(move || accept_loop(listener, shared, stop, active))
            {
                Ok(t) => threads.push(t),
                Err(e) => last_err = Some(e),
            }
        }
        if threads.is_empty() {
            return Err(last_err.unwrap_or_else(|| io::Error::other("no private address to listen on")));
        }
        Ok(Server { stop, threads, addrs })
    }

    /// The LAN addresses (without 127.0.0.1), for the phone.
    pub fn lan_addrs(&self) -> impl Iterator<Item = &SocketAddr> {
        self.addrs.iter().filter(|a| !a.ip().is_loopback())
    }

    /// Stops accepting and waits for the listeners to close (at most about
    /// 100 ms); open connections end at their next read or write.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv6Addr;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";
    const PIN: &str = "12345678";
    const LAN: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20));

    fn entry(path: &str, size: u64, is_dir: bool) -> Entry {
        let mut e = Entry { path: path.into(), size, mtime: 1_700_000_000, is_dir, norm_name: "".into() };
        e.norm_name = crate::normalize::normalize(e.name()).into();
        e
    }

    fn shared_with(entries: Vec<Entry>) -> Shared {
        let s = Shared::new(TOKEN.into(), PIN.into(), false);
        let mut entries = entries;
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        s.set_index(Arc::new(Index { entries, roots: vec![], built_at: 0 }));
        s
    }

    fn req(raw: &str) -> Request {
        read_request(&mut raw.as_bytes()).expect("parses")
    }

    fn get(path: &str, cookie: Option<&str>) -> Request {
        let c = cookie.map(|c| format!("Cookie: {COOKIE}={c}\r\n")).unwrap_or_default();
        req(&format!("GET {path} HTTP/1.1\r\nHost: 192.168.1.5:8765\r\n{c}\r\n"))
    }

    fn body(r: &Response) -> String {
        String::from_utf8_lossy(r.body_bytes()).into_owned()
    }

    #[test]
    fn private_addresses() {
        for ip in ["10.1.2.3", "172.16.0.1", "172.31.255.255", "192.168.0.10", "169.254.3.4", "127.0.0.1"] {
            assert!(is_private(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "172.32.0.1", "172.15.255.255", "192.169.0.1", "100.64.0.1", "0.0.0.0", "1.1.1.1"] {
            assert!(!is_private(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["::1", "fe80::1", "fd12:3456::1", "::ffff:192.168.1.2"] {
            assert!(is_private(ip.parse::<Ipv6Addr>().unwrap().into()), "{ip}");
        }
        for ip in ["2001:4860::8888", "::ffff:8.8.8.8", "::"] {
            assert!(!is_private(ip.parse::<Ipv6Addr>().unwrap().into()), "{ip}");
        }
        assert!(accept_ok(LAN, "192.168.1.5".parse().unwrap()));
        assert!(!accept_ok("8.8.8.8".parse().unwrap(), "192.168.1.5".parse().unwrap()));
        assert!(!accept_ok(LAN, "203.0.113.5".parse().unwrap()));
    }

    #[test]
    fn lan_addresses_are_private() {
        assert!(lan_addresses().iter().all(|ip| private_v4(*ip) && !ip.is_loopback()));
    }

    #[test]
    fn hosts() {
        for h in ["192.168.1.5:8765", "127.0.0.1", "localhost:8765", "LOCALHOST", "[::1]:8765", "[fe80::1]"] {
            assert!(host_ok(Some(h)), "{h}");
        }
        for h in ["evil.example:8765", "evil.example", "192.168.1.5:x", "[::1", "", "192.168.1.5.nip.io"] {
            assert!(!host_ok(Some(h)), "{h}");
        }
        assert!(!host_ok(None));
    }

    #[test]
    fn secrets_from_the_os_generator() {
        let (a, b) = (new_token().unwrap(), new_token().unwrap());
        assert!(valid_token(&a) && valid_token(&b));
        assert_ne!(a, b);
        let p = new_pin().unwrap();
        assert!(valid_pin(&p), "{p}");
        assert_eq!(pin_display("12345678"), "1234 5678");
        assert!(!valid_token("xyz") && !valid_pin("1234567a"));
    }

    #[test]
    fn constant_time_compare() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"abcd"));
        assert!(ct_eq(b"", b""));
    }

    #[test]
    fn parses_requests() {
        let r = req("GET /?q=%D9%81%D8%A7%D8%AA%D9%88%D8%B1%D8%A9+pdf&t=x HTTP/1.1\r\nHost: a\r\nCookie: x=1; bahth_t=abc\r\n\r\n");
        assert_eq!(r.method, "GET");
        assert_eq!(r.path, "/");
        assert_eq!(r.param("q"), Some("فاتورة pdf"));
        assert_eq!(r.param("t"), Some("x"));
        assert_eq!(r.header("host"), Some("a"));
        assert_eq!(r.cookie(COOKIE), Some("abc"));
        assert_eq!(r.cookie("x"), Some("1"));
        assert_eq!(r.cookie("y"), None);

        let r = req("POST /login HTTP/1.1\r\nHost: a\r\nContent-Length: 12\r\n\r\npin=12345678EXTRA");
        assert_eq!(r.body, b"pin=12345678");
    }

    #[test]
    fn rejects_bad_requests() {
        let bad = |raw: &str| read_request(&mut raw.as_bytes()).err();
        assert_eq!(bad("GET / HTTP/2.0\r\n\r\n"), Some(HttpError::Bad));
        assert_eq!(bad("GET http://x/ HTTP/1.1\r\n\r\n"), Some(HttpError::Bad));
        assert_eq!(bad("GET  / HTTP/1.1\r\n\r\n"), Some(HttpError::Bad));
        assert_eq!(bad("get / HTTP/1.1\r\n\r\n"), Some(HttpError::Bad));
        assert_eq!(bad("GET / HTTP/1.1\r\nBad Header: x\r\n\r\n"), Some(HttpError::Bad));
        assert_eq!(bad("GET / HTTP/1.1\r\n folded\r\n\r\n"), Some(HttpError::Bad));
        assert_eq!(bad("POST /login HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n"), Some(HttpError::Bad));
        assert_eq!(bad("POST /login HTTP/1.1\r\nContent-Length: 5\r\nContent-Length: 6\r\n\r\n"), Some(HttpError::Bad));
        assert_eq!(bad("POST /login HTTP/1.1\r\nContent-Length: 99999\r\n\r\n"), Some(HttpError::BodyTooLarge));
        assert_eq!(bad("POST /login HTTP/1.1\r\nContent-Length: 10\r\n\r\nshort"), Some(HttpError::Bad));
        let huge = format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(MAX_HEAD));
        assert_eq!(bad(&huge), Some(HttpError::HeadTooLarge));
        let unfinished = format!("GET / HTTP/1.1\r\nX: {}", "a".repeat(MAX_HEAD + 10));
        assert_eq!(bad(&unfinished), Some(HttpError::HeadTooLarge));
        assert_eq!(bad(""), Some(HttpError::Io));
        assert_eq!(bad("GET / HTTP/1.1\r\n"), Some(HttpError::Bad));
        let many: String = (0..70).map(|i| format!("X{i}: 1\r\n")).collect();
        assert_eq!(bad(&format!("GET / HTTP/1.1\r\n{many}\r\n")), Some(HttpError::HeadTooLarge));
    }

    #[test]
    fn percent_coding() {
        assert_eq!(percent_decode("a%20b+c", true), "a b c");
        assert_eq!(percent_decode("a%20b+c", false), "a b+c");
        assert_eq!(percent_decode("%zz%4", true), "%zz%4");
        assert_eq!(percent_decode("%", true), "%");
        assert_eq!(percent_decode("%41", true), "A");
        assert_eq!(percent_decode("%ff", true), "\u{FFFD}");
        let p = r"C:\docs\فاتورة & co.pdf";
        assert_eq!(percent_decode(&percent_encode(p), true), p);
        assert!(percent_encode(p).bytes().all(|b| b.is_ascii_alphanumeric() || b"%-_.~".contains(&b)));
    }

    #[test]
    fn escapes_html() {
        assert_eq!(esc(r#"<script>alert('x')</script> & "q""#), "&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt; &amp; &quot;q&quot;");
    }

    #[test]
    fn needs_the_token() {
        let s = shared_with(vec![entry(r"C:\docs\report.pdf", 10, false)]);
        let now = Instant::now();
        // No cookie: the PIN page, nothing else.
        let r = handle(&get("/?q=report", None), LAN, &s, now);
        assert_eq!(r.status, 401);
        assert!(!body(&r).contains("report.pdf"));
        assert!(body(&r).contains("/login"));
        // Wrong cookie.
        assert_eq!(handle(&get("/?q=report", Some("0123456789abcdef0123456789abcdee")), LAN, &s, now).status, 401);
        // Right cookie.
        let r = handle(&get("/?q=report", Some(TOKEN)), LAN, &s, now);
        assert_eq!(r.status, 200);
        assert!(body(&r).contains("report.pdf"));
        // A public peer is refused even with the token.
        assert_eq!(handle(&get("/?q=report", Some(TOKEN)), "8.8.8.8".parse().unwrap(), &s, now).status, 403);
        // A foreign Host (DNS rebinding) is refused.
        let r = req(&format!("GET /?q=report HTTP/1.1\r\nHost: evil.example\r\nCookie: {COOKIE}={TOKEN}\r\n\r\n"));
        assert_eq!(handle(&r, LAN, &s, now).status, 400);
        // Other methods.
        let r = req("PUT / HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
        assert_eq!(handle(&r, LAN, &s, now).status, 405);
    }

    #[test]
    fn token_link_sets_the_cookie_and_hides_the_token() {
        let s = shared_with(vec![]);
        let r = handle(&get(&format!("/?t={TOKEN}&q=a%26b"), None), LAN, &s, Instant::now());
        assert_eq!(r.status, 303);
        assert_eq!(r.header("Location"), Some("/?q=a%26b"));
        let c = r.header("Set-Cookie").unwrap();
        assert!(c.starts_with(&format!("{COOKIE}={TOKEN};")));
        assert!(c.contains("HttpOnly") && c.contains("SameSite=Strict"));
        // Odd paths never become the redirect target.
        let r = handle(&get(&format!("//evil.example/?t={TOKEN}"), None), LAN, &s, Instant::now());
        assert_eq!(r.header("Location"), Some("/"));
    }

    fn login(s: &Shared, pin: &str, now: Instant) -> Response {
        let b = format!("pin={}", percent_encode(pin));
        let r = req(&format!("POST /login HTTP/1.1\r\nHost: 127.0.0.1:8765\r\nContent-Length: {}\r\n\r\n{b}", b.len()));
        handle(&r, LAN, s, now)
    }

    #[test]
    fn pin_login_and_lockout() {
        let s = shared_with(vec![]);
        let now = Instant::now();
        let r = login(&s, "1234 5678", now);
        assert_eq!(r.status, 303);
        assert!(r.header("Set-Cookie").unwrap().contains(TOKEN));
        // Arabic-Indic digits are accepted too.
        assert_eq!(login(&s, "١٢٣٤٥٦٧٨", now).status, 303);
        for _ in 0..MAX_FAILURES {
            assert_eq!(login(&s, "00000000", now).status, 401);
        }
        // Locked: even the right PIN waits.
        let r = login(&s, PIN, now);
        assert_eq!(r.status, 429);
        assert!(r.header("Retry-After").is_some());
        assert_eq!(handle(&get(&format!("/?t={TOKEN}"), None), LAN, &s, now).status, 429);
        // After the lock-out it works again; the next lock-out is longer.
        let later = now + FIRST_LOCKOUT + Duration::from_secs(1);
        for _ in 0..MAX_FAILURES {
            login(&s, "00000000", later);
        }
        assert_eq!(login(&s, PIN, later + FIRST_LOCKOUT + Duration::from_secs(1)).status, 429);
        assert_eq!(login(&s, PIN, later + 2 * FIRST_LOCKOUT + Duration::from_secs(1)).status, 303);
        // Wrong tokens count as failures too.
        let t = now + Duration::from_secs(10_000);
        for _ in 0..MAX_FAILURES {
            assert_eq!(handle(&get("/?t=bad", None), LAN, &s, t).status, 401);
        }
        assert_eq!(login(&s, PIN, t).status, 429);
    }

    #[test]
    fn new_secrets_log_everyone_out() {
        let s = shared_with(vec![]);
        s.set_secrets("ffffffffffffffffffffffffffffffff".into(), "87654321".into());
        assert_eq!(handle(&get("/", Some(TOKEN)), LAN, &s, Instant::now()).status, 401);
        assert_eq!(login(&s, PIN, Instant::now()).status, 401);
        assert_eq!(login(&s, "87654321", Instant::now()).status, 303);
    }

    #[test]
    fn results_are_escaped() {
        let s = shared_with(vec![entry(r"C:\x\<img src=x onerror=alert(1)>.txt", 2048, false), entry(r"C:\x\a&b", 0, true)]);
        let r = handle(&get("/?q=%3Cimg", Some(TOKEN)), LAN, &s, Instant::now());
        let b = body(&r);
        assert!(!b.contains("<img"), "{b}");
        assert!(b.contains("&lt;img src=x onerror=alert(1)&gt;.txt"));
        // The query echoed in the box is escaped too.
        let r = handle(&get("/?q=%22%3E%3Cscript%3E", Some(TOKEN)), LAN, &s, Instant::now());
        assert!(!body(&r).contains("<script>"));
        assert!(body(&r).contains("&quot;&gt;&lt;script&gt;"));
        // Page direction follows the language (Arabic in the tests).
        assert!(body(&r).contains("dir=\"rtl\""));
        assert!(r.header("Content-Type").unwrap().starts_with("text/html"));
    }

    #[test]
    fn result_details() {
        let s = shared_with(vec![entry(r"C:\docs\تقرير.pdf", 2048, false), entry(r"C:\docs\تقارير", 0, true)]);
        let b = body(&handle(&get("/?q=%D8%AA%D9%82%D8%B1%D9%8A%D8%B1", Some(TOKEN)), LAN, &s, Instant::now()));
        assert!(b.contains("تقرير.pdf"));
        assert!(b.contains(r"C:\docs"));
        assert!(b.contains("2.0 ك.ب"));
        // Downloads are off: no links.
        assert!(!b.contains("/f?p="));
        assert_eq!(folder_of(r"C:\a.txt"), r"C:\");
        assert_eq!(folder_of(r"C:\d\a.txt"), r"C:\d");
        assert_eq!(folder_of("/tmp/a"), "/tmp");
        assert_eq!(folder_of("/a"), "/");
    }

    #[test]
    fn downloads_only_indexed_files_when_allowed() {
        let dir = std::env::temp_dir().join(format!("bahth-web-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("ملف.txt");
        std::fs::write(&file, b"hello").unwrap();
        let secret = dir.join("secret.txt");
        std::fs::write(&secret, b"secret").unwrap();
        let path = file.to_string_lossy().into_owned();
        let s = shared_with(vec![entry(&path, 5, false), entry(&dir.to_string_lossy(), 0, true)]);
        let url = format!("/f?p={}", percent_encode(&path));

        // Off by default.
        assert_eq!(handle(&get(&url, Some(TOKEN)), LAN, &s, Instant::now()).status, 403);
        s.allow_download.store(true, Ordering::Relaxed);
        // Needs the token.
        assert_eq!(handle(&get(&url, None), LAN, &s, Instant::now()).status, 401);
        let r = handle(&get(&url, Some(TOKEN)), LAN, &s, Instant::now());
        assert_eq!(r.status, 200);
        assert!(matches!(r.body, Body::File(_, 5)));
        assert!(r.header("Content-Disposition").unwrap().contains("filename*=UTF-8''"));
        // Links appear in the results now.
        let b = body(&handle(&get("/?q=txt", Some(TOKEN)), LAN, &s, Instant::now()));
        assert!(b.contains("/f?p="));
        // Not in the index, traversal, folders: refused.
        for p in [
            secret.to_string_lossy().into_owned(),
            format!("{}/../{}/secret.txt", dir.display(), dir.file_name().unwrap().to_string_lossy()),
            dir.to_string_lossy().into_owned(),
            String::new(),
        ] {
            let r = handle(&get(&format!("/f?p={}", percent_encode(&p)), Some(TOKEN)), LAN, &s, Instant::now());
            assert_eq!(r.status, 404, "{p}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn serves_over_tcp() {
        let shared = Arc::new(shared_with(vec![entry(r"C:\docs\report.pdf", 10, false)]));
        // Port 0: any free port.
        let mut server = Server::start_on(&[Ipv4Addr::LOCALHOST, Ipv4Addr::new(8, 8, 8, 8)], 0, shared).unwrap();
        assert_eq!(server.addrs.len(), 1, "public addresses are never bound");
        let addr = server.addrs[0];
        let fetch = |raw: String| {
            let mut s = TcpStream::connect(addr).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            s.write_all(raw.as_bytes()).unwrap();
            let mut out = String::new();
            let _ = s.read_to_string(&mut out);
            out
        };
        let host = format!("Host: {addr}\r\n");
        let out = fetch(format!("GET /?q=report HTTP/1.1\r\n{host}\r\n"));
        assert!(out.starts_with("HTTP/1.1 401"), "{out}");
        assert!(out.contains("Content-Security-Policy:"));
        let out = fetch(format!("GET /?q=report HTTP/1.1\r\n{host}Cookie: {COOKIE}={TOKEN}\r\n\r\n"));
        assert!(out.starts_with("HTTP/1.1 200"), "{out}");
        assert!(out.contains("report.pdf"));
        let out = fetch(format!("HEAD /?q=report HTTP/1.1\r\n{host}Cookie: {COOKIE}={TOKEN}\r\n\r\n"));
        assert!(out.starts_with("HTTP/1.1 200") && !out.contains("report.pdf"), "{out}");
        let out = fetch("GARBAGE\r\n\r\n".into());
        assert!(out.starts_with("HTTP/1.1 400"), "{out}");
        server.stop();
        std::thread::sleep(Duration::from_millis(50));
        assert!(TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_err(), "stopped");
    }
}
