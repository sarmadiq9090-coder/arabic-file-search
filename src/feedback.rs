//! Feedback from users, without telemetry.
//!
//! Options ▸ "Send feedback" opens the browser on a new GitHub issue whose
//! form is already filled with the version, Windows version, processor
//! architecture and interface language. Nothing is sent by the program: the
//! user reads the page and presses "Submit" themselves.
//!
//! Crash reports work the same way. The panic hook (`crash_log`) writes each
//! report to `crash.txt` on the user's own disk. At the next start the
//! program asks once whether to report the new ones; "Yes" opens a prefilled
//! issue with the report, "No" forgets them. Nothing leaves the computer
//! without that click and the user's own "Submit" on GitHub.
//!
//! The issue forms live in `.github/ISSUE_TEMPLATE/`; the query keys below
//! are the `id`s of their fields, which GitHub fills from the URL.

use std::fmt::Write as _;

pub const REPO_URL: &str = "https://github.com/sarmadiq9090-coder/arabic-file-search";

/// GitHub answers "414 URI Too Long" well above this; browsers and
/// `ShellExecute` cope with it too.
const MAX_URL: usize = 7000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Something does not work (`bug_report.yml`).
    Bug,
    /// A wish for something new (`feature_request.yml`).
    Idea,
}

/// What the issue form is filled with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemInfo {
    pub version: String,
    pub os: String,
    pub arch: String,
    /// `ar` or `en`.
    pub lang: String,
}

impl SystemInfo {
    /// This copy of the program on this computer.
    #[cfg(windows)]
    pub fn current() -> SystemInfo {
        SystemInfo {
            version: env!("CARGO_PKG_VERSION").to_string(),
            os: windows_version(),
            arch: std::env::consts::ARCH.to_string(),
            lang: crate::i18n::current().code().to_string(),
        }
    }
}

/// The page to discuss, ask or share ideas less formally than an issue.
pub fn discussions_url() -> String {
    format!("{REPO_URL}/discussions")
}

/// A new issue of `kind`, its form filled with `info` and, for a bug, the
/// crash report `crash` (shortened to keep the address within limits).
pub fn issue_url(kind: Kind, info: &SystemInfo, crash: Option<&str>) -> String {
    let (template, title) = match (kind, crash.is_some()) {
        (Kind::Bug, true) => ("bug_report.yml", format!("[Crash] v{}", info.version)),
        (Kind::Bug, false) => ("bug_report.yml", String::new()),
        (Kind::Idea, _) => ("feature_request.yml", String::new()),
    };
    let mut url = format!("{REPO_URL}/issues/new?template={template}");
    let mut add = |key: &str, value: &str| {
        if !value.is_empty() {
            let _ = write!(url, "&{key}={}", crate::web::percent_encode(value));
        }
    };
    add("title", &title);
    add("version", &info.version);
    add("os", &info.os);
    add("arch", &info.arch);
    add("lang", &info.lang);
    if let (Kind::Bug, Some(report)) = (kind, crash) {
        let budget = MAX_URL.saturating_sub(url.len() + "&crash=".len());
        let text = fit_encoded(&anonymize(report), budget);
        let _ = write!(url, "&crash={}", crate::web::percent_encode(&text));
    }
    url
}

/// A new issue, from the quick search, about a search that did not find
/// what the user expected: the form is filled as for a bug, with the query
/// in "what happened" (cut to a sane length) for the user to complete.
pub fn search_issue_url(info: &SystemInfo, query: &str, english: bool) -> String {
    let q: String = query.trim().chars().filter(|c| !c.is_control()).take(200).collect();
    let what = if english {
        format!("The quick search for “{q}” did not find what I expected.\n\nI expected to find: ")
    } else {
        format!("البحث السريع عن «{q}» لم يجد ما توقعته.\n\nكنت أتوقع أن أجد: ")
    };
    let mut url = issue_url(Kind::Bug, info, None);
    let _ = write!(url, "&title={}&what={}", crate::web::percent_encode("[Search] "), crate::web::percent_encode(&what));
    url
}

/// The longest start of `text` (whole lines where possible) whose
/// percent-encoding fits in `budget` bytes, with a note when cut.
fn fit_encoded(text: &str, budget: usize) -> String {
    const CUT: &str = "\n… (shortened; the full report is in %LOCALAPPDATA%\\BahthSari\\crash.txt)";
    let enc = |s: &str| crate::web::percent_encode(s).len();
    if enc(text) <= budget {
        return text.to_string();
    }
    let budget = budget.saturating_sub(enc(CUT));
    let mut out = String::new();
    let mut used = 0;
    for line in text.split_inclusive('\n') {
        let n = enc(line);
        if used + n > budget {
            // A first line too long on its own is cut by characters.
            if out.is_empty() {
                for c in line.chars() {
                    let n = enc(c.encode_utf8(&mut [0; 4]));
                    if used + n > budget {
                        break;
                    }
                    used += n;
                    out.push(c);
                }
            }
            break;
        }
        used += n;
        out.push_str(line);
    }
    out.truncate(out.trim_end().len());
    out.push_str(CUT);
    out
}

/// Replaces the account name in `C:\Users\<name>\…` paths, which a panic
/// message may contain, so a public issue does not show it.
pub fn anonymize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let lower = rest.to_ascii_lowercase();
        let found = ["\\users\\", "/users/"].iter().filter_map(|p| lower.find(p).map(|i| (i, p.len()))).min();
        let Some((at, len)) = found else {
            out.push_str(rest);
            return out;
        };
        let start = at + len;
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let end = tail.find(['\\', '/', '\'', '"', ' ', '\n', '\r', ')', ':']).unwrap_or(tail.len());
        let name = &tail[..end];
        let keep = ["public", "default", "all users"].iter().any(|k| name.eq_ignore_ascii_case(k)) || name.is_empty();
        out.push_str(if keep { name } else { "<user>" });
        rest = &tail[end..];
    }
}

/// One report in `crash.txt`: when it was written and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashReport {
    pub time: u64,
    pub text: String,
}

/// The reports in `crash.txt` written by `version`. Each entry is
/// `<seconds> PANIC (v<version>) …` up to `END PANIC`.
pub fn crash_reports(file: &str, version: &str) -> Vec<CrashReport> {
    let head = format!(" PANIC (v{version}) ");
    let mut out = Vec::new();
    let mut current: Option<CrashReport> = None;
    for line in file.lines() {
        if let Some((secs, _)) = line.split_once(' ')
            && let Ok(time) = secs.parse::<u64>()
            && line[secs.len()..].starts_with(" PANIC (v")
        {
            out.extend(current.take());
            if line[secs.len()..].starts_with(&head) {
                current = Some(CrashReport { time, text: line[secs.len() + 1..].to_string() });
            }
            continue;
        }
        if let Some(r) = &mut current {
            r.text.push('\n');
            r.text.push_str(line);
            if line.trim() == "END PANIC" {
                out.extend(current.take());
            }
        }
    }
    out.extend(current);
    out
}

/// The reports newer than `seen` (the time of the last one already asked
/// about), newest last.
pub fn unseen(reports: Vec<CrashReport>, seen: u64) -> Vec<CrashReport> {
    reports.into_iter().filter(|r| r.time > seen).collect()
}

/// What goes in the issue for `reports` (newest first, then how many more).
pub fn crash_text(reports: &[CrashReport]) -> String {
    let Some(last) = reports.last() else { return String::new() };
    let mut s = last.text.clone();
    if reports.len() > 1 {
        let _ = write!(s, "\n\n({} earlier report(s) of this version are in crash.txt)", reports.len() - 1);
    }
    s
}

/// `Windows 11 24H2 (build 26100.2314)`, read from the registry; plain
/// `Windows` when that fails.
#[cfg(windows)]
fn windows_version() -> String {
    let key = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion";
    let build = reg_string(key, "CurrentBuildNumber").or_else(|| reg_string(key, "CurrentBuild"));
    let display = reg_string(key, "DisplayVersion").or_else(|| reg_string(key, "ReleaseId"));
    let ubr = reg_dword(key, "UBR");
    format_windows(build.as_deref(), display.as_deref(), ubr)
}

/// Windows 11 still says "Windows 10" in `ProductName`; the build number tells them apart.
pub fn format_windows(build: Option<&str>, display: Option<&str>, ubr: Option<u32>) -> String {
    let Some(build) = build.filter(|b| !b.is_empty()) else { return "Windows".to_string() };
    let n: u32 = build.parse().unwrap_or(0);
    let name = if n >= 22000 {
        "Windows 11"
    } else if n >= 10240 {
        "Windows 10"
    } else {
        "Windows"
    };
    let mut s = name.to_string();
    if let Some(d) = display.filter(|d| !d.is_empty()) {
        let _ = write!(s, " {d}");
    }
    let _ = write!(s, " (build {build}");
    if let Some(u) = ubr {
        let _ = write!(s, ".{u}");
    }
    s.push(')');
    s
}

#[cfg(windows)]
fn reg_value(key: &str, name: &str, flags: u32, buf: &mut [u8]) -> Option<usize> {
    use winapi::um::winreg::{HKEY_LOCAL_MACHINE, RegGetValueW};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (k, n) = (wide(key), wide(name));
    let mut size = buf.len() as u32;
    let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, k.as_ptr(), n.as_ptr(), flags, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size) };
    (r == 0).then_some(size as usize)
}

#[cfg(windows)]
fn reg_string(key: &str, name: &str) -> Option<String> {
    const RRF_RT_REG_SZ: u32 = 0x0000_0002;
    let mut buf = [0u8; 256];
    let n = reg_value(key, name, RRF_RT_REG_SZ, &mut buf)?.min(buf.len());
    let units: Vec<u16> = buf[..n].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|&u| u != 0).collect();
    let s = String::from_utf16_lossy(&units).trim().to_string();
    (!s.is_empty()).then_some(s)
}

#[cfg(windows)]
fn reg_dword(key: &str, name: &str) -> Option<u32> {
    const RRF_RT_REG_DWORD: u32 = 0x0000_0010;
    let mut buf = [0u8; 4];
    let n = reg_value(key, name, RRF_RT_REG_DWORD, &mut buf)?;
    (n == 4).then(|| u32::from_le_bytes(buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> SystemInfo {
        SystemInfo { version: "0.7.0".into(), os: "Windows 11 24H2 (build 26100.2314)".into(), arch: "aarch64".into(), lang: "ar".into() }
    }

    fn query(url: &str) -> Vec<(String, String)> {
        crate::web::parse_form(url.split_once('?').unwrap().1)
    }

    #[test]
    fn bug_and_idea_urls_fill_the_form() {
        let url = issue_url(Kind::Bug, &info(), None);
        assert!(url.starts_with("https://github.com/sarmadiq9090-coder/arabic-file-search/issues/new?template=bug_report.yml&"));
        let q = query(&url);
        assert!(q.contains(&("version".into(), "0.7.0".into())));
        assert!(q.contains(&("os".into(), "Windows 11 24H2 (build 26100.2314)".into())));
        assert!(q.contains(&("arch".into(), "aarch64".into())));
        assert!(q.contains(&("lang".into(), "ar".into())));
        assert!(!q.iter().any(|(k, _)| k == "title" || k == "crash"));
        let url = issue_url(Kind::Idea, &info(), Some("ignored"));
        assert!(url.contains("template=feature_request.yml"));
        assert!(!url.contains("crash="));
        assert!(!url.contains(' '));
    }

    #[test]
    fn search_feedback_carries_the_query() {
        let url = search_issue_url(&info(), "  تقرير\u{7}  ", false);
        assert!(url.contains("template=bug_report.yml"));
        let q = query(&url);
        let what = &q.iter().find(|(k, _)| k == "what").unwrap().1;
        assert!(what.contains("«تقرير»"), "{what}");
        assert!(q.contains(&("title".into(), "[Search] ".into())));
        assert!(q.contains(&("version".into(), "0.7.0".into())));
        let url = search_issue_url(&info(), &"x".repeat(5000), true);
        assert!(url.len() < MAX_URL);
        assert!(query(&url).iter().any(|(k, v)| k == "what" && v.starts_with("The quick search")));
    }

    #[test]
    fn crash_url_carries_the_report_and_stays_short() {
        let report = "PANIC (v0.7.0) in thread 'main': boom at C:\\Users\\سيف\\Desktop\\x.pdf\n  at src\\search.rs:1:1\nEND PANIC";
        let q = query(&issue_url(Kind::Bug, &info(), Some(report)));
        let crash = &q.iter().find(|(k, _)| k == "crash").unwrap().1;
        assert!(crash.contains("C:\\Users\\<user>\\Desktop\\x.pdf"), "{crash}");
        assert!(!crash.contains("سيف"));
        assert!(q.contains(&("title".into(), "[Crash] v0.7.0".into())));
        // A huge report (and a huge single line) is cut to fit.
        let big = format!("PANIC (v0.7.0) x\n{}END PANIC", "   12: 0x7ff6a1b2c3d4 - <unknown> (bahth.exe+0x489519)\n".repeat(2000));
        let url = issue_url(Kind::Bug, &info(), Some(&big));
        assert!(url.len() <= MAX_URL, "{}", url.len());
        assert!(query(&url).iter().any(|(k, v)| k == "crash" && v.contains("shortened") && v.starts_with("PANIC")));
        let line = "ب".repeat(20_000);
        let url = issue_url(Kind::Bug, &info(), Some(&line));
        assert!(url.len() <= MAX_URL);
        let small = fit_encoded("abc\ndef", 4);
        assert!(small.len() < 200);
    }

    #[test]
    fn user_names_are_hidden() {
        assert_eq!(anonymize("C:\\Users\\Saif\\a.txt and c:\\users\\bob"), "C:\\Users\\<user>\\a.txt and c:\\users\\<user>");
        assert_eq!(anonymize("C:\\Users\\Public\\x"), "C:\\Users\\Public\\x");
        assert_eq!(anonymize("/Users/ana/x 'C:\\Users\\Ü ber'"), "/Users/<user>/x 'C:\\Users\\<user> ber'");
        assert_eq!(anonymize("no paths, \\users\\"), "no paths, \\users\\");
        assert_eq!(anonymize(""), "");
    }

    #[test]
    fn reports_of_this_version_are_read_from_crash_txt() {
        let file = "100 PANIC (v0.6.1) in thread 'a': old\n  at x\nEND PANIC\n\n\
                    200 PANIC (v0.7.0) in thread 'b': one\n  at y\n  backtrace:\n  0: 0x1\nEND PANIC\n\n\
                    garbage line\n\
                    300 PANIC (v0.7.0) in thread 'c': two\n  at z\n";
        let r = crash_reports(file, "0.7.0");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].time, 200);
        assert_eq!(r[0].text, "PANIC (v0.7.0) in thread 'b': one\n  at y\n  backtrace:\n  0: 0x1\nEND PANIC");
        assert_eq!(r[1].time, 300);
        assert!(r[1].text.ends_with("  at z"));
        assert_eq!(unseen(r.clone(), 200).len(), 1);
        assert!(unseen(r.clone(), 300).is_empty());
        let text = crash_text(&r);
        assert!(text.starts_with("PANIC (v0.7.0) in thread 'c'") && text.contains("1 earlier report"));
        assert_eq!(crash_text(&[]), "");
        assert!(crash_reports("", "0.7.0").is_empty());
        assert!(crash_reports("x PANIC (v0.7.0) y\nEND PANIC", "0.7.0").is_empty());
        assert!(crash_reports("\u{0}\u{feff}PANIC", "0.7.0").is_empty());
    }

    #[test]
    fn windows_names() {
        assert_eq!(format_windows(Some("26100"), Some("24H2"), Some(2314)), "Windows 11 24H2 (build 26100.2314)");
        assert_eq!(format_windows(Some("19045"), Some("22H2"), None), "Windows 10 22H2 (build 19045)");
        assert_eq!(format_windows(Some("9600"), None, None), "Windows (build 9600)");
        assert_eq!(format_windows(None, Some("24H2"), Some(1)), "Windows");
        assert_eq!(format_windows(Some(""), None, None), "Windows");
        assert!(discussions_url().ends_with("/discussions"));
    }

    #[test]
    fn native_crash_lines_are_reports() {
        let file = "1700000000 PANIC (v9.9.9) native crash: exception 0xC00000FD at bahth.exe+0x1A2B\r\nEND PANIC\r\n\r\n";
        let r = crash_reports(file, "9.9.9");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].time, 1_700_000_000);
        assert!(r[0].text.contains("0xC00000FD"));
    }
}
