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
    /// Read the contents of cloud-only files (OneDrive Files On-Demand and
    /// the like) too, which makes Windows download them. Off unless the user
    /// switches it on: the program never downloads anything on its own.
    pub read_cloud_files: bool,
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
    /// The user was offered once to move a copy that runs from Downloads or
    /// a temp folder to a permanent place (`crate::relocate`).
    pub move_asked: bool,
    /// The desktop shortcut was made, or tried, once (1.0.1): by the move,
    /// or at the first start of a copy already in the permanent place
    /// (`crate::relocate::desktop_shortcut_at_start`). Never made again
    /// after that, so a shortcut the user deleted stays deleted.
    pub desktop_shortcut_done: bool,
    /// Files opened from search results (window or quick popup), counted
    /// only until the Store rating question was asked (`crate::rating`).
    /// Kept on this computer only.
    pub file_opens: u32,
    /// The one-time Store rating question was shown or answered.
    pub rating_asked: bool,
    /// Use the new main window (WebView2) instead of the classic one, when
    /// WebView2 is there (see `crate::webui`). Always on since 1.0
    /// ([`Settings::make_new_look_only`]); still written, for older versions.
    pub new_ui: bool,
    /// The classic window took over at the last launch because the new
    /// look failed ([`Settings::note_fallback`]): the next start tries the
    /// new look again, without the second chances
    /// (`crate::webui::start_tolerance`). Cleared by a page that works a while.
    pub new_ui_failed: bool,
    /// Starts in a row that began creating the new look's page and never
    /// heard its hello (a hard crash leaves no fallback behind): saved right
    /// before the page is created, cleared by its hello. From 2 the next
    /// start uses the classic window for that run (`crate::webui::start_look`).
    pub new_ui_trying: u32,
    /// 0.10.1's one-time switch to the new look was made. Kept and written:
    /// `crate::i18n` takes a file with it as written by a version with English.
    pub new_ui_default: bool,
    /// 1.0's one-time move of classic users to the new look was made
    /// ([`Settings::make_new_look_only`]).
    pub new_ui_only: bool,
    /// The global hotkey that opens InstaFind (Ctrl+Alt+F unless the user
    /// picked another, e.g. because another program has it).
    pub hotkey: Hotkey,
    /// The main window's colours (Settings ▸ Display); Windows' own
    /// light or dark unless the user picked one.
    pub theme: Theme,
}

/// The colour themes of the main window (the page's `data-theme`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    /// Follows Windows' light or dark app mode.
    #[default]
    Auto,
    Light,
    Dark,
    /// Dark teal with lime accents: the look of 1.0.
    Midnight,
    /// Warm off-white with kiwi-green accents (fastkiwi.app).
    Kiwi,
}

impl Theme {
    pub const ALL: [Theme; 5] = [Theme::Auto, Theme::Light, Theme::Dark, Theme::Midnight, Theme::Kiwi];

    /// As saved, and as the page names it (the serde name).
    pub fn code(self) -> &'static str {
        match self {
            Theme::Auto => "auto",
            Theme::Light => "light",
            Theme::Dark => "dark",
            Theme::Midnight => "midnight",
            Theme::Kiwi => "kiwi",
        }
    }

    pub fn from_code(code: &str) -> Option<Theme> {
        let code = code.trim().to_ascii_lowercase();
        Theme::ALL.into_iter().find(|t| t.code() == code)
    }

    /// Whether its colours are dark (the title bar follows), given whether
    /// Windows' apps are dark now.
    #[cfg_attr(not(all(windows, target_env = "msvc")), allow(dead_code))]
    pub fn dark(self, windows_dark: bool) -> bool {
        match self {
            Theme::Auto => windows_dark,
            Theme::Light | Theme::Kiwi => false,
            Theme::Dark | Theme::Midnight => true,
        }
    }
}

/// The keys of the global "open InstaFind" hotkey the user can choose from
/// (Settings ▸ Search from anywhere). Ctrl+Alt+F unless another program has
/// it; the others are offered then.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hotkey {
    #[default]
    CtrlAltF,
    CtrlAltSpace,
    CtrlShiftF,
    WinAltF,
}

impl Hotkey {
    pub const ALL: [Hotkey; 4] = [Hotkey::CtrlAltF, Hotkey::CtrlAltSpace, Hotkey::CtrlShiftF, Hotkey::WinAltF];

    /// As saved, and as the page names it (the serde name).
    pub fn code(self) -> &'static str {
        match self {
            Hotkey::CtrlAltF => "ctrl_alt_f",
            Hotkey::CtrlAltSpace => "ctrl_alt_space",
            Hotkey::CtrlShiftF => "ctrl_shift_f",
            Hotkey::WinAltF => "win_alt_f",
        }
    }

    pub fn from_code(code: &str) -> Option<Hotkey> {
        let code = code.trim().to_ascii_lowercase();
        Hotkey::ALL.into_iter().find(|h| h.code() == code)
    }

    /// The keys as Windows writes them, in every language.
    pub fn label(self) -> &'static str {
        match self {
            Hotkey::CtrlAltF => "Ctrl+Alt+F",
            Hotkey::CtrlAltSpace => "Ctrl+Alt+Space",
            Hotkey::CtrlShiftF => "Ctrl+Shift+F",
            Hotkey::WinAltF => "Win+Alt+F",
        }
    }
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
        Settings { width: 1100, height: 680, maximized: false, columns: Vec::new(), arabic_digits: false, kind: 0, sort_col: -1, sort_desc: false, update_check: 0, content_index: true, read_cloud_files: false, auto_index: true, hide_system: true, crash_seen: 0, floating_mode: FloatingMode::Off, floating_pos: None, hotkey_opens: HotkeyOpens::Quick, alt_space: false, tray_icon: true, floating_asked: false, move_asked: false, desktop_shortcut_done: false, file_opens: 0, rating_asked: false, new_ui: false, new_ui_failed: false, new_ui_trying: 0, new_ui_default: false, new_ui_only: false, hotkey: Hotkey::CtrlAltF, theme: Theme::Auto }
    }
}

impl Settings {
    /// The new look is the only one (1.0). Once per settings file (the
    /// marker `new_ui_only`), whoever used the classic window is moved to
    /// the new look: settings of 0.10.0 or earlier, "Back to the classic
    /// look" of 0.10.x (`new_ui=0` with 0.10.1's marker `new_ui_default`),
    /// and machines where 0.10.x gave the new look up after two failed
    /// starts. From then on `new_ui=0` in the file is ignored as well. The
    /// classic window is only a fallback for one run, when WebView2 is
    /// missing or fails ([`Settings::note_fallback`]), or for the tests
    /// (`crate::webui::CLASSIC_ARG`). `new_ui_default` is set with it, as
    /// 0.10.1 did: `crate::i18n` looks for that key. Returns whether
    /// anything changed (then save).
    pub fn make_new_look_only(&mut self) -> bool {
        let before = (self.new_ui, self.new_ui_default, self.new_ui_only);
        self.new_ui = true;
        self.new_ui_default = true;
        self.new_ui_only = true;
        before != (true, true, true)
    }

    /// The classic window took over for this run (WebView2 failed to start,
    /// or its page failed twice): remembered so the next start tries the
    /// new look again but gives up quickly if it still fails. The new look
    /// is never turned off by it.
    // (Only MSVC builds host the new look, see `crate::webui`.)
    #[cfg_attr(not(all(windows, target_env = "msvc")), allow(dead_code))]
    pub fn note_fallback(&mut self) {
        self.new_ui_failed = true;
    }

    /// Whether the one-time switch just done changed the look for someone
    /// who used the classic window (a settings file was there, the new look
    /// was off): they are told once. Not for a new user.
    pub fn switch_to_tell(had_file: bool, was_on: bool, switched: bool) -> bool {
        switched && had_file && !was_on
    }

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
                // A type the list does not have (a damaged file) is "all
                // types": the last entry, folders only, would hide every file.
                "kind" => s.kind = value.parse().ok().filter(|&k| k < crate::search::KINDS.len()).unwrap_or(0),
                "sort_col" => s.sort_col = value.parse().unwrap_or(-1).clamp(-1, 3),
                "sort_desc" => s.sort_desc = value == "1",
                "update_check" => s.update_check = value.parse().unwrap_or(0),
                "content_index" => s.content_index = value != "0",
                // Only an explicit 1 downloads anything.
                "read_cloud_files" => s.read_cloud_files = value == "1",
                "auto_index" => s.auto_index = value != "0",
                "hide_system" => s.hide_system = value != "0",
                "crash_seen" => s.crash_seen = value.parse().unwrap_or(0),
                "floating_mode" => s.floating_mode = FloatingMode::from_code(value).unwrap_or_default(),
                "floating_pos" => s.floating_pos = FloatPos::parse(value),
                "hotkey_opens" => s.hotkey_opens = HotkeyOpens::from_code(value).unwrap_or_default(),
                "alt_space" => s.alt_space = value == "1",
                "tray_icon" => s.tray_icon = value != "0",
                "floating_asked" => s.floating_asked = value == "1",
                "move_asked" => s.move_asked = value == "1",
                "desktop_shortcut_done" => s.desktop_shortcut_done = value == "1",
                "file_opens" => s.file_opens = value.parse().unwrap_or(0),
                "rating_asked" => s.rating_asked = value == "1",
                "new_ui" => s.new_ui = value == "1",
                "new_ui_failed" => s.new_ui_failed = value == "1",
                "new_ui_trying" => s.new_ui_trying = value.parse().unwrap_or(0).min(crate::webui::MAX_TRYING),
                "new_ui_default" => s.new_ui_default = value == "1",
                "new_ui_only" => s.new_ui_only = value == "1",
                "hotkey" => s.hotkey = Hotkey::from_code(value).unwrap_or_default(),
                "theme" => s.theme = Theme::from_code(value).unwrap_or_default(),
                _ => {}
            }
        }
        s
    }

    pub fn to_text(&self) -> String {
        let cols: Vec<String> = self.columns.iter().map(i32::to_string).collect();
        format!(
            "width={}\nheight={}\nmaximized={}\ncolumns={}\narabic_digits={}\nkind={}\nsort_col={}\nsort_desc={}\nupdate_check={}\ncontent_index={}\nauto_index={}\nhide_system={}\ncrash_seen={}\nfloating_mode={}\nfloating_pos={}\nhotkey_opens={}\nalt_space={}\ntray_icon={}\nfloating_asked={}\nmove_asked={}\nfile_opens={}\nrating_asked={}\nnew_ui={}\nnew_ui_failed={}\nnew_ui_default={}\nread_cloud_files={}\nnew_ui_only={}\nnew_ui_trying={}\nhotkey={}\ndesktop_shortcut_done={}\ntheme={}\n",
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
            self.floating_asked as u8,
            self.move_asked as u8,
            self.file_opens,
            self.rating_asked as u8,
            self.new_ui as u8,
            self.new_ui_failed as u8,
            self.new_ui_default as u8,
            self.read_cloud_files as u8,
            self.new_ui_only as u8,
            self.new_ui_trying,
            self.hotkey.code(),
            self.desktop_shortcut_done as u8,
            self.theme.code()
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

    /// Whether a settings file is there (a program run before).
    #[cfg(windows)]
    pub fn exists() -> bool {
        Self::path().is_file()
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
        let s = Settings { width: 900, height: 500, maximized: true, columns: vec![200, 300, 80], arabic_digits: true, kind: 3, sort_col: 2, sort_desc: true, update_check: 1_700_000_000, content_index: false, read_cloud_files: true, auto_index: false, hide_system: false, crash_seen: 1_800_000_000, floating_mode: FloatingMode::Hidden, floating_pos: Some(FloatPos { monitor: [-1920, 0, 0, 1080], x: -500, y: 990 }), hotkey_opens: HotkeyOpens::Main, alt_space: true, tray_icon: false, floating_asked: true, move_asked: true, desktop_shortcut_done: true, file_opens: 7, rating_asked: true, new_ui: true, new_ui_failed: true, new_ui_trying: 2, new_ui_default: true, new_ui_only: true, hotkey: Hotkey::WinAltF, theme: Theme::Kiwi };
        assert_eq!(Settings::parse(&s.to_text()), s);
        for mode in [FloatingMode::Fade, FloatingMode::Hidden, FloatingMode::Off] {
            let s = Settings { floating_mode: mode, ..Settings::default() };
            assert_eq!(Settings::parse(&s.to_text()), s);
        }
        assert_eq!(Settings::parse("garbage\nwidth=abc"), Settings::default());
    }

    #[test]
    fn hotkey_choice() {
        assert_eq!(Settings::default().hotkey, Hotkey::CtrlAltF);
        assert!(Settings::default().to_text().contains("hotkey=ctrl_alt_f\n"));
        for h in Hotkey::ALL {
            let s = Settings { hotkey: h, ..Settings::default() };
            assert_eq!(Settings::parse(&s.to_text()), s);
            assert_eq!(Hotkey::from_code(h.code()), Some(h));
            // The page names it by the same code (bridge.rs `Request::Hotkey`).
            assert_eq!(serde_json::from_str::<Hotkey>(&format!("\"{}\"", h.code())).unwrap(), h);
        }
        assert_eq!(Settings::parse("hotkey=CTRL_SHIFT_F\n").hotkey, Hotkey::CtrlShiftF);
        // An older file (no key) or a damaged one: Ctrl+Alt+F.
        assert_eq!(Settings::parse("hotkey=alt_f4\n").hotkey, Hotkey::CtrlAltF);
        assert_eq!(Settings::parse("width=900\n").hotkey, Hotkey::CtrlAltF);
        let labels: Vec<&str> = Hotkey::ALL.iter().map(|h| h.label()).collect();
        assert_eq!(labels, ["Ctrl+Alt+F", "Ctrl+Alt+Space", "Ctrl+Shift+F", "Win+Alt+F"]);
    }

    #[test]
    fn theme_choice() {
        assert_eq!(Settings::default().theme, Theme::Auto);
        assert!(Settings::default().to_text().contains("theme=auto\n"));
        for t in Theme::ALL {
            let s = Settings { theme: t, ..Settings::default() };
            assert_eq!(Settings::parse(&s.to_text()), s);
            assert_eq!(Theme::from_code(t.code()), Some(t));
            // The page names it by the same code (bridge.rs `Request::Theme`).
            assert_eq!(serde_json::from_str::<Theme>(&format!("\"{}\"", t.code())).unwrap(), t);
        }
        // An older file (no key) or a damaged one: Windows' own.
        assert_eq!(Settings::parse("theme=sepia\n").theme, Theme::Auto);
        assert_eq!(Settings::parse("theme= Midnight\n").theme, Theme::Midnight);
        assert_eq!(Settings::parse("width=900\n").theme, Theme::Auto);
        assert!(Theme::Auto.dark(true) && !Theme::Auto.dark(false));
        assert!(Theme::Dark.dark(false) && Theme::Midnight.dark(false));
        assert!(!Theme::Light.dark(true) && !Theme::Kiwi.dark(true));
    }

    #[test]
    fn bad_values_fall_back_or_clamp() {
        let d = Settings::default();
        let s = Settings::parse("width=50\nheight=99999\nsort_col=7\nkind=abc\nupdate_check=-1\nmaximized=yes");
        assert_eq!((s.width, s.height, s.sort_col, s.kind, s.update_check, s.maximized), (400, 10_000, 3, 0, 0, false));
        let s = Settings::parse("width=-5\nheight=\nsort_col=-9\nkind=-1\nsort_desc=true\narabic_digits=0");
        assert_eq!((s.width, s.height, s.sort_col, s.kind, s.sort_desc, s.arabic_digits), (d.width, d.height, -1, 0, false, false));
        // A file type past the end of the list is "all types", not the last
        // entry (folders only, which would hide every file).
        let last = crate::search::KINDS.len() - 1;
        assert_eq!(Settings::parse(&format!("kind={last}")).kind, last);
        for bad in [format!("kind={}", last + 1), "kind=4294967295".into(), "kind=18446744073709551615".into(), "kind=99999999999999999999".into()] {
            assert_eq!(Settings::parse(&bad).kind, 0, "{bad}");
        }
        // Unparsable column widths are dropped, the rest kept in order.
        assert_eq!(Settings::parse("columns=100,x,, 200 ,-3,9999999999").columns, vec![100, 200, -3]);
        assert!(Settings::parse("columns=").columns.is_empty());
        // `=` inside a value makes it unparsable; the last duplicate key wins.
        assert_eq!(Settings::parse("width=900=1").width, d.width);
        assert_eq!(Settings::parse("width=900\nwidth=1000").width, 1000);
        // The content index is on unless switched off.
        assert!(Settings::parse("content_index=yes").content_index && !Settings::parse("content_index=0").content_index);
        // Cloud-only files are read only when switched on with a 1: missing,
        // unknown or absurd values mean off.
        assert!(!d.read_cloud_files && Settings::parse("read_cloud_files=1").read_cloud_files);
        for v in ["", "0", "yes", "true", "2", "-1", "١", "1 1", "99999999999999999999", "\u{0}"] {
            assert!(!Settings::parse(&format!("read_cloud_files={v}")).read_cloud_files, "{v:?}");
        }
        assert!(Settings::parse("read_cloud_files = 1 \r\n").read_cloud_files);
        assert!(!Settings::parse("read_cloud_files=1\nread_cloud_files=0").read_cloud_files);
        let on = Settings { read_cloud_files: true, ..d.clone() };
        assert_eq!(Settings::parse(&on.to_text()), on);
        assert!(Settings::default().to_text().contains("read_cloud_files=0\n"));
        // Automatic indexing is on unless explicitly turned off.
        assert!(d.auto_index && Settings::parse("auto_index=yes").auto_index && !Settings::parse("auto_index=0").auto_index);
        // System files are hidden unless explicitly shown.
        assert!(d.hide_system && Settings::parse("hide_system=yes").hide_system && !Settings::parse("hide_system=0").hide_system);
    }

    #[test]
    fn quick_search_settings() {
        let d = Settings::default();
        // 1.0 defaults: no box above the clock until the user turns it on (no
        // first-run question), the hotkey opens the quick search, a tray icon, Alt+Space off.
        assert_eq!((d.floating_mode, d.hotkey_opens, d.alt_space, d.tray_icon, d.floating_asked), (FloatingMode::Off, HotkeyOpens::Quick, false, true, false));
        assert_eq!(d.floating_pos, None);
        // Settings files from before 0.8 get the defaults.
        assert_eq!(Settings::parse("width=900\nhide_system=1\n"), Settings { width: 900, ..d.clone() });
        // Unknown modes fall back to the defaults; case and spaces are ignored.
        let s = Settings::parse("floating_mode=blink\nhotkey_opens=both\ntray_icon=yes\nalt_space=yes");
        assert_eq!((s.floating_mode, s.hotkey_opens, s.tray_icon, s.alt_space), (FloatingMode::Fade, HotkeyOpens::Quick, true, false));
        let s = Settings::parse("floating_mode= OFF \nhotkey_opens=Main\ntray_icon=0\nfloating_asked=1");
        assert_eq!((s.floating_mode, s.hotkey_opens, s.tray_icon, s.floating_asked), (FloatingMode::Off, HotkeyOpens::Main, false, true));
        // The move offer is made once; older settings files have not had it.
        assert!(!d.move_asked && !Settings::parse("floating_asked=1").move_asked && Settings::parse("move_asked=1").move_asked);
        // So is the desktop shortcut; files of 1.0.0 and older have not had it.
        assert!(!d.desktop_shortcut_done && !Settings::parse("move_asked=1").desktop_shortcut_done);
        assert!(Settings::parse("desktop_shortcut_done=1").desktop_shortcut_done && !Settings::parse("desktop_shortcut_done=yes").desktop_shortcut_done);
        assert!(d.to_text().contains("desktop_shortcut_done=0\n"));
        // The Store rating counter starts at zero and the question is not asked yet.
        assert_eq!((d.file_opens, d.rating_asked), (0, false));
        let s = Settings::parse("file_opens=5\nrating_asked=1");
        assert_eq!((s.file_opens, s.rating_asked), (5, true));
        assert_eq!(Settings::parse("file_opens=-3\nrating_asked=yes").file_opens, 0);
        assert!(!Settings::parse("rating_asked=yes").rating_asked);
        // Read as written (the one-time move turns the new look on, not parsing).
        assert!(!d.new_ui && !Settings::parse("width=900").new_ui && !Settings::parse("new_ui=yes").new_ui && Settings::parse("new_ui=1").new_ui);
        assert!(!Settings::parse("new_ui=1\nnew_ui=0").new_ui);
        assert!(!d.new_ui_failed && Settings::parse("new_ui_failed=1").new_ui_failed && !Settings::parse("new_ui=1").new_ui_failed);
        assert_eq!(d.new_ui_trying, 0);
        assert_eq!(Settings::parse("new_ui_trying=1").new_ui_trying, 1);
        // A damaged or hand-edited count means "crashed twice" at most.
        assert_eq!(Settings::parse("new_ui_trying=999").new_ui_trying, crate::webui::MAX_TRYING);
        assert_eq!(Settings::parse("new_ui_trying=-1").new_ui_trying, 0);
        assert!(!d.new_ui_default && Settings::parse("new_ui_default=1").new_ui_default && !Settings::parse("new_ui_default=yes").new_ui_default);
        assert!(!d.new_ui_only && Settings::parse("new_ui_only=1").new_ui_only && !Settings::parse("new_ui_only=yes").new_ui_only);
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
    fn the_new_look_is_the_only_one() {
        // A new user (no settings file): the new look, with every marker.
        let mut s = Settings::parse("");
        assert!(s.make_new_look_only());
        assert!(s.new_ui && s.new_ui_default && s.new_ui_only);
        // Saved and read again: nothing more to do.
        let mut again = Settings::parse(&s.to_text());
        assert!(!again.make_new_look_only());
        assert_eq!(again, s);
        // 0.10.0 or earlier (new_ui=0, no marker): moved, the rest kept.
        let mut old = Settings::parse("width=900\nfloating_asked=1\nnew_ui=0\nnew_ui_failed=0\n");
        assert!(old.make_new_look_only());
        assert!(old.new_ui && old.new_ui_default && old.new_ui_only && old.width == 900 && old.floating_asked);
        // "Back to the classic look" of 0.10.x (new_ui=0 after 0.10.1's
        // switch), or 0.10.x's give-up after two failed starts: moved once.
        for text in ["new_ui=0\nnew_ui_default=1\n", "new_ui=0\nnew_ui_failed=0\nnew_ui_default=1\n"] {
            let mut back = Settings::parse(text);
            assert!(back.make_new_look_only(), "{text:?}");
            assert!(back.new_ui && back.new_ui_only, "{text:?}");
            // Start after start: the new look, nothing to save.
            for _ in 0..3 {
                back = Settings::parse(&back.to_text());
                assert!(!back.make_new_look_only() && back.new_ui, "{text:?}");
            }
        }
        // Already on: stays on, the marker is written.
        let mut on = Settings::parse("new_ui=1\nnew_ui_default=1\n");
        assert!(on.make_new_look_only() && on.new_ui && on.new_ui_only);
        // After the move, new_ui=0 in the file (an older version, a hand
        // edit) is ignored: the new look again.
        let mut edited = Settings::parse("new_ui=0\nnew_ui_default=1\nnew_ui_only=1\n");
        assert!(edited.make_new_look_only());
        assert!(edited.new_ui);
        // Settings written by 1.0 still carry 0.10.1's key (`crate::i18n`).
        assert!(s.to_text().lines().any(|l| l == "new_ui_default=1"));
        assert!(s.to_text().lines().any(|l| l == "new_ui_only=1"));
    }

    #[test]
    fn a_fallback_does_not_persist() {
        // The classic window took over (WebView2 failed): remembered, but
        // the new look stays chosen for the next start.
        let mut s = Settings::parse("");
        s.make_new_look_only();
        s.note_fallback();
        let mut next = Settings::parse(&s.to_text());
        assert!(next.new_ui && next.new_ui_failed);
        assert!(!next.make_new_look_only());
        assert!(next.new_ui && crate::webui::chosen(next.new_ui) == !crate::webui::classic_requested());
        // ... tried again without the second chances.
        assert_eq!(crate::webui::start_tolerance(next.new_ui_failed), (1, 1));
        // Falling back again and again never turns it off.
        for _ in 0..5 {
            next.note_fallback();
            next = Settings::parse(&next.to_text());
            next.make_new_look_only();
            assert!(next.new_ui && next.new_ui_only);
        }
        // A failure remembered by 0.10.x (new_ui=1, new_ui_failed=1) is kept.
        let mut once = Settings::parse("new_ui=1\nnew_ui_failed=1\n");
        assert!(once.make_new_look_only());
        assert!(once.new_ui && once.new_ui_failed);
    }

    #[test]
    fn only_classic_users_are_told_about_the_switch() {
        assert!(Settings::switch_to_tell(true, false, true));
        // A new user, someone who had the new look on, no switch made.
        assert!(!Settings::switch_to_tell(false, false, true));
        assert!(!Settings::switch_to_tell(true, true, true));
        assert!(!Settings::switch_to_tell(true, false, false));
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
            "width", "height", "maximized", "columns", "kind", "sort_col", "update_check", "content_index", "read_cloud_files", "floating_mode",
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
        let _tmp = crate::test_temp::TempPath::at(dir.clone());
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
        let _tmp = crate::test_temp::TempPath::at(dir.clone());
        let file = dir.join("sub").join("settings.txt");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Settings::load_from(&file), Settings::default());
        let s = Settings { width: 1234, columns: vec![1, 2], sort_col: 0, ..Settings::default() };
        s.save_to(&file).unwrap();
        assert_eq!(Settings::load_from(&file), s);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
