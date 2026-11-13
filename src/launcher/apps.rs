//! Installed programs: Start menu shortcuts and Store apps.
//!
//! [`scan`] lists the shell's Applications folder (`shell:AppsFolder`),
//! which holds desktop programs and Store apps exactly as Start shows them.
//! If that fails it walks the two Start Menu\Programs folders instead.

use std::path::{Path, PathBuf};

use super::aliases;
use crate::normalize::normalize;

/// One installed program.
#[derive(Clone, Debug, PartialEq)]
pub struct App {
    /// Display name as Windows shows it ("Google Chrome").
    pub name: String,
    /// `.lnk` path or `shell:AppsFolder\<AUMID>`.
    pub target: String,
    /// Extra names it should be found by (Arabic names, abbreviations).
    pub aliases: Vec<String>,
    /// File the icon can be taken from (the `.exe` or `.lnk`), when known.
    pub icon_path: Option<String>,
}

impl App {
    /// A program called `name` started through `target`, with the Arabic
    /// names of [`aliases`] filled in.
    pub fn new(name: String, target: String, icon_path: Option<String>) -> App {
        let aliases = aliases::for_name(&name);
        App { name, target, aliases, icon_path }
    }
}

/// Words in a name that mark an entry nobody wants to start from search:
/// uninstallers, read-me files, help and web links put next to a program.
const JUNK_WORDS: &[&str] = &[
    "uninstall",
    "uninstaller",
    "uninst",
    "unins000",
    "readme",
    "read me",
    "help",
    "website",
    "web site",
    "homepage",
    "home page",
    "release notes",
    "license",
    "licence",
    "changelog",
    "documentation",
    "manual",
    "إلغاء التثبيت",
    "إلغاء تثبيت",
    "إزالة التثبيت",
    "ازالة التثبيت",
    "تعليمات",
    "اقرأني",
];

fn is_junk(name: &str, target: &str) -> bool {
    let n = normalize(name);
    let words_match = |hay: &str, pat: &str| {
        let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
        hay.match_indices(pat).any(|(i, m)| !word(hay[..i].chars().next_back()) && !word(hay[i + m.len()..].chars().next()))
    };
    // "Get Help" is a Windows app, not a help file.
    if n == "get help" {
        return false;
    }
    if n.trim().is_empty() || JUNK_WORDS.iter().any(|w| words_match(&n, &normalize(w))) {
        return true;
    }
    // Uninstallers whose name does not say so ("Foo Remove"): look at the file.
    let file = target.rsplit(['\\', '/']).next().unwrap_or(target).to_lowercase();
    file.starts_with("unins") || file.starts_with("uninst") || file.contains("uninstall")
}

/// Drops uninstallers, read-me and help entries, empty names and
/// duplicates (same name once normalised, the first one kept), and sorts
/// by name.
pub fn clean(apps: Vec<App>) -> Vec<App> {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<App> = apps
        .into_iter()
        .filter_map(|mut a| {
            a.name = a.name.trim().to_string();
            if is_junk(&a.name, &a.target) || a.target.trim().is_empty() {
                return None;
            }
            seen.insert(normalize(&a.name)).then_some(a)
        })
        .collect();
    out.sort_by_cached_key(|a| normalize(&a.name));
    out
}

/// Shortcut files the Start menu shows.
fn is_shortcut(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["lnk", "url", "appref-ms"].iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Programs from the shortcuts under `roots` (the Start Menu\Programs
/// folders), at most 6 folders deep. The shortcut file is both the target
/// and the icon; its name without the extension is the program name.
pub fn walk_start_menu(roots: &[PathBuf]) -> Vec<App> {
    fn walk(dir: &Path, depth: u32, out: &mut Vec<App>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for entry in rd.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                if depth < 6 {
                    walk(&path, depth + 1, out);
                }
            } else if is_shortcut(&path) {
                let Some(name) = path.file_stem().and_then(|s| s.to_str()) else { continue };
                let target = path.to_string_lossy().into_owned();
                out.push(App::new(name.to_string(), target.clone(), Some(target)));
            }
        }
    }
    let mut out = Vec::new();
    for r in roots {
        walk(r, 0, &mut out);
    }
    out
}

/// The two Start Menu\Programs folders: all users, then the current user.
#[cfg(windows)]
fn start_menu_roots() -> Vec<PathBuf> {
    let sub = r"Microsoft\Windows\Start Menu\Programs";
    ["ProgramData", "APPDATA"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|base| PathBuf::from(base).join(sub))
        .collect()
}

/// Installed programs, as Start lists them, cleaned up and sorted. Never
/// panics: on any failure it returns what it found (possibly nothing).
/// Starts COM (apartment-threaded) on the calling thread if needed, so call
/// it from a background thread.
#[cfg(windows)]
pub fn scan() -> Vec<App> {
    let found = std::panic::catch_unwind(apps_folder::list).unwrap_or_default();
    let found = if found.is_empty() {
        std::panic::catch_unwind(|| walk_start_menu(&start_menu_roots())).unwrap_or_default()
    } else {
        found
    };
    clean(found)
}

/// For an Applications folder parsing name that is a file path, either
/// absolute ("C:\…\app.exe") or below a known folder ("{GUID}\Vendor\app.exe"),
/// the GUID and the rest. Store app ids (`Microsoft.WindowsCalculator_8we…!App`)
/// give `None`.
fn known_folder_path(parsing: &str) -> Option<(&str, &str)> {
    let rest = parsing.strip_prefix('{')?;
    let close = rest.find('}')?;
    let guid = &rest[..close];
    let tail = rest[close + 1..].strip_prefix('\\')?;
    (guid.len() == 36 && !tail.is_empty()).then_some((guid, tail))
}

/// Whether a parsing name points to a program file usable as an icon source.
fn is_program_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".exe", ".lnk", ".msc", ".cpl", ".bat", ".cmd"].iter().any(|e| lower.ends_with(e))
}

#[cfg(windows)]
mod apps_folder {
    //! `shell:AppsFolder` through COM: SHGetKnownFolderItem, then
    //! IShellItem::BindToHandler(BHID_EnumItems) and IEnumShellItems.

    use winapi::ctypes::c_void;
    use std::ptr::null_mut;

    use winapi::Interface;
    use winapi::shared::guiddef::{GUID, REFIID};
    use winapi::shared::minwindef::ULONG;
    use winapi::shared::winerror::{HRESULT, S_FALSE, S_OK, SUCCEEDED};
    use winapi::um::combaseapi::{CoInitializeEx, CoTaskMemFree, CoUninitialize};
    use winapi::um::knownfolders::FOLDERID_AppsFolder;
    use winapi::um::objbase::COINIT_APARTMENTTHREADED;
    use winapi::um::shlobj::{KF_FLAG_DEFAULT, SHGetKnownFolderItem, SHGetKnownFolderPath};
    use winapi::um::shobjidl_core::{IShellItem, SIGDN, SIGDN_NORMALDISPLAY};
    use winapi::um::unknwnbase::{IUnknown, IUnknownVtbl};
    use winapi::um::winnt::{LPWSTR, PWSTR};

    use super::App;

    /// `SIGDN_PARSINGNAME`, the item's name relative to its parent folder
    /// (winapi spells it SIGDN_PARENTRELATIVEPARSING).
    const SIGDN_PARSINGNAME: SIGDN = 0x8001_8001;

    /// BHID_EnumItems {94F60519-2850-4924-AA5A-D15E84868039}.
    const BHID_ENUM_ITEMS: GUID = GUID {
        Data1: 0x94f6_0519,
        Data2: 0x2850,
        Data3: 0x4924,
        Data4: [0xaa, 0x5a, 0xd1, 0x5e, 0x84, 0x86, 0x80, 0x39],
    };

    /// IID_IEnumShellItems {70629033-E363-4A28-A567-0DB78006E6D7}.
    const IID_IENUM_SHELL_ITEMS: GUID = GUID {
        Data1: 0x7062_9033,
        Data2: 0xe363,
        Data3: 0x4a28,
        Data4: [0xa5, 0x67, 0x0d, 0xb7, 0x80, 0x06, 0xe6, 0xd7],
    };

    /// IEnumShellItems, which winapi 0.3 does not define: IUnknown then
    /// Next, Skip, Reset and Clone (only Next is called).
    #[repr(C)]
    #[allow(dead_code)]
    struct IEnumShellItemsVtbl {
        parent: IUnknownVtbl,
        next: unsafe extern "system" fn(
            this: *mut IEnumShellItems,
            celt: ULONG,
            rgelt: *mut *mut IShellItem,
            fetched: *mut ULONG,
        ) -> HRESULT,
        skip: unsafe extern "system" fn(this: *mut IEnumShellItems, celt: ULONG) -> HRESULT,
        reset: unsafe extern "system" fn(this: *mut IEnumShellItems) -> HRESULT,
        clone: unsafe extern "system" fn(this: *mut IEnumShellItems, out: *mut *mut IEnumShellItems) -> HRESULT,
    }

    #[repr(C)]
    struct IEnumShellItems {
        vtbl: *const IEnumShellItemsVtbl,
    }

    /// Releases a COM pointer when dropped.
    struct Com<T>(*mut T);

    impl<T> Drop for Com<T> {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: every COM interface starts with IUnknown, and the
                // pointer is a live reference this guard owns.
                unsafe { (*(self.0 as *mut IUnknown)).Release() };
            }
        }
    }

    /// Ends COM on this thread if `CoInitializeEx` started it here.
    struct ComInit(bool);

    impl Drop for ComInit {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: paired with a successful CoInitializeEx on this thread.
                unsafe { CoUninitialize() };
            }
        }
    }

    /// Takes a string the shell allocated with CoTaskMemAlloc.
    fn take_string(p: LPWSTR) -> Option<String> {
        if p.is_null() {
            return None;
        }
        // SAFETY: the shell returned a NUL-terminated wide string.
        let s = unsafe {
            let mut len = 0usize;
            while *p.add(len) != 0 {
                len += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
        };
        // SAFETY: allocated by the shell with CoTaskMemAlloc, freed once.
        unsafe { CoTaskMemFree(p as *mut c_void) };
        Some(s)
    }

    fn display_name(item: *mut IShellItem, sigdn: SIGDN) -> Option<String> {
        let mut p: LPWSTR = null_mut();
        // SAFETY: `item` is a live IShellItem; `p` receives an owned string.
        let hr = unsafe { (*item).GetDisplayName(sigdn, &mut p) };
        if SUCCEEDED(hr) { take_string(p) } else { None }
    }

    /// The path of the known folder with this GUID text
    /// ("6D809377-6AF0-444B-8957-A3773F02200E").
    fn known_folder(guid: &str) -> Option<String> {
        let hex = |r: std::ops::Range<usize>| guid.get(r).and_then(|s| u64::from_str_radix(s, 16).ok());
        let b = guid.as_bytes();
        if b.len() != 36 || b[8] != b'-' || b[13] != b'-' || b[18] != b'-' || b[23] != b'-' {
            return None;
        }
        let mut data4 = [0u8; 8];
        data4[0] = hex(19..21)? as u8;
        data4[1] = hex(21..23)? as u8;
        for (i, d) in data4[2..].iter_mut().enumerate() {
            *d = hex(24 + 2 * i..26 + 2 * i)? as u8;
        }
        let id = GUID { Data1: hex(0..8)? as u32, Data2: hex(9..13)? as u16, Data3: hex(14..18)? as u16, Data4: data4 };
        let mut p: PWSTR = null_mut();
        // SAFETY: valid GUID, `p` receives an owned string (freed even on failure).
        let hr = unsafe { SHGetKnownFolderPath(&id, 0, null_mut(), &mut p) };
        let s = take_string(p);
        if SUCCEEDED(hr) { s } else { None }
    }

    /// The file behind a desktop program's parsing name, when it is one.
    fn icon_file(parsing: &str) -> Option<String> {
        let path = if let Some((guid, tail)) = super::known_folder_path(parsing) {
            format!("{}\\{}", known_folder(guid)?, tail)
        } else if parsing.len() > 3 && parsing.as_bytes()[1] == b':' {
            parsing.to_string()
        } else {
            return None;
        };
        (super::is_program_file(&path) && std::path::Path::new(&path).is_file()).then_some(path)
    }

    /// Everything in `shell:AppsFolder` (not cleaned). Empty on failure.
    pub fn list() -> Vec<App> {
        // SAFETY: plain COM start-up on this thread; undone by ComInit.
        let hr = unsafe { CoInitializeEx(null_mut(), COINIT_APARTMENTTHREADED) };
        // RPC_E_CHANGED_MODE (already multithreaded) still lets us use COM.
        let _init = ComInit(hr == S_OK || hr == S_FALSE);
        let mut out = Vec::new();

        let mut folder: *mut c_void = null_mut();
        // SAFETY: out-pointer for an IShellItem owned by the guard below.
        let hr = unsafe {
            SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, null_mut(), &IShellItem::uuidof() as REFIID, &mut folder)
        };
        if !SUCCEEDED(hr) || folder.is_null() {
            return out;
        }
        let folder = Com(folder as *mut IShellItem);

        let mut en: *mut c_void = null_mut();
        // SAFETY: live IShellItem; out-pointer for an IEnumShellItems.
        let hr = unsafe { (*folder.0).BindToHandler(null_mut(), &BHID_ENUM_ITEMS, &IID_IENUM_SHELL_ITEMS, &mut en) };
        if !SUCCEEDED(hr) || en.is_null() {
            return out;
        }
        let en = Com(en as *mut IEnumShellItems);

        // Start lists a few hundred; the bound only guards against a broken enumerator.
        for _ in 0..20_000 {
            let mut item: *mut IShellItem = null_mut();
            let mut fetched: ULONG = 0;
            // SAFETY: live enumerator; asks for one item into `item`.
            let hr = unsafe { ((*(*en.0).vtbl).next)(en.0, 1, &mut item, &mut fetched) };
            if hr != S_OK || fetched == 0 || item.is_null() {
                break;
            }
            let item = Com(item);
            let (Some(name), Some(parsing)) =
                (display_name(item.0, SIGDN_NORMALDISPLAY), display_name(item.0, SIGDN_PARSINGNAME))
            else {
                continue;
            };
            if parsing.is_empty() {
                continue;
            }
            let icon = icon_file(&parsing);
            out.push(App::new(name, format!("shell:AppsFolder\\{parsing}"), icon));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, target: &str) -> App {
        App::new(name.into(), target.into(), None)
    }

    #[test]
    fn clean_drops_junk_and_duplicates() {
        let apps = vec![
            app("Google Chrome", r"shell:AppsFolder\Chrome"),
            app("Uninstall Foo", r"C:\Foo\unins000.exe"),
            app("Foo Remover", r"C:\Foo\unins001.exe"),
            app("Foo Help", r"C:\Foo\help.chm"),
            app("Foo Website", "https://foo.example"),
            app("Readme", r"C:\Foo\readme.txt"),
            app("google chrome ", r"C:\dup.lnk"),
            app("إلغاء تثبيت Foo", r"C:\x.lnk"),
            app("Get Help", r"shell:AppsFolder\Microsoft.GetHelp_8wekyb3d8bbwe!App"),
            app("إلغاء التثبيت", r"C:\x.lnk"),
            app("Calculator", r"shell:AppsFolder\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"),
            app("  ", r"C:\x.lnk"),
            app("Helper Tool", r"C:\h.lnk"),
        ];
        let names: Vec<String> = clean(apps).into_iter().map(|a| a.name).collect();
        assert_eq!(names, ["Calculator", "Get Help", "Google Chrome", "Helper Tool"]);
    }

    #[test]
    fn aliases_are_filled() {
        assert!(app("Google Chrome", "x").aliases.iter().any(|a| a == "كروم"));
    }

    #[test]
    fn parsing_names() {
        assert_eq!(
            known_folder_path(r"{6D809377-6AF0-444B-8957-A3773F02200E}\Google\Chrome\Application\chrome.exe"),
            Some(("6D809377-6AF0-444B-8957-A3773F02200E", r"Google\Chrome\Application\chrome.exe"))
        );
        assert_eq!(known_folder_path("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"), None);
        assert_eq!(known_folder_path("{bad}\\x"), None);
        assert!(is_program_file(r"C:\A\B.EXE"));
        assert!(!is_program_file(r"C:\A\readme.txt"));
    }

    #[test]
    fn walks_start_menu_shortcuts() {
        let dir = std::env::temp_dir().join(format!("bahth-startmenu-{}", std::process::id()));
        let _tmp = crate::test_temp::TempPath::at(dir.clone());
        let sub = dir.join("Vendor").join("Tools");
        std::fs::create_dir_all(&sub).unwrap();
        for f in [dir.join("Word.lnk"), sub.join("Foo.url"), sub.join("Bar.appref-ms"), sub.join("notes.txt")] {
            std::fs::write(f, b"").unwrap();
        }
        let mut names: Vec<String> = walk_start_menu(std::slice::from_ref(&dir)).into_iter().map(|a| a.name).collect();
        names.sort();
        assert_eq!(names, ["Bar", "Foo", "Word"]);
        let word = walk_start_menu(std::slice::from_ref(&dir)).into_iter().find(|a| a.name == "Word").unwrap();
        assert!(word.aliases.iter().any(|a| a == "وورد"));
        assert_eq!(word.icon_path.as_deref(), Some(word.target.as_str()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(walk_start_menu(&[dir]).is_empty());
    }
}
