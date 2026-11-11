//! Bookkeeping for the drive watcher's follow-up work, free of Win32 so it
//! is tested on every platform: which paths a batch needs looked up on disk,
//! and how often a whole root may be walked again after lost events.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::index::{Change, change_path, skipped_path};

/// Size, mtime and folder flag of a path, or `None` when it is gone (see
/// `watch::stat`).
pub type Stat = Option<(u64, i64, bool)>;
/// Stats taken on the watcher thread, by path, so the UI thread does not
/// touch the disk while it applies a batch.
pub type Stats = HashMap<String, Stat>;

/// At most one walk of the same root this often; more requests wait.
pub const RESCAN_GAP: Duration = Duration::from_secs(10 * 60);
/// A root that needs walking again and again (a disk written to non-stop
/// overflows the watch for hours) is walked at growing intervals, up to this.
pub const MAX_RESCAN_GAP: Duration = Duration::from_secs(2 * 60 * 60);

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
        // Spelled as `apply_changes` will ask for them.
        .filter_map(change_path)
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
///
/// The pause after a walk counts from its END ([`finished`](Self::finished))
/// and grows when the root keeps asking: a walk holds a second set of that
/// drive's entries in memory and reads the whole drive, so on a PC whose
/// disks never rest a fixed ten minutes meant walking every drive six times
/// an hour for ever. Each walk that follows the one before within three of
/// its pauses doubles the pause (10, 20, 40, 80 minutes, then two hours); a
/// root left alone for longer starts again at the short one.
#[derive(Debug)]
pub struct RescanPace {
    gap: Duration,
    max_gap: Duration,
    /// Each root's last walk.
    last: HashMap<String, Walked>,
    /// Roots asked for while too soon or already being walked.
    deferred: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct Walked {
    /// When it started, or ended once that is known.
    at: Instant,
    /// The pause before the next walk of this root.
    gap: Duration,
}

impl Default for RescanPace {
    fn default() -> Self {
        RescanPace::new(RESCAN_GAP)
    }
}

impl RescanPace {
    pub fn new(gap: Duration) -> Self {
        RescanPace { gap, max_gap: MAX_RESCAN_GAP.max(gap), last: HashMap::new(), deferred: Vec::new() }
    }

    /// True when `root` should be walked now. `busy` means a walk of it is
    /// already running; then, or when the last one started too recently,
    /// the request is kept and comes back from [`due`](Self::due).
    pub fn request(&mut self, root: &str, now: Instant, busy: bool) -> bool {
        let last = self.last.get(root).copied();
        let recent = last.is_some_and(|w| now.saturating_duration_since(w.at) < w.gap);
        if busy || recent {
            if !self.deferred.iter().any(|r| r == root) {
                self.deferred.push(root.to_string());
            }
            return false;
        }
        // Needed again soon after the last one: the next pause is longer.
        let gap = match last {
            Some(w) if now.saturating_duration_since(w.at) < w.gap.saturating_mul(3) => w.gap.saturating_mul(2).min(self.max_gap),
            _ => self.gap,
        };
        self.last.insert(root.to_string(), Walked { at: now, gap });
        self.deferred.retain(|r| r != root);
        true
    }

    /// The walk of `root` has ended: its pause counts from now. (Counted
    /// from the start, a walk slower than the pause, on a big busy drive,
    /// was followed by the next at once.)
    pub fn finished(&mut self, root: &str, now: Instant) {
        if let Some(w) = self.last.get_mut(root) {
            w.at = now;
        }
    }

    /// The pause now in force for `root` (for the log).
    pub fn pause_of(&self, root: &str) -> Duration {
        self.last.get(root).map_or(self.gap, |w| w.gap)
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

/// Watcher changes that cannot be applied yet may wait, but not without
/// limit: at most this many, and about this many bytes, per queue.
pub const MAX_QUEUED_CHANGES: usize = 200_000;
pub const MAX_QUEUED_BYTES: usize = 64 * 1024 * 1024;

/// What [`ChangeQueue::take`] hands over.
#[derive(Debug, Default)]
pub struct Taken {
    /// The changes in the order they came; a `Change::Rescan` for every
    /// root in `collapsed` comes last.
    pub changes: Vec<Change>,
    pub stats: Stats,
    /// Roots whose changes were dropped because the queue was full: each is
    /// walked again instead (also in `changes`, as `Change::Rescan`).
    pub collapsed: Vec<String>,
    /// Changes dropped that way.
    pub dropped: u64,
}

/// Watcher changes waiting until the index may change (a search inside
/// documents is running, the UI thread is busy, a folder is being walked).
///
/// On a PC that writes files non-stop the wait can be long and the changes
/// millions (about 150 to 300 bytes each). The changes are kept exactly as
/// they came, in order; a queue that grows past its limits is thrown away
/// and replaced by ONE walk of each root it was about (paced by
/// [`RescanPace`]), which costs time but a fixed amount of memory. Until it
/// is handed over, nothing more is kept for a root that is to be walked.
#[derive(Debug)]
pub struct ChangeQueue {
    changes: Vec<Change>,
    stats: Stats,
    /// About how much memory `changes` and `stats` take.
    bytes: usize,
    /// Watched roots, to name the one a dropped change belongs to.
    roots: Vec<String>,
    collapsed: Vec<String>,
    dropped: u64,
    max_changes: usize,
    max_bytes: usize,
}

impl Default for ChangeQueue {
    fn default() -> Self {
        ChangeQueue::new(MAX_QUEUED_CHANGES, MAX_QUEUED_BYTES)
    }
}

/// What a string on the heap costs besides its bytes (rounded up).
const STRING_COST: usize = 40;

impl ChangeQueue {
    pub fn new(max_changes: usize, max_bytes: usize) -> ChangeQueue {
        ChangeQueue {
            changes: Vec::new(),
            stats: Stats::new(),
            bytes: 0,
            roots: Vec::new(),
            collapsed: Vec::new(),
            dropped: 0,
            max_changes,
            max_bytes,
        }
    }

    /// A queue for changes under `roots` (the watched drives, or the one
    /// folder being walked).
    pub fn for_roots(roots: &[String]) -> ChangeQueue {
        ChangeQueue { roots: roots.to_vec(), ..ChangeQueue::default() }
    }

    /// The same queue for changes under `roots` (for tests with small limits).
    #[cfg(test)]
    pub fn under(mut self, roots: &[String]) -> ChangeQueue {
        self.roots = roots.to_vec();
        self
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.collapsed.is_empty()
    }

    /// Changes waiting (a collapsed root counts as one).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.changes.len() + self.collapsed.len()
    }

    /// About how many bytes the queue holds.
    #[cfg(test)]
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Adds a batch and the stats taken for it.
    pub fn extend(&mut self, changes: impl IntoIterator<Item = Change>, stats: impl IntoIterator<Item = (String, Stat)>) {
        for c in changes {
            self.push(c);
        }
        for (path, stat) in stats {
            self.push_stat(path, stat);
        }
    }

    /// Adds everything `other` held.
    pub fn absorb(&mut self, other: Taken) {
        self.dropped += other.dropped;
        // Its collapsed roots stay collapsed here (their rescans, last in
        // `changes`, are then left out and come back from `take`).
        for root in other.collapsed {
            self.collapse_root(root);
        }
        self.extend(other.changes, other.stats);
    }

    pub fn push(&mut self, change: Change) {
        let (rescan, path): (bool, &str) = match &change {
            Change::Added(p) | Change::Modified(p) | Change::Removed(p) => (false, p),
            Change::Rescan(p) => (true, p),
            Change::Renamed(_, new) => (false, new),
        };
        // A root that is going to be walked anyway needs nothing more.
        if self.under_collapsed(path) || matches!(&change, Change::Renamed(old, _) if self.under_collapsed(old)) {
            if let Change::Renamed(old, new) = &change {
                // One end of the move is in a root that is not walked.
                for p in [old, new] {
                    if !self.under_collapsed(p) {
                        let root = self.root_of(p);
                        self.collapse_root(root);
                    }
                }
            }
            self.dropped += u64::from(!rescan);
            return;
        }
        self.bytes += change_cost(&change);
        self.changes.push(change);
        if self.changes.len() > self.max_changes || self.bytes > self.max_bytes {
            self.collapse();
        }
    }

    fn push_stat(&mut self, path: String, stat: Stat) {
        if self.under_collapsed(&path) {
            return;
        }
        let cost = path.len() + STRING_COST + std::mem::size_of::<(String, Stat)>();
        if self.stats.insert(path, stat).is_none() {
            self.bytes += cost;
            if self.bytes > self.max_bytes {
                self.collapse();
            }
        }
    }

    fn under_collapsed(&self, path: &str) -> bool {
        self.collapsed.iter().any(|r| within(path, r))
    }

    fn root_of(&self, path: &str) -> String {
        root_of(&self.roots, path)
    }

    fn collapse_root(&mut self, root: String) {
        if !self.collapsed.contains(&root) {
            self.collapsed.push(root);
        }
    }

    /// Too much is waiting: forget it all, and walk the roots it was about.
    fn collapse(&mut self) {
        let changes = std::mem::take(&mut self.changes);
        self.dropped += changes.len() as u64;
        for c in &changes {
            let paths: [Option<&str>; 2] = match c {
                Change::Added(p) | Change::Modified(p) | Change::Removed(p) | Change::Rescan(p) => [Some(p), None],
                Change::Renamed(a, b) => [Some(a), Some(b)],
            };
            for p in paths.into_iter().flatten() {
                if !self.under_collapsed(p) {
                    let root = self.root_of(p);
                    self.collapse_root(root);
                }
            }
        }
        // Stats left over belong to changes already handed over or dropped.
        self.stats = Stats::new();
        self.bytes = 0;
    }

    /// Hands everything over and starts empty.
    pub fn take(&mut self) -> Taken {
        let mut changes = std::mem::take(&mut self.changes);
        let collapsed = std::mem::take(&mut self.collapsed);
        changes.extend(collapsed.iter().cloned().map(Change::Rescan));
        self.bytes = 0;
        Taken { changes, stats: std::mem::take(&mut self.stats), collapsed, dropped: std::mem::take(&mut self.dropped) }
    }

    /// Forgets everything (the watcher was stopped: a scan sees it all).
    pub fn clear(&mut self) {
        self.take();
    }
}

/// `path` is `root` or inside it (`root` with or without a trailing separator).
fn within(path: &str, root: &str) -> bool {
    path.strip_prefix(root).is_some_and(|rest| rest.is_empty() || root.ends_with(['\\', '/']) || rest.starts_with(['\\', '/']))
}

fn change_cost(c: &Change) -> usize {
    std::mem::size_of::<Change>()
        + match c {
            Change::Added(p) | Change::Modified(p) | Change::Removed(p) | Change::Rescan(p) => p.len() + STRING_COST,
            Change::Renamed(a, b) => a.len() + b.len() + 2 * STRING_COST,
        }
}

/// Folder walks running at once, at the most; more wait their turn.
pub const MAX_WALKS: usize = 4;
/// Folders that may wait for a walk; past that their drive is walked instead.
pub const MAX_WAITING_WALKS: usize = 500;

/// What to do with a folder that needs walking (see [`WalkQueue::ask`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Walk {
    /// Walk it now.
    Start,
    /// It waits; [`WalkQueue::next`] hands it out when a walk ends.
    Wait,
    /// Too many folders wait: none of them is walked one by one any more.
    /// Walk these roots instead (through [`RescanPace`]).
    Rescan(Vec<String>),
}

/// Folders waiting to be walked because their contents are unknown (they
/// appeared, or were moved in). Before 0.10.1 every such folder got an
/// operating-system thread of its own at once, without any limit: an import
/// or an unpacked archive that makes thousands of folders started thousands
/// of threads. Now a few walks run at a time, a bounded number of folders
/// wait, and past that the drive is walked once instead.
#[derive(Debug)]
pub struct WalkQueue {
    waiting: std::collections::VecDeque<String>,
    max_running: usize,
    max_waiting: usize,
}

impl Default for WalkQueue {
    fn default() -> Self {
        WalkQueue::new(MAX_WALKS, MAX_WAITING_WALKS)
    }
}

impl WalkQueue {
    pub fn new(max_running: usize, max_waiting: usize) -> WalkQueue {
        WalkQueue { waiting: std::collections::VecDeque::new(), max_running, max_waiting }
    }

    /// `dir` needs a walk while `running` walks are under way. `roots` are
    /// the watched roots (to name the one to walk when too many wait).
    pub fn ask(&mut self, dir: &str, running: usize, roots: &[String]) -> Walk {
        if self.waiting.iter().any(|w| w == dir) {
            return Walk::Wait;
        }
        if running < self.max_running && self.waiting.is_empty() {
            return Walk::Start;
        }
        if self.waiting.len() < self.max_waiting {
            self.waiting.push_back(dir.to_string());
            return Walk::Wait;
        }
        let mut walk: Vec<String> = Vec::new();
        for d in self.waiting.drain(..).chain([dir.to_string()]) {
            let root = root_of(roots, &d);
            if !walk.contains(&root) {
                walk.push(root);
            }
        }
        Walk::Rescan(walk)
    }

    /// The next folder to walk, if fewer than the limit are running.
    pub fn next(&mut self, running: usize) -> Option<String> {
        if running < self.max_running { self.waiting.pop_front() } else { None }
    }

    /// `root` is going to be walked as a whole: the folders in it need no
    /// walk of their own.
    pub fn forget_under(&mut self, root: &str) {
        self.waiting.retain(|d| !within(d, root));
    }

    pub fn clear(&mut self) {
        self.waiting.clear();
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.waiting.len()
    }
}

/// The watched root `path` is in; failing that, its drive or first folder.
fn root_of(roots: &[String], path: &str) -> String {
    if let Some(r) = roots.iter().filter(|r| within(path, r)).max_by_key(|r| r.len()) {
        return r.clone();
    }
    match path.find(['\\', '/']) {
        Some(i) => path[..=i].to_string(),
        None => path.to_string(),
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
        // Trailing separators are dropped and roots are never looked up,
        // matching what `apply_changes` asks for.
        let odd = vec![
            Change::Modified(r"C:\a\".into()),
            Change::Removed(r"C:\".into()),
            Change::Renamed("/".into(), "/home/u//".into()),
            Change::Added(String::new()),
        ];
        assert_eq!(stat_paths(&odd).collect::<Vec<_>>(), [r"C:\a", "/home/u"]);
    }

    /// Random requests at random times: never panics, a deferred root is
    /// listed once, and a root is never walked twice within the gap.
    #[test]
    fn random_pacing_keeps_the_gap() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let gap = Duration::from_secs(60);
        let mut pace = RescanPace::new(gap);
        let t0 = Instant::now();
        let roots = ["C:\\", "D:\\", r"\\server\share", ""];
        let mut walked: HashMap<&str, Instant> = HashMap::new();
        let mut now = t0;
        for _ in 0..5000 {
            now += Duration::from_secs(next(20));
            let root = roots[next(roots.len() as u64) as usize];
            let mut started = Vec::new();
            if next(2) == 0 {
                if pace.request(root, now, next(4) == 0) {
                    started.push(root.to_string());
                }
            } else {
                let busy = roots[next(roots.len() as u64) as usize];
                started = pace.due(now, |r| r == busy);
            }
            for r in &started {
                let r = roots.iter().find(|x| **x == r.as_str()).unwrap();
                if let Some(&t) = walked.get(r) {
                    assert!(now.duration_since(t) >= gap, "{r} walked again too soon");
                }
                walked.insert(r, now);
            }
            let mut d = pace.deferred.clone();
            d.sort();
            d.dedup();
            assert_eq!(d.len(), pace.deferred.len(), "a root deferred twice");
        }
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
        // The next request counts from that walk, and, coming so soon
        // after it, waits twice as long.
        assert!(!pace.request("C:\\", t0 + gap + Duration::from_secs(10), false));
        assert!(pace.due(t0 + gap * 2 + Duration::from_secs(2), |_| false).is_empty());
        assert_eq!(pace.due(t0 + gap * 3 + Duration::from_secs(1), |_| false), ["C:\\"]);
    }

    /// A drive that overflows the watch all day: one walk per pause, the
    /// pause counted from the walk's end and doubling up to two hours; a
    /// drive that then stays quiet starts again at ten minutes.
    #[test]
    fn a_root_that_keeps_asking_is_walked_less_and_less_often() {
        let base = Duration::from_secs(600);
        let mut pace = RescanPace::new(base);
        let walk_takes = Duration::from_secs(240);
        let mut now = Instant::now();
        let t0 = now;
        let mut starts = Vec::new();
        let mut running_until: Option<Instant> = None;
        // An overflow every 5 seconds for 12 hours.
        for _ in 0..(12 * 3600 / 5) {
            now += Duration::from_secs(5);
            if running_until.is_some_and(|t| now >= t) {
                running_until = None;
                pace.finished("C:\\", now);
            }
            let busy = running_until.is_some();
            let mut started = pace.request("C:\\", now, busy);
            started |= !pace.due(now, |_| busy).is_empty();
            if started {
                assert!(!busy, "two walks of one root at once");
                starts.push(now.duration_since(t0).as_secs());
                running_until = Some(now + walk_takes);
            }
        }
        // Pauses between the end of one walk and the start of the next.
        let pauses: Vec<u64> = starts.windows(2).map(|w| w[1] - w[0] - walk_takes.as_secs()).collect();
        assert_eq!(&pauses[..5], [600, 1200, 2400, 4800, 7200], "{pauses:?}");
        assert!(pauses[5..].iter().all(|&p| p == 7200), "{pauses:?}");
        assert!(starts.len() <= 9, "{} walks in 12 hours (72 with a fixed ten minutes)", starts.len());
        assert_eq!(pace.pause_of("C:\\"), MAX_RESCAN_GAP);
        // Quiet for a day, then one overflow: walked at once, short pause again.
        now += Duration::from_secs(24 * 3600);
        assert!(pace.request("C:\\", now, false));
        assert_eq!(pace.pause_of("C:\\"), base);
        // Another root was never slowed down.
        assert!(pace.request("D:\\", now, false));
        assert_eq!(pace.pause_of("D:\\"), base);
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

    /// The report behind 0.10.1: files rewritten for hours while the index
    /// cannot change. However many events come, the queue holds a bounded
    /// number of them and of bytes, and what it hands over in the end is
    /// exactly ONE rescan of the root.
    #[test]
    fn a_full_queue_becomes_exactly_one_rescan() {
        let paths: Vec<String> = (0..500).map(|i| format!(r"D:\render\shot_{:02}\frame_{i:05}.png", i % 20)).collect();
        let mut q = ChangeQueue::for_roots(&[r"D:\".to_string()]);
        let (mut most_bytes, mut most_len) = (0, 0);
        for i in 0..3_000_000u64 {
            let p = paths[(i * 7919 % 500) as usize].clone();
            let c = match i % 3 {
                0 => Change::Added(p.clone()),
                1 => Change::Modified(p.clone()),
                _ => Change::Removed(p.clone()),
            };
            q.extend([c], [(p, Some((i, i as i64, false)))]);
            most_bytes = most_bytes.max(q.bytes());
            most_len = most_len.max(q.len());
        }
        assert!(most_len <= MAX_QUEUED_CHANGES + 1, "{most_len} changes at the most");
        assert!(most_bytes <= MAX_QUEUED_BYTES + 4096, "{most_bytes} bytes at the most");
        assert!(q.bytes() < 1024, "{} bytes once collapsed", q.bytes());
        let taken = q.take();
        assert_eq!(taken.changes, [Change::Rescan(r"D:\".into())], "one rescan, whatever the number of events");
        assert_eq!(taken.collapsed, [r"D:\"]);
        assert_eq!(taken.dropped, 3_000_000);
        assert!(taken.stats.is_empty());
        assert!(q.is_empty() && q.bytes() == 0);
        // That one request is one walk, however often it is asked again.
        let mut pace = RescanPace::default();
        let now = Instant::now();
        let walks = taken.changes.iter().chain(&taken.changes).filter(|_| pace.request(r"D:\", now, false)).count();
        assert_eq!(walks, 1);
    }

    /// Below the limits nothing is touched: the changes come out as they
    /// went in, in order, repeats included (their order matters: a file
    /// removed and made again is not the file it was).
    #[test]
    fn below_the_limits_the_changes_pass_unchanged() {
        let mut q = ChangeQueue::default();
        let stream = vec![
            Change::Added(r"C:\a\x".into()),
            Change::Removed(r"C:\a\x".into()),
            Change::Added(r"C:\a\x".into()),
            Change::Modified(r"C:\a\x".into()),
            Change::Modified(r"C:\a\x".into()),
            Change::Renamed(r"C:\a".into(), r"C:\b".into()),
            Change::Rescan(r"C:\".into()),
            Change::Modified(r"C:\c".into()),
        ];
        q.extend(stream.clone(), [(r"C:\a\x".to_string(), None), (r"C:\a\x".to_string(), Some((3, 4, false)))]);
        assert_eq!(q.len(), stream.len());
        let taken = q.take();
        assert_eq!(taken.changes, stream);
        assert!(taken.collapsed.is_empty() && taken.dropped == 0);
        // Of the stats for one path, the latest.
        assert_eq!(taken.stats.get(r"C:\a\x"), Some(&Some((3, 4, false))));
    }

    /// Millions of different files (a download, an import) on one drive:
    /// that drive is walked; another drive's changes keep queueing.
    #[test]
    fn a_flood_on_one_drive_leaves_the_others_alone() {
        let roots = [r"C:\".to_string(), r"D:\".to_string(), r"E:\".to_string()];
        let mut q = ChangeQueue::for_roots(&roots);
        q.push(Change::Modified(r"D:\notes\todo.txt".into()));
        for i in 0..400_000u64 {
            let p = format!(r"C:\steam\downloading\{:03}\chunk_{i:08}.bin", i % 512);
            q.extend([Change::Added(p.clone())], [(p, Some((i, 1, false)))]);
        }
        // The two roots the dropped changes were about; nothing more is kept for them.
        assert_eq!(q.len(), 2);
        q.push(Change::Modified(r"E:\a.txt".into()));
        q.push(Change::Modified(r"D:\notes\todo.txt".into()));
        let taken = q.take();
        assert_eq!(taken.collapsed, [r"D:\", r"C:\"]);
        assert_eq!(taken.changes, [Change::Modified(r"E:\a.txt".into()), Change::Rescan(r"D:\".into()), Change::Rescan(r"C:\".into())]);
        assert_eq!(taken.dropped, 400_002);
    }

    #[test]
    fn small_limits_and_merging_queues() {
        // The count limit.
        let mut q = ChangeQueue::new(3, usize::MAX);
        for i in 0..3 {
            q.push(Change::Added(format!(r"C:\a\{i}")));
        }
        assert_eq!(q.len(), 3);
        q.push(Change::Added(r"C:\a\3".into()));
        assert_eq!(q.take().changes, [Change::Rescan(r"C:\".into())]);
        // The byte limit, reached by the stats.
        let mut q = ChangeQueue::new(usize::MAX, 400);
        q.extend([Change::Added(r"/home/u/a".into())], [("/home/u/a".to_string(), None)]);
        assert!(q.take().collapsed.is_empty());
        q.extend((0..4).map(|i| Change::Added(format!("/home/u/{i}"))), (0..4).map(|i| (format!("/home/u/{i}"), None)));
        assert_eq!(q.take().collapsed, ["/"]);

        // A folder being walked: its own queue collapses to that folder, and
        // a sibling with the same prefix is not taken for it.
        let mut q = ChangeQueue::new(2, usize::MAX);
        q.roots = vec![r"C:\proj\Library".to_string()];
        for i in 0..3 {
            q.push(Change::Added(format!(r"C:\proj\Library\{i}")));
        }
        q.push(Change::Added(r"C:\proj\Library\more".into()));
        q.push(Change::Added(r"C:\proj\LibraryOld\x".into()));
        let taken = q.take();
        assert_eq!(taken.changes, [Change::Added(r"C:\proj\LibraryOld\x".into()), Change::Rescan(r"C:\proj\Library".into())]);

        // The inbox hands its collapsed root on; later changes there stay out.
        let mut inbox = ChangeQueue::new(1, usize::MAX);
        inbox.roots = vec![r"D:\".to_string()];
        inbox.push(Change::Added(r"D:\a".into()));
        inbox.push(Change::Added(r"D:\b".into()));
        let mut state = ChangeQueue::for_roots(&[r"D:\".to_string()]);
        state.push(Change::Modified(r"D:\before".into()));
        state.absorb(inbox.take());
        state.push(Change::Modified(r"D:\after".into()));
        let taken = state.take();
        assert_eq!(taken.changes, [Change::Modified(r"D:\before".into()), Change::Rescan(r"D:\".into())]);
        assert_eq!(taken.dropped, 3);

        // A move out of a collapsed root: the other end's root is walked too.
        let mut q = ChangeQueue::new(1, usize::MAX);
        q.roots = vec![r"C:\".to_string(), r"D:\".to_string()];
        q.push(Change::Added(r"C:\a".into()));
        q.push(Change::Added(r"C:\b".into()));
        q.push(Change::Renamed(r"C:\x".into(), r"D:\x".into()));
        assert_eq!(q.take().collapsed, [r"C:\", r"D:\"]);

        // The watcher was stopped: everything is forgotten, limits and roots kept.
        q.push(Change::Added(r"C:\a".into()));
        q.push(Change::Added(r"C:\b".into()));
        q.clear();
        assert!(q.is_empty() && q.take().changes.is_empty());
        q.push(Change::Added(r"D:\c".into()));
        q.push(Change::Added(r"D:\d".into()));
        assert_eq!(q.take().collapsed, [r"D:\"]);
    }

    /// An import that makes ten thousand folders in a minute: a few walks at
    /// a time, a bounded number waiting, and past that one walk of the drive
    /// instead of ten thousand threads.
    #[test]
    fn folder_walks_are_limited_and_queued() {
        let roots = [r"C:\".to_string(), r"D:\".to_string()];
        let mut q = WalkQueue::default();
        let mut running: Vec<String> = Vec::new();
        let (mut started, mut most_running, mut most_waiting, mut drive_walks) = (0, 0, 0, Vec::new());
        for i in 0..10_000 {
            let dir = format!(r"D:\proj\Library\Artifacts\{i:05}");
            match q.ask(&dir, running.len(), &roots) {
                Walk::Start => {
                    running.push(dir);
                    started += 1;
                }
                Walk::Wait => {}
                Walk::Rescan(walk) => drive_walks.push(walk),
            }
            most_running = most_running.max(running.len());
            most_waiting = most_waiting.max(q.len());
            // Now and then a walk ends and the next waiting folder starts.
            if i % 7 == 0 && !running.is_empty() {
                running.remove(0);
                while let Some(next) = q.next(running.len()) {
                    assert!(!running.contains(&next));
                    running.push(next);
                    started += 1;
                }
            }
        }
        assert_eq!(most_running, MAX_WALKS);
        assert_eq!(most_waiting, MAX_WAITING_WALKS);
        assert!(started < 3_000, "{started} folder walks started");
        // Each overflow names the drive once, and empties the queue.
        assert!(!drive_walks.is_empty());
        assert!(drive_walks.iter().all(|w| w == &[r"D:\".to_string()]), "{drive_walks:?}");

        // In order, without repeats; a folder asked twice waits once.
        let mut q = WalkQueue::new(1, 3);
        assert_eq!(q.ask(r"C:\a", 0, &roots), Walk::Start);
        assert_eq!(q.ask(r"C:\b", 1, &roots), Walk::Wait);
        assert_eq!(q.ask(r"C:\b", 1, &roots), Walk::Wait);
        assert_eq!(q.ask(r"D:\c", 1, &roots), Walk::Wait);
        // Nothing jumps the queue, even when a place is free.
        assert_eq!(q.ask(r"C:\d", 0, &roots), Walk::Wait);
        assert_eq!(q.len(), 3);
        assert_eq!(q.next(1), None);
        assert_eq!(q.next(0).as_deref(), Some(r"C:\b"));
        // A drive about to be walked as a whole takes its folders with it.
        q.forget_under(r"C:\");
        assert_eq!(q.next(0).as_deref(), Some(r"D:\c"));
        assert_eq!(q.next(0), None);
        // Too many waiting, on two drives: both are walked, once each.
        for d in [r"C:\1", r"D:\2", r"C:\3"] {
            assert_eq!(q.ask(d, 1, &roots), Walk::Wait);
        }
        assert_eq!(q.ask(r"C:\4", 1, &roots), Walk::Rescan(vec![r"C:\".to_string(), r"D:\".to_string()]));
        assert_eq!(q.len(), 0);
        q.ask(r"C:\5", 1, &roots);
        q.clear();
        assert_eq!(q.next(0), None);
    }
}
