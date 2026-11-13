//! The memory guard's side in the window (`crate::mem_guard` decides):
//! samples the process every half minute from the timer, writes the
//! `memory:` line every ten minutes, and sheds what can be rebuilt when the
//! guard says so. It also notes the user's input in the program's windows
//! and runs the idle trim (`mem_guard::idle_trim_due`).

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use super::App;
use super::startup::log;
use crate::mem_guard::{self, Sample};

/// Private bytes of this process (`PrivateUsage`), if Windows says.
fn private_bytes() -> Option<u64> {
    private_and_peak().map(|(now, _)| now)
}

/// Private bytes of this process now and at their peak (`PrivateUsage`,
/// `PeakPagefileUsage`: the commit charge, which is the private bytes), if
/// Windows says. `K32GetProcessMemoryInfo` lives in kernel32 (Windows 7 and
/// later), so the exe does not import PSAPI.DLL (`ci/check_imports.py`
/// allows system DLLs only).
pub(super) fn private_and_peak() -> Option<(u64, u64)> {
    #[repr(C)]
    #[allow(non_snake_case)]
    struct Counters {
        cb: u32,
        PageFaultCount: u32,
        PeakWorkingSetSize: usize,
        WorkingSetSize: usize,
        QuotaPeakPagedPoolUsage: usize,
        QuotaPagedPoolUsage: usize,
        QuotaPeakNonPagedPoolUsage: usize,
        QuotaNonPagedPoolUsage: usize,
        PagefileUsage: usize,
        PeakPagefileUsage: usize,
        PrivateUsage: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn K32GetProcessMemoryInfo(process: *mut core::ffi::c_void, counters: *mut Counters, cb: u32) -> i32;
        fn GetCurrentProcess() -> *mut core::ffi::c_void;
    }
    let mut pmc: Counters = unsafe { std::mem::zeroed() };
    pmc.cb = std::mem::size_of::<Counters>() as u32;
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb) } != 0;
    ok.then_some((pmc.PrivateUsage as u64, (pmc.PeakPagefileUsage as u64).max(pmc.PrivateUsage as u64)))
}

/// True when one of this program's windows is in front and was typed or
/// clicked in during the last second: activity for the idle trim.
fn input_in_our_window() -> bool {
    use winapi::um::processthreadsapi::GetCurrentProcessId;
    use winapi::um::sysinfoapi::GetTickCount;
    use winapi::um::winuser::{GetForegroundWindow, GetLastInputInfo, GetWindowThreadProcessId, LASTINPUTINFO};
    // SAFETY: plain Win32 calls; the out parameters live for the calls.
    unsafe {
        let front = GetForegroundWindow();
        let mut pid = 0;
        if front.is_null() || GetWindowThreadProcessId(front, &mut pid) == 0 || pid != GetCurrentProcessId() {
            return false;
        }
        let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        GetLastInputInfo(&mut info) != 0 && GetTickCount().wrapping_sub(info.dwTime) < 1_000
    }
}

/// Gives the free memory of the heap back to Windows. Rust's allocator on
/// Windows is the process heap (there is no custom allocator), whose freed
/// blocks otherwise stay committed: `HeapOptimizeResources` (Windows 8.1
/// and later) decommits what the heaps keep free, and `HeapCompact`
/// coalesces the process heap's free blocks and decommits the big ones.
/// Neither touches live data. Only the process heap is compacted: other
/// heaps belong to system components, and may not be locked.
/// (`SetProcessWorkingSetSize(-1, -1)` is not used: it does not lower the
/// private bytes, and the next search would page the index back in.)
fn release_free_heap() {
    /// `HEAP_OPTIMIZE_RESOURCES_INFORMATION`.
    #[repr(C)]
    struct Optimize {
        version: u32,
        flags: u32,
    }
    /// `HeapOptimizeResources` in `HEAP_INFORMATION_CLASS`.
    const HEAP_OPTIMIZE_RESOURCES: i32 = 3;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetProcessHeap() -> *mut core::ffi::c_void;
        fn HeapCompact(heap: *mut core::ffi::c_void, flags: u32) -> usize;
        fn HeapSetInformation(heap: *mut core::ffi::c_void, class: i32, info: *mut core::ffi::c_void, len: usize) -> i32;
    }
    let mut info = Optimize { version: 1, flags: 0 };
    // SAFETY: documented calls with a valid heap handle (or null, meaning
    // every heap, for HeapOptimizeResources) and a structure of the size given.
    unsafe {
        HeapSetInformation(std::ptr::null_mut(), HEAP_OPTIMIZE_RESOURCES, (&mut info as *mut Optimize).cast(), std::mem::size_of::<Optimize>());
        let heap = GetProcessHeap();
        if !heap.is_null() {
            HeapCompact(heap, 0);
        }
    }
}

impl App {
    /// From the timer (five times a second); does its work at most every
    /// `mem_guard::SAMPLE_EVERY`.
    pub(super) fn memory_tick(&self) {
        let now = Instant::now();
        if input_in_our_window() {
            mem_guard::mark_active();
        }
        if let Some(at) = self.mem.resume_at.get()
            && now >= at
        {
            self.mem.resume_at.set(None);
            self.memory_resume();
        }
        if self.mem.sampled.get().is_some_and(|t| now.duration_since(t) < mem_guard::SAMPLE_EVERY) {
            return;
        }
        self.mem.sampled.set(Some(now));
        let Some(private) = private_bytes() else { return };
        let sample = {
            let st = self.state.borrow();
            Sample {
                private,
                entries: st.index.entries.len(),
                documents: super::content_index::opened().map_or(0, |c| c.documents_now()),
                queue: st.changes.len(),
            }
        };
        if self.mem.lined.get().is_none_or(|t| now.duration_since(t) >= mem_guard::LINE_EVERY) {
            self.mem.lined.set(Some(now));
            log(&sample.line());
        }
        let verdict = self.mem.guard.borrow_mut().decide(&sample, now);
        if let Some(gb) = verdict.warn_gb {
            log(&format!("memory: passed {gb} GB; {}", sample.line()));
        }
        if verdict.shed {
            self.memory_shed(&sample);
        } else if mem_guard::idle_trim_due(now, mem_guard::last_active(), self.background_quiet(), self.mem.trimmed.get()) {
            self.mem.trimmed.set(Some(now));
            self.memory_trim(&sample);
        }
    }

    /// Nothing runs in the background that the idle trim should wait for:
    /// the content index's warm-up has ended (or there is none), and no
    /// scan, index load or search is running.
    fn background_quiet(&self) -> bool {
        let st = self.state.borrow();
        st.warm.as_ref().is_none_or(|w| w.finished.load(Ordering::Relaxed))
            && st.scan.is_none()
            && st.loading.is_none()
            && !st.name_running
            && st.content.as_ref().is_none_or(|c| c.progress.done.load(Ordering::Relaxed))
    }

    /// The idle trim (see `crate::mem_guard`): lets go of what is cheap to
    /// rebuild and gives the free heap back to Windows. Never the index, its
    /// normalised names or the content index's map: the next search stays
    /// instant. Logs the private bytes before and after.
    fn memory_trim(&self, sample: &Sample) {
        if !self.thumbs_on() {
            self.thumbs.cache.replace(None);
        }
        self.webui_shed_thumbs();
        crate::folder_sizes::forget();
        {
            let mut st = self.state.borrow_mut();
            st.rows.shrink_to_fit();
            // Only while nothing else holds the index (a save, the new look's
            // list): the shown rows' own reference is put back as it was.
            let rows_share = Arc::ptr_eq(&st.rows_index, &st.index);
            if rows_share {
                st.rows_index = Arc::default();
            }
            if let Some(index) = Arc::get_mut(&mut st.index) {
                index.shrink();
            }
            if rows_share {
                st.rows_index = st.index.clone();
            }
        }
        // The rest on a thread of its own: the content index's maps are
        // shrunk under its lock, which a document search or a save of
        // `content.idx` may hold, and the heap is compacted, which takes the
        // heap's lock and walks it. Neither may keep the window from
        // answering. (Until 1.0 all of the trim ran here.)
        let (before, entries, documents) = (sample.private, sample.entries, sample.documents);
        let started = Instant::now();
        let spawned = crate::crash_log::spawn_named("idle-trim", move || {
            if let Some(c) = super::content_index::opened() {
                c.shrink();
            }
            release_free_heap();
            let after = private_bytes().unwrap_or(before);
            log(&format!("{} in {} ms", mem_guard::trim_line(before, after, entries, documents), started.elapsed().as_millis()));
        });
        if spawned.is_none() {
            log(&mem_guard::trim_line(before, private_bytes().unwrap_or(before), entries, documents));
        }
    }

    /// Far more memory than the index explains: lets go of everything that
    /// can be rebuilt, and watches the drives again (with a walk of each,
    /// paced as usual) once the guard's rest is over.
    fn memory_shed(&self, sample: &Sample) {
        log(&format!(
            "memory guard: private {} MB is over {}x the expected {} MB; stopped watching, dropped {} waiting changes and the caches; watching again in {} min",
            sample.private / (1024 * 1024),
            mem_guard::SHED_FACTOR,
            sample.expected() / (1024 * 1024),
            sample.queue,
            mem_guard::SHED_COOLDOWN.as_secs() / 60
        ));
        let was_watching = self.state.borrow().watcher.is_some();
        self.stop_watching();
        if was_watching {
            // Changes on disk go unseen until watching resumes: a start after
            // leaving meanwhile scans the drives again.
            super::files::SAVES.index_out_of_date();
        }
        if !self.thumbs_on() {
            self.thumbs.cache.replace(None);
        }
        self.webui_shed_thumbs();
        crate::folder_sizes::forget();
        if was_watching {
            self.mem.resume_at.set(Some(Instant::now() + mem_guard::SHED_COOLDOWN));
        }
    }

    fn memory_resume(&self) {
        // Not when watching was switched off meanwhile, nor while the index
        // is being scanned or loaded (that ends by watching again itself).
        {
            let st = self.state.borrow();
            if !st.settings.auto_index || st.scan.is_some() || st.loading.is_some() {
                log("memory guard: not watching again now (watching is off, or the index is being scanned or loaded)");
                return;
            }
        }
        self.start_watching();
        let roots = self.state.borrow().index.roots.clone();
        log(&format!("memory guard: watching again; walking {} (paced)", roots.join(" ")));
        for root in roots {
            self.request_rescan(root);
        }
    }
}
