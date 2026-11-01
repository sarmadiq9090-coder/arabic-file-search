//! Recognises Windows system files and folders (the Windows folder, the
//! recycle bin, page files, temp and cache folders under user profiles…)
//! so searches can leave them out. The index keeps them; the filter runs at
//! search time, so switching it is instant.
//!
//! [`is_system_path`] runs on every candidate of every search, so it never
//! allocates: it walks the path's bytes and compares ASCII case-insensitively.

/// Folders directly under a drive root that hold only system data.
const ROOT_DIRS: &[&str] = &[
    "windows",
    "windows.old",
    "$recycle.bin",
    "recycler",
    "$winreagent",
    "$sysreset",
    "$windows.~bt",
    "$windows.~ws",
    "$getcurrent",
    "$windows.old",
    "config.msi",
    "recovery",
    "perflogs",
    "system volume information",
    "msocache",
    "programdata",
    "documents and settings",
];

/// Files directly under a drive root that belong to Windows.
const ROOT_FILES: &[&str] = &[
    "pagefile.sys",
    "hiberfil.sys",
    "swapfile.sys",
    "dumpstack.log",
    "dumpstack.log.tmp",
    "bootmgr",
    "bootnxt",
    "bootsect.bak",
];

/// Profiles under `Users` that are templates or links, not a person's files.
const SYSTEM_PROFILES: &[&str] = &["default", "default user", "all users"];

/// Files at the root of a user profile that hold the registry hive.
const PROFILE_FILES: &[&str] = &["ntuser.ini", "ntuser.pol", "usrclass.dat"];

/// Folders under `AppData\Local` that hold only Windows and app data.
const LOCAL_DIRS: &[&str] = &["temp", "microsoft", "packages", "crashdumps", "d3dscache"];

/// Cache folders of browsers and Electron apps, hidden anywhere below
/// `AppData\Local\<app>` or `AppData\Roaming\<app>`.
const CACHE_DIRS: &[&str] = &[
    "cache",
    "cache2",
    "cache_data",
    "code cache",
    "gpucache",
    "shadercache",
    "grshadercache",
    "graphitedawncache",
    "dawncache",
    "dawngraphitecache",
    "dawnwebgpucache",
    "cachestorage",
    "scriptcache",
];

fn is_sep(b: u8) -> bool {
    b == b'\\' || b == b'/'
}

fn eq(a: &[u8], b: &str) -> bool {
    a.eq_ignore_ascii_case(b.as_bytes())
}

fn any_eq(a: &[u8], list: &[&str]) -> bool {
    list.iter().any(|b| eq(a, b))
}

fn starts_with(a: &[u8], prefix: &str) -> bool {
    a.len() >= prefix.len() && eq(&a[..prefix.len()], prefix)
}

/// True when `path` (a full Windows path such as `C:\Windows\notepad.exe`,
/// with `\` or `/`, any drive letter, optionally `\\?\`-prefixed) is a
/// system file or folder that searches hide by default. Paths without a
/// drive letter (network shares, relative paths) are never system paths.
pub fn is_system_path(path: &str) -> bool {
    let mut b = path.as_bytes();
    // `\\?\`, `\\.\` and `\??\` prefixes.
    if b.len() >= 4 && is_sep(b[0]) && matches!(b[1], b'?' | b'\\' | b'/') && matches!(b[2], b'?' | b'.') && is_sep(b[3]) {
        b = &b[4..];
    }
    // A drive letter and a root separator.
    if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' || !is_sep(b[2]) {
        return false;
    }
    let mut parts = b[3..].split(|&c| is_sep(c)).filter(|p| !p.is_empty());
    let Some(top) = parts.next() else { return false };
    let Some(second) = parts.next() else {
        // Directly under the root: a system folder or a Windows file.
        return any_eq(top, ROOT_DIRS) || any_eq(top, ROOT_FILES);
    };
    if any_eq(top, ROOT_DIRS) {
        return true;
    }
    if !eq(top, "users") {
        return false;
    }
    // `Users\<profile>\…`
    if any_eq(second, SYSTEM_PROFILES) {
        return true;
    }
    let Some(third) = parts.next() else { return false };
    let Some(fourth) = parts.next() else {
        // At the profile root: the registry hive and its logs.
        return starts_with(third, "ntuser.dat") || any_eq(third, PROFILE_FILES);
    };
    if !eq(third, "appdata") {
        return false;
    }
    // `Users\<profile>\AppData\<fourth>\…`
    if eq(fourth, "locallow") {
        return true;
    }
    let local = eq(fourth, "local");
    if !local && !eq(fourth, "roaming") {
        return false;
    }
    let Some(app) = parts.next() else { return false };
    if local && any_eq(app, LOCAL_DIRS) {
        return true;
    }
    parts.any(|p| any_eq(p, CACHE_DIRS))
}

/// Whether a search should include system files: when the "hide system
/// files" option (`hide`) is off, or when the search is limited to a folder
/// that is itself a system folder (the user asked for it explicitly).
pub fn show_system(hide: bool, scope: &str) -> bool {
    !hide || is_system_path(scope)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_and_root_folders() {
        for p in [
            r"C:\Windows",
            r"C:\Windows\",
            r"C:\Windows\System32\notepad.exe",
            r"c:\WINDOWS\WinSxS\x.dll",
            r"D:\windows\x",
            r"C:/Windows/System32",
            r"C:\$Recycle.Bin\S-1-5-21\$R123.docx",
            r"E:\$RECYCLE.BIN",
            r"C:\$WinREAgent\Scratch",
            r"C:\$SysReset\Logs\a.log",
            r"C:\$Windows.~BT\Sources\setup.exe",
            r"C:\$Windows.~WS\x",
            r"C:\$GetCurrent\media",
            r"C:\Config.Msi\abc.rbf",
            r"C:\Recovery\WindowsRE\Winre.wim",
            r"C:\PerfLogs",
            r"C:\System Volume Information\tracking.log",
            r"C:\MSOCache\All Users\x.msi",
            r"C:\ProgramData\Microsoft\Windows\Start Menu\x.lnk",
            r"C:\programdata",
            r"C:\Windows.old\Users\a\Documents\x.docx",
        ] {
            assert!(is_system_path(p), "{p}");
        }
    }

    #[test]
    fn root_files() {
        for p in [
            r"C:\pagefile.sys",
            r"C:\hiberfil.sys",
            r"C:\swapfile.sys",
            r"C:\DumpStack.log",
            r"C:\DumpStack.log.tmp",
            r"C:\bootmgr",
            r"C:\BOOTNXT",
            r"C:\BOOTSECT.BAK",
            r"d:\PAGEFILE.SYS",
        ] {
            assert!(is_system_path(p), "{p}");
        }
        // Same names deeper down are ordinary files.
        assert!(!is_system_path(r"C:\Backup\pagefile.sys"));
        assert!(!is_system_path(r"C:\Users\a\Desktop\bootmgr"));
        assert!(!is_system_path(r"C:\Data\Windows\notes.txt"));
        assert!(!is_system_path(r"C:\Projects\ProgramData\x"));
    }

    #[test]
    fn user_profile_system_data() {
        for p in [
            r"C:\Users\Ahmad\NTUSER.DAT",
            r"C:\Users\Ahmad\ntuser.dat.LOG1",
            r"C:\Users\Ahmad\NTUSER.DAT{53b39e88-18c4-11ea-a811-000d3aa4692b}.TM.blf",
            r"C:\Users\Ahmad\ntuser.ini",
            r"C:\Users\Ahmad\UsrClass.dat",
            r"C:\Users\Ahmad\AppData\Local\Temp\x.tmp",
            r"C:\Users\Ahmad\AppData\Local\Temp",
            r"C:\Users\Ahmad\AppData\Local\Microsoft\Windows\INetCache\x",
            r"C:\Users\Ahmad\AppData\Local\Packages\Microsoft.Windows.Photos\x",
            r"C:\Users\Ahmad\AppData\LocalLow\Sun\Java",
            r"C:\Users\Ahmad\AppData\LocalLow",
            r"C:\Users\Ahmad\AppData\Local\CrashDumps\app.dmp",
            r"C:\Users\Ahmad\AppData\Local\D3DSCache\x",
            r"C:\Users\Ahmad\AppData\Local\Google\Chrome\User Data\Default\Cache\Cache_Data\f_000001",
            r"C:\Users\Ahmad\AppData\Local\Google\Chrome\User Data\Default\Code Cache\js\x",
            r"C:\Users\Ahmad\AppData\Local\Google\Chrome\User Data\Profile 1\GPUCache\data_0",
            r"C:\Users\Ahmad\AppData\Local\Google\Chrome\User Data\Default\Service Worker\CacheStorage\x",
            r"C:\Users\Ahmad\AppData\Local\BraveSoftware\Brave-Browser\User Data\Default\Cache",
            r"C:\Users\Ahmad\AppData\Local\Mozilla\Firefox\Profiles\ab.default\cache2\entries\x",
            r"C:\Users\Ahmad\AppData\Roaming\Code\Cache\Cache_Data\x",
            r"C:\Users\Ahmad\AppData\Roaming\discord\GPUCache\x",
            r"c:/users/ahmad/appdata/local/temp/x",
            r"C:\Users\Default\NTUSER.DAT",
            r"C:\Users\Default\Desktop",
            r"C:\Users\All Users\x",
        ] {
            assert!(is_system_path(p), "{p}");
        }
    }

    #[test]
    fn user_files_and_programs_stay_visible() {
        for p in [
            "",
            "C:",
            r"C:\",
            r"C:\Users",
            r"C:\Users\Ahmad",
            r"C:\Users\Ahmad\Desktop\report.docx",
            r"C:\Users\Ahmad\Documents\تقرير.pdf",
            r"C:\Users\Ahmad\Downloads\setup.exe",
            r"C:\Users\Ahmad\AppData",
            r"C:\Users\Ahmad\AppData\Local",
            r"C:\Users\Ahmad\AppData\Roaming",
            r"C:\Users\Ahmad\AppData\Roaming\Microsoft\Word\notes.asd",
            r"C:\Users\Ahmad\AppData\Roaming\Mozilla\Firefox\Profiles\x\places.sqlite",
            r"C:\Users\Ahmad\AppData\Local\Programs\Microsoft VS Code\Code.exe",
            r"C:\Users\Ahmad\AppData\Local\Google\Chrome\User Data\Default\Bookmarks",
            r"C:\Users\Ahmad\AppData\Local\Cache",
            r"C:\Users\Ahmad\Documents\Cache\notes.txt",
            r"C:\Users\Ahmad\Documents\Temp\x.txt",
            r"C:\Users\Ahmad\Documents\NTUSER.DAT",
            r"C:\Users\Public\Documents\x.pdf",
            r"C:\Program Files\App\app.exe",
            r"C:\Program Files (x86)\App\app.exe",
            r"D:\Movies\Windows Tips.mp4",
            r"D:\Windowsx\x",
            r"D:\Windows Backup\x",
            r"C:\Windowsold",
            r"Windows\System32",
            r"\Windows\System32",
            r"\\server\share\Windows\x",
            r"\\?\UNC\server\share\Windows\x",
            r"1:\Windows",
            r"CC:\Windows",
        ] {
            assert!(!is_system_path(p), "{p}");
        }
    }

    #[test]
    fn long_path_prefixes_and_odd_separators() {
        assert!(is_system_path(r"\\?\C:\Windows\System32\x.dll"));
        assert!(is_system_path(r"\\.\C:\$Recycle.Bin"));
        assert!(is_system_path(r"\??\C:\pagefile.sys"));
        assert!(is_system_path("//?/C:/Windows"));
        assert!(is_system_path(r"C:\\Windows\\System32"));
        assert!(is_system_path(r"C:\Users\\a\AppData\Local\\Temp"));
        assert!(!is_system_path(r"\\?\C:\Users\a\Documents\x"));
        assert!(!is_system_path(r"\\?\"));
        let long = format!(r"\\?\C:\Users\a\Documents\{}\x.txt", "d\\".repeat(200));
        assert!(!is_system_path(&long));
        let long = format!(r"\\?\C:\Users\a\AppData\Local\App\{}Cache\x", "d\\".repeat(200));
        assert!(is_system_path(&long));
    }

    #[test]
    fn non_ascii_names_never_panic() {
        // Multi-byte names next to the compared ASCII ones.
        for p in ["C:\\ويندوز\\x", "C:\\Users\\أحمد\\AppData\\Local\\Temp\\ملف", "C:\\Windowsé", "é:\\Windows", "C:\\\u{0}", "C:\\Users\\😀\\NTUSER.DAT"] {
            let _ = is_system_path(p);
        }
        assert!(is_system_path("C:\\Users\\أحمد\\AppData\\Local\\Temp\\ملف"));
        assert!(is_system_path("C:\\Users\\😀\\NTUSER.DAT"));
        assert!(!is_system_path("C:\\Windowsé"));
        // Random bytes-as-text: no panic.
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let alphabet: Vec<char> = "C:\\/?.$Windows Users AppData Local Temp ntuser.dat أحمد😀\u{0}".chars().collect();
        for _ in 0..5000 {
            let mut s = String::new();
            for _ in 0..(x % 40) {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                s.push(alphabet[(x % alphabet.len() as u64) as usize]);
            }
            let _ = is_system_path(&s);
        }
    }

    #[test]
    fn scope_choice() {
        assert!(!show_system(true, ""));
        assert!(!show_system(true, r"C:\Users\a\"));
        assert!(show_system(false, ""));
        // Searching inside a system folder shows what is in it.
        assert!(show_system(true, r"C:\Windows\"));
        assert!(show_system(true, r"C:\Users\a\AppData\Local\Temp\"));
    }
}
