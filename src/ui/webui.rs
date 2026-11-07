//! The new main window (phase 1 of the new interface, opt-in while it is
//! built: `--webui` or `BAHTH_WEBUI=1`, see `crate::webui`).
//!
//! The program runs as always (index, drive watcher, tray icon, Ctrl+Alt+F
//! and its quick popup, the box above the clock, updates); only the main
//! window is replaced: the classic window stays hidden and a window holding
//! the page (`crate::webui::host`) takes its place and its title, so a second
//! launch brings it to the front. Without WebView2, in a mingw build, or when
//! the page can't be created (tried twice), the classic window is used as
//! before.

pub(super) use imp::{WebUi, wanted};

#[cfg(target_env = "msvc")]
mod imp {
    use native_windows_gui as nwg;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::{Arc, Mutex, OnceLock};

    use winapi::shared::minwindef::{LPARAM, UINT, WPARAM};
    use winapi::shared::windef::HWND;
    use winapi::um::winuser::{GetForegroundWindow, IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow, WA_INACTIVE, WM_ACTIVATE};

    use super::super::lazy::Lazy;
    use super::super::startup::log;
    use super::super::{App, WS_EX_LAYOUTRTL, autostart, shell, surface, title};
    use crate::webui::bridge::{self, List, Outbound, Params, Request, Shown};
    use crate::webui::host::{self, Host};

    /// Children of the window are not mirrored in Arabic (only its caption
    /// buttons are): WebView2 lays the page out right to left itself.
    const WS_EX_NOINHERITLAYOUT: u32 = 0x0010_0000;

    /// Whether this launch uses the new main window: it was asked for and
    /// WebView2 is installed. Decided once, before the windows are built.
    pub(in super::super) fn wanted() -> bool {
        static WANTED: OnceLock<bool> = OnceLock::new();
        *WANTED.get_or_init(|| {
            if !crate::webui::requested() {
                return false;
            }
            match host::runtime_version() {
                Some(v) => {
                    log(&format!("webui: WebView2 {v}"));
                    true
                }
                None => {
                    log("webui: asked for, but WebView2 is not installed; using the classic window");
                    false
                }
            }
        })
    }

    #[derive(Default)]
    pub(in super::super) struct Parts {
        window: nwg::Window,
        /// Wakes the UI thread when a background search finishes.
        notice: nwg::Notice,
    }

    #[derive(Default)]
    pub(in super::super) struct WebUi {
        // Dropped before the window that holds it.
        host: RefCell<Option<Host>>,
        parts: Lazy<Parts>,
        active: Cell<bool>,
        token: RefCell<String>,
        /// The page has loaded and said hello.
        ready: Cell<bool>,
        /// The finished background search, for the UI thread.
        inbox: Arc<Mutex<Option<(Shown, Outbound)>>>,
        running: Cell<bool>,
        /// The newest search asked for while one was running.
        pending: RefCell<Option<Params>>,
        /// What the page lists now, to act on its rows.
        shown: RefCell<Option<Shown>>,
        recent: RefCell<Vec<String>>,
        last_status: RefCell<String>,
        /// The classic search box's text last handed to the page (the quick
        /// popup's "open in the full window" puts its search there).
        handed: RefCell<String>,
    }

    impl App {
        /// Builds the new main window when it is wanted, right after the
        /// classic one (built hidden then). Returns its event handlers;
        /// `None` when the classic window is used.
        pub(in super::super) fn setup_webui(self: &Rc<Self>) -> Option<(nwg::EventHandler, nwg::RawEventHandler)> {
            if !wanted() {
                return None;
            }
            match self.build_webui() {
                Ok(handler) => Some(handler),
                Err(e) => {
                    log(&format!("webui: {e}; using the classic window"));
                    self.webui.active.set(false);
                    self.webui.host.replace(None);
                    self.window.set_text(title());
                    if let Some(h) = self.webui.parts.window.handle.hwnd() {
                        unsafe { winapi::um::winuser::DestroyWindow(h) };
                    }
                    if !started_hidden() {
                        self.window.set_visible(true);
                    }
                    None
                }
            }
        }

        fn build_webui(self: &Rc<Self>) -> Result<(nwg::EventHandler, nwg::RawEventHandler), String> {
            let (w, h, maximized) = {
                let s = &self.state.borrow().settings;
                (s.width as i32, s.height as i32, s.maximized)
            };
            let mut parts = Parts::default();
            let flags = if maximized { nwg::WindowFlags::MAIN_WINDOW | nwg::WindowFlags::MAXIMIZED } else { nwg::WindowFlags::MAIN_WINDOW };
            nwg::Window::builder()
                .title(title())
                .size((w, h))
                .center(true)
                .ex_flags(crate::i18n::rtl_only(WS_EX_LAYOUTRTL | WS_EX_NOINHERITLAYOUT))
                .flags(flags)
                .build(&mut parts.window)
                .map_err(|e| e.to_string())?;
            nwg::Notice::builder().parent(&parts.window).build(&mut parts.notice).map_err(|e| e.to_string())?;
            let hwnd = parts.window.handle.hwnd().ok_or("no window handle")?;
            crate::icon::set_window_icon(hwnd);
            self.webui.parts.set(parts);

            let token = bridge::new_token();
            self.webui.token.replace(token.clone());
            let dark = surface::apps_dark();
            let make = || {
                let weak = Rc::downgrade(self);
                Host::create(hwnd as isize, crate::i18n::current(), &token, dark, move |body, source| {
                    if let Some(app) = weak.upgrade() {
                        app.on_webui_message(body, source);
                    }
                })
            };
            // A WebView2 that fails to start once (its browser process
            // crashed, a busy profile) often starts the second time.
            let host = make().or_else(|e| {
                log(&format!("webui: first try failed: {e}"));
                make()
            })?;
            self.webui.host.replace(Some(host));
            self.webui.active.set(true);
            // The second launch looks for the main window by its title: let
            // it find this one, not the hidden classic window.
            self.window.set_text(&format!("{} (classic)", title()));

            let weak = Rc::downgrade(self);
            let handle = self.webui.parts.window.handle;
            let handler = nwg::full_bind_event_handler(
                &handle,
                crate::crash_log::guard_events(move |evt, data, h| {
                    let Some(app) = weak.upgrade() else { return };
                    let parts = &app.webui.parts;
                    match evt {
                        nwg::Event::OnNotice if h == parts.notice => app.on_webui_found(),
                        nwg::Event::OnWindowClose if h == parts.window => {
                            if let nwg::EventData::OnWindowClose(d) = &data {
                                d.close(false);
                            }
                            if app.close_hides() {
                                app.webui_hide();
                            } else {
                                app.quit();
                            }
                        }
                        _ => {}
                    }
                }),
            );
            // Shown by other means than `webui_show` (a second launch brings
            // it to the front): the page leaves its low-memory mode.
            let weak = Rc::downgrade(self);
            let raw = nwg::bind_raw_event_handler(
                &handle,
                0x1_0041,
                crate::crash_log::guard_raw(move |_h: HWND, msg: UINT, w: WPARAM, _l: LPARAM| {
                    if msg == WM_ACTIVATE && (w & 0xFFFF) as u16 != WA_INACTIVE {
                        let app = weak.upgrade()?;
                        // (Not while the page is being created or closed.)
                        if let Ok(host) = app.webui.host.try_borrow()
                            && let Some(host) = &*host
                        {
                            host.set_idle(false);
                        }
                    }
                    None
                }),
            );
            let raw = match raw {
                Ok(raw) => raw,
                Err(e) => {
                    nwg::unbind_event_handler(&handler);
                    return Err(e.to_string());
                }
            };
            if !started_hidden() {
                self.webui_show();
            }
            log("webui: main window ready");
            Ok((handler, raw))
        }

        /// Shows the new main window in front, if it is the one in use.
        pub(in super::super) fn webui_show(&self) -> bool {
            if !self.webui.active.get() {
                return false;
            }
            let window = &self.webui.parts.window;
            if let Some(h) = window.handle.hwnd() {
                surface::dwm_style(h, surface::apps_dark(), false, false);
                window.set_visible(true);
                unsafe {
                    if IsIconic(h) != 0 {
                        ShowWindow(h, SW_RESTORE);
                    }
                    SetForegroundWindow(h);
                }
            }
            if let Some(host) = &*self.webui.host.borrow() {
                host.set_idle(false);
                host.focus();
            }
            self.webui_hand_over_query();
            self.webui_send_recent();
            self.webui_send(&Outbound::Focus);
            true
        }

        /// The hotkey set to open the main window: shows it, or hides it
        /// when it is already in front.
        pub(in super::super) fn webui_toggle(&self) -> bool {
            if !self.webui.active.get() {
                return false;
            }
            let window = &self.webui.parts.window;
            let front = window.handle.hwnd().is_some_and(|h| unsafe { GetForegroundWindow() } == h);
            if front && window.visible() {
                self.webui_hide();
            } else {
                self.webui_show();
            }
            true
        }

        fn webui_hide(&self) {
            self.webui.parts.window.set_visible(false);
            if let Some(host) = &*self.webui.host.borrow() {
                host.set_idle(true);
            }
            if self.state.borrow().settings.tray_icon && !self.tray_hint_shown.replace(true) {
                self.tray.show(
                    tr!("البرنامج ما زال يعمل هنا. انقر الأيقونة أو شغّل البرنامج مرة أخرى لإظهاره فوراً.", "The program is still running here. Click the icon or start the program again to show it at once."),
                    Some(title()),
                    None,
                    None,
                );
            }
        }

        /// Called when the program closes: the page goes before its window.
        pub(in super::super) fn webui_close(&self) {
            self.webui.ready.set(false);
            self.webui.host.replace(None);
        }

        fn webui_send(&self, msg: &Outbound) {
            if !self.webui.ready.get() {
                return;
            }
            if let Ok(host) = self.webui.host.try_borrow()
                && let Some(host) = &*host
            {
                host.eval(&bridge::script(msg));
            }
        }

        /// The quick popup's search, when it asked for the full window.
        fn webui_hand_over_query(&self) {
            let text = self.query.text();
            if *self.webui.handed.borrow() == text {
                return;
            }
            if self.webui.ready.get() {
                self.webui.handed.replace(text.clone());
                if !text.trim().is_empty() {
                    self.webui_send(&Outbound::Query { q: text });
                }
            }
        }

        fn webui_send_recent(&self) {
            if !self.webui.ready.get() {
                return;
            }
            let (paths, rows) = {
                let st = self.state.borrow();
                let opens = st.history.recent_opens();
                bridge::recent_rows(&opens, |p| {
                    let m = std::fs::metadata(p).ok()?;
                    let mtime = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
                    Some((if m.is_dir() { 0 } else { m.len() }, mtime, m.is_dir()))
                })
            };
            self.webui.recent.replace(paths);
            self.webui_send(&Outbound::Recent { rows });
        }

        // ------------------------------------------------------------ messages

        fn on_webui_message(&self, body: &str, source: &str) {
            let req = match bridge::parse(body, source, &self.webui.token.borrow()) {
                Ok(r) => r,
                Err(why) => {
                    log(&format!("webui: refused a message ({why:?}) from {}", source.chars().take(80).collect::<String>()));
                    return;
                }
            };
            match req {
                Request::Hello => {
                    self.webui.ready.set(true);
                    self.webui.last_status.replace(String::new());
                    let digits = self.state.borrow().settings.arabic_digits;
                    let kinds = bridge::kinds(crate::i18n::current());
                    self.webui_send(&Outbound::Hello { kinds, digits, version: env!("CARGO_PKG_VERSION") });
                    self.webui_status(true);
                    self.webui_send_recent();
                    self.webui.handed.replace(String::new());
                    self.webui_hand_over_query();
                }
                Request::Search { seq, q, kind } => {
                    let params = self.webui_params(seq, q, kind);
                    self.webui_search(params);
                }
                Request::More { seq, from } => {
                    let rows = match &*self.webui.shown.borrow() {
                        Some(s) if s.params.seq == seq => s.page(from, bridge::MORE_PAGE),
                        _ => return,
                    };
                    self.webui_send(&Outbound::Rows { seq, from, rows });
                }
                Request::Open { list, seq, id } => {
                    if let Some(path) = self.webui_path(list, seq, id) {
                        self.webui_note_use(&path);
                        if !shell::open(&path) {
                            self.webui_send(&Outbound::Notice { text: trf!("تعذر فتح الملف: {}", "Could not open the file: {}", path) });
                        }
                    }
                }
                Request::Reveal { list, seq, id } => {
                    if let Some(path) = self.webui_path(list, seq, id) {
                        self.webui_note_use(&path);
                        shell::reveal(&path);
                    }
                }
                Request::Copy { list, seq, id } => {
                    if let Some(path) = self.webui_path(list, seq, id) {
                        nwg::Clipboard::set_data_text(&self.webui.parts.window, &path);
                        self.webui_send(&Outbound::Notice { text: tr!("تم نسخ المسار.", "Path copied.").to_string() });
                    }
                }
                Request::Web { q } => {
                    shell::open(&crate::suggest::web_url(&q));
                }
                Request::Hide => self.webui_hide(),
            }
        }

        /// The full path of row `id`, from what the page was sent.
        fn webui_path(&self, list: List, seq: u64, id: u32) -> Option<String> {
            match list {
                List::Results => {
                    let shown = self.webui.shown.borrow();
                    shown.as_ref().filter(|s| s.params.seq == seq).and_then(|s| s.path(id)).map(str::to_string)
                }
                List::Recent => self.webui.recent.borrow().get(id as usize).cloned(),
            }
        }

        /// A result was used: its search goes into the history and the file
        /// ranks higher next time (as in the classic window).
        fn webui_note_use(&self, path: &str) {
            let text = self.webui.shown.borrow().as_ref().map(|s| s.params.text.clone()).unwrap_or_default();
            let mut st = self.state.borrow_mut();
            if !text.trim().is_empty() {
                st.history.add_recent(&text);
            }
            st.history.record_open(path, crate::index::now_secs());
            st.history.save();
        }

        // ------------------------------------------------------------ search

        fn webui_params(&self, seq: u64, text: String, kind: usize) -> Params {
            let st = self.state.borrow();
            // Phase 1 searches all drives (the folder chip comes later).
            let scope = String::new();
            Params {
                seq,
                text,
                kind,
                show_system: crate::system_files::show_system(st.settings.hide_system, &scope),
                boosted: st.history.boosted(&st.index, crate::index::now_secs()),
                suggest: st.scan.is_none() && st.loading.is_none(),
                scope,
            }
        }

        /// Runs the search on a background thread. While one runs, newer
        /// requests fold into a single follow-up search.
        fn webui_search(&self, params: Params) {
            if self.webui.running.get() {
                self.webui.pending.replace(Some(params));
                return;
            }
            self.webui.running.set(true);
            let index = self.state.borrow().index.clone();
            let inbox = self.webui.inbox.clone();
            let notice = self.webui.parts.notice.sender();
            let lang = crate::i18n::current();
            let started = crate::crash_log::spawn_named("webui-search", move || {
                // After a panic (logged by the hook) the search still ends.
                let out = crate::crash_log::catch(|| bridge::search(index, params, lang));
                if let (Some(out), Ok(mut slot)) = (out, inbox.lock()) {
                    *slot = Some(out);
                }
                notice.notice();
            });
            if started.is_none() {
                self.webui.running.set(false);
            }
        }

        fn on_webui_found(&self) {
            self.webui.running.set(false);
            let found = self.webui.inbox.lock().ok().and_then(|mut s| s.take());
            let pending = self.webui.pending.take();
            if let Some((shown, msg)) = found {
                // A newer search is waiting: the page would drop this answer.
                if pending.as_ref().is_none_or(|p| p.seq == shown.params.seq) {
                    self.webui_send(&msg);
                }
                self.webui.shown.replace(Some(shown));
            }
            if let Some(p) = pending {
                self.webui_search(p);
            }
        }

        /// From the timer, about once a second: the indexing status, and the
        /// shown results kept in step with the index (finished scan, files
        /// changed on disk).
        pub(in super::super) fn webui_tick(&self, tick: u32) {
            if !self.webui.active.get() || !self.webui.ready.get() || !tick.is_multiple_of(5) {
                return;
            }
            self.webui_status(false);
            if self.webui.running.get() || !self.webui.parts.window.visible() {
                return;
            }
            let stale = {
                let st = self.state.borrow();
                let shown = self.webui.shown.borrow();
                shown.as_ref().filter(|s| !Arc::ptr_eq(&s.index, &st.index)).map(|s| (s.params.seq, s.params.text.clone(), s.params.kind))
            };
            if let Some((seq, text, kind)) = stale {
                let params = self.webui_params(seq, text, kind);
                self.webui_search(params);
            }
        }

        fn webui_status(&self, force: bool) {
            let ix = self.indexing();
            let msg = {
                let st = self.state.borrow();
                bridge::status(st.loading.is_some(), ix, st.index.entries.len(), crate::i18n::current())
            };
            let key = serde_json::to_string(&msg).unwrap_or_default();
            if force || *self.webui.last_status.borrow() != key {
                self.webui.last_status.replace(key);
                self.webui_send(&msg);
            }
        }
    }

    /// Started with Windows: wait in the tray until called.
    fn started_hidden() -> bool {
        std::env::args().any(|a| a == autostart::STARTUP_ARG)
    }
}

/// mingw builds have no WebView2 host (see Cargo.toml): always the classic window.
#[cfg(not(target_env = "msvc"))]
mod imp {
    use native_windows_gui as nwg;
    use std::rc::Rc;

    use super::super::App;

    pub(in super::super) fn wanted() -> bool {
        false
    }

    #[derive(Default)]
    pub(in super::super) struct WebUi;

    impl App {
        pub(in super::super) fn setup_webui(self: &Rc<Self>) -> Option<(nwg::EventHandler, nwg::RawEventHandler)> {
            if crate::webui::requested() {
                super::super::startup::log("webui: the new interface needs the MSVC build; using the classic window");
            }
            None
        }

        pub(in super::super) fn webui_show(&self) -> bool {
            false
        }

        pub(in super::super) fn webui_toggle(&self) -> bool {
            false
        }

        pub(in super::super) fn webui_close(&self) {}

        pub(in super::super) fn webui_tick(&self, _tick: u32) {}
    }
}
