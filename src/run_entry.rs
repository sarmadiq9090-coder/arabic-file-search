//! Which exe the "start with Windows" entry (`ui/autostart.rs`) starts.
//! Platform-independent and unit-tested.
//!
//! After the one-time move to `%LOCALAPPDATA%\Programs\InstaFind`
//! (`relocate`), the old copy in Downloads is never deleted. Started again
//! from an old pin or shortcut, it must not point the entry back at itself:
//! a copy in Downloads or a temp folder keeps an entry that starts another
//! copy, as long as that copy still exists and is not in such a folder too.

use crate::relocate::{Folders, Place, classify, normalize};

/// The exe of a `Run` entry's command: `"C:\x\bahth.exe" --startup` gives
/// `C:\x\bahth.exe`; an unquoted command gives its first word. `None`
/// unless that is a full path (`C:\…` or `\\server\…`): a bare name would
/// be looked up relative to the current folder.
pub fn exe_of(command: &str) -> Option<&str> {
    let c = command.trim();
    let exe = match c.strip_prefix('"') {
        Some(rest) => rest.split('"').next()?,
        None => c.split_whitespace().next()?,
    };
    let b = exe.as_bytes();
    let drive = b.len() > 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/');
    (drive || exe.starts_with("\\\\")).then_some(exe)
}

/// The exe the entry should start, when `current` (this exe) turns it on or
/// refreshes it and the entry now holds `existing` (`None`: no entry).
/// `is_file` says whether a path is an existing file.
pub fn target<'a>(current: &'a str, existing: Option<&'a str>, folders: &Folders, is_file: &dyn Fn(&str) -> bool) -> &'a str {
    let transient = |p: &str| matches!(classify(p, folders), Place::Downloads | Place::Temp);
    match existing.and_then(exe_of) {
        Some(other)
            if transient(current) && !transient(other) && normalize(other) != normalize(current) && is_file(other) =>
        {
            other
        }
        _ => current,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOWNLOADS: &str = "C:\\Users\\Sara\\Downloads\\bahth.exe";
    const TEMP: &str = "C:\\Users\\Sara\\AppData\\Local\\Temp\\Temp1_bahth.zip\\bahth.exe";
    const INSTALLED: &str = "C:\\Users\\Sara\\AppData\\Local\\Programs\\InstaFind\\bahth.exe";
    const ELSEWHERE: &str = "D:\\Tools\\bahth.exe";

    fn folders() -> Folders {
        let env = |name: &str| {
            let v = match name {
                "LOCALAPPDATA" => "C:\\Users\\Sara\\AppData\\Local",
                "USERPROFILE" => "C:\\Users\\Sara",
                "TEMP" | "TMP" => "C:\\Users\\Sara\\AppData\\Local\\Temp",
                "SystemRoot" => "C:\\WINDOWS",
                "ProgramFiles" => "C:\\Program Files",
                _ => return None,
            };
            Some(v.to_string())
        };
        Folders::from_env(&env, Some("C:\\Users\\Sara\\Downloads".into()))
    }

    fn entry(exe: &str) -> String {
        format!("\"{exe}\" --startup")
    }

    /// `target` with every path existing except `missing`.
    fn pick<'a>(current: &'a str, existing: Option<&'a str>, missing: &str) -> &'a str {
        target(current, existing, &folders(), &|p| !p.eq_ignore_ascii_case(missing))
    }

    #[test]
    fn the_exe_of_a_command() {
        assert_eq!(exe_of(&entry(INSTALLED)), Some(INSTALLED));
        assert_eq!(exe_of("\"C:\\My Apps\\bahth.exe\" --startup"), Some("C:\\My Apps\\bahth.exe"));
        assert_eq!(exe_of("  C:\\x\\bahth.exe --startup"), Some("C:\\x\\bahth.exe"));
        assert_eq!(exe_of("C:\\x\\bahth.exe"), Some("C:\\x\\bahth.exe"));
        assert_eq!(exe_of("\"\\\\pc\\share\\bahth.exe\" --startup"), Some("\\\\pc\\share\\bahth.exe"));
        for bad in ["", "   ", "\"\" --startup", "\"", "bahth.exe --startup", "\"bahth.exe\" --startup", "C:bahth.exe"] {
            assert_eq!(exe_of(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn no_entry_starts_this_copy() {
        for current in [DOWNLOADS, TEMP, INSTALLED, ELSEWHERE] {
            assert_eq!(pick(current, None, ""), current);
        }
    }

    #[test]
    fn a_downloads_copy_keeps_the_installed_one() {
        let e = entry(INSTALLED);
        assert_eq!(pick(DOWNLOADS, Some(&e), ""), INSTALLED);
        assert_eq!(pick(TEMP, Some(&e), ""), INSTALLED);
        // A copy the user put somewhere else of their own is kept as well.
        let e = entry(ELSEWHERE);
        assert_eq!(pick(DOWNLOADS, Some(&e), ""), ELSEWHERE);
    }

    #[test]
    fn an_entry_to_a_missing_file_is_replaced() {
        let e = entry(INSTALLED);
        assert_eq!(pick(DOWNLOADS, Some(&e), INSTALLED), DOWNLOADS);
    }

    #[test]
    fn an_installed_copy_takes_the_entry() {
        // The moved copy (or a winget/Scoop one) always points it at itself,
        // also away from an old Downloads copy that still exists.
        let e = entry(DOWNLOADS);
        assert_eq!(pick(INSTALLED, Some(&e), ""), INSTALLED);
        assert_eq!(pick(ELSEWHERE, Some(&e), ""), ELSEWHERE);
        let e = entry(ELSEWHERE);
        assert_eq!(pick(INSTALLED, Some(&e), ""), INSTALLED);
    }

    #[test]
    fn another_downloads_or_temp_copy_is_not_kept() {
        let e = entry("C:\\Users\\Sara\\Downloads\\bahth (1).exe");
        assert_eq!(pick(DOWNLOADS, Some(&e), ""), DOWNLOADS);
        let e = entry(TEMP);
        assert_eq!(pick(DOWNLOADS, Some(&e), ""), DOWNLOADS);
    }

    #[test]
    fn the_same_copy_in_another_spelling_is_this_one() {
        let e = entry("c:\\users\\sara\\downloads\\BAHTH.EXE");
        assert_eq!(pick(DOWNLOADS, Some(&e), ""), DOWNLOADS);
        assert_eq!(pick(DOWNLOADS, Some("garbage \""), ""), DOWNLOADS);
    }
}
