//! User preferences kept between runs in a small `key=value` text file next to
//! the index cache.

// Only the Windows UI and the tests use this module.

use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Window size in logical pixels.
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
    /// Result-list column widths, in the order the columns are created.
    pub columns: Vec<i32>,
    /// Show numbers with Arabic-Indic digits (٠١٢٣) instead of 0123.
    pub arabic_digits: bool,
    /// Selected entry of the file-type filter.
    pub kind: usize,
    /// Column the results are sorted by, or -1 for best match first.
    pub sort_col: i32,
    pub sort_desc: bool,
    /// When the program last looked for a new version (seconds since 1970).
    pub update_check: u64,
    /// Keep a persistent index of document text, filled in the background.
    pub content_index: bool,
    /// Keep the index current by watching the drives for changes.
    pub auto_index: bool,
    /// Leave Windows system files (the Windows folder, temp and cache
    /// folders…) out of search results.
    pub hide_system: bool,
    /// Time of the newest crash report the user was already asked about
    /// (seconds since 1970), so each report is offered once.
    pub crash_seen: u64,
    /// The small search box above the taskbar clock (see `crate::quick`).
    pub floating_mode: FloatingMode,
    /// Where the user dragged the box, or `None` for the default place.
    pub floating_pos: Option<FloatPos>,
    /// What the global Ctrl+Alt+F hotkey opens.
    pub hotkey_opens: HotkeyOpens,
    /// Alt+Space opens the quick search too (off by default).
    pub alt_space: bool,
    /// Show the icon in the notification area.
    pub tray_icon: bool,
    /// The user was asked once whether they want the floating box.
    pub floating_asked: bool,
}

/// How the floating search box above the clock shows itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FloatingMode {
    /// Almost invisible until the cursor comes near, then fades in.
    #[default]
    Fade,
    /// Fully invisible until the cursor reaches its corner.
    Hidden,
    /// No box at all.
    Off,
}

impl FloatingMode {
    pub fn code(self) -> &'static str {
        match self {
            FloatingMode::Fade => "fade",
            FloatingMode::Hidden => "hidden",
            FloatingMode::Off => "off",
        }
    }

    pub fn from_code(code: &str) -> Option<FloatingMode> {
        match code.trim().to_ascii_lowercase().as_str() {
            "fade" => Some(FloatingMode::Fade),
            "hidden" => Some(FloatingMode::Hidden),
            "off" => Some(FloatingMode::Off),
            _ => None,
        }
    }
}

/// What the global hotkey opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HotkeyOpens {
    /// The small quick-search popup.
    #[default]
    Quick,
    /// The full window, as before 0.8.
    Main,
}

impl HotkeyOpens {
    pub fn code(self) -> &'static str {
        match self {
            HotkeyOpens::Quick => "quick",
            HotkeyOpens::Main => "main",
        }
    }

    pub fn from_code(code: &str) -> Option<HotkeyOpens> {
        match code.trim().to_ascii_lowercase().as_str() {
            "quick" => Some(HotkeyOpens::Quick),
            "main" => Some(HotkeyOpens::Main),
            _ => None,
        }
    }
}

/// A position the floating box was dragged to: its top-left corner in
/// screen pixels, and the monitor it was on (that monitor's rectangle), so
/// a different screen setup falls back to the default place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FloatPos {
    pub monitor: [i32; 4],
    pub x: i32,
    pub y: i32,
}

impl FloatPos {
    /// `left,top,right,bottom,x,y`.
    pub fn to_text(self) -> String {
        let [l, t, r, b] = self.monitor;
        format!("{l},{t},{r},{b},{},{}", self.x, self.y)
    }

    /// The text of [`FloatPos::to_text`]; `None` when malformed or when the
    /// monitor rectangle is empty.
    pub fn parse(text: &str) -> Option<FloatPos> {
        let v: Vec<i32> = text.split(',').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
        let [l, t, r, b, x, y] = v[..] else { return None };
        (r > l && b > t).then_some(FloatPos { monitor: [l, t, r, b], x, y })
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings { width: 1100, height: 680, maximized: false, columns: Vec::new(), arabic_digits: false, kind: 0, sort_col: -1, sort_desc: false, update_check: 0, content_index: true, auto_index: true, hide_system: true, crash_seen: 0, floating_mode: FloatingMode::Fade, floating_pos: None, hotkey_opens: HotkeyOpens::Quick, alt_space: false, tray_icon: true, floating_asked: false }
    }
}

impl Settings {
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = value.trim();
            match key.trim() {
                "width" => s.width = value.parse().unwrap_or(s.width).clamp(400, 10_000),
                "height" => s.height = value.parse().unwrap_or(s.height).clamp(300, 10_000),
                "maximized" => s.maximized = value == "1",
                "columns" => s.columns = value.split(',').filter_map(|w| w.trim().parse().ok()).collect(),
                "arabic_digits" => s.arabic_digits = value == "1",
                "kind" => s.kind = value.parse().unwrap_or(0),
                "sort_col" => s.sort_col = value.parse().unwrap_or(-1).clamp(-1, 3),
                "sort_desc" => s.sort_desc = value == "1",
                "update_check" => s.update_check = value.parse().unwrap_or(0),
                "content_index" => s.content_index = value != "0",
                "auto_index" => s.auto_index = value != "0",
                "hide_system" => s.hide_system = value != "0",
                "crash_seen" => s.crash_seen = value.parse().unwrap_or(0),
                "floating_mode" => s.floating_mode = FloatingMode::from_code(value).unwrap_or_default(),
                "floating_pos" => s.floating_pos = FloatPos::parse(value),
                "hotkey_opens" => s.hotkey_opens = HotkeyOpens::from_code(value).unwrap_or_default(),
                "alt_space" => s.alt_space = value == "1",
                "tray_icon" => s.tray_icon = value != "0",
                "floating_asked" => s.floating_asked = value == "1",
                _ => {}
            }
        }
        s
    }

    pub fn to_text(&self) -> String {
        let cols: Vec<String> = self.columns.iter().map(i32::to_string).collect();
        format!(
            "width={}\nheight={}\nmaximized={}\ncolumns={}\narabic_digits={}\nkind={}\nsort_col={}\nsort_desc={}\nupdate_check={}\ncontent_index={}\nauto_index={}\nhide_system={}\ncrash_seen={}\nfloating_mode={}\nfloating_pos={}\nhotkey_opens={}\nalt_space={}\ntray_icon={}\nfloating_asked={}\n",
            self.width,
            self.height,
            self.maximized as u8,
            cols.join(","),
            self.arabic_digits as u8,
            self.kind,
            self.sort_col,
            self.sort_desc as u8,
            self.update_check,
            self.content_index as u8,
            self.auto_index as u8,
            self.hide_system as u8,
            self.crash_seen,
            self.floating_mode.code(),
            self.floating_pos.map(FloatPos::to_text).unwrap_or_default(),
            self.hotkey_opens.code(),
            self.alt_space as u8,
            self.tray_icon as u8,
            self.floating_asked as u8
        )
    }

    /// Reads `path`; a missing file gives the defaults.
    pub fn load_from(path: &Path) -> Settings {
        std::fs::read_to_string(path).map(|t| Settings::parse(&t)).unwrap_or_default()
    }

    /// Writes a temp file and renames it over `path`, so a crash never
    /// leaves half a settings file.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("txt.tmp");
        std::fs::write(&tmp, self.to_text())?;
        std::fs::rename(tmp, path)
    }

    /// `%LOCALAPPDATA%\BahthSari\settings.txt`.
    #[cfg(windows)]
    fn path() -> std::path::PathBuf {
        crate::index::cache_path().with_file_name("settings.txt")
    }

    #[cfg(windows)]
    pub fn load() -> Settings {
        Self::load_from(&Self::path())
    }

    /// Saves to the default location, ignoring errors (settings are a convenience).
    #[cfg(windows)]
    pub fn save(&self) {
        let _ = self.save_to(&Self::path());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let s = Settings { width: 900, height: 500, maximized: true, columns: vec![200, 300, 80], arabic_digits: true, kind: 3, sort_col: 2, sort_desc: true, update_check: 1_700_000_000, content_index: false, auto_index: false, hide_system: false, crash_seen: 1_800_000_000, floating_mode: FloatingMode::Hidden, floating_pos: Some(FloatPos { monitor: [-1920, 0, 0, 1080], x: -500, y: 990 }), hotkey_opens: HotkeyOpens::Main, alt_space: true, tray_icon: false, floating_asked: true };
        assert_eq!(Settings::parse(&s.to_text()), s);
        for mode in [FloatingMode::Fade, FloatingMode::Hidden, FloatingMode::Off] {
            let s = Settings { floating_mode: mode, ..Settings::default() };
            assert_eq!(Settings::parse(&s.to_text()), s);
        }
        assert_eq!(Settings::parse("garbage\nwidth=abc"), Settings::default());
    }

    #[test]
    fn bad_values_fall_back_or_clamp() {
        let d = Settings::default();
        let s = Settings::parse("width=50\nheight=99999\nsort_col=7\nkind=abc\nupdate_check=-1\nmaximized=yes");
        assert_eq!((s.width, s.height, s.sort_col, s.kind, s.update_check, s.maximized), (400, 10_000, 3, 0, 0, false));
        let s = Settings::parse("width=-5\nheight=\nsort_col=-9\nkind=-1\nsort_desc=true\narabic_digits=0");
        assert_eq!((s.width, s.height, s.sort_col, s.kind, s.sort_desc, s.arabic_digits), (d.width, d.height, -1, 0, false, false));
        // Unparsable column widths are dropped, the rest kept in order.
        assert_eq!(Settings::parse("columns=100,x,, 200 ,-3,9999999999").columns, vec![100, 200, -3]);
        assert!(Settings::parse("columns=").columns.is_empty());
        // `=` inside a value makes it unparsable; the last duplicate key wins.
        assert_eq!(Settings::parse("width=900=1").width, d.width);
        assert_eq!(Settings::parse("width=900\nwidth=1000").width, 1000);
        // The content index is on unless switched off.
        assert!(Settings::parse("content_index=yes").content_index && !Settings::parse("content_index=0").content_index);
        // Automatic indexing is on unless explicitly turned off.
        assert!(d.auto_index && Settings::parse("auto_index=yes").auto_index && !Settings::parse("auto_index=0").auto_index);
        // System files are hidden unless explicitly shown.
        assert!(d.hide_system && Settings::parse("hide_system=yes").hide_system && !Settings::parse("hide_system=0").hide_system);
    }

    #[test]
    fn quick_search_settings() {
        let d = Settings::default();
        // 0.8 defaults: a fading box, the hotkey opens the quick search, a tray icon, Alt+Space off.
        assert_eq!((d.floating_mode, d.hotkey_opens, d.alt_space, d.tray_icon, d.floating_asked), (FloatingMode::Fade, HotkeyOpens::Quick, false, true, false));
        assert_eq!(d.floating_pos, None);
        // Settings files from before 0.8 get the defaults.
        assert_eq!(Settings::parse("width=900\nhide_system=1\n"), Settings { width: 900, ..d.clone() });
        // Unknown modes fall back to the defaults; case and spaces are ignored.
        let s = Settings::parse("floating_mode=blink\nhotkey_opens=both\ntray_icon=yes\nalt_space=yes");
        assert_eq!((s.floating_mode, s.hotkey_opens, s.tray_icon, s.alt_space), (FloatingMode::Fade, HotkeyOpens::Quick, true, false));
        let s = Settings::parse("floating_mode= OFF \nhotkey_opens=Main\ntray_icon=0\nfloating_asked=1");
        assert_eq!((s.floating_mode, s.hotkey_opens, s.tray_icon, s.floating_asked), (FloatingMode::Off, HotkeyOpens::Main, false, true));
        // Positions: six numbers and a real monitor rectangle, else the default place.
        let p = FloatPos { monitor: [0, 0, 2560, 1440], x: 1800, y: 1360 };
        assert_eq!(FloatPos::parse(&p.to_text()), Some(p));
        assert_eq!(FloatPos::parse(" 0, 0 ,2560,1440, 1800 ,1360"), Some(p));
        for bad in ["", "1,2,3", "0,0,10,10,1,2,3", "0,0,0,10,1,1", "0,0,10,-5,1,1", "a,b,c,d,e,f", "0,0,10,10,1,99999999999"] {
            assert_eq!(FloatPos::parse(bad), None, "{bad:?}");
        }
        assert_eq!(Settings::parse("floating_pos=").floating_pos, None);
        assert_eq!(Settings::parse("floating_pos=0,0,1920,1080,10,20").floating_pos, Some(FloatPos { monitor: [0, 0, 1920, 1080], x: 10, y: 20 }));
    }

    #[test]
    fn tolerant_syntax() {
        let s = Settings::parse("\u{FEFF}junk\r\n  width = 900 \r\nmaximized=1\r\n[section]\r\nunknown=5\r\n=1\r\n");
        assert_eq!((s.width, s.maximized), (900, true));
        assert_eq!(Settings { width: 900, maximized: true, ..Settings::default() }, s);
        assert_eq!(Settings::parse(""), Settings::default());
    }

    /// Random text: never a panic, every value stays in its range, and what
    /// is parsed survives a save and a reload unchanged.
    #[test]
    fn random_text_parses_into_valid_settings() {
        const PIECES: &[&str] = &[
            "width", "height", "maximized", "columns", "kind", "sort_col", "update_check", "content_index", "floating_mode",
            "floating_pos", "hotkey_opens", "fade", "off", "main", "=", "\n",
            "\r\n", ",", " ", "-", "1", "0", "99999999999999999999", "٣", "\u{FEFF}", "\u{0}", "ب", "😀", "\u{200F}",
        ];
        let mut seed = 0xABCD_EF01_u64;
        let mut below = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        for _ in 0..5000 {
            let text: String = (0..below(30)).map(|_| PIECES[below(PIECES.len())]).collect();
            let s = Settings::parse(&text);
            assert!((400..=10_000).contains(&s.width) && (300..=10_000).contains(&s.height), "{text:?}");
            assert!((-1..=3).contains(&s.sort_col), "{text:?}");
            assert_eq!(Settings::parse(&s.to_text()), s, "{text:?}");
        }
    }

    #[test]
    fn unreadable_file_gives_defaults() {
        let dir = std::env::temp_dir().join(format!("bahth_settings_bad_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("folder.txt")).unwrap();
        let file = dir.join("settings.txt");
        // Not UTF-8, a folder, nothing there.
        std::fs::write(&file, [0xFF, 0xFE, b'w', 0x80]).unwrap();
        assert_eq!(Settings::load_from(&file), Settings::default());
        assert_eq!(Settings::load_from(&dir.join("folder.txt")), Settings::default());
        // Saving where a folder sits fails without touching it.
        assert!(Settings::default().save_to(&dir.join("folder.txt")).is_err());
        assert!(dir.join("folder.txt").is_dir());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_and_load_file() {
        let dir = std::env::temp_dir().join(format!("bahth_settings_{}", std::process::id()));
        let file = dir.join("sub").join("settings.txt");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Settings::load_from(&file), Settings::default());
        let s = Settings { width: 1234, columns: vec![1, 2], sort_col: 0, ..Settings::default() };
        s.save_to(&file).unwrap();
        assert_eq!(Settings::load_from(&file), s);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
