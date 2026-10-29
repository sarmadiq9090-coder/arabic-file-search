//! The Windows user interface, built on native Win32 controls so Arabic text
//! is shaped, joined and laid out right-to-left by Windows itself.

use native_windows_gui as nwg;
use nwg::NativeUi;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::Write;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use winapi::shared::minwindef::{LPARAM, UINT, WPARAM};
use winapi::shared::windef::{HWND, POINT, RECT};
use winapi::um::commctrl::{
    LVIF_IMAGE, LVIF_TEXT, LVM_GETCOLUMNWIDTH, LVM_SETIMAGELIST, LVN_GETDISPINFOW, LVS_EX_DOUBLEBUFFER,
    LVS_OWNERDATA, LVS_SHAREIMAGELISTS, LVSIL_SMALL, NMLVDISPINFOW, SB_SETPARTS,
};
use winapi::um::winuser::{
    EM_SETSEL, GetCursorPos, GetKeyState, GetWindowRect, IsIconic, IsZoomed, NMHDR, SW_MAXIMIZE, SW_RESTORE,
    SendMessageW, SetForegroundWindow, ShowWindow, VK_CONTROL, VK_DOWN, VK_F3, VK_UP, WM_NOTIFY, WM_SETFONT,
};

use crate::index::{self, Entry, Index, ScanProgress};
use crate::layout;
use crate::search::{self, ContentHit, ContentProgress, Query};
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
    settings: Settings,
}

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
    folder_dialog: nwg::FileDialog,
    export_dialog: nwg::FileDialog,
    menu: nwg::Menu,
    menu_open: nwg::MenuItem,
    menu_folder: nwg::MenuItem,
    menu_copy: nwg::MenuItem,
    menu_copy_all: nwg::MenuItem,
    options_menu: nwg::Menu,
    menu_digits: nwg::MenuItem,
    menu_export: nwg::MenuItem,
    menu_hide: nwg::MenuItem,
    tray_icon: nwg::Icon,
    tray: nwg::TrayNotification,
    tray_menu: nwg::Menu,
    tray_show: nwg::MenuItem,
    tray_exit: nwg::MenuItem,
    /// System image-list index of the icon for each extension.
    icons: RefCell<HashMap<String, i32>>,
    tray_hint_shown: Cell<bool>,
    state: RefCell<State>,
}

impl App {
    /// Height of the status bar in logical units (it sizes itself to its font).
    fn status_height(&self) -> i32 {
        let Some(hwnd) = self.status.handle.hwnd() else { return 26 };
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe { GetWindowRect(hwnd, &mut r) };
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

        // Row 1: a large query box + search button.
        self.query.set_position(m, m);
        self.query.set_size((w - 3 * m - btn_w).max(100) as u32, query_h as u32);
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
        let mut st = self.state.borrow_mut();
        if st.scan.is_some() || st.loading.is_some() {
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
        if st.name_pending {
            // The query changed meanwhile: these results are already stale.
            drop(st);
            self.search_names();
            return;
        }
        if st.content.is_some() || self.content_mode() {
            return;
        }
        st.rows = res.hits.into_iter().map(|entry| Row { entry, snippet: None }).collect();
        st.rows_index = res.index;
        let n = st.rows.len();
        let busy = st.scan.is_some() || st.loading.is_some();
        drop(st);
        self.refresh_list();
        if res.empty {
            if !busy {
                self.set_status("اكتب اسم الملف أو جزءاً منه للبحث.");
            }
            return;
        }
        let tail = if busy { " (الفهرسة ما زالت جارية)" } else { "" };
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
        self.poll_load();
        self.poll_scan();
        self.poll_content();
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
        if changed {
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
        if self.folder_dialog.run(Some(&self.window))
            && let Ok(dir) = self.folder_dialog.get_selected_item()
        {
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

    fn open_selected(&self) {
        match self.selected_path() {
            Some(p) => {
                if !shell::open(&p) {
                    nwg::modal_error_message(&self.window, "خطأ", &format!("تعذر فتح الملف:\n{p}"));
                }
            }
            None => self.set_status("اختر ملفاً من القائمة أولاً."),
        }
    }

    fn open_folder(&self) {
        match self.selected_path() {
            Some(p) => shell::reveal(&p),
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

    /// Saves the result list as a CSV file that Excel opens with Arabic intact.
    fn export_csv(&self) {
        if self.state.borrow().rows.is_empty() {
            self.set_status("لا توجد نتائج لتصديرها.");
            return;
        }
        if !self.export_dialog.run(Some(&self.window)) {
            return;
        }
        let Ok(path) = self.export_dialog.get_selected_item() else { return };
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

    fn set_status(&self, text: &str) {
        self.status.set_text(0, text);
    }

    // ---------------------------------------------------------------- keyboard

    fn on_key(&self, handle: nwg::ControlHandle, key: u32) {
        let ctrl = unsafe { GetKeyState(VK_CONTROL) } < 0;
        if (ctrl && key == 'F' as u32) || key == VK_F3 as u32 {
            self.focus_query();
        } else if handle == self.query.handle && key == VK_DOWN as u32 {
            // Down arrow moves from the search box into the results.
            if self.state.borrow().rows.is_empty() {
                return;
            }
            if self.list.selected_item().is_none() {
                self.list.select_item(0, true);
            }
            self.list.set_focus();
        } else if handle == self.list.handle && key == VK_UP as u32 && self.list.selected_item() == Some(0) {
            self.focus_query();
        } else if handle == self.list.handle && ctrl && key == 'C' as u32 {
            self.copy_path();
        }
    }

    fn on_enter(&self) {
        let ctrl = unsafe { GetKeyState(VK_CONTROL) } < 0;
        if ctrl {
            self.open_folder();
        } else if self.list.focus() {
            self.open_selected();
        } else {
            self.on_search();
        }
    }

    /// Esc clears the search box; a second Esc hides the window to the tray.
    fn on_escape(&self) {
        if self.state.borrow().content.is_some() {
            self.on_stop();
        } else if !self.query.text().is_empty() {
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
        self.save_settings();
        self.cancel_content();
        if let Some(job) = &self.state.borrow().scan {
            job.progress.cancel.store(true, Ordering::Relaxed);
        }
        self.tray.set_visibility(false);
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
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    digits(out)
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
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
        let settings = Settings::load();
        ARABIC_DIGITS.store(settings.arabic_digits, Ordering::Relaxed);

        let family = ui_font_family();
        let scale = nwg::scale_factor();
        nwg::Font::builder().family(&family).size_absolute((15.0 * scale).round() as u32).build(&mut app.font)?;
        nwg::Font::builder()
            .family(&family)
            .size_absolute((19.0 * scale).round() as u32)
            .build(&mut app.query_font)?;
        let f = Some(&app.font);

        nwg::Window::builder()
            .title(TITLE)
            .size((settings.width as i32, settings.height as i32))
            .center(true)
            .ex_flags(WS_EX_LAYOUTRTL)
            .flags(nwg::WindowFlags::MAIN_WINDOW | nwg::WindowFlags::VISIBLE)
            .build(&mut app.window)?;
        let w = &app.window;
        if let Some(h) = w.handle.hwnd() {
            crate::icon::set_window_icon(h);
        }

        nwg::TextInput::builder()
            .parent(w)
            .font(Some(&app.query_font))
            .placeholder_text(Some("اكتب اسم الملف أو كلمة للبحث…   أمثلة:  تقرير ‎.pdf   ·   عقد !قديم   ·   صورة|فيديو"))
            .focus(true)
            .build(&mut app.query)?;
        nwg::Button::builder().parent(w).font(f).text("بحث").build(&mut app.search_btn)?;
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
        nwg::FileDialog::builder()
            .title("اختر المجلد الذي تريد البحث فيه")
            .action(nwg::FileDialogAction::OpenDirectory)
            .build(&mut app.folder_dialog)?;
        nwg::FileDialog::builder()
            .title("حفظ النتائج")
            .action(nwg::FileDialogAction::Save)
            .filters("CSV(*.csv)")
            .build(&mut app.export_dialog)?;

        nwg::Menu::builder().popup(true).parent(w).build(&mut app.menu)?;
        nwg::MenuItem::builder().text("فتح الملف\tEnter").parent(&app.menu).build(&mut app.menu_open)?;
        nwg::MenuItem::builder().text("فتح المجلد الذي يحتويه\tCtrl+Enter").parent(&app.menu).build(&mut app.menu_folder)?;
        nwg::MenuItem::builder().text("نسخ المسار\tCtrl+C").parent(&app.menu).build(&mut app.menu_copy)?;
        nwg::MenuItem::builder().text("نسخ مسارات كل النتائج").parent(&app.menu).build(&mut app.menu_copy_all)?;

        nwg::Menu::builder().popup(true).parent(w).build(&mut app.options_menu)?;
        nwg::MenuItem::builder()
            .text("عرض الأرقام بالعربية (٠١٢٣)")
            .check(settings.arabic_digits)
            .parent(&app.options_menu)
            .build(&mut app.menu_digits)?;
        nwg::MenuItem::builder().text("تصدير النتائج إلى Excel (CSV)").parent(&app.options_menu).build(&mut app.menu_export)?;
        nwg::MenuItem::builder().text("إخفاء إلى شريط المهام\tEsc").parent(&app.options_menu).build(&mut app.menu_hide)?;

        // Tray icon, so a hidden window comes back instantly.
        app.tray_icon.handle = crate::icon::load_icon(true) as _;
        if app.tray_icon.handle.is_null() {
            nwg::Icon::builder().source_system(Some(nwg::OemIcon::Information)).build(&mut app.tray_icon)?;
        }
        nwg::TrayNotification::builder()
            .parent(w)
            .icon(Some(&app.tray_icon))
            .tip(Some(TITLE))
            .build(&mut app.tray)?;
        nwg::Menu::builder().popup(true).parent(w).build(&mut app.tray_menu)?;
        nwg::MenuItem::builder().text("إظهار بحث سريع").parent(&app.tray_menu).build(&mut app.tray_show)?;
        nwg::MenuItem::builder().text("خروج").parent(&app.tray_menu).build(&mut app.tray_exit)?;

        app.state.get_mut().settings = settings.clone();
        let ui = AppUi { inner: Rc::new(app), handlers: Default::default(), raw_handlers: Default::default() };

        // Regular events.
        let weak = Rc::downgrade(&ui.inner);
        let handler = nwg::full_bind_event_handler(&ui.window.handle, move |evt, data, handle| {
            use nwg::Event as E;
            let Some(app) = weak.upgrade() else { return };
            match evt {
                E::OnResize | E::OnWindowMaximize => app.layout(),
                E::OnWindowClose if handle == app.window => app.on_close(),
                E::OnTextInput if handle == app.query => app.on_query_changed(),
                // The dialog-style message loop turns Enter into IDOK and Esc
                // into IDCANCEL, which nwg reports as OnKeyEnter / OnKeyEsc.
                E::OnKeyEnter => app.on_enter(),
                E::OnKeyEsc => app.on_escape(),
                E::OnKeyPress => app.on_key(handle, data.on_key()),
                E::OnTimerTick if handle == app.timer => app.on_tick(),
                E::OnNotice if handle == app.name_notice => app.on_names_found(),
                E::OnComboxBoxSelection if handle == app.kind_combo => app.on_kind_changed(),
                E::OnButtonClick => {
                    if handle == app.search_btn {
                        app.on_search();
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
                E::OnMousePress(nwg::MousePressEvent::MousePressLeftUp) if handle == app.tray => app.show_window(),
                E::OnContextMenu if handle == app.tray => app.show_menu(&app.tray_menu),
                E::OnMenuItemSelected => {
                    if handle == app.menu_open {
                        app.open_selected();
                    } else if handle == app.menu_folder {
                        app.open_folder();
                    } else if handle == app.menu_copy {
                        app.copy_path();
                    } else if handle == app.menu_copy_all {
                        app.copy_all_paths();
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
        let raw = nwg::bind_raw_event_handler(&ui.window.handle, 0x1_0001, move |_hwnd: HWND, msg: UINT, _w: WPARAM, l: LPARAM| {
            if msg != WM_NOTIFY {
                return None;
            }
            let hdr = unsafe { &*(l as *const NMHDR) };
            if hdr.hwndFrom != list_hwnd || hdr.code != LVN_GETDISPINFOW {
                return None;
            }
            let app = weak.upgrade()?;
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

        if settings.maximized
            && let Some(h) = ui.window.handle.hwnd()
        {
            unsafe { ShowWindow(h, SW_MAXIMIZE) };
        }
        ui.layout();
        ui.start();
        Ok(ui)
    }
}

/// The first installed font from a list of fonts known to render Arabic well.
fn ui_font_family() -> String {
    const PREFERRED: [&str; 5] = ["Segoe UI", "Tahoma", "Arial", "Noto Sans Arabic", "Noto Naskh Arabic"];
    let installed = nwg::Font::families();
    PREFERRED
        .iter()
        .find(|f| installed.iter().any(|i| i.eq_ignore_ascii_case(f)))
        .unwrap_or(&PREFERRED[0])
        .to_string()
}

/// Lets the user interface win when a background thread competes for the CPU.
fn background_priority() {
    use winapi::um::processthreadsapi::{GetCurrentThread, SetThreadPriority};
    use winapi::um::winbase::THREAD_PRIORITY_BELOW_NORMAL;
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL as i32) };
}

/// Appends a timestamped line to `%LOCALAPPDATA%\BahthSari\log.txt`, kept short.
fn log(line: &str) {
    let path = index::cache_path().with_file_name("log.txt");
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

    let name = shell::wide("Local\\BahthSari.SingleInstance");
    // The mutex lives as long as this process; Windows frees it on exit.
    let mutex = unsafe { CreateMutexW(std::ptr::null_mut(), 0, name.as_ptr()) };
    if mutex.is_null() || unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
        return false;
    }
    let (class, title) = (shell::wide("NativeWindowsGuiWindow"), shell::wide(TITLE));
    let hwnd = unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) };
    if hwnd.is_null() {
        return false;
    }
    unsafe {
        ShowWindow(hwnd, if IsIconic(hwnd) != 0 { SW_RESTORE } else { SW_SHOW });
        SetForegroundWindow(hwnd);
    }
    true
}

pub fn run() {
    if activate_running_copy() {
        return;
    }
    unsafe {
        winapi::um::objbase::CoInitialize(std::ptr::null_mut());
    }
    if let Err(e) = nwg::init() {
        eprintln!("failed to initialise the UI: {e}");
        return;
    }
    let _ = nwg::Font::set_global_family(&ui_font_family());
    let started = std::time::Instant::now();
    let ui = match App::build_ui(App::default()) {
        Ok(ui) => {
            log(&format!("window ready in {} ms (v{})", started.elapsed().as_millis(), env!("CARGO_PKG_VERSION")));
            ui
        }
        Err(e) => {
            nwg::fatal_message(TITLE, &format!("تعذر إنشاء الواجهة: {e}"));
        }
    };
    nwg::dispatch_thread_events();
    drop(ui);
}
