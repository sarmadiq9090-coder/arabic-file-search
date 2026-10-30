//! Keeping the index current while the program runs: batches from the drive
//! watcher are applied to the index, and folders that appear with unknown
//! contents are walked again in the background.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use super::App;
use super::startup::{background_priority, log};
use crate::index::{self, Change, Entry, Index, ScanProgress};
use crate::watch::{self, Watcher};

/// A folder being walked because the watcher could not say what is inside.
pub(super) struct SubtreeScan {
    dir: String,
    progress: Arc<ScanProgress>,
    result: Arc<Mutex<Option<Vec<Entry>>>>,
    /// Changes under `dir` applied while the walk ran; the walk may have
    /// missed them, so they are applied again on top of its result.
    replay: Vec<Change>,
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
            if let Ok(mut q) = inbox.lock() {
                q.extend(batch);
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
        drop(st);
        // Joins the watcher threads, which may be handing over a batch.
        drop(watcher);
        if let Ok(mut q) = self.watch_inbox.lock() {
            q.clear();
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
        self.state.borrow_mut().changes.extend(batch);
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
        if !self.index_free() {
            return;
        }
        self.finish_subtree_scans();
        let changes = std::mem::take(&mut self.state.borrow_mut().changes);
        if changes.is_empty() {
            return;
        }
        for job in &mut self.state.borrow_mut().subtree_scans {
            job.replay.extend(changes.iter().filter(|c| c.touches(&job.dir)).cloned());
        }
        let mut scan = Vec::new();
        self.edit_index(
            |idx| {
                let applied = idx.apply_changes(&changes, watch::stat);
                scan = applied.scan;
                applied.changed > 0
            },
            true,
        );
        for dir in scan {
            self.start_subtree_scan(dir);
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
        st.subtree_scans.push(SubtreeScan { dir, progress, result, replay: Vec::new() });
    }

    fn finish_subtree_scans(&self) {
        let mut done = Vec::new();
        self.state.borrow_mut().subtree_scans.retain_mut(|j| match j.result.lock().ok().and_then(|mut r| r.take()) {
            Some(found) => {
                done.push((std::mem::take(&mut j.dir), found, std::mem::take(&mut j.replay)));
                false
            }
            None => true,
        });
        if done.is_empty() {
            return;
        }
        self.edit_index(
            |idx| {
                for (dir, found, replay) in done {
                    // The folder may have gone again while it was walked.
                    if dir.ends_with('\\') || idx.find(&dir).is_some() {
                        idx.replace_subtree(&dir, found);
                        // Folders found here were just walked; no new scans.
                        idx.apply_changes(&replay, watch::stat);
                    }
                }
                true
            },
            true,
        );
    }
}
