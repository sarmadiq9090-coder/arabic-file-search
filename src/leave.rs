//! Leaving the program in bounded time, whatever the disk is doing.
//!
//! Closing saves the settings and the search history, stops the watcher and
//! gives the last index save a short, fixed time (`index_save::flush_within`).
//! Each of these can meet a disk or a thread that does not answer, and until
//! 0.10.0 leaving then hung for as long as that lasted: with the window
//! frozen, with the new copy of an update waiting behind it (two processes
//! in Task Manager), or with Windows waiting to shut down. So every way of
//! leaving also starts a watchdog that ends the process when its time is up.
//! That is safe for the files: the index is written to a temp file and
//! renamed (`Index::save`), the unsaved changes are marked for the next start
//! (`index_save`), and the settings are written before anything that can be
//! slow.
//!
//! Free of Win32, so it is tested on every platform.

use std::time::Duration;

/// Why the program is leaving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leaving {
    /// The user closed it (Exit, or the close button without a tray icon).
    Exit,
    /// It starts again right away: after an update, a move to the permanent
    /// place or a change of language. The new copy is already waiting.
    Restart,
    /// Windows is shutting down or the user is signing out.
    SessionEnd,
}

/// How long leaving may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// The wait for the last index save (the saving thread writes it; the
    /// UI thread only waits, and only this long).
    pub save_wait: Duration,
    /// After this the process is ended, whatever is still going on.
    pub deadline: Duration,
}

impl Leaving {
    pub fn budget(self) -> Budget {
        let secs = |save_wait, deadline| Budget { save_wait: Duration::from_secs(save_wait), deadline: Duration::from_secs(deadline) };
        match self {
            Leaving::Exit => secs(8, 10),
            Leaving::Restart => secs(2, 5),
            // Windows gives a program about five seconds.
            Leaving::SessionEnd => secs(3, 5),
        }
    }
}

/// Starts the watchdog: `end_process` runs on its own thread once `deadline`
/// has passed. False when the thread could not be started.
pub fn watchdog(deadline: Duration, end_process: impl FnOnce() + Send + 'static) -> bool {
    crate::crash_log::spawn_named("exit-watchdog", move || {
        std::thread::sleep(deadline);
        end_process();
    })
    .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::Index;
    use crate::index_save::{Flush, SaveQueue};
    use std::sync::mpsc;
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Instant;

    #[test]
    fn every_way_of_leaving_is_bounded() {
        for kind in [Leaving::Exit, Leaving::Restart, Leaving::SessionEnd] {
            let b = kind.budget();
            assert!(b.save_wait < b.deadline, "{kind:?}: the save must give up before the watchdog");
            assert!(b.deadline <= Duration::from_secs(10), "{kind:?}");
        }
        assert!(Leaving::SessionEnd.budget().deadline <= Duration::from_secs(5));
        assert!(Leaving::Restart.budget().deadline <= Duration::from_secs(5));
    }

    /// Leaving with the last save stuck on the disk and, on top of it, a
    /// step of closing that never returns: the wait for the save ends on
    /// time, and the watchdog ends the process at its deadline. (Scaled: 60
    /// and 150 ms stand for the seconds of a real budget.)
    #[test]
    fn leaving_ends_on_time_with_a_blocked_save_and_a_stuck_step() {
        // A disk that never finishes a write.
        let stuck = Arc::new((Mutex::new(false), Condvar::new()));
        let s = stuck.clone();
        let queue = SaveQueue::new(
            Box::new(move |_| {
                let mut open = s.0.lock().unwrap();
                while !*open {
                    open = s.1.wait(open).unwrap();
                }
                Ok(())
            }),
            Box::new(|_| {}),
            Box::new(|_| {}),
            || {},
            0,
            Duration::ZERO,
        );
        queue.set_pending(Arc::new(Index::default()), Duration::from_secs(3600));

        let budget = Budget { save_wait: Duration::from_millis(60), deadline: Duration::from_millis(150) };
        let started = Instant::now();
        let (ended, at_end) = mpsc::channel();
        assert!(watchdog(budget.deadline, move || ended.send(Instant::now()).unwrap()));
        // The closing sequence, on this thread: the save does not finish...
        assert_eq!(queue.flush_within(budget.save_wait), Flush::TimedOut);
        let after_flush = started.elapsed();
        assert!(after_flush >= budget.save_wait && after_flush < budget.deadline, "{after_flush:?}");
        // ...and then something hangs for good. The watchdog still fires.
        let end = at_end.recv_timeout(Duration::from_secs(20)).expect("the watchdog ends the process");
        let took = end.duration_since(started);
        assert!(took >= budget.deadline, "{took:?}");
        assert!(took < budget.deadline + Duration::from_secs(5), "{took:?}");
        // (Let the stuck write go, so the test leaves no thread behind.)
        *stuck.0.lock().unwrap() = true;
        stuck.1.notify_all();
    }
}
