//! The Windows user interface, built on native Win32 controls so Arabic text
//! is shaped, joined and laid out right-to-left by Windows itself.

use native_windows_gui as nwg;
use nwg::NativeUi;
use std::cell::RefCell;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use winapi::shared::minwindef::{LPARAM, UINT, WPARAM};
use winapi::shared::windef::{HWND, POINT};
use winapi::um::commctrl::{LVIF_TEXT, LVN_GETDISPINFOW, LVS_OWNERDATA, NMLVDISPINFOW};
use winapi::um::winuser::{GetCursorPos, NMHDR, SendMessageW, VK_RETURN, WM_CHAR, WM_KEYDOWN, WM_NOTIFY, WM_SETFONT};

use crate::index::{self, Entry, Index, ScanProgress};
use crate::search::{self, ContentHit, ContentProgress, Query};

/// Mirrors the whole window so it reads right-to-left.
const WS_EX_LAYOUTRTL: u32 = 0x0040_0000;

const COL_NAME: usize = 0;
const COL_PATH: usize = 1;
const COL_SIZE: usize = 2;
const COL_DATE: usize = 3;
const COL_SNIPPET: usize = 4;

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

#[derive(Default)]
struct State {
    index: Arc<Index>,
    /// The index snapshot that `rows` point into.
    rows_index: Arc<Index>,
    rows: Vec<Row>,
    /// Folder the search is restricted to; empty means all drives.
    scope: String,
    scan: Option<ScanJob>,
    content: Option<ContentJob>,
}

#[derive(Default)]
pub struct App {
    window: nwg::Window,
    font: nwg::Font,
    query: nwg::TextInput,
    search_btn: nwg::Button,
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
    stop_btn: nwg::Button,
    status: nwg::StatusBar,
    timer: nwg::AnimationTimer,
    enter_notice: nwg::Notice,
    folder_dialog: nwg::FileDialog,
    menu: nwg::Menu,
    menu_open: nwg::MenuItem,
    menu_folder: nwg::MenuItem,
    menu_copy: nwg::MenuItem,
    state: RefCell<State>,
}

impl App {
    fn scaled(&self, v: i32) -> i32 {
        (v as f64 * nwg::scale_factor()).round() as i32
    }

    // ---------------------------------------------------------------- layout

    fn layout(&self) {
        let (w, h) = self.window.size();
        let (w, h) = (w as i32, h as i32);
        let m = 10;
        let row_h = 30;
        let btn_w = 120;

        // Row 1: query box + search button.
        self.query.set_position(m, m);
        self.query.set_size((w - 3 * m - btn_w).max(100) as u32, row_h as u32);
        self.search_btn.set_position(w - m - btn_w, m);
        self.search_btn.set_size(btn_w as u32, row_h as u32);

        // Row 2: content checkbox, scope and index buttons.
        let y2 = m * 2 + row_h;
        let mut x = m;
        self.content_chk.set_position(x, y2);
        self.content_chk.set_size(240, row_h as u32);
        x += 240 + m;
        self.scope_label.set_position(x, y2 + 4);
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

        // Bottom: action buttons above the status bar.
        let status_h = 26;
        let y_btn = h - status_h - m - row_h;
        let mut x = m;
        for b in [&self.open_btn, &self.folder_btn, &self.copy_btn, &self.stop_btn] {
            b.set_position(x, y_btn);
            b.set_size(btn_w as u32, row_h as u32);
            x += btn_w + m;
        }

        // The result list fills the rest.
        let y3 = y2 + row_h + m;
        self.list.set_position(m, y3);
        self.list.set_size((w - 2 * m).max(100) as u32, (y_btn - m - y3).max(50) as u32);
    }

    // ---------------------------------------------------------------- index

    fn start(&self) {
        let cache = index::cache_path();
        match Index::load(&cache) {
            Some(idx) => {
                let n = idx.entries.len();
                self.state.borrow_mut().index = Arc::new(idx);
                self.set_status(&format!("تم تحميل الفهرس ({} عنصر). اكتب للبحث.", fmt_num(n)));
            }
            None => self.start_scan(),
        }
        self.query.set_focus();
    }

    fn start_scan(&self) {
        let mut st = self.state.borrow_mut();
        if st.scan.is_some() {
            return;
        }
        let progress = Arc::new(ScanProgress::default());
        let result = Arc::new(Mutex::new(None));
        let (p, r) = (progress.clone(), result.clone());
        std::thread::spawn(move || {
            let idx = Index::build(&index::list_drives(), &p);
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

    fn search_names(&self) {
        self.cancel_content();
        let q = Query::parse(&self.query.text());
        let mut st = self.state.borrow_mut();
        let started = std::time::Instant::now();
        let hits = search::search_names(&st.index.entries, &q, &st.scope);
        st.rows = hits.into_iter().map(|entry| Row { entry, snippet: None }).collect();
        st.rows_index = st.index.clone();
        let n = st.rows.len();
        let scanning = st.scan.is_some();
        drop(st);
        self.refresh_list();
        if q.is_empty() {
            if !scanning {
                self.set_status("اكتب اسم الملف أو جزءاً منه للبحث.");
            }
        } else {
            self.set_status(&format!(
                "وجدت {} نتيجة في {} ملي ثانية{}",
                fmt_num(n),
                started.elapsed().as_millis(),
                if scanning { " (الفهرسة ما زالت جارية)" } else { "" }
            ));
        }
    }

    fn search_content(&self) {
        self.cancel_content();
        let q = Query::parse(&self.query.text());
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
    }

    fn selected_path(&self) -> Option<String> {
        let row = self.list.selected_item()?;
        let st = self.state.borrow();
        let r = st.rows.get(row)?;
        Some(st.rows_index.entries.get(r.entry as usize)?.path.to_string())
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
                self.set_status(&format!("تم نسخ المسار: {p}"));
            }
            None => self.set_status("اختر ملفاً من القائمة أولاً."),
        }
    }

    fn show_menu(&self) {
        let mut pt = POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut pt) };
        self.menu.popup(pt.x, pt.y);
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
            COL_DATE => shell::format_time(e.mtime),
            COL_SNIPPET => r.snippet.clone().unwrap_or_default(),
            _ => String::new(),
        }
    }

    fn set_status(&self, text: &str) {
        self.status.set_text(0, text);
    }

    fn on_close(&self) {
        self.cancel_content();
        if let Some(job) = &self.state.borrow().scan {
            job.progress.cancel.store(true, Ordering::Relaxed);
        }
        nwg::stop_thread_dispatch();
    }
}

// -------------------------------------------------------------------- formatting

/// Keeps a path reading left-to-right even when folder names are Arabic, by
/// putting left-to-right marks around every separator.
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

fn fmt_size(e: &Entry) -> String {
    if e.is_dir {
        return "مجلد".into();
    }
    let b = e.size as f64;
    if b < 1024.0 {
        format!("{} بايت", e.size)
    } else if b < 1024.0 * 1024.0 {
        format!("{:.1} ك.ب", b / 1024.0)
    } else if b < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} م.ب", b / 1024.0 / 1024.0)
    } else {
        format!("{:.2} ج.ب", b / 1024.0 / 1024.0 / 1024.0)
    }
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
    out
}

// -------------------------------------------------------------------- shell helpers

mod shell {
    use std::os::windows::process::CommandExt;
    use winapi::um::minwinbase::SYSTEMTIME;
    use winapi::um::shellapi::ShellExecuteW;
    use winapi::um::timezoneapi::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    use winapi::um::winuser::SW_SHOWNORMAL;

    fn wide(s: &str) -> Vec<u16> {
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

    /// Formats Unix seconds as local `YYYY/MM/DD HH:MM`.
    pub fn format_time(secs: i64) -> String {
        if secs <= 0 {
            return String::new();
        }
        // FILETIME counts 100 ns intervals since 1601-01-01.
        let ft = (secs as u64 + 11_644_473_600) * 10_000_000;
        let ft = winapi::shared::minwindef::FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
        unsafe {
            let mut utc: SYSTEMTIME = std::mem::zeroed();
            let mut local: SYSTEMTIME = std::mem::zeroed();
            if FileTimeToSystemTime(&ft, &mut utc) == 0
                || SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) == 0
            {
                return String::new();
            }
            format!(
                "{:04}/{:02}/{:02} {:02}:{:02}",
                local.wYear, local.wMonth, local.wDay, local.wHour, local.wMinute
            )
        }
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
        let font_px = (18.0 * nwg::scale_factor()).round() as u32;
        nwg::Font::builder().family("Segoe UI").size(font_px).build(&mut app.font)?;
        let f = Some(&app.font);

        nwg::Window::builder()
            .title("بحث سريع — البحث عن الملفات وداخل المستندات")
            .size((1100, 680))
            .center(true)
            .ex_flags(WS_EX_LAYOUTRTL)
            .flags(nwg::WindowFlags::MAIN_WINDOW | nwg::WindowFlags::VISIBLE)
            .build(&mut app.window)?;
        let w = &app.window;

        nwg::TextInput::builder()
            .parent(w)
            .font(f)
            .placeholder_text(Some("اكتب اسم الملف أو كلمة للبحث…  (مثال: تقرير  ‎.pdf)"))
            .focus(true)
            .build(&mut app.query)?;
        nwg::Button::builder().parent(w).font(f).text("بحث").build(&mut app.search_btn)?;
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
                    | LVS_OWNERDATA,
            )
        };
        nwg::ListView::builder()
            .parent(w)
            .flags(list_flags)
            .list_style(nwg::ListViewStyle::Detailed)
            .ex_flags(nwg::ListViewExFlags::FULL_ROW_SELECT | nwg::ListViewExFlags::GRID)
            .double_buffer(true)
            .build(&mut app.list)?;
        if let Some(h) = app.list.handle.hwnd() {
            unsafe { SendMessageW(h, WM_SETFONT, app.font.handle as WPARAM, 1) };
        }
        let columns = [("الاسم", 260), ("المسار الكامل", 460), ("الحجم", 90), ("تاريخ التعديل", 140), ("النص المطابق", 360)];
        for (i, (title, width)) in columns.iter().enumerate() {
            app.list.insert_column(nwg::InsertListViewColumn {
                index: Some(i as i32),
                fmt: None,
                width: Some(app.scaled(*width)),
                text: Some(title.to_string()),
            });
        }

        nwg::Button::builder().parent(w).font(f).text("فتح الملف").build(&mut app.open_btn)?;
        nwg::Button::builder().parent(w).font(f).text("فتح المجلد").build(&mut app.folder_btn)?;
        nwg::Button::builder().parent(w).font(f).text("نسخ المسار").build(&mut app.copy_btn)?;
        nwg::Button::builder().parent(w).font(f).text("إيقاف البحث").enabled(false).build(&mut app.stop_btn)?;
        nwg::StatusBar::builder().parent(w).font(f).text("").build(&mut app.status)?;

        nwg::AnimationTimer::builder()
            .parent(w)
            .interval(Duration::from_millis(250))
            .active(true)
            .build(&mut app.timer)?;
        nwg::Notice::builder().parent(w).build(&mut app.enter_notice)?;
        nwg::FileDialog::builder()
            .title("اختر المجلد الذي تريد البحث فيه")
            .action(nwg::FileDialogAction::OpenDirectory)
            .build(&mut app.folder_dialog)?;

        nwg::Menu::builder().popup(true).parent(w).build(&mut app.menu)?;
        nwg::MenuItem::builder().text("فتح الملف").parent(&app.menu).build(&mut app.menu_open)?;
        nwg::MenuItem::builder().text("فتح المجلد الذي يحتويه").parent(&app.menu).build(&mut app.menu_folder)?;
        nwg::MenuItem::builder().text("نسخ المسار").parent(&app.menu).build(&mut app.menu_copy)?;

        let ui = AppUi { inner: Rc::new(app), handlers: Default::default(), raw_handlers: Default::default() };

        // Regular events.
        let weak = Rc::downgrade(&ui.inner);
        let handler = nwg::full_bind_event_handler(&ui.window.handle, move |evt, _data, handle| {
            use nwg::Event as E;
            let Some(app) = weak.upgrade() else { return };
            match evt {
                E::OnResize | E::OnWindowMaximize => app.layout(),
                E::OnWindowClose if handle == app.window => app.on_close(),
                E::OnTextInput if handle == app.query => app.on_query_changed(),
                E::OnNotice if handle == app.enter_notice => app.on_search(),
                E::OnTimerTick if handle == app.timer => app.on_tick(),
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
                    } else if handle == app.stop_btn {
                        app.on_stop();
                    }
                }
                E::OnListViewItemActivated if handle == app.list => app.open_selected(),
                E::OnListViewRightClick if handle == app.list => app.show_menu(),
                E::OnMenuItemSelected => {
                    if handle == app.menu_open {
                        app.open_selected();
                    } else if handle == app.menu_folder {
                        app.open_folder();
                    } else if handle == app.menu_copy {
                        app.copy_path();
                    }
                }
                _ => {}
            }
        });
        ui.handlers.borrow_mut().push(handler);

        // Virtual list: answer LVN_GETDISPINFO with the text of each cell.
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
            Some(0)
        })?;
        ui.raw_handlers.borrow_mut().push(raw);

        // Enter in the query box starts a search.
        let notice = ui.enter_notice.sender();
        let raw = nwg::bind_raw_event_handler(&ui.query.handle, 0x1_0002, move |_hwnd, msg, w, _l| {
            match msg {
                WM_KEYDOWN if w as i32 == VK_RETURN => {
                    notice.notice();
                    Some(0)
                }
                // Swallow the matching character so the edit box does not beep.
                WM_CHAR if w == '\r' as WPARAM => Some(0),
                _ => None,
            }
        })?;
        ui.raw_handlers.borrow_mut().push(raw);

        ui.layout();
        ui.start();
        Ok(ui)
    }
}

pub fn run() {
    if let Err(e) = nwg::init() {
        eprintln!("failed to initialise the UI: {e}");
        return;
    }
    let _ = nwg::Font::set_global_family("Segoe UI");
    let ui = match App::build_ui(App::default()) {
        Ok(ui) => ui,
        Err(e) => {
            nwg::fatal_message("بحث سريع", &format!("تعذر إنشاء الواجهة: {e}"));
        }
    };
    nwg::dispatch_thread_events();
    drop(ui);
}
