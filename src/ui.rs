//! The Windows user interface, built on native Win32 controls so Arabic text
//! is shaped, joined and laid out right-to-left by Windows itself.

use native_windows_gui as nwg;
use nwg::NativeUi;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::Write;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use winapi::shared::minwindef::{LPARAM, UINT, WPARAM};
use winapi::shared::windef::{HWND, POINT, RECT};
use winapi::um::commctrl::{
    LVIF_IMAGE, LVIF_TEXT, LVM_GETCOLUMNWIDTH, LVM_SETIMAGELIST, LVN_GETDISPINFOW, LVS_EX_DOUBLEBUFFER,
    LVS_OWNERDATA, LVS_SHAREIMAGELISTS, LVSIL_SMALL, NMLVDISPINFOW, SB_SETPARTS,
};
use winapi::um::winuser::{
    EM_SETSEL, GetCursorPos, GetKeyState, GetWindowRect, IsIconic, IsZoomed, NMHDR, SW_RESTORE,
    SendMessageW, SetForegroundWindow, ShowWindow, VK_CONTROL, VK_DOWN, VK_F3, VK_UP, WM_NOTIFY, WM_SETFONT,
};

use crate::index::{self, Entry, Index, ScanProgress};
use crate::layout;
use crate::history::{History, SavedSearch};
use crate::search::{self, ContentHit, ContentProgress, Query, SortKey};
use crate::settings::Settings;

/// Mirrors the whole window so it reads right-to-left.
const WS_EX_LAYOUTRTL: u32 = 0x0040_0000;

/// The window title; also how a second copy of the program finds the first.
const TITLE: &str = "بحث سريع";

const COL_NAME: usize = 0;
const COL_PATH: usize = 1;
const COL_SIZE: usize = 2;
const COL_DATE: usize = 3;
const COL_SNIPPET: usize = 4;
const COLUMNS: [(&str, i32); 5] =
    [("الاسم", 260), ("المسار الكامل", 460), ("الحجم", 90), ("تاريخ التعديل", 140), ("النص المطابق", 360)];

/// Set once the message loop runs, so start-up timing ends at a real tick.
static LOOP_STARTED: AtomicBool = AtomicBool::new(false);

/// Show numbers with Arabic-Indic digits.
static ARABIC_DIGITS: AtomicBool = AtomicBool::new(false);

/// A result row: an index into `State::rows_index` plus an optional snippet.
struct Row {
    entry: u32,
    snippet: Option<String>,
}

struct ScanJob {
    progress: Arc<ScanProgress>,
    result: Arc<Mutex<Option<Index>>>,
}

struct ContentJob {
    progress: Arc<ContentProgress>,
    hits: Arc<Mutex<Vec<ContentHit>>>,
}

/// The cached index being read from disk in the background. The inner
/// `Option` is `None` when there is no usable cache.
type LoadJob = Arc<Mutex<Option<Option<Index>>>>;

#[derive(Default)]
struct State {
    index: Arc<Index>,
    /// The index snapshot that `rows` point into.
    rows_index: Arc<Index>,
    rows: Vec<Row>,
    /// Folder the search is restricted to; empty means all drives.
    scope: String,
    loading: Option<LoadJob>,
    scan: Option<ScanJob>,
    content: Option<ContentJob>,
    /// A filename search is running on a background thread.
    name_running: bool,
    /// The query changed while a search was running; search again when it ends.
    name_pending: bool,
    /// The query behind the current rows, used to underline the matches.
    shown_query: Option<Query>,
    /// Recent and saved searches and how often files were opened.
    history: History,
    settings: Settings,
}

/// What a background update thread reports back.
enum UpdateEvent {
    /// Result of a check; `manual` checks also report "up to date" and errors.
    Checked { manual: bool, result: Result<Option<crate::update::Release>, String> },
    Installed(Result<(), String>),
}

/// The pop-up menu of saved and recent searches and what each item does.
#[derive(Default)]
struct HistoryMenu {
    menu: nwg::Menu,
    items: Vec<(nwg::MenuItem, HistoryAction)>,
    separators: Vec<nwg::MenuSeparator>,
}

enum HistoryAction {
    Saved(SavedSearch),
    Recent(String),
    SaveCurrent,
    ClearRecent,
    ClearSaved,
    Nothing,
}

/// Id of the global "show the program" hotkey, Ctrl+Alt+F.
const HOTKEY_ID: i32 = 1;
const HOTKEY_TEXT: &str = "Ctrl+Alt+F";

/// The outcome of a background filename search.
struct NameResult {
    index: Arc<Index>,
    hits: Vec<u32>,
    text: String,
    /// The query re-typed in the other keyboard layout, when that found the hits.
    converted: Option<String>,
    empty: bool,
    ms: usize,
}

#[derive(Default)]
pub struct App {
    window: nwg::Window,
    font: nwg::Font,
    query_font: nwg::Font,
    query: nwg::TextInput,
    search_btn: nwg::Button,
    history_btn: nwg::Button,
    kind_combo: nwg::ComboBox<&'static str>,
    content_chk: nwg::CheckBox,
    scope_label: nwg::Label,
    scope_input: nwg::TextInput,
    scope_btn: nwg::Button,
    all_btn: nwg::Button,
    reindex_btn: nwg::Button,
    list: nwg::ListView,
    open_btn: nwg::Button,
    folder_btn: nwg::Button,
    copy_btn: nwg::Button,
    export_btn: nwg::Button,
    stop_btn: nwg::Button,
    options_btn: nwg::Button,
    status: nwg::StatusBar,
    timer: nwg::AnimationTimer,
    /// Wakes the UI thread when a background filename search finishes.
    name_notice: nwg::Notice,
    name_result: Arc<Mutex<Option<NameResult>>>,
    /// Wakes the UI thread when an update check or download finishes.
    update_notice: nwg::Notice,
    update_result: Arc<Mutex<Option<UpdateEvent>>>,
    /// Bytes downloaded / total of a running update download.
    update_progress: Arc<(AtomicU64, AtomicU64)>,
    update_busy: Cell<bool>,
    /// Timer ticks since start, to delay the automatic update check.
    ticks: Cell<u32>,
    /// Built on first use: each one starts a COM file dialog, which is slow.
    folder_dialog: RefCell<Option<nwg::FileDialog>>,
    export_dialog: RefCell<Option<nwg::FileDialog>>,
    menu: nwg::Menu,
    menu_open: nwg::MenuItem,
    menu_folder: nwg::MenuItem,
    menu_copy: nwg::MenuItem,
    menu_copy_all: nwg::MenuItem,
    menu_copy_file: nwg::MenuItem,
    menu_rename: nwg::MenuItem,
    menu_delete: nwg::MenuItem,
    options_menu: nwg::Menu,
    menu_hotkey: nwg::MenuItem,
    menu_digits: nwg::MenuItem,
    menu_update: nwg::MenuItem,
    menu_export: nwg::MenuItem,
    menu_hide: nwg::MenuItem,
    tray_icon: nwg::Icon,
    tray: nwg::TrayNotification,
    tray_menu: nwg::Menu,
    tray_show: nwg::MenuItem,
    tray_exit: nwg::MenuItem,
    /// The recent/saved searches menu, rebuilt each time it opens.
    history_menu: RefCell<HistoryMenu>,
    /// True while the user edits a file name in the list (F2).
    renaming: Cell<bool>,
    /// The file whose name is being edited, fixed when the edit starts so a
    /// change of the results meanwhile can't redirect the rename.
    renaming_path: RefCell<Option<String>>,
    /// System image-list index of the icon for each extension.
    icons: RefCell<HashMap<String, i32>>,
    tray_hint_shown: Cell<bool>,
    startup_logged: Cell<bool>,
    state: RefCell<State>,
}

impl App {
    /// Height of the status bar in logical units (it sizes itself to its font).
    fn status_height(&self) -> i32 {
        let Some(hwnd) = self.status.handle.hwnd() else { return 26 };
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe {
            // Let the status bar fit itself to the window and its font first.
            SendMessageW(hwnd, winapi::um::winuser::WM_SIZE, 0, 0);
            GetWindowRect(hwnd, &mut r);
        }
        ((r.bottom - r.top) as f64 / nwg::scale_factor()).round() as i32
    }

    // ---------------------------------------------------------------- layout

    fn layout(&self) {
        let (w, h) = self.window.size();
        let (w, h) = (w as i32, h as i32);
        let m = 12;
        let row_h = 32;
        let query_h = 38;
        let btn_w = 120;

        // Row 1: a large query box, the search button and the history menu.
        let hist_w = 90;
        self.query.set_position(m, m);
        self.query.set_size((w - 4 * m - btn_w - hist_w).max(100) as u32, query_h as u32);
        self.history_btn.set_position(w - 2 * m - btn_w - hist_w, m);
        self.history_btn.set_size(hist_w as u32, query_h as u32);
        self.search_btn.set_position(w - m - btn_w, m);
        self.search_btn.set_size(btn_w as u32, query_h as u32);

        // Row 2: file-type filter, content checkbox, scope and index buttons.
        let y2 = m * 2 + query_h;
        let mut x = m;
        self.kind_combo.set_position(x, y2 + 1);
        self.kind_combo.set_size(150, 300);
        x += 150 + m;
        self.content_chk.set_position(x, y2);
        self.content_chk.set_size(230, row_h as u32);
        x += 230 + m;
        self.scope_label.set_position(x, y2 + 6);
        self.scope_label.set_size(55, row_h as u32);
        x += 55;
        let right_buttons = 3 * (btn_w + m);
        let scope_w = (w - x - right_buttons - m).max(80);
        self.scope_input.set_position(x, y2);
        self.scope_input.set_size(scope_w as u32, row_h as u32);
        x += scope_w + m;
        for b in [&self.scope_btn, &self.all_btn, &self.reindex_btn] {
            b.set_position(x, y2);
            b.set_size(btn_w as u32, row_h as u32);
            x += btn_w + m;
        }

        // Bottom: action buttons above the status bar, options at the far end.
        let status_h = self.status_height();
        let y_btn = h - status_h - m - row_h;
        let mut x = m;
        for b in [&self.open_btn, &self.folder_btn, &self.copy_btn, &self.export_btn, &self.stop_btn] {
            b.set_position(x, y_btn);
            b.set_size(btn_w as u32, row_h as u32);
            x += btn_w + m;
        }
        self.options_btn.set_position(w - m - btn_w, y_btn);
        self.options_btn.set_size(btn_w as u32, row_h as u32);

        // The result list fills the rest.
        let y3 = y2 + row_h + m;
        self.list.set_position(m, y3);
        self.list.set_size((w - 2 * m).max(100) as u32, (y_btn - m - y3).max(50) as u32);

        // Status bar: messages on one side, the selected file's path on the other.
        if let Some(sb) = self.status.handle.hwnd() {
            let px = (w as f64 * nwg::scale_factor()) as i32;
            let parts: [i32; 2] = [px * 45 / 100, -1];
            unsafe { SendMessageW(sb, SB_SETPARTS, 2, parts.as_ptr() as LPARAM) };
        }
    }

    // ---------------------------------------------------------------- index

    /// Reads the cached index on a background thread so the window appears at once.
    fn start(&self) {
        let slot: LoadJob = Arc::new(Mutex::new(None));
        let s = slot.clone();
        std::thread::spawn(move || {
            background_priority();
            let started = std::time::Instant::now();
            let idx = Index::load(&index::cache_path());
            log(&format!(
                "index load: {} entries in {} ms",
                idx.as_ref().map_or(0, |i| i.entries.len()),
                started.elapsed().as_millis()
            ));
            if let Ok(mut slot) = s.lock() {
                *slot = Some(idx);
            }
        });
        self.state.borrow_mut().loading = Some(slot);
        self.set_status("جاري تحميل الفهرس…");
        self.query.set_focus();
    }

    fn poll_load(&self) {
        let mut st = self.state.borrow_mut();
        let Some(job) = &st.loading else { return };
        let Some(loaded) = job.lock().ok().and_then(|mut r| r.take()) else { return };
        st.loading = None;
        match loaded {
            Some(idx) => {
                let n = idx.entries.len();
                st.index = Arc::new(idx);
                drop(st);
                if self.query.text().trim().is_empty() || self.content_mode() {
                    self.set_status(&format!("تم تحميل الفهرس ({} عنصر). اكتب للبحث.", fmt_num(n)));
                } else {
                    self.search_names();
                }
            }
            None => {
                drop(st);
                self.start_scan();
            }
        }
    }

    fn start_scan(&self) {
        // An edit still waiting to be saved must not land after the new scan.
        flush_index_save();
        let mut st = self.state.borrow_mut();
        if st.loading.is_some() {
            drop(st);
            self.set_status("انتظر حتى يكتمل تحميل الفهرس، ثم حدّثه.");
            return;
        }
        if st.scan.is_some() {
            return;
        }
        let progress = Arc::new(ScanProgress::default());
        let result = Arc::new(Mutex::new(None));
        let (p, r) = (progress.clone(), result.clone());
        std::thread::spawn(move || {
            background_priority();
            let started = std::time::Instant::now();
            let idx = Index::build(&index::list_drives(), &p);
            log(&format!("index build: {} entries in {} ms", idx.entries.len(), started.elapsed().as_millis()));
            let _ = idx.save(&index::cache_path());
            if let Ok(mut slot) = r.lock() {
                *slot = Some(idx);
            }
        });
        st.scan = Some(ScanJob { progress, result });
        drop(st);
        self.reindex_btn.set_enabled(false);
        self.set_status("جاري فهرسة الملفات على كل الأقراص…");
    }

    // ---------------------------------------------------------------- search

    fn content_mode(&self) -> bool {
        self.content_chk.check_state() == nwg::CheckBoxState::Checked
    }

    fn kind(&self) -> usize {
        self.kind_combo.selection().unwrap_or(0)
    }

    fn parse_query(&self, text: &str) -> Query {
        let mut q = Query::parse(text);
        q.apply_kind(self.kind());
        q
    }

    /// Live filename search, run on every keystroke.
    fn on_query_changed(&self) {
        if !self.content_mode() {
            self.search_names();
        }
    }

    fn on_search(&self) {
        if self.content_mode() {
            self.search_content();
        } else {
            self.search_names();
        }
    }

    fn on_kind_changed(&self) {
        self.state.borrow_mut().settings.kind = self.kind();
        if !self.content_mode() {
            self.search_names();
        }
    }

    /// Starts a filename search on a background thread, so typing never
    /// waits for a search over millions of names. While one runs, later
    /// keystrokes are folded into a single follow-up search.
    fn search_names(&self) {
        self.cancel_content();
        let mut st = self.state.borrow_mut();
        if st.name_running {
            st.name_pending = true;
            return;
        }
        st.name_running = true;
        st.name_pending = false;
        let index = st.index.clone();
        let scope = st.scope.clone();
        let (sort, desc) = (SortKey::from_column(st.settings.sort_col.max(-1) as usize), st.settings.sort_desc);
        let sort = if st.settings.sort_col < 0 { SortKey::Relevance } else { sort };
        let boosted = if sort == SortKey::Relevance { st.history.boosted(&index, index::now_secs()) } else { Vec::new() };
        drop(st);

        let text = self.query.text();
        let kind = self.kind();
        let parse = move |t: &str| {
            let mut q = Query::parse(t);
            q.apply_kind(kind);
            q
        };
        let slot = self.name_result.clone();
        let notice = self.name_notice.sender();
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let q = parse(&text);
            let mut hits = search::search_names(&index.entries, &q, &scope);
            // Typed with the wrong keyboard layout? ("hgjrvdv" → "التقرير")
            let mut converted = None;
            if hits.is_empty() && !q.is_empty()
                && let Some(alt) = layout::convert_layout(&text)
            {
                let alt_hits = search::search_names(&index.entries, &parse(&alt), &scope);
                if !alt_hits.is_empty() {
                    hits = alt_hits;
                    converted = Some(alt);
                }
            }
            if sort == SortKey::Relevance {
                // Files the user opens often come first.
                search::promote(&mut hits, &boosted);
            } else {
                search::sort_hits(&index.entries, &mut hits, sort, desc);
            }
            let ms = started.elapsed().as_millis() as usize;
            if let Ok(mut r) = slot.lock() {
                *r = Some(NameResult { index, hits, text, converted, empty: q.is_empty(), ms });
            }
            notice.notice();
        });
    }

    fn on_names_found(&self) {
        let Some(res) = self.name_result.lock().ok().and_then(|mut r| r.take()) else { return };
        let mut st = self.state.borrow_mut();
        st.name_running = false;
        if st.content.is_some() || self.content_mode() {
            // A document search took over meanwhile; leave it alone.
            st.name_pending = false;
            return;
        }
        if st.name_pending {
            // The query changed meanwhile: these results are already stale.
            drop(st);
            self.search_names();
            return;
        }
        st.rows = res.hits.into_iter().map(|entry| Row { entry, snippet: None }).collect();
        st.rows_index = res.index;
        st.shown_query = Some(self.parse_query(res.converted.as_deref().unwrap_or(&res.text)));
        let n = st.rows.len();
        let busy = st.scan.is_some() || st.loading.is_some();
        let loading = st.loading.is_some();
        drop(st);
        self.clear_selection();
        self.refresh_list();
        if res.empty {
            if !busy {
                self.set_status("اكتب اسم الملف أو جزءاً منه للبحث.");
            }
            return;
        }
        let tail = match (loading, busy) {
            (true, _) => " (جاري تحميل الفهرس…)",
            (false, true) => " (الفهرسة ما زالت جارية)",
            _ => "",
        };
        match res.converted {
            Some(alt) => self.set_status(&format!(
                "لا نتائج لـ «{}»، عرض نتائج «{}»: {} نتيجة في {} ملي ثانية{tail}",
                res.text.trim(),
                alt,
                fmt_num(n),
                fmt_num(res.ms)
            )),
            None => self.set_status(&format!("وجدت {} نتيجة في {} ملي ثانية{tail}", fmt_num(n), fmt_num(res.ms))),
        }
    }

    fn search_content(&self) {
        self.cancel_content();
        let q = self.parse_query(&self.query.text());
        if q.terms.is_empty() {
            self.set_status("اكتب الكلمة أو العبارة التي تريد البحث عنها داخل المستندات.");
            return;
        }
        let mut st = self.state.borrow_mut();
        st.shown_query = Some(self.parse_query(&self.query.text()));
        st.history.add_recent(&self.query.text());
        let index = st.index.clone();
        let scope = st.scope.clone();
        let progress = Arc::new(ContentProgress::default());
        let hits = Arc::new(Mutex::new(Vec::new()));
        let (p, h, idx) = (progress.clone(), hits.clone(), index.clone());
        std::thread::spawn(move || search::search_content(&idx.entries, &q, &scope, &p, &h));
        st.content = Some(ContentJob { progress, hits });
        st.rows.clear();
        st.rows_index = index;
        drop(st);
        self.clear_selection();
        self.refresh_list();
        self.stop_btn.set_enabled(true);
        self.set_status("جاري البحث داخل المستندات…");
    }

    fn cancel_content(&self) {
        if let Some(job) = self.state.borrow_mut().content.take() {
            job.progress.cancel.store(true, Ordering::Relaxed);
        }
        self.stop_btn.set_enabled(false);
    }

    fn on_stop(&self) {
        let n = self.state.borrow().rows.len();
        self.cancel_content();
        self.set_status(&format!("تم إيقاف البحث. وجدت {} ملف.", fmt_num(n)));
    }

    /// Polls background jobs a few times per second.
    fn on_tick(&self) {
        // The first tick handled by the message loop, after the first paint.
        if LOOP_STARTED.load(Ordering::Relaxed) && !self.startup_logged.replace(true) {
            phase("first-tick");
            log_phases();
        }
        self.poll_load();
        self.poll_scan();
        self.poll_content();
        self.poll_update_progress();
        // Look for a new version about 10 s after start (off the busy
        // startup) and then every hour, for copies that live in the tray.
        let t = self.ticks.get().wrapping_add(1);
        self.ticks.set(t);
        if t % 18_000 == 50 {
            self.maybe_check_for_update();
        }
    }

    // ---------------------------------------------------------------- updates

    /// Looks for a newer release on GitHub in the background. The automatic
    /// check runs at most once a day and stays silent unless it finds one.
    fn check_for_update(&self, manual: bool) {
        if self.update_busy.replace(true) {
            if manual {
                self.set_status("جاري التحقق من التحديثات…");
            }
            return;
        }
        if manual {
            self.set_status("جاري التحقق من التحديثات…");
        }
        let slot = self.update_result.clone();
        let notice = self.update_notice.sender();
        std::thread::spawn(move || {
            background_priority();
            let result = crate::update::check_latest();
            if let Ok(mut s) = slot.lock() {
                *s = Some(UpdateEvent::Checked { manual, result });
            }
            notice.notice();
        });
    }

    fn maybe_check_for_update(&self) {
        let last = self.state.borrow().settings.update_check;
        if crate::update::should_check_now(last, index::now_secs().max(0) as u64) {
            self.check_for_update(false);
        }
    }

    fn on_update_event(&self) {
        let Some(event) = self.update_result.lock().ok().and_then(|mut s| s.take()) else { return };
        match event {
            UpdateEvent::Checked { manual, result } => {
                self.update_busy.set(false);
                // A failed check (no network yet, rate limit) is retried at
                // the next hourly tick instead of a day later.
                if result.is_ok() {
                    let mut st = self.state.borrow_mut();
                    st.settings.update_check = index::now_secs().max(0) as u64;
                    st.settings.save();
                }
                match result {
                    Ok(Some(rel)) => self.offer_update(rel),
                    Ok(None) if manual => {
                        let v = env!("CARGO_PKG_VERSION");
                        self.set_status(&format!("لديك أحدث إصدار ({v})."));
                        nwg::modal_info_message(&self.window, TITLE, &format!("لديك أحدث إصدار من البرنامج ({v})."));
                    }
                    Err(e) if manual => {
                        self.set_status("تعذر التحقق من التحديثات.");
                        nwg::modal_error_message(&self.window, TITLE, &format!("تعذر التحقق من التحديثات:\n{e}"));
                    }
                    _ => {}
                }
            }
            UpdateEvent::Installed(Ok(())) => {
                self.set_status("اكتمل التحديث. جاري إعادة التشغيل…");
                match crate::update::restart_new_version() {
                    Ok(()) => self.on_close(),
                    Err(e) => {
                        self.update_busy.set(false);
                        nwg::modal_info_message(
                            &self.window,
                            TITLE,
                            &format!("تم تثبيت الإصدار الجديد، لكن تعذرت إعادة التشغيل تلقائياً ({e}).\nأغلق البرنامج وافتحه من جديد."),
                        );
                    }
                }
            }
            UpdateEvent::Installed(Err(e)) => {
                self.update_busy.set(false);
                self.set_status("تعذر التحديث.");
                nwg::modal_error_message(&self.window, TITLE, &format!("تعذر تنزيل التحديث أو تثبيته:\n{e}\n\nالإصدار الحالي يعمل كما هو."));
            }
        }
    }

    fn offer_update(&self, rel: crate::update::Release) {
        // An automatic check must not pop up over a hidden window; show it.
        if !self.window.visible() {
            self.show_window();
        }
        let current = env!("CARGO_PKG_VERSION");
        let notes = if rel.notes.trim().is_empty() { String::new() } else { format!("\n\nالجديد:\n{}", rel.notes.trim()) };
        let text = format!(
            "يتوفر إصدار جديد من بحث سريع: {} (لديك {current}).{notes}\n\nهل تريد التحديث الآن؟ سيُعاد تشغيل البرنامج بعد التنزيل.",
            rel.version
        );
        let params = nwg::MessageParams {
            title: TITLE,
            content: &text,
            buttons: nwg::MessageButtons::YesNo,
            icons: nwg::MessageIcons::Question,
        };
        if nwg::modal_message(&self.window, &params) != nwg::MessageChoice::Yes {
            self.set_status(&format!("الإصدار {} متاح. يمكنك التحديث لاحقاً من «خيارات».", rel.version));
            return;
        }
        self.update_busy.set(true);
        self.save_settings();
        let total = rel.exe_size;
        self.update_progress.0.store(0, Ordering::Relaxed);
        self.update_progress.1.store(total, Ordering::Relaxed);
        let progress = self.update_progress.clone();
        let slot = self.update_result.clone();
        let notice = self.update_notice.sender();
        std::thread::spawn(move || {
            let result = crate::update::download_and_install(&rel, |done, total| {
                progress.0.store(done, Ordering::Relaxed);
                progress.1.store(total, Ordering::Relaxed);
            });
            progress.1.store(0, Ordering::Relaxed);
            if let Ok(mut s) = slot.lock() {
                *s = Some(UpdateEvent::Installed(result));
            }
            notice.notice();
        });
    }

    fn poll_update_progress(&self) {
        let total = self.update_progress.1.load(Ordering::Relaxed);
        if total > 0 && self.update_busy.get() {
            let done = self.update_progress.0.load(Ordering::Relaxed);
            self.set_status(&format!("جاري تنزيل التحديث… {}٪", (done * 100 / total).min(100)));
        }
    }

    fn poll_scan(&self) {
        let mut st = self.state.borrow_mut();
        let Some(job) = &st.scan else { return };
        let finished = job.result.lock().ok().and_then(|mut r| r.take());
        match finished {
            None => {
                let n = job.progress.files.load(Ordering::Relaxed);
                drop(st);
                self.set_status(&format!("جاري فهرسة الملفات… {} عنصر حتى الآن", fmt_num(n)));
            }
            Some(idx) => {
                let n = idx.entries.len();
                st.index = Arc::new(idx);
                st.scan = None;
                drop(st);
                self.reindex_btn.set_enabled(true);
                if self.query.text().trim().is_empty() || self.content_mode() {
                    self.set_status(&format!("اكتملت الفهرسة: {} عنصر.", fmt_num(n)));
                } else {
                    self.search_names();
                }
            }
        }
    }

    fn poll_content(&self) {
        let mut st = self.state.borrow_mut();
        let Some(job) = &st.content else { return };
        let new_hits: Vec<ContentHit> = job.hits.lock().map(|mut h| h.drain(..).collect()).unwrap_or_default();
        let total = job.progress.total.load(Ordering::Relaxed);
        let checked = job.progress.checked.load(Ordering::Relaxed);
        let done = job.progress.done.load(Ordering::Relaxed);
        let changed = !new_hits.is_empty();
        st.rows.extend(new_hits.into_iter().map(|h| Row { entry: h.index, snippet: Some(h.snippet) }));
        let n = st.rows.len();
        if done {
            st.content = None;
        }
        drop(st);
        if done {
            self.sort_rows_in_place();
        }
        if changed || done {
            self.refresh_list();
        }
        if done {
            self.stop_btn.set_enabled(false);
            self.set_status(&format!("انتهى البحث داخل {} مستند: وجدت {} ملف.", fmt_num(total), fmt_num(n)));
        } else {
            self.set_status(&format!(
                "جاري البحث داخل المستندات… تم فحص {} من {} — وجدت {}",
                fmt_num(checked),
                fmt_num(total),
                fmt_num(n)
            ));
        }
    }

    // ---------------------------------------------------------------- scope

    fn choose_folder(&self) {
        let mut slot = self.folder_dialog.borrow_mut();
        if slot.is_none() {
            let mut d = nwg::FileDialog::default();
            let built = nwg::FileDialog::builder()
                .title("اختر المجلد الذي تريد البحث فيه")
                .action(nwg::FileDialogAction::OpenDirectory)
                .build(&mut d);
            if built.is_err() {
                return;
            }
            *slot = Some(d);
        }
        let Some(dialog) = slot.as_ref() else { return };
        let chosen = if dialog.run(Some(&self.window)) { dialog.get_selected_item().ok() } else { None };
        drop(slot);
        if let Some(dir) = chosen {
            let mut dir = dir.to_string_lossy().into_owned();
            if !dir.ends_with('\\') {
                dir.push('\\');
            }
            self.set_scope(dir);
        }
    }

    fn set_scope(&self, scope: String) {
        self.scope_input.set_text(if scope.is_empty() { "كل الأقراص" } else { &scope });
        self.state.borrow_mut().scope = scope;
        if !self.query.text().trim().is_empty() && !self.content_mode() {
            self.search_names();
        }
    }

    // ---------------------------------------------------------------- results

    /// Drops the selection, so a row number never points at a different file
    /// after the results change.
    fn clear_selection(&self) {
        for i in self.list.selected_items() {
            self.list.select_item(i, false);
        }
    }

    fn refresh_list(&self) {
        let n = self.state.borrow().rows.len();
        self.list.set_item_count(n as u32);
        self.list.invalidate();
        self.show_selected_path();
    }

    fn selected_path(&self) -> Option<String> {
        let row = self.list.selected_item()?;
        let st = self.state.borrow();
        let r = st.rows.get(row)?;
        Some(st.rows_index.entries.get(r.entry as usize)?.path.to_string())
    }

    /// Shows the full path of the selected file in the status bar.
    fn show_selected_path(&self) {
        let text = self.selected_path().map(|p| ltr_path(&p)).unwrap_or_default();
        self.status.set_text(1, &text);
    }

    /// Remembers that a result was used: its search goes into the history and
    /// the file ranks higher next time.
    fn note_use(&self, path: &str) {
        let text = self.query.text();
        let mut st = self.state.borrow_mut();
        st.history.add_recent(&text);
        st.history.record_open(path, index::now_secs());
        st.history.save();
    }

    /// Keeps a search that found something in the recent list, so it can be
    /// picked again from the history menu.
    fn remember_query(&self) {
        let text = self.query.text();
        let mut st = self.state.borrow_mut();
        if !st.rows.is_empty() {
            st.history.add_recent(&text);
            st.history.save();
        }
    }

    fn open_selected(&self) {
        match self.selected_path() {
            Some(p) => {
                self.note_use(&p);
                if !shell::open(&p) {
                    nwg::modal_error_message(&self.window, "خطأ", &format!("تعذر فتح الملف:\n{p}"));
                }
            }
            None => self.set_status("اختر ملفاً من القائمة أولاً."),
        }
    }

    fn open_folder(&self) {
        match self.selected_path() {
            Some(p) => {
                self.note_use(&p);
                shell::reveal(&p);
            }
            None => self.set_status("اختر ملفاً من القائمة أولاً."),
        }
    }

    fn copy_path(&self) {
        match self.selected_path() {
            Some(p) => {
                nwg::Clipboard::set_data_text(&self.window, &p);
                self.set_status("تم نسخ المسار.");
            }
            None => self.set_status("اختر ملفاً من القائمة أولاً."),
        }
    }

    fn copy_all_paths(&self) {
        let st = self.state.borrow();
        let paths: Vec<&str> =
            st.rows.iter().filter_map(|r| st.rows_index.entries.get(r.entry as usize)).map(|e| &*e.path).collect();
        if paths.is_empty() {
            drop(st);
            self.set_status("لا توجد نتائج لنسخها.");
            return;
        }
        let n = paths.len();
        nwg::Clipboard::set_data_text(&self.window, &paths.join("\r\n"));
        drop(st);
        self.set_status(&format!("تم نسخ {} مسار.", fmt_num(n)));
    }

    /// Puts the selected file itself on the clipboard, ready to paste into
    /// Explorer, an e-mail or a chat.
    fn copy_file(&self) {
        let Some(p) = self.selected_path() else {
            self.set_status("اختر ملفاً من القائمة أولاً.");
            return;
        };
        let ok = self.window.handle.hwnd().is_some_and(|h| crate::dragdrop::copy_files(h, std::slice::from_ref(&p)));
        self.set_status(if ok { "تم نسخ الملف. الصقه حيث تريد بـ Ctrl+V." } else { "تعذر نسخ الملف." });
    }

    fn drag_row(&self, row: usize) {
        let path = {
            let st = self.state.borrow();
            st.rows.get(row).and_then(|r| st.rows_index.entries.get(r.entry as usize)).map(|e| e.path.to_string())
        };
        if let (Some(p), Some(h)) = (path, self.list.handle.hwnd()) {
            crate::dragdrop::drag_files_from(h, std::slice::from_ref(&p));
        }
    }

    // ---------------------------------------------------------------- sorting

    fn sort_key(&self) -> (SortKey, bool) {
        let st = self.state.borrow();
        let col = st.settings.sort_col;
        (if col < 0 { SortKey::Relevance } else { SortKey::from_column(col as usize) }, st.settings.sort_desc)
    }

    /// A click on a column header sorts by it; a second click reverses the
    /// order, and a third goes back to best match first.
    fn on_column_click(&self, col: usize) {
        let key = SortKey::from_column(col);
        if key == SortKey::Relevance {
            return;
        }
        {
            let mut st = self.state.borrow_mut();
            let s = &mut st.settings;
            if s.sort_col == col as i32 {
                if s.sort_desc {
                    s.sort_col = -1;
                    s.sort_desc = false;
                } else {
                    s.sort_desc = true;
                }
            } else {
                s.sort_col = col as i32;
                s.sort_desc = false;
            }
        }
        self.show_sort_arrow();
        if self.content_mode() || self.state.borrow().content.is_some() {
            self.sort_rows_in_place();
            self.clear_selection();
            self.refresh_list();
        } else {
            self.search_names();
        }
    }

    fn show_sort_arrow(&self) {
        let (key, desc) = self.sort_key();
        for c in 0..COLUMNS.len() {
            let arrow = match key.column() {
                Some(k) if k == c => {
                    Some(if desc { nwg::ListViewColumnSortArrow::Down } else { nwg::ListViewColumnSortArrow::Up })
                }
                _ => None,
            };
            self.list.set_column_sort_arrow(c, arrow);
        }
    }

    /// Sorts the current rows (used for document-search results).
    fn sort_rows_in_place(&self) {
        let (key, desc) = self.sort_key();
        let mut st = self.state.borrow_mut();
        let mut hits: Vec<u32> = st.rows.iter().map(|r| r.entry).collect();
        if key == SortKey::Relevance {
            // Document hits have no score; path order is the neutral order.
            hits.sort_unstable();
        } else {
            search::sort_hits(&st.rows_index.entries, &mut hits, key, desc);
        }
        let mut snippets: HashMap<u32, Option<String>> = st.rows.drain(..).map(|r| (r.entry, r.snippet)).collect();
        st.rows = hits.into_iter().map(|entry| Row { entry, snippet: snippets.remove(&entry).flatten() }).collect();
    }

    // ---------------------------------------------------------------- rename & delete

    /// Results of a running document search point into the index, so the
    /// index must not change under them.
    fn index_busy(&self) -> bool {
        let st = self.state.borrow();
        let msg = if st.content.is_some() {
            "انتظر انتهاء البحث داخل المستندات أو أوقفه أولاً."
        } else if st.scan.is_some() || st.loading.is_some() {
            "انتظر حتى تكتمل الفهرسة."
        } else {
            return false;
        };
        drop(st);
        self.set_status(msg);
        true
    }

    fn start_rename(&self) {
        let Some(row) = self.list.selected_item() else {
            self.set_status("اختر ملفاً من القائمة أولاً.");
            return;
        };
        if self.index_busy() {
            return;
        }
        if let Some(h) = self.list.handle.hwnd() {
            self.list.set_focus();
            unsafe { SendMessageW(h, winapi::um::commctrl::LVM_EDITLABELW, row, 0) };
        }
    }

    /// Starts an in-place rename. Returns true to refuse it.
    fn begin_rename(&self, row: usize) -> bool {
        if self.index_busy() {
            return true;
        }
        let path = {
            let st = self.state.borrow();
            st.rows.get(row).and_then(|r| st.rows_index.entries.get(r.entry as usize)).map(|e| e.path.to_string())
        };
        let refuse = path.is_none();
        *self.renaming_path.borrow_mut() = path;
        self.renaming.set(!refuse);
        refuse
    }

    /// Finishes an in-place rename. Returns true to accept the new label.
    fn finish_rename(&self, new_name: Option<String>) -> bool {
        self.renaming.set(false);
        let old = self.renaming_path.borrow_mut().take();
        let Some(new_name) = new_name.map(|n| n.trim().to_string()) else { return false };
        let Some(old) = old else { return false };
        let old_name = old.rsplit('\\').next().unwrap_or(&old).to_string();
        if new_name.is_empty() || new_name == old_name {
            return false;
        }
        if new_name.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) {
            self.set_status("لا يمكن أن يحتوي الاسم على أي من هذه الرموز: \\ / : * ? \" < > |");
            return false;
        }
        // Windows drops a trailing dot or space and reserves device names.
        let stem = new_name.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit());
        if new_name.ends_with(['.', ' ']) || reserved {
            self.set_status("هذا الاسم غير مسموح في ويندوز.");
            return false;
        }
        let new = match old.rfind('\\') {
            Some(i) => format!("{}\\{new_name}", &old[..i]),
            None => new_name.clone(),
        };
        // fs::rename replaces an existing file, so check first. Changing only
        // the letter case of the same file is fine.
        if std::path::Path::new(&new).exists() && new.to_lowercase() != old.to_lowercase() {
            nwg::modal_info_message(&self.window, TITLE, &format!("يوجد ملف آخر بالاسم «{new_name}» في هذا المجلد."));
            return false;
        }
        if let Err(e) = std::fs::rename(&old, &new) {
            nwg::modal_error_message(&self.window, "خطأ", &format!("تعذر تغيير الاسم:\n{e}"));
            return false;
        }
        self.update_index(|idx| {
            idx.rename_path(&old, &new);
        });
        {
            let mut st = self.state.borrow_mut();
            st.history.rename_path(&old, &new);
            st.history.save();
        }
        self.set_status(&format!("تم تغيير الاسم إلى «{new_name}»."));
        // The list is refreshed from the updated index; the edit box text is not needed.
        false
    }

    fn delete_selected(&self) {
        let Some(p) = self.selected_path() else {
            self.set_status("اختر ملفاً من القائمة أولاً.");
            return;
        };
        if self.index_busy() {
            return;
        }
        if !shell::recycle(&p) {
            self.set_status("لم يُحذف الملف.");
            return;
        }
        self.update_index(|idx| {
            idx.remove_path(&p);
        });
        {
            let mut st = self.state.borrow_mut();
            st.history.forget_path(&p);
            st.history.save();
        }
        self.set_status("نُقل الملف إلى سلة المحذوفات.");
    }

    /// Applies a change to the index, saves it in the background and shows
    /// the results again.
    fn update_index(&self, change: impl FnOnce(&mut Index)) {
        let mut st = self.state.borrow_mut();
        // Paths of the current rows, to find them again after the change.
        let row_paths: Vec<(String, Option<String>)> = st
            .rows
            .iter()
            .filter_map(|r| st.rows_index.entries.get(r.entry as usize).map(|e| (e.path.to_string(), r.snippet.clone())))
            .collect();
        st.rows_index = Arc::default();
        // A save still waiting holds the index too; it is about to be stale.
        PENDING_SAVE.lock().map(|mut p| p.take()).ok();
        let mut idx = std::mem::take(&mut st.index);
        let inner = Arc::get_mut(&mut idx);
        let mut owned = match inner {
            Some(i) => std::mem::take(i),
            None => Index { entries: idx.entries.clone(), roots: idx.roots.clone(), built_at: idx.built_at },
        };
        change(&mut owned);
        let idx = Arc::new(owned);
        st.index = idx.clone();
        st.rows_index = idx.clone();
        st.rows = row_paths
            .into_iter()
            .filter_map(|(p, snippet)| idx.find(&p).map(|i| Row { entry: i as u32, snippet }))
            .collect();
        drop(st);
        // Several quick edits only need the last state written; on_close
        // writes it at once if it is still waiting.
        if let Ok(mut p) = PENDING_SAVE.lock() {
            *p = Some(idx);
        }
        std::thread::spawn(|| {
            background_priority();
            std::thread::sleep(std::time::Duration::from_millis(500));
            flush_index_save();
        });
        self.clear_selection();
        self.refresh_list();
        if !self.content_mode() {
            self.search_names();
        }
    }

    // ---------------------------------------------------------------- history

    fn show_history_menu(&self) {
        let st = self.state.borrow();
        let mut hm = HistoryMenu::default();
        let w = &self.window;
        if nwg::Menu::builder().popup(true).parent(w).build(&mut hm.menu).is_err() {
            return;
        }
        let add = |hm: &mut HistoryMenu, text: &str, action: HistoryAction, disabled: bool| {
            let mut item = nwg::MenuItem::default();
            if nwg::MenuItem::builder().text(text).disabled(disabled).parent(&hm.menu).build(&mut item).is_ok() {
                hm.items.push((item, action));
            }
        };
        let sep = |hm: &mut HistoryMenu| {
            let mut s = nwg::MenuSeparator::default();
            if nwg::MenuSeparator::builder().parent(&hm.menu).build(&mut s).is_ok() {
                hm.separators.push(s);
            }
        };
        if !st.history.saved.is_empty() {
            add(&mut hm, "البحوث المحفوظة", HistoryAction::Nothing, true);
            for s in &st.history.saved {
                add(&mut hm, &format!("★  {}", s.name), HistoryAction::Saved(s.clone()), false);
            }
            sep(&mut hm);
        }
        if st.history.recent.is_empty() {
            add(&mut hm, "لا توجد عمليات بحث سابقة بعد", HistoryAction::Nothing, true);
        } else {
            add(&mut hm, "عمليات البحث الأخيرة", HistoryAction::Nothing, true);
            for q in st.history.recent.iter().take(15) {
                add(&mut hm, q, HistoryAction::Recent(q.clone()), false);
            }
        }
        sep(&mut hm);
        let has_query = !self.query.text().trim().is_empty();
        add(&mut hm, "حفظ البحث الحالي", HistoryAction::SaveCurrent, !has_query);
        add(&mut hm, "مسح عمليات البحث الأخيرة", HistoryAction::ClearRecent, st.history.recent.is_empty());
        add(&mut hm, "حذف كل البحوث المحفوظة", HistoryAction::ClearSaved, st.history.saved.is_empty());
        drop(st);
        *self.history_menu.borrow_mut() = hm;
        self.show_menu(&self.history_menu.borrow().menu);
    }

    /// Runs the history-menu item behind `handle`; false if it is not one.
    fn on_history_item(&self, handle: nwg::ControlHandle) -> bool {
        let action = {
            let hm = self.history_menu.borrow();
            let Some((_, a)) = hm.items.iter().find(|(i, _)| i.handle == handle) else { return false };
            match a {
                HistoryAction::Saved(s) => HistoryAction::Saved(s.clone()),
                HistoryAction::Recent(q) => HistoryAction::Recent(q.clone()),
                HistoryAction::SaveCurrent => HistoryAction::SaveCurrent,
                HistoryAction::ClearRecent => HistoryAction::ClearRecent,
                HistoryAction::ClearSaved => HistoryAction::ClearSaved,
                HistoryAction::Nothing => HistoryAction::Nothing,
            }
        };
        match action {
            HistoryAction::Saved(s) => self.apply_saved(&s),
            HistoryAction::Recent(q) => {
                self.query.set_text(&q);
                self.focus_query();
                self.on_search();
            }
            HistoryAction::SaveCurrent => {
                let text = self.query.text().trim().to_string();
                let mut st = self.state.borrow_mut();
                let saved = SavedSearch {
                    name: text.clone(),
                    query: text.clone(),
                    kind: self.kind(),
                    scope: st.scope.clone(),
                    content: self.content_mode(),
                };
                st.history.add_saved(saved);
                st.history.save();
                drop(st);
                self.set_status(&format!("حُفظ البحث «{text}». تجده في قائمة «السجل»."));
            }
            HistoryAction::ClearRecent => {
                let mut st = self.state.borrow_mut();
                st.history.recent.clear();
                st.history.save();
            }
            HistoryAction::ClearSaved => {
                let mut st = self.state.borrow_mut();
                st.history.saved.clear();
                st.history.save();
            }
            HistoryAction::Nothing => {}
        }
        true
    }

    fn apply_saved(&self, s: &SavedSearch) {
        self.kind_combo.set_selection(Some(s.kind.min(search::KINDS.len() - 1)));
        self.state.borrow_mut().settings.kind = self.kind();
        self.content_chk.set_check_state(if s.content {
            nwg::CheckBoxState::Checked
        } else {
            nwg::CheckBoxState::Unchecked
        });
        self.scope_input.set_text(if s.scope.is_empty() { "كل الأقراص" } else { &s.scope });
        self.state.borrow_mut().scope = s.scope.clone();
        self.query.set_text(&s.query);
        self.focus_query();
        self.on_search();
    }

    // ---------------------------------------------------------------- hotkey

    fn register_hotkey(&self) {
        use winapi::um::winuser::{MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey};
        let Some(h) = self.window.handle.hwnd() else { return };
        let ok = unsafe { RegisterHotKey(h, HOTKEY_ID, (MOD_CONTROL | MOD_ALT | MOD_NOREPEAT) as u32, 'F' as u32) } != 0;
        if !ok {
            self.menu_hotkey.set_enabled(false);
            log("hotkey Ctrl+Alt+F is taken by another program");
        }
    }

    /// The global hotkey shows the window, or hides it if it is already in front.
    fn on_hotkey(&self) {
        use winapi::um::winuser::GetForegroundWindow;
        let front = self.window.handle.hwnd().is_some_and(|h| unsafe { GetForegroundWindow() } == h);
        if front && self.window.visible() {
            self.hide_to_tray();
        } else {
            self.show_window();
        }
    }

    /// Saves the result list as a CSV file that Excel opens with Arabic intact.
    fn export_csv(&self) {
        if self.state.borrow().rows.is_empty() {
            self.set_status("لا توجد نتائج لتصديرها.");
            return;
        }
        let mut slot = self.export_dialog.borrow_mut();
        if slot.is_none() {
            let mut d = nwg::FileDialog::default();
            let built = nwg::FileDialog::builder()
                .title("حفظ النتائج")
                .action(nwg::FileDialogAction::Save)
                .filters("CSV(*.csv)")
                .build(&mut d);
            if built.is_err() {
                return;
            }
            *slot = Some(d);
        }
        let Some(dialog) = slot.as_ref() else { return };
        let chosen = if dialog.run(Some(&self.window)) { dialog.get_selected_item().ok() } else { None };
        drop(slot);
        let Some(path) = chosen else { return };
        let mut path = std::path::PathBuf::from(path);
        if path.extension().is_none() {
            path.set_extension("csv");
        }
        let st = self.state.borrow();
        let write = || -> std::io::Result<usize> {
            let mut f = std::io::BufWriter::new(std::fs::File::create(&path)?);
            f.write_all("\u{FEFF}الاسم,المسار,الحجم بالبايت,تاريخ التعديل,النص المطابق\r\n".as_bytes())?;
            let mut n = 0;
            for r in &st.rows {
                let Some(e) = st.rows_index.entries.get(r.entry as usize) else { continue };
                let size = if e.is_dir { String::new() } else { e.size.to_string() };
                let fields = [e.name(), &e.path, &size, &shell::format_time(e.mtime, false), r.snippet.as_deref().unwrap_or("")];
                let line: Vec<String> = fields.iter().map(|s| csv_field(s)).collect();
                f.write_all(line.join(",").as_bytes())?;
                f.write_all(b"\r\n")?;
                n += 1;
            }
            f.flush()?;
            Ok(n)
        };
        let result = write();
        drop(st);
        match result {
            Ok(n) => self.set_status(&format!("تم تصدير {} نتيجة.", fmt_num(n))),
            Err(e) => {
                nwg::modal_error_message(&self.window, "خطأ", &format!("تعذر حفظ الملف:\n{e}"));
            }
        }
    }

    fn show_menu(&self, menu: &nwg::Menu) {
        let mut pt = POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut pt) };
        menu.popup(pt.x, pt.y);
    }

    /// Text for one cell of the virtual list.
    fn cell_text(&self, row: usize, col: usize) -> String {
        let Ok(st) = self.state.try_borrow() else { return String::new() };
        let Some(r) = st.rows.get(row) else { return String::new() };
        let Some(e) = st.rows_index.entries.get(r.entry as usize) else { return String::new() };
        match col {
            COL_NAME => e.name().to_string(),
            COL_PATH => ltr_path(&e.path),
            COL_SIZE => fmt_size(e),
            COL_DATE => shell::format_time(e.mtime, true),
            COL_SNIPPET => r.snippet.clone().unwrap_or_default(),
            _ => String::new(),
        }
    }

    /// System image-list index of the small icon for a row's file type.
    fn row_icon(&self, row: usize) -> i32 {
        let key = {
            let Ok(st) = self.state.try_borrow() else { return 0 };
            let Some(e) = st.rows.get(row).and_then(|r| st.rows_index.entries.get(r.entry as usize)) else {
                return 0;
            };
            if e.is_dir { String::from("/") } else { e.ext() }
        };
        if let Some(&i) = self.icons.borrow().get(&key) {
            return i;
        }
        let i = shell::type_icon(&key);
        self.icons.borrow_mut().insert(key, i);
        i
    }

    /// Byte ranges of the query's matches in a row's file name.
    fn match_ranges(&self, row: usize) -> Vec<(usize, usize)> {
        let Ok(st) = self.state.try_borrow() else { return Vec::new() };
        let Some(q) = &st.shown_query else { return Vec::new() };
        let Some(e) = st.rows.get(row).and_then(|r| st.rows_index.entries.get(r.entry as usize)) else {
            return Vec::new();
        };
        search::match_ranges(e.name(), q)
    }

    fn custom_draw(&self, l: LPARAM) -> Option<isize> {
        crate::draw::handle_custom_draw(l, |row| self.match_ranges(row), |row| self.cell_text(row, COL_NAME))
    }

    fn set_status(&self, text: &str) {
        self.status.set_text(0, text);
    }

    // ---------------------------------------------------------------- keyboard

    fn on_key(&self, handle: nwg::ControlHandle, key: u32) {
        let ctrl = unsafe { GetKeyState(VK_CONTROL) } < 0;
        if (ctrl && key == 'F' as u32) || key == VK_F3 as u32 {
            self.focus_query();
        } else if handle == self.query.handle && key == VK_DOWN as u32 {
            // Down arrow moves from the search box into the results, or
            // opens the search history when there are none.
            if self.state.borrow().rows.is_empty() {
                if self.query.text().trim().is_empty() {
                    self.show_history_menu();
                }
                return;
            }
            if self.list.selected_item().is_none() {
                self.list.select_item(0, true);
            }
            self.list.set_focus();
        } else if handle == self.list.handle && key == VK_UP as u32 && self.list.selected_item() == Some(0) {
            self.focus_query();
        } else if handle == self.list.handle && ctrl && key == 'C' as u32 {
            if unsafe { GetKeyState(winapi::um::winuser::VK_SHIFT) } < 0 {
                self.copy_path();
            } else {
                self.copy_file();
            }
        } else if handle == self.list.handle && key == winapi::um::winuser::VK_F2 as u32 {
            self.start_rename();
        } else if handle == self.list.handle && key == winapi::um::winuser::VK_DELETE as u32 {
            self.delete_selected();
        } else if ctrl && key == 'H' as u32 {
            self.show_history_menu();
        }
    }

    fn on_enter(&self) {
        if self.renaming.get() {
            return;
        }
        let ctrl = unsafe { GetKeyState(VK_CONTROL) } < 0;
        if ctrl {
            self.open_folder();
        } else if self.list.focus() {
            self.open_selected();
        } else {
            self.remember_query();
            self.on_search();
        }
    }

    /// Esc clears the search box; a second Esc hides the window to the tray.
    fn on_escape(&self) {
        if self.renaming.get() {
            return;
        }
        if self.state.borrow().content.is_some() {
            self.on_stop();
        } else if !self.query.text().is_empty() {
            self.remember_query();
            self.query.set_text("");
            self.focus_query();
        } else {
            self.hide_to_tray();
        }
    }

    fn focus_query(&self) {
        self.query.set_focus();
        if let Some(h) = self.query.handle.hwnd() {
            unsafe { SendMessageW(h, EM_SETSEL as UINT, 0, -1) };
        }
    }

    // ---------------------------------------------------------------- tray & options

    fn hide_to_tray(&self) {
        self.remember_query();
        self.window.set_visible(false);
        if !self.tray_hint_shown.replace(true) {
            self.tray.show(
                "البرنامج ما زال يعمل هنا. انقر الأيقونة أو شغّل البرنامج مرة أخرى لإظهاره فوراً.",
                Some(TITLE),
                None,
                None,
            );
        }
    }

    fn show_window(&self) {
        self.window.set_visible(true);
        if let Some(h) = self.window.handle.hwnd() {
            unsafe {
                if IsIconic(h) != 0 {
                    ShowWindow(h, SW_RESTORE);
                }
                SetForegroundWindow(h);
            }
        }
        self.focus_query();
    }

    fn toggle_digits(&self) {
        let on = !self.menu_digits.checked();
        self.menu_digits.set_checked(on);
        ARABIC_DIGITS.store(on, Ordering::Relaxed);
        self.state.borrow_mut().settings.arabic_digits = on;
        self.list.invalidate();
    }

    fn save_settings(&self) {
        let mut st = self.state.borrow_mut();
        let s = &mut st.settings;
        if let Some(h) = self.window.handle.hwnd() {
            s.maximized = unsafe { IsZoomed(h) } != 0;
            if !s.maximized && unsafe { IsIconic(h) } == 0 && self.window.visible() {
                (s.width, s.height) = self.window.size();
            }
        }
        if let Some(h) = self.list.handle.hwnd() {
            let scale = nwg::scale_factor();
            s.columns = (0..COLUMNS.len())
                .map(|i| (unsafe { SendMessageW(h, LVM_GETCOLUMNWIDTH, i, 0) } as f64 / scale).round() as i32)
                .collect();
        }
        s.save();
    }

    fn on_close(&self) {
        flush_index_save();
        self.remember_query();
        self.save_settings();
        self.cancel_content();
        if let Some(job) = &self.state.borrow().scan {
            job.progress.cancel.store(true, Ordering::Relaxed);
        }
        self.tray.set_visibility(false);
        if let Some(h) = self.window.handle.hwnd() {
            unsafe { winapi::um::winuser::UnregisterHotKey(h, HOTKEY_ID) };
        }
        nwg::stop_thread_dispatch();
    }
}

// -------------------------------------------------------------------- formatting

/// Keeps a path reading left-to-right even when folder names are Arabic, by
/// putting left-to-right marks around every separator. Display only: never
/// pass the result to the shell or the clipboard.
fn ltr_path(path: &str) -> String {
    let mut s = String::with_capacity(path.len() + 16);
    s.push('\u{200E}');
    for c in path.chars() {
        if c == '\\' {
            s.push_str("\u{200E}\\\u{200E}");
        } else {
            s.push(c);
        }
    }
    s
}

/// Reads a NUL-terminated UTF-16 string.
///
/// # Safety
/// `p` must point to a NUL-terminated UTF-16 buffer.
unsafe fn wide_to_string(p: *const u16) -> String {
    let mut n = 0;
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, n) })
}

/// Swaps ASCII digits for Arabic-Indic ones when the user asked for them.
fn digits(s: String) -> String {
    if !ARABIC_DIGITS.load(Ordering::Relaxed) {
        return s;
    }
    s.chars()
        .map(|c| if c.is_ascii_digit() { char::from_u32(c as u32 - '0' as u32 + 0x0660).unwrap_or(c) } else { c })
        .collect()
}

fn fmt_size(e: &Entry) -> String {
    if e.is_dir {
        return "مجلد".into();
    }
    let b = e.size as f64;
    digits(if b < 1024.0 {
        format!("{} بايت", e.size)
    } else if b < 1024.0 * 1024.0 {
        format!("{:.1} ك.ب", b / 1024.0)
    } else if b < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} م.ب", b / 1024.0 / 1024.0)
    } else {
        format!("{:.2} ج.ب", b / 1024.0 / 1024.0 / 1024.0)
    })
}

fn fmt_num(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    digits(out)
}

/// Quotes a CSV cell, and stops Excel from reading a cell that starts with
/// `= + - @` as a formula by prefixing an apostrophe.
fn csv_field(s: &str) -> String {
    let s = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) { format!("'{s}") } else { s.to_string() };
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s }
}

/// "قبل 5 دقائق" with the right Arabic plural for the count.
fn minutes_ago(n: i64) -> String {
    match n {
        0 => "الآن".into(),
        1 => "قبل دقيقة".into(),
        2 => "قبل دقيقتين".into(),
        3..=10 => format!("قبل {n} دقائق"),
        _ => format!("قبل {n} دقيقة"),
    }
}

// -------------------------------------------------------------------- shell helpers

mod shell {
    use std::os::windows::process::CommandExt;
    use winapi::um::minwinbase::SYSTEMTIME;
    use winapi::um::shellapi::{
        SHFILEINFOW, SHGFI_SMALLICON, SHGFI_SYSICONINDEX, SHGFI_USEFILEATTRIBUTES, SHGetFileInfoW, ShellExecuteW,
    };
    use winapi::um::timezoneapi::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    use winapi::um::winnt::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
    use winapi::um::winuser::SW_SHOWNORMAL;

    pub fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Opens a file with its default program.
    pub fn open(path: &str) -> bool {
        let (verb, file) = (wide("open"), wide(path));
        let r = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        r as isize > 32
    }

    /// Moves a file or folder to the Recycle Bin, with Windows' own
    /// confirmation. True if it is gone.
    pub fn recycle(path: &str) -> bool {
        use winapi::um::shellapi::{FO_DELETE, FOF_ALLOWUNDO, SHFILEOPSTRUCTW, SHFileOperationW};
        // The list of paths ends with two NULs.
        let mut from: Vec<u16> = path.encode_utf16().collect();
        from.extend([0, 0]);
        let mut op: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
        op.wFunc = FO_DELETE as u32;
        op.pFrom = from.as_ptr();
        op.fFlags = FOF_ALLOWUNDO;
        let r = unsafe { SHFileOperationW(&mut op) };
        r == 0 && op.fAnyOperationsAborted == 0 && !std::path::Path::new(path).exists()
    }

    /// Opens Explorer with the file selected.
    pub fn reveal(path: &str) {
        let _ = std::process::Command::new("explorer.exe").raw_arg(format!("/select,\"{path}\"")).spawn();
    }

    fn query_icon(name: &str, attrs: u32, flags: u32) -> (usize, i32) {
        let name = wide(name);
        let mut info: SHFILEINFOW = unsafe { std::mem::zeroed() };
        let r = unsafe {
            SHGetFileInfoW(name.as_ptr(), attrs, &mut info, std::mem::size_of::<SHFILEINFOW>() as u32, flags)
        };
        (r, info.iIcon)
    }

    /// The system's small-icon image list, shared by every program.
    pub fn system_image_list() -> usize {
        query_icon(".txt", FILE_ATTRIBUTE_NORMAL, SHGFI_SYSICONINDEX | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES).0
    }

    /// Icon index for an extension, or for folders when `ext` is "/".
    pub fn type_icon(ext: &str) -> i32 {
        let flags = SHGFI_SYSICONINDEX | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES;
        if ext == "/" {
            query_icon("folder", FILE_ATTRIBUTE_DIRECTORY, flags).1
        } else {
            query_icon(&format!("file.{ext}"), FILE_ATTRIBUTE_NORMAL, flags).1
        }
    }

    fn local_time(secs: i64) -> Option<SYSTEMTIME> {
        // FILETIME counts 100 ns intervals since 1601-01-01.
        let ft = (secs as u64 + 11_644_473_600) * 10_000_000;
        let ft = winapi::shared::minwindef::FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
        unsafe {
            let mut utc: SYSTEMTIME = std::mem::zeroed();
            let mut local: SYSTEMTIME = std::mem::zeroed();
            if FileTimeToSystemTime(&ft, &mut utc) == 0
                || SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) == 0
            {
                return None;
            }
            Some(local)
        }
    }

    fn same_day(a: &SYSTEMTIME, b: &SYSTEMTIME) -> bool {
        (a.wYear, a.wMonth, a.wDay) == (b.wYear, b.wMonth, b.wDay)
    }

    /// Formats Unix seconds as local time: `YYYY/MM/DD HH:MM`, or with
    /// `relative` "قبل 5 دقائق", "اليوم 14:30" and "أمس 09:12" for recent files.
    pub fn format_time(secs: i64, relative: bool) -> String {
        if secs <= 0 {
            return String::new();
        }
        let Some(t) = local_time(secs) else { return String::new() };
        let hm = format!("{:02}:{:02}", t.wHour, t.wMinute);
        if relative {
            let now = crate::index::now_secs();
            let age = now - secs;
            if (0..3600).contains(&age) {
                return super::digits(super::minutes_ago(age / 60));
            }
            if let Some(today) = local_time(now) {
                if same_day(&t, &today) {
                    return super::digits(format!("اليوم {hm}"));
                }
                if local_time(now - 86_400).is_some_and(|y| same_day(&t, &y)) {
                    return super::digits(format!("أمس {hm}"));
                }
            }
        }
        let s = format!("{:04}/{:02}/{:02} {hm}", t.wYear, t.wMonth, t.wDay);
        if relative { super::digits(s) } else { s }
    }
}

// -------------------------------------------------------------------- building

pub struct AppUi {
    inner: Rc<App>,
    handlers: RefCell<Vec<nwg::EventHandler>>,
    raw_handlers: RefCell<Vec<nwg::RawEventHandler>>,
}

impl Deref for AppUi {
    type Target = App;
    fn deref(&self) -> &App {
        &self.inner
    }
}

impl Drop for AppUi {
    fn drop(&mut self) {
        for h in self.handlers.borrow().iter() {
            nwg::unbind_event_handler(h);
        }
        for h in self.raw_handlers.borrow().iter() {
            let _ = nwg::unbind_raw_event_handler(h);
        }
    }
}

impl NativeUi<AppUi> for App {
    fn build_ui(mut app: App) -> Result<AppUi, nwg::NwgError> {
        let mut settings = Settings::load();
        // A size saved on a bigger screen must still fit this one.
        let (max_w, max_h) = work_area();
        settings.width = settings.width.min(max_w);
        settings.height = settings.height.min(max_h);
        ARABIC_DIGITS.store(settings.arabic_digits, Ordering::Relaxed);

        let family = ui_font_family();
        phase("settings");
        let scale = nwg::scale_factor();
        nwg::Font::builder().family(family).size_absolute((15.0 * scale).round() as u32).build(&mut app.font)?;
        nwg::Font::builder()
            .family(family)
            .size_absolute((19.0 * scale).round() as u32)
            .build(&mut app.query_font)?;
        let f = Some(&app.font);

        nwg::Window::builder()
            .title(TITLE)
            .size((settings.width as i32, settings.height as i32))
            .center(true)
            .ex_flags(WS_EX_LAYOUTRTL)
            .flags(if settings.maximized {
                nwg::WindowFlags::MAIN_WINDOW | nwg::WindowFlags::VISIBLE | nwg::WindowFlags::MAXIMIZED
            } else {
                nwg::WindowFlags::MAIN_WINDOW | nwg::WindowFlags::VISIBLE
            })
            .build(&mut app.window)?;
        let w = &app.window;
        if let Some(h) = w.handle.hwnd() {
            crate::icon::set_window_icon(h);
        }
        phase("window");

        nwg::TextInput::builder()
            .parent(w)
            .font(Some(&app.query_font))
            .placeholder_text(Some("اكتب اسم الملف أو كلمة للبحث…   أمثلة:  تقرير ‎.pdf   ·   عقد !قديم   ·   صورة|فيديو"))
            .focus(true)
            .build(&mut app.query)?;
        nwg::Button::builder().parent(w).font(f).text("بحث").build(&mut app.search_btn)?;
        nwg::Button::builder().parent(w).font(f).text("السجل").build(&mut app.history_btn)?;
        let kinds: Vec<&'static str> = search::KINDS.iter().map(|(label, _)| *label).collect();
        let kind = settings.kind.min(kinds.len().saturating_sub(1));
        nwg::ComboBox::builder()
            .parent(w)
            .font(f)
            .collection(kinds)
            .selected_index(Some(kind))
            .build(&mut app.kind_combo)?;
        nwg::CheckBox::builder()
            .parent(w)
            .font(f)
            .text("البحث داخل المستندات أيضاً")
            .build(&mut app.content_chk)?;
        nwg::Label::builder().parent(w).font(f).text("النطاق:").build(&mut app.scope_label)?;
        nwg::TextInput::builder()
            .parent(w)
            .font(f)
            .readonly(true)
            .text("كل الأقراص")
            .build(&mut app.scope_input)?;
        nwg::Button::builder().parent(w).font(f).text("اختيار مجلد…").build(&mut app.scope_btn)?;
        nwg::Button::builder().parent(w).font(f).text("كل الأقراص").build(&mut app.all_btn)?;
        nwg::Button::builder().parent(w).font(f).text("تحديث الفهرس").build(&mut app.reindex_btn)?;

        // Owner-data (virtual) list: rows are drawn on demand, so even a
        // million results display instantly.
        let list_flags = unsafe {
            nwg::ListViewFlags::from_bits_unchecked(
                nwg::ListViewFlags::VISIBLE.bits()
                    | nwg::ListViewFlags::TAB_STOP.bits()
                    | nwg::ListViewFlags::SINGLE_SELECTION.bits()
                    | nwg::ListViewFlags::ALWAYS_SHOW_SELECTION.bits()
                    | LVS_OWNERDATA
                    | winapi::um::commctrl::LVS_EDITLABELS
                    | LVS_SHAREIMAGELISTS,
            )
        };
        nwg::ListView::builder()
            .parent(w)
            .flags(list_flags)
            .list_style(nwg::ListViewStyle::Detailed)
            // Native double buffering (LVS_EX_DOUBLEBUFFER). nwg's own
            // `double_buffer` paints through an unmirrored memory DC, which
            // draws text backwards in a right-to-left window.
            .ex_flags(unsafe {
                nwg::ListViewExFlags::from_bits_unchecked(
                    nwg::ListViewExFlags::FULL_ROW_SELECT.bits() | LVS_EX_DOUBLEBUFFER,
                )
            })
            .build(&mut app.list)?;
        if let Some(h) = app.list.handle.hwnd() {
            unsafe {
                SendMessageW(h, WM_SETFONT, app.font.handle as WPARAM, 1);
                // Explorer-style rows: soft hover and selection colours.
                let theme = shell::wide("Explorer");
                winapi::um::uxtheme::SetWindowTheme(h, theme.as_ptr(), std::ptr::null());
                // File-type icons from the shared system image list.
                SendMessageW(h, LVM_SETIMAGELIST, LVSIL_SMALL as WPARAM, shell::system_image_list() as LPARAM);
            }
        }
        phase("list");
        for (i, (title, width)) in COLUMNS.iter().enumerate() {
            let width = settings.columns.get(i).copied().filter(|w| (20..3000).contains(w)).unwrap_or(*width);
            app.list.insert_column(nwg::InsertListViewColumn {
                index: Some(i as i32),
                fmt: None,
                width: Some((width as f64 * scale).round() as i32),
                text: Some(title.to_string()),
            });
        }
        app.list.set_headers_enabled(true);

        nwg::Button::builder().parent(w).font(f).text("فتح الملف").build(&mut app.open_btn)?;
        nwg::Button::builder().parent(w).font(f).text("فتح المجلد").build(&mut app.folder_btn)?;
        nwg::Button::builder().parent(w).font(f).text("نسخ المسار").build(&mut app.copy_btn)?;
        nwg::Button::builder().parent(w).font(f).text("تصدير النتائج").build(&mut app.export_btn)?;
        nwg::Button::builder().parent(w).font(f).text("إيقاف البحث").enabled(false).build(&mut app.stop_btn)?;
        nwg::Button::builder().parent(w).font(f).text("خيارات…").build(&mut app.options_btn)?;
        nwg::StatusBar::builder().parent(w).font(f).text("").build(&mut app.status)?;

        nwg::AnimationTimer::builder()
            .parent(w)
            .interval(Duration::from_millis(200))
            .active(true)
            .build(&mut app.timer)?;
        nwg::Notice::builder().parent(w).build(&mut app.name_notice)?;
        nwg::Notice::builder().parent(w).build(&mut app.update_notice)?;
        phase("controls");
        nwg::Menu::builder().popup(true).parent(w).build(&mut app.menu)?;
        nwg::MenuItem::builder().text("فتح الملف\tEnter").parent(&app.menu).build(&mut app.menu_open)?;
        nwg::MenuItem::builder().text("فتح المجلد الذي يحتويه\tCtrl+Enter").parent(&app.menu).build(&mut app.menu_folder)?;
        nwg::MenuItem::builder().text("نسخ الملف\tCtrl+C").parent(&app.menu).build(&mut app.menu_copy_file)?;
        nwg::MenuItem::builder().text("نسخ المسار\tCtrl+Shift+C").parent(&app.menu).build(&mut app.menu_copy)?;
        nwg::MenuItem::builder().text("نسخ مسارات كل النتائج").parent(&app.menu).build(&mut app.menu_copy_all)?;
        nwg::MenuItem::builder().text("تغيير الاسم\tF2").parent(&app.menu).build(&mut app.menu_rename)?;
        nwg::MenuItem::builder().text("نقل إلى سلة المحذوفات\tDelete").parent(&app.menu).build(&mut app.menu_delete)?;

        nwg::Menu::builder().popup(true).parent(w).build(&mut app.options_menu)?;
        nwg::MenuItem::builder()
            .text("عرض الأرقام بالعربية (٠١٢٣)")
            .check(settings.arabic_digits)
            .parent(&app.options_menu)
            .build(&mut app.menu_digits)?;
        nwg::MenuItem::builder().text("تصدير النتائج إلى Excel (CSV)").parent(&app.options_menu).build(&mut app.menu_export)?;
        nwg::MenuItem::builder().text("إخفاء إلى شريط المهام\tEsc").parent(&app.options_menu).build(&mut app.menu_hide)?;
        nwg::MenuItem::builder()
            .text(&format!("إظهار البرنامج من أي مكان\t{HOTKEY_TEXT}"))
            .parent(&app.options_menu)
            .build(&mut app.menu_hotkey)?;
        nwg::MenuItem::builder()
            .text(&format!("التحقق من التحديثات (الإصدار {})", env!("CARGO_PKG_VERSION")))
            .parent(&app.options_menu)
            .build(&mut app.menu_update)?;

        // Tray icon, so a hidden window comes back instantly.
        app.tray_icon.handle = crate::icon::load_icon(true) as _;
        if app.tray_icon.handle.is_null() {
            nwg::Icon::builder().source_system(Some(nwg::OemIcon::Information)).build(&mut app.tray_icon)?;
        }
        nwg::TrayNotification::builder()
            .parent(w)
            .icon(Some(&app.tray_icon))
            .tip(Some(&format!("{TITLE} ({HOTKEY_TEXT})")))
            .build(&mut app.tray)?;
        nwg::Menu::builder().popup(true).parent(w).build(&mut app.tray_menu)?;
        nwg::MenuItem::builder().text("إظهار بحث سريع").parent(&app.tray_menu).build(&mut app.tray_show)?;
        nwg::MenuItem::builder().text("خروج").parent(&app.tray_menu).build(&mut app.tray_exit)?;

        phase("tray");
        app.state.get_mut().settings = settings.clone();
        app.state.get_mut().history = History::load();
        let ui = AppUi { inner: Rc::new(app), handlers: Default::default(), raw_handlers: Default::default() };

        // Regular events.
        let weak = Rc::downgrade(&ui.inner);
        let handler = nwg::full_bind_event_handler(&ui.window.handle, move |evt, data, handle| {
            use nwg::Event as E;
            let Some(app) = weak.upgrade() else { return };
            match evt {
                // Only the main window's own size matters: child controls
                // report OnResize too, and re-laying out on each of those
                // nests layouts inside layouts.
                E::OnResize | E::OnWindowMaximize if handle == app.window => app.layout(),
                E::OnWindowClose if handle == app.window => app.on_close(),
                E::OnTextInput if handle == app.query => app.on_query_changed(),
                // The dialog-style message loop turns Enter into IDOK and Esc
                // into IDCANCEL, which nwg reports as OnKeyEnter / OnKeyEsc.
                E::OnKeyEnter => app.on_enter(),
                E::OnKeyEsc => app.on_escape(),
                E::OnKeyPress => app.on_key(handle, data.on_key()),
                E::OnTimerTick if handle == app.timer => app.on_tick(),
                E::OnNotice if handle == app.name_notice => app.on_names_found(),
                E::OnNotice if handle == app.update_notice => app.on_update_event(),
                E::OnComboxBoxSelection if handle == app.kind_combo => app.on_kind_changed(),
                E::OnButtonClick => {
                    if handle == app.search_btn {
                        app.on_search();
                    } else if handle == app.history_btn {
                        app.show_history_menu();
                    } else if handle == app.content_chk {
                        app.on_query_changed();
                    } else if handle == app.scope_btn {
                        app.choose_folder();
                    } else if handle == app.all_btn {
                        app.set_scope(String::new());
                    } else if handle == app.reindex_btn {
                        app.start_scan();
                    } else if handle == app.open_btn {
                        app.open_selected();
                    } else if handle == app.folder_btn {
                        app.open_folder();
                    } else if handle == app.copy_btn {
                        app.copy_path();
                    } else if handle == app.export_btn {
                        app.export_csv();
                    } else if handle == app.stop_btn {
                        app.on_stop();
                    } else if handle == app.options_btn {
                        app.show_menu(&app.options_menu);
                    }
                }
                E::OnListViewItemActivated if handle == app.list => app.open_selected(),
                E::OnListViewItemChanged if handle == app.list => app.show_selected_path(),
                E::OnListViewRightClick if handle == app.list => app.show_menu(&app.menu),
                E::OnListViewColumnClick if handle == app.list => app.on_column_click(data.on_list_view_item_index().1),
                E::OnMousePress(nwg::MousePressEvent::MousePressLeftUp) if handle == app.tray => app.show_window(),
                E::OnContextMenu if handle == app.tray => app.show_menu(&app.tray_menu),
                E::OnMenuItemSelected => {
                    if app.on_history_item(handle) {
                    } else if handle == app.menu_open {
                        app.open_selected();
                    } else if handle == app.menu_folder {
                        app.open_folder();
                    } else if handle == app.menu_copy {
                        app.copy_path();
                    } else if handle == app.menu_copy_all {
                        app.copy_all_paths();
                    } else if handle == app.menu_copy_file {
                        app.copy_file();
                    } else if handle == app.menu_rename {
                        app.start_rename();
                    } else if handle == app.menu_delete {
                        app.delete_selected();
                    } else if handle == app.menu_hotkey {
                        app.set_status(&format!("اضغط {HOTKEY_TEXT} من أي برنامج لإظهار بحث سريع أو إخفائه."));
                    } else if handle == app.menu_update {
                        app.check_for_update(true);
                    } else if handle == app.menu_digits {
                        app.toggle_digits();
                    } else if handle == app.menu_export {
                        app.export_csv();
                    } else if handle == app.menu_hide {
                        app.hide_to_tray();
                    } else if handle == app.tray_show {
                        app.show_window();
                    } else if handle == app.tray_exit {
                        app.window.close();
                    }
                }
                _ => {}
            }
        });
        ui.handlers.borrow_mut().push(handler);

        // Virtual list: answer LVN_GETDISPINFO with the text and icon of each cell.
        let weak = Rc::downgrade(&ui.inner);
        let list_hwnd = ui.list.handle.hwnd().unwrap_or(std::ptr::null_mut());
        let raw = nwg::bind_raw_event_handler(&ui.window.handle, 0x1_0001, move |_hwnd: HWND, msg: UINT, w: WPARAM, l: LPARAM| {
            if msg == winapi::um::winuser::WM_HOTKEY && w as i32 == HOTKEY_ID {
                weak.upgrade()?.on_hotkey();
                return Some(0);
            }
            if msg != WM_NOTIFY {
                return None;
            }
            let hdr = unsafe { &*(l as *const NMHDR) };
            if hdr.hwndFrom != list_hwnd {
                return None;
            }
            let app = weak.upgrade()?;
            use winapi::um::commctrl::{LVN_BEGINDRAG, LVN_BEGINLABELEDITW, LVN_ENDLABELEDITW, NM_CUSTOMDRAW, NMLISTVIEW};
            match hdr.code {
                LVN_GETDISPINFOW => {}
                LVN_BEGINLABELEDITW => {
                    let info = unsafe { &*(l as *const NMLVDISPINFOW) };
                    return Some(app.begin_rename(info.item.iItem as usize) as isize);
                }
                LVN_ENDLABELEDITW => {
                    let info = unsafe { &*(l as *const NMLVDISPINFOW) };
                    let text = (!info.item.pszText.is_null()).then(|| unsafe { wide_to_string(info.item.pszText) });
                    return Some(app.finish_rename(text) as isize);
                }
                LVN_BEGINDRAG => {
                    let nm = unsafe { &*(l as *const NMLISTVIEW) };
                    app.drag_row(nm.iItem as usize);
                    return Some(0);
                }
                NM_CUSTOMDRAW => return app.custom_draw(l),
                _ => return None,
            }
            let info = unsafe { &mut *(l as *mut NMLVDISPINFOW) };
            let item = &mut info.item;
            if item.mask & LVIF_TEXT != 0 && !item.pszText.is_null() && item.cchTextMax > 0 {
                let text = app.cell_text(item.iItem as usize, item.iSubItem as usize);
                let max = item.cchTextMax as usize - 1;
                let buf = unsafe { std::slice::from_raw_parts_mut(item.pszText, max + 1) };
                let mut n = 0;
                for u in text.encode_utf16().take(max) {
                    buf[n] = u;
                    n += 1;
                }
                buf[n] = 0;
            }
            if item.mask & LVIF_IMAGE != 0 && item.iSubItem == 0 {
                item.iImage = app.row_icon(item.iItem as usize);
            }
            Some(0)
        })?;
        ui.raw_handlers.borrow_mut().push(raw);

        ui.register_hotkey();
        ui.show_sort_arrow();
        phase("events");
        ui.layout();
        phase("layout");
        ui.start();
        Ok(ui)
    }
}

/// The first installed font from a list of fonts known to render Arabic well.
/// Each name is looked up directly: listing every font on the system takes
/// seconds on a computer with many fonts installed.
fn ui_font_family() -> &'static str {
    static FAMILY: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    const PREFERRED: [&str; 5] = ["Segoe UI", "Tahoma", "Arial", "Noto Sans Arabic", "Noto Naskh Arabic"];
    FAMILY.get_or_init(|| PREFERRED.iter().copied().find(|f| font_installed(f)).unwrap_or(PREFERRED[0]))
}

fn font_installed(name: &str) -> bool {
    use winapi::shared::minwindef::{DWORD, LPARAM};
    use winapi::um::wingdi::{DEFAULT_CHARSET, EnumFontFamiliesExW, LOGFONTW, TEXTMETRICW};
    use winapi::um::winuser::{GetDC, ReleaseDC};

    unsafe extern "system" fn found(_: *const LOGFONTW, _: *const TEXTMETRICW, _: DWORD, flag: LPARAM) -> i32 {
        unsafe { *(flag as *mut bool) = true };
        0 // one match is enough
    }
    let mut lf: LOGFONTW = unsafe { std::mem::zeroed() };
    lf.lfCharSet = DEFAULT_CHARSET as u8;
    // The last unit stays 0 as the terminator.
    for (dst, src) in lf.lfFaceName.iter_mut().take(31).zip(shell::wide(name)) {
        *dst = src;
    }
    let mut ok = false;
    unsafe {
        let hdc = GetDC(std::ptr::null_mut());
        EnumFontFamiliesExW(hdc, &mut lf, Some(found), &mut ok as *mut bool as LPARAM, 0);
        ReleaseDC(std::ptr::null_mut(), hdc);
    }
    ok
}

thread_local! {
    /// Start-up phase timings, written to the log after the first paint.
    static PHASES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Milliseconds since Windows created this process, so the timings include
/// loading the program before `main`.
fn ms_since_process_start() -> u64 {
    use winapi::shared::minwindef::FILETIME;
    use winapi::um::processthreadsapi::{GetCurrentProcess, GetProcessTimes};
    use winapi::um::sysinfoapi::GetSystemTimeAsFileTime;
    let as_u64 = |f: &FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    unsafe {
        let zero = || std::mem::zeroed::<FILETIME>();
        let (mut created, mut exited, mut kernel, mut user, mut now) = (zero(), zero(), zero(), zero(), zero());
        if GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut kernel, &mut user) == 0 {
            return 0;
        }
        GetSystemTimeAsFileTime(&mut now);
        as_u64(&now).saturating_sub(as_u64(&created)) / 10_000
    }
}

/// Records how long start-up took to reach `name`.
fn phase(name: &str) {
    let ms = ms_since_process_start();
    PHASES.with(|p| p.borrow_mut().push(format!("{name}={ms}")));
}

/// Writes the recorded phases to the log, off the UI thread.
fn log_phases() {
    let marks = PHASES.with(|p| std::mem::take(&mut *p.borrow_mut()).join(" "));
    if !marks.is_empty() {
        std::thread::spawn(move || log(&format!("startup ms (v{}): {marks}", env!("CARGO_PKG_VERSION"))));
    }
}

/// The usable screen size (without the taskbar) in logical pixels.
fn work_area() -> (u32, u32) {
    use winapi::um::winuser::{SPI_GETWORKAREA, SystemParametersInfoW};
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    let ok = unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut r as *mut RECT as _, 0) } != 0;
    if !ok || r.right <= r.left {
        return (u32::MAX, u32::MAX);
    }
    let scale = nwg::scale_factor();
    // Leave room for the window frame and title bar.
    let w = ((r.right - r.left) as f64 / scale) as u32;
    let h = ((r.bottom - r.top) as f64 / scale) as u32;
    (w.saturating_sub(16).max(400), h.saturating_sub(40).max(300))
}

/// Lets the user interface win when a background thread competes for the CPU.
fn background_priority() {
    use winapi::um::processthreadsapi::{GetCurrentThread, SetThreadPriority};
    use winapi::um::winbase::THREAD_PRIORITY_BELOW_NORMAL;
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL as i32) };
}

/// Appends a timestamped line to `%LOCALAPPDATA%\BahthSari\log.txt`, kept short.
fn log(line: &str) {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock();
    let path = index::cache_path().with_file_name("log.txt");
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 64 * 1024) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{} {line}", index::now_secs());
    }
}

/// If the program is already running, brings its window forward and returns true.
fn activate_running_copy() -> bool {
    use winapi::shared::winerror::ERROR_ALREADY_EXISTS;
    use winapi::um::errhandlingapi::GetLastError;
    use winapi::um::synchapi::CreateMutexW;
    use winapi::um::winuser::{FindWindowW, SW_SHOW};

    let name = shell::wide(crate::update::SINGLE_INSTANCE_MUTEX);
    // The mutex lives as long as this process; Windows frees it on exit.
    let mutex = unsafe { CreateMutexW(std::ptr::null_mut(), 0, name.as_ptr()) };
    if mutex.is_null() || unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
        return false;
    }
    // The first copy may still be creating its window; give it a few seconds.
    let (class, title) = (shell::wide("NativeWindowsGuiWindow"), shell::wide(TITLE));
    let mut hwnd = std::ptr::null_mut();
    for _ in 0..50 {
        hwnd = unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) };
        if !hwnd.is_null() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if hwnd.is_null() {
        return true;
    }
    unsafe {
        ShowWindow(hwnd, if IsIconic(hwnd) != 0 { SW_RESTORE } else { SW_SHOW });
        SetForegroundWindow(hwnd);
    }
    true
}

/// The edited index waiting to be written to disk (see `update_index`).
static PENDING_SAVE: Mutex<Option<Arc<Index>>> = Mutex::new(None);

fn flush_index_save() {
    let idx = PENDING_SAVE.lock().ok().and_then(|mut p| p.take());
    if let Some(idx) = idx {
        let _ = idx.save(&index::cache_path());
    }
}

pub fn run() {
    if std::env::args().any(|a| a == "--updated") {
        // The previous version is still closing; let it go first. A big
        // index can take a while to free, so wait up to a minute.
        for _ in 0..6 {
            crate::update::wait_for_previous_instance_exit();
        }
    }
    if activate_running_copy() {
        return;
    }
    // Only the copy that keeps running tidies up after an update.
    crate::update::cleanup_old();
    phase("single-instance");
    unsafe {
        winapi::um::objbase::CoInitialize(std::ptr::null_mut());
    }
    if let Err(e) = nwg::init() {
        eprintln!("failed to initialise the UI: {e}");
        return;
    }
    phase("init");
    let _ = nwg::Font::set_global_family(ui_font_family());
    phase("font");
    let ui = match App::build_ui(App::default()) {
        Ok(ui) => {
            phase("built");
            ui
        }
        Err(e) => {
            nwg::fatal_message(TITLE, &format!("تعذر إنشاء الواجهة: {e}"));
        }
    };
    phase("loop");
    LOOP_STARTED.store(true, Ordering::Relaxed);
    nwg::dispatch_thread_events();
    drop(ui);
}
