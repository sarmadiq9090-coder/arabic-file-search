//! The new main window (the new interface, opt-in while it is built:
//! `--webui` or `BAHTH_WEBUI=1`, see `crate::webui`).
//!
//! The program runs as always (index, drive watcher, tray icon, Ctrl+Alt+F
//! and its quick popup, the box above the clock, updates); only the main
//! window is replaced: the classic window stays hidden and a window holding
//! the page (`crate::webui::host`) takes its place and its title, so a second
//! launch brings it to the front. Without WebView2, in a mingw build, or when
//! the page can't be created (tried twice), the classic window is used as
//! before.
//!
//! Phase 2 brings the classic window's features to the page: search inside
//! documents (snippets, match counts, every matching line), the size, date
//! and folder filters, sorting, the right-click actions (copy the file,
//! rename, Recycle Bin, search in this folder…), recent and saved searches,
//! and the settings. Every action names a row the page was sent, never a
//! path (`crate::webui::bridge`).

pub(super) use imp::{WebUi, wanted};

#[cfg(target_env = "msvc")]
mod imp {
    use native_windows_gui as nwg;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex, OnceLock};

    use winapi::shared::minwindef::{LPARAM, UINT, WPARAM};
    use winapi::shared::windef::HWND;
    use winapi::um::winuser::{EnableWindow, GetForegroundWindow, IsIconic, IsWindowEnabled, SW_RESTORE, SetForegroundWindow, ShowWindow, WA_INACTIVE, WM_ACTIVATE};

    use super::super::lazy::Lazy;
    use super::super::startup::log;
    use super::super::{App, WS_EX_LAYOUTRTL, autostart, content_index, shell, surface, title};
    use crate::history::SavedSearch;
    use crate::i18n::{self, Lang};
    use crate::search::{self, ContentHit, ContentProgress, KINDS};
    use crate::settings::{FloatingMode, HotkeyOpens};
    use crate::webui::bridge::{self, ContentState, List, Lists, Miss, Outbound, Params, Request, Setting, SettingsOut, Shown, Sort};
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
        next_lang: Cell<Option<Lang>>,
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

        /// Whether the new main window is the one in use and on screen.
        pub(in super::super) fn webui_visible(&self) -> bool {
            self.webui.active.get() && self.webui.parts.window.visible()
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
            self.webui_stop_content();
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

        fn webui_notice(&self, text: impl Into<String>) {
            self.webui_send(&Outbound::Notice { text: text.into() });
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
            let ver = self.webui.lists.borrow_mut().set_recent(paths);
            self.webui_send(&Outbound::Recent { ver, rows });
        }

        fn webui_send_history(&self) {
            let msg = bridge::history(&self.state.borrow().history);
            self.webui_send(&msg);
        }

        fn webui_send_settings(&self) {
            let current = i18n::current();
            let msg = {
                let s = &self.state.borrow().settings;
                Outbound::Settings(SettingsOut {
                    lang: current.code(),
                    next_lang: self.webui.next_lang.get().unwrap_or(current).code(),
                    digits: s.arabic_digits,
                    autostart: autostart::is_enabled(),
                    content_index: s.content_index,
                    auto_index: s.auto_index,
                    hide_system: s.hide_system,
                    hotkey_quick: s.hotkey_opens == HotkeyOpens::Quick,
                    floating: s.floating_mode != FloatingMode::Off,
                    tray_icon: s.tray_icon,
                    version: env!("CARGO_PKG_VERSION"),
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
            let lang = i18n::current();
            match req {
                Request::Hello {} => {
                    // A new page (the first, or WebView2 loaded it again): it
                    // lists nothing yet, so nothing of the page before is
                    // acted on or sent to it.
                    self.webui_stop_content();
                    self.webui.lists.borrow_mut().reset();
                    self.webui.ready.set(true);
                    self.webui.last_status.replace(String::new());
                    let hello = {
                        let s = &self.state.borrow().settings;
                        Outbound::Hello {
                            kinds: bridge::kinds(lang),
                            sizes: bridge::labels(bridge::SIZES, lang),
                            dates: bridge::labels(bridge::DATES, lang),
                            kind: s.kind.min(KINDS.len() - 1),
                            sort: Sort::from_column(s.sort_col),
                            desc: s.sort_desc,
                            digits: s.arabic_digits,
                            version: env!("CARGO_PKG_VERSION"),
                        }
                    };
                    self.webui_send(&hello);
                    let scope = self.webui.scope.borrow().clone();
                    self.webui_send(&Outbound::Scope { path: scope, search: false });
                    self.webui_send_settings();
                    self.webui_status(true);
                    self.webui_send_recent();
                    self.webui_send_history();
                    self.webui.handed.replace(String::new());
                    self.webui_hand_over_query();
                }
                Request::Search { seq, q, kind, size, date, content, sort, desc } => {
                    {
                        let s = &mut self.state.borrow_mut().settings;
                        s.kind = kind;
                        s.sort_col = sort.column();
                        s.sort_desc = desc;
                    }
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
                    let msg = self.webui.lists.borrow().shown_at(seq, ver).ok().and_then(|s| s.matches(id));
                    if let Some(msg) = msg {
                        self.webui_send(&msg);
                    }
                }
                Request::Open { list, seq, ver, id } => {
                    if let Some(path) = self.webui_path(list, seq, ver, id) {
                        self.webui_note_use(&path);
                        if !shell::open(&path) {
                            self.webui_notice(trf!("تعذر فتح الملف: {}", "Could not open the file: {}", path));
                        }
                    }
                }
                Request::Reveal { list, seq, ver, id } => {
                    if let Some(path) = self.webui_path(list, seq, ver, id) {
                        self.webui_note_use(&path);
                        shell::reveal(&path);
                    }
                }
                Request::Copy { list, seq, ver, id } => {
                    if let Some(path) = self.webui_path(list, seq, ver, id) {
                        nwg::Clipboard::set_data_text(&self.webui.parts.window, &path);
                        self.webui_notice(tr!("تم نسخ المسار.", "Path copied."));
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
                            nwg::Clipboard::set_data_text(&self.webui.parts.window, &text);
                            self.webui_notice(trf!("تم نسخ {} مسار.", "Copied {} paths.", crate::indexing_status::group_digits(n)));
                        }
                        None => self.webui_notice(tr!("لا توجد نتائج لنسخها.", "There are no results to copy.")),
                    }
                }
                Request::Rename { list, seq, ver, id, name } => self.webui_rename(list, seq, ver, id, &name),
                Request::Recycle { list, seq, ver, id } => self.webui_recycle(list, seq, ver, id),
                Request::Scope { list, seq, ver, id } => {
                    let Some(path) = self.webui_path(list, seq, ver, id) else { return };
                    let is_dir = match list {
                        List::Results => self.webui.lists.borrow().shown_at(seq, ver).is_ok_and(|s| s.is_dir(id)),
                        List::Recent => std::path::Path::new(&path).is_dir(),
                    };
                    let scope = bridge::scope_for(&path, is_dir);
                    if !scope.is_empty() {
                        self.webui_set_scope(scope, true);
                    }
                }
                Request::PickFolder {} => self.webui_pick_folder(),
                Request::AllDrives {} => self.webui_set_scope(String::new(), true),
                Request::Saved { id } => {
                    let Some(s) = self.state.borrow().history.saved.get(id as usize).cloned() else { return };
                    self.webui_set_scope(s.scope, false);
                    self.webui_send(&Outbound::Apply { q: s.query, kind: s.kind.min(KINDS.len() - 1), content: s.content });
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
                    let chosen = chosen.lang();
                    if i18n::save(chosen) {
                        self.menu_lang_ar.set_checked(chosen == Lang::Ar);
                        self.menu_lang_en.set_checked(chosen == Lang::En);
                        self.webui.next_lang.set(Some(chosen));
                    } else {
                        self.webui_notice(tr!("تعذر حفظ اللغة.", "Could not save the language."));
                    }
                    self.webui_send_settings();
                }
                Request::Restart {} => match crate::update::restart_new_version() {
                    Ok(()) => {
                        // As after an update: save everything and leave at
                        // once, so the new copy gets the single-instance lock.
                        self.on_close();
                        std::process::exit(0);
                    }
                    Err(e) => self.webui_notice(trf!(
                        "تعذرت إعادة التشغيل تلقائياً ({e}). أغلق البرنامج وافتحه من جديد.",
                        "The program could not restart itself ({e}). Close the program and open it again."
                    )),
                },
                Request::Reindex {} => {
                    if self.webui.content.borrow().is_some() {
                        self.webui_notice(tr!("انتظر انتهاء البحث داخل المستندات أو أوقفه أولاً.", "Wait for the document search to finish, or stop it first."));
                        return;
                    }
                    self.start_scan();
                    let st = self.state.borrow();
                    let text = if st.loading.is_some() {
                        tr!("انتظر حتى يكتمل تحميل الفهرس، ثم حدّثه.", "Wait until the index has loaded, then update it.")
                    } else {
                        tr!("جاري فهرسة الملفات على كل الأقراص…", "Indexing files on all drives…")
                    };
                    drop(st);
                    self.webui_notice(text);
                    self.webui_status(true);
                }
                Request::Web { q } => {
                    if !shell::open(&crate::suggest::web_url(&q)) {
                        self.webui_notice(tr!("تعذر فتح المتصفح.", "Could not open the browser."));
                    }
                }
                Request::Hide {} => self.webui_hide(),
            }
        }

        /// The full path of row `id` of version `ver` of `list`, from what
        /// the page was sent; `None`, and a word to the user when the list
        /// changed since, when it is not there.
        fn webui_path(&self, list: List, seq: u64, ver: u64, id: u32) -> Option<String> {
            let found = self.webui.lists.borrow().path(list, seq, ver, id).map(str::to_string);
            found.map_err(|miss| self.webui_missed(miss)).ok()
        }

        fn webui_missed(&self, miss: Miss) {
            if let Some(text) = miss.message(i18n::current()) {
                self.webui_notice(text);
            }
        }

        /// A result was used: its search goes into the history and the file
        /// ranks higher next time (as in the classic window).
        fn webui_note_use(&self, path: &str) {
            let text = self.webui.lists.borrow().shown.as_ref().map(|s| s.params.text.clone()).unwrap_or_default();
            let mut st = self.state.borrow_mut();
            if !text.trim().is_empty() {
                st.history.add_recent(&text);
            }
            st.history.record_open(path, crate::index::now_secs());
            st.history.save();
            drop(st);
            self.webui_send_history();
        }

        /// Forgets the recent searches, or deletes the saved ones.
        fn webui_clear_history(&self, recent: bool) {
            {
                let mut st = self.state.borrow_mut();
                if recent {
                    st.history.recent.clear();
                } else {
                    st.history.saved.clear();
                }
                st.history.save();
            }
            self.webui_send_history();
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
                self.webui_notice(why);
                return;
            }
            let new = match bridge::rename_target(&old, name) {
                Ok(new) => new,
                Err(bridge::BadName::Unchanged) => return,
                Err(e) => {
                    self.webui_notice(e.message(i18n::current()));
                    return;
                }
            };
            let new_name = name.trim().to_string();
            // Another file by that name: said before trying, and the rename
            // itself never replaces one (one that appears meanwhile makes it
            // fail). Changing only the letter case of the same file is fine.
            let taken = || trf!("يوجد ملف آخر بالاسم «{new_name}» في هذا المجلد.", "Another file named “{new_name}” already exists in this folder.");
            if std::path::Path::new(&new).exists() && new.to_lowercase() != old.to_lowercase() {
                self.webui_notice(taken());
                return;
            }
            if let Err(e) = shell::rename_no_replace(&old, &new) {
                self.webui_notice(if shell::already_exists(&e) { taken() } else { trf!("تعذر تغيير الاسم: {e}", "Could not rename: {e}") });
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
                self.webui_notice(why);
                return;
            }
            if !shell::recycle(&path) {
                self.webui_notice(tr!("لم يُحذف الملف.", "The file was not deleted."));
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
            // Nothing is borrowed while it runs: its loop delivers the page's messages.
            let chosen = if dialog.run(Some(&self.webui.parts.window)) { dialog.get_selected_item().ok() } else { None };
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
                        self.webui_notice(tr!("تعذر تغيير إعداد التشغيل مع ويندوز.", "Could not change the start-with-Windows setting."));
                    }
                }
                Setting::ContentIndex => {
                    if self.menu_content_index.checked() != on {
                        self.toggle_content_index();
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
            let progress = Arc::new(ContentProgress::default());
            let hits = Arc::new(Mutex::new(Vec::new()));
            let (p, h, idx, scope) = (progress.clone(), hits.clone(), index.clone(), params.scope.clone());
            let started = crate::crash_log::spawn_named("webui-content", move || {
                let cache = if use_index { content_index::shared() } else { None };
                let started = std::time::Instant::now();
                // After a panic (logged by the hook) the search still ends.
                if crate::crash_log::catch(|| search::search_content(&idx.entries, &q, &scope, &p, &h, cache)).is_none() {
                    p.done.store(true, Ordering::Relaxed);
                }
                log(&format!(
                    "webui content search: {} documents ({} from the content index) in {} ms",
                    p.checked.load(Ordering::Relaxed),
                    p.from_cache.load(Ordering::Relaxed),
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
        pub(in super::super) fn webui_tick(&self, tick: u32) {
            if !self.webui.active.get() || !self.webui.ready.get() {
                return;
            }
            self.webui_poll_content();
            if !tick.is_multiple_of(5) {
                return;
            }
            self.webui_status(false);
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

        pub(in super::super) fn webui_visible(&self) -> bool {
            false
        }

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
    }
}
