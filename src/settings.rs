//! User preferences kept between runs in a small `key=value` text file next to
//! the index cache.

use std::path::PathBuf;

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
}

impl Default for Settings {
    fn default() -> Self {
        Settings { width: 1100, height: 680, maximized: false, columns: Vec::new(), arabic_digits: false, kind: 0 }
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
                _ => {}
            }
        }
        s
    }

    pub fn to_text(&self) -> String {
        let cols: Vec<String> = self.columns.iter().map(i32::to_string).collect();
        format!(
            "width={}\nheight={}\nmaximized={}\ncolumns={}\narabic_digits={}\nkind={}\n",
            self.width,
            self.height,
            self.maximized as u8,
            cols.join(","),
            self.arabic_digits as u8,
            self.kind
        )
    }

    pub fn load() -> Settings {
        std::fs::read_to_string(path()).map(|t| Settings::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self) {
        let p = path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, self.to_text());
    }
}

/// `%LOCALAPPDATA%\BahthSari\settings.txt`.
fn path() -> PathBuf {
    crate::index::cache_path().with_file_name("settings.txt")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let s = Settings { width: 900, height: 500, maximized: true, columns: vec![200, 300, 80], arabic_digits: true, kind: 3 };
        assert_eq!(Settings::parse(&s.to_text()), s);
        assert_eq!(Settings::parse("garbage\nwidth=abc"), Settings::default());
    }
}
