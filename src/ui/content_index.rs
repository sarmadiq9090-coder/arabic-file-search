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
            cache.ok()
        })
        .as_ref()
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
    /// unless the user switched it off. Shows nothing while it runs.
    pub(super) fn start_warm(&self) {
        self.stop_warm();
        let mut st = self.state.borrow_mut();
        if !st.settings.content_index {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        st.warm = Some(cancel.clone());
        let index = st.index.clone();
        drop(st);
        crate::crash_log::spawn_named("content-warm", move || {
            idle_priority();
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
            let progress = WarmProgress::default();
            content_cache::warm(&docs, cache, &progress, &cancel);
            let s = cache.stats();
            log(&format!(
                "content index warm: {} of {} new documents in {} ms{}; stored {} ({} with text), {} KB{}",
                progress.added.load(Ordering::Relaxed),
                progress.total.load(Ordering::Relaxed),
                started.elapsed().as_millis(),
                if cancel.load(Ordering::Relaxed) { " (stopped)" } else { "" },
                s.documents,
                s.texts,
                s.bytes / 1024,
                if s.full { ", full" } else { "" }
            ));
        });
    }

    pub(super) fn stop_warm(&self) {
        if let Some(cancel) = self.state.borrow_mut().warm.take() {
            cancel.store(true, Ordering::Relaxed);
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
