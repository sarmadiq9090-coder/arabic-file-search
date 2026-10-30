//! Keeping the index current while the program runs: batches from the drive
//! watcher are applied to the index, and folders that appear with unknown
//! contents are walked again in the background.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{App, State};
use super::startup::{background_priority, log};
use crate::index::{self, Change, Entry, Index, ScanProgress};
use crate::watch::{self, Watcher};
use crate::watch_pace::{self, RescanPace, Stats};

/// While something else holds the index, each edit copies all of it; then
/// watcher changes are gathered for this long between edits.
const COPY_GAP: Duration = Duration::from_secs(10);

/// What the watcher threads hand over: changes and the stats they took.
#[derive(Default)]
pub(super) struct Inbox {
    changes: Vec<Change>,
    stats: Stats,
}

/// Watcher state kept with the index (`State::live`).
#[derive(Default)]
pub(super) struct Live {
    /// Stats for the paths in `State::changes`; a later batch's replace an earlier one's.
    stats: Stats,
    rescans: RescanPace,
    /// When an edit last had to copy the whole index (see `files::edit_index`).
    pub(super) last_copy: Option<Instant>,
}

/// A folder being walked because the watcher could not say what is inside.
pub(super) struct SubtreeScan {
    dir: String,
    progress: Arc<ScanProgress>,
    result: Arc<Mutex<Option<Vec<Entry>>>>,
    /// Changes under `dir` applied while the walk ran; the walk may have
    /// missed them, so they are applied again on top of its result.
    replay: Vec<Change>,
    replay_stats: Stats,
}

impl App {
    /// Starts watching the indexed drives, unless turned off or a full scan
    /// (which sees everything anyway) is running.
    pub(super) fn start_watching(&self) {
        let mut st = self.state.borrow_mut();
        if !st.settings.auto_index || st.watcher.is_some() || st.loading.is_some() || st.scan.is_some() {
            return;
        }
        let roots = if st.index.roots.is_empty() { index::list_drives() } else { st.index.roots.clone() };
        let inbox = self.watch_inbox.clone();
        let notice = self.watch_notice.sender();
        st.watcher = Some(Watcher::start(&roots, move |batch| {
            // Looked up here, on the watcher thread, and not while the UI
            // applies the batch. Anything that changes after this is in a
            // later batch with its own stats.
            let stats: Vec<_> = watch_pace::stat_paths(&batch).map(|p| (p.to_string(), watch::stat(p))).collect();
            if let Ok(mut q) = inbox.lock() {
                q.changes.extend(batch);
                q.stats.extend(stats);
            }
            notice.notice();
        }));
        drop(st);
        log(&format!("watch: {}", roots.join(" ")));
    }

    /// Stops the watcher and drops whatever it reported but was not applied.
    pub(super) fn stop_watching(&self) {
        let mut st = self.state.borrow_mut();
        let watcher = st.watcher.take();
        for job in st.subtree_scans.drain(..) {
            job.progress.cancel.store(true, Ordering::Relaxed);
        }
        st.changes.clear();
        st.live.stats.clear();
        st.live.rescans.clear();
        drop(st);
        // Joins the watcher threads, which may be handing over a batch.
        drop(watcher);
        if let Ok(mut q) = self.watch_inbox.lock() {
            *q = Inbox::default();
        }
    }

    pub(super) fn toggle_auto_index(&self) {
        let on = !self.menu_auto_index.checked();
        self.menu_auto_index.set_checked(on);
        self.state.borrow_mut().settings.auto_index = on;
        if on {
            self.start_watching();
        } else {
            self.stop_watching();
        }
    }

    pub(super) fn on_watch_notice(&self) {
        let batch = self.watch_inbox.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
        let mut st = self.state.borrow_mut();
        st.changes.extend(batch.changes);
        st.live.stats.extend(batch.stats);
        drop(st);
        self.apply_watched();
    }

    /// The index may only change when nothing holds entry indices into it:
    /// document-search hits, a filename search under way, a rename being typed.
    fn index_free(&self) -> bool {
        let st = self.state.borrow();
        st.content.is_none() && st.scan.is_none() && st.loading.is_none() && !st.name_running && !self.renaming.get()
    }

    /// Applies queued changes and finished folder walks. Also called on every
    /// tick, so work queued while the index was busy runs once it is free.
    pub(super) fn apply_watched(&self) {
        self.start_due_rescans();
        if !self.index_free() {
            return;
        }
        self.finish_subtree_scans();
        let mut st = self.state.borrow_mut();
        if st.changes.is_empty() {
            return;
        }
        // Copying a big index for every one-second batch would keep the UI
        // thread busy; gather them instead (the tick brings us back).
        if self.index_shared(&st) && st.live.last_copy.is_some_and(|t| t.elapsed() < COPY_GAP) {
            return;
        }
        let (rescans, changes): (Vec<Change>, Vec<Change>) =
            std::mem::take(&mut st.changes).into_iter().partition(|c| matches!(c, Change::Rescan(_)));
        let stats = std::mem::take(&mut st.live.stats);
        let State { subtree_scans, .. } = &mut *st;
        for job in subtree_scans.iter_mut() {
            let before = job.replay.len();
            job.replay.extend(changes.iter().filter(|c| c.touches(&job.dir)).cloned());
            for p in watch_pace::stat_paths(&job.replay[before..]) {
                if let Some(s) = stats.get(p) {
                    job.replay_stats.insert(p.to_string(), *s);
                }
            }
        }
        drop(st);
        for c in rescans {
            if let Change::Rescan(root) = c {
                self.request_rescan(root);
            }
        }
        if changes.is_empty() {
            return;
        }
        let mut scan = Vec::new();
        self.edit_index(
            |idx| {
                let applied = idx.apply_changes(&changes, |p| watch_pace::lookup(&stats, p, watch::stat));
                scan = applied.scan;
                applied.changed > 0
            },
            true,
        );
        for dir in scan {
            self.start_subtree_scan(dir);
        }
    }

    /// A walk of a whole root after lost events, paced by `RescanPace`.
    fn request_rescan(&self, root: String) {
        let mut st = self.state.borrow_mut();
        let busy = st.subtree_scans.iter().any(|j| j.dir == root);
        let now = st.live.rescans.request(&root, Instant::now(), busy);
        drop(st);
        // Otherwise `start_due_rescans` starts it later.
        if now {
            self.start_subtree_scan(root);
        }
    }

    /// Starts the postponed root walks whose turn has come.
    fn start_due_rescans(&self) {
        let mut st = self.state.borrow_mut();
        let State { subtree_scans, live, .. } = &mut *st;
        let due = live.rescans.due(Instant::now(), |r| subtree_scans.iter().any(|j| j.dir == r));
        drop(st);
        for root in due {
            self.start_subtree_scan(root);
        }
    }

    fn start_subtree_scan(&self, dir: String) {
        let mut st = self.state.borrow_mut();
        if st.subtree_scans.iter().any(|j| j.dir == dir) {
            return;
        }
        let progress = Arc::new(ScanProgress::default());
        let result = Arc::new(Mutex::new(None));
        let (d, p, r) = (dir.clone(), progress.clone(), result.clone());
        std::thread::spawn(move || {
            background_priority();
            let started = std::time::Instant::now();
            let found = Index::scan_subtree(&d, &p);
            // Only a whole-drive walk is worth a line in the log.
            if d.ends_with('\\') {
                log(&format!("rescan {d}: {} entries in {} ms", found.len(), started.elapsed().as_millis()));
            }
            if let Ok(mut slot) = r.lock() {
                *slot = Some(found);
            }
        });
        st.subtree_scans.push(SubtreeScan { dir, progress, result, replay: Vec::new(), replay_stats: Stats::new() });
    }

    fn finish_subtree_scans(&self) {
        let mut done = Vec::new();
        self.state.borrow_mut().subtree_scans.retain_mut(|j| match j.result.lock().ok().and_then(|mut r| r.take()) {
            Some(found) => {
                done.push((
                    std::mem::take(&mut j.dir),
                    found,
                    std::mem::take(&mut j.replay),
                    std::mem::take(&mut j.replay_stats),
                ));
                false
            }
            None => true,
        });
        if done.is_empty() {
            return;
        }
        self.edit_index(
            |idx| {
                for (dir, found, replay, stats) in done {
                    // The folder may have gone again while it was walked.
                    if dir.ends_with('\\') || idx.find(&dir).is_some() {
                        idx.replace_subtree(&dir, found);
                        // Folders found here were just walked; no new scans.
                        idx.apply_changes(&replay, |p| watch_pace::lookup(&stats, p, watch::stat));
                    }
                }
                true
            },
            true,
        );
    }
}
