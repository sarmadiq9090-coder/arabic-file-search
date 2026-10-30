//! Bookkeeping for the drive watcher's follow-up work, free of Win32 so it
//! is tested on every platform: which paths a batch needs looked up on disk,
//! and how often a whole root may be walked again after lost events.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::index::{Change, skipped_path};

/// Size, mtime and folder flag of a path, or `None` when it is gone (see
/// `watch::stat`).
pub type Stat = Option<(u64, i64, bool)>;
/// Stats taken on the watcher thread, by path, so the UI thread does not
/// touch the disk while it applies a batch.
pub type Stats = HashMap<String, Stat>;

/// At most one walk of the same root this often; more requests wait.
pub const RESCAN_GAP: Duration = Duration::from_secs(10 * 60);

/// Every path whose entry `Index::apply_changes` will check on disk.
pub fn stat_paths(changes: &[Change]) -> impl Iterator<Item = &str> {
    changes
        .iter()
        .flat_map(|c| match c {
            Change::Added(p) | Change::Removed(p) | Change::Modified(p) => [Some(p.as_str()), None],
            Change::Renamed(a, b) => [Some(a.as_str()), Some(b.as_str())],
            Change::Rescan(_) => [None, None],
        })
        .flatten()
        .filter(|p| !skipped_path(p))
}

/// Looks `path` up in `stats`, or asks `fallback` when it was not taken
/// beforehand (paths moved along by a folder rename, for one).
pub fn lookup(stats: &Stats, path: &str, fallback: impl Fn(&str) -> Stat) -> Stat {
    match stats.get(path) {
        Some(s) => *s,
        None => fallback(path),
    }
}

/// Paces whole-root walks: a busy disk can overflow the watch buffer every
/// few seconds, and each overflow asks for a walk of the whole drive.
/// Requests are never dropped, only postponed, so the index catches up.
#[derive(Debug)]
pub struct RescanPace {
    gap: Duration,
    /// When each root's last walk started.
    last: HashMap<String, Instant>,
    /// Roots asked for while too soon or already being walked.
    deferred: Vec<String>,
}

impl Default for RescanPace {
    fn default() -> Self {
        RescanPace::new(RESCAN_GAP)
    }
}

impl RescanPace {
    pub fn new(gap: Duration) -> Self {
        RescanPace { gap, last: HashMap::new(), deferred: Vec::new() }
    }

    /// True when `root` should be walked now. `busy` means a walk of it is
    /// already running; then, or when the last one started too recently,
    /// the request is kept and comes back from [`due`](Self::due).
    pub fn request(&mut self, root: &str, now: Instant, busy: bool) -> bool {
        let recent = self.last.get(root).is_some_and(|&t| now.saturating_duration_since(t) < self.gap);
        if busy || recent {
            if !self.deferred.iter().any(|r| r == root) {
                self.deferred.push(root.to_string());
            }
            return false;
        }
        self.last.insert(root.to_string(), now);
        self.deferred.retain(|r| r != root);
        true
    }

    /// Postponed roots whose turn has come; the others stay postponed.
    pub fn due(&mut self, now: Instant, busy: impl Fn(&str) -> bool) -> Vec<String> {
        if self.deferred.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for root in std::mem::take(&mut self.deferred) {
            let b = busy(&root);
            if self.request(&root, now, b) {
                out.push(root);
            }
        }
        out
    }

    /// Forgets everything (the watcher was stopped).
    pub fn clear(&mut self) {
        self.deferred.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_to_stat() {
        let changes = vec![
            Change::Added(r"C:\a.txt".into()),
            Change::Renamed(r"C:\b".into(), r"C:\c".into()),
            Change::Rescan(r"C:\".into()),
            Change::Removed(r"C:\d".into()),
            Change::Modified(r"C:\e".into()),
        ];
        let got: Vec<&str> = stat_paths(&changes).collect();
        assert_eq!(got, [r"C:\a.txt", r"C:\b", r"C:\c", r"C:\d", r"C:\e"]);
    }

    #[test]
    fn lookup_prefers_the_watcher_stat() {
        let mut stats = Stats::new();
        stats.insert("x".into(), Some((1, 2, false)));
        stats.insert("gone".into(), None);
        let fallback = |_: &str| Some((9, 9, true));
        assert_eq!(lookup(&stats, "x", fallback), Some((1, 2, false)));
        assert_eq!(lookup(&stats, "gone", fallback), None, "a path known to be gone is not looked up again");
        assert_eq!(lookup(&stats, "other", fallback), Some((9, 9, true)));
    }

    #[test]
    fn rescans_are_paced_not_dropped() {
        let gap = Duration::from_secs(600);
        let mut pace = RescanPace::new(gap);
        let t0 = Instant::now();
        assert!(pace.request("C:\\", t0, false), "the first one runs at once");
        // Overflows while it runs and soon after: postponed, once.
        assert!(!pace.request("C:\\", t0 + Duration::from_secs(5), true));
        assert!(!pace.request("C:\\", t0 + Duration::from_secs(30), false));
        assert!(pace.due(t0 + Duration::from_secs(60), |_| false).is_empty());
        // Another drive is not held back.
        assert!(pace.request("D:\\", t0 + Duration::from_secs(60), false));
        // Once the gap has passed the postponed walk runs, but not while
        // a walk of that root is still going.
        assert!(pace.due(t0 + gap, |_| true).is_empty());
        assert_eq!(pace.due(t0 + gap + Duration::from_secs(1), |_| false), ["C:\\"]);
        assert!(pace.due(t0 + gap + Duration::from_secs(2), |_| false).is_empty(), "only once");
        // The next request counts from that walk.
        assert!(!pace.request("C:\\", t0 + gap + Duration::from_secs(10), false));
        assert_eq!(pace.due(t0 + gap * 2 + Duration::from_secs(1), |_| false), ["C:\\"]);
    }

    #[test]
    fn clear_drops_postponed() {
        let mut pace = RescanPace::new(Duration::from_secs(600));
        let t0 = Instant::now();
        assert!(pace.request("C:\\", t0, false));
        assert!(!pace.request("C:\\", t0, false));
        pace.clear();
        assert!(pace.due(t0 + Duration::from_secs(601), |_| false).is_empty());
    }
}
