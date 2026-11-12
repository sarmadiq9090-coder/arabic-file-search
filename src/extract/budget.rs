//! Per-file budgets for one extraction, on the thread doing it: a time
//! budget, so a parser that turns out quadratic on some input gives up
//! instead of running for hours (and the file is stored as "no text", so
//! it is not tried again at every start), and an inflation budget shared by
//! every zip member read for the file, nested documents included.
//!
//! The checks are cooperative: the loops of our own parsers call [`tick`]
//! (cheap: the clock is read once every [`TICK_EVERY`] calls) and stop
//! when it says the time is up; [`inflate`] is charged by every bounded
//! read of decompressed data. Third-party parsers (pdf-extract, calamine)
//! are not interrupted, only what runs after them.

use std::cell::Cell;
use std::time::{Duration, Instant};

/// How long one file may take. Generous: OCR of a 50-page scan takes about
/// a second a page; everything else takes well under ten seconds.
pub const TIME: Duration = Duration::from_secs(120);
/// Decompressed bytes one file may read in all (its parts, the members of an
/// archive and the parts of documents inside it).
pub const INFLATE: u64 = 512 * 1024 * 1024;
/// The clock is read once per this many [`tick`]s.
const TICK_EVERY: u32 = 1024;

#[derive(Clone, Copy)]
struct State {
    deadline: Option<Instant>,
    left: u64,
    ticks: u32,
    over: bool,
}

const FREE: State = State { deadline: None, left: u64::MAX, ticks: 0, over: false };

thread_local! {
    static STATE: Cell<State> = const { Cell::new(FREE) };
    /// A shorter time limit for the next [`start`] (tests).
    static LIMIT: Cell<Option<Duration>> = const { Cell::new(None) };
}

/// The budgets of the extraction started on this thread, until dropped
/// (then the previous ones apply again).
pub struct Budget(State);

impl Drop for Budget {
    fn drop(&mut self) {
        STATE.with(|s| s.set(self.0));
    }
}

/// Starts the budgets of one file on this thread.
pub fn start() -> Budget {
    let time = LIMIT.with(Cell::get).unwrap_or(TIME);
    let fresh = State { deadline: Some(Instant::now() + time), left: INFLATE, ticks: 0, over: false };
    Budget(STATE.with(|s| s.replace(fresh)))
}

/// Tests: the extractions started on this thread until the guard is dropped
/// get `time` instead of [`TIME`].
#[cfg(test)]
pub fn with_limit(time: Duration) -> impl Drop {
    struct Restore(Option<Duration>);
    impl Drop for Restore {
        fn drop(&mut self) {
            LIMIT.with(|l| l.set(self.0));
        }
    }
    Restore(LIMIT.with(|l| l.replace(Some(time))))
}

/// True once this file is over its time (or inflation) budget: the caller
/// stops what it is doing.
pub fn tick() -> bool {
    STATE.with(|s| {
        let mut st = s.get();
        if st.over {
            return true;
        }
        let Some(deadline) = st.deadline else { return false };
        st.ticks += 1;
        if st.ticks >= TICK_EVERY {
            st.ticks = 0;
            st.over = Instant::now() >= deadline;
        }
        s.set(st);
        st.over
    })
}

/// How many more decompressed bytes this file may read, at most `want`.
pub fn room(want: u64) -> u64 {
    STATE.with(|s| s.get().left.min(want))
}

/// Charges `bytes` decompressed bytes; true when that used the budget up
/// (more was read than [`room`] allowed): the read is to be dropped.
pub fn inflate(bytes: u64) -> bool {
    STATE.with(|s| {
        let mut st = s.get();
        let over = bytes > st.left;
        st.left = st.left.saturating_sub(bytes);
        st.over |= over;
        s.set(st);
        over
    })
}

/// Moves this file's deadline `by` later: time spent waiting (for the OCR
/// permit, or while the user is busy) is not the file's own.
pub fn extend(by: Duration) {
    STATE.with(|s| {
        let mut st = s.get();
        st.deadline = st.deadline.map(|d| d + by);
        s.set(st);
    })
}

/// True when this file went over a budget.
pub fn exceeded() -> bool {
    STATE.with(|s| {
        let mut st = s.get();
        if !st.over && st.deadline.is_some_and(|d| Instant::now() >= d) {
            st.over = true;
            s.set(st);
        }
        st.over
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budgets_start_stop_and_nest() {
        assert!(!tick() && !exceeded(), "no budget outside an extraction");
        {
            let _b = start();
            assert!(!tick());
            assert_eq!(room(10), 10);
            assert!(!inflate(INFLATE - 5));
            assert_eq!(room(10), 5);
            assert!(inflate(6) && exceeded() && tick());
        }
        assert!(!exceeded() && room(7) == 7);
        let _l = with_limit(Duration::ZERO);
        let _b = start();
        std::thread::sleep(Duration::from_millis(2));
        assert!((0..TICK_EVERY).any(|_| tick()));
    }
}
