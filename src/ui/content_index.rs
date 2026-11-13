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
static CACHE: OnceLock<Option<ContentCache>> = OnceLock::new();

/// The store, only if it is open already (never opens it: no disk access).
pub(super) fn opened() -> Option<&'static ContentCache> {
    CACHE.get()?.as_ref()
}

pub(super) fn shared() -> Option<&'static ContentCache> {
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
                // makes scanned PDFs and images worth reading once more; a
                // new version of the program alone does not (only the files
                // a run was ended while reading are tried once more).
                let signature = content_cache::signature(&crate::ocr::languages());
                let dropped = c.revalidate(&signature, env!("CARGO_PKG_VERSION"));
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
            let mut docs: Vec<Entry> = index
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
            // Not program and system files, unless the user asked (see
            // `warm_wanted`): the wanted ones are moved to the front of
            // `docs` (the warm-up sorts them itself), which stays whole for
            // `evict_missing`.
            let everything = content_cache::read_programs();
            let mut wanted = 0;
            for i in 0..docs.len() {
                if content_cache::warm_wanted(&docs[i], everything) {
                    docs.swap(wanted, i);
                    wanted += 1;
                }
            }
            log(&format!(
                "content index: {wanted} of {} documents to read inside{}",
                docs.len(),
                if everything { " (program and system files too)" } else { "" }
            ));
            content_cache::warm(&docs[..wanted], cache, progress, cancel, threads, &idle_priority);
            if index_settled && !cancel.load(Ordering::Relaxed) {
                let indexed: std::collections::HashSet<&str> = docs.iter().map(|e| &*e.path).collect();
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
                let gone = cache.evict_missing(&indexed, now);
                if gone > 0 {
                    log(&format!("content index: forgot {gone} deleted documents"));
                }
            }
            let s = cache.stats();
            let w = cache.word_stats();
            log(&format!(
                "content index warm: {} of {} new documents in {} ms on {threads} threads{}; {} cloud-only not read; stored {} ({} with text), {} KB{}; word index: {} added from the store, {} documents ({} waiting in memory), {} segments, {} KB on disk, about {} KB in memory{}",
                progress.added.load(Ordering::Relaxed),
                progress.total.load(Ordering::Relaxed),
                started.elapsed().as_millis(),
                if cancel.load(Ordering::Relaxed) { " (stopped)" } else { "" },
                progress.cloud.load(Ordering::Relaxed),
                s.documents,
                s.texts,
                s.bytes / 1024,
                if s.full { ", full" } else { "" },
                progress.indexed.load(Ordering::Relaxed),
                w.docs,
                w.pending,
                w.segments,
                w.bytes / 1024,
                w.heap / 1024,
                if w.broken { ", damaged: rebuilt by the next warm-up" } else { "" }
            ));
            drop(finished);
        });
    }

    pub(super) fn stop_warm(&self) {
        if let Some(job) = self.state.borrow_mut().warm.take() {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// The Options menu switch «قراءة محتوى الملفات الموجودة على السحابة
    /// فقط»: on, content reading opens cloud-only files too, so Windows
    /// downloads them, and the content index reads the ones it skipped
    /// (they were never marked, so a new warm-up finds them). Off: none is
    /// read any more; what was read stays in the content index.
    pub(super) fn toggle_read_cloud(&self) {
        let on = !self.menu_read_cloud.checked();
        self.menu_read_cloud.set_checked(on);
        self.state.borrow_mut().settings.read_cloud_files = on;
        // Saved now: a crash or a forced end would otherwise lose the choice.
        self.save_settings();
        crate::cloud::set_read_cloud(on);
        log(&format!("cloud-only files: {}", if on { "read (downloaded)" } else { "not read" }));
        if on {
            self.start_warm();
        }
    }

    /// The switch «اقرأ داخل ملفات البرامج والنظام أيضاً»: on, the content
    /// index reads the files in program, system and developer-cache folders
    /// and every image too, and searches inside documents look in them; off
    /// (the default), only their names are indexed, and what was read stays
    /// stored. The warm-up starts over with the new choice.
    pub(super) fn toggle_read_programs(&self) {
        let on = !self.menu_read_programs.checked();
        self.menu_read_programs.set_checked(on);
        self.state.borrow_mut().settings.read_program_files = on;
        self.save_settings();
        content_cache::set_read_programs(on);
        log(&format!("program and system files: {}", if on { "read inside" } else { "names only" }));
        self.start_warm();
    }

    /// The Options menu switch. Off: no background indexing, and document
    /// searches neither read nor add to the store (it stays on disk).
    pub(super) fn toggle_content_index(&self) {
        let on = !self.menu_content_index.checked();
        self.menu_content_index.set_checked(on);
        self.state.borrow_mut().settings.content_index = on;
        self.save_settings();
        if on {
            self.start_warm();
        } else {
            self.stop_warm();
        }
    }
}
