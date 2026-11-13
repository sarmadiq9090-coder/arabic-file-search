//! The first-run offer to move a portable copy that runs from Downloads or a
//! temp folder to a permanent place: which kind of folder the exe is in, and
//! where it goes. Platform-independent and unit-tested; the copying, the
//! Start-menu shortcut and the restart are in `ui/relocate.rs`.

/// Where a moved copy lives, under `%LOCALAPPDATA%` (the per-user
/// "Programs" folder Windows itself uses; no admin rights needed).
pub const INSTALL_SUBDIR: &str = "Programs\\InstaFind";

/// The permanent place under the program's older name, still found on
/// some PCs: a copy there counts as installed too.
pub const OLD_INSTALL_SUBDIR: &str = "Programs\\Bahth";

/// The file name of a moved copy: the same name winget, Scoop and the
/// updater use, whatever the downloaded file was called ("bahth (1).exe").
pub const EXE_NAME: &str = "bahth.exe";

/// Command-line flag of the copy started from the new place: it waits for
/// the old one to exit before taking the single-instance lock.
pub const MOVED_ARG: &str = "--moved";

/// The kind of folder the program runs from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// The Downloads folder (or a folder in it, e.g. an unpacked zip).
    Downloads,
    /// A temp folder: started from inside a zip, or from a browser's "Open".
    Temp,
    /// Installed by winget, Scoop or Chocolatey, in Program Files, or
    /// already moved: never offered.
    Installed,
    /// Any other folder the user chose: left alone.
    Elsewhere,
}

impl Place {
    /// Whether to offer the move.
    pub fn offer_move(self) -> bool {
        matches!(self, Place::Downloads | Place::Temp)
    }
}

/// The folders [`classify`] compares with, each in [`normalize`]d form.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Folders {
    pub downloads: Vec<String>,
    pub temp: Vec<String>,
    pub installed: Vec<String>,
}

impl Folders {
    /// From environment variables (`env("LOCALAPPDATA")` …) and the
    /// Downloads folder as Windows reports it (`known_downloads`, which the
    /// user may have moved to another drive).
    pub fn from_env(env: &dyn Fn(&str) -> Option<String>, known_downloads: Option<String>) -> Folders {
        let var = |name: &str| env(name).filter(|v| !v.trim().is_empty());
        let under = |base: Option<String>, tail: &str| base.map(|b| format!("{}\\{tail}", b.trim_end_matches(['\\', '/'])));
        let local = var("LOCALAPPDATA");
        let profile = var("USERPROFILE");
        let program_data = var("ProgramData").or_else(|| var("ALLUSERSPROFILE"));
        let windows = var("SystemRoot").or_else(|| var("windir"));

        let downloads = [known_downloads, under(profile.clone(), "Downloads")];
        let temp = [
            var("TEMP"),
            var("TMP"),
            under(local.clone(), "Temp"),
            under(windows, "Temp"),
            // Where Internet Explorer and some mail programs put files opened straight away.
            under(local.clone(), "Microsoft\\Windows\\INetCache"),
        ];
        let installed = [
            local.as_deref().map(install_dir),
            // The permanent place under its older name (copies moved or
            // unpacked there by hand before the program was InstaFind).
            under(local.clone(), OLD_INSTALL_SUBDIR),
            under(local.clone(), "Microsoft\\WinGet"),
            under(local.clone(), "Microsoft\\WindowsApps"),
            under(profile.clone(), "scoop"),
            var("SCOOP"),
            var("SCOOP_GLOBAL"),
            under(program_data.clone(), "scoop"),
            var("ChocolateyInstall"),
            under(program_data, "chocolatey"),
            var("ProgramFiles"),
            var("ProgramFiles(x86)"),
            var("ProgramW6432"),
        ];
        let profile = profile.map(|p| normalize(&p));
        // A drive root or the profile folder itself (a TEMP of "C:\", say)
        // would make every path match: such entries are dropped.
        let keep = |list: &[Option<String>]| -> Vec<String> {
            let mut out: Vec<String> = Vec::new();
            for f in list.iter().flatten().map(|f| normalize(f)) {
                if is_specific(&f) && Some(&f) != profile.as_ref() && !out.contains(&f) {
                    out.push(f);
                }
            }
            out
        };
        Folders { downloads: keep(&downloads), temp: keep(&temp), installed: keep(&installed) }
    }
}

/// `%LOCALAPPDATA%\Programs\InstaFind`.
pub fn install_dir(local_app_data: &str) -> String {
    format!("{}\\{INSTALL_SUBDIR}", local_app_data.trim_end_matches(['\\', '/']))
}

/// A path for comparing: backslashes, no `\\?\` prefix, lower case, and a
/// backslash at the end (so `C:\Temp` does not contain `C:\Temperature`).
pub fn normalize(path: &str) -> String {
    let mut p = path.trim().replace('/', "\\");
    if let Some(rest) = p.strip_prefix("\\\\?\\UNC\\") {
        p = format!("\\\\{rest}");
    } else if let Some(rest) = p.strip_prefix("\\\\?\\") {
        p = rest.to_string();
    }
    let mut p = p.to_lowercase();
    if !p.ends_with('\\') {
        p.push('\\');
    }
    p
}

/// Not empty and deeper than a drive root (`c:\`) or a share (`\\pc\share\`).
fn is_specific(normalized: &str) -> bool {
    let parts = normalized.split('\\').filter(|s| !s.is_empty()).count();
    if normalized.starts_with("\\\\") { parts > 2 } else { parts > 1 }
}

/// Which kind of folder the exe at `exe` runs from. Installed places win, so
/// a Scoop folder set up inside Downloads is still left alone.
pub fn classify(exe: &str, folders: &Folders) -> Place {
    let dir = match exe.trim().replace('/', "\\").rsplit_once('\\') {
        Some((dir, _)) => normalize(dir),
        None => return Place::Elsewhere,
    };
    let inside = |list: &[String]| list.iter().any(|f| dir.starts_with(f.as_str()));
    if inside(&folders.installed) {
        Place::Installed
    } else if inside(&folders.downloads) {
        Place::Downloads
    } else if inside(&folders.temp) {
        Place::Temp
    } else {
        Place::Elsewhere
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A typical Windows 11 user.
    fn env(name: &str) -> Option<String> {
        let v = match name {
            "LOCALAPPDATA" => "C:\\Users\\Sara\\AppData\\Local",
            "USERPROFILE" => "C:\\Users\\Sara",
            "TEMP" | "TMP" => "C:\\Users\\Sara\\AppData\\Local\\Temp",
            "ProgramData" => "C:\\ProgramData",
            "SystemRoot" => "C:\\WINDOWS",
            "ProgramFiles" | "ProgramW6432" => "C:\\Program Files",
            "ProgramFiles(x86)" => "C:\\Program Files (x86)",
            _ => return None,
        };
        Some(v.to_string())
    }

    fn folders() -> Folders {
        Folders::from_env(&env, Some("C:\\Users\\Sara\\Downloads".into()))
    }

    fn place(exe: &str) -> Place {
        classify(exe, &folders())
    }

    #[test]
    fn downloads_are_offered() {
        assert_eq!(place("C:\\Users\\Sara\\Downloads\\bahth.exe"), Place::Downloads);
        // Any case, a renamed download, an unpacked zip in Downloads.
        assert_eq!(place("c:\\users\\SARA\\downloads\\bahth (1).exe"), Place::Downloads);
        assert_eq!(place("C:\\Users\\Sara\\Downloads\\bahth-x86\\bahth-x86.exe"), Place::Downloads);
        assert_eq!(place("C:/Users/Sara/Downloads/bahth.exe"), Place::Downloads);
        assert_eq!(place("\\\\?\\C:\\Users\\Sara\\Downloads\\bahth.exe"), Place::Downloads);
        assert!(Place::Downloads.offer_move());
    }

    #[test]
    fn a_moved_downloads_folder_counts() {
        // Downloads moved to D: in its Properties ▸ Location.
        let f = Folders::from_env(&env, Some("D:\\تنزيلات".into()));
        assert_eq!(classify("D:\\تنزيلات\\bahth.exe", &f), Place::Downloads);
        // The default place still counts too.
        assert_eq!(classify("C:\\Users\\Sara\\Downloads\\bahth.exe", &f), Place::Downloads);
    }

    #[test]
    fn temp_folders_are_offered() {
        // Opened from inside a zip in Explorer.
        assert_eq!(place("C:\\Users\\Sara\\AppData\\Local\\Temp\\Temp1_bahth.zip\\bahth.exe"), Place::Temp);
        assert_eq!(place("C:\\Windows\\Temp\\bahth.exe"), Place::Temp);
        assert_eq!(place("C:\\Users\\Sara\\AppData\\Local\\Microsoft\\Windows\\INetCache\\IE\\X1\\bahth.exe"), Place::Temp);
        assert!(Place::Temp.offer_move());
    }

    #[test]
    fn package_managers_and_program_files_are_left_alone() {
        for exe in [
            "C:\\Users\\Sara\\AppData\\Local\\Microsoft\\WinGet\\Packages\\Sarmad.InstaFind_Microsoft.Winget.Source_8wekyb3d8bbwe\\bahth.exe",
            "C:\\Users\\Sara\\AppData\\Local\\Microsoft\\WinGet\\Links\\bahth.exe",
            "C:\\Users\\Sara\\scoop\\apps\\instafind\\current\\bahth.exe",
            "C:\\Program Files\\InstaFind\\bahth.exe",
            "C:\\Program Files (x86)\\InstaFind\\bahth.exe",
            "C:\\ProgramData\\chocolatey\\lib\\instafind\\tools\\bahth.exe",
            "C:\\ProgramData\\scoop\\apps\\instafind\\current\\bahth.exe",
        ] {
            assert_eq!(place(exe), Place::Installed, "{exe}");
            assert!(!place(exe).offer_move());
        }
    }

    #[test]
    fn a_moved_copy_is_not_offered_again() {
        let target = format!("{}\\{EXE_NAME}", install_dir("C:\\Users\\Sara\\AppData\\Local"));
        assert_eq!(target, "C:\\Users\\Sara\\AppData\\Local\\Programs\\InstaFind\\bahth.exe");
        assert_eq!(place(&target), Place::Installed);
    }

    #[test]
    fn the_restart_flag_is_its_own() {
        // Not the updater's flag nor the sign-in one: each waits differently.
        assert!(MOVED_ARG.starts_with("--"));
        assert_ne!(MOVED_ARG, "--updated");
        assert_ne!(MOVED_ARG, "--startup");
    }

    #[test]
    fn a_custom_scoop_root_wins_over_downloads() {
        let env2 = |n: &str| if n == "SCOOP" { Some("C:\\Users\\Sara\\Downloads\\scoop".to_string()) } else { env(n) };
        let f = Folders::from_env(&env2, None);
        assert_eq!(classify("C:\\Users\\Sara\\Downloads\\scoop\\apps\\instafind\\current\\bahth.exe", &f), Place::Installed);
        assert_eq!(classify("C:\\Users\\Sara\\Downloads\\bahth.exe", &f), Place::Downloads);
    }

    #[test]
    fn other_folders_are_left_alone() {
        for exe in [
            "C:\\Users\\Sara\\Desktop\\bahth.exe",
            "C:\\Users\\Sara\\Documents\\Tools\\bahth.exe",
            "D:\\Portable\\bahth.exe",
            "E:\\bahth.exe",
            // Look-alike names are not the folders themselves.
            "C:\\Users\\Sara\\Downloads2\\bahth.exe",
            "C:\\Users\\Sara\\AppData\\Local\\Temporary\\bahth.exe",
            "C:\\Program Files Portable\\bahth.exe",
            "bahth.exe",
        ] {
            assert_eq!(place(exe), Place::Elsewhere, "{exe}");
        }
    }

    #[test]
    fn a_root_or_profile_temp_does_not_match_everything() {
        let env2 = |n: &str| match n {
            "TEMP" => Some("C:\\".to_string()),
            "TMP" => Some("C:\\Users\\Sara".to_string()),
            _ => env(n),
        };
        let f = Folders::from_env(&env2, None);
        assert!(!f.temp.contains(&"c:\\".to_string()));
        assert!(!f.temp.contains(&"c:\\users\\sara\\".to_string()));
        assert_eq!(classify("C:\\Users\\Sara\\Desktop\\bahth.exe", &f), Place::Elsewhere);
        assert_eq!(classify("D:\\bahth.exe", &f), Place::Elsewhere);
    }

    #[test]
    fn missing_variables_give_no_folders() {
        let f = Folders::from_env(&|_| None, None);
        assert_eq!(f, Folders::default());
        assert_eq!(classify("C:\\Users\\Sara\\Downloads\\bahth.exe", &f), Place::Elsewhere);
        // Empty values are ignored too.
        let f = Folders::from_env(&|_| Some(String::new()), Some("  ".into()));
        assert_eq!(f, Folders::default());
    }

    #[test]
    fn normalize_handles_prefixes_and_separators() {
        assert_eq!(normalize("C:\\Users\\Sara\\Downloads"), "c:\\users\\sara\\downloads\\");
        assert_eq!(normalize("C:/Users/Sara/Downloads/"), "c:\\users\\sara\\downloads\\");
        assert_eq!(normalize("\\\\?\\C:\\X"), "c:\\x\\");
        assert_eq!(normalize("\\\\?\\UNC\\server\\share\\x"), "\\\\server\\share\\x\\");
        assert!(!is_specific("c:\\"));
        assert!(!is_specific("\\\\server\\share\\"));
        assert!(is_specific("\\\\server\\share\\dl\\"));
        assert!(is_specific("d:\\تنزيلات\\"));
    }
}
