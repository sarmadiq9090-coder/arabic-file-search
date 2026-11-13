//! A last line of defence against runaway memory use: the window samples the
//! process's private bytes every half minute and this decides what to do.
//!
//! - Passing 1, 2 and 4 GB is logged once each, with what the program holds.
//! - Far above what the index explains (more than [`SHED_FACTOR`] times the
//!   expected size and more than [`SHED_FLOOR`]), the program sheds what it
//!   can rebuild: it stops watching the drives, drops the waiting changes and
//!   the caches that can be filled again, and walks the drives again later
//!   (paced). At most once per [`SHED_COOLDOWN`].
//!
//! The expected size is about [`ENTRY_BYTES`] per file or folder in the index
//! (measured: 195 to 240 bytes on the Windows heap, with room for paths
//! longer than the synthetic ones), [`DOCUMENT_BYTES`] per document known to
//! the content index (measured: about 260, plus about 30 for its word
//! index's map from path to document, 29 bytes for 300k documents, plus the
//! warm-up's own list at about 180) and [`BASE_BYTES`] for the program
//! itself. The word index's postings and vocabulary stay on disk (read per
//! search); while the warm-up runs it also holds up to 4 MB of word lists.
//!
//! The idle trim ([`idle_trim_due`]): after [`TRIM_IDLE`] with no search and
//! no input in the program's windows, once the background document reading
//! has ended, the window lets go of what is cheap to rebuild (thumbnail and
//! folder-size caches, spare capacity) and gives the heap's free memory back
//! to Windows. The index and the content index's map stay, so the next
//! search is as fast as ever. What it can give back is mostly what the
//! scan, the load and the warm-up left free in the heap: the warm-up's list
//! (about 180 of the 480 bytes per document) and the temporary copies of
//! the index. What stays is about 230 bytes per entry, 290 per document and
//! the program itself.
//!
//! Free of Win32, so it is tested on every platform.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub const ENTRY_BYTES: u64 = 300;
pub const DOCUMENT_BYTES: u64 = 480;
pub const BASE_BYTES: u64 = 200 * MB;
pub const SHED_FACTOR: u64 = 3;
pub const SHED_FLOOR: u64 = 1536 * MB;
pub const SHED_COOLDOWN: Duration = Duration::from_secs(30 * 60);
/// How often the window samples (at most).
pub const SAMPLE_EVERY: Duration = Duration::from_secs(30);
/// How often the `memory:` line is written.
pub const LINE_EVERY: Duration = Duration::from_secs(10 * 60);
const MB: u64 = 1024 * 1024;
const GB: u64 = 1024 * MB;
/// The sizes whose passing is logged, once each.
const WARN_AT: [u64; 3] = [GB, 2 * GB, 4 * GB];
/// How long without a search or input in the program's windows before the
/// idle trim.
pub const TRIM_IDLE: Duration = Duration::from_secs(120);
/// While the program stays idle, how often the trim may run again (the
/// drive watcher's batches allocate meanwhile).
pub const TRIM_AGAIN: Duration = Duration::from_secs(30 * 60);

/// When the program started counting activity, and the last activity in
/// milliseconds after it.
static ORIGIN: OnceLock<Instant> = OnceLock::new();
static LAST_ACTIVE_MS: AtomicU64 = AtomicU64::new(0);

fn origin() -> Instant {
    *ORIGIN.get_or_init(Instant::now)
}

/// Notes the user's activity now (any thread): input in one of the
/// program's windows, which is also how every search the user runs starts.
/// A search the program runs on its own (results refreshed after a change
/// on disk) is not activity: on a busy PC it would put the trim off for ever.
pub fn mark_active() {
    let ms = origin().elapsed().as_millis();
    LAST_ACTIVE_MS.fetch_max(u64::try_from(ms).unwrap_or(u64::MAX), Ordering::Relaxed);
}

/// The last activity ([`mark_active`]); the start if none yet.
pub fn last_active() -> Instant {
    origin() + Duration::from_millis(LAST_ACTIVE_MS.load(Ordering::Relaxed))
}

/// Whether the idle trim should run `now`: nothing was searched or typed
/// for [`TRIM_IDLE`] since `last_active`, the background work is `quiet`
/// (the content index's warm-up ended, no scan, load or search running),
/// and it has not run since that activity, or not for [`TRIM_AGAIN`].
pub fn idle_trim_due(now: Instant, last_active: Instant, quiet: bool, last_trim: Option<Instant>) -> bool {
    quiet
        && now.saturating_duration_since(last_active) >= TRIM_IDLE
        && last_trim.is_none_or(|t| t < last_active || now.saturating_duration_since(t) >= TRIM_AGAIN)
}

/// The log line after an idle trim.
pub fn trim_line(before: u64, after: u64, entries: usize, documents: usize) -> String {
    let freed = before.saturating_sub(after);
    format!(
        "memory: idle trim: private {} MB -> {} MB ({} MB given back; entries {entries}, documents {documents})",
        before / MB,
        after / MB,
        freed / MB
    )
}

/// One sample of the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    /// Private bytes (Windows: `PrivateUsage`).
    pub private: u64,
    /// Files and folders in the index.
    pub entries: usize,
    /// Documents known to the content index.
    pub documents: usize,
    /// Watcher changes waiting.
    pub queue: usize,
}

impl Sample {
    /// What the index and the content index explain.
    pub fn expected(&self) -> u64 {
        (self.entries as u64).saturating_mul(ENTRY_BYTES) + (self.documents as u64).saturating_mul(DOCUMENT_BYTES) + BASE_BYTES
    }

    /// The `memory:` line for log.txt.
    pub fn line(&self) -> String {
        format!(
            "memory: private {} MB, entries {}, documents {}, queue {} (expected about {} MB)",
            self.private / MB,
            self.entries,
            self.documents,
            self.queue,
            self.expected() / MB
        )
    }
}

/// What to do after a sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Verdict {
    /// The highest of 1, 2 and 4 GB passed for the first time, in GB.
    pub warn_gb: Option<u64>,
    /// Shed what can be rebuilt (see the module notes).
    pub shed: bool,
}

/// The guard's memory between samples.
#[derive(Debug, Default)]
pub struct Guard {
    /// How many of `WARN_AT` were logged already.
    warned: usize,
    last_shed: Option<Instant>,
}

impl Guard {
    pub fn decide(&mut self, s: &Sample, now: Instant) -> Verdict {
        let mut v = Verdict::default();
        while self.warned < WARN_AT.len() && s.private > WARN_AT[self.warned] {
            v.warn_gb = Some(WARN_AT[self.warned] / GB);
            self.warned += 1;
        }
        let far_above = s.private > s.expected().saturating_mul(SHED_FACTOR) && s.private > SHED_FLOOR;
        let rested = self.last_shed.is_none_or(|t| now.saturating_duration_since(t) >= SHED_COOLDOWN);
        if far_above && rested {
            v.shed = true;
            self.last_shed = Some(now);
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(private_mb: u64, entries: usize, documents: usize) -> Sample {
        Sample { private: private_mb * MB, entries, documents, queue: 0 }
    }

    #[test]
    fn the_expected_size_follows_the_index() {
        assert_eq!(sample(0, 0, 0).expected(), 200 * MB);
        // A big Unity/Steam PC: 4 million files, 3 million documents.
        let big = sample(0, 4_000_000, 3_000_000);
        assert_eq!(big.expected() / MB, (4_000_000 * 300 + 3_000_000 * 480) / MB + 200);
        assert!(big.expected() > 2 * GB);
        let line = Sample { private: 5161 * MB, entries: 4_000_000, documents: 3_000_000, queue: 12 }.line();
        assert_eq!(line, format!("memory: private 5161 MB, entries 4000000, documents 3000000, queue 12 (expected about {} MB)", big.expected() / MB));
    }

    #[test]
    fn passing_1_2_and_4_gb_is_said_once_each() {
        let mut g = Guard::default();
        let t = Instant::now();
        // A big index explains it all: warnings, but no shedding.
        let at = |mb| sample(mb, 10_000_000, 3_000_000);
        assert_eq!(g.decide(&at(900), t), Verdict::default());
        assert_eq!(g.decide(&at(1100), t).warn_gb, Some(1));
        assert_eq!(g.decide(&at(1200), t).warn_gb, None);
        // Straight from 1 to past 4: one line, for the highest.
        assert_eq!(g.decide(&at(4500), t), Verdict { warn_gb: Some(4), shed: false });
        assert_eq!(g.decide(&at(9000), t), Verdict::default());
        // Down and up again: not said twice.
        assert_eq!(g.decide(&at(500), t), Verdict::default());
        assert_eq!(g.decide(&at(2500), t), Verdict::default());
    }

    /// The field report: 5,161 MB with an index that explains far less.
    #[test]
    fn far_above_what_the_index_explains_sheds() {
        let mut g = Guard::default();
        let t = Instant::now();
        // 1 million files, 300k documents: about 620 MB expected.
        let v = g.decide(&sample(5161, 1_000_000, 300_000), t);
        assert!(v.shed);
        assert_eq!(v.warn_gb, Some(4));
    }

    #[test]
    fn a_small_index_never_sheds_below_the_floor() {
        let mut g = Guard::default();
        let t = Instant::now();
        // 10,000 files: 203 MB expected; 1.4 GB is 7x that, but under 1.5 GB.
        assert!(!g.decide(&sample(1400, 10_000, 0), t).shed);
        assert!(g.decide(&sample(1600, 10_000, 0), t).shed);
    }

    #[test]
    fn a_huge_index_sheds_only_far_above_its_size() {
        let mut g = Guard::default();
        let t = Instant::now();
        let s = |mb| sample(mb, 20_000_000, 5_000_000);
        let expected_mb = s(0).expected() / MB;
        assert!(!g.decide(&s(expected_mb * 2), t).shed, "twice: a save or a rescan, expected");
        assert!(!g.decide(&s(expected_mb * 3), t).shed, "exactly three times is not more");
        assert!(g.decide(&s(expected_mb * 3 + 10), t).shed);
    }

    #[test]
    fn sampling_and_lines_are_spaced() {
        assert_eq!((SAMPLE_EVERY.as_secs(), LINE_EVERY.as_secs()), (30, 600));
    }

    #[test]
    fn idle_trim_waits_for_two_quiet_minutes_and_runs_once() {
        let t = Instant::now();
        let idle = |s| t + Duration::from_secs(s);
        // Not before two minutes, nor while the background is busy.
        assert!(!idle_trim_due(idle(119), t, true, None));
        assert!(!idle_trim_due(idle(600), t, false, None));
        assert!(idle_trim_due(idle(120), t, true, None));
        // Once per idle spell…
        let trimmed = Some(idle(120));
        assert!(!idle_trim_due(idle(150), t, true, trimmed));
        assert!(!idle_trim_due(idle(120) + TRIM_AGAIN - Duration::from_secs(1), t, true, trimmed));
        // …again after half an hour of it, or after the next activity.
        assert!(idle_trim_due(idle(120) + TRIM_AGAIN, t, true, trimmed));
        let active = idle(200);
        assert!(!idle_trim_due(active + Duration::from_secs(60), active, true, trimmed));
        assert!(idle_trim_due(active + TRIM_IDLE, active, true, trimmed));
        // A clock that went back never trims early.
        assert!(!idle_trim_due(t, idle(10), true, None));
    }

    #[test]
    fn activity_is_noted_from_any_thread() {
        let before = last_active();
        std::thread::spawn(mark_active).join().unwrap();
        let after = last_active();
        assert!(after >= before);
        assert!(Instant::now().saturating_duration_since(after) < Duration::from_secs(60));
    }

    #[test]
    fn the_trim_line() {
        assert_eq!(
            trim_line(656 * MB, 340 * MB, 1_000_000, 300_000),
            "memory: idle trim: private 656 MB -> 340 MB (316 MB given back; entries 1000000, documents 300000)"
        );
        assert!(trim_line(300 * MB, 310 * MB, 1, 1).contains("(0 MB given back"));
    }

    #[test]
    fn shedding_rests_half_an_hour() {
        let mut g = Guard::default();
        let t = Instant::now();
        let bad = sample(6000, 100_000, 0);
        assert!(g.decide(&bad, t).shed);
        assert!(!g.decide(&bad, t + Duration::from_secs(60)).shed);
        assert!(!g.decide(&bad, t + SHED_COOLDOWN - Duration::from_secs(1)).shed);
        assert!(g.decide(&bad, t + SHED_COOLDOWN).shed);
        assert!(!g.decide(&bad, t + SHED_COOLDOWN + Duration::from_secs(1)).shed);
        // Normal again: nothing.
        assert!(!g.decide(&sample(300, 100_000, 0), t + SHED_COOLDOWN * 3).shed);
    }
}
