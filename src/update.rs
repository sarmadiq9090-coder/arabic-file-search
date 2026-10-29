//! Auto-update from GitHub Releases.
//!
//! The pure logic (version comparison, parsing the GitHub API answer, notes
//! trimming, PE validation, check scheduling) builds on every platform so it
//! can be unit-tested on Linux. Everything that touches the network or the
//! file system of the installed program is Windows-only.
//!
//! All Windows functions block; call them from a background thread except
//! `cleanup_old` and `wait_for_previous_instance_exit`, which are meant for
//! startup.


/// GitHub API endpoint for the newest published (non-draft, non-prerelease) release.
pub const LATEST_URL: &str =
    "https://api.github.com/repos/sarmadiq9090-coder/arabic-file-search/releases/latest";
/// Named mutex that keeps a single copy of the program running (ui.rs).
pub const SINGLE_INSTANCE_MUTEX: &str = "Local\\BahthSari.SingleInstance";
/// Name of the release asset that holds the program.
pub const ASSET_NAME: &str = "bahth.exe";
/// Upper bound for the JSON answer.
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
    pub sha256: Option<String>,
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
        .and_then(|a| a.iter().find(|x| x["name"].as_str() == Some(ASSET_NAME)));
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

/// Checks that `head` (the first bytes of a file) is a PE image for this
/// architecture: "MZ", a sane e_lfanew, "PE\0\0" and a matching machine type.
pub fn validate_pe(head: &[u8]) -> Result<(), String> {
    if head.len() < 0x40 || &head[..2] != b"MZ" {
        return Err("downloaded file is not a Windows program (no MZ)".into());
    }
    let lfanew = u32::from_le_bytes([head[0x3c], head[0x3d], head[0x3e], head[0x3f]]) as usize;
    if lfanew < 0x40 || lfanew + 6 > head.len() {
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

#[cfg(windows)]
#[allow(unused_imports)]
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
    /// (at startup, through `cleanup_old`), before any rename can happen.
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

    fn sibling(exe: &Path, suffix: &str) -> PathBuf {
        let mut name = exe.file_name().unwrap_or_default().to_os_string();
        name.push(suffix);
        exe.with_file_name(name)
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
                12002 => "انتهت مهلة الاتصال".into(),
                12007 => "تعذر الوصول إلى الخادم (هل الإنترنت متصل؟)".into(),
                12029 | 12030 => "فشل الاتصال بالخادم".into(),
                12175 => "فشل الاتصال الآمن (TLS)".into(),
                _ => format!("فشل الطلب (خطأ {code})"),
            });
        }
        let status = query_number(&request, WINHTTP_QUERY_STATUS_CODE).unwrap_or(0);
        if status != 200 {
            return Err(match status {
                403 | 429 => format!("رفض الخادم الطلب مؤقتاً (HTTP {status}). حاول لاحقاً."),
                404 => "لا يوجد إصدار منشور بعد (HTTP 404)".into(),
                _ => format!("رد غير متوقع من الخادم (HTTP {status})"),
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
            return Err("انقطع التنزيل قبل اكتماله".into());
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
        )?;
        let text = String::from_utf8(body).map_err(|_| "GitHub answer is not UTF-8".to_string())?;
        parse_release(&text, env!("CARGO_PKG_VERSION"))
    }

    /// Downloads `rel` next to the program and swaps it in place of the
    /// running exe (which is kept as `bahth.exe.old` until the next start).
    /// `progress(done, total)` is called as bytes arrive. Does not restart.
    pub fn download_and_install(rel: &Release, progress: impl Fn(u64, u64)) -> Result<(), String> {
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
        if rel.exe_size == 0 || rel.exe_size > MAX_DOWNLOAD {
            return Err("the update has an invalid size".into());
        }
        let file = fs::File::create(new).map_err(|e| format!("cannot create {}: {e}", new.display()))?;
        let mut out = std::io::BufWriter::with_capacity(256 * 1024, file);
        let total = rel.exe_size;
        let mut done: u64 = 0;
        let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
        progress(0, total);
        http_get(
            &rel.exe_url,
            "application/octet-stream",
            total.min(MAX_DOWNLOAD),
            &mut |len| match len {
                Some(n) if n != total => Err(format!("the server sent {n} bytes, expected {total}")),
                _ => Ok(()),
            },
            &mut |chunk| {
                out.write_all(chunk).map_err(|e| format!("cannot write the update: {e}"))?;
                sha2::Digest::update(&mut hasher, chunk);
                done += chunk.len() as u64;
                progress(done, total);
                Ok(())
            },
        )?;
        let file = out.into_inner().map_err(|e| format!("cannot write the update: {e}"))?;
        file.sync_all().map_err(|e| format!("cannot write the update: {e}"))?;
        drop(file);

        if let Some(want) = &rel.sha256 {
            let got: String = sha2::Digest::finalize(hasher).iter().map(|b| format!("{b:02x}")).collect();
            if &got != want {
                return Err("الملف الذي نُزّل لا يطابق بصمة الإصدار (SHA-256)".into());
            }
        }
        let size = fs::metadata(new).map_err(|e| e.to_string())?.len();
        if size != total {
            return Err(format!("downloaded {size} bytes, expected {total}"));
        }
        let mut head = vec![0u8; 4096];
        let n = fs::File::open(new)
            .and_then(|mut f| f.read(&mut head))
            .map_err(|e| format!("cannot read the update: {e}"))?;
        head.truncate(n);
        validate_pe(&head)
    }

    fn swap_in(exe: &Path, new: &Path, old: &Path) -> Result<(), String> {
        // A leftover from an earlier update; if it cannot be removed (still
        // running?) the rename below reports the problem.
        let _ = fs::remove_file(old);
        fs::rename(exe, old).map_err(|e| format!("cannot move the running program aside: {e}"))?;
        if let Err(e) = fs::rename(new, exe) {
            // Roll back: put the original program back under its name.
            let back = fs::rename(old, exe);
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

    /// Starts the (new) program at its original path with `--updated`. The
    /// caller should exit right after a successful return.
    pub fn restart_new_version() -> Result<(), String> {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        let exe = original_exe()?;
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("--updated").creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
        if let Some(dir) = exe.parent() {
            cmd.current_dir(dir);
        }
        cmd.spawn().map(drop).map_err(|e| format!("cannot start the new version: {e}"))
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

    /// Deletes a `bahth.exe.old` (and a stale `.new`) left by an update.
    /// Call at every startup; errors are ignored.
    pub fn cleanup_old() {
        if let Ok(exe) = original_exe() {
            let _ = fs::remove_file(sibling(exe, ".old"));
            let _ = fs::remove_file(sibling(exe, ".new"));
        }
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
        {"name": "bahth.exe", "size": 4321000, "browser_download_url": "https://github.com/sarmadiq9090-coder/arabic-file-search/releases/download/v0.3.1/bahth.exe"}
      ]
    }"###;

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
        assert!(r.exe_url.ends_with("/v0.3.1/bahth.exe"));
        assert_eq!(r.notes, "## الجديد\n- تحديث تلقائي\n- إصلاحات");        assert_eq!(r.sha256, None);
        let h = "AB".repeat(32);
        let with = SAMPLE.replace("\"size\": 4321000,", &format!("\"size\": 4321000, \"digest\": \"sha256:{h}\","));
        let r = parse_release(&with, "0.2.2").unwrap().unwrap();
        assert_eq!(r.sha256.as_deref(), Some("ab".repeat(32).as_str()));
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
        let no_exe = SAMPLE.replace("\"bahth.exe\"", "\"other.exe\"");
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
}
