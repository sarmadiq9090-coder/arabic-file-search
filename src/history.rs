//! Search history, saved searches and how often files are opened, kept in a
//! small JSON file next to the index cache. Often and recently opened files
//! are boosted to the top of the results.

// Only the Windows UI and the tests use this module.
#![cfg(any(windows, test))]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::index::Index;

/// How many recent queries are kept.
const MAX_RECENT: usize = 20;
/// How many saved searches are kept.
const MAX_SAVED: usize = 30;
/// How many opened files are remembered.
const MAX_OPENS: usize = 500;
/// After this long an open counts half as much.
const HALF_LIFE_SECS: f64 = 14.0 * 24.0 * 3600.0;

/// A named search the user can run again.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct SavedSearch {
    pub name: String,
    pub query: String,
    /// Selected entry of [`crate::search::KINDS`].
    pub kind: usize,
    /// Folder the search is limited to, "" for everywhere.
    pub scope: String,
    /// Search inside documents too.
    pub content: bool,
}

#[derive(Serialize, Deserialize, Default, Debug)]
#[serde(default)]
pub struct History {
    /// Recent queries, newest first.
    pub recent: Vec<String>,
    pub saved: Vec<SavedSearch>,
    /// Opened files: path → (times opened, last open in Unix seconds).
    opens: HashMap<String, (u32, i64)>,
}

/// Frecency: the open count, halved for every [`HALF_LIFE_SECS`] since the last open.
fn score(count: u32, last: i64, now: i64) -> f64 {
    let age = (now - last).max(0) as f64;
    count as f64 * 0.5f64.powf(age / HALF_LIFE_SECS)
}

/// True when `path` is `root` or lies inside it.
fn under(path: &str, root: &str) -> bool {
    path.strip_prefix(root)
        .is_some_and(|rest| rest.is_empty() || root.ends_with(['\\', '/']) || rest.starts_with(['\\', '/']))
}

impl History {
    /// `history.json` next to the index cache.
    #[cfg(windows)]
    pub fn path() -> std::path::PathBuf {
        crate::index::cache_path().with_file_name("history.json")
    }

    #[cfg(windows)]
    pub fn load() -> History {
        Self::load_from(&Self::path())
    }

    /// Reads `path`; a missing or corrupt file gives an empty history.
    pub fn load_from(path: &Path) -> History {
        fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// Saves to the default location, ignoring errors (history is a convenience).
    #[cfg(windows)]
    pub fn save(&self) {
        let _ = self.save_to(&Self::path());
    }

    /// Writes to a temp file and renames it, so a crash never leaves a broken file.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec(self).map_err(std::io::Error::other)?)?;
        fs::rename(tmp, path)
    }

    /// Remembers a query at the front of the list. Queries shorter than two
    /// characters are ignored and repeats move to the front.
    pub fn add_recent(&mut self, query: &str) {
        let q = query.trim();
        if q.chars().count() < 2 {
            return;
        }
        self.recent.retain(|r| r != q);
        self.recent.insert(0, q.to_string());
        self.recent.truncate(MAX_RECENT);
    }

    /// Adds a saved search, replacing one with the same name in place. When
    /// the list is full the oldest one is dropped.
    pub fn add_saved(&mut self, s: SavedSearch) {
        if let Some(old) = self.saved.iter_mut().find(|o| o.name == s.name) {
            *old = s;
            return;
        }
        self.saved.push(s);
        if self.saved.len() > MAX_SAVED {
            self.saved.remove(0);
        }
    }

    /// Counts an open of `path` at time `now` (Unix seconds). When more than
    /// [`MAX_OPENS`] files are remembered the lowest-scoring one is forgotten.
    pub fn record_open(&mut self, path: &str, now: i64) {
        let e = self.opens.entry(path.to_string()).or_insert((0, now));
        e.0 = e.0.saturating_add(1);
        e.1 = now;
        if self.opens.len() > MAX_OPENS {
            // Never evict the file that was just opened.
            let worst = self
                .opens
                .iter()
                .filter(|(k, _)| k.as_str() != path)
                .min_by(|a, b| score(a.1.0, a.1.1, now).total_cmp(&score(b.1.0, b.1.1, now)))
                .map(|(k, _)| k.clone());
            if let Some(k) = worst {
                self.opens.remove(&k);
            }
        }
    }

    /// Follows a rename of a file or folder (everything inside it too).
    pub fn rename_path(&mut self, old: &str, new: &str) {
        let moved: Vec<String> = self.opens.keys().filter(|k| under(k, old)).cloned().collect();
        for k in moved {
            if let Some(v) = self.opens.remove(&k) {
                self.opens.insert(format!("{new}{}", &k[old.len()..]), v);
            }
        }
    }

    /// Forgets a deleted file or folder (everything inside it too).
    pub fn forget_path(&mut self, path: &str) {
        self.opens.retain(|k, _| !under(k, path));
    }

    /// Opened files still in `index`, as (entry index, frecency score),
    /// highest first. Feed it to [`crate::search::promote`].
    pub fn boosted(&self, index: &Index, now: i64) -> Vec<(u32, f64)> {
        let mut v: Vec<(u32, f64)> =
            self.opens.iter().filter_map(|(p, &(c, t))| index.find(p).map(|i| (i as u32, score(c, t, now)))).collect();
        v.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::Entry;

    const DAY: i64 = 24 * 3600;

    #[test]
    fn recent_queries() {
        let mut h = History::default();
        for q in ["  تقرير ", "a", "   ", "عقد", "تقرير"] {
            h.add_recent(q);
        }
        assert_eq!(h.recent, vec!["تقرير", "عقد"]);
        for i in 0..30 {
            h.add_recent(&format!("q{i}"));
        }
        assert_eq!(h.recent.len(), MAX_RECENT);
        assert_eq!(h.recent[0], "q29");
    }

    #[test]
    fn saved_searches() {
        let mut h = History::default();
        let s = |name: &str, query: &str| SavedSearch { name: name.into(), query: query.into(), ..Default::default() };
        h.add_saved(s("a", "1"));
        h.add_saved(s("b", "2"));
        h.add_saved(s("a", "3"));
        assert_eq!(h.saved, vec![s("a", "3"), s("b", "2")]);
        for i in 0..40 {
            h.add_saved(s(&i.to_string(), ""));
        }
        assert_eq!(h.saved.len(), MAX_SAVED);
        assert_eq!(h.saved.last().unwrap().name, "39");
    }

    #[test]
    fn opens_rename_forget_and_cap() {
        let mut h = History::default();
        h.record_open(r"C:\a\x.txt", 0);
        h.record_open(r"C:\a\x.txt", 10);
        h.record_open(r"C:\ab\y.txt", 0);
        h.record_open(r"C:\a", 0);
        assert_eq!(h.opens[r"C:\a\x.txt"], (2, 10));
        h.rename_path(r"C:\a", r"D:\b");
        let mut keys: Vec<&str> = h.opens.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, vec![r"C:\ab\y.txt", r"D:\b", r"D:\b\x.txt"]);
        h.forget_path(r"D:\b");
        assert_eq!(h.opens.len(), 1);
        h.forget_path(r"C:\");
        assert!(h.opens.is_empty());

        h.record_open("keep", 0);
        h.record_open("keep", 0);
        for i in 0..MAX_OPENS + 10 {
            h.record_open(&format!("f{i}"), 0);
        }
        assert_eq!(h.opens.len(), MAX_OPENS);
        assert!(h.opens.contains_key("keep"));
    }

    #[test]
    fn frecency_boost() {
        let entries = [r"C:\new.txt", r"C:\old.txt", r"C:\x.txt"]
            .iter()
            .map(|p| Entry { path: (*p).into(), size: 0, mtime: 0, is_dir: false, norm_name: Box::default() })
            .collect();
        let index = Index { entries, ..Index::default() };
        let now = 100 * DAY;
        let mut h = History::default();
        for _ in 0..3 {
            h.record_open(r"C:\old.txt", now - 60 * DAY);
        }
        h.record_open(r"C:\new.txt", now - DAY);
        h.record_open(r"C:\gone.txt", now);
        let b = h.boosted(&index, now);
        assert_eq!(b.iter().map(|x| x.0).collect::<Vec<_>>(), vec![0, 1]);
        assert!((score(2, now - 14 * DAY, now) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn save_and_load() {
        let dir = std::env::temp_dir().join(format!("bahth_hist_{}", std::process::id()));
        let file = dir.join("history.json");
        let _ = fs::remove_dir_all(&dir);
        assert!(History::load_from(&file).recent.is_empty());
        let mut h = History::default();
        h.add_recent("تقرير السنة");
        h.add_saved(SavedSearch {
            name: "عقود".into(),
            query: "عقد ext:pdf".into(),
            kind: 1,
            scope: r"D:\".into(),
            content: true,
        });
        h.record_open(r"C:\x.txt", 5);
        h.save_to(&file).unwrap();
        let l = History::load_from(&file);
        assert_eq!(l.recent, h.recent);
        assert_eq!(l.saved, h.saved);
        assert_eq!(l.opens, h.opens);
        // Unknown or missing fields are fine; garbage gives an empty history.
        fs::write(&file, r#"{"recent":["x1"],"extra":1}"#).unwrap();
        assert_eq!(History::load_from(&file).recent, vec!["x1"]);
        fs::write(&file, "not json").unwrap();
        assert!(History::load_from(&file).saved.is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }
}
