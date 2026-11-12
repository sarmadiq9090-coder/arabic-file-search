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
    use winapi::um::processthreadsapi::GetCurrentProcess;
    use winapi::um::psapi::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
    let mut pmc: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
    pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc as *mut _ as *mut PROCESS_MEMORY_COUNTERS, pmc.cb) } != 0;
    ok.then_some(pmc.PrivateUsage as u64)
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
        if !self.thumbs_on() {
            self.thumbs.cache.replace(None);
        }
        crate::folder_sizes::forget();
        if was_watching {
            self.mem.resume_at.set(Some(Instant::now() + mem_guard::SHED_COOLDOWN));
        }
    }

    fn memory_resume(&self) {
        self.start_watching();
        let roots = self.state.borrow().index.roots.clone();
        log(&format!("memory guard: watching again; walking {} (paced)", roots.join(" ")));
        for root in roots {
            self.request_rescan(root);
        }
    }
}
