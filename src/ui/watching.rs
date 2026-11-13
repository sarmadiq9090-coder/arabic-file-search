//! Keeping the index current while the program runs: batches from the drive
//! watcher are applied to the index, and folders that appear with unknown
//! contents are walked again in the background.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::files::SAVES;
use super::startup::{background_priority, log};
use super::{App, State};
use crate::index::{self, Change, Entry, Index, ScanProgress};
use crate::index_save::RateLog;
use crate::watch::{self, Watcher};
use crate::watch_pace::{self, ChangeQueue, RescanPace, Walk, WalkQueue};

/// While something else holds the index, each edit copies all of it; then
/// watcher changes are gathered for this long between edits.
const COPY_GAP: Duration = Duration::from_secs(10);

/// While a whole drive is being walked again (or its result waits), an edit
/// that would copy the index makes a third set of that drive's entries: it
/// waits for the other reader (usually a save being written) instead. Not
/// longer than this, so that a walk's result is always put in in the end.
const SHARED_WAIT: Duration = Duration::from_secs(120);

/// How often the drives are looked at, for drives plugged in or taken out.
const DRIVE_LOOK: Duration = Duration::from_secs(5);

/// What the watcher threads hand over: changes and the stats they took.
/// Like every queue of changes here it has a size limit, past which it asks
/// for a walk of the drive instead (`watch_pace::ChangeQueue`): before
/// 0.10.1 these queues grew without limit while the index could not change.
pub(super) type Inbox = ChangeQueue;

/// Watcher state kept with the index (`State::live`).
#[derive(Default)]
pub(super) struct Live {
    rescans: RescanPace,
    /// When an edit last had to copy the whole index (see `files::edit_index`).
    pub(super) last_copy: Option<Instant>,
    /// Since when work has waited for another reader of the index to finish.
    waiting_since: Option<Instant>,
    /// Folders waiting to be walked: a few walks run at a time.
    walks: WalkQueue,
    /// When the drives were last looked at, and the look under way (on its
    /// own thread: an ailing USB disk must not stop the window).
    drives_looked: Option<Instant>,
    drive_look: Option<Arc<Mutex<Option<crate::drives::Look>>>>,
    /// Drives plugged in while watching whose first walk has not ended;
    /// the walk's result makes them roots of the index.
    new_roots: Vec<String>,
}

/// A folder being walked because the watcher could not say what is inside.
pub(super) struct SubtreeScan {
    dir: String,
    progress: Arc<ScanProgress>,
    /// `Some(None)` when the walk failed (a panic, logged): the job is
    /// dropped and the index left as it was.
    result: Arc<Mutex<Option<Option<Vec<Entry>>>>>,
    /// Changes under `dir` applied while the walk ran; the walk may have
    /// missed them, so they are applied again on top of its result. Past its
    /// size limit they are dropped and `dir` is walked once more.
    replay: ChangeQueue,
}

impl SubtreeScan {
    /// A walk of a whole watched root (`C:\`), as after lost events.
    fn whole_root(&self) -> bool {
        self.dir.ends_with('\\')
    }

    /// The walk has ended and its result waits to be put into the index.
    fn ready(&self) -> bool {
        self.result.lock().is_ok_and(|r| r.is_some())
    }
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
        st.changes = ChangeQueue::for_roots(&roots);
        if let Ok(mut q) = self.watch_inbox.lock() {
            *q = ChangeQueue::for_roots(&roots);
        }
        let inbox = self.watch_inbox.clone();
        let notice = self.watch_notice.sender();
        st.watcher = Some(Watcher::start(&roots, move |batch| {
            // Looked up here, on the watcher thread, and not while the UI
            // applies the batch. Anything that changes after this is in a
            // later batch with its own stats.
            let stats: Vec<_> = watch_pace::stat_paths(&batch).map(|p| (p.to_string(), watch::stat(p))).collect();
            if let Ok(mut q) = inbox.lock() {
                q.extend(batch, stats);
            }
            notice.notice();
        }));
        drop(st);
        log(&format!("watch: {}", roots.join(" ")));
    }

    /// True when the index shows every file as it is on disk, give or take
    /// the watcher's last moments: the drives are watched, and no change,
    /// folder walk or rescan waits. A search inside documents then trusts
    /// the index's sizes and times (`search::search_content`); otherwise it
    /// asks the disk about every document its word index rules out.
    pub(super) fn index_current(&self) -> bool {
        let st = self.state.borrow();
        let current = st.watcher.is_some()
            && st.scan.is_none()
            && st.loading.is_none()
            && st.changes.is_empty()
            && st.subtree_scans.is_empty()
            && st.live.walks.is_empty()
            && !st.live.rescans.has_deferred();
        drop(st);
        current && self.watch_inbox.lock().is_ok_and(|q| q.is_empty())
    }

    /// Stops the watcher and drops whatever it reported but was not applied.
    pub(super) fn stop_watching(&self) {
        let mut st = self.state.borrow_mut();
        let watcher = st.watcher.take();
        for job in st.subtree_scans.drain(..) {
            job.progress.cancel.store(true, Ordering::Relaxed);
        }
        st.changes.clear();
        st.live.rescans.clear();
        st.live.walks.clear();
        st.live.new_roots.clear();
        st.live.waiting_since = None;
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
        let batch = self.watch_inbox.lock().map(|mut q| q.take()).unwrap_or_default();
        self.state.borrow_mut().changes.absorb(batch);
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
    ///
    /// At most two sets of entries are in memory at any time: when another
    /// reader holds the index (a save being written, mostly), an edit has to
    /// copy it, so edits are spaced out (`COPY_GAP`), and while a whole
    /// drive's second set already exists (its walk running, or done and
    /// waiting) they wait for that reader to finish (`SHARED_WAIT` at the
    /// most, so the walk's result always gets in). Before 0.10.1 a walk's
    /// result was put in at once, copying the index if it was shared: three
    /// sets of a drive's entries.
    pub(super) fn apply_watched(&self) {
        self.check_drives();
        self.start_due_rescans();
        self.start_waiting_walks();
        if !self.index_free() {
            return;
        }
        {
            let mut st = self.state.borrow_mut();
            if st.changes.is_empty() && !st.subtree_scans.iter().any(SubtreeScan::ready) {
                st.live.waiting_since = None;
                return;
            }
        }
        // A list of the new look that points into this index keeps its own
        // rows from here on, so it is no reason to copy.
        self.webui_release_index();
        {
            let mut st = self.state.borrow_mut();
            if self.index_shared(&st) {
                // Copying a big index for every one-second batch would keep
                // the UI thread busy; gather them instead (the tick brings
                // us back).
                if st.live.last_copy.is_some_and(|t| t.elapsed() < COPY_GAP) {
                    return;
                }
                let now = Instant::now();
                let since = *st.live.waiting_since.get_or_insert(now);
                if st.subtree_scans.iter().any(SubtreeScan::whole_root) && now.duration_since(since) < SHARED_WAIT {
                    return;
                }
            }
            st.live.waiting_since = None;
        }
        self.finish_subtree_scans();
        // Putting a walk's result in may have started a new search of the
        // list, which reads the index: the changes wait for it (the timer
        // brings us back) rather than copy the index under it.
        if !self.index_free() {
            return;
        }
        let mut st = self.state.borrow_mut();
        if st.changes.is_empty() {
            return;
        }
        let taken = st.changes.take();
        let (rescans, changes): (Vec<Change>, Vec<Change>) = taken.changes.into_iter().partition(|c| matches!(c, Change::Rescan(_)));
        let stats = taken.stats;
        for job in st.subtree_scans.iter_mut() {
            let touching: Vec<Change> = changes.iter().filter(|c| c.touches(&job.dir)).cloned().collect();
            let their_stats: Vec<_> =
                watch_pace::stat_paths(&touching).filter_map(|p| stats.get(p).map(|s| (p.to_string(), *s))).collect();
            job.replay.extend(touching, their_stats);
        }
        drop(st);
        if !taken.collapsed.is_empty() {
            note_dropped(taken.dropped, &taken.collapsed);
        }
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
            self.walk_folder(dir);
        }
    }

    /// A folder appeared with unknown contents: walk it, now or when one of
    /// the few walks allowed at once has ended. When too many wait, their
    /// drive is walked instead (paced like every whole-drive walk).
    fn walk_folder(&self, dir: String) {
        let mut st = self.state.borrow_mut();
        if st.subtree_scans.iter().any(|j| j.dir == dir) {
            return;
        }
        let running = st.subtree_scans.iter().filter(|j| !j.ready()).count();
        let State { index, live, .. } = &mut *st;
        let what = live.walks.ask(&dir, running, &index.roots);
        drop(st);
        match what {
            Walk::Start => self.start_subtree_scan(dir),
            Walk::Wait => {}
            Walk::Rescan(roots) => {
                note_too_many_walks(&roots);
                for root in roots {
                    self.request_rescan(root);
                }
            }
        }
    }

    /// Starts waiting folder walks while fewer than the limit are running.
    fn start_waiting_walks(&self) {
        loop {
            let mut st = self.state.borrow_mut();
            let running = st.subtree_scans.iter().filter(|j| !j.ready()).count();
            let Some(dir) = st.live.walks.next(running) else { return };
            drop(st);
            self.start_subtree_scan(dir);
        }
    }

    /// Every few seconds: a drive plugged in since (a USB stick, an external
    /// disk, a new partition) is watched and walked alone, without a new
    /// full scan, and becomes a root of the index when its walk ends; the
    /// results on drives that are not connected are marked (`crate::drives`).
    /// Network and cloud drives are never added.
    fn check_drives(&self) {
        let mut st = self.state.borrow_mut();
        let Some(look) = st.live.drive_look.take() else {
            if st.live.drives_looked.is_none_or(|t| t.elapsed() >= DRIVE_LOOK) {
                st.live.drives_looked = Some(Instant::now());
                let slot = Arc::new(Mutex::new(None));
                let s = slot.clone();
                if crate::crash_log::spawn_named("drive-look", move || {
                    let look = crate::drives::look();
                    if let Ok(mut s) = s.lock() {
                        *s = Some(look);
                    }
                })
                .is_some()
                {
                    st.live.drive_look = Some(slot);
                }
            }
            return;
        };
        let Some(found) = look.lock().ok().and_then(|mut l| l.take()) else {
            st.live.drive_look = Some(look); // Still looking.
            return;
        };
        let mut roots = st.index.roots.clone();
        roots.extend(st.live.new_roots.iter().cloned());
        let plan = crate::drives::plan(&roots, found);
        // Added while watching only, to a finished index (a full scan sees
        // every drive there is).
        let add = st.watcher.is_some() && !st.index.roots.is_empty() && st.scan.is_none() && st.loading.is_none();
        let add = if add { plan.add } else { Vec::new() };
        for root in &add {
            if let Some(w) = st.watcher.as_mut() {
                w.add_root(root);
            }
            st.changes.add_root(root);
            st.live.new_roots.push(root.clone());
        }
        drop(st);
        if crate::drives::set_absent(plan.absent) {
            let gone: Vec<String> = roots.iter().filter(|r| crate::drives::is_absent(r)).cloned().collect();
            log(&format!("drives: not connected now: {}", if gone.is_empty() { "none".to_string() } else { gone.join(" ") }));
            self.refresh_list();
        }
        if add.is_empty() {
            return;
        }
        if let Ok(mut q) = self.watch_inbox.lock() {
            for root in &add {
                q.add_root(root);
            }
        }
        for root in add {
            log(&format!("drives: {root} plugged in; walking it"));
            self.request_rescan(root);
        }
    }

    /// A walk of a whole root after lost events, paced by `RescanPace`.
    pub(super) fn request_rescan(&self, root: String) {
        let mut st = self.state.borrow_mut();
        let busy = st.subtree_scans.iter().any(|j| j.dir == root) || walk_must_wait();
        // The walk of the whole root covers the folders in it that wait.
        st.live.walks.forget_under(&root);
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
        let wait = walk_must_wait();
        let State { subtree_scans, live, .. } = &mut *st;
        let due = live.rescans.due(Instant::now(), |r| wait || subtree_scans.iter().any(|j| j.dir == r));
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
        let started_thread = crate::crash_log::spawn_named("subtree-scan", move || {
            background_priority();
            let started = std::time::Instant::now();
            let found = crate::crash_log::catch(|| Index::scan_subtree(&d, &p));
            // Only a whole-drive walk is worth a line in the log.
            if let Some(found) = &found
                && d.ends_with('\\')
            {
                log(&format!("rescan {d}: {} entries in {} ms", found.len(), started.elapsed().as_millis()));
            }
            if let Ok(mut slot) = r.lock() {
                *slot = Some(found);
            }
        });
        if started_thread.is_none() {
            return;
        }
        let replay = ChangeQueue::for_roots(std::slice::from_ref(&dir));
        st.subtree_scans.push(SubtreeScan { dir, progress, result, replay });
    }

    fn finish_subtree_scans(&self) {
        let mut done = Vec::new();
        {
            let now = Instant::now();
            let mut st = self.state.borrow_mut();
            let State { subtree_scans, live, .. } = &mut *st;
            subtree_scans.retain_mut(|j| {
                let Some(found) = j.result.lock().ok().and_then(|mut r| r.take()) else { return true };
                // The pause before the next walk of this folder counts from now.
                live.rescans.finished(&j.dir, now);
                // A new drive's walk ended: in the index now, or (it failed)
                // to be added again at the next look.
                live.new_roots.retain(|r| *r != j.dir);
                if j.whole_root() {
                    log(&format!("rescan {}: done; the next one waits at least {} min", j.dir, live.rescans.pause_of(&j.dir).as_secs() / 60));
                }
                // (`None`: the walk failed. Forget it; a later change can ask again.)
                if let Some(found) = found {
                    done.push((std::mem::take(&mut j.dir), found, j.replay.take()));
                }
                false
            });
        }
        if done.is_empty() {
            return;
        }
        // Folders whose own queue of changes overflowed while they were
        // walked: what was dropped is found by walking them once more.
        let mut again = Vec::new();
        self.edit_index(
            |idx| {
                for (dir, found, replay) in done {
                    // The folder may have gone again while it was walked.
                    if dir.ends_with('\\') || idx.find(&dir).is_some() {
                        idx.replace_subtree(&dir, found);
                        // A drive plugged in while watching: now one of the roots.
                        idx.add_root(&dir);
                        // Folders found here were just walked; no new scans.
                        idx.apply_changes(&replay.changes, |p| watch_pace::lookup(&replay.stats, p, watch::stat));
                    }
                    if !replay.collapsed.is_empty() {
                        note_dropped(replay.dropped, &replay.collapsed);
                        again.extend(replay.collapsed);
                    }
                }
                true
            },
            true,
        );
        for dir in again {
            self.request_rescan(dir);
        }
    }
}

/// Too many folders waited for a walk: a line in the log, at most one every
/// ten minutes.
fn note_too_many_walks(roots: &[String]) {
    static LINES: Mutex<RateLog> = Mutex::new(RateLog::new(Duration::from_secs(10 * 60)));
    let more = LINES.lock().unwrap_or_else(|e| e.into_inner()).due(Instant::now());
    if let Some(more) = more {
        log(&format!(
            "watch: too many new folders waiting to be walked; walking {} instead{}",
            roots.join(" "),
            if more > 0 { format!("; {more} more times since the last line") } else { String::new() }
        ));
    }
}

/// A new walk waits while a save is being written: the save holds one set
/// of entries, and the window may already have had to copy the index because
/// of it; the walk would add a third. (`RescanPace` keeps the request and
/// the timer asks again.)
fn walk_must_wait() -> bool {
    SAVES.running()
}

/// Queued changes were dropped because there were too many: a line in the
/// log, at most one every ten minutes (the log file is short).
fn note_dropped(dropped: u64, roots: &[String]) {
    static LINES: Mutex<RateLog> = Mutex::new(RateLog::new(crate::index_save::LINE_GAP));
    let more = LINES.lock().unwrap_or_else(|e| e.into_inner()).due(Instant::now());
    if let Some(more) = more {
        log(&format!(
            "watch queue: collapsed to rescan ({dropped} events; walking {} again){}",
            roots.join(" "),
            if more > 0 { format!(" [{more} more since the last such line]") } else { String::new() }
        ));
    }
}
