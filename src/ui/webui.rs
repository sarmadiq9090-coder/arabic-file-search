//! The new main window (the new interface; since 1.0 the only look users
//! get, see `crate::webui`; `--classic` / `BAHTH_CLASSIC=1` give the tests
//! the classic window).
//!
//! The program runs as always (index, drive watcher, tray icon, Ctrl+Alt+F
//! and its quick popup, the box above the clock, updates); only the main
//! window is replaced: the classic window stays hidden and a window holding
//! the page (`crate::webui::host`) takes its place and its title, so a second
//! launch brings it to the front. Without WebView2, in a mingw build, or when
//! the page can't be created (tried twice) or fails twice, or when it never
//! said hello at the last two starts (a hard crash, which nothing else
//! remembers: `crate::webui::start_look`), the classic window
//! takes over for this run only, with a line saying why: the next start
//! tries the new look again (quickly giving up if it still fails,
//! `crate::webui::start_tolerance`), and «جرّب الشكل الجديد» in its Options
//! menu tries it at once.
//!
//! Phase 2 brings the classic window's features to the page: search inside
//! documents (snippets, match counts, every matching line), the size, date
//! and folder filters, sorting, the right-click actions (copy the file,
//! rename, Recycle Bin, search in this folder…), recent and saved searches,
//! and the settings. Every action names a row the page was sent, never a
//! path (`crate::webui::bridge`).

pub(super) use imp::{WebUi, available, wanted};

/// How a search handed over from the quick popup runs in the main window
/// (its suggestions for a search that found nothing).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct Handover {
    /// Inside documents too.
    pub content: bool,
    /// Every file type.
    pub all_kinds: bool,
    /// Every drive, not the folder chosen.
    pub all_drives: bool,
}

#[cfg(target_env = "msvc")]
mod imp {
    use native_windows_gui as nwg;
    use std::cell::{Cell, RefCell};
    use std::rc::{Rc, Weak};
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::{Duration, Instant};

    use winapi::shared::minwindef::{LPARAM, UINT, WPARAM};
    use winapi::shared::windef::HWND;
    use winapi::um::winuser::{EnableWindow, GetForegroundWindow, IsIconic, IsWindowEnabled, SW_RESTORE, SetForegroundWindow, ShowWindow, WA_INACTIVE, WM_ACTIVATE};

    use super::super::lazy::Lazy;
    use super::super::startup::log;
    use super::super::{App, WS_EX_LAYOUTRTL, autostart, content_index, shell, surface, title};
    use crate::history::SavedSearch;
    use crate::i18n;
    use crate::search::{self, ContentHit, ContentProgress};
    use crate::settings::{FloatingMode, HotkeyOpens};
    use crate::webui::bridge::{self, ContentState, List, Lists, Miss, Outbound, Params, Request, Setting, SettingsOut, Shown, Sort};
    use crate::webui::host::{self, Host};

    /// Children of the window are not mirrored in Arabic (only its caption
    /// buttons are): WebView2 lays the page out right to left itself.
    const WS_EX_NOINHERITLAYOUT: u32 = 0x0010_0000;

    /// Microsoft's free WebView2 download page, offered by the classic
    /// window's line when WebView2 is not installed.
    const WEBVIEW2_DOWNLOAD: &str = "https://developer.microsoft.com/microsoft-edge/webview2/";

    /// A page that has not said hello this long after it was created or
    /// reloaded is taken as failed (a blank window).
    const PAGE_TIMEOUT: Duration = Duration::from_secs(20);

    /// Whether WebView2 is installed, so the new main window can be used.
    /// Asked once.
    pub(in super::super) fn available() -> bool {
        static AVAILABLE: OnceLock<bool> = OnceLock::new();
        *AVAILABLE.get_or_init(|| match host::runtime_version() {
            Some(v) => {
                log(&format!("webui: WebView2 {v}"));
                true
            }
            None => {
                log("webui: WebView2 is not installed");
                false
            }
        })
    }

    /// Whether this launch starts with the new main window: it was chosen
    /// (`new_ui` in the settings) or asked for (`--webui`, `BAHTH_WEBUI`),
    /// and WebView2 is installed. Asked before the windows are built.
    pub(in super::super) fn wanted(new_ui: bool) -> bool {
        crate::webui::chosen(new_ui) && available()
    }

    #[derive(Default)]
    pub(in super::super) struct Parts {
        window: nwg::Window,
        /// Wakes the UI thread when a background search finishes.
        notice: nwg::Notice,
        /// Wakes the UI thread to start a drag the page asked for, right
        /// after WebView2's message handler returns.
        drag_notice: nwg::Notice,
        /// «جارٍ الفتح…» while the page is created (the window shows at once).
        opening: nwg::Label,
        opening_font: nwg::Font,
    }

    /// A search inside documents running on its own threads.
    struct ContentRun {
        seq: u64,
        progress: Arc<ContentProgress>,
        hits: Arc<Mutex<Vec<ContentHit>>>,
        /// The last progress text sent.
        said: String,
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
        /// The search inside documents under way, if any.
        content: RefCell<Option<ContentRun>>,
        /// What the page lists now (results, recent files), with their
        /// versions, to act on its rows; and the search waiting to run.
        lists: RefCell<Lists>,
        /// The folder searched, "" for all drives (the page never sets it).
        scope: RefCell<String>,
        /// The language chosen for the next start, when it was changed.
        next_lang: Cell<Option<&'static crate::i18n::Locale>>,
        last_status: RefCell<String>,
        /// The classic search box's text last handed to the page (the quick
        /// popup's "open in the full window" puts its search there).
        handed: RefCell<String>,
        /// The page asked for the classic window: its WebView2 is dropped on
        /// the next tick (not inside its own message handler).
        leaving: Cell<bool>,
        /// A switch to the new window is under way (creating the page runs
        /// a message loop).
        switching: Cell<bool>,
        /// The program, for the timer to create the page again.
        me: RefCell<Weak<App>>,
        /// A WebView2 process failed (true: the whole browser), seen on the
        /// next tick (not inside WebView2's own handler).
        failed: Cell<Option<bool>>,
        /// Failures since the new look was started: the first is tried
        /// again, the second hands over to the classic window for this run.
        failures: Cell<u32>,
        /// The new look failed at the last start: WebView2 is created at
        /// one try, not two (`crate::webui::start_tolerance`).
        one_try: Cell<bool>,
        /// This run saved `new_ui_trying` before creating its first page
        /// (`crate::webui::start_look`); the page's hello clears it.
        trying_saved: Cell<bool>,
        /// The classic window is in use because the new look can't run
        /// (no WebView2, or it failed): the line saying so above its
        /// buttons (`place_webui_note`).
        simple: Cell<bool>,
        note: Lazy<nwg::Label>,
        /// When the page was created or reloaded, until it says hello.
        loading_since: Cell<Option<Instant>>,
        /// When the page last said hello, until it fails.
        ready_since: Cell<Option<Instant>>,
        /// The window is on screen without its page: created on the next tick.
        host_pending: Cell<bool>,
        /// The launch-to-window time was logged.
        shown_logged: Cell<bool>,
        /// This start made the new look the default for a classic user: the
        /// page says so once (`Outbound::Hello`).
        tell_switch: Cell<bool>,
        /// The apps offered for a search: (its seq, its text, the apps).
        apps: RefCell<(u64, String, Vec<crate::launcher::Item>)>,
        /// The file a drag the page started is for, until the drag starts.
        drag: RefCell<Option<String>>,
        /// The grid view's thumbnails: one-time keys and the pictures.
        thumbs: super::super::webui_thumbs::ThumbBroker,
        /// What the page searched last: given back to a page loaded again
        /// (after a renderer failure) and to the classic window on going back.
        page_query: RefCell<String>,
        /// A dialog runs inside the page's own message handler (the folder
        /// dialog): the timer must not drop or create the page meanwhile.
        in_page_call: Cell<bool>,
        /// The page asked for the Options menu: opened on the next tick, not
        /// inside WebView2's handler (the menu runs a message loop).
        options_pending: Cell<bool>,
        /// An Options item may have changed a setting the page shows.
        settings_stale: Cell<bool>,
        /// The page asked to check for updates: done on the next tick.
        updates_pending: Cell<bool>,
        /// The program is restarting from the page's own message handler:
        /// the page is left to the process exit, not dropped inside it.
        exiting: Cell<bool>,
        /// A Save/Open dialog waits since Ctrl+Alt+G (`super::super::quickswitch`):
        /// the hint the page shows meanwhile.
        quick_switch: RefCell<Option<String>>,
        /// A search the quick popup handed over (its text and how to run
        /// it), sent once the page is there.
        handover: RefCell<Option<(String, super::Handover)>>,
        /// Since when the window has been hidden (to the tray) without a
        /// break; `None` while on screen (`crate::webui::idle_action`).
        hidden_since: Cell<Option<Instant>>,
        /// The hidden page was asked to suspend (once per hide).
        suspend_asked: Cell<bool>,
        /// The page was closed on purpose after a long while hidden, to free
        /// WebView2's memory: not a failure, and the next show creates it again.
        released: Cell<bool>,
        /// When the window was shown to create a released page again, until
        /// that page says hello.
        recreate_since: Cell<Option<Instant>>,
        /// What the page searched last, with its filters.
        page_search: RefCell<Option<bridge::Restore>>,
        /// `page_search` when the page was released, for the next page.
        restore: RefCell<Option<bridge::Restore>>,
    }

    impl App {
        /// Builds the new main window when it is wanted, right after the
        /// classic one (built hidden then). Returns its event handlers;
        /// `None` when the classic window is used from the start.
        pub(in super::super) fn setup_webui(self: &Rc<Self>) -> Option<(nwg::EventHandler, nwg::RawEventHandler)> {
            let (new_ui, failed_last_time, trying) = {
                let s = &self.state.borrow().settings;
                (s.new_ui, s.new_ui_failed, s.new_ui_trying)
            };
            if crate::webui::classic_requested() {
                log("webui: the classic window was asked for (--classic or BAHTH_CLASSIC)");
            }
            if !wanted(new_ui) {
                // No WebView2 (`available` logged it): the classic window,
                // saying why; the next start looks again.
                if crate::webui::chosen(new_ui) {
                    self.webui_note(true);
                }
                return None;
            }
            // The page never said hello at the last two starts: most likely
            // a hard crash, which no fallback remembered. The classic window
            // for this run; the next start tries the new look again.
            let careful = match crate::webui::start_look(trying, failed_last_time) {
                crate::webui::StartLook::Classic => {
                    log(&format!("webui: the page did not say hello at the last {trying} starts (the program ended first); using the classic window for this run"));
                    self.state.borrow_mut().settings.new_ui_trying = crate::webui::TRYING_AFTER_CLASSIC;
                    self.webui_simple_look();
                    return None;
                }
                crate::webui::StartLook::New { careful } => careful,
            };
            // After a fallback at the last start: tried again, but giving
            // up quickly if it still fails.
            let (attempts, counted) = crate::webui::start_tolerance(careful);
            self.webui.one_try.set(attempts < 2);
            self.webui.failures.set(counted);
            if failed_last_time {
                log("webui: the new look failed at the last start; trying it again, once");
            } else if careful {
                log("webui: the page did not say hello at the last start (the program ended first); trying the new look again, once");
            }
            let handlers = match self.build_webui_window() {
                Ok(handlers) => handlers,
                Err(e) => {
                    log(&format!("webui: {e}; using the classic window"));
                    self.webui_fall_back(!started_hidden());
                    return None;
                }
            };
            // The window is the main one now; its page is created when it is
            // first shown (`webui_show`, then the next tick): a start in the
            // tray creates no WebView2 at all, and a start on screen shows
            // the window with «جارٍ الفتح…» at once instead of nothing.
            self.webui.me.replace(Rc::downgrade(self));
            self.webui.active.set(true);
            self.webui_titles();
            log("webui: main window ready");
            if started_hidden() {
                log("webui: started in the tray; the page is created when the window is first shown");
            } else {
                self.webui_show();
            }
            Some(handlers)
        }

        /// The classic window after the new one failed to start, for this
        /// run only (`webui_simple_look`).
        fn webui_fall_back(&self, show: bool) {
            self.webui.active.set(false);
            self.webui.host.replace(None);
            self.webui.parts.window.set_visible(false);
            self.webui_titles();
            if show {
                self.window.set_visible(true);
            }
            self.webui_simple_look();
        }

        /// The classic window has taken over because the new look failed:
        /// it says so, and the next start tries the new look again, giving
        /// up quickly if it still fails (`Settings::note_fallback`; the new
        /// look is never turned off).
        fn webui_simple_look(&self) {
            {
                let s = &mut self.state.borrow_mut().settings;
                s.note_fallback();
                // An orderly fallback: this run's page did not crash the program.
                if self.webui.trying_saved.get() {
                    s.new_ui_trying = 0;
                }
            }
            self.save_settings();
            self.webui_note(true);
        }

        /// Shows or hides the line that the classic window is used because
        /// WebView2 is not working (built the first time it shows).
        fn webui_note(&self, show: bool) {
            if self.webui.simple.replace(show) == show {
                return;
            }
            if show && !self.webui.note.is_built() {
                // Not installed at all: the line offers Microsoft's free
                // download page, opened in the browser on a click.
                let missing = !available();
                let mut label = nwg::Label::default();
                let built = nwg::Label::builder()
                    .parent(&self.window)
                    .font(Some(&self.font))
                    .v_align(nwg::VTextAlign::Center)
                    .text(if missing {
                        tr!(
                            "يعمل البرنامج بالشكل البسيط لأن مكوّن WebView2 غير مثبّت. انقر هنا لتنزيله مجاناً من Microsoft.",
                            "Running in the simple look because Windows' WebView2 component is not installed. Click here to download it free from Microsoft."
                        )
                    } else {
                        tr!(
                            "يعمل البرنامج بالشكل البسيط لأن مكوّن WebView2 في ويندوز لا يعمل.",
                            "Running in the simple look because Windows' WebView2 component is not working."
                        )
                    })
                    .build(&mut label);
                match built {
                    Ok(()) => {
                        if missing {
                            let at = label.handle;
                            // (Kept bound for the run, as the line itself.)
                            let _ = self.bind_main_events(move |evt, _data, handle| {
                                if evt == nwg::Event::OnLabelClick && handle == at && !shell::open(WEBVIEW2_DOWNLOAD) {
                                    log("webui: could not open the WebView2 download page");
                                }
                            });
                        }
                        self.webui.note.set(label);
                    }
                    Err(e) => log(&format!("webui: the note could not be shown: {e}")),
                }
            }
            if self.webui.note.is_built() {
                self.webui.note.set_visible(show);
            }
            self.layout();
        }

        /// Lays out the line of `webui_note` (when shown) in the classic
        /// window, ending at `bottom`; returns the new bottom for the list.
        pub(in super::super) fn place_webui_note(&self, x: i32, bottom: i32, w: i32) -> i32 {
            use super::super::style::{self, CTRL_H, GAP};
            if !self.webui.simple.get() || !self.webui.note.is_built() {
                return bottom;
            }
            let y = bottom - CTRL_H;
            style::place(&self.webui.note.handle, x, y, w.max(0), CTRL_H);
            y - GAP
        }

        /// The window that holds the page, and its event handlers.
        fn build_webui_window(self: &Rc<Self>) -> Result<(nwg::EventHandler, nwg::RawEventHandler), String> {
            let (w, h, maximized) = {
                let s = &self.state.borrow().settings;
                (s.width as i32, s.height as i32, s.maximized)
            };
            let mut parts = Parts::default();
            let flags = if maximized { nwg::WindowFlags::MAIN_WINDOW | nwg::WindowFlags::MAXIMIZED } else { nwg::WindowFlags::MAIN_WINDOW };
            // Centred in the work area (the monitor without the taskbar, on
            // whichever side it is), not on the whole screen, and shrunk to
            // fit it, as the classic window (`placement`): on 1366×768, or
            // 1080p at 150 %, the footer was under the taskbar.
            let (size, position) = super::super::placement::opening(w.max(0) as u32, h.max(0) as u32);
            nwg::Window::builder()
                .title(&format!("{} (new)", title()))
                .size(size)
                .position(position)
                .ex_flags(crate::i18n::rtl_only(WS_EX_LAYOUTRTL | WS_EX_NOINHERITLAYOUT))
                .flags(flags)
                .build(&mut parts.window)
                .map_err(|e| e.to_string())?;
            nwg::Notice::builder().parent(&parts.window).build(&mut parts.notice).map_err(|e| e.to_string())?;
            nwg::Notice::builder().parent(&parts.window).build(&mut parts.drag_notice).map_err(|e| e.to_string())?;
            nwg::Font::builder().family(super::super::startup::ui_font_family()).size_absolute(20).build(&mut parts.opening_font).map_err(|e| e.to_string())?;
            let (cw, ch) = parts.window.size();
            nwg::Label::builder()
                .parent(&parts.window)
                .text(tr!("جارٍ الفتح…", "Opening…"))
                .font(Some(&parts.opening_font))
                .h_align(nwg::HTextAlign::Center)
                .v_align(nwg::VTextAlign::Center)
                .position((0, 0))
                .size((cw as i32, ch as i32))
                .build(&mut parts.opening)
                .map_err(|e| e.to_string())?;
            let hwnd = parts.window.handle.hwnd().ok_or("no window handle")?;
            crate::icon::set_window_icon(hwnd);
            // With the real frame, on the monitor it opened on.
            super::super::placement::keep_on_screen(hwnd);
            self.webui.parts.set(parts);

            let weak = Rc::downgrade(self);
            let handle = self.webui.parts.window.handle;
            let handler = nwg::full_bind_event_handler(
                &handle,
                crate::crash_log::guard_events(move |evt, data, h| {
                    let Some(app) = weak.upgrade() else { return };
                    let parts = &app.webui.parts;
                    match evt {
                        nwg::Event::OnNotice if h == parts.notice => app.on_webui_found(),
                        nwg::Event::OnNotice if h == parts.drag_notice => app.webui_drag(),
                        // An item of the Options menu opened over this window
                        // (`webui_options_menu`): it acts in the classic
                        // window's handlers, where every item is wired.
                        nwg::Event::OnMenuItemSelected => app.webui_forward_menu(h),
                        nwg::Event::OnResize | nwg::Event::OnWindowMaximize if h == parts.window => {
                            let (w, ht) = parts.window.size();
                            parts.opening.set_size(w, ht);
                        }
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
            // it to the front): the page leaves its low-memory mode. Every
            // time it shows, and when it comes back from the taskbar, it is
            // kept inside the work area of its monitor: the screen may have
            // changed while it waited (a monitor unplugged, another
            // resolution or scale, the taskbar moved).
            let weak = Rc::downgrade(self);
            let minimized = Cell::new(false);
            let raw = nwg::bind_raw_event_handler(
                &handle,
                0x1_0041,
                crate::crash_log::guard_raw(move |h: HWND, msg: UINT, w: WPARAM, l: LPARAM| {
                    use winapi::um::winuser::{SIZE_MINIMIZED, SIZE_RESTORED, WM_SETTINGCHANGE, WM_SHOWWINDOW, WM_SIZE};
                    // Windows switched light/dark: the title bar of "Automatic" follows (the page by itself).
                    if msg == WM_SETTINGCHANGE && surface::is_theme_change(l) {
                        weak.upgrade()?.webui_title_bar();
                    }
                    if msg == WM_SHOWWINDOW && w != 0 {
                        super::super::placement::keep_on_screen(h);
                        // (Shown by any means: the page is visible and resumed.)
                        if let Some(app) = weak.upgrade() {
                            app.webui_back_on_screen();
                        }
                    }
                    if msg == WM_SIZE {
                        if w == SIZE_MINIMIZED {
                            minimized.set(true);
                        } else if w == SIZE_RESTORED && minimized.replace(false) {
                            super::super::placement::keep_on_screen(h);
                        }
                    }
                    if msg == WM_ACTIVATE && (w & 0xFFFF) as u16 != WA_INACTIVE {
                        weak.upgrade()?.webui_back_on_screen();
                    }
                    None
                }),
            );
            match raw {
                Ok(raw) => Ok((handler, raw)),
                Err(e) => {
                    nwg::unbind_event_handler(&handler);
                    Err(e.to_string())
                }
            }
        }

        /// Fills the window with a new page (with a new token) and makes it
        /// the main window. Not called from the page's own message handler:
        /// it drops the page there was.
        fn start_webui(self: &Rc<Self>) -> Result<(), String> {
            if crate::webui::fail_requested() {
                return Err(format!("{} is set (a test)", crate::webui::FAIL_ENV));
            }
            let hwnd = self.webui.parts.window.handle.hwnd().ok_or("no window handle")?;
            self.webui.me.replace(Rc::downgrade(self));
            self.webui.leaving.set(false);
            self.webui.released.set(false);
            self.webui.ready.set(false);
            self.webui.failed.set(None);
            self.webui.loading_since.set(None);
            self.webui.ready_since.set(None);
            self.webui.host.replace(None);
            let token = bridge::new_token();
            self.webui.token.replace(token.clone());
            let dark = self.webui_dark();
            // Creating the page runs a message loop: a second launch then
            // must not find the classic window by its title and show it too.
            self.window.set_text(&format!("{} (classic)", title()));
            let make = || {
                let weak = Rc::downgrade(self);
                let on_failed = {
                    let weak = Rc::downgrade(self);
                    move |browser_gone: bool| {
                        // (Not the page closed on purpose, `webui_release`.)
                        if let Some(app) = weak.upgrade()
                            && app.webui.active.get()
                            && !app.webui.released.get()
                        {
                            log(&format!("webui: a WebView2 process failed (browser: {browser_gone})"));
                            let before = app.webui.failed.get().unwrap_or(false);
                            app.webui.failed.set(Some(before || browser_gone));
                        }
                    }
                };
                let on_message = move |body: &str, source: &str| {
                    if let Some(app) = weak.upgrade() {
                        app.on_webui_message(body, source);
                    }
                };
                let broker = self.webui.thumbs.clone();
                Host::create(hwnd as isize, crate::i18n::current(), &token, dark, on_message, on_failed, move |key: &str, reply| broker.serve(key, reply))
            };
            // On disk before WebView2 is created (once per run): a hard crash
            // from here to the page's hello is then seen by the next start.
            if !self.webui.trying_saved.replace(true) {
                let mut st = self.state.borrow_mut();
                st.settings.new_ui_trying = crate::webui::trying_before_page(st.settings.new_ui_trying);
                drop(st);
                self.save_settings();
            }
            // A WebView2 that fails to start once (its browser process
            // crashed, a busy profile) often starts the second time; not
            // after a fallback at the last start (`one_try`).
            let host = make().or_else(|e| {
                log(&format!("webui: first try failed: {e}"));
                if self.webui.one_try.get() { Err(e) } else { make() }
            })?;
            self.webui.host.replace(Some(host));
            self.webui.active.set(true);
            self.webui.loading_since.set(Some(Instant::now()));
            self.webui_titles();
            Ok(())
        }

        /// A second launch looks for the main window by its title: let it
        /// find the one in use, not the hidden one.
        fn webui_titles(&self) {
            let active = self.webui.active.get();
            self.window.set_text(&if active { format!("{} (classic)", title()) } else { title().to_string() });
            if self.webui.parts.is_built() {
                self.webui.parts.window.set_text(&if active { title().to_string() } else { format!("{} (new)", title()) });
            }
            // The Options menu (the tray's "Options…" opens it while the new
            // look is on screen too) offers «جرّب الشكل الجديد» only in the
            // classic window: there is no way back from the new look.
            // The thumbnail view is the classic list's.
            self.enable_thumbs_item(!active);
            show_menu_items(&[(&self.menu_new_ui.handle, Some(tr!("جرّب الشكل الجديد", "Try the new look"))), (&self.menu_new_ui_sep.handle, None)], !active);
        }

        /// The Options menu's «جرّب الشكل الجديد» (the classic window only).
        pub(in super::super) fn webui_menu_item(self: &Rc<Self>) {
            // (Not while a page is being created: its message loop delivers
            // menu clicks, and the page would come back over the classic window.)
            if self.webui.switching.get() || self.webui.active.get() {
                return;
            }
            self.webui_enter();
        }

        /// «جرّب الشكل الجديد» in the classic window: the new main window
        /// takes its place now (it is the one chosen anyway, `new_ui`). When
        /// the page can't be created the classic window stays, with a word why.
        pub(in super::super) fn webui_enter(self: &Rc<Self>) {
            if self.webui.active.get() || self.webui.switching.replace(true) {
                return;
            }
            // Tried again by the user: earlier failures are forgiven.
            self.webui.failures.set(0);
            self.webui.one_try.set(false);
            let started = (|| {
                if !available() {
                    return Err("WebView2 is not installed".to_string());
                }
                if !self.webui.parts.is_built() {
                    let built = self.build_lazily("new main", |app| {
                        app.build_webui_window().map(|(h, raw)| (vec![h], vec![raw])).map_err(nwg::NwgError::ControlCreationError)
                    });
                    if !built {
                        return Err("the window could not be created".to_string());
                    }
                }
                self.start_webui()
            })();
            self.webui.switching.set(false);
            if let Err(e) = started {
                log(&format!("webui: {e}; staying with the classic window"));
                self.webui.active.set(false);
                self.webui.host.replace(None);
                self.webui_titles();
                self.set_status(tr!("تعذر فتح الشكل الجديد، فبقي الشكل القديم.", "Could not open the new look; the classic one stays."));
                return;
            }
            log("webui: switched to the new look");
            self.state.borrow_mut().settings.new_ui = true;
            self.save_settings();
            self.webui_note(false);
            let visible = self.window.visible();
            place_like(&self.window, &self.webui.parts.window);
            self.remember_query();
            self.window.set_visible(false);
            if visible {
                self.webui_show();
            }
        }

        /// The new look failed twice: the classic window takes its place
        /// now, shown when `show`, for this run only (`webui_simple_look`).
        /// The page itself goes on the next tick (not inside a handler of
        /// its own).
        fn webui_leave(&self, show: bool) {
            log("webui: using the classic window for this run");
            self.webui_stop_content();
            self.webui.ready.set(false);
            self.webui.active.set(false);
            self.webui.leaving.set(true);
            self.webui_titles();
            self.webui.loading_since.set(None);
            self.webui.failed.set(None);
            place_like(&self.webui.parts.window, &self.window);
            self.webui.parts.window.set_visible(false);
            // The classic window goes on with what the page searched.
            let q = self.webui.page_query.borrow().clone();
            if !q.trim().is_empty() && self.query.text() != q {
                self.query.set_text(&q);
            }
            if show {
                self.show_window();
            }
            self.webui_simple_look();
        }

        /// Creates the page of the window on screen (from the timer, after
        /// the window and «جارٍ الفتح…» were painted). When WebView2 can't
        /// start, the classic window takes over (and the failed start counts).
        fn webui_create_page(&self) {
            let Some(app) = self.webui.me.borrow().upgrade() else { return };
            let visible = self.webui.parts.window.visible();
            self.webui.switching.set(true);
            let started = app.start_webui();
            self.webui.switching.set(false);
            match started {
                Ok(()) => {
                    log(&format!("webui: page created {} ms after launch", super::super::startup::ms_since_process_start()));
                    if visible {
                        self.webui_show();
                    }
                }
                Err(e) => {
                    log(&format!("webui: {e}; using the classic window"));
                    self.webui_fall_back(visible);
                    if visible {
                        self.show_window();
                    }
                }
            }
        }

        /// A page that has worked a while (`crate::webui::STABLE`): its
        /// earlier failures, and a failed start at the last launch, no
        /// longer count.
        fn webui_forgive(&self) {
            if !crate::webui::stable(self.webui.ready_since.get().map(|t| t.elapsed())) {
                return;
            }
            self.webui.failures.set(0);
            self.webui.one_try.set(false);
            let failed_start = std::mem::take(&mut self.state.borrow_mut().settings.new_ui_failed);
            if failed_start {
                self.save_settings();
            }
        }

        /// A WebView2 process failed, or the page stayed blank: the first
        /// time the page is shown again (a new one when the browser itself
        /// is gone, else reloaded); the second time the classic window takes
        /// over for this run, with a word why. From the timer. (After a
        /// fallback at the last start the first failure already counts:
        /// `crate::webui::start_tolerance`.)
        fn webui_failed(&self, browser_gone: bool, why: &str) {
            self.webui.loading_since.set(None);
            self.webui.ready_since.set(None);
            let n = self.webui.failures.get() + 1;
            self.webui.failures.set(n);
            let visible = self.webui.parts.window.visible();
            if n == 1 {
                log(&format!("webui: {why}; showing the page again"));
                let again = if browser_gone {
                    let app = self.webui.me.borrow().upgrade();
                    match app {
                        Some(app) if !self.webui.switching.replace(true) => {
                            let started = app.start_webui();
                            self.webui.switching.set(false);
                            started.map_err(|e| log(&format!("webui: {e}"))).is_ok()
                        }
                        _ => false,
                    }
                } else {
                    self.webui.ready.set(false);
                    let reloaded = self.webui.host.borrow().as_ref().is_some_and(Host::reload);
                    if reloaded {
                        self.webui.loading_since.set(Some(Instant::now()));
                    }
                    reloaded
                };
                if again {
                    if visible {
                        self.webui_show();
                    }
                    return;
                }
            }
            log(&format!("webui: {why} ({n} times); using the classic window for this run"));
            self.webui_leave(visible);
        }

        /// Called at start when the one-time switch to the new look changed
        /// a classic user's setting (`Settings::make_new_look_only`).
        pub(in super::super) fn webui_tell_switch(&self) {
            self.webui.tell_switch.set(true);
        }

        /// The memory guard: the grid's made thumbnails go (made again).
        pub(in super::super) fn webui_shed_thumbs(&self) {
            self.webui.thumbs.shed();
        }

        /// The new main window, while it is the one in use.
        pub(in super::super) fn webui_window(&self) -> Option<&nwg::Window> {
            self.webui.active.get().then_some(&self.webui.parts.window)
        }

        /// A page is being created (its WebView2 runs a message loop): the
        /// first-run questions wait.
        pub(in super::super) fn webui_busy(&self) -> bool {
            self.webui.switching.get()
        }

        /// Whether the new main window is the one in use (on screen or
        /// hidden to the tray).
        pub(in super::super) fn webui_in_use(&self) -> bool {
            self.webui.active.get()
        }

        /// Whether the new main window is the one in use and on screen.
        pub(in super::super) fn webui_visible(&self) -> bool {
            self.webui.active.get() && self.webui.parts.window.visible()
        }

        /// Whether the page has loaded and takes messages.
        pub(in super::super) fn webui_ready(&self) -> bool {
            self.webui.ready.get()
        }

        /// Runs `f` with the window a message box of the program belongs to
        /// (the first-run offers, the crash report): the new main window
        /// when it is the one on screen, so the box comes over it and keeps
        /// it disabled while open (MessageBoxW disables its owner); the
        /// classic window otherwise, as before. The hidden classic window is
        /// disabled meanwhile too: the offers wait for it to be enabled
        /// before asking the next question.
        pub(in super::super) fn with_modal_owner<R>(&self, f: impl FnOnce(HWND) -> R) -> R {
            let classic = self.window.handle.hwnd().unwrap_or(std::ptr::null_mut());
            let owner = if self.webui_visible() { self.webui.parts.window.handle.hwnd() } else { None };
            let Some(owner) = owner else { return f(classic) };
            let was_enabled = !classic.is_null() && unsafe { IsWindowEnabled(classic) } != 0;
            if was_enabled {
                unsafe { EnableWindow(classic, 0) };
            }
            let answer = f(owner);
            if was_enabled {
                unsafe { EnableWindow(classic, 1) };
            }
            answer
        }

        /// Shows the new main window in front, if it is the one in use.
        pub(in super::super) fn webui_show(&self) -> bool {
            if !self.webui.active.get() {
                return false;
            }
            let window = &self.webui.parts.window;
            if let Some(h) = window.handle.hwnd() {
                surface::dwm_style(h, self.webui_dark(), false, false);
                // Inside the work area before it shows (see the raw handler).
                super::super::placement::keep_on_screen(h);
                window.set_visible(true);
                unsafe {
                    if IsIconic(h) != 0 {
                        ShowWindow(h, SW_RESTORE);
                        super::super::placement::keep_on_screen(h);
                    }
                    SetForegroundWindow(h);
                    winapi::um::winuser::UpdateWindow(h);
                }
                if !self.webui.shown_logged.replace(true) {
                    log(&format!("webui: window on screen {} ms after launch", super::super::startup::ms_since_process_start()));
                }
            }
            // (Also from WM_SHOWWINDOW, but a window already on screen gets none.)
            self.webui_back_on_screen();
            let has_page = match &*self.webui.host.borrow() {
                Some(host) => {
                    host.focus();
                    true
                }
                None => false,
            };
            if !has_page {
                // «جارٍ الفتح…» is on screen; the page comes on the next tick.
                self.webui.parts.opening.set_visible(true);
                self.webui.host_pending.set(true);
                return true;
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

        /// The window out of the way (to the tray), its page not visible
        /// and in its low-memory mode. The timer then suspends the page, and
        /// closes it after a long while hidden (`webui_idle`).
        fn webui_put_away(&self) {
            self.webui.parts.window.set_visible(false);
            if self.webui.hidden_since.get().is_none() {
                self.webui.hidden_since.set(Some(Instant::now()));
                self.webui.suspend_asked.set(false);
            }
            if let Ok(host) = self.webui.host.try_borrow()
                && let Some(host) = &*host
            {
                host.set_hidden(true);
            }
        }

        /// The window is on screen again (shown by any means, or brought to
        /// the front): its page is visible, at normal memory and resumed, or,
        /// when there is none yet (a start in the tray) or it was released
        /// while hidden, it is created on the next tick.
        fn webui_back_on_screen(&self) {
            self.webui.hidden_since.set(None);
            self.webui.suspend_asked.set(false);
            // (Not while the page is being created or closed.)
            let Ok(host) = self.webui.host.try_borrow() else { return };
            match &*host {
                Some(host) => host.set_hidden(false),
                None if self.webui.active.get() => {
                    self.webui.host_pending.set(true);
                    if self.webui.released.get() && self.webui.recreate_since.get().is_none() {
                        self.webui.recreate_since.set(Some(Instant::now()));
                    }
                }
                None => {}
            }
        }

        /// Whether a modal dialog or menu is open (it disables its owner:
        /// the new window or the hidden classic one), a dialog runs inside
        /// the page's own handler, or a page is being created.
        fn webui_modal_open(&self) -> bool {
            let disabled = |h: Option<HWND>| h.is_some_and(|h| unsafe { IsWindowEnabled(h) } == 0);
            self.webui.in_page_call.get()
                || self.webui.switching.get()
                || self.webui.options_pending.get()
                || disabled(self.webui.parts.window.handle.hwnd())
                || disabled(self.window.handle.hwnd())
        }

        /// The timer's look at the hidden window's page: suspended once
        /// hidden, closed after `crate::webui::release_after` hidden
        /// (`crate::webui::idle_action`).
        fn webui_idle(&self) {
            // A window shown by some way that told nothing.
            if self.webui.hidden_since.get().is_some() && self.webui.parts.window.visible() {
                self.webui_back_on_screen();
            }
            let Ok(has_page) = self.webui.host.try_borrow().map(|h| h.is_some()) else { return };
            let idle = crate::webui::Idle {
                hidden_for: self.webui.hidden_since.get().map(|t| t.elapsed()),
                has_page,
                suspend_asked: self.webui.suspend_asked.get(),
                hand_off: self.webui.quick_switch.borrow().is_some(),
                modal: self.webui_modal_open(),
                busy: self.webui.content.borrow().is_some(),
            };
            match crate::webui::idle_action(idle, crate::webui::release_after()) {
                crate::webui::IdleAction::Keep => {}
                crate::webui::IdleAction::Suspend => {
                    self.webui.suspend_asked.set(true);
                    let asked = self.webui.host.borrow().as_ref().is_some_and(|host| {
                        host.suspend(|suspended| {
                            log(if suspended { "webui: suspended (hidden)" } else { "webui: not suspended (WebView2 declined)" });
                        })
                    });
                    if !asked {
                        log("webui: this WebView2 can't suspend a hidden page");
                    }
                }
                crate::webui::IdleAction::Release => self.webui_release(),
            }
        }

        /// Closes the hidden window's WebView2 so that its processes exit
        /// (from the timer, never inside a handler of the page). Not a
        /// failure: nothing counts towards the classic window, and the next
        /// show creates the page again, with the search it had.
        fn webui_release(&self) {
            self.webui_stop_content();
            let search = self.webui.page_search.borrow().clone();
            self.webui.restore.replace(search);
            self.webui.released.set(true);
            self.webui.ready.set(false);
            self.webui.loading_since.set(None);
            self.webui.ready_since.set(None);
            self.webui.failed.set(None);
            self.webui.host.replace(None);
            // What only that page used.
            self.webui.lists.borrow_mut().reset();
            self.webui.thumbs.shed();
            self.webui.parts.opening.set_visible(true);
            log(&format!("webui: released after {} hidden", crate::webui::log_duration(crate::webui::release_after())));
        }

        /// Ctrl+Alt+G's move: the new main window, if it is the one in use,
        /// gets out of the way of the Save/Open dialog (no balloon). False
        /// when the new look is not in use.
        pub(in super::super) fn webui_step_aside(&self) -> bool {
            if !self.webui.active.get() {
                return false;
            }
            self.webui_put_away();
            true
        }

        /// The Store rating question in the page (`super::super::rating`):
        /// false when there is no page on screen to show it.
        pub(in super::super) fn webui_ask_rating(&self) -> bool {
            if !self.webui.ready.get() || !self.webui_visible() {
                return false;
            }
            self.webui_send(&Outbound::Rating);
            true
        }

        /// A Save/Open dialog started waiting for a result (`Some`, with the
        /// hint), or stopped waiting: the page says so and Enter, Esc and its
        /// result menu follow (`Outbound::QuickSwitch`).
        pub(in super::super) fn webui_quick_switch(&self, hint: Option<&str>) {
            let hint = hint.map(str::to_string);
            if *self.webui.quick_switch.borrow() == hint {
                return;
            }
            self.webui.quick_switch.replace(hint);
            self.webui_send_quick_switch();
        }

        fn webui_send_quick_switch(&self) {
            let text = self.webui.quick_switch.borrow().clone();
            self.webui_send(&Outbound::QuickSwitch { on: text.is_some(), text: text.unwrap_or_default() });
        }

        /// Hides the window by the clock (Esc, its close button, "Hide next
        /// to the clock"), as the classic window: what was changed up to
        /// here (size, sort, type, settings) is saved first, since the
        /// program may now run for days and end with Windows itself; the
        /// first time, the balloon says how to open it again and how to exit.
        fn webui_hide(&self) {
            // (Measured while still on screen.)
            self.save_settings();
            self.webui_put_away();
            self.hidden_balloon();
        }

        /// Called when the program closes: the page goes before its window.
        pub(in super::super) fn webui_close(&self) {
            self.webui_stop_content();
            self.webui.ready.set(false);
            // (Not when restarting from inside the page's own handler: the
            // process exits right after.)
            if !self.webui.exiting.get() {
                self.webui.host.replace(None);
            }
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

        fn webui_notice(&self, text: impl Into<String>) {
            self.webui_send(&Outbound::Notice { text: text.into(), warn: false });
        }

        /// A problem ("could not…", "wait…"): shown in the warning colour
        /// until the next key or click, and read out by screen readers.
        fn webui_warn(&self, text: impl Into<String>) {
            self.webui_send(&Outbound::Notice { text: text.into(), warn: true });
        }

        /// The quick popup's search continues in the new window, run as
        /// `how` says (inside documents, all types, all drives), never in the
        /// hidden classic window. Shows the window. False when the new look
        /// is not in use.
        pub(in super::super) fn webui_take_search(&self, q: &str, how: super::Handover) -> bool {
            if !self.webui.active.get() {
                return false;
            }
            // (The classic box keeps the text, as before; the plain hand-over
            // below then has nothing more to send.)
            self.query.set_text(q);
            self.webui.handed.replace(q.to_string());
            self.webui.handover.replace(Some((q.to_string(), how)));
            self.webui_show();
            true
        }

        /// The quick popup's search, when it asked for the full window.
        fn webui_hand_over_query(&self) {
            if !self.webui.ready.get() {
                return;
            }
            if let Some((q, how)) = self.webui.handover.take() {
                if how.all_drives {
                    self.webui_set_scope(String::new(), false);
                }
                self.webui_send(&Outbound::Query { q, content: how.content, all_kinds: how.all_kinds });
                return;
            }
            let text = self.query.text();
            if *self.webui.handed.borrow() == text {
                return;
            }
            if self.webui.ready.get() {
                self.webui.handed.replace(text.clone());
                if !text.trim().is_empty() {
                    self.webui_send(&Outbound::Query { q: text, content: false, all_kinds: false });
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
            // (Not again when the page already lists these files.)
            let Some(ver) = self.webui.lists.borrow_mut().set_recent(paths) else { return };
            self.webui_send(&Outbound::Recent { ver, rows });
        }

        fn webui_send_history(&self) {
            let msg = bridge::history(&self.state.borrow().history);
            self.webui_send(&msg);
        }

        /// The theme's colours are dark: Windows' own app mode for
        /// "Automatic", else the theme chosen in Settings.
        fn webui_dark(&self) -> bool {
            self.state.borrow().settings.theme.dark(surface::apps_dark())
        }

        /// The new window's title bar dark or light, as its theme.
        fn webui_title_bar(&self) {
            if let Some(h) = self.webui.parts.window.handle.hwnd() {
                surface::dwm_style(h, self.webui_dark(), false, false);
                // (Windows 10 repaints the caption only when its frame changes.)
                unsafe {
                    use winapi::um::winuser::{SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos};
                    SetWindowPos(h, std::ptr::null_mut(), 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
                }
            }
        }

        /// Something the Settings view shows changed outside the page (a
        /// check for updates started or ended, search from phone started or
        /// stopped): sent now when the page is there, else on the first
        /// tick it is.
        pub(in super::super) fn webui_settings_changed(&self) {
            if self.webui.active.get() && self.webui.ready.get() {
                self.webui_send_settings();
            } else {
                self.webui.settings_stale.set(true);
            }
        }

        fn webui_send_settings(&self) {
            let current = i18n::locale();
            let msg = {
                let s = &self.state.borrow().settings;
                Outbound::Settings(SettingsOut {
                    lang: current.code,
                    next_lang: self.webui.next_lang.get().unwrap_or(current).code,
                    digits: s.arabic_digits,
                    autostart: autostart::is_enabled(),
                    content_index: s.content_index,
                    read_cloud_files: s.read_cloud_files,
                    auto_index: s.auto_index,
                    hide_system: s.hide_system,
                    hotkey_quick: s.hotkey_opens == HotkeyOpens::Quick,
                    floating: s.floating_mode != FloatingMode::Off,
                    tray_icon: s.tray_icon,
                    version: env!("CARGO_PKG_VERSION"),
                    store: crate::store::is_packaged(),
                    checking_updates: self.update_check_asked(),
                    hotkey: s.hotkey.code(),
                    hotkey_taken: super::super::keys::hotkey_taken(),
                    hotkeys: bridge::hotkeys(),
                    theme: s.theme.code(),
                    phone: self.phone_settings(),
                })
            };
            self.webui_send(&msg);
        }

        fn webui_set_scope(&self, scope: String, search: bool) {
            self.webui.scope.replace(scope.clone());
            self.webui_send(&Outbound::Scope { path: scope, search });
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
            // Back on the classic window: the page goes on the next tick, and
            // until then what it asks is not done.
            if !self.webui.active.get() && !matches!(req, Request::Hello {}) {
                return;
            }
            let lang = i18n::current();
            match req {
                Request::Hello {} => {
                    // A new page (the first, or WebView2 loaded it again): it
                    // lists nothing yet, so nothing of the page before is
                    // acted on or sent to it.
                    self.webui_stop_content();
                    self.webui.lists.borrow_mut().reset();
                    self.webui.thumbs.forget_keys();
                    self.webui.ready.set(true);
                    if let Some(t) = self.webui.loading_since.get() {
                        log(&format!(
                            "webui: page said hello {} ms after launch ({} ms after it was created)",
                            super::super::startup::ms_since_process_start(),
                            t.elapsed().as_millis()
                        ));
                    }
                    self.webui.parts.opening.set_visible(false);
                    self.webui.loading_since.set(None);
                    self.webui.ready_since.set(Some(Instant::now()));
                    if let Some(t) = self.webui.recreate_since.take() {
                        log(&format!("webui: recreated on show in {} ms", t.elapsed().as_millis()));
                    }
                    // The page works: this start did not crash on the way.
                    let trying = std::mem::take(&mut self.state.borrow_mut().settings.new_ui_trying);
                    if trying != 0 {
                        self.save_settings();
                    }
                    self.webui.last_status.replace(String::new());
                    let hello = {
                        let s = &self.state.borrow().settings;
                        Outbound::Hello {
                            kinds: bridge::kinds(lang),
                            sizes: bridge::labels(bridge::SIZES, lang),
                            dates: bridge::labels(bridge::DATES, lang),
                            kind: search::kind_or_all(s.kind),
                            sort: Sort::from_column(s.sort_col),
                            desc: s.sort_desc,
                            digits: s.arabic_digits,
                            version: env!("CARGO_PKG_VERSION"),
                            // Once: a page loaded again does not say it again.
                            switched: self.webui.tell_switch.replace(false),
                            store: crate::store::is_packaged(),
                        }
                    };
                    self.webui_send(&hello);
                    let scope = self.webui.scope.borrow().clone();
                    self.webui_send(&Outbound::Scope { path: scope, search: false });
                    self.webui_send_settings();
                    self.webui_status(true);
                    self.webui_send_recent();
                    self.webui_send_history();
                    if self.webui.quick_switch.borrow().is_some() {
                        self.webui_send_quick_switch();
                    }
                    // A page loaded again gets back what it searched (not an
                    // older text of the classic box); a first page gets the
                    // classic box's text (the quick popup's search).
                    // A page created again after a release gets its search
                    // back with its filters (or nothing, if it had none).
                    let again = self.webui.page_query.borrow().clone();
                    let restore = self.webui.restore.take();
                    if self.webui.handover.borrow().is_some() {
                        self.webui_hand_over_query();
                    } else if let Some(restore) = restore {
                        self.webui.handed.replace(self.query.text());
                        if !restore.q.trim().is_empty() {
                            self.webui_send(&Outbound::Restore(restore));
                        }
                    } else if again.trim().is_empty() {
                        self.webui.handed.replace(String::new());
                        self.webui_hand_over_query();
                    } else {
                        self.webui.handed.replace(self.query.text());
                        self.webui_send(&Outbound::Query { q: again, content: false, all_kinds: false });
                    }
                }
                Request::Search { seq, q, kind, size, date, content, sort, desc } => {
                    // (The rating question waits while the user types.)
                    self.note_typing();
                    self.webui.page_query.replace(q.clone());
                    self.webui.page_search.replace(Some(bridge::Restore { q: q.clone(), kind, size, date, content }));
                    {
                        let s = &mut self.state.borrow_mut().settings;
                        s.kind = kind;
                        s.sort_col = sort.column();
                        s.sort_desc = desc;
                    }
                    // The apps the text names, as the classic window's buttons.
                    let apps = if content { Vec::new() } else { self.app_matches_for(&q) };
                    let titles = apps.iter().map(|a| a.title.clone()).collect();
                    self.webui.apps.replace((seq, q.clone(), apps));
                    self.webui_send(&Outbound::Apps { seq, apps: titles });
                    let params = self.webui_params(Params { seq, text: q, kind, size, date, content, sort, desc, ..Params::default() });
                    if content {
                        self.webui_content(params);
                    } else {
                        self.webui_stop_content();
                        self.webui_search(params);
                    }
                }
                Request::More { seq, ver, from } => {
                    // (Another version's rows would not fit the list the page has; it gets the new list anyway.)
                    let Ok(msg) = self.webui.lists.borrow().shown_at(seq, ver).map(|s| s.rows(from, bridge::MORE_PAGE)) else { return };
                    self.webui_send(&msg);
                }
                Request::Sort { seq, sort, desc } => {
                    {
                        let s = &mut self.state.borrow_mut().settings;
                        s.sort_col = sort.column();
                        s.sort_desc = desc;
                    }
                    // A new order is a new version of the list.
                    let msg = match self.webui.lists.borrow_mut().resort(seq, sort, desc) {
                        Some(s) => {
                            let suggestions = if s.hits.is_empty() && s.done { bridge::content_suggestions(&s.params) } else { Vec::new() };
                            s.results(0, None, suggestions, lang)
                        }
                        None => return,
                    };
                    self.webui_send(&msg);
                }
                Request::Stop { seq } => {
                    if let Some(run) = &*self.webui.content.borrow()
                        && run.seq == seq
                    {
                        run.progress.cancel.store(true, Ordering::Relaxed);
                    }
                }
                Request::Matches { seq, ver, id } => {
                    // Lines not kept with the row are read again on another
                    // thread (a big document can take long); the answer
                    // comes from `webui_matches_read`.
                    let to_read = self.webui.lists.borrow().shown_at(seq, ver).ok().and_then(|s| s.matches_to_read(id));
                    if let Some((path, q)) = to_read {
                        let use_index = self.state.borrow().settings.content_index;
                        let slot = self.matches_read.clone();
                        let notice = self.matches_notice.sender();
                        crate::crash_log::spawn_named("matches-read", move || {
                            let cache = if use_index { content_index::shared() } else { None };
                            let found = crate::crash_log::catch(|| crate::search::document_matches(&path, &q, cache)).flatten();
                            if let Ok(mut s) = slot.lock() {
                                s.push(super::super::MatchesRead::Web { seq, ver, id, found });
                            }
                            notice.notice();
                        });
                        return;
                    }
                    let msg = self.webui.lists.borrow().shown_at(seq, ver).ok().and_then(|s| s.matches(id));
                    if let Some(msg) = msg {
                        self.webui_send(&msg);
                    }
                }
                Request::Open { list, seq, ver, id } => {
                    if let Some(path) = self.webui_path(list, seq, ver, id) {
                        self.webui_note_use(list, &path);
                        if !shell::open(&path) {
                            self.webui_warn(trf!("تعذر فتح الملف: {}", "Could not open the file: {}", path));
                        } else if !self.webui_is_dir(list, seq, ver, id, &path) {
                            // A file opened: counts toward the Store rating
                            // question, as in the classic window (folders don't).
                            self.count_file_open();
                        }
                    }
                }
                Request::Reveal { list, seq, ver, id } => {
                    if let Some(path) = self.webui_path(list, seq, ver, id) {
                        self.webui_note_use(list, &path);
                        shell::reveal(&path);
                    }
                }
                Request::Copy { list, seq, ver, id } => {
                    if let Some(path) = self.webui_path(list, seq, ver, id) {
                        // Another program may hold the clipboard: then it says so.
                        if shell::copy_text(&self.webui.parts.window, &path) {
                            self.webui_notice(tr!("تم نسخ المسار.", "Path copied."));
                        } else {
                            self.webui_warn(shell::copy_failed_text());
                        }
                    }
                }
                Request::CopyFile { list, seq, ver, id } => {
                    if let Some(path) = self.webui_path(list, seq, ver, id) {
                        let ok = self.webui.parts.window.handle.hwnd().is_some_and(|h| crate::dragdrop::copy_files(h, std::slice::from_ref(&path)));
                        self.webui_notice(if ok { tr!("تم نسخ الملف. الصقه حيث تريد بـ Ctrl+V.", "File copied. Paste it anywhere with Ctrl+V.") } else { tr!("تعذر نسخ الملف.", "Could not copy the file.") });
                    }
                }
                Request::CopyAll { seq, ver } => {
                    let text = {
                        let lists = self.webui.lists.borrow();
                        let paths = match lists.shown_at(seq, ver) {
                            Ok(s) => s.all_paths(),
                            Err(miss) => {
                                drop(lists);
                                self.webui_missed(miss);
                                return;
                            }
                        };
                        (!paths.is_empty()).then(|| (paths.len(), paths.join("\r\n")))
                    };
                    match text {
                        // Not borrowed while the clipboard notifies its previous owner.
                        Some((n, text)) => {
                            if shell::copy_text(&self.webui.parts.window, &text) {
                                self.webui_notice(trf!("تم نسخ المسارات. عددها: {}.", "Copied {} paths.", crate::indexing_status::group_digits(n)));
                            } else {
                                self.webui_warn(shell::copy_failed_text());
                            }
                        }
                        None => self.webui_notice(tr!("لا توجد نتائج لنسخها.", "There are no results to copy.")),
                    }
                }
                Request::Rename { list, seq, ver, id, name } => self.webui_rename(list, seq, ver, id, &name),
                Request::Recycle { list, seq, ver, id } => self.webui_recycle(list, seq, ver, id),
                Request::Scope { list, seq, ver, id } => {
                    let Some(path) = self.webui_path(list, seq, ver, id) else { return };
                    let is_dir = self.webui_is_dir(list, seq, ver, id, &path);
                    let scope = bridge::scope_for(&path, is_dir);
                    if !scope.is_empty() {
                        self.webui_set_scope(scope, true);
                    }
                }
                Request::PickFolder {} => self.webui_pick_folder(),
                Request::AllDrives {} => self.webui_set_scope(String::new(), true),
                Request::Saved { id, name } => {
                    // By its place, checked against its name: the page's list
                    // may be older than the saved searches now.
                    let Some(s) = self.state.borrow().history.saved.get(id as usize).filter(|s| s.name == name).cloned() else {
                        self.webui_missed(bridge::Miss::Stale);
                        self.webui_send_history();
                        return;
                    };
                    self.webui_set_scope(s.scope, false);
                    self.webui_send(&Outbound::Apply { q: s.query, kind: search::kind_or_all(s.kind), content: s.content });
                }
                Request::SaveSearch { q, kind, content } => {
                    let text = q.trim().to_string();
                    {
                        let scope = self.webui.scope.borrow().clone();
                        let mut st = self.state.borrow_mut();
                        st.history.add_saved(SavedSearch { name: text.clone(), query: text.clone(), kind, scope, content });
                        st.history.save();
                    }
                    self.webui_send_history();
                    self.webui_notice(trf!("حُفظ البحث «{text}». تجده في قائمة «السجل».", "Saved the search “{text}”. Find it in the “History” menu."));
                }
                Request::ClearRecent {} => self.webui_clear_history(true),
                Request::ClearSaved {} => self.webui_clear_history(false),
                Request::Set { key, on } => self.webui_set(key, on),
                Request::Lang { lang: chosen } => {
                    let chosen = chosen.0;
                    if i18n::save_locale(chosen) {
                        self.check_lang_item(chosen);
                        self.webui.next_lang.set(Some(chosen));
                    } else {
                        self.webui_warn(tr!("تعذر حفظ اللغة.", "Could not save the language."));
                    }
                    self.webui_send_settings();
                }
                Request::Restart {} => match crate::update::restart_new_version() {
                    Ok(child) => {
                        // Leaves when the new copy runs; keeps running otherwise.
                        self.webui.exiting.set(true);
                        self.hand_over(child, "language");
                        self.webui.exiting.set(false);
                        self.webui_notice(tr!("تعذرت إعادة التشغيل: لم تعمل النسخة الجديدة.", "Could not restart: the new copy did not start."));
                    }
                    Err(e) => self.webui_warn(trf!(
                        "تعذرت إعادة التشغيل تلقائياً ({e}). أغلق البرنامج وافتحه من جديد.",
                        "The program could not restart itself ({e}). Close the program and open it again."
                    )),
                },
                Request::Reindex {} => {
                    if self.webui.content.borrow().is_some() {
                        self.webui_warn(tr!("انتظر انتهاء البحث داخل المستندات أو أوقفه أولاً.", "Wait for the document search to finish, or stop it first."));
                        return;
                    }
                    self.start_scan();
                    let st = self.state.borrow();
                    let text = if st.loading.is_some() {
                        tr!("انتظر حتى يكتمل تحميل الفهرس، ثم حدّثه.", "Wait until the index has loaded, then update it.")
                    } else {
                        tr!("جارٍ فهرسة الملفات على كل الأقراص…", "Indexing files on all drives…")
                    };
                    drop(st);
                    self.webui_notice(text);
                    self.webui_status(true);
                }
                Request::Web { q } => {
                    if !shell::open(&crate::suggest::web_url(&q)) {
                        self.webui_warn(tr!("تعذر فتح المتصفح.", "Could not open the browser."));
                    }
                }
                Request::Hide {} => self.webui_hide(),
                Request::OptionsMenu {} => self.webui.options_pending.set(true),
                // The grid view: a one-time key per row still listed (rows
                // of another version get none; the page never names a path).
                Request::Thumbs { seq, ver, ids } => {
                    let keys: Vec<(u32, String)> = {
                        let lists = self.webui.lists.borrow();
                        let Ok(shown) = lists.shown_at(seq, ver) else { return };
                        ids.iter().filter_map(|&id| shown.thumb_entry(id).map(|(path, size, mtime, _)| (id, self.webui.thumbs.register(path, size, mtime)))).collect()
                    };
                    self.webui_send(&Outbound::Thumbs { seq, ver, keys });
                }
                Request::ReportProblem {} => {
                    if let Some(app) = self.webui.me.borrow().upgrade() {
                        app.show_report();
                    }
                }
                Request::PhoneAllow {} => self.web_allow_now(),
                Request::PhoneWindow {} => {
                    if let Some(app) = self.webui.me.borrow().upgrade() {
                        app.open_web_window();
                    }
                }
                // The page shows the theme at once; the title bar follows it here.
                Request::Theme { key } => {
                    self.state.borrow_mut().settings.theme = key;
                    self.save_settings();
                    self.webui_title_bar();
                    self.webui_send_settings();
                }
                Request::Hotkey { key } => {
                    if !self.change_hotkey(key) {
                        self.webui_warn(trf!(
                            "الاختصار {} يستخدمه برنامج آخر أيضاً. اختر غيره.",
                            "Another program has {} too. Pick another one.",
                            key.label()
                        ));
                    }
                    self.webui_send_settings();
                }
                // The Save dialog runs inside this handler: the timer leaves the page alone meanwhile.
                Request::Export {} => {
                    self.webui.in_page_call.set(true);
                    self.export_csv();
                    self.webui.in_page_call.set(false);
                }
                Request::ReplaceText {} => {
                    if let Some(app) = self.webui.me.borrow().upgrade()
                        && app.ensure_replace()
                    {
                        app.open_replace();
                    }
                }
                Request::FolderSizes { list, seq, ver, id } => {
                    let Some(path) = self.webui_path(list, seq, ver, id) else { return };
                    let folder = if std::path::Path::new(&path).is_dir() { path } else { bridge::parent(&path).to_string() };
                    if let Some(app) = self.webui.me.borrow().upgrade() {
                        app.show_disk_usage(folder);
                    }
                }
                Request::App { seq, id } => {
                    let found = {
                        let apps = self.webui.apps.borrow();
                        (apps.0 == seq).then(|| apps.2.get(id as usize).cloned().map(|item| (item, apps.1.clone()))).flatten()
                    };
                    if let Some((item, text)) = found
                        && !self.launch_app_item(&item, &text)
                    {
                        self.webui_warn(trf!("تعذر فتح: {}", "Could not open: {}", item.title));
                    }
                    self.webui_send_history();
                }
                Request::Rating { rate } => self.rating_answer(rate),
                // On the next tick, not inside WebView2's handler (it may ask
                // with a message box).
                Request::CheckUpdates {} => self.webui.updates_pending.set(true),
                Request::OpenWith { list, seq, ver, id } => self.webui_verb(list, seq, ver, id, "openas"),
                Request::Properties { list, seq, ver, id } => self.webui_verb(list, seq, ver, id, "properties"),
                Request::SwitchTo { list, seq, ver, id } => {
                    let Some(path) = self.webui_path(list, seq, ver, id) else { return };
                    let is_dir = self.webui_is_dir(list, seq, ver, id, &path);
                    if self.quick_switch_waiting() {
                        self.webui_note_use(list, &path);
                    }
                    self.quick_switch_to(&path, is_dir);
                }
                Request::SwitchCancel {} => {
                    // (Already forgotten: the page is told it is over.)
                    if !self.quick_switch_cancel() {
                        self.webui.quick_switch.replace(None);
                        self.webui_send_quick_switch();
                    }
                }
                Request::Drag { list, seq, ver, id } => {
                    // Started right after this handler returns (the drag
                    // runs its own message loop, not inside WebView2's).
                    if let Some(path) = self.webui_path(list, seq, ver, id) {
                        self.webui.drag.replace(Some(path));
                        self.webui.parts.drag_notice.sender().notice();
                    }
                }
            }
        }

        /// The full path of row `id` of version `ver` of `list`, from what
        /// the page was sent; `None`, and a word to the user when the list
        /// changed since, when it is not there.
        fn webui_path(&self, list: List, seq: u64, ver: u64, id: u32) -> Option<String> {
            let found = self.webui.lists.borrow().path(list, seq, ver, id).map(str::to_string);
            found.map_err(|miss| self.webui_missed(miss)).ok()
        }

        /// Windows' "Open with" ("openas") or Properties ("properties") for
        /// row `id`, over the new window, as the classic result menu does.
        fn webui_verb(&self, list: List, seq: u64, ver: u64, id: u32, verb: &str) {
            let Some(path) = self.webui_path(list, seq, ver, id) else { return };
            // (Whatever Windows shows may run a message loop: the timer
            // leaves the page alone meanwhile.)
            self.webui.in_page_call.set(true);
            let ok = shell::invoke_verb(self.webui.parts.window.handle.hwnd(), &path, verb);
            self.webui.in_page_call.set(false);
            if !ok {
                self.webui_warn(trf!("تعذّر تنفيذ ذلك على: {}", "Windows could not do that for: {}", super::super::format::ltr_path(&path)));
            }
        }

        /// Whether row `id` (whose file is `path`) is a folder.
        fn webui_is_dir(&self, list: List, seq: u64, ver: u64, id: u32, path: &str) -> bool {
            match list {
                List::Results => self.webui.lists.borrow().shown_at(seq, ver).is_ok_and(|s| s.is_dir(id)),
                List::Recent => std::path::Path::new(path).is_dir(),
            }
        }

        fn webui_missed(&self, miss: Miss) {
            if let Some(text) = miss.message(i18n::current()) {
                self.webui_warn(text);
            }
        }

        /// A row was used: the file ranks higher next time and, for a
        /// result, its search goes into the history (as in the classic
        /// window). The recent files were listed by no search.
        fn webui_note_use(&self, list: List, path: &str) {
            let text = self.webui.lists.borrow().used_search(list).map(str::to_string);
            let mut st = self.state.borrow_mut();
            if let Some(text) = text {
                st.history.add_recent(&text);
            }
            st.history.record_open(path, crate::index::now_secs());
            st.history.save();
            drop(st);
            self.webui_send_history();
        }

        /// "Clear history and recent files" (`recent`), as the classic
        /// History menu: the recent searches, the opened files (the page's
        /// «ملفات فتحتها مؤخراً» and the quick search's), the quick-search
        /// launches and the web searches they hold. Or deletes the saved
        /// searches.
        fn webui_clear_history(&self, recent: bool) {
            {
                let mut st = self.state.borrow_mut();
                if recent {
                    st.history.clear_activity();
                } else {
                    st.history.saved.clear();
                }
                st.history.save();
            }
            self.webui_send_history();
            if recent {
                self.webui_send_recent();
                self.webui_notice(tr!("مُسح السجل والملفات الأخيرة.", "History and recent files cleared."));
            }
        }

        // ------------------------------------------------------------ files

        /// Why the index can't change now, if it can't: a search inside
        /// documents points into it, or it is being built.
        fn webui_index_busy(&self) -> Option<&'static str> {
            if self.webui.content.borrow().is_some() {
                return Some(tr!("انتظر انتهاء البحث داخل المستندات أو أوقفه أولاً.", "Wait for the document search to finish, or stop it first."));
            }
            let st = self.state.borrow();
            (st.scan.is_some() || st.loading.is_some()).then(|| tr!("انتظر حتى تكتمل الفهرسة.", "Wait until indexing is complete."))
        }

        /// Renames row `id`'s file to `name`, with the classic window's checks.
        fn webui_rename(&self, list: List, seq: u64, ver: u64, id: u32, name: &str) {
            let Some(old) = self.webui_path(list, seq, ver, id) else { return };
            if let Some(why) = self.webui_index_busy() {
                self.webui_warn(why);
                return;
            }
            let new = match bridge::rename_target(&old, name) {
                Ok(new) => new,
                Err(bridge::BadName::Unchanged) => return,
                Err(e) => {
                    self.webui_warn(e.message(i18n::current()));
                    return;
                }
            };
            let new_name = name.trim().to_string();
            // Another file by that name: said before trying, and the rename
            // itself never replaces one (one that appears meanwhile makes it
            // fail). Changing only the letter case of the same file is fine.
            let taken = || trf!("يوجد ملف آخر بالاسم «{new_name}» في هذا المجلد.", "Another file named “{new_name}” already exists in this folder.");
            if std::path::Path::new(&new).exists() && new.to_lowercase() != old.to_lowercase() {
                self.webui_warn(taken());
                return;
            }
            if let Err(e) = shell::rename_no_replace(&old, &new) {
                self.webui_warn(if shell::already_exists(&e) { taken() } else { trf!("تعذر تغيير الاسم: {e}", "Could not rename: {e}") });
                return;
            }
            self.update_index(|idx| {
                idx.rename_path(&old, &new);
            });
            {
                let mut st = self.state.borrow_mut();
                st.history.rename_path(&old, &new);
                st.history.save();
            }
            if list == List::Results
                && let Ok(s) = self.webui.lists.borrow_mut().shown_at_mut(seq, ver)
            {
                s.changed.insert(id, Some(new));
            }
            self.webui_send(&Outbound::Renamed { list, seq, ver, id, name: new_name.clone() });
            if list == List::Recent {
                self.webui_send_recent();
            }
            self.webui_notice(trf!("تم تغيير الاسم إلى «{new_name}».", "Renamed to “{new_name}”."));
        }

        /// Moves row `id`'s file to the Recycle Bin (the page asked first,
        /// with the file's name).
        fn webui_recycle(&self, list: List, seq: u64, ver: u64, id: u32) {
            let Some(path) = self.webui_path(list, seq, ver, id) else { return };
            if let Some(why) = self.webui_index_busy() {
                self.webui_warn(why);
                return;
            }
            if !shell::recycle(&path) {
                self.webui_warn(tr!("لم يُحذف الملف.", "The file was not deleted."));
                return;
            }
            self.update_index(|idx| {
                idx.remove_path(&path);
            });
            {
                let mut st = self.state.borrow_mut();
                st.history.forget_path(&path);
                st.history.save();
            }
            if list == List::Results
                && let Ok(s) = self.webui.lists.borrow_mut().shown_at_mut(seq, ver)
            {
                s.changed.insert(id, None);
            }
            self.webui_send(&Outbound::Removed { list, seq, ver, id });
            self.webui_send_recent();
            self.webui_notice(tr!("نُقل الملف إلى سلة المحذوفات.", "The file was moved to the Recycle Bin."));
        }

        /// The program's Options menu over the new window, at the pointer
        /// (the page's "Open the Options menu…", the tray's "Options…"):
        /// owned by the window on screen, so the keyboard comes back to the
        /// page when it closes; the chosen item is passed to the classic
        /// window's handlers (`webui_forward_menu`), which act on the new
        /// look while it is in use. False when the new look is not in use.
        pub(in super::super) fn webui_options_menu(&self) -> bool {
            use winapi::shared::windef::POINT;
            use winapi::um::winuser::{GetCursorPos, TrackPopupMenu};
            if !self.webui.active.get() {
                return false;
            }
            let (Some(owner), Some((_, menu))) = (self.webui.parts.window.handle.hwnd(), self.options_menu.handle.pop_hmenu()) else { return false };
            let mut pt = POINT { x: 0, y: 0 };
            unsafe {
                GetCursorPos(&mut pt);
                SetForegroundWindow(owner);
                TrackPopupMenu(menu, 0, pt.x, pt.y, 0, owner, std::ptr::null());
            }
            self.webui.settings_stale.set(true);
            if let Ok(host) = self.webui.host.try_borrow()
                && let Some(host) = &*host
            {
                host.focus();
            }
            true
        }

        /// Passes an item chosen in the Options menu over the new window to
        /// the classic window, as if chosen there (nwg's WM_MENUCOMMAND).
        fn webui_forward_menu(&self, item: nwg::ControlHandle) {
            use winapi::um::winuser::{GetMenuItemCount, GetMenuItemID, PostMessageW, WM_MENUCOMMAND};
            let (Some((menu, id)), Some(classic)) = (item.hmenu_item(), self.window.handle.hwnd()) else { return };
            let n = unsafe { GetMenuItemCount(menu) };
            if let Some(pos) = (0..n.max(0)).find(|&i| unsafe { GetMenuItemID(menu, i) } == id) {
                unsafe { PostMessageW(classic, WM_MENUCOMMAND, pos as WPARAM, menu as LPARAM) };
            }
        }

        /// The new look's results, while it is in use, for the Options
        /// items that work on "the results" (export, duplicates).
        pub(in super::super) fn webui_results(&self) -> Option<Vec<bridge::ExportRow>> {
            self.webui.active.get().then(|| self.webui.lists.borrow().shown.as_ref().map(|s| s.export_rows()).unwrap_or_default())
        }

        /// The folder the new look searches ("" for all drives), while it is in use.
        pub(in super::super) fn webui_scope(&self) -> Option<String> {
            self.webui.active.get().then(|| self.webui.scope.borrow().clone())
        }

        /// "Hide next to the clock" while the new look is in use.
        pub(in super::super) fn webui_hide_main(&self) -> bool {
            if !self.webui.active.get() {
                return false;
            }
            self.webui_hide();
            true
        }

        /// A status-bar message of the classic window (from an Options item,
        /// an update…) shown in the new look while it is on screen.
        pub(in super::super) fn webui_status_text(&self, text: &str) {
            if self.webui_visible() && !text.trim().is_empty() {
                if bridge::looks_like_a_problem(text) {
                    self.webui_warn(text);
                } else {
                    self.webui_notice(text);
                }
            }
        }

        /// Drags the file the page asked for (`Request::Drag`), as the
        /// classic list does; nothing happens when the mouse button was
        /// already released.
        fn webui_drag(&self) {
            use winapi::shared::windef::POINT;
            use winapi::um::winuser::{GA_ROOT, GetAncestor, GetAsyncKeyState, GetCursorPos, VK_LBUTTON, WindowFromPoint};
            let Some(path) = self.webui.drag.take() else { return };
            let Some(h) = self.webui.parts.window.handle.hwnd() else { return };
            // Only while the left button is still down and the pointer still
            // over this window: a drag started after the button came up
            // would drop the file at once wherever the pointer is.
            let down = unsafe { GetAsyncKeyState(VK_LBUTTON) } as u16 & 0x8000 != 0;
            let mut pt = POINT { x: 0, y: 0 };
            let over = unsafe { GetCursorPos(&mut pt) != 0 && GetAncestor(WindowFromPoint(pt), GA_ROOT) == h };
            if !down || !over {
                log(&format!("webui: drag skipped (button down: {down}, over the window: {over})"));
                return;
            }
            log("webui: drag started");
            crate::dragdrop::drag_files_from(h, std::slice::from_ref(&path));
        }

        /// Windows' folder dialog; the chosen folder becomes the one searched.
        fn webui_pick_folder(&self) {
            let mut dialog = nwg::FileDialog::default();
            let built = nwg::FileDialog::builder()
                .title(tr!("اختر المجلد الذي تريد البحث فيه", "Choose the folder to search in"))
                .action(nwg::FileDialogAction::OpenDirectory)
                .build(&mut dialog);
            if built.is_err() {
                return;
            }
            // Nothing is borrowed while it runs: its loop delivers the page's
            // messages, and the timer's (which then leaves the page alone).
            self.webui.in_page_call.set(true);
            let ran = dialog.run(Some(&self.webui.parts.window));
            self.webui.in_page_call.set(false);
            let chosen = if ran { dialog.get_selected_item().ok() } else { None };
            if let Some(dir) = chosen {
                let mut dir = dir.to_string_lossy().into_owned();
                if !dir.ends_with('\\') {
                    dir.push('\\');
                }
                self.webui_set_scope(dir, true);
            }
        }

        // ------------------------------------------------------------ settings

        fn webui_set(&self, key: Setting, on: bool) {
            match key {
                Setting::Digits => {
                    if self.menu_digits.checked() != on {
                        self.toggle_digits();
                    }
                }
                Setting::Autostart => {
                    if autostart::set_enabled(on) {
                        self.menu_autostart.set_checked(on);
                    } else {
                        // The Store version says where the user can change it.
                        self.webui_warn(autostart::change_failed_message());
                    }
                }
                Setting::ContentIndex => {
                    if self.menu_content_index.checked() != on {
                        self.toggle_content_index();
                    }
                }
                Setting::ReadCloudFiles => {
                    if self.menu_read_cloud.checked() != on {
                        self.toggle_read_cloud();
                    }
                }
                Setting::AutoIndex => {
                    if self.menu_auto_index.checked() != on {
                        self.toggle_auto_index();
                    }
                }
                Setting::HideSystem => {
                    if self.menu_hide_system.checked() != on {
                        self.toggle_hide_system();
                    }
                }
                Setting::HotkeyQuick => self.set_hotkey_quick(on),
                Setting::Floating => self.set_floating_mode(if on { FloatingMode::Fade } else { FloatingMode::Off }),
                Setting::TrayIcon => self.set_tray_icon(on),
                Setting::Phone => self.set_web_on(on),
            }
            self.save_settings();
            self.webui_send_settings();
        }

        // ------------------------------------------------------------ search

        /// `p` with what the program knows: the folder, system files, the
        /// often-opened files and whether to look for a better spelling; and
        /// the version its list will have.
        fn webui_params(&self, p: Params) -> Params {
            let st = self.state.borrow();
            let scope = self.webui.scope.borrow().clone();
            Params {
                ver: self.webui.lists.borrow_mut().next_ver(),
                show_system: crate::system_files::show_system(st.settings.hide_system, &scope),
                boosted: if p.sort == Sort::Best { st.history.boosted(&st.index, crate::index::now_secs()) } else { Vec::new() },
                suggest: st.scan.is_none() && st.loading.is_none(),
                scope,
                ..p
            }
        }

        /// Runs the search on a background thread. While one runs, newer
        /// requests fold into a single follow-up search.
        fn webui_search(&self, params: Params) {
            if self.webui.running.get() {
                self.webui.lists.borrow_mut().pending = Some(params);
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
            let pending = match found {
                // (Not kept when a search inside documents started since, or the page loaded again.)
                Some((shown, msg)) => {
                    let (send, pending) = self.webui.lists.borrow_mut().finished(shown);
                    if send {
                        self.webui_send(&msg);
                    }
                    pending
                }
                None => self.webui.lists.borrow_mut().pending.take(),
            };
            if let Some(p) = pending {
                self.webui_search(p);
            }
        }

        /// Starts a search inside documents; its rows arrive as it runs
        /// (`webui_poll_content`).
        fn webui_content(&self, params: Params) {
            self.webui_stop_content();
            let lang = i18n::current();
            let index = self.state.borrow().index.clone();
            let (Some(q), Some(marked)) = (bridge::content_query(&params), bridge::content_query(&params)) else {
                // Only filters, or nothing typed: nothing to look for in a text.
                let mut shown = Shown::documents(index, Default::default(), params);
                shown.done = true;
                let msg = shown.results(0, None, Vec::new(), lang);
                self.webui.lists.borrow_mut().shown = Some(shown);
                self.webui_send(&msg);
                self.webui_notice(tr!("اكتب الكلمة أو العبارة التي تريد البحث عنها داخل المستندات.", "Type the word or phrase to search for inside documents."));
                return;
            };
            let use_index = {
                let mut st = self.state.borrow_mut();
                st.history.add_recent(&params.text);
                st.history.save();
                st.settings.content_index
            };
            self.webui_send_history();
            let current = self.index_current();
            let progress = Arc::new(ContentProgress::default());
            let hits = Arc::new(Mutex::new(Vec::new()));
            let (p, h, idx, scope) = (progress.clone(), hits.clone(), index.clone(), params.scope.clone());
            let started = crate::crash_log::spawn_named("webui-content", move || {
                let cache = if use_index { content_index::shared() } else { None };
                let started = std::time::Instant::now();
                // After a panic (logged by the hook) the search still ends.
                if crate::crash_log::catch(|| search::search_content(&idx.entries, &q, &scope, &p, &h, cache, current)).is_none() {
                    p.done.store(true, Ordering::Relaxed);
                }
                log(&format!(
                    "webui content search: {} documents ({} from the content index{}) in {} ms",
                    p.checked.load(Ordering::Relaxed),
                    p.from_cache.load(Ordering::Relaxed),
                    search::word_index_note(&p),
                    started.elapsed().as_millis()
                ));
            });
            if started.is_none() {
                progress.done.store(true, Ordering::Relaxed);
            }
            let seq = params.seq;
            let shown = Shown::documents(index, marked, params);
            let msg = shown.results(0, None, Vec::new(), lang);
            self.webui.lists.borrow_mut().shown = Some(shown);
            self.webui.content.replace(Some(ContentRun { seq, progress, hits, said: String::new() }));
            self.webui_send(&msg);
        }

        fn webui_stop_content(&self) {
            if let Some(run) = self.webui.content.take() {
                run.progress.cancel.store(true, Ordering::Relaxed);
            }
        }

        /// From the timer, five times a second: the rows a search inside
        /// documents found since, and how far it got.
        fn webui_poll_content(&self) {
            let Some((seq, progress, new)) = self.webui.content.borrow().as_ref().map(|run| {
                let new: Vec<ContentHit> = run.hits.lock().map(|mut h| h.drain(..).collect()).unwrap_or_default();
                (run.seq, run.progress.clone(), new)
            }) else {
                return;
            };
            let done = progress.done.load(Ordering::Relaxed);
            let lang = i18n::current();
            let (found, results) = {
                let mut lists = self.webui.lists.borrow_mut();
                let Some(s) = lists.shown.as_mut().filter(|s| s.seq == seq) else {
                    drop(lists);
                    self.webui_stop_content();
                    return;
                };
                s.absorb(new);
                let (sort, desc) = (s.params.sort, s.params.desc);
                // Put in order at the end: a new version of the list.
                let results = if done {
                    lists.resort(seq, sort, desc).map(|s| {
                        s.done = true;
                        let suggestions = if s.hits.is_empty() { bridge::content_suggestions(&s.params) } else { Vec::new() };
                        s.results(0, None, suggestions, lang)
                    })
                } else {
                    None
                };
                (lists.shown.as_ref().map_or(0, |s| s.hits.len()), results)
            };
            let state = ContentState {
                checked: progress.checked.load(Ordering::Relaxed),
                total: progress.total.load(Ordering::Relaxed),
                found,
                cached: progress.from_cache.load(Ordering::Relaxed),
                unindexed: progress.unindexed.load(Ordering::Relaxed),
                done,
                indexing: self.indexing(),
                ocr_arabic_missing: done && crate::ocr::arabic_missing(),
            };
            let text = bridge::content_text(&state, lang);
            if let Some(results) = results {
                self.webui.content.replace(None);
                self.webui_send(&results);
            } else {
                let mut content = self.webui.content.borrow_mut();
                let Some(run) = content.as_mut() else { return };
                if run.said == text {
                    return;
                }
                run.said = text.clone();
            }
            self.webui_send(&Outbound::Progress { seq, total: found, done, text });
        }

        /// From the timer, five times a second: a search inside documents;
        /// about once a second: the indexing status, and the shown name
        /// results kept in step with the index (finished scan, files changed
        /// on disk).
        /// Matches of a row read again in the background: sent to the page,
        /// unless its list changed meanwhile.
        pub(in super::super) fn webui_matches_read(&self, seq: u64, ver: u64, id: u32, found: Option<(u32, Vec<(u32, String)>)>) {
            let msg = self.webui.lists.borrow().shown_at(seq, ver).ok().and_then(|s| s.matches_read(id, found));
            if let Some(msg) = msg {
                self.webui_send(&msg);
            }
        }

        pub(in super::super) fn webui_tick(&self, tick: u32) {
            // A dialog runs inside the page's handler: no page dropped or
            // created under it (a failure is handled when it closes).
            if self.webui.in_page_call.get() {
                return;
            }
            if self.webui.leaving.replace(false) {
                // Back to the classic window: its WebView2 is freed.
                self.webui.host.replace(None);
            }
            if self.webui.host_pending.replace(false) && self.webui.active.get() && !self.webui.switching.get() && self.webui.host.borrow().is_none() {
                self.webui_create_page();
            }
            if self.webui.options_pending.replace(false) && self.webui_visible() {
                self.webui_options_menu();
            }
            if self.webui.updates_pending.replace(false) && self.webui.active.get() {
                // As the Options menu's item: the Store version is told where
                // its updates come from and offered its Store page.
                self.check_for_update(true);
                // The page said "Checking…" at the click; now it hears
                // whether a check really runs (none for a managed copy).
                self.webui_settings_changed();
            }
            if self.webui.active.get() && !self.webui.switching.get() {
                if let Some(browser_gone) = self.webui.failed.take() {
                    self.webui_failed(browser_gone, "a WebView2 process failed");
                } else if self.webui.loading_since.get().is_some_and(|t| t.elapsed() > PAGE_TIMEOUT) {
                    self.webui_failed(true, "the page did not load");
                } else {
                    self.webui_forgive();
                }
            }
            if self.webui.active.get() && !self.webui.switching.get() {
                self.webui_idle();
            }
            if !self.webui.active.get() || !self.webui.ready.get() {
                return;
            }
            if self.webui.settings_stale.replace(false) {
                self.webui_send_settings();
            }
            self.webui_poll_content();
            if !tick.is_multiple_of(5) {
                return;
            }
            self.webui_status(false);
            // A dialog closed while it waited: the page leaves the mode.
            if self.webui.quick_switch.borrow().is_some() {
                self.quick_switch_waiting();
            }
            if self.webui.running.get() || !self.webui.parts.window.visible() {
                return;
            }
            let stale = {
                let st = self.state.borrow();
                let lists = self.webui.lists.borrow();
                // A search inside documents is not run again by itself (it
                // reads every document); its renamed rows are kept in `changed`.
                lists.shown.as_ref().filter(|s| !s.params.content && !Arc::ptr_eq(&s.index, &st.index)).map(|s| s.params.clone())
            };
            if let Some(p) = stale {
                let params = self.webui_params(p);
                self.webui_search(params);
            }
        }

        /// The window's index is about to change: a results list that
        /// points into it keeps its own rows from here on (`Shown::detach`;
        /// every row id still gives the same file, or nothing), so the index
        /// has one owner and is changed in place instead of copied, also
        /// while the window is hidden. The list then counts as out of date
        /// and `webui_tick` searches again once the window shows.
        pub(in super::super) fn webui_release_index(&self) {
            let (Ok(st), Ok(mut lists)) = (self.state.try_borrow(), self.webui.lists.try_borrow_mut()) else { return };
            if let Some(shown) = lists.shown.as_mut()
                && Arc::ptr_eq(&shown.index, &st.index)
            {
                shown.detach();
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

    /// Started with Windows (the `Run` entry's `--startup`, or the Store
    /// version's startup task): wait in the tray until called.
    fn started_hidden() -> bool {
        autostart::launched_at_sign_in()
    }

    /// Puts window `to` where `from` is, with its size, so a switch of
    /// look keeps the window in place (not from a minimised or maximised
    /// window, nor one never shown).
    fn place_like(from: &nwg::Window, to: &nwg::Window) {
        use winapi::um::winuser::IsZoomed;
        let Some(h) = from.handle.hwnd() else { return };
        if !from.visible() || unsafe { IsIconic(h) != 0 || IsZoomed(h) != 0 } {
            return;
        }
        let (x, y) = from.position();
        let (w, ht) = from.size();
        to.set_position(x, y);
        to.set_size(w, ht);
    }

    /// Puts menu items (nwg handles; a separator has no text) at the top of
    /// their menu when `show`, in this order, or takes them out (Win32 menus
    /// can't hide an item). Their ids stay, so nwg's events still name
    /// them; an item already where it should be is left alone.
    fn show_menu_items(items: &[(&nwg::ControlHandle, Option<&str>)], show: bool) {
        use winapi::um::winuser::{GetMenuState, InsertMenuItemW, MENUITEMINFOW, MF_BYCOMMAND, MFT_SEPARATOR, MFT_STRING, MIIM_FTYPE, MIIM_ID, MIIM_STRING, RemoveMenu};
        for (pos, (item, text)) in items.iter().enumerate() {
            let Some((menu, id)) = item.hmenu_item() else { continue };
            let present = unsafe { GetMenuState(menu, id, MF_BYCOMMAND) } != u32::MAX;
            if present == show {
                continue;
            }
            if !show {
                unsafe { RemoveMenu(menu, id, MF_BYCOMMAND) };
                continue;
            }
            let mut wide = shell::wide(text.unwrap_or(""));
            let mut info: MENUITEMINFOW = unsafe { std::mem::zeroed() };
            info.cbSize = std::mem::size_of::<MENUITEMINFOW>() as u32;
            info.fMask = MIIM_FTYPE | MIIM_ID;
            info.wID = id;
            if text.is_some() {
                info.fMask |= MIIM_STRING;
                info.fType = MFT_STRING;
                info.dwTypeData = wide.as_mut_ptr();
            } else {
                info.fType = MFT_SEPARATOR;
            }
            unsafe { InsertMenuItemW(menu, pos as u32, 1, &info) };
        }
    }
}

/// mingw builds have no WebView2 host (see Cargo.toml): always the classic window.
#[cfg(not(target_env = "msvc"))]
mod imp {
    use native_windows_gui as nwg;
    use std::rc::Rc;

    use super::super::App;

    pub(in super::super) fn available() -> bool {
        false
    }

    pub(in super::super) fn wanted(_new_ui: bool) -> bool {
        false
    }

    #[derive(Default)]
    pub(in super::super) struct WebUi;

    impl App {
        pub(in super::super) fn setup_webui(self: &Rc<Self>) -> Option<(nwg::EventHandler, nwg::RawEventHandler)> {
            if crate::webui::chosen(self.state.borrow().settings.new_ui) {
                super::super::startup::log("webui: the new interface needs the MSVC build; using the classic window");
            }
            None
        }

        /// Never called: the classic window offers no new look here.
        pub(in super::super) fn webui_menu_item(self: &Rc<Self>) {}

        pub(in super::super) fn place_webui_note(&self, _x: i32, bottom: i32, _w: i32) -> i32 {
            bottom
        }

        pub(in super::super) fn webui_visible(&self) -> bool {
            false
        }

        pub(in super::super) fn webui_ready(&self) -> bool {
            false
        }

        pub(in super::super) fn webui_in_use(&self) -> bool {
            false
        }

        pub(in super::super) fn webui_window(&self) -> Option<&nwg::Window> {
            None
        }

        pub(in super::super) fn webui_tell_switch(&self) {}

        pub(in super::super) fn webui_settings_changed(&self) {}

        pub(in super::super) fn webui_shed_thumbs(&self) {}

        pub(in super::super) fn webui_busy(&self) -> bool {
            false
        }

        pub(in super::super) fn webui_options_menu(&self) -> bool {
            false
        }

        pub(in super::super) fn webui_results(&self) -> Option<Vec<crate::webui::bridge::ExportRow>> {
            None
        }

        pub(in super::super) fn webui_scope(&self) -> Option<String> {
            None
        }

        pub(in super::super) fn webui_hide_main(&self) -> bool {
            false
        }

        pub(in super::super) fn webui_step_aside(&self) -> bool {
            false
        }

        pub(in super::super) fn webui_take_search(&self, _q: &str, _how: super::Handover) -> bool {
            false
        }

        pub(in super::super) fn webui_ask_rating(&self) -> bool {
            false
        }

        pub(in super::super) fn webui_quick_switch(&self, _hint: Option<&str>) {}

        pub(in super::super) fn webui_status_text(&self, _text: &str) {}

        pub(in super::super) fn with_modal_owner<R>(&self, f: impl FnOnce(winapi::shared::windef::HWND) -> R) -> R {
            f(self.window.handle.hwnd().unwrap_or(std::ptr::null_mut()))
        }

        pub(in super::super) fn webui_show(&self) -> bool {
            false
        }

        pub(in super::super) fn webui_toggle(&self) -> bool {
            false
        }

        pub(in super::super) fn webui_close(&self) {}

        pub(in super::super) fn webui_tick(&self, _tick: u32) {}

        pub(in super::super) fn webui_matches_read(&self, _seq: u64, _ver: u64, _id: u32, _found: Option<(u32, Vec<(u32, String)>)>) {}

        pub(in super::super) fn webui_release_index(&self) {}
    }
}
