//! User preferences kept between runs in a small `key=value` text file next to
//! the index cache.

// Only the Windows UI and the tests use this module.
#![cfg(any(windows, test))]

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
}

impl Default for Settings {
    fn default() -> Self {
        Settings { width: 1100, height: 680, maximized: false, columns: Vec::new(), arabic_digits: false, kind: 0, sort_col: -1, sort_desc: false, update_check: 0 }
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
                _ => {}
            }
        }
        s
    }

    pub fn to_text(&self) -> String {
        let cols: Vec<String> = self.columns.iter().map(i32::to_string).collect();
        format!(
            "width={}\nheight={}\nmaximized={}\ncolumns={}\narabic_digits={}\nkind={}\nsort_col={}\nsort_desc={}\nupdate_check={}\n",
            self.width,
            self.height,
            self.maximized as u8,
            cols.join(","),
            self.arabic_digits as u8,
            self.kind,
            self.sort_col,
            self.sort_desc as u8,
            self.update_check
        )
    }

    /// Reads `path`; a missing file gives the defaults.
    pub fn load_from(path: &Path) -> Settings {
        std::fs::read_to_string(path).map(|t| Settings::parse(&t)).unwrap_or_default()
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_text())
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
        let s = Settings { width: 900, height: 500, maximized: true, columns: vec![200, 300, 80], arabic_digits: true, kind: 3, sort_col: 2, sort_desc: true, update_check: 1_700_000_000 };
        assert_eq!(Settings::parse(&s.to_text()), s);
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
    }

    #[test]
    fn tolerant_syntax() {
        let s = Settings::parse("\u{FEFF}junk\r\n  width = 900 \r\nmaximized=1\r\n[section]\r\nunknown=5\r\n=1\r\n");
        assert_eq!((s.width, s.maximized), (900, true));
        assert_eq!(Settings { width: 900, maximized: true, ..Settings::default() }, s);
        assert_eq!(Settings::parse(""), Settings::default());
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
