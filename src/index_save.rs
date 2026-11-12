//! Writing the index to disk in the background, and changing it in memory,
//! without ever keeping more than two copies of it.
//!
//! On a PC where files change all the time (rendering, a game engine, a big
//! download) the index is edited every second and saved every half minute.
//! A save reads one snapshot of the index from start to end, so an edit that
//! arrives meanwhile has to copy the index. That is the one extra copy there
//! may be: the snapshot being written, and the index the window uses.
//!
//! Until 0.10.0 every waiting save held a snapshot of its own while it queued
//! behind a slow one, so on a busy or slow disk the copies piled up without
//! limit (several gigabytes at 0% CPU). Here:
//! - one thread saves, and only one save runs at a time;
//! - what waits to be saved is a single slot holding the *current* index (the
//!   window gives it up before each edit, so it never forces a copy), and it
//!   is taken only at the moment its save starts;
//! - a slow save makes the next one wait longer (`next_gap`), so the program
//!   does not spend its time rewriting a big index on a struggling disk;
//! - a save that fails is tried again later, and whatever happens on the
//!   saving thread (an error, a panic) the queue goes on working;
//! - the UI thread never writes and never waits for the write lock: leaving
//!   the program waits a bounded time for the last save (`flush_within`), and
//!   an unfinished one only leaves `index.bin` a little older (it is written
//!   to a temp file and renamed, so it is never cut off).
//!
//! A save that never happens (the program is killed, the PC loses power,
//! leaving gives up on a slow disk) loses the changes that were only in
//! memory, and nothing else would ever notice: the file on disk is whole,
//! just older. So while changes are unsaved a marker file exists
//! (`index.dirty`, see [`marker_path`]); a start that finds it scans the
//! drives again, and only that scan's own save removes it.
//!
//! Free of Win32, so it is tested on every platform.

use std::io;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::index::Index;

/// A save waits at least this many times as long as the last one took before
/// the next one starts: the second copy of the index exists for at most
/// about a fifth of the time, however slow the disk.
pub const GAP_FACTOR: u32 = 4;
/// ... but never longer than this.
pub const MAX_GAP: Duration = Duration::from_secs(10 * 60);
/// A failed save is tried again after this long, then after twice as long
/// each time it fails again, up to [`MAX_GAP`].
pub const RETRY: Duration = Duration::from_secs(30);

/// How long after `failures` failed saves in a row the next try starts.
pub fn retry_in(base: Duration, failures: u32) -> Duration {
    base.saturating_mul(1u32 << failures.saturating_sub(1).min(10)).min(MAX_GAP)
}

/// The test hook `BAHTH_TEST_SAVE_DELAY_MS`: every index save waits this
/// long before it writes, so the Windows memory test can make saves slower
/// than the changes. `None` (no delay) when unset, empty, not a number or
/// zero; at most ten minutes.
pub const TEST_SAVE_DELAY_VAR: &str = "BAHTH_TEST_SAVE_DELAY_MS";

pub fn test_save_delay(value: Option<&str>) -> Option<Duration> {
    let ms: u64 = value?.trim().parse().ok()?;
    (ms > 0).then(|| Duration::from_millis(ms.min(600_000)))
}

/// How long after a save that took `took` the next one may start.
pub fn next_gap(took: Duration, factor: u32) -> Duration {
    took.saturating_mul(factor).min(MAX_GAP)
}

/// What `flush_within` found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flush {
    /// Nothing was waiting and nothing was being written.
    Nothing,
    /// Everything is on disk.
    Saved,
    /// The time ran out with a save still running or waiting: the file on
    /// disk stays as the last finished save left it.
    TimedOut,
}

/// What one edit did (see [`edit`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edited {
    pub changed: bool,
    /// The whole index had to be copied, because something still read it.
    pub copied: bool,
}

type Writer = dyn Fn(&Index) -> io::Result<()> + Send + Sync;
type Logger = dyn Fn(&str) + Send + Sync;
/// Creates (`true`) or removes (`false`) the marker file.
type Marker = dyn Fn(bool) + Send + Sync;

/// The marker file next to the index file `index`: it exists while the
/// index in memory has changes that are not on disk yet.
pub fn marker_path(index: &std::path::Path) -> std::path::PathBuf {
    index.with_extension("dirty")
}

struct Inner {
    /// The newest index waiting to be written, and its number.
    pending: Option<(Arc<Index>, u64)>,
    /// When the waiting save should start.
    due: Option<Instant>,
    /// The earliest start allowed after the last save (`next_gap`).
    not_before: Option<Instant>,
    /// The saving thread exists.
    worker: bool,
    /// Writes under way (the saving thread's, or a `save_now`).
    running: usize,
    /// Numbers given so far, and the newest one written: an older index is
    /// never written over a newer one.
    next_seq: u64,
    written_seq: u64,
    /// The program is leaving: no more waiting between saves.
    leaving: bool,
    /// Indexes numbered up to this were given up (`discard`, or a newer
    /// one written by `save_now`): a failed save of one is not retried.
    dropped_seq: u64,
    /// Saves that failed since the last one that worked.
    failures: u32,
    /// The marker file was created for the changes now unsaved.
    marked: bool,
    /// The last session ended with unsaved changes: the marker stays until
    /// a scan's own save (`save_now`) has put a complete index on disk.
    scan_owed: bool,
}

/// The single place index saves go through.
pub struct SaveQueue {
    inner: Mutex<Inner>,
    wake: Condvar,
    /// One write at a time, whoever asks.
    turn: Mutex<()>,
    /// One change of the marker at a time, so the file always says what
    /// `Inner::marked` says.
    marking: Mutex<()>,
    writer: Box<Writer>,
    marker: Box<Marker>,
    log: Box<Logger>,
    /// Run first on the saving thread (lowers its priority in the program).
    thread_init: fn(),
    gap_factor: u32,
    retry: Duration,
    /// Lines for log.txt, at most one per kind per [`LINE_GAP`] (the file is
    /// short and starts over when full); failures always get one.
    lines: Mutex<Lines>,
}

/// At most one line of each kind this often (failures excepted).
pub const LINE_GAP: Duration = Duration::from_secs(60);

/// The rate limits of the save queue's lines.
struct Lines {
    started: RateLog,
    done: RateLog,
    skipped: RateLog,
}

impl SaveQueue {
    pub fn new(
        writer: Box<Writer>,
        marker: Box<Marker>,
        log: Box<Logger>,
        thread_init: fn(),
        gap_factor: u32,
        retry: Duration,
    ) -> Arc<SaveQueue> {
        Arc::new(SaveQueue {
            inner: Mutex::new(Inner {
                pending: None,
                due: None,
                not_before: None,
                worker: false,
                running: 0,
                next_seq: 0,
                written_seq: 0,
                leaving: false,
                dropped_seq: 0,
                failures: 0,
                marked: false,
                scan_owed: false,
            }),
            wake: Condvar::new(),
            turn: Mutex::new(()),
            marking: Mutex::new(()),
            writer,
            marker,
            log,
            thread_init,
            gap_factor,
            retry,
            lines: Mutex::new(Lines { started: RateLog::new(LINE_GAP), done: RateLog::new(LINE_GAP), skipped: RateLog::new(LINE_GAP) }),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `index` (the window's current one) is to be written within `delay`.
    /// It replaces whatever was waiting; a save already due sooner stays due.
    pub fn set_pending(self: &Arc<Self>, index: Arc<Index>, delay: Duration) {
        let mut inner = self.lock();
        inner.next_seq += 1;
        let seq = inner.next_seq;
        inner.pending = Some((index, seq));
        let at = Instant::now() + delay;
        inner.due = Some(inner.due.map_or(at, |d| d.min(at)));
        self.ensure_worker(&mut inner);
        drop(inner);
        self.wake.notify_all();
    }

    /// Gives up the waiting index so it can be changed in place. True when
    /// one was waiting: the caller hands the changed index back with
    /// `set_pending` (the time its save is due is kept).
    pub fn take_pending(&self) -> bool {
        self.lock().pending.take().is_some()
    }

    /// Forgets the waiting index: something newer (a full scan) is about to
    /// be written instead.
    pub fn discard(&self) {
        let mut inner = self.lock();
        inner.pending = None;
        inner.due = None;
        inner.dropped_seq = inner.next_seq;
    }

    /// The marker file was there at start: the last session's changes were
    /// not all saved. It now stays until a scan has been saved (`save_now`).
    pub fn scan_owed(&self) {
        self.lock().scan_owed = true;
    }

    /// Creates the marker file once there are unsaved changes (called on
    /// the saving thread, never on the UI thread).
    fn mark_unsaved(&self) {
        let _one = self.marking.lock().unwrap_or_else(|e| e.into_inner());
        let mut inner = self.lock();
        if inner.pending.is_some() && !inner.marked {
            inner.marked = true;
            drop(inner);
            (self.marker)(true);
        }
    }

    /// Removes the marker file after a write, when nothing newer waits and
    /// no scan is owed (`after_scan`: this write was a scan's).
    fn mark_saved(&self, after_scan: bool) {
        let _one = self.marking.lock().unwrap_or_else(|e| e.into_inner());
        let mut inner = self.lock();
        if after_scan {
            inner.scan_owed = false;
        }
        if inner.pending.is_none() && !inner.scan_owed {
            inner.marked = false;
            drop(inner);
            (self.marker)(false);
        }
    }

    /// Whether `index` is the one waiting to be written.
    pub fn is_pending(&self, index: &Arc<Index>) -> bool {
        self.lock().pending.as_ref().is_some_and(|(p, _)| Arc::ptr_eq(p, index))
    }

    /// A write is under way (it holds a snapshot of the index).
    pub fn running(&self) -> bool {
        self.lock().running > 0
    }

    /// Writes `index` now, on the calling thread (a finished scan, off the
    /// UI thread). It is newer than anything waiting, which is forgotten.
    pub fn save_now(&self, index: &Index) -> io::Result<()> {
        let seq = {
            let mut inner = self.lock();
            inner.next_seq += 1;
            inner.pending = None;
            inner.due = None;
            inner.running += 1;
            // Everything numbered before is older than this index.
            inner.dropped_seq = inner.next_seq - 1;
            inner.next_seq
        };
        // Counted down again however the write ends.
        let _running = Running(self);
        if self.write(index, seq)? {
            self.mark_saved(true);
        }
        Ok(())
    }

    /// Waits up to `budget` for the write under way to end. True when none
    /// is running any more.
    pub fn wait_idle(&self, budget: Duration) -> bool {
        let deadline = Instant::now() + budget;
        let mut inner = self.lock();
        while inner.running > 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            inner = self.wake.wait_timeout(inner, left).unwrap_or_else(|e| e.into_inner()).0;
        }
        true
    }

    /// The program is leaving: writes what is waiting at once, and waits up
    /// to `budget` for it and for a write already under way. Never longer:
    /// an unfinished save leaves the file on disk as it was.
    pub fn flush_within(self: &Arc<Self>, budget: Duration) -> Flush {
        let deadline = Instant::now() + budget;
        let mut inner = self.lock();
        if inner.pending.is_none() && inner.running == 0 {
            return Flush::Nothing;
        }
        inner.leaving = true;
        if inner.pending.is_some() {
            inner.due = Some(Instant::now());
            self.ensure_worker(&mut inner);
        }
        self.wake.notify_all();
        while inner.pending.is_some() || inner.running > 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                drop(inner);
                (self.log)(&format!("index save: not finished within {} ms on exit; the file on disk stays as it was", budget.as_millis()));
                return Flush::TimedOut;
            }
            inner = self.wake.wait_timeout(inner, left).unwrap_or_else(|e| e.into_inner()).0;
        }
        Flush::Saved
    }

    fn ensure_worker(self: &Arc<Self>, inner: &mut Inner) {
        if inner.worker {
            return;
        }
        let queue = self.clone();
        inner.worker = crate::crash_log::spawn_named("index-save", move || {
            (queue.thread_init)();
            queue.work();
        })
        .is_some();
    }

    /// The saving thread: sleeps until a save is due, takes the waiting
    /// index only then, writes it, and ends when nothing is left.
    fn work(&self) {
        // Should this thread end in any other way than by finding nothing
        // left (a panic in the logger, say), the queue must not go on
        // believing it has a saving thread: the next request starts a new one.
        let mut alive = Worker { queue: self, ended: false };
        let mut inner = self.lock();
        loop {
            let Some(due) = inner.due.filter(|_| inner.pending.is_some()) else {
                // Nothing to write (the window may be in the middle of an
                // edit; `set_pending` starts a new thread then).
                inner.due = None;
                inner.worker = false;
                alive.ended = true;
                return;
            };
            if !inner.marked {
                // First of all, on disk: there are changes only memory has.
                drop(inner);
                self.mark_unsaved();
                inner = self.lock();
                continue;
            }
            let start = if inner.leaving { due } else { inner.not_before.map_or(due, |n| n.max(due)) };
            let wait = start.saturating_duration_since(Instant::now());
            if !wait.is_zero() {
                inner = self.wake.wait_timeout(inner, wait).unwrap_or_else(|e| e.into_inner()).0;
                continue;
            }
            let Some((index, seq)) = inner.pending.take() else { continue };
            inner.due = None;
            inner.running += 1;
            drop(inner);

            let started = Instant::now();
            let entries = index.entries.len();
            // Counted down again however the write ends, before anyone is
            // told that the save is over.
            let running = Running(self);
            self.line(|l| &mut l.started, || format!("index save: started gen {seq}"));
            let result = self.write(&index, seq);
            let took = started.elapsed();
            match result {
                Ok(written) => {
                    // The snapshot goes first.
                    drop(index);
                    if written {
                        let gap = next_gap(took, self.gap_factor).as_secs();
                        self.line(|l| &mut l.done, || {
                            format!("index save: done gen {seq} in {} ms ({entries} entries; the next one waits {gap} s)", took.as_millis())
                        });
                    }
                    // (Also when skipped: a newer index is on disk then.)
                    self.mark_saved(false);
                    let mut inner = self.lock();
                    inner.failures = 0;
                    inner.not_before = Some(Instant::now() + next_gap(took, self.gap_factor));
                }
                Err(e) => {
                    // Tried again later: with a newer index if one came
                    // meanwhile, otherwise with this one (then still the
                    // window's own, so keeping it costs no copy).
                    let mut inner = self.lock();
                    inner.failures += 1;
                    let again = retry_in(self.retry, inner.failures);
                    // Not when it was given up meanwhile (a scan took over),
                    // or something newer is on disk: putting it back would
                    // keep an old snapshot alive beside the new index.
                    let stale = seq <= inner.dropped_seq || seq < inner.written_seq;
                    if inner.pending.is_none() && !stale {
                        inner.pending = Some((index, seq));
                    }
                    let at = Instant::now() + again;
                    inner.due = Some(inner.due.map_or(at, |d| d.max(at)));
                    inner.not_before = Some(at);
                    drop(inner);
                    (self.log)(&format!(
                        "index save: failed gen {seq} after {} ms: {e}; {}",
                        took.as_millis(),
                        if stale { "not retried (a newer index took its place)".to_string() } else { format!("trying again in {} s", again.as_secs()) }
                    ));
                }
            }
            drop(running);
            inner = self.lock();
        }
    }

    /// Writes `index` unless something newer is already on disk. `Ok(false)`
    /// when it was skipped.
    fn write(&self, index: &Index, seq: u64) -> io::Result<bool> {
        let _turn = self.turn.lock().unwrap_or_else(|e| e.into_inner());
        let newest = self.lock().written_seq;
        if newest > seq {
            self.line(|l| &mut l.skipped, || format!("index save: skipped gen {seq} (gen {newest} is already on disk)"));
            return Ok(false);
        }
        // A panic in the writer (reported by the panic hook) is a failed save.
        crate::crash_log::catch(|| (self.writer)(index)).unwrap_or_else(|| Err(io::Error::other("the save panicked")))?;
        let mut inner = self.lock();
        inner.written_seq = inner.written_seq.max(seq);
        Ok(true)
    }
}

impl SaveQueue {
    /// Writes a line of the kind `which` picks, unless one was written less
    /// than [`LINE_GAP`] ago (then counted, and the count added to the next).
    fn line(&self, which: impl FnOnce(&mut Lines) -> &mut RateLog, text: impl FnOnce() -> String) {
        let due = which(&mut self.lines.lock().unwrap_or_else(|e| e.into_inner())).due(Instant::now());
        if let Some(more) = due {
            let text = text();
            (self.log)(&if more > 0 { format!("{text} [{more} more since the last such line]") } else { text });
        }
    }
}

/// One write under way: counted down when dropped, on every path.
struct Running<'a>(&'a SaveQueue);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        let mut inner = self.0.lock();
        inner.running = inner.running.saturating_sub(1);
        drop(inner);
        self.0.wake.notify_all();
    }
}

/// The saving thread, for as long as it lives.
struct Worker<'a> {
    queue: &'a SaveQueue,
    /// It ended the regular way and said so itself, under the lock.
    ended: bool,
}

impl Drop for Worker<'_> {
    fn drop(&mut self) {
        if !self.ended {
            self.queue.lock().worker = false;
            self.queue.wake.notify_all();
        }
    }
}

/// Takes the index out of `slot` to change it: without copying when `slot`
/// is its only owner, as a copy when something else still holds it. True
/// when it was copied; `slot` then still holds the old snapshot.
pub fn take_for_edit(slot: &mut Arc<Index>) -> (Index, bool) {
    match Arc::get_mut(slot) {
        Some(index) => (std::mem::take(index), false),
        None => (Index { entries: slot.entries.clone(), roots: slot.roots.clone(), built_at: slot.built_at }, true),
    }
}

/// Whether changing `index` now would have to copy it: something other than
/// its owner, `own` more references of the caller's (its result rows) and
/// the save waiting in `queue` holds it.
pub fn shared(queue: &SaveQueue, index: &Arc<Index>, own: usize) -> bool {
    Arc::strong_count(index) > 1 + own + usize::from(queue.is_pending(index))
}

/// Changes the window's index: gives up the save slot's reference, takes
/// the index out (copying only when something else still reads it), applies
/// `change` and puts the result back. With `save_in`, a changed index (or
/// one whose save was waiting) is queued to be written within that time.
///
/// The caller drops its own other references (result rows) first.
pub fn edit(
    queue: &Arc<SaveQueue>,
    index: &mut Arc<Index>,
    change: impl FnOnce(&mut Index) -> bool,
    save_in: Option<Duration>,
) -> Edited {
    edit_keeping_old(queue, index, change, save_in).0
}

/// [`edit`], also handing back the snapshot left behind when the index had
/// to be copied (whoever made the copy necessary still holds it). The tests
/// use it to count the copies alive.
fn edit_keeping_old(
    queue: &Arc<SaveQueue>,
    index: &mut Arc<Index>,
    change: impl FnOnce(&mut Index) -> bool,
    save_in: Option<Duration>,
) -> (Edited, Option<Arc<Index>>) {
    let had_pending = queue.take_pending();
    let (mut owned, copied) = take_for_edit(index);
    let changed = change(&mut owned);
    let old = std::mem::replace(index, Arc::new(owned));
    match save_in {
        Some(delay) if changed || had_pending => queue.set_pending(index.clone(), delay),
        _ => {}
    }
    (Edited { changed, copied }, copied.then_some(old))
}

/// Lets a line through at most once per `gap`, counting the ones held back,
/// so an event that repeats for hours does not push everything else out of
/// the short log file.
#[derive(Debug)]
pub struct RateLog {
    gap: Duration,
    last: Option<Instant>,
    held: u32,
}

impl RateLog {
    pub const fn new(gap: Duration) -> RateLog {
        RateLog { gap, last: None, held: 0 }
    }

    /// `Some(n)` when a line may be written now, `n` being how many were
    /// held back since the last one; `None` to hold this one back.
    pub fn due(&mut self, now: Instant) -> Option<u32> {
        if self.last.is_some_and(|t| now.saturating_duration_since(t) < self.gap) {
            self.held += 1;
            return None;
        }
        self.last = Some(now);
        Some(std::mem::take(&mut self.held))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{Change, Entry};
    use std::sync::Weak;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn index_of(n: usize) -> Index {
        let mut entries: Vec<Entry> = (0..n)
            .map(|i| Entry {
                path: format!(r"D:\render\shot_{:03}\frame_{i:06}.png", i % 50).into(),
                size: 1,
                mtime: 0,
                is_dir: false,
                norm_name: format!("frame_{i:06}.png").into(),
            })
            .collect();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Index { entries, ..Index::default() }
    }

    /// A "disk" the test opens and closes: a save blocks in the writer until
    /// the test lets one through.
    #[derive(Default)]
    struct Disk {
        /// Saves the test has allowed to finish.
        allowed: Mutex<usize>,
        gate: Condvar,
        started: AtomicUsize,
        finished: AtomicUsize,
        /// Writes inside the writer right now, and the most there ever were.
        inside: AtomicUsize,
        most_inside: AtomicUsize,
        /// `size` of the first entry of the last index written, and of
        /// every index written, in order.
        last_written: AtomicUsize,
        written: Mutex<Vec<usize>>,
    }

    impl Disk {
        fn write(&self, index: &Index) -> io::Result<()> {
            let turn = self.started.fetch_add(1, Ordering::SeqCst) + 1;
            let inside = self.inside.fetch_add(1, Ordering::SeqCst) + 1;
            self.most_inside.fetch_max(inside, Ordering::SeqCst);
            let mut allowed = self.allowed.lock().unwrap();
            while *allowed < turn {
                allowed = self.gate.wait(allowed).unwrap();
            }
            self.last_written.store(index.entries[0].size as usize, Ordering::SeqCst);
            self.written.lock().unwrap().push(index.entries[0].size as usize);
            self.inside.fetch_sub(1, Ordering::SeqCst);
            self.finished.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn allow(&self, n: usize) {
            *self.allowed.lock().unwrap() += n;
            self.gate.notify_all();
        }

        fn open(&self) {
            self.allow(usize::MAX / 2);
        }
    }

    fn until(what: &str, mut cond: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !cond() {
            assert!(Instant::now() < deadline, "timed out waiting for: {what}");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn queue_with(writer: Box<Writer>) -> Arc<SaveQueue> {
        SaveQueue::new(writer, Box::new(|_| {}), Box::new(|_| {}), || {}, 0, Duration::ZERO)
    }

    fn queue_on(disk: &Arc<Disk>) -> Arc<SaveQueue> {
        let d = disk.clone();
        queue_with(Box::new(move |i| d.write(i)))
    }

    /// Nothing waits and nothing is being written.
    fn at_rest(queue: &SaveQueue) -> bool {
        let inner = queue.lock();
        inner.pending.is_none() && inner.running == 0
    }

    /// One watcher batch, as `ui/watching.rs` applies it: while something
    /// else holds the index and the last whole copy is recent, wait.
    struct Window {
        index: Arc<Index>,
        rows_index: Arc<Index>,
        last_copy: Option<usize>,
        copies: usize,
        /// Snapshots left behind by a copy; alive while someone holds them.
        stale: Vec<Weak<Index>>,
    }

    const COPY_GAP_STEPS: usize = 10;

    impl Window {
        fn new(index: Index) -> Window {
            let index = Arc::new(index);
            Window { rows_index: index.clone(), index, last_copy: None, copies: 0, stale: Vec::new() }
        }

        /// False when the batch has to wait (the copy pacing).
        fn batch(&mut self, queue: &Arc<SaveQueue>, step: usize) -> bool {
            let own = usize::from(Arc::ptr_eq(&self.rows_index, &self.index));
            if shared(queue, &self.index, own) && self.last_copy.is_some_and(|t| step - t < COPY_GAP_STEPS) {
                return false;
            }
            self.rows_index = Arc::default();
            let (done, old) = edit_keeping_old(
                queue,
                &mut self.index,
                |idx| {
                    // A render frame rewritten: its size changes.
                    idx.apply_changes(&[Change::Modified(idx.entries[0].path.to_string())], |_| Some((step as u64 + 2, 1, false)))
                        .changed
                        > 0
                },
                Some(Duration::ZERO),
            );
            assert!(done.changed);
            if let Some(old) = old {
                self.last_copy = Some(step);
                self.copies += 1;
                self.stale.push(Arc::downgrade(&old));
            }
            self.rows_index = self.index.clone();
            true
        }

        /// How many whole copies of the index are alive: the window's, and
        /// every older snapshot something still holds.
        fn alive(&mut self) -> usize {
            self.stale.retain(|w| w.strong_count() > 0);
            1 + self.stale.len()
        }
    }

    /// The report behind 0.10.1: files change every second for a long time
    /// and every save is much slower than the changes. However long it
    /// lasts, there are never more than two copies of the index: the one a
    /// save is writing and the one the window uses.
    #[test]
    fn slow_saves_never_pile_up_copies() {
        let disk = Arc::new(Disk::default());
        let queue = queue_on(&disk);
        let mut w = Window::new(index_of(2_000));
        let mut most = 0;
        // 6,000 batches (100 minutes at one a second); a save takes 150 of them.
        for step in 0..6_000 {
            w.batch(&queue, step);
            if step % 150 == 0 {
                // A save begins: it takes the index as it is now.
                until("a save to start", || disk.started.load(Ordering::SeqCst) > disk.finished.load(Ordering::SeqCst));
            }
            most = most.max(w.alive());
            assert!(most <= 2, "{most} copies of the index alive at step {step}");
            // What waits to be saved is always the window's own index.
            assert!(at_rest(&queue) || queue.running() || queue.is_pending(&w.index));
            if step % 150 == 149 {
                let before = disk.finished.load(Ordering::SeqCst);
                disk.allow(1);
                until("the slow save to end", || disk.finished.load(Ordering::SeqCst) > before);
            }
        }
        assert_eq!(most, 2, "a save in progress keeps one snapshot");
        // Saves kept happening, one at a time, and each cost one copy at most.
        assert_eq!(disk.finished.load(Ordering::SeqCst), 40, "one save per 150 batches");
        assert_eq!(disk.most_inside.load(Ordering::SeqCst), 1, "never two saves at once");
        assert_eq!(w.copies, 40, "each save cost one copy of the index, and no more");

        // Leaving: the last state gets written, and then one copy is left.
        disk.open();
        // (`Nothing` when the saving thread got there before this call.)
        assert_ne!(queue.flush_within(Duration::from_secs(20)), Flush::TimedOut);
        assert_eq!(disk.last_written.load(Ordering::SeqCst), 5_999 + 2);
        until("the saving thread to let go", || w.alive() == 1);
        assert!(at_rest(&queue) && !queue.is_pending(&w.index));
    }

    /// The saver model, in random interleavings: edits, saves starting and
    /// saves ending, in any order (the saving thread adds its own timing on
    /// top). After every step: at most one snapshot is held besides the
    /// window's index, never two writes at once; what gets written only ever
    /// gets newer; and once everything settles the disk holds exactly what
    /// memory holds.
    #[test]
    fn random_interleavings_keep_the_invariants() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        for round in 0..150 {
            let disk = Arc::new(Disk::default());
            let queue = queue_on(&disk);
            let mut w = Window::new(index_of(30));
            let (mut step, mut last) = (0, None);
            for _ in 0..40 {
                match next(5) {
                    // An edit (most often), with or without the copy pacing.
                    0..=2 => {
                        step += 1 + next(12) as usize;
                        if w.batch(&queue, step) {
                            last = Some(step);
                        }
                    }
                    // Let the saving thread get as far as starting a write.
                    // (Or as far as having nothing left to write.)
                    3 => until("a save to start", || {
                        at_rest(&queue) || disk.started.load(Ordering::SeqCst) > disk.finished.load(Ordering::SeqCst)
                    }),
                    // The disk finishes the write under way, if there is one.
                    _ => {
                        let (started, finished) = (disk.started.load(Ordering::SeqCst), disk.finished.load(Ordering::SeqCst));
                        if started > finished {
                            disk.allow(1);
                            until("the write to end", || disk.finished.load(Ordering::SeqCst) > finished);
                        }
                    }
                }
                assert!(w.alive() <= 2, "round {round}: {} copies alive", w.alive());
                assert!(disk.most_inside.load(Ordering::SeqCst) <= 1, "round {round}: two writes at once");
                let written = disk.written.lock().unwrap();
                assert!(written.windows(2).all(|p| p[0] <= p[1]), "round {round}: an older index was written after a newer one: {written:?}");
            }
            disk.open();
            until("the queue to come to rest", || at_rest(&queue));
            if let Some(last) = last {
                assert_eq!(disk.last_written.load(Ordering::SeqCst), last + 2, "round {round}: the disk holds the last state");
            }
            until("the last snapshot to be let go", || w.alive() == 1);
            assert!(!shared(&queue, &w.index, 1));
            let written = disk.written.lock().unwrap();
            assert!(written.windows(2).all(|p| p[0] <= p[1]), "round {round}: {written:?}");
        }
    }

    /// The same bound measured in bytes, with a counting allocator: a save
    /// is stuck on the disk during 100 edits; the edits allocate one more
    /// copy of the index and no second one.
    #[test]
    fn a_stuck_save_costs_one_copy_in_bytes() {
        // What one whole copy of this index allocates.
        let model = index_of(20_000);
        let (one, copy) = crate::test_alloc::peak_of(|| Index { entries: model.entries.clone(), roots: model.roots.clone(), built_at: 0 });
        drop(copy);
        assert!(one > 2_000_000, "one copy is {one} bytes");

        let disk = Arc::new(Disk::default());
        let queue = queue_on(&disk);
        let mut w = Window::new(model);
        w.batch(&queue, 0);
        until("the save to start", || disk.started.load(Ordering::SeqCst) == 1);
        // The save now holds the index and stays stuck: 100 edits.
        let (peak, ()) = crate::test_alloc::peak_of(|| {
            for step in 1..=100 {
                w.batch(&queue, step * COPY_GAP_STEPS);
            }
        });
        assert_eq!(w.copies, 1);
        assert!(peak >= one * 9 / 10 && peak <= one * 11 / 10, "the edits allocated {peak} bytes at the most; one copy is {one}");
        assert_eq!(w.alive(), 2);
        disk.open();
        until("the queue to come to rest", || at_rest(&queue));
        until("the snapshot to be let go", || w.alive() == 1);
    }

    /// The scheme up to 0.10.0, replayed step by step (no threads): a save is
    /// queued 30 steps after a change, takes the index as it is then, and
    /// waits for the disk holding it. The same run piles up copies, which is
    /// what the test above would have caught.
    #[test]
    fn the_old_scheme_piled_up_under_the_same_load() {
        struct Saver {
            wakes: usize,
            holds: Option<Arc<Index>>,
        }
        let mut index = Arc::new(index_of(200));
        let mut rows_index = index.clone();
        let mut pending: Option<Arc<Index>> = None;
        let mut queued = false;
        let mut savers: Vec<Saver> = Vec::new();
        let mut disk_busy_until: Option<usize> = None;
        let mut last_copy: Option<usize> = None;
        let mut most = 0;
        const SAVE_STEPS: usize = 150;
        for step in 0..6_000 {
            // Savers whose sleep ended take what is waiting and queue for the disk.
            for s in savers.iter_mut().filter(|s| s.holds.is_none() && s.wakes == step) {
                queued = false;
                s.holds = pending.take();
            }
            savers.retain(|s| s.wakes > step || s.holds.is_some());
            // The disk writes one snapshot at a time.
            if disk_busy_until.is_some_and(|t| t <= step) {
                disk_busy_until = None;
                let at = savers.iter().position(|s| s.holds.is_some()).unwrap();
                savers.remove(at);
            }
            if disk_busy_until.is_none() && savers.iter().any(|s| s.holds.is_some()) {
                disk_busy_until = Some(step + SAVE_STEPS);
            }
            // The watcher batch (`apply_watched` + `edit_index` of 0.10.0).
            let own = usize::from(Arc::ptr_eq(&rows_index, &index)) + usize::from(pending.as_ref().is_some_and(|p| Arc::ptr_eq(p, &index)));
            if !(Arc::strong_count(&index) > 1 + own && last_copy.is_some_and(|t| step - t < COPY_GAP_STEPS)) {
                drop(std::mem::take(&mut rows_index));
                drop(pending.take());
                let (mut owned, copied) = take_for_edit(&mut index);
                if copied {
                    last_copy = Some(step);
                }
                owned.entries[0].size = step as u64 + 2;
                index = Arc::new(owned);
                rows_index = index.clone();
                pending = Some(index.clone());
                if !std::mem::replace(&mut queued, true) {
                    savers.push(Saver { wakes: step + 30, holds: None });
                }
            }
            // Copies alive: the window's, and every different snapshot a
            // waiting saver holds.
            let held = savers.iter().filter_map(|s| s.holds.as_ref()).filter(|h| !Arc::ptr_eq(h, &index)).count();
            most = most.max(1 + held);
        }
        assert!(most > 20, "the old scheme held {most} copies");
    }

    #[test]
    fn the_waiting_save_is_the_current_index_and_never_forces_a_copy() {
        let disk = Arc::new(Disk::default());
        let queue = queue_on(&disk);
        let mut index = Arc::new(index_of(10));
        // A save far in the future: nothing is written during the test.
        let far = Duration::from_secs(3600);
        for i in 0..100u64 {
            let before = index.entries.as_ptr();
            let done = edit(&queue, &mut index, |idx| { idx.entries[0].size = i; true }, Some(far));
            assert!(!done.copied, "edit {i} copied the index");
            assert_eq!(index.entries.as_ptr(), before, "the entries were moved, not copied");
            assert!(queue.is_pending(&index));
            assert!(!shared(&queue, &index, 0));
        }
        assert_eq!(disk.started.load(Ordering::SeqCst), 0);
        // An edit that changes nothing keeps the save that was waiting.
        let done = edit(&queue, &mut index, |_| false, Some(far));
        assert!(!done.changed && queue.is_pending(&index));
        // Without `save_in` nothing is queued (a scan is running: it saves).
        queue.discard();
        edit(&queue, &mut index, |_| true, None);
        assert!(!queue.is_pending(&index));
        disk.open();
    }

    #[test]
    fn another_reader_costs_one_copy() {
        let mut index = Arc::new(index_of(10));
        let reader = index.clone();
        let (owned, copied) = take_for_edit(&mut index);
        assert!(copied);
        assert_eq!((reader.entries.len(), owned.entries.len()), (10, 10), "the reader's snapshot is untouched");
        drop(reader);
        // Alone again: the entries are taken out as they are.
        let at = index.entries.as_ptr();
        let (owned, copied) = take_for_edit(&mut index);
        assert!(!copied);
        assert_eq!(owned.entries.as_ptr(), at);
    }

    /// At rest, what is on disk is what is in memory: after a burst of edits
    /// and no further change, the real file holds the last state, without
    /// anyone asking for a flush.
    #[test]
    fn at_rest_the_file_holds_what_memory_holds() {
        let dir = std::env::temp_dir().join(format!("bahth_save_rest_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("index.bin");
        let f = file.clone();
        let queue = queue_with(Box::new(move |i| i.save(&f)));
        let mut index = Arc::new(index_of(300));
        for round in 0..5u64 {
            for i in 0..40u64 {
                let size = round * 1000 + i;
                edit(&queue, &mut index, |idx| { idx.entries[(i % 7) as usize].size = size; true }, Some(Duration::ZERO));
            }
            until("the queue to come to rest", || at_rest(&queue));
            let on_disk = Index::load(&file).expect("a whole file");
            let sizes = |i: &Index| i.entries.iter().map(|e| (e.path.to_string(), e.size)).collect::<Vec<_>>();
            assert_eq!(sizes(&on_disk), sizes(&index), "round {round}");
            assert!(!shared(&queue, &index, 0), "nothing holds the index at rest");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A save that fails is tried again by itself (no new change needed),
    /// with growing pauses, until the disk takes it; saving is never
    /// switched off for the rest of the session.
    #[test]
    fn a_failed_save_is_tried_again_until_it_works() {
        let tries = Arc::new(AtomicUsize::new(0));
        let written = Arc::new(AtomicUsize::new(0));
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let (t, w, l) = (tries.clone(), written.clone(), lines.clone());
        let queue = SaveQueue::new(
            Box::new(move |i| {
                if t.fetch_add(1, Ordering::SeqCst) < 3 {
                    return Err(io::Error::other("disk full"));
                }
                w.store(i.entries[0].size as usize, Ordering::SeqCst);
                Ok(())
            }),
            Box::new(|_| {}),
            Box::new(move |s| l.lock().unwrap().push(s.to_string())),
            || {},
            0,
            Duration::from_millis(1),
        );
        let mut index = Arc::new(index_of(3));
        edit(&queue, &mut index, |idx| { idx.entries[0].size = 41; true }, Some(Duration::ZERO));
        until("the save to get through", || written.load(Ordering::SeqCst) == 41);
        until("the queue to come to rest", || at_rest(&queue));
        assert_eq!(tries.load(Ordering::SeqCst), 4, "three failures, then the one that worked");
        // The snapshot kept for the retries was the window's own: no copy.
        assert!(!shared(&queue, &index, 0));
        // A line for each failure (they are never held back).
        let lines = lines.lock().unwrap();
        assert_eq!(lines.iter().filter(|l| l.starts_with("index save: failed gen ") && l.contains("disk full")).count(), 3, "{lines:?}");
        drop(lines);
        // The pauses grow, up to the longest gap.
        assert_eq!(retry_in(RETRY, 1), Duration::from_secs(30));
        assert_eq!(retry_in(RETRY, 2), Duration::from_secs(60));
        assert_eq!(retry_in(RETRY, 4), Duration::from_secs(240));
        assert_eq!(retry_in(RETRY, 9), MAX_GAP);
        assert_eq!(retry_in(RETRY, u32::MAX), MAX_GAP);
        // And the next change is saved as usual.
        edit(&queue, &mut index, |idx| { idx.entries[0].size = 42; true }, Some(Duration::ZERO));
        until("the next save", || written.load(Ordering::SeqCst) == 42);
    }

    /// A newer index that came while a save was failing is the one retried.
    #[test]
    fn a_retry_writes_the_newest_index() {
        let disk = Arc::new(Disk::default());
        let fail = Arc::new(AtomicUsize::new(1));
        let (d, f) = (disk.clone(), fail.clone());
        let queue = queue_with(Box::new(move |i| {
            d.write(i)?;
            if f.swap(0, Ordering::SeqCst) == 1 { Err(io::Error::other("sharing violation")) } else { Ok(()) }
        }));
        let mut w = Window::new(index_of(5));
        w.batch(&queue, 0);
        until("the first save to start", || disk.started.load(Ordering::SeqCst) == 1);
        // The window changes the index while that save is stuck (one copy).
        w.batch(&queue, 100);
        assert_eq!((w.copies, w.alive()), (1, 2));
        disk.open();
        until("the queue to come to rest", || at_rest(&queue));
        assert_eq!(disk.last_written.load(Ordering::SeqCst), 100 + 2, "the retry wrote the newer index");
        assert_eq!(w.alive(), 1, "the failed snapshot was let go");
    }

    /// A panic while writing (the hook reports it) counts as a failed save:
    /// the flags are reset, the save is tried again, later ones still run.
    #[test]
    fn a_panic_in_the_writer_does_not_stop_saving() {
        let calls = Arc::new(AtomicUsize::new(0));
        let written = Arc::new(AtomicUsize::new(0));
        let (c, w) = (calls.clone(), written.clone());
        let queue = queue_with(Box::new(move |i| {
            if c.fetch_add(1, Ordering::SeqCst) == 0 {
                panic!("synthetic panic in the index writer (expected by this test)");
            }
            w.store(i.entries[0].size as usize, Ordering::SeqCst);
            Ok(())
        }));
        let mut index = Arc::new(index_of(3));
        edit(&queue, &mut index, |idx| { idx.entries[0].size = 7; true }, Some(Duration::ZERO));
        until("the retry after the panic", || written.load(Ordering::SeqCst) == 7);
        until("the queue to come to rest", || at_rest(&queue));
        assert!(!queue.running());
        edit(&queue, &mut index, |idx| { idx.entries[0].size = 8; true }, Some(Duration::ZERO));
        until("a later save", || written.load(Ordering::SeqCst) == 8);
        // `save_now` (a finished scan) reports the panic as an error and
        // counts itself out too.
        calls.store(0, Ordering::SeqCst);
        assert!(queue.save_now(&index).is_err());
        assert!(!queue.running());
        assert!(queue.save_now(&index).is_ok());
    }

    /// Should the saving thread itself die (a panic outside the writer), the
    /// next request starts a new one.
    #[test]
    fn a_dead_saving_thread_is_replaced() {
        let written = Arc::new(AtomicUsize::new(0));
        let w = written.clone();
        let first_line = Arc::new(AtomicUsize::new(0));
        let f = first_line.clone();
        let queue = SaveQueue::new(
            Box::new(move |i| {
                w.store(i.entries[0].size as usize, Ordering::SeqCst);
                Err(io::Error::other("no"))
            }),
            Box::new(|_| {}),
            // The logger panics once, on the saving thread, at the failure.
            Box::new(move |line| {
                if line.contains("failed") && f.fetch_add(1, Ordering::SeqCst) == 0 {
                    panic!("synthetic panic in the logger (expected by this test)");
                }
            }),
            || {},
            0,
            Duration::from_secs(3600),
        );
        let mut index = Arc::new(index_of(3));
        edit(&queue, &mut index, |idx| { idx.entries[0].size = 5; true }, Some(Duration::ZERO));
        until("the thread to die", || !queue.lock().worker && !queue.running());
        assert_eq!(written.load(Ordering::SeqCst), 5);
        // The failed index still waits, and a new thread takes the next request.
        assert!(queue.is_pending(&index));
        edit(&queue, &mut index, |idx| { idx.entries[0].size = 6; true }, Some(Duration::ZERO));
        assert!(queue.lock().worker);
        assert_eq!(queue.flush_within(Duration::from_millis(30)), Flush::TimedOut, "its retry is an hour away");
    }

    /// The marker file: there while memory has changes the disk has not,
    /// gone once they are written; still there if the program dies first.
    #[test]
    fn the_marker_follows_unsaved_changes() {
        let dir = std::env::temp_dir().join(format!("bahth_save_marker_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("index.bin");
        let marker = marker_path(&file);
        assert_eq!(marker.file_name().unwrap(), "index.dirty");
        let disk = Arc::new(Disk::default());
        let queue_for = |disk: &Arc<Disk>| {
            let (d, m) = (disk.clone(), marker.clone());
            SaveQueue::new(
                Box::new(move |i| d.write(i)),
                Box::new(move |on| {
                    if on {
                        std::fs::write(&m, b"").unwrap();
                    } else {
                        let _ = std::fs::remove_file(&m);
                    }
                }),
                Box::new(|_| {}),
                || {},
                0,
                Duration::ZERO,
            )
        };
        let queue = queue_for(&disk);
        let mut index = Arc::new(index_of(5));
        assert!(!marker.exists());
        // A change: marked before its save has even been let through.
        edit(&queue, &mut index, |idx| { idx.entries[0].size = 2; true }, Some(Duration::ZERO));
        until("the marker", || marker.exists());
        until("the save to start", || disk.started.load(Ordering::SeqCst) == 1);
        // Another change while that save is stuck: one save ending is not enough.
        edit(&queue, &mut index, |idx| { idx.entries[0].size = 3; true }, Some(Duration::ZERO));
        disk.allow(1);
        until("the first save to end", || disk.finished.load(Ordering::SeqCst) == 1);
        until("the second save to start", || disk.started.load(Ordering::SeqCst) == 2);
        assert!(marker.exists(), "the newer change is not on disk yet");
        disk.allow(1);
        until("the marker to go", || !marker.exists());
        until("rest", || at_rest(&queue));
        assert_eq!(disk.last_written.load(Ordering::SeqCst), 3);

        // The program dies with a change unsaved (its save never ends): the
        // marker is what the next start finds.
        edit(&queue, &mut index, |idx| { idx.entries[0].size = 4; true }, Some(Duration::ZERO));
        until("the marker", || marker.exists());
        assert_eq!(queue.flush_within(Duration::from_millis(20)), Flush::TimedOut);
        assert!(marker.exists());

        // The next start: a scan is owed. Saving edits does not clear the
        // marker; only the scan's own save does.
        let disk = Arc::new(Disk::default());
        disk.open();
        let queue = queue_for(&disk);
        queue.scan_owed();
        edit(&queue, &mut index, |idx| { idx.entries[0].size = 5; true }, Some(Duration::ZERO));
        until("the edit's save", || disk.last_written.load(Ordering::SeqCst) == 5);
        until("rest", || at_rest(&queue));
        assert!(marker.exists(), "an edit's save is not a reconciliation");
        queue.save_now(&index_of(6)).unwrap();
        assert!(!marker.exists());
        // From then on the marker follows the edits again.
        edit(&queue, &mut index, |idx| { idx.entries[0].size = 6; true }, Some(Duration::ZERO));
        until("rest", || at_rest(&queue));
        until("the marker to go", || !marker.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A full scan whose own save failed hands its index to the queue (as
    /// `poll_scan` does): it is marked unsaved, tried again and, at rest,
    /// the disk holds it.
    #[test]
    fn a_scan_whose_save_failed_is_saved_later() {
        let fail = Arc::new(AtomicUsize::new(1));
        let written = Arc::new(AtomicUsize::new(0));
        let marked = Arc::new(AtomicUsize::new(0));
        let (f, w, m) = (fail.clone(), written.clone(), marked.clone());
        let queue = SaveQueue::new(
            Box::new(move |i| {
                if f.swap(0, Ordering::SeqCst) == 1 {
                    return Err(io::Error::other("disk full"));
                }
                w.store(i.entries[0].size as usize, Ordering::SeqCst);
                Ok(())
            }),
            Box::new(move |on| m.store(usize::from(on), Ordering::SeqCst)),
            Box::new(|_| {}),
            || {},
            0,
            Duration::ZERO,
        );
        let mut scan = index_of(4);
        scan.entries[0].size = 9;
        let scan = Arc::new(scan);
        assert!(queue.save_now(&scan).is_err());
        assert_eq!(written.load(Ordering::SeqCst), 0);
        queue.set_pending(scan.clone(), Duration::ZERO);
        until("the marker", || marked.load(Ordering::SeqCst) == 1 || written.load(Ordering::SeqCst) == 9);
        until("the retry", || written.load(Ordering::SeqCst) == 9);
        until("rest", || at_rest(&queue));
        until("the marker to go", || marked.load(Ordering::SeqCst) == 0);
    }

    /// A save that fails after a scan has taken over (`discard`) is not put
    /// back: its snapshot is let go at once, no third copy.
    #[test]
    fn a_failed_save_given_up_meanwhile_is_not_retried() {
        let disk = Arc::new(Disk::default());
        let fail = Arc::new(AtomicUsize::new(1));
        let (d, f) = (disk.clone(), fail.clone());
        let queue = queue_with(Box::new(move |i| {
            d.write(i)?;
            if f.swap(0, Ordering::SeqCst) == 1 { Err(io::Error::other("sharing violation")) } else { Ok(()) }
        }));
        let old = Arc::new(index_of(5));
        let held = Arc::downgrade(&old);
        queue.set_pending(old, Duration::ZERO);
        until("the save to start", || disk.started.load(Ordering::SeqCst) == 1);
        queue.discard();
        disk.open();
        until("rest", || at_rest(&queue));
        assert!(held.upgrade().is_none(), "the given-up snapshot was kept for a retry");
        assert_eq!(disk.started.load(Ordering::SeqCst), 1);
    }

    /// A save skipped because a newer index is already on disk still clears
    /// the marker: nothing is unsaved.
    #[test]
    fn a_skipped_save_clears_the_marker() {
        let marked = Arc::new(AtomicUsize::new(0));
        let m = marked.clone();
        let queue = SaveQueue::new(
            Box::new(|_| Ok(())),
            Box::new(move |on| m.store(usize::from(on), Ordering::SeqCst)),
            Box::new(|_| {}),
            || {},
            0,
            Duration::ZERO,
        );
        // A newer index is already written (number 5)...
        assert!(queue.write(&index_of(3), 5).unwrap());
        // ...then an older one comes up (number 1): skipped.
        queue.lock().next_seq = 0;
        queue.set_pending(Arc::new(index_of(3)), Duration::ZERO);
        until("rest", || at_rest(&queue));
        until("the marker to go", || marked.load(Ordering::SeqCst) == 0);
        assert!(!queue.lock().marked);
    }

    #[test]
    fn leaving_waits_a_bounded_time_for_a_stuck_disk() {
        let disk = Arc::new(Disk::default());
        let queue = queue_on(&disk);
        assert_eq!(queue.flush_within(Duration::from_secs(5)), Flush::Nothing);
        let index = Arc::new(index_of(5));
        queue.set_pending(index.clone(), Duration::from_secs(3600));
        // The disk never answers: leaving still returns.
        let started = Instant::now();
        assert_eq!(queue.flush_within(Duration::from_millis(60)), Flush::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(10));
        // The save was started at once, although it was due in an hour.
        until("the save to start", || disk.started.load(Ordering::SeqCst) == 1);
        assert_eq!(disk.finished.load(Ordering::SeqCst), 0);
        // Once the disk answers, a second wait sees it through.
        disk.open();
        assert_ne!(queue.flush_within(Duration::from_secs(20)), Flush::TimedOut);
        assert_eq!(disk.finished.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn leaving_writes_at_once_what_was_due_later() {
        let disk = Arc::new(Disk::default());
        disk.open();
        let queue = queue_on(&disk);
        let mut index = Arc::new(index_of(5));
        edit(&queue, &mut index, |idx| { idx.entries[0].size = 77; true }, Some(Duration::from_secs(3600)));
        assert_eq!(queue.flush_within(Duration::from_secs(20)), Flush::Saved);
        assert_eq!(disk.last_written.load(Ordering::SeqCst), 77);
        assert_eq!(queue.flush_within(Duration::from_secs(20)), Flush::Nothing);
    }

    #[test]
    fn an_older_index_is_never_written_over_a_newer_one() {
        let disk = Arc::new(Disk::default());
        let queue = queue_on(&disk);
        // An edit's save starts and is stuck on the disk...
        let mut old = index_of(5);
        old.entries[0].size = 1;
        queue.set_pending(Arc::new(old), Duration::ZERO);
        until("the first save to start", || disk.started.load(Ordering::SeqCst) == 1);
        // ...another edit waits behind it...
        let mut newer = index_of(5);
        newer.entries[0].size = 2;
        queue.set_pending(Arc::new(newer), Duration::ZERO);
        // ...and a finished scan writes its index, which is the newest.
        let mut scan = index_of(5);
        scan.entries[0].size = 3;
        let q = queue.clone();
        let scan_thread = std::thread::spawn(move || q.save_now(&scan).unwrap());
        until("the scan to take its number", || queue.lock().next_seq == 3);
        disk.open();
        scan_thread.join().unwrap();
        assert!(queue.wait_idle(Duration::from_secs(20)));
        assert_eq!(disk.last_written.load(Ordering::SeqCst), 3);
        // The edit that waited was forgotten: the scan had seen its change.
        assert_eq!(queue.flush_within(Duration::from_secs(20)), Flush::Nothing);
        assert_eq!(disk.finished.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_save_that_lost_the_race_to_a_newer_one_is_skipped() {
        let disk = Arc::new(Disk::default());
        disk.open();
        let queue = queue_on(&disk);
        let (mut newer, mut older) = (index_of(3), index_of(3));
        (newer.entries[0].size, older.entries[0].size) = (9, 4);
        assert!(queue.write(&newer, 5).unwrap());
        assert!(!queue.write(&older, 3).unwrap(), "the older snapshot must not replace the newer file");
        assert_eq!(disk.last_written.load(Ordering::SeqCst), 9);
        assert_eq!(disk.finished.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_slow_save_test_hook_is_read_carefully() {
        assert_eq!(TEST_SAVE_DELAY_VAR, "BAHTH_TEST_SAVE_DELAY_MS");
        assert_eq!(test_save_delay(None), None, "inert when unset");
        for bad in ["", " ", "abc", "-5", "1.5", "0", "9999999999999999999999"] {
            assert_eq!(test_save_delay(Some(bad)), None, "{bad:?}");
        }
        assert_eq!(test_save_delay(Some("90000")), Some(Duration::from_secs(90)));
        assert_eq!(test_save_delay(Some(" 250 ")), Some(Duration::from_millis(250)));
        assert_eq!(test_save_delay(Some("600000")), Some(Duration::from_secs(600)));
        assert_eq!(test_save_delay(Some("5000000")), Some(Duration::from_secs(600)), "capped at ten minutes");
    }

    #[test]
    fn slow_saves_are_spaced_out() {
        assert_eq!(next_gap(Duration::from_millis(200), GAP_FACTOR), Duration::from_millis(800));
        assert_eq!(next_gap(Duration::from_secs(45), GAP_FACTOR), Duration::from_secs(180));
        assert_eq!(next_gap(Duration::from_secs(3600), GAP_FACTOR), MAX_GAP);
        assert_eq!(next_gap(Duration::from_secs(45), 0), Duration::ZERO);
    }

    /// The save queue's lines in log.txt: started and done at most once a
    /// minute each, with the count of the ones held back; failures always.
    #[test]
    fn save_lines_are_limited_per_kind() {
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let l = lines.clone();
        let queue = SaveQueue::new(Box::new(|_| Ok(())), Box::new(|_| {}), Box::new(move |s| l.lock().unwrap().push(s.to_string())), || {}, 0, Duration::ZERO);
        for _ in 0..5 {
            queue.set_pending(Arc::new(index_of(3)), Duration::ZERO);
            until("rest", || at_rest(&queue));
        }
        // An older save that comes after a newer one is skipped, said once.
        queue.lock().written_seq = 100;
        for _ in 0..3 {
            queue.set_pending(Arc::new(index_of(3)), Duration::ZERO);
            until("rest", || at_rest(&queue));
        }
        let lines = lines.lock().unwrap();
        let count = |p: &str| lines.iter().filter(|l| l.starts_with(p)).count();
        assert_eq!(count("index save: started gen 1"), 1, "{lines:?}");
        assert_eq!(count("index save: started"), 1, "{lines:?}");
        assert_eq!(count("index save: done gen 1 in "), 1, "{lines:?}");
        assert_eq!(count("index save: done"), 1, "{lines:?}");
        assert_eq!(count("index save: skipped gen 6 (gen 100 is already on disk)"), 1, "{lines:?}");
        assert_eq!(count("index save: skipped"), 1, "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("(3 entries; the next one waits 0 s)")), "{lines:?}");
    }

    #[test]
    fn repeated_log_lines_are_held_back_and_counted() {
        let mut log = RateLog::new(Duration::from_secs(600));
        let t = Instant::now();
        assert_eq!(log.due(t), Some(0));
        for s in 1..=5 {
            assert_eq!(log.due(t + Duration::from_secs(s * 10)), None);
        }
        assert_eq!(log.due(t + Duration::from_secs(601)), Some(5));
        assert_eq!(log.due(t + Duration::from_secs(602)), None);
    }
}
