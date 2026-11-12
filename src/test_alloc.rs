//! Test builds only: an allocator that can count, for one thread at a time,
//! the bytes it allocates that are still alive, whichever thread frees them.
//! `index_save`'s tests use it to prove how many copies of the index exist.
//! Outside [`peak_of`] it only passes requests on, to the capped counting
//! allocator of `extract::hostile` (whose child processes refuse to grow past
//! `BAHTH_TEST_MEM_CAP`), so both kinds of test share one global allocator.

use std::alloc::{GlobalAlloc, Layout};
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct Counting;

/// Where requests go: the system allocator, with live/peak bytes and the cap.
const INNER: crate::extract::hostile::Counting = crate::extract::hostile::Counting;

thread_local! {
    /// This thread's allocations are being counted.
    static ON: Cell<bool> = const { Cell::new(false) };
    /// Inside the bookkeeping itself (whose own allocations do not count).
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

/// Counted blocks that are alive: address to size.
static BLOCKS: Mutex<Option<HashMap<usize, usize>>> = Mutex::new(None);
static COUNTED: AtomicUsize = AtomicUsize::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn bookkeeping(f: impl FnOnce(&mut HashMap<usize, usize>)) {
    let Ok(busy) = BUSY.try_with(|b| b.replace(true)) else { return };
    if !busy {
        let mut blocks = BLOCKS.lock().unwrap_or_else(|e| e.into_inner());
        f(blocks.get_or_insert_with(HashMap::new));
    }
    let _ = BUSY.try_with(|b| b.set(busy));
}

fn added(p: *mut u8, size: usize) {
    if !p.is_null() && ON.try_with(Cell::get).unwrap_or(false) {
        bookkeeping(|blocks| {
            blocks.insert(p as usize, size);
            COUNTED.fetch_add(1, Ordering::SeqCst);
            let live = LIVE.fetch_add(size, Ordering::SeqCst) + size;
            PEAK.fetch_max(live, Ordering::SeqCst);
        });
    }
}

fn removed(p: *mut u8) {
    if COUNTED.load(Ordering::SeqCst) > 0 {
        bookkeeping(|blocks| {
            if let Some(size) = blocks.remove(&(p as usize)) {
                COUNTED.fetch_sub(1, Ordering::SeqCst);
                LIVE.fetch_sub(size, Ordering::SeqCst);
            }
        });
    }
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { INNER.alloc(layout) };
        added(p, layout.size());
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { INNER.alloc_zeroed(layout) };
        added(p, layout.size());
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        removed(p);
        unsafe { INNER.dealloc(p, layout) }
    }

    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let q = unsafe { INNER.realloc(p, layout, new_size) };
        if !q.is_null() {
            removed(p);
            added(q, new_size);
        }
        q
    }
}

/// Runs `f` with this thread's allocations counted and returns the most
/// bytes of them that were alive at once, with `f`'s result. One caller at
/// a time; whatever `f` allocated and still holds stops counting afterwards.
pub fn peak_of<R>(f: impl FnOnce() -> R) -> (usize, R) {
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    LIVE.store(0, Ordering::SeqCst);
    PEAK.store(0, Ordering::SeqCst);
    ON.with(|on| on.set(true));
    let result = f();
    ON.with(|on| on.set(false));
    bookkeeping(|blocks| {
        blocks.clear();
        COUNTED.store(0, Ordering::SeqCst);
    });
    (PEAK.load(Ordering::SeqCst), result)
}
