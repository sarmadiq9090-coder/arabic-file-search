//! Keeping startup quick: the secondary windows (duplicates, replace,
//! folder sizes, search from phone) are created the first time they open,
//! not at startup; their menu items are still built at startup. Handlers
//! that only need the main window's events are bound to it alone.

use native_windows_gui as nwg;
use std::cell::{OnceCell, RefCell};
use std::ops::Deref;
use std::rc::Rc;
use std::time::Instant;

use super::App;
use super::startup::log;

/// The controls of a window created on first use. Until then it reads as
/// controls without a handle, which never match an event's handle.
#[derive(Default)]
pub struct Lazy<T> {
    built: OnceCell<T>,
    empty: T,
}

impl<T> Lazy<T> {
    pub fn is_built(&self) -> bool {
        self.built.get().is_some()
    }

    pub fn set(&self, value: T) {
        let _ = self.built.set(value);
    }
}

impl<T> Deref for Lazy<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.built.get().unwrap_or(&self.empty)
    }
}

/// Event handlers bound to the windows created on first use; unbound with
/// the main window's handlers.
#[derive(Default)]
pub struct LazyHandlers {
    events: RefCell<Vec<nwg::EventHandler>>,
    raw: RefCell<Vec<nwg::RawEventHandler>>,
}

impl LazyHandlers {
    pub fn unbind(&self) {
        for h in self.events.borrow_mut().drain(..) {
            nwg::unbind_event_handler(&h);
        }
        for h in self.raw.borrow_mut().drain(..) {
            let _ = nwg::unbind_raw_event_handler(&h);
        }
    }
}

pub(super) type Bound = (Vec<nwg::EventHandler>, Vec<nwg::RawEventHandler>);

impl App {
    /// Creates a window and binds its events the first time it is needed.
    /// False if that failed (logged); the caller then doesn't show it.
    pub(super) fn build_lazily(self: &Rc<Self>, name: &str, build: impl FnOnce(&Rc<Self>) -> Result<Bound, nwg::NwgError>) -> bool {
        let t = Instant::now();
        match build(self) {
            Ok((events, raw)) => {
                self.lazy_handlers.events.borrow_mut().extend(events);
                self.lazy_handlers.raw.borrow_mut().extend(raw);
                log(&format!("{name} window built in {} ms", t.elapsed().as_millis()));
                true
            }
            Err(e) => {
                log(&format!("{name} window: {e}"));
                false
            }
        }
    }

    /// Binds `f` to the main window's own events only: menu items, timer,
    /// notices and list notifications. `full_bind_event_handler` also hooks
    /// every control, and each hook slows every message the control gets,
    /// which added up to a slower layout at startup. nwg wants a second
    /// handle: the status bar, whose own events these handlers don't use.
    pub(super) fn bind_main_events(&self, f: impl Fn(nwg::Event, nwg::EventData, nwg::ControlHandle) + 'static) -> nwg::EventHandler {
        nwg::bind_event_handler(&self.status.handle, &self.window.handle, crate::crash_log::guard_events(f))
    }
}
