//! Which exe the "start with Windows" entry (`ui/autostart.rs`) starts.
//! Platform-independent and unit-tested.
//!
//! After the one-time move to `%LOCALAPPDATA%\Programs\InstaFind`
//! (`relocate`), the old copy in Downloads is never deleted. Started again
//! from an old pin or shortcut, it must not point the entry back at itself:
//! a copy in Downloads or a temp folder keeps an entry that starts another
//! copy, as long as that copy still exists and is not in such a folder too.
//! Likewise a copy outside the installed places keeps an entry that starts
//! an installed copy, unless it replaced that copy or the user turned the
//! entry on in it (see [`target`]).

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

/// Why the entry is being written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// The refresh at every start (`apply_default`).
    Refresh,
    /// This copy has just replaced the one that ran: an update, the move to
    /// the permanent place, or a newer copy that took an older one's place.
    TookOver,
    /// The user turned starting with Windows on in this copy.
    User,
}

/// The exe the entry should start, when `current` (this exe) turns it on or
/// refreshes it and the entry now holds `existing` (`None`: no entry).
/// `is_file` says whether a path is an existing file.
///
/// The entry stays on another copy that still exists:
/// - from a copy in Downloads or a temp folder, always (unless that other
///   copy is in such a folder too);
/// - from any copy outside the installed places (`Place::Installed`: the
///   permanent place, under its new or old name, winget, Scoop, Chocolatey,
///   Program Files) when the entry starts an installed copy, unless this
///   copy replaced it ([`Why::TookOver`]) or the user turned the entry on
///   here ([`Why::User`]). A portable copy run once from a folder of its
///   own (seen on a real PC: `C:\BahthTest\rc4\bahth-x86.exe`) used to
///   take the entry from `%LOCALAPPDATA%\Programs\Bahth\bahth.exe`.
pub fn target<'a>(current: &'a str, existing: Option<&'a str>, folders: &Folders, is_file: &dyn Fn(&str) -> bool, why: Why) -> &'a str {
    let place = |p: &str| classify(p, folders);
    let transient = |p: &str| matches!(place(p), Place::Downloads | Place::Temp);
    let Some(other) = existing.and_then(exe_of) else { return current };
    if normalize(other) == normalize(current) || !is_file(other) {
        return current;
    }
    let keep = (transient(current) && !transient(other))
        || (why == Why::Refresh && place(other) == Place::Installed && place(current) != Place::Installed);
    if keep { other } else { current }
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

    /// `target` at a start's refresh, with every path existing except `missing`.
    fn pick<'a>(current: &'a str, existing: Option<&'a str>, missing: &str) -> &'a str {
        pick_for(current, existing, missing, Why::Refresh)
    }

    fn pick_for<'a>(current: &'a str, existing: Option<&'a str>, missing: &str, why: Why) -> &'a str {
        target(current, existing, &folders(), &|p| !p.eq_ignore_ascii_case(missing), why)
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
        // From one installed place to another (a winget copy after the moved one).
        let winget = "C:\\Users\\Sara\\AppData\\Local\\Microsoft\\WinGet\\Packages\\Sarmad.InstaFind_x\\bahth.exe";
        assert_eq!(pick(winget, Some(&entry(INSTALLED)), ""), winget);
    }

    /// The second real PC (FIN1): a portable copy run once from a test
    /// folder rewrote the entry from the installed copy, which lives in the
    /// permanent place under its older name, `Programs\Bahth`.
    #[test]
    fn a_portable_copy_run_once_leaves_the_installed_entry() {
        const OLD_NAME: &str = "C:\\Users\\Sara\\AppData\\Local\\Programs\\Bahth\\bahth.exe";
        const PORTABLE: &str = "C:\\BahthTest\\rc4\\bahth-x86.exe";
        assert_eq!(classify(OLD_NAME, &folders()), Place::Installed);
        for installed in [OLD_NAME, INSTALLED] {
            let e = entry(installed);
            for current in [PORTABLE, ELSEWHERE, DOWNLOADS, TEMP] {
                assert_eq!(pick(current, Some(&e), ""), installed, "{current} at a start");
            }
            // Unless that copy replaced the installed one, or the user
            // turned the entry on in it (not from Downloads or temp).
            for why in [Why::TookOver, Why::User] {
                assert_eq!(pick_for(PORTABLE, Some(&e), "", why), PORTABLE, "{why:?}");
                assert_eq!(pick_for(DOWNLOADS, Some(&e), "", why), installed, "{why:?}");
            }
            // The installed copy is gone: the entry follows the one that runs.
            assert_eq!(pick(PORTABLE, Some(&e), installed), PORTABLE);
        }
        // Two copies of the user's own: the one that runs (as before; the
        // upgrade test moved C:\BahthTest\old to C:\BahthTest\dl).
        assert_eq!(pick("C:\\BahthTest\\dl\\bahth.exe", Some(&entry("C:\\BahthTest\\old\\bahth.exe")), ""), "C:\\BahthTest\\dl\\bahth.exe");
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
