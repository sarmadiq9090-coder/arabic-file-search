//! Auto-update from GitHub Releases.
//!
//! The pure logic (version comparison, parsing the GitHub API answer, notes
//! trimming, PE validation, check scheduling) builds on every platform so it
//! can be unit-tested on Linux. Everything that touches the network or the
//! file system of the installed program is Windows-only.
//!
//! All Windows functions block; call them from a background thread except
//! `restore_old_if_missing` and `wait_for_previous_instance_exit`, which are meant for
//! startup.


/// GitHub API endpoint for the newest published (non-draft, non-prerelease) release.
#[cfg(windows)]
pub const LATEST_URL: &str =
    "https://api.github.com/repos/sarmadiq9090-coder/arabic-file-search/releases/latest";
/// Named mutex that keeps a single copy of the program running (ui/startup.rs).
#[cfg(windows)]
pub const SINGLE_INSTANCE_MUTEX: &str = "Local\\BahthSari.SingleInstance";
/// Name of the release asset that holds the program for this CPU. The x64
/// name must never change: every copy installed since 0.3.0 looks for it.
pub const ASSET_NAME: &str = if cfg!(target_arch = "aarch64") {
    ASSET_ARM64
} else if cfg!(target_arch = "x86") {
    ASSET_X86
} else {
    ASSET_X64
};
/// The release assets for each CPU (build.yml publishes all three).
pub const ASSET_X64: &str = "bahth.exe";
pub const ASSET_ARM64: &str = "bahth-arm64.exe";
pub const ASSET_X86: &str = "bahth-x86.exe";
/// Upper bound for the JSON answer.
#[cfg(windows)]
pub const MAX_JSON: usize = 1 << 20;
/// Upper bound for the downloaded program.
pub const MAX_DOWNLOAD: u64 = 100 << 20;
/// How often to look for an update automatically.
pub const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;
/// How many characters of the release notes are kept.
pub const NOTES_MAX_CHARS: usize = 600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// "0.3.1" (no leading v).
    pub version: String,
    /// The tag as published, e.g. "v0.3.1".
    pub tag: String,
    /// Release body, trimmed to about `NOTES_MAX_CHARS` characters.
    pub notes: String,
    pub exe_url: String,
    pub exe_size: u64,
    /// SHA-256 of the program as hex, when GitHub publishes one (`digest`).
    /// Installing requires it (`required_sha256`).
    pub sha256: Option<String>,
}

/// Web page that redirects to the newest release; used when the API refuses
/// (it allows only 60 calls an hour per address without a login).
#[cfg(windows)]
pub const LATEST_PAGE_URL: &str = "https://github.com/sarmadiq9090-coder/arabic-file-search/releases/latest";
/// GitHub API endpoint for one release, by tag (appended). Gives the SHA-256
/// of a release that was found through the web page.
#[cfg(windows)]
pub const RELEASE_BY_TAG_URL: &str = "https://api.github.com/repos/sarmadiq9090-coder/arabic-file-search/releases/tags/";
/// Where users download the program by hand when the updater refuses a release.
pub const MANUAL_DOWNLOAD_URL: &str = "https://github.com/sarmadiq9090-coder/arabic-file-search/releases/latest";

/// The SHA-256 the downloaded program must match. Fails closed: a release
/// without a checksum (GitHub's `digest`) is never installed, because nothing
/// would show that the file is the one that was published.
pub fn required_sha256(rel: &Release) -> Result<&str, String> {
    rel.sha256.as_deref().ok_or_else(missing_digest_message)
}

/// Why a release without a checksum is refused, and where to get it by hand.
pub fn missing_digest_message() -> String {
    trf!(
        "لا يمكن التحقق من هذا التحديث: لم ينشر GitHub بصمة SHA-256 لملف البرنامج، لذلك لم يُثبَّت.\nيمكنك تنزيل الإصدار الجديد يدوياً من:\n{}",
        "This update cannot be verified: GitHub published no SHA-256 checksum for the program file, so it was not installed.\nYou can download the new version by hand from:\n{}",
        MANUAL_DOWNLOAD_URL
    )
}

/// A package manager that installed this copy and so owns its updates.
/// Replacing the exe behind its back breaks its records (version, hash,
/// winget's uninstall entry), so the self-updater stays out of the way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManager {
    Scoop,
    Winget,
}

impl PackageManager {
    pub fn name(self) -> &'static str {
        match self {
            PackageManager::Scoop => "Scoop",
            PackageManager::Winget => "winget",
        }
    }

    /// The command that updates InstaFind (`bucket/instafind.json`, `Sarmad.InstaFind`).
    pub fn update_command(self) -> &'static str {
        match self {
            PackageManager::Scoop => "scoop update instafind",
            PackageManager::Winget => "winget upgrade Sarmad.InstaFind",
        }
    }
}

/// Which package manager installed the program at `exe`, judged by where it
/// lives; `var` reads an environment variable. Case-insensitive.
/// - Scoop: `apps\` or `shims\` under `%SCOOP%`, `%USERPROFILE%\scoop`, and
///   the global `%SCOOP_GLOBAL%` / `%ProgramData%\scoop`.
/// - winget: `Packages\` (portable installs) or `Links\` (their command
///   shims) under `%LOCALAPPDATA%\Microsoft\WinGet`, and the machine-wide
///   `%ProgramFiles%\WinGet` / `%ProgramFiles(x86)%\WinGet`.
pub fn package_manager_for(exe: &str, var: impl Fn(&str) -> Option<String>) -> Option<PackageManager> {
    fn norm(p: &str) -> String {
        let p = p.replace('/', "\\");
        let p = p.strip_prefix(r"\\?\").unwrap_or(&p);
        p.trim_end_matches('\\').to_lowercase()
    }
    let exe = norm(exe);
    // An unset or empty variable is no root (`\scoop` is not a Scoop folder).
    let var = |k: &str| var(k).filter(|v| !norm(v).is_empty());
    // True when `exe` is a file somewhere below `<root>\<sub>\`.
    let below = |root: &Option<String>, sub: &str| {
        let Some(root) = root.as_deref().map(norm).filter(|r| !r.is_empty()) else { return false };
        let dir = format!("{root}\\{}\\", sub.to_lowercase());
        exe.len() > dir.len() && exe.starts_with(&dir)
    };
    let scoop = [
        var("SCOOP"),
        var("USERPROFILE").map(|h| format!("{h}\\scoop")),
        var("SCOOP_GLOBAL"),
        var("ProgramData").map(|d| format!("{d}\\scoop")),
    ];
    if scoop.iter().any(|r| below(r, "apps") || below(r, "shims")) {
        return Some(PackageManager::Scoop);
    }
    let winget = [
        var("LOCALAPPDATA").map(|d| format!("{d}\\Microsoft\\WinGet")),
        var("ProgramFiles").map(|d| format!("{d}\\WinGet")),
        var("ProgramFiles(x86)").map(|d| format!("{d}\\WinGet")),
    ];
    if winget.iter().any(|r| below(r, "Packages") || below(r, "Links")) {
        return Some(PackageManager::Winget);
    }
    None
}

/// What the user is told instead of a self-update when a package manager
/// installed this copy.
pub fn managed_message(pm: PackageManager) -> String {
    trf!(
        "هذه النسخة مثبّتة عبر {0}، لذلك يحدّثها {0} ولا يحدّث البرنامج نفسه بنفسه.\nللتحديث، شغّل هذا الأمر في الطرفية (Terminal):\n{1}",
        "This copy was installed with {0}, so it is updated by {0}, not by the program itself.\nTo update, run this in a terminal:\n{1}",
        pm.name(),
        pm.update_command()
    )
}

/// The tag in a ".../releases/tag/<tag>" address (where LATEST_PAGE_URL
/// leads), if any.
pub fn tag_from_release_url(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("/releases/tag/")?;
    let tag = rest.split(['?', '#', '/']).next()?.trim();
    (!tag.is_empty()).then(|| tag.to_string())
}

/// A release known only by its tag: the program is fetched from the usual
/// download address, its size is learned while downloading.
pub fn release_from_tag(tag: &str, current: &str) -> Option<Release> {
    if !is_newer(tag, current) {
        return None;
    }
    let version = tag.trim().trim_start_matches(['v', 'V']).to_string();
    Some(Release {
        exe_url: format!(
            "https://github.com/sarmadiq9090-coder/arabic-file-search/releases/download/{tag}/{ASSET_NAME}"
        ),
        version,
        tag: tag.to_string(),
        notes: String::new(),
        exe_size: 0,
        sha256: None,
    })
}

/// Parses "v1.2.3" / "1.2" / "1.2.3.4" into numbers. Returns None for anything
/// else, including prerelease or build suffixes ("1.2.3-beta", "1.2.3+x").
pub fn parse_version(s: &str) -> Option<Vec<u64>> {
    let s = s.trim();
    let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
    if s.is_empty() {
        return None;
    }
    s.split('.')
        .map(|p| {
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                None
            } else {
                p.parse::<u64>().ok()
            }
        })
        .collect()
}

/// True when `candidate` is a strictly newer numeric version than `current`.
/// Missing components count as zero, so "1.2" == "1.2.0".
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (Some(a), Some(b)) = (parse_version(candidate), parse_version(current)) else {
        return false;
    };
    let n = a.len().max(b.len());
    for i in 0..n {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

/// Trims release notes to at most `max` characters, preferring to cut at a
/// line break or space, and appends "…" when something was cut.
pub fn trim_notes(body: &str, max: usize) -> String {
    let body = body.replace("\r\n", "\n");
    let body = body.trim();
    if body.chars().count() <= max {
        return body.to_string();
    }
    let cut = body.char_indices().nth(max).map(|(i, _)| i).unwrap_or(body.len());
    let head = &body[..cut];
    // Prefer a natural break in the last third of the kept text.
    let floor = head.len() * 2 / 3;
    let at = head
        .rfind('\n')
        .filter(|&i| i >= floor)
        .or_else(|| head.rfind(char::is_whitespace).filter(|&i| i >= floor))
        .unwrap_or(head.len());
    format!("{}…", head[..at].trim_end())
}

/// Turns the GitHub "release" JSON into a `Release`, if it is a usable
/// release that is newer than `current`. Errors only for malformed input.
pub fn parse_release(json: &str, current: &str) -> Result<Option<Release>, String> {
    parse_release_for(json, current, ASSET_NAME)
}

/// `parse_release` for the program asset named `asset_name` (another CPU's).
pub fn parse_release_for(json: &str, current: &str, asset_name: &str) -> Result<Option<Release>, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("bad JSON from GitHub: {e}"))?;
    if !v.is_object() {
        return Err("unexpected answer from GitHub".into());
    }
    if v["draft"].as_bool().unwrap_or(false) || v["prerelease"].as_bool().unwrap_or(false) {
        return Ok(None);
    }
    let tag = v["tag_name"]
        .as_str()
        .ok_or("release has no tag_name")?
        .trim()
        .to_string();
    if parse_version(&tag).is_none() || !is_newer(&tag, current) {
        return Ok(None);
    }
    let version = tag.strip_prefix(['v', 'V']).unwrap_or(&tag).to_string();
    let asset = v["assets"]
        .as_array()
        .and_then(|a| a.iter().find(|x| x["name"].as_str() == Some(asset_name)));
    let Some(asset) = asset else {
        // A newer tag without the program attached is not installable.
        return Ok(None);
    };
    let exe_url = asset["browser_download_url"]
        .as_str()
        .ok_or("asset has no download URL")?
        .to_string();
    if !exe_url.starts_with("https://") {
        return Err("asset URL is not https".into());
    }
    if !download_host_ok(&exe_url) {
        return Err("asset URL is not on GitHub".into());
    }
    let exe_size = asset["size"].as_u64().ok_or("asset has no size")?;
    if exe_size == 0 || exe_size > MAX_DOWNLOAD {
        return Err(format!("asset size {exe_size} is out of range"));
    }
    let notes = trim_notes(v["body"].as_str().unwrap_or(""), NOTES_MAX_CHARS);
    let sha256 = asset["digest"]
        .as_str()
        .and_then(|d| d.strip_prefix("sha256:"))
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()));
    Ok(Some(Release { version, tag, notes, exe_url, exe_size, sha256 }))
}

/// The program is only ever downloaded from GitHub (`github.com`, which
/// redirects to its own download hosts), whatever the API answer says.
pub fn download_host_ok(url: &str) -> bool {
    let Ok((host, port, _)) = split_https_url(url) else { return false };
    let host = host.to_ascii_lowercase();
    port == 443
        && (host == "github.com"
            || host.ends_with(".github.com")
            || host.ends_with(".githubusercontent.com"))
}

/// Checks that `head` (the first bytes of a file) is a PE image for this
/// architecture: "MZ", a sane e_lfanew, "PE\0\0" and a matching machine type.
pub fn validate_pe(head: &[u8]) -> Result<(), String> {
    if head.len() < 0x40 || &head[..2] != b"MZ" {
        return Err("downloaded file is not a Windows program (no MZ)".into());
    }
    let lfanew = u32::from_le_bytes([head[0x3c], head[0x3d], head[0x3e], head[0x3f]]) as usize;
    // Checked: on a 32-bit build `lfanew + 6` can wrap past the length test.
    if lfanew < 0x40 || lfanew.checked_add(6).is_none_or(|end| end > head.len()) {
        return Err("downloaded file has a bad PE header offset".into());
    }
    if &head[lfanew..lfanew + 4] != b"PE\0\0" {
        return Err("downloaded file has no PE signature".into());
    }
    let machine = u16::from_le_bytes([head[lfanew + 4], head[lfanew + 5]]);
    let expected: Option<u16> = if cfg!(target_arch = "x86_64") {
        Some(0x8664)
    } else if cfg!(target_arch = "x86") {
        Some(0x014c)
    } else if cfg!(target_arch = "aarch64") {
        Some(0xaa64)
    } else {
        None
    };
    if let Some(m) = expected
        && machine != m
    {
        return Err(format!("downloaded program is for another CPU (machine {machine:#06x})"));
    }
    Ok(())
}

/// True when the last automatic check is at least a day old (or in the future,
/// which means the clock moved back).
pub fn should_check_now(last_check_secs: u64, now: u64) -> bool {
    last_check_secs == 0 || now < last_check_secs || now - last_check_secs >= CHECK_INTERVAL_SECS
}

/// Splits "https://host[:port]/path?q" into (host, port, path). Only https.
pub fn split_https_url(url: &str) -> Result<(String, u16, String), String> {
    let rest = url.strip_prefix("https://").ok_or("only https URLs are supported")?;
    let (authority, path) = match rest.find(['/', '?']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    if authority.is_empty() || authority.contains('@') {
        return Err("bad URL host".into());
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().map_err(|_| "bad URL port")?),
        None => (authority, 443),
    };
    let path = if path.starts_with('?') { format!("/{path}") } else { path.to_string() };
    Ok((host.to_string(), port, path))
}

/// `exe` with `suffix` added to its file name (`bahth.exe.old`).
#[cfg_attr(not(windows), allow(dead_code))]
fn sibling(exe: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    exe.with_file_name(name)
}

/// Puts the downloaded `new` program in the place of `exe`, keeping the
/// current one as `old`. On any failure `exe` is a working program again:
/// the original is renamed back, or copied back if even that fails.
#[cfg_attr(not(windows), allow(dead_code))]
fn swap_in(exe: &std::path::Path, new: &std::path::Path, old: &std::path::Path) -> Result<(), String> {
    use std::fs;
    if !new.is_file() {
        return Err("the downloaded update is missing".into());
    }
    // A leftover from an earlier update; if it cannot be removed (still
    // running?) the rename below reports the problem.
    let _ = fs::remove_file(old);
    fs::rename(exe, old).map_err(|e| format!("cannot move the running program aside: {e}"))?;
    if let Err(e) = fs::rename(new, exe) {
        // Roll back: put the original program back under its name. A copy
        // works where a rename may not (a scanner holding the file).
        let back = fs::rename(old, exe).or_else(|_| fs::copy(old, exe).map(drop));
        return Err(match back {
            Ok(()) => format!("cannot put the update in place: {e}"),
            Err(e2) => format!(
                "cannot put the update in place: {e}; restoring the old program also failed: {e2} (it is at {})",
                old.display()
            ),
        });
    }
    Ok(())
}

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
mod win {
    use super::*;
    use std::ffi::OsStr;
    use std::fs;
    use std::io::{Read, Write};
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};
    use winapi::shared::minwindef::{DWORD, FALSE, LPVOID};
    use winapi::um::errhandlingapi::GetLastError;
    use winapi::um::winhttp::*;

    const TIMEOUT_MS: i32 = 15_000;

    fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
        s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
    }

    /// Path of the installed program, captured the first time it is asked for
    /// (at startup, through `restore_old_if_missing`), before any rename can happen.
    fn original_exe() -> Result<&'static Path, String> {
        static EXE: OnceLock<Option<PathBuf>> = OnceLock::new();
        EXE.get_or_init(|| {
            let p = std::env::current_exe().ok()?;
            let p = fs::canonicalize(&p).map(strip_verbatim).unwrap_or(p);
            // If we are somehow already running from the renamed copy,
            // the real program is the one without ".old".
            match p.file_name().and_then(|n| n.to_str()) {
                Some(n) if n.to_ascii_lowercase().ends_with(".exe.old") => {
                    Some(p.with_file_name(&n[..n.len() - 4]))
                }
                _ => Some(p),
            }
        })
        .as_deref()
        .ok_or_else(|| "cannot find the program's own path".to_string())
    }

    /// Turns "\\?\C:\x" into "C:\x" so the path stays friendly for CreateProcess.
    fn strip_verbatim(p: PathBuf) -> PathBuf {
        match p.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
            Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
            _ => p,
        }
    }

    // ---------------------------------------------------------------- HTTP

    struct Handle(HINTERNET);
    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { WinHttpCloseHandle(self.0) };
            }
        }
    }

    fn last_error(what: &str) -> String {
        format!("{what} failed (error {})", unsafe { GetLastError() })
    }

    fn open_session() -> Result<Handle, String> {
        let agent = wide(concat!("Bahth/", env!("CARGO_PKG_VERSION"), " (+https://github.com/sarmadiq9090-coder/arabic-file-search)"));
        // AUTOMATIC_PROXY exists from Windows 8.1; older systems reject it.
        let mut h = unsafe {
            WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, std::ptr::null(), std::ptr::null(), 0)
        };
        if h.is_null() {
            h = unsafe {
                WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, std::ptr::null(), std::ptr::null(), 0)
            };
        }
        if h.is_null() {
            return Err(last_error("WinHttpOpen"));
        }
        let session = Handle(h);
        unsafe {
            WinHttpSetTimeouts(h, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS);
            // Ask for TLS 1.2/1.3 explicitly (Windows 7 defaults lack 1.2).
            // TLS 1.3 is unknown to older systems, so fall back to 1.2 only.
            for mut protocols in [0x0000_2A00u32, 0x0000_0A00u32] {
                if WinHttpSetOption(
                    h,
                    WINHTTP_OPTION_SECURE_PROTOCOLS,
                    &mut protocols as *mut u32 as LPVOID,
                    4,
                ) != FALSE
                {
                    break;
                }
            }
        }
        Ok(session)
    }

    /// GETs `url`, following redirects, and hands the body to `sink` chunk by
    /// chunk. Fails on non-200 statuses or when more than `max` bytes arrive.
    /// `on_length` sees the Content-Length (if any) before the body.
    fn http_get(
        url: &str,
        accept: &str,
        max: u64,
        on_length: &mut dyn FnMut(Option<u64>) -> Result<(), String>,
        sink: &mut dyn FnMut(&[u8]) -> Result<(), String>,
        final_url: Option<&mut String>,
    ) -> Result<(), String> {
        let (host, port, path) = split_https_url(url)?;
        let session = open_session()?;
        let host_w = wide(&host);
        let connect = Handle(unsafe { WinHttpConnect(session.0, host_w.as_ptr(), port, 0) });
        if connect.0.is_null() {
            return Err(last_error("WinHttpConnect"));
        }
        let (verb, path_w) = (wide("GET"), wide(&path));
        let request = Handle(unsafe {
            WinHttpOpenRequest(
                connect.0,
                verb.as_ptr(),
                path_w.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                WINHTTP_FLAG_SECURE,
            )
        });
        if request.0.is_null() {
            return Err(last_error("WinHttpOpenRequest"));
        }
        let headers = wide(format!("Accept: {accept}\r\nX-GitHub-Api-Version: 2022-11-28\r\n"));
        let ok = unsafe {
            WinHttpSendRequest(request.0, headers.as_ptr(), !0, std::ptr::null_mut(), 0, 0, 0) != FALSE
                && WinHttpReceiveResponse(request.0, std::ptr::null_mut()) != FALSE
        };
        if !ok {
            let code = unsafe { GetLastError() };
            return Err(match code {
                12002 => tr!("انتهت مهلة الاتصال", "the connection timed out").into(),
                12007 => tr!("تعذر الوصول إلى الخادم (هل الإنترنت متصل؟)", "cannot reach the server (is the internet connected?)").into(),
                12029 | 12030 => tr!("فشل الاتصال بالخادم", "the connection to the server failed").into(),
                12175 => tr!("فشل الاتصال الآمن (TLS)", "the secure connection (TLS) failed").into(),
                _ => trf!("فشل الطلب (خطأ {code})", "the request failed (error {code})"),
            });
        }
        if let Some(out) = final_url {
            // Where the redirects ended up.
            let mut buf = vec![0u16; 4096];
            let mut len = (buf.len() * 2) as DWORD;
            if unsafe { WinHttpQueryOption(request.0, WINHTTP_OPTION_URL, buf.as_mut_ptr() as LPVOID, &mut len) }
                != FALSE
            {
                *out = String::from_utf16_lossy(&buf[..(len as usize / 2).min(buf.len())]);
            }
        }
        let status = query_number(&request, WINHTTP_QUERY_STATUS_CODE).unwrap_or(0);
        if status != 200 {
            return Err(match status {
                403 | 429 => trf!("GitHub حدّ مؤقتاً عدد الطلبات من جهازك (HTTP {status}). حاول بعد ساعة.", "GitHub is temporarily limiting requests from this computer (HTTP {status}). Try again in an hour."),
                404 => tr!("لا يوجد إصدار منشور بعد (HTTP 404)", "no release has been published yet (HTTP 404)").into(),
                _ => trf!("رد غير متوقع من الخادم (HTTP {status})", "unexpected answer from the server (HTTP {status})"),
            });
        }
        let length = query_number(&request, WINHTTP_QUERY_CONTENT_LENGTH).map(u64::from);
        if length.is_some_and(|n| n > max) {
            return Err("the answer is too large".into());
        }
        on_length(length)?;
        let mut buf = vec![0u8; 64 * 1024];
        let mut total: u64 = 0;
        loop {
            let mut read: DWORD = 0;
            let ok = unsafe {
                WinHttpReadData(request.0, buf.as_mut_ptr() as LPVOID, buf.len() as DWORD, &mut read)
            };
            if ok == FALSE {
                return Err(last_error("reading from the network"));
            }
            if read == 0 {
                break;
            }
            total += u64::from(read);
            if total > max {
                return Err("the answer is too large".into());
            }
            sink(&buf[..read as usize])?;
        }
        if length.is_some_and(|n| n != total) {
            return Err(tr!("انقطع التنزيل قبل اكتماله", "the download stopped before it was complete").into());
        }
        Ok(())
    }

    fn query_number(request: &Handle, what: DWORD) -> Option<u32> {
        let mut value: u32 = 0;
        let mut len: DWORD = 4;
        let ok = unsafe {
            WinHttpQueryHeaders(
                request.0,
                what | WINHTTP_QUERY_FLAG_NUMBER,
                std::ptr::null(),
                &mut value as *mut u32 as LPVOID,
                &mut len,
                std::ptr::null_mut(),
            )
        };
        (ok != FALSE).then_some(value)
    }

    // ---------------------------------------------------------------- API

    /// Asks GitHub for the latest release. Some(..) only when it is newer than
    /// this build and has a `bahth.exe` asset.
    pub fn check_latest() -> Result<Option<Release>, String> {
        match check_latest_api() {
            Ok(r) => Ok(r),
            // The API limits calls per address; the web page does not.
            Err(api_err) => check_latest_page().map_err(|_| api_err),
        }
    }

    fn check_latest_page() -> Result<Option<Release>, String> {
        let mut url = String::new();
        http_get(LATEST_PAGE_URL, "text/html", 8 << 20, &mut |_| Ok(()), &mut |_| Ok(()), Some(&mut url))?;
        let tag = tag_from_release_url(&url).ok_or("no release page")?;
        Ok(release_from_tag(&tag, env!("CARGO_PKG_VERSION")))
    }

    fn check_latest_api() -> Result<Option<Release>, String> {
        let mut body = Vec::new();
        http_get(
            LATEST_URL,
            "application/vnd.github+json",
            MAX_JSON as u64,
            &mut |_| Ok(()),
            &mut |chunk| {
                body.extend_from_slice(chunk);
                Ok(())
            },
            None,
        )?;
        let text = String::from_utf8(body).map_err(|_| "GitHub answer is not UTF-8".to_string())?;
        parse_release(&text, env!("CARGO_PKG_VERSION"))
    }

    /// The SHA-256 GitHub publishes for this CPU's program in release `tag`,
    /// if the API answers (it may still be rate-limited).
    fn fetch_sha256(tag: &str) -> Option<String> {
        if tag.is_empty() || !tag.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-') {
            return None;
        }
        let body = fetch_json(&format!("{RELEASE_BY_TAG_URL}{tag}"), MAX_JSON as u64).ok()?;
        let rel = parse_release(std::str::from_utf8(&body).ok()?, env!("CARGO_PKG_VERSION")).ok()??;
        (rel.tag == tag).then_some(rel.sha256).flatten()
    }

    /// The package manager that installed this copy, if any (then it, not
    /// the self-updater, updates the program). Both the path the program was
    /// started by (a winget `Links` shim) and the resolved one are checked.
    pub fn managed_by() -> Option<PackageManager> {
        static PM: OnceLock<Option<PackageManager>> = OnceLock::new();
        *PM.get_or_init(|| {
            let var = |k: &str| std::env::var(k).ok();
            let started = std::env::current_exe().ok();
            let resolved = original_exe().ok().map(Path::to_path_buf);
            started
                .into_iter()
                .chain(resolved)
                .find_map(|p| package_manager_for(&p.to_string_lossy(), var))
        })
    }

    /// GETs a small JSON document of at most `max` bytes (the exchange rates
    /// of the currency answers), with the same connection settings as the
    /// update check.
    pub fn fetch_json(url: &str, max: u64) -> Result<Vec<u8>, String> {
        let mut body = Vec::new();
        http_get(
            url,
            "application/json",
            max,
            &mut |_| Ok(()),
            &mut |chunk| {
                body.extend_from_slice(chunk);
                Ok(())
            },
            None,
        )?;
        Ok(body)
    }

    /// Downloads `rel` next to the program and swaps it in place of the
    /// running exe (which is kept as `bahth.exe.old` until the next start).
    /// `progress(done, total)` is called as bytes arrive. Does not restart.
    pub fn download_and_install(rel: &Release, progress: impl Fn(u64, u64)) -> Result<(), String> {
        if let Some(pm) = managed_by() {
            return Err(managed_message(pm));
        }
        // Found through the web page (API rate-limited): ask the API once
        // more, for this release only, for the checksum. Without it, refuse.
        let mut rel = rel.clone();
        if rel.sha256.is_none() {
            rel.sha256 = fetch_sha256(&rel.tag);
        }
        required_sha256(&rel)?;
        let rel = &rel;
        let exe = original_exe()?.to_path_buf();
        let new = sibling(&exe, ".new");
        let old = sibling(&exe, ".old");
        let _ = fs::remove_file(&new);
        let result = download_to(rel, &new, &progress).and_then(|()| swap_in(&exe, &new, &old));
        if result.is_err() {
            let _ = fs::remove_file(&new);
        }
        result
    }

    fn download_to(rel: &Release, new: &Path, progress: &dyn Fn(u64, u64)) -> Result<(), String> {
        if rel.exe_size > MAX_DOWNLOAD {
            return Err("the update has an invalid size".into());
        }
        let file = fs::File::create(new).map_err(|e| format!("cannot create {}: {e}", new.display()))?;
        let mut out = std::io::BufWriter::with_capacity(256 * 1024, file);
        // 0 = unknown (release found without the API): taken from the
        // download's Content-Length, which is then required.
        let total = std::cell::Cell::new(rel.exe_size);
        let mut done: u64 = 0;
        let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
        progress(0, total.get());
        http_get(
            &rel.exe_url,
            "application/octet-stream",
            if total.get() == 0 { MAX_DOWNLOAD } else { total.get().min(MAX_DOWNLOAD) },
            &mut |len| match len {
                Some(n) if total.get() == 0 => {
                    total.set(n);
                    Ok(())
                }
                None if total.get() == 0 => Err("the server did not say how big the update is".into()),
                Some(n) if n != total.get() => Err(format!("the server sent {n} bytes, expected {}", total.get())),
                _ => Ok(()),
            },
            &mut |chunk| {
                out.write_all(chunk).map_err(|e| format!("cannot write the update: {e}"))?;
                sha2::Digest::update(&mut hasher, chunk);
                done += chunk.len() as u64;
                progress(done, total.get());
                Ok(())
            },
            None,
        )?;
        let file = out.into_inner().map_err(|e| format!("cannot write the update: {e}"))?;
        file.sync_all().map_err(|e| format!("cannot write the update: {e}"))?;
        drop(file);

        let want = required_sha256(rel)?;
        let got: String = sha2::Digest::finalize(hasher).iter().map(|b| format!("{b:02x}")).collect();
        if got != want {
            return Err(tr!("الملف الذي نُزّل لا يطابق بصمة الإصدار (SHA-256)", "the downloaded file does not match the release checksum (SHA-256)").into());
        }
        let size = fs::metadata(new).map_err(|e| e.to_string())?.len();
        if size != total.get() {
            return Err(format!("downloaded {size} bytes, expected {}", total.get()));
        }
        let mut head = vec![0u8; 4096];
        let n = fs::File::open(new)
            .and_then(|mut f| f.read(&mut head))
            .map_err(|e| format!("cannot read the update: {e}"))?;
        head.truncate(n);
        validate_pe(&head)
    }

    /// Starts the (new) program at its original path with `--updated` and
    /// this process's id (`handover::PREV_PID_ARG`), and returns it so the
    /// caller can see that it keeps running (`handover::took_over`) before
    /// it leaves.
    pub fn restart_new_version() -> Result<std::process::Child, String> {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        let exe = original_exe()?;
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("--updated")
            .arg(crate::handover::prev_pid_arg(std::process::id()))
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
        if let Some(dir) = exe.parent() {
            cmd.current_dir(dir);
        }
        cmd.spawn().map_err(|e| format!("cannot start the new version: {e}"))
    }

    /// The new version did not start: puts the running (old) program back
    /// as the installed one (`handover::roll_back`).
    pub fn roll_back_update() -> Result<(), String> {
        let exe = original_exe()?;
        crate::handover::roll_back(exe, &sibling(exe, ".old"))
    }

    /// Call at startup, before the single-instance check, when the program was
    /// started with `--updated`. Waits up to ~10 s for the previous copy to
    /// exit, i.e. for the single-instance mutex to disappear. Must be called
    /// before this process creates that mutex itself.
    pub fn wait_for_previous_instance_exit() {
        use winapi::um::handleapi::CloseHandle;
        use winapi::um::synchapi::OpenMutexW;
        use winapi::um::winnt::SYNCHRONIZE;
        let name = wide(super::SINGLE_INSTANCE_MUTEX);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let h = unsafe { OpenMutexW(SYNCHRONIZE, FALSE, name.as_ptr()) };
            if h.is_null() {
                // ERROR_FILE_NOT_FOUND: nobody holds it any more. Any other
                // error (e.g. access denied) also means we cannot wait on it.
                return;
            }
            unsafe { CloseHandle(h) };
            if Instant::now() >= deadline {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// At start: an interrupted update that left only `bahth.exe.old` gets
    /// it back as `bahth.exe`. (Nothing is deleted here: see
    /// [`remove_update_leftovers`].)
    pub fn restore_old_if_missing() -> bool {
        original_exe().is_ok_and(crate::handover::restore_if_missing)
    }

    /// Once this copy has shown it runs (its first timer tick): deletes
    /// `bahth.exe.old` and `bahth.exe.new` left by an update, with a few
    /// tries; one still locked is deleted at a later start. Blocks for a few
    /// seconds at most; the line for the log, if there was anything.
    pub fn remove_update_leftovers() -> Option<String> {
        crate::handover::remove_leftovers(original_exe().ok()?, 5, Duration::from_millis(500))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r###"{
      "url": "https://api.github.com/repos/sarmadiq9090-coder/arabic-file-search/releases/1",
      "tag_name": "v0.3.1",
      "name": "بحث سريع 0.3.1",
      "draft": false,
      "prerelease": false,
      "body": "## الجديد\r\n- تحديث تلقائي\r\n- إصلاحات",
      "assets": [
        {"name": "bahth-setup.zip", "size": 5, "browser_download_url": "https://github.com/x/y/releases/download/v0.3.1/bahth-setup.zip"},
        {"name": "bahth-arm64.exe", "size": 4321000, "browser_download_url": "https://github.com/sarmadiq9090-coder/arabic-file-search/releases/download/v0.3.1/bahth-arm64.exe"},
        {"name": "bahth.exe", "size": 4321000, "browser_download_url": "https://github.com/sarmadiq9090-coder/arabic-file-search/releases/download/v0.3.1/bahth.exe"},
        {"name": "bahth-x86.exe", "size": 4321000, "browser_download_url": "https://github.com/sarmadiq9090-coder/arabic-file-search/releases/download/v0.3.1/bahth-x86.exe"}
      ]
    }"###;

    #[test]
    fn release_from_web_page_redirect() {
        let url = "https://github.com/sarmadiq9090-coder/arabic-file-search/releases/tag/v0.4.1";
        assert_eq!(tag_from_release_url(url).as_deref(), Some("v0.4.1"));
        assert_eq!(tag_from_release_url(&format!("{url}?x=1")).as_deref(), Some("v0.4.1"));
        assert_eq!(tag_from_release_url("https://github.com/x/y/releases"), None);
        let r = release_from_tag("v0.4.1", "0.4.0").unwrap();
        assert_eq!(r.version, "0.4.1");
        assert_eq!(r.exe_size, 0);
        assert!(r.exe_url.ends_with(&format!("/releases/download/v0.4.1/{ASSET_NAME}")));
        assert!(release_from_tag("v0.4.0", "0.4.0").is_none());
        assert!(release_from_tag("v0.5.0-beta", "0.4.0").is_none());
    }

    #[test]
    fn versions() {
        assert_eq!(parse_version("v0.3.1"), Some(vec![0, 3, 1]));
        assert_eq!(parse_version("1.2"), Some(vec![1, 2]));
        assert_eq!(parse_version("1.2.3-beta"), None);
        assert_eq!(parse_version("1..2"), None);
        assert_eq!(parse_version(""), None);
        assert!(is_newer("v0.3.0", "0.2.2"));
        assert!(is_newer("0.2.10", "0.2.9"));
        assert!(is_newer("1.0", "0.99.99"));
        assert!(!is_newer("0.2.2", "0.2.2"));
        assert!(!is_newer("0.2.2.0", "0.2.2"));
        assert!(!is_newer("0.2.1", "0.2.2"));
        assert!(!is_newer("0.3.0-rc1", "0.2.2"));
        assert!(!is_newer("garbage", "0.2.2"));
    }

    #[test]
    fn parses_newer_release() {
        let r = parse_release(SAMPLE, "0.2.2").unwrap().unwrap();
        assert_eq!(r.version, "0.3.1");
        assert_eq!(r.tag, "v0.3.1");
        assert_eq!(r.exe_size, 4_321_000);
        assert!(r.exe_url.ends_with(&format!("/v0.3.1/{ASSET_NAME}")));
        assert_eq!(r.notes, "## الجديد\n- تحديث تلقائي\n- إصلاحات");        assert_eq!(r.sha256, None);
        let h = "AB".repeat(32);
        let with = SAMPLE.replace("\"size\": 4321000,", &format!("\"size\": 4321000, \"digest\": \"sha256:{h}\","));
        let r = parse_release(&with, "0.2.2").unwrap().unwrap();
        assert_eq!(r.sha256.as_deref(), Some("ab".repeat(32).as_str()));
    }

    #[test]
    fn no_checksum_means_no_install() {
        // GitHub gave no `digest`: refused, with the manual download link.
        let r = parse_release(SAMPLE, "0.2.2").unwrap().unwrap();
        let e = required_sha256(&r).unwrap_err();
        assert!(e.contains(MANUAL_DOWNLOAD_URL) && e.contains("SHA-256"), "{e}");
        assert_eq!(e, missing_digest_message());
        // Found through the web page: no checksum either until the API gives one.
        let r = release_from_tag("v0.4.1", "0.4.0").unwrap();
        assert!(required_sha256(&r).is_err());
        // A malformed digest is no digest.
        for bad in ["sha256:abc", "md5:", "sha256:", &format!("sha1:{}", "a".repeat(64)), &format!("sha256:{}", "g".repeat(64))] {
            let json = SAMPLE.replace("\"size\": 4321000,", &format!("\"size\": 4321000, \"digest\": \"{bad}\","));
            let r = parse_release(&json, "0.2.2").unwrap().unwrap();
            assert!(required_sha256(&r).is_err(), "{bad}");
        }
        // With one, it is what the download must match.
        let h = "CD".repeat(32);
        let json = SAMPLE.replace("\"size\": 4321000,", &format!("\"size\": 4321000, \"digest\": \"sha256:{h}\","));
        let r = parse_release(&json, "0.2.2").unwrap().unwrap();
        assert_eq!(required_sha256(&r), Ok("cd".repeat(32).as_str()));
    }

    /// A typical Windows environment for `package_manager_for`.
    fn win_env(k: &str) -> Option<String> {
        Some(
            match k {
                "USERPROFILE" => r"C:\Users\سرمد",
                "LOCALAPPDATA" => r"C:\Users\سرمد\AppData\Local",
                "ProgramData" => r"C:\ProgramData",
                "ProgramFiles" => r"C:\Program Files",
                "ProgramFiles(x86)" => r"C:\Program Files (x86)",
                _ => return None,
            }
            .to_string(),
        )
    }

    #[test]
    fn scoop_installs_are_left_to_scoop() {
        use PackageManager::Scoop;
        let pm = |p: &str| package_manager_for(p, win_env);
        assert_eq!(pm(r"C:\Users\سرمد\scoop\apps\instafind\current\bahth.exe"), Some(Scoop));
        assert_eq!(pm(r"C:\Users\سرمد\scoop\apps\instafind\0.9.0\bahth.exe"), Some(Scoop));
        assert_eq!(pm(r"c:\users\سرمد\SCOOP\Apps\instafind\current\BAHTH.EXE"), Some(Scoop));
        assert_eq!(pm(r"\\?\C:\Users\سرمد\scoop\apps\instafind\current\bahth.exe"), Some(Scoop));
        assert_eq!(pm("C:/Users/سرمد/scoop/apps/instafind/current/bahth.exe"), Some(Scoop));
        assert_eq!(pm(r"C:\Users\سرمد\scoop\shims\instafind.exe"), Some(Scoop));
        assert_eq!(pm(r"C:\ProgramData\scoop\apps\instafind\current\bahth.exe"), Some(Scoop));
        // A custom Scoop folder ($env:SCOOP, with a trailing backslash).
        let custom = |k: &str| if k == "SCOOP" { Some(r"D:\Tools\Scoop\".to_string()) } else { win_env(k) };
        assert_eq!(package_manager_for(r"D:\Tools\Scoop\apps\instafind\current\bahth.exe", custom), Some(Scoop));
        let global = |k: &str| if k == "SCOOP_GLOBAL" { Some(r"E:\GlobalScoop".to_string()) } else { None };
        assert_eq!(package_manager_for(r"E:\GlobalScoop\apps\instafind\current\bahth.exe", global), Some(Scoop));
        // Look-alikes are not Scoop.
        assert_eq!(pm(r"C:\Users\سرمد\scoop\apps2\bahth.exe"), None);
        assert_eq!(pm(r"C:\Users\سرمد\scoop\bahth.exe"), None);
        assert_eq!(pm(r"C:\Users\سرمد\scoop\apps"), None);
        assert_eq!(pm(r"C:\Users\سرمد\Downloads\scoop\apps\bahth.exe"), None);
        assert_eq!(pm(r"D:\Tools\Scoop\apps\instafind\current\bahth.exe"), None);
    }

    #[test]
    fn winget_installs_are_left_to_winget() {
        use PackageManager::Winget;
        let pm = |p: &str| package_manager_for(p, win_env);
        assert_eq!(
            pm(r"C:\Users\سرمد\AppData\Local\Microsoft\WinGet\Packages\Sarmad.InstaFind_Microsoft.Winget.Source_8wekyb3d8bbwe\bahth.exe"),
            Some(Winget)
        );
        assert_eq!(pm(r"C:\Users\سرمد\AppData\Local\Microsoft\WinGet\Links\instafind.exe"), Some(Winget));
        assert_eq!(pm(r"c:\users\سرمد\appdata\local\microsoft\winget\links\bahth.exe"), Some(Winget));
        assert_eq!(pm(r"C:\Program Files\WinGet\Packages\Sarmad.InstaFind_x\bahth.exe"), Some(Winget));
        assert_eq!(pm(r"C:\Program Files (x86)\WinGet\Links\instafind.exe"), Some(Winget));
        // The ordinary portable copy, and look-alikes, update themselves.
        assert_eq!(pm(r"C:\Users\سرمد\Downloads\bahth.exe"), None);
        assert_eq!(pm(r"C:\Users\سرمد\Desktop\WinGet\Packages\bahth.exe"), None);
        assert_eq!(pm(r"C:\Users\سرمد\AppData\Local\Microsoft\WinGet\bahth.exe"), None);
        assert_eq!(pm(r"C:\Users\سرمد\AppData\Local\Microsoft\WinGetX\Packages\bahth.exe"), None);
        assert_eq!(pm(r"C:\Program Files\InstaFind\bahth.exe"), None);
        assert_eq!(pm(""), None);
        // No environment at all (empty or missing variables): nothing matches.
        assert_eq!(package_manager_for(r"\scoop\apps\x\bahth.exe", |_| Some(String::new())), None);
        assert_eq!(package_manager_for(r"C:\Users\x\scoop\apps\x\bahth.exe", |_| None), None);
    }

    #[test]
    fn managed_message_names_the_update_command() {
        for (pm, cmd) in [(PackageManager::Scoop, "scoop update instafind"), (PackageManager::Winget, "winget upgrade Sarmad.InstaFind")] {
            let m = managed_message(pm);
            assert!(m.contains(pm.name()) && m.contains(cmd), "{m}");
            assert_eq!(pm.update_command(), cmd);
        }
    }

    #[test]
    fn ignores_same_older_draft_prerelease_and_missing_asset() {
        assert_eq!(parse_release(SAMPLE, "0.3.1").unwrap(), None);
        assert_eq!(parse_release(SAMPLE, "0.4.0").unwrap(), None);
        let draft = SAMPLE.replace(r#""draft": false"#, r#""draft": true"#);
        assert_eq!(parse_release(&draft, "0.2.2").unwrap(), None);
        let pre = SAMPLE.replace(r#""prerelease": false"#, r#""prerelease": true"#);
        assert_eq!(parse_release(&pre, "0.2.2").unwrap(), None);
        let tagged_pre = SAMPLE.replace("\"v0.3.1\"", "\"v0.3.1-beta\"");
        assert_eq!(parse_release(&tagged_pre, "0.2.2").unwrap(), None);
        let no_exe = SAMPLE.replace(&format!("\"{ASSET_NAME}\""), "\"other.exe\"");
        assert_eq!(parse_release(&no_exe, "0.2.2").unwrap(), None);
    }

    #[test]
    fn rejects_bad_json() {
        assert!(parse_release("not json", "0.2.2").is_err());
        assert!(parse_release("[]", "0.2.2").is_err());
        assert!(parse_release(r#"{"draft":false}"#, "0.2.2").is_err());
        let http = SAMPLE.replace("https://github.com/sarmadiq", "http://github.com/sarmadiq");
        assert!(parse_release(&http, "0.2.2").is_err());
        let huge = SAMPLE.replace("4321000", "999999999999");
        assert!(parse_release(&huge, "0.2.2").is_err());
    }

    #[test]
    fn each_cpu_gets_its_own_asset() {
        // The x64 name is what every installed copy since 0.3.0 looks for.
        assert_eq!(ASSET_X64, "bahth.exe");
        let expected = match std::env::consts::ARCH {
            "aarch64" => ASSET_ARM64,
            "x86" => ASSET_X86,
            _ => ASSET_X64,
        };
        assert_eq!(ASSET_NAME, expected);
        for name in [ASSET_X64, ASSET_ARM64, ASSET_X86] {
            let r = parse_release_for(SAMPLE, "0.2.2", name).unwrap().unwrap();
            assert!(r.exe_url.ends_with(&format!("/v0.3.1/{name}")), "{name}: {}", r.exe_url);
        }
        // The API path does not offer a release without this CPU's program
        // (no fallback to another CPU's exe, which validate_pe would refuse).
        let x64_only = SAMPLE
            .replace(&format!("\"{ASSET_ARM64}\""), "\"a.zip\"")
            .replace(&format!("\"{ASSET_X86}\""), "\"b.zip\"");
        assert!(parse_release_for(&x64_only, "0.2.2", ASSET_X64).unwrap().is_some());
        assert_eq!(parse_release_for(&x64_only, "0.2.2", ASSET_ARM64).unwrap(), None);
        assert_eq!(parse_release_for(&x64_only, "0.2.2", ASSET_X86).unwrap(), None);
    }

    #[test]
    fn notes_trimming() {
        assert_eq!(trim_notes("  short  ", 600), "short");
        let long = "كلمة ".repeat(300);
        let t = trim_notes(&long, 600);
        assert!(t.ends_with('…'));
        assert!(t.chars().count() <= 601);
        assert!(t.chars().count() > 400);
        assert!(!t.contains("  "));
        // No break point at all: hard cut on a char boundary.
        let solid = "ب".repeat(1000);
        assert_eq!(trim_notes(&solid, 600).chars().count(), 601);
    }

    fn pe(machine: u16) -> Vec<u8> {
        let mut v = vec![0u8; 512];
        v[0] = b'M';
        v[1] = b'Z';
        v[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        v[0x80..0x84].copy_from_slice(b"PE\0\0");
        v[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        v
    }

    #[test]
    fn pe_validation() {
        let native = if cfg!(target_arch = "x86") {
            0x014c
        } else if cfg!(target_arch = "aarch64") {
            0xaa64
        } else {
            0x8664
        };
        assert!(validate_pe(&pe(native)).is_ok());
        assert!(validate_pe(b"<html>not an exe</html>").is_err());
        let mut bad = pe(native);
        bad[0x80] = b'X';
        assert!(validate_pe(&bad).is_err());
        let mut far = pe(native);
        far[0x3c..0x40].copy_from_slice(&0x10000u32.to_le_bytes());
        assert!(validate_pe(&far).is_err());
        if cfg!(target_arch = "x86_64") {
            assert!(validate_pe(&pe(0x014c)).is_err());
        }
    }

    #[test]
    fn check_schedule() {
        let day = CHECK_INTERVAL_SECS;
        assert!(should_check_now(0, 1_000_000));
        assert!(!should_check_now(1_000_000, 1_000_000 + day - 1));
        assert!(should_check_now(1_000_000, 1_000_000 + day));
        assert!(should_check_now(1_000_000, 999_000)); // clock went back
    }

    #[test]
    fn url_split() {
        assert_eq!(
            split_https_url("https://api.github.com/repos/a/b/releases/latest").unwrap(),
            ("api.github.com".into(), 443, "/repos/a/b/releases/latest".into())
        );
        assert_eq!(split_https_url("https://h:8443").unwrap(), ("h".into(), 8443, "/".into()));
        assert_eq!(split_https_url("https://h?x=1").unwrap(), ("h".into(), 443, "/?x=1".into()));
        assert!(split_https_url("http://h/").is_err());
        assert!(split_https_url("https://u@h/").is_err());
    }

    /// A tiny xorshift generator: the same cases on every run.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    #[test]
    fn version_edge_cases() {
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(!is_newer("0.9.9", "0.10.0"));
        assert!(is_newer("v1.0.0", "0.99"));
        assert!(!is_newer("V0.5.1", "0.5.1"));
        // Too big for u64, other digits, signs, spaces inside, pre-releases.
        for bad in ["99999999999999999999999.0", "١.٢.٣", "+1.0", " 1 .0", "1.0-rc.1", "1.0+build", "vv1", "v", ".", "1."] {
            assert!(!is_newer(bad, "0.0.1"), "{bad}");
        }
        // Garbage as the current version never offers anything.
        assert!(!is_newer("9.9.9", "garbage"));
        assert!(!is_newer("9.9.9", ""));
    }

    #[test]
    fn is_newer_is_a_strict_order() {
        let mut rng = Rng(7);
        let parts = ["0", "1", "2", "9", "10", "010", "99999999999999999999", "", "a", "-1"];
        let mut v = || -> String {
            let n = 1 + rng.below(4);
            let s: Vec<&str> = (0..n).map(|_| parts[rng.below(parts.len())]).collect();
            format!("{}{}", ["", "v", "V", " "][rng.below(4)], s.join("."))
        };
        for _ in 0..5_000 {
            let (a, b, c) = (v(), v(), v());
            assert!(!is_newer(&a, &a), "{a}");
            assert!(!(is_newer(&a, &b) && is_newer(&b, &a)), "{a} {b}");
            if is_newer(&a, &b) && is_newer(&b, &c) {
                assert!(is_newer(&a, &c), "{a} {b} {c}");
            }
        }
    }

    #[test]
    fn hostile_release_json() {
        let cases = [
            "".to_string(),
            "null".into(),
            "\"x\"".into(),
            "{}".into(),
            r#"{"tag_name": 5}"#.into(),
            r#"{"tag_name": "v9.0.0", "assets": "nope"}"#.into(),
            r#"{"tag_name": "v9.0.0", "assets": [null, 1, "x", {"name": 5}]}"#.into(),
            r#"{"tag_name": "v9.0.0", "assets": [{"name": "ASSET"}]}"#.into(),
            r#"{"tag_name": "v9.0.0", "assets": [{"name": "ASSET", "browser_download_url": "https://github.com/a", "size": -1}]}"#.into(),
            r#"{"tag_name": "v9.0.0", "assets": [{"name": "ASSET", "browser_download_url": "https://github.com/a", "size": 1.5}]}"#.into(),
            r#"{"tag_name": "v9.0.0", "assets": [{"name": "ASSET", "browser_download_url": "https://github.com/a", "size": 0}]}"#.into(),
            r#"{"tag_name": "v9.0.0", "draft": "yes", "prerelease": null, "body": 7, "assets": []}"#.into(),
            format!("{}1{}", "[".repeat(10_000), "]".repeat(10_000)),
            format!("{{\"tag_name\": \"v9.0.0\", \"body\": \"{}\"}}", "\\u0628".repeat(10_000)),
        ]
        .map(|c| c.replace("\"ASSET\"", &format!("\"{ASSET_NAME}\"")));
        for c in &cases {
            let r = std::panic::catch_unwind(|| parse_release(c, "0.5.1"));
            assert!(r.is_ok(), "panic for {c:.80}");
            assert!(!matches!(r.unwrap(), Ok(Some(_))), "{c:.80}");
        }
    }

    #[test]
    fn download_url_must_be_on_github() {
        for (url, ok) in [
            ("https://github.com/sarmadiq9090-coder/arabic-file-search/releases/download/v1/bahth.exe", true),
            ("https://objects.githubusercontent.com/x", true),
            ("https://GitHub.com/x", true),
            ("https://evil.example/bahth.exe", false),
            ("https://github.com.evil.example/bahth.exe", false),
            ("https://evilgithub.com/x", false),
            ("https://github.com:8443/x", false),
            ("https://u@github.com/x", false),
            ("http://github.com/x", false),
        ] {
            assert_eq!(download_host_ok(url), ok, "{url}");
        }
        let evil = SAMPLE.replace(
            &format!("https://github.com/sarmadiq9090-coder/arabic-file-search/releases/download/v0.3.1/{ASSET_NAME}\""),
            "https://evil.example/bahth.exe\"",
        );
        assert!(parse_release(&evil, "0.2.2").is_err());
    }

    #[test]
    fn pe_offset_near_the_top_of_u32() {
        let mut v = pe(0x8664);
        for off in [u32::MAX, u32::MAX - 5, u32::MAX - 6, 0x3f, 0] {
            v[0x3c..0x40].copy_from_slice(&off.to_le_bytes());
            assert!(validate_pe(&v).is_err(), "{off:#x}");
        }
        let mut rng = Rng(99);
        for _ in 0..5_000 {
            let n = rng.below(600);
            let mut b: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
            if n >= 2 && rng.below(2) == 0 {
                b[0] = b'M';
                b[1] = b'Z';
            }
            let _ = validate_pe(&b);
        }
    }

    #[test]
    fn random_notes_and_urls_never_panic() {
        let mut rng = Rng(1234);
        let pieces = ["a", " ", "\n", "\r\n", "ب", "\u{0651}", "\u{1F600}", "/releases/tag/", "?", "#", "/", "https://", ":", "@", "v1.2"];
        for _ in 0..5_000 {
            let t: String = (0..rng.below(40)).map(|_| pieces[rng.below(pieces.len())]).collect();
            let max = rng.below(30);
            let n = trim_notes(&t, max);
            assert!(n.chars().count() <= max + 1, "{t:?} {max} {n:?}");
            let _ = tag_from_release_url(&t);
            let _ = split_https_url(&t);
            let _ = download_host_ok(&t);
            if let Some(tag) = tag_from_release_url(&t)
                && let Some(r) = release_from_tag(&tag, "0.0.1")
            {
                assert!(r.exe_url.starts_with("https://github.com/") && !r.exe_url.contains(['?', '#', ' ']));
            }
        }
    }

    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new(name: &str) -> TempDir {
            let d = std::env::temp_dir().join(format!("bahth_update_{name}_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            TempDir(d)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn swap_in_never_leaves_no_program() {
        let t = TempDir::new("swap");
        let exe = t.0.join("bahth.exe");
        let (new, old) = (sibling(&exe, ".new"), sibling(&exe, ".old"));
        assert!(old.ends_with("bahth.exe.old"));
        std::fs::write(&exe, b"v1").unwrap();
        // The download is missing: nothing is touched.
        assert!(swap_in(&exe, &new, &old).is_err());
        assert_eq!(std::fs::read(&exe).unwrap(), b"v1");
        // The download is a folder (cannot be renamed over a file): rolled back.
        std::fs::create_dir(&new).unwrap();
        std::fs::write(new.join("x"), b"").unwrap();
        assert!(swap_in(&exe, &new, &old).is_err());
        assert_eq!(std::fs::read(&exe).unwrap(), b"v1");
        std::fs::remove_dir_all(&new).unwrap();
        // The normal case, with a stale .old from before.
        std::fs::write(&old, b"v0").unwrap();
        std::fs::write(&new, b"v2").unwrap();
        swap_in(&exe, &new, &old).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"v2");
        assert_eq!(std::fs::read(&old).unwrap(), b"v1");
        assert!(!new.exists());
    }
}
