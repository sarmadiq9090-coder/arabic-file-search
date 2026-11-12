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
//! the content index (measured: about 260, plus the warm-up's own list at
//! about 180) and [`BASE_BYTES`] for the program itself.
//!
//! Free of Win32, so it is tested on every platform.

use std::time::{Duration, Instant};

pub const ENTRY_BYTES: u64 = 300;
pub const DOCUMENT_BYTES: u64 = 450;
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
        assert_eq!(big.expected() / MB, (4_000_000 * 300 + 3_000_000 * 450) / MB + 200);
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
