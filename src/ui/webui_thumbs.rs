//! Thumbnails for the new look's grid view (Ctrl+T): the classic thumbnail
//! view's pictures (`super::thumbs::shell_thumbnail`, the same shell
//! thumbnails Explorer shows), made off the UI thread and served to the
//! page as PNG through the program's own `instafind://` protocol.
//!
//! The page never names a path. It asks, over the typed bridge, for the
//! rows of a list version it was sent (`Request::Thumbs`); the program
//! checks them like any other row request and answers with a random
//! one-time key per row (`register`). The page then loads `/thumb/<key>`;
//! an unknown key is a 404 (`serve`). Keys are forgotten when the page
//! loads again, and only the newest `MAX_KEYS` are kept.
//!
//! At most `WORKERS` pictures are made at a time, newest request first; the
//! queue is bounded (the oldest, scrolled past, are answered "none"). Made
//! pictures stay in a small LRU (`MAX_CACHED`) keyed by path, size and
//! date, which the memory guard can drop (`shed`). A file without one is
//! remembered too; if it was cloud-only then, only for
//! `thumb_cache::CLOUD_MISS_RETRY` (it may be downloaded since).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use winapi::shared::windef::HGDIOBJ;
use winapi::um::wingdi::DeleteObject;

/// The answer to one thumbnail request: the PNG, or none (no thumbnail,
/// unknown key, dropped from the queue).
pub(super) type Reply = Box<dyn FnOnce(Option<Arc<Vec<u8>>>) + Send>;

/// Tile edge in pixels (the grid shows them at 96–128 CSS pixels; this is
/// sharp up to 150 % and still small: about 20 KB of PNG).
const TILE: i32 = 160;
const WORKERS: usize = 2;
const MAX_QUEUE: usize = 120;
const MAX_KEYS: usize = 4096;
const MAX_CACHED: usize = 256;
const MAX_NONE: usize = 4096;

struct Job {
    path: String,
    cache_key: String,
    reply: Reply,
}

#[derive(Default)]
struct State {
    /// One-time key -> (path, cache key).
    keys: HashMap<String, (String, String)>,
    order: VecDeque<String>,
    /// Cache key -> (PNG, last use).
    cache: HashMap<String, (Arc<Vec<u8>>, u64)>,
    tick: u64,
    /// Files that have no thumbnail, with when they were missed if they
    /// were cloud-only then.
    none: HashMap<String, Option<Instant>>,
    jobs: Vec<Job>,
    workers: usize,
}

#[derive(Default)]
struct Inner {
    state: Mutex<State>,
    wake: Condvar,
}

/// Shared between the UI thread (keys), the protocol handler (requests)
/// and the worker threads.
#[derive(Clone, Default)]
pub(super) struct ThumbBroker(Arc<Inner>);

impl ThumbBroker {
    /// A one-time key for the thumbnail of a file the page was sent.
    pub(super) fn register(&self, path: &str, size: u64, mtime: i64) -> String {
        let key = crate::webui::bridge::new_token();
        let Ok(mut st) = self.0.state.lock() else { return key };
        st.keys.insert(key.clone(), (path.to_string(), crate::thumb_cache::ThumbCache::key(path, size, mtime)));
        st.order.push_back(key.clone());
        while st.order.len() > MAX_KEYS {
            if let Some(old) = st.order.pop_front() {
                st.keys.remove(&old);
            }
        }
        key
    }

    /// The page loaded again: its keys are no longer valid.
    pub(super) fn forget_keys(&self) {
        if let Ok(mut st) = self.0.state.lock() {
            st.keys.clear();
            st.order.clear();
        }
    }

    /// The memory guard: lets go of the made pictures (they are made again).
    pub(super) fn shed(&self) {
        if let Ok(mut st) = self.0.state.lock() {
            st.cache.clear();
            st.none.clear();
        }
    }

    /// Answers `/thumb/<key>`: from the cache, else made by a worker.
    pub(super) fn serve(&self, key: &str, reply: Reply) {
        let Ok(mut st) = self.0.state.lock() else { return reply(None) };
        let Some((path, cache_key)) = st.keys.get(key).cloned() else {
            drop(st);
            return reply(None);
        };
        st.tick += 1;
        let tick = st.tick;
        if let Some((png, used)) = st.cache.get_mut(&cache_key) {
            *used = tick;
            let png = png.clone();
            drop(st);
            return reply(Some(png));
        }
        if let Some(&cloud) = st.none.get(&cache_key) {
            if crate::thumb_cache::miss_stands(cloud.map(|at| at.elapsed())) {
                drop(st);
                return reply(None);
            }
            st.none.remove(&cache_key);
        }
        st.jobs.push(Job { path, cache_key, reply });
        let mut dropped = Vec::new();
        while st.jobs.len() > MAX_QUEUE {
            dropped.push(st.jobs.remove(0));
        }
        let start = st.workers < WORKERS;
        if start {
            st.workers += 1;
        }
        drop(st);
        for j in dropped {
            (j.reply)(None);
        }
        if start {
            let inner = self.0.clone();
            let ok = crate::crash_log::spawn_named("webui-thumbs", move || worker(inner));
            if ok.is_none()
                && let Ok(mut st) = self.0.state.lock()
            {
                st.workers -= 1;
            }
        }
        self.0.wake.notify_one();
    }
}

/// Makes thumbnails, newest request first, until the program ends.
fn worker(inner: Arc<Inner>) {
    unsafe {
        winapi::um::combaseapi::CoInitializeEx(std::ptr::null_mut(), winapi::um::objbase::COINIT_APARTMENTTHREADED);
    }
    loop {
        let job = {
            let Ok(mut st) = inner.state.lock() else { return };
            loop {
                if let Some(j) = st.jobs.pop() {
                    break j;
                }
                st = match inner.wake.wait(st) {
                    Ok(st) => st,
                    Err(_) => return,
                };
            }
        };
        let (png, local) = crate::crash_log::catch(|| make_png(&job.path)).unwrap_or((None, true));
        let png = png.map(Arc::new);
        if let Ok(mut st) = inner.state.lock() {
            match &png {
                Some(p) => {
                    st.tick += 1;
                    let tick = st.tick;
                    st.cache.insert(job.cache_key.clone(), (p.clone(), tick));
                    if st.cache.len() > MAX_CACHED
                        && let Some(old) = st.cache.iter().min_by_key(|(_, (_, used))| *used).map(|(k, _)| k.clone())
                    {
                        st.cache.remove(&old);
                    }
                }
                None => {
                    if st.none.len() >= MAX_NONE {
                        st.none.clear();
                    }
                    st.none.insert(job.cache_key.clone(), (!local).then(Instant::now));
                }
            }
        }
        (job.reply)(png);
    }
}

/// The shell's thumbnail of `path` as PNG, or `None` when it has none, and
/// whether the file was on this PC (see `thumbs::shell_thumbnail`).
fn make_png(path: &str) -> (Option<Vec<u8>>, bool) {
    let (tile, local) = super::thumbs::shell_thumbnail(path, TILE);
    if tile.is_null() {
        return (None, local);
    }
    let px = super::thumbs::bitmap_pixels(tile);
    unsafe { DeleteObject(tile as HGDIOBJ) };
    let png = px.and_then(|(w, h, px)| crate::webui::png::from_argb(w as u32, h as u32, &px));
    (png, local)
}
