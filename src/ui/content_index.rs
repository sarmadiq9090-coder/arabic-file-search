//! The persistent content index: one store of document text shared by the
//! document searches and a low-priority thread that fills it in the background.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use super::App;
use super::startup::log;
use crate::content_cache::{self, ContentCache, WarmProgress};
use crate::index::{self, Entry};

/// The store in `%LOCALAPPDATA%\BahthSari\content\`, opened on first use
/// (always from a background thread). `None` when it can't be opened.
pub(super) fn shared() -> Option<&'static ContentCache> {
    static CACHE: OnceLock<Option<ContentCache>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let started = Instant::now();
            let cache = ContentCache::open(&index::cache_path().with_file_name("content"));
            match &cache {
                Ok(c) => log(&format!(
                    "content index open: {} documents, {} KB in {} ms",
                    c.stats().documents,
                    c.stats().bytes / 1024,
                    started.elapsed().as_millis()
                )),
                Err(e) => log(&format!("content index open failed: {e}")),
            }
            if let Ok(c) = &cache {
                // The last run died reading these: now skipped until they change.
                for path in c.recovered() {
                    log(&format!("content index: the last run stopped while reading {path}; skipped until the file changes"));
                }
                c.clear_markers_at_exit();
                // Installing the Arabic OCR language pack (or a newer reader)
                // makes scanned PDFs and images worth reading once more.
                let signature = format!(
                    "extractor {}; ocr {}",
                    content_cache::EXTRACTOR_VERSION,
                    crate::ocr::languages().join(",")
                );
                let dropped = c.revalidate(&signature);
                if dropped > 0 {
                    log(&format!("content index: {dropped} documents to read again ({signature})"));
                }
            }
            cache.ok()
        })
        .as_ref()
}

/// A content-index warm-up: what stops it and how far it got, shared with
/// its thread so the window can show the progress (`ui/indexing.rs`).
#[derive(Default)]
pub(super) struct Warm {
    pub(super) cancel: AtomicBool,
    pub(super) progress: WarmProgress,
    /// Its thread has ended: all done, stopped, or no store to fill.
    pub(super) finished: AtomicBool,
}

/// Marks a warm-up finished however its thread ends, a panic included.
struct Finished(Arc<Warm>);

impl Drop for Finished {
    fn drop(&mut self) {
        self.0.finished.store(true, Ordering::Relaxed);
    }
}

/// Background OCR waits while this is true: the user typed or clicked in
/// the last few seconds, or a document search is running.
fn user_busy() -> bool {
    use winapi::um::sysinfoapi::GetTickCount;
    use winapi::um::winuser::{GetLastInputInfo, LASTINPUTINFO};
    let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    // SAFETY: a plain struct with its size set, as the API requires.
    let typed = unsafe { GetLastInputInfo(&mut info) } != 0 && unsafe { GetTickCount() }.wrapping_sub(info.dwTime) < 3_000;
    typed || shared().is_some_and(ContentCache::searching)
}

/// Lowest CPU priority, and low disk priority where Windows supports it.
fn idle_priority() {
    use winapi::um::processthreadsapi::{GetCurrentThread, SetThreadPriority};
    use winapi::um::winbase::{THREAD_MODE_BACKGROUND_BEGIN, THREAD_PRIORITY_IDLE};
    unsafe {
        if SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN as i32) == 0 {
            SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_IDLE as i32);
        }
    }
}

impl App {
    /// (Re)starts filling the content index from the current file index,
    /// unless the user switched it off. Its progress shows in the status bar
    /// (`poll_indexing`).
    pub(super) fn start_warm(&self) {
        self.stop_warm();
        let mut st = self.state.borrow_mut();
        if !st.settings.content_index {
            return;
        }
        let job = Arc::new(Warm::default());
        st.warm = Some(job.clone());
        let index = st.index.clone();
        // Deleted documents are forgotten only after a warm-up over a
        // complete index: not while it loads or a scan is rebuilding it.
        let index_settled = st.loading.is_none() && st.scan.is_none() && !index.entries.is_empty();
        drop(st);
        crate::crash_log::spawn_named("content-warm", move || {
            let finished = Finished(job.clone());
            let (cancel, progress) = (&job.cancel, &job.progress);
            idle_priority();
            crate::extract::set_ocr_pause(user_busy);
            // Only the documents are needed. Letting go of the index at once
            // matters: while another holder shares it, every change found by
            // the drive watcher has to copy the whole index.
            let docs: Vec<Entry> = index
                .entries
                .iter()
                .filter(|e| content_cache::is_document(e))
                .map(|e| Entry { path: e.path.clone(), size: e.size, mtime: e.mtime, is_dir: false, norm_name: Box::default() })
                .collect();
            drop(index);
            // Let start-up and the first searches go first.
            for _ in 0..50 {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            let Some(cache) = shared() else { return };
            let started = Instant::now();
            let threads = content_cache::warm_threads();
            content_cache::warm(&docs, cache, progress, cancel, threads, &idle_priority);
            if index_settled && !cancel.load(Ordering::Relaxed) {
                let indexed: std::collections::HashSet<&str> = docs.iter().map(|e| &*e.path).collect();
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
                let gone = cache.evict_missing(&indexed, now);
                if gone > 0 {
                    log(&format!("content index: forgot {gone} deleted documents"));
                }
            }
            let s = cache.stats();
            log(&format!(
                "content index warm: {} of {} new documents in {} ms on {threads} threads{}; stored {} ({} with text), {} KB{}",
                progress.added.load(Ordering::Relaxed),
                progress.total.load(Ordering::Relaxed),
                started.elapsed().as_millis(),
                if cancel.load(Ordering::Relaxed) { " (stopped)" } else { "" },
                s.documents,
                s.texts,
                s.bytes / 1024,
                if s.full { ", full" } else { "" }
            ));
            drop(finished);
        });
    }

    pub(super) fn stop_warm(&self) {
        if let Some(job) = self.state.borrow_mut().warm.take() {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// The Options menu switch. Off: no background indexing, and document
    /// searches neither read nor add to the store (it stays on disk).
    pub(super) fn toggle_content_index(&self) {
        let on = !self.menu_content_index.checked();
        self.menu_content_index.set_checked(on);
        self.state.borrow_mut().settings.content_index = on;
        if on {
            self.start_warm();
        } else {
            self.stop_warm();
        }
    }
}
