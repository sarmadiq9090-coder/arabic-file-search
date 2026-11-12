//! The memory guard's side in the window (`crate::mem_guard` decides):
//! samples the process every half minute from the timer, writes the
//! `memory:` line every ten minutes, and sheds what can be rebuilt when the
//! guard says so.

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

impl App {
    /// From the timer (five times a second); does its work at most every
    /// `mem_guard::SAMPLE_EVERY`.
    pub(super) fn memory_tick(&self) {
        let now = Instant::now();
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
                documents: super::content_index::opened().map_or(0, |c| c.documents()),
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
