//! The duplicate-file finder (Options → Find duplicate files…): an owned
//! window with a virtual list of duplicate groups, filled by `crate::dupes`
//! on a background thread with progress and a Stop button.
//!
//! Nothing is ever deleted without the user selecting the files and
//! confirming, and all copies of a group can't be deleted together.

use native_windows_gui as nwg;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use winapi::shared::minwindef::{DWORD, LPARAM, UINT, WPARAM};
use winapi::shared::windef::HWND;
use winapi::shared::winerror::ERROR_SUCCESS;
use winapi::um::commctrl::{LVIF_TEXT, LVN_GETDISPINFOW, LVS_EX_DOUBLEBUFFER, LVS_OWNERDATA, NMLVDISPINFOW};
use winapi::um::winnt::REG_DWORD;
use winapi::um::winreg::{RegQueryValueExW, RegSetValueExW};
use winapi::um::winuser::{NMHDR, SendMessageW, WM_NOTIFY, WM_SETFONT};

use super::format::{digits, fmt_num, ltr_path};
use super::shell::{self, wide};
use super::lazy::{Bound, Lazy};
use super::startup::background_priority;
use super::{App, WS_EX_LAYOUTRTL, autostart, title};
use crate::dupes::{self, Mode, Progress, Report, Scope};

/// Minimum file sizes offered, in bytes. Empty files are always skipped.
const MIN_SIZES: [u64; 6] = [1, 1024, 100 * 1024, 1024 * 1024, 10 * 1024 * 1024, 100 * 1024 * 1024];
const MIN_LABELS: [(&str, &str); 6] = [
    ("كل الأحجام", "Any size"),
    ("1 ك.ب فأكثر", "1 KB or more"),
    ("100 ك.ب فأكثر", "100 KB or more"),
    ("1 م.ب فأكثر", "1 MB or more"),
    ("10 م.ب فأكثر", "10 MB or more"),
    ("100 م.ب فأكثر", "100 MB or more"),
];
const DEFAULT_MIN: usize = 1;

/// Remembered choices, DWORDs under `HKCU\Software\BahthSari`.
const REG_MIN: &str = "DupesMinSize";
const REG_MODE: &str = "DupesMode";

const COLUMNS: [(&str, &str, i32); 4] = [
    ("الاسم", "Name", 280),
    ("المجلد", "Folder", 400),
    ("الحجم", "Size", 100),
    ("تاريخ التعديل", "Date modified", 140),
];

/// A list row: a group heading or one of its files.
#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    File(usize, usize),
}

struct Job {
    progress: Arc<Progress>,
    /// `Some(None)` when it was cancelled.
    result: Arc<Mutex<Option<Option<Report>>>>,
}

/// The finder's menu item, window and state, one field of `App`.
#[derive(Default)]
pub struct Dupes {
    pub(super) menu_item: nwg::MenuItem,
    /// Created the first time the finder opens.
    ui: Lazy<DupesUi>,
    folder_dialog: RefCell<Option<nwg::FileDialog>>,
    /// What to search; looked up in the current index at each start, so
    /// the finder never holds an old index alive.
    scope: RefCell<Option<Scope>>,
    job: RefCell<Option<Job>>,
    report: RefCell<Report>,
    rows: RefCell<Vec<Row>>,
}

impl std::ops::Deref for Dupes {
    type Target = DupesUi;
    fn deref(&self) -> &DupesUi {
        &self.ui
    }
}

/// The finder window's controls.
#[derive(Default)]
pub struct DupesUi {
    window: nwg::Window,
    scope_label: nwg::Label,
    folder_btn: nwg::Button,
    min_combo: nwg::ComboBox<&'static str>,
    mode_combo: nwg::ComboBox<&'static str>,
    start_btn: nwg::Button,
    stop_btn: nwg::Button,
    list: nwg::ListView,
    open_btn: nwg::Button,
    reveal_btn: nwg::Button,
    delete_btn: nwg::Button,
    summary: nwg::Label,
    timer: nwg::AnimationTimer,
    menu: nwg::Menu,
    menu_open: nwg::MenuItem,
    menu_reveal: nwg::MenuItem,
    menu_copy: nwg::MenuItem,
    menu_delete: nwg::MenuItem,
}

// ---------------------------------------------------------------- registry

fn load_dword(name: &str) -> Option<u32> {
    let key = autostart::open(autostart::OWN_KEY, false)?;
    let name = wide(name);
    let (mut value, mut size, mut kind): (DWORD, DWORD, DWORD) = (0, 4, 0);
    let ok = unsafe {
        RegQueryValueExW(key.0, name.as_ptr(), std::ptr::null_mut(), &mut kind, (&mut value as *mut DWORD).cast(), &mut size)
    } == ERROR_SUCCESS as i32;
    (ok && kind == REG_DWORD).then_some(value)
}

fn save_dword(name: &str, value: u32) {
    let Some(key) = autostart::open(autostart::OWN_KEY, true) else { return };
    let name = wide(name);
    unsafe {
        RegSetValueExW(key.0, name.as_ptr(), 0, REG_DWORD, (&value as *const DWORD).cast(), 4);
    }
}

// ---------------------------------------------------------------- formatting

fn size_text(b: u64) -> String {
    let f = b as f64;
    digits(if f < 1024.0 {
        trf!("{} بايت", "{} bytes", b)
    } else if f < 1024.0 * 1024.0 {
        trf!("{:.1} ك.ب", "{:.1} KB", f / 1024.0)
    } else if f < 1024.0 * 1024.0 * 1024.0 {
        trf!("{:.1} م.ب", "{:.1} MB", f / 1024.0 / 1024.0)
    } else {
        trf!("{:.2} ج.ب", "{:.2} GB", f / 1024.0 / 1024.0 / 1024.0)
    })
}

fn split_path(path: &str) -> (&str, &str) {
    match path.rfind('\\') {
        Some(i) => (&path[i + 1..], &path[..i + 1]),
        None => (path, ""),
    }
}

fn button(parent: &nwg::Window, font: Option<&nwg::Font>, text: &str, out: &mut nwg::Button) -> Result<(), nwg::NwgError> {
    nwg::Button::builder().parent(parent).font(font).text(text).build(out)
}

impl App {
    /// Builds the menu item. Called from `build_ui`; the window is created
    /// when it first opens.
    pub(super) fn build_dupes(&mut self) -> Result<(), nwg::NwgError> {
        nwg::MenuItem::builder()
            .text(tr!("البحث عن الملفات المكررة…", "Find duplicate files…"))
            .parent(&self.options_menu)
            .build(&mut self.dupes.menu_item)
    }

    /// Builds the (hidden) window and binds its events, once.
    fn ensure_dupes(self: &Rc<Self>) -> bool {
        self.dupes.ui.is_built() || self.build_lazily("dupes", |app| {
            app.build_dupes_window()?;
            app.bind_dupes_window()
        })
    }

    fn build_dupes_window(&self) -> Result<(), nwg::NwgError> {
        let mut ui = DupesUi::default();
        let d = &mut ui;
        let f = Some(&self.font);
        nwg::Window::builder()
            .title(tr!("الملفات المكررة", "Duplicate files"))
            .size((960, 600))
            .center(true)
            .ex_flags(crate::i18n::rtl_only(WS_EX_LAYOUTRTL))
            .flags(nwg::WindowFlags::WINDOW | nwg::WindowFlags::RESIZABLE | nwg::WindowFlags::MAXIMIZE_BOX)
            .icon(None)
            .parent(Some(&self.window))
            .build(&mut d.window)?;
        let w = &d.window;
        nwg::Label::builder().parent(w).font(f).text("").build(&mut d.scope_label)?;
        button(w, f, tr!("اختيار مجلد…", "Choose folder…"), &mut d.folder_btn)?;
        let mins: Vec<&'static str> = MIN_LABELS.iter().map(|(a, e)| tr!(*a, *e)).collect();
        let min = load_dword(REG_MIN).map(|v| v as usize).filter(|&v| v < MIN_SIZES.len()).unwrap_or(DEFAULT_MIN);
        nwg::ComboBox::builder().parent(w).font(f).collection(mins).selected_index(Some(min)).build(&mut d.min_combo)?;
        let modes = vec![
            tr!("نفس المحتوى (مقارنة دقيقة)", "Same content (exact)"),
            tr!("نفس الاسم والحجم (سريع)", "Same name and size (quick)"),
        ];
        let mode = load_dword(REG_MODE).map(|v| v as usize).filter(|&v| v < 2).unwrap_or(0);
        nwg::ComboBox::builder().parent(w).font(f).collection(modes).selected_index(Some(mode)).build(&mut d.mode_combo)?;
        button(w, f, tr!("بحث", "Search"), &mut d.start_btn)?;
        nwg::Button::builder().parent(w).font(f).text(tr!("إيقاف", "Stop")).enabled(false).build(&mut d.stop_btn)?;

        let flags = unsafe {
            nwg::ListViewFlags::from_bits_unchecked(
                nwg::ListViewFlags::VISIBLE.bits()
                    | nwg::ListViewFlags::TAB_STOP.bits()
                    | nwg::ListViewFlags::ALWAYS_SHOW_SELECTION.bits()
                    | LVS_OWNERDATA,
            )
        };
        nwg::ListView::builder()
            .parent(w)
            .flags(flags)
            .list_style(nwg::ListViewStyle::Detailed)
            .ex_flags(unsafe {
                nwg::ListViewExFlags::from_bits_unchecked(nwg::ListViewExFlags::FULL_ROW_SELECT.bits() | LVS_EX_DOUBLEBUFFER)
            })
            .build(&mut d.list)?;
        if let Some(h) = d.list.handle.hwnd() {
            unsafe {
                SendMessageW(h, WM_SETFONT, self.font.handle as WPARAM, 1);
                let theme = wide("Explorer");
                winapi::um::uxtheme::SetWindowTheme(h, theme.as_ptr(), std::ptr::null());
            }
        }
        let scale = nwg::scale_factor();
        for (i, (ar, en, width)) in COLUMNS.iter().enumerate() {
            d.list.insert_column(nwg::InsertListViewColumn {
                index: Some(i as i32),
                fmt: None,
                width: Some((*width as f64 * scale).round() as i32),
                text: Some(tr!(*ar, *en).to_string()),
            });
        }
        d.list.set_headers_enabled(true);

        button(w, f, tr!("فتح الملف", "Open file"), &mut d.open_btn)?;
        button(w, f, tr!("فتح المجلد", "Open folder"), &mut d.reveal_btn)?;
        button(w, f, tr!("نقل المحدد إلى سلة المحذوفات…", "Move selected to Recycle Bin…"), &mut d.delete_btn)?;
        nwg::Label::builder().parent(w).font(f).text("").build(&mut d.summary)?;
        nwg::AnimationTimer::builder().parent(w).interval(Duration::from_millis(200)).active(false).build(&mut d.timer)?;

        nwg::Menu::builder().popup(true).parent(w).build(&mut d.menu)?;
        nwg::MenuItem::builder().text(tr!("فتح الملف\tEnter", "Open file\tEnter")).parent(&d.menu).build(&mut d.menu_open)?;
        nwg::MenuItem::builder().text(tr!("فتح المجلد الذي يحتويه", "Open containing folder")).parent(&d.menu).build(&mut d.menu_reveal)?;
        nwg::MenuItem::builder().text(tr!("نسخ المسار", "Copy path")).parent(&d.menu).build(&mut d.menu_copy)?;
        nwg::MenuItem::builder()
            .text(tr!("نقل المحدد إلى سلة المحذوفات…\tDelete", "Move selected to Recycle Bin…\tDelete"))
            .parent(&d.menu)
            .build(&mut d.menu_delete)?;
        self.dupes.ui.set(ui);
        Ok(())
    }

    /// Binds the menu item's event. Called once from `build_ui`; the
    /// handler is unbound with the others.
    pub(super) fn setup_dupes(self: &Rc<Self>) -> nwg::EventHandler {
        let weak = Rc::downgrade(self);
        self.bind_main_events(move |evt, _data, handle| {
            let Some(app) = weak.upgrade() else { return };
            if evt == nwg::Event::OnMenuItemSelected && handle == app.dupes.menu_item && app.ensure_dupes() {
                app.open_dupes();
            }
        })
    }

    /// Binds the finder window's events, when it is created.
    fn bind_dupes_window(self: &Rc<Self>) -> Result<Bound, nwg::NwgError> {
        let weak = Rc::downgrade(self);
        let own = nwg::full_bind_event_handler(&self.dupes.window.handle, move |evt, data, handle| {
            use nwg::Event as E;
            let Some(app) = weak.upgrade() else { return };
            let d = &app.dupes;
            match evt {
                E::OnWindowClose if handle == d.window => {
                    if let nwg::EventData::OnWindowClose(c) = &data {
                        c.close(false);
                    }
                    app.stop_dupes();
                    d.window.set_visible(false);
                }
                E::OnResize | E::OnWindowMaximize if handle == d.window => app.dupes_layout(),
                E::OnTimerTick if handle == d.timer => app.poll_dupes(),
                E::OnButtonClick => {
                    if handle == d.start_btn {
                        app.start_dupes();
                    } else if handle == d.stop_btn {
                        app.stop_dupes();
                    } else if handle == d.folder_btn {
                        if app.pick_dupes_folder() {
                            app.start_dupes();
                        }
                    } else if handle == d.open_btn {
                        app.dupes_open(false);
                    } else if handle == d.reveal_btn {
                        app.dupes_open(true);
                    } else if handle == d.delete_btn {
                        app.dupes_delete();
                    }
                }
                E::OnComboxBoxSelection => {
                    if handle == d.min_combo {
                        save_dword(REG_MIN, d.min_combo.selection().unwrap_or(DEFAULT_MIN) as u32);
                    } else if handle == d.mode_combo {
                        save_dword(REG_MODE, d.mode_combo.selection().unwrap_or(0) as u32);
                    }
                }
                E::OnListViewItemActivated if handle == d.list => app.dupes_open(false),
                E::OnListViewRightClick if handle == d.list => app.show_menu(&d.menu),
                E::OnKeyEnter if d.list.focus() => app.dupes_open(false),
                E::OnKeyPress if handle == d.list && data.on_key() == winapi::um::winuser::VK_DELETE as u32 => {
                    app.dupes_delete();
                }
                E::OnMenuItemSelected => {
                    if handle == d.menu_open {
                        app.dupes_open(false);
                    } else if handle == d.menu_reveal {
                        app.dupes_open(true);
                    } else if handle == d.menu_copy {
                        if let Some(p) = app.dupes_selected_files().first() {
                            nwg::Clipboard::set_data_text(&d.window, p);
                        }
                    } else if handle == d.menu_delete {
                        app.dupes_delete();
                    }
                }
                _ => {}
            }
        });

        // Virtual list: answer LVN_GETDISPINFO with each cell's text.
        let weak = Rc::downgrade(self);
        let list_hwnd = self.dupes.list.handle.hwnd().unwrap_or(std::ptr::null_mut());
        let raw = nwg::bind_raw_event_handler(&self.dupes.window.handle, 0x1_0026, move |_h: HWND, msg: UINT, _w: WPARAM, l: LPARAM| {
            if msg != WM_NOTIFY {
                return None;
            }
            let hdr = unsafe { &*(l as *const NMHDR) };
            if hdr.hwndFrom != list_hwnd || hdr.code != LVN_GETDISPINFOW {
                return None;
            }
            let app = weak.upgrade()?;
            let item = unsafe { &mut (*(l as *mut NMLVDISPINFOW)).item };
            if item.mask & LVIF_TEXT != 0 && !item.pszText.is_null() && item.cchTextMax > 0 {
                let text = app.dupes_cell(item.iItem as usize, item.iSubItem as usize);
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
        Ok((vec![own], vec![raw]))
    }

    fn dupes_layout(&self) {
        let d = &self.dupes;
        let (w, h) = d.window.size();
        let (w, h) = (w as i32, h as i32);
        let (m, row_h, btn_w) = (10, 30, 130);
        // Row 1: the scope and the folder button.
        d.folder_btn.set_position(w - m - btn_w, m);
        d.folder_btn.set_size(btn_w as u32, row_h as u32);
        d.scope_label.set_position(m, m + 6);
        d.scope_label.set_size((w - 3 * m - btn_w).max(50) as u32, row_h as u32);
        // Row 2: options and start/stop.
        let y2 = 2 * m + row_h;
        let mut x = m;
        d.min_combo.set_position(x, y2 + 1);
        d.min_combo.set_size(170, 300);
        x += 170 + m;
        d.mode_combo.set_position(x, y2 + 1);
        d.mode_combo.set_size(260, 300);
        x += 260 + m;
        for b in [&d.start_btn, &d.stop_btn] {
            b.set_position(x, y2);
            b.set_size(100, row_h as u32);
            x += 100 + m;
        }
        // Bottom: summary line above the action buttons.
        let y_btn = h - m - row_h;
        let mut x = m;
        for (b, bw) in [(&d.open_btn, btn_w), (&d.reveal_btn, btn_w), (&d.delete_btn, 260)] {
            b.set_position(x, y_btn);
            b.set_size(bw as u32, row_h as u32);
            x += bw + m;
        }
        let y_sum = y_btn - m - 22;
        d.summary.set_position(m, y_sum);
        d.summary.set_size((w - 2 * m).max(50) as u32, 22);
        let y3 = y2 + row_h + m;
        d.list.set_position(m, y3);
        d.list.set_size((w - 2 * m).max(100) as u32, (y_sum - m - y3).max(50) as u32);
    }

    // ---------------------------------------------------------------- scope & job

    /// Opens the finder on the current results, or on a folder the user picks.
    fn open_dupes(&self) {
        let d = &self.dupes;
        if d.job.borrow().is_none() {
            let st = self.state.borrow();
            let has_results = !self.query.text().trim().is_empty() && !st.rows.is_empty();
            if has_results {
                let paths: Vec<String> =
                    st.rows.iter().filter_map(|r| st.rows_index.entries.get(r.entry as usize)).map(|e| e.path.to_string()).collect();
                let n = paths.len();
                *d.scope.borrow_mut() = Some(Scope::Paths(Arc::new(paths)));
                drop(st);
                d.scope_label.set_text(&trf!(
                    "النطاق: نتائج البحث الحالية ({} نتيجة)",
                    "Scope: the current search results ({} results)",
                    fmt_num(n)
                ));
            } else {
                drop(st);
                if !self.pick_dupes_folder() {
                    return;
                }
            }
            *d.report.borrow_mut() = Report::default();
            self.dupes_show_report();
        }
        d.window.set_visible(true);
        if let Some(h) = d.window.handle.hwnd() {
            unsafe { winapi::um::winuser::SetForegroundWindow(h) };
        }
        self.dupes_layout();
        if d.job.borrow().is_none() {
            self.start_dupes();
        }
    }

    /// Asks for a folder and makes it the scope. False when cancelled or the
    /// index isn't ready.
    fn pick_dupes_folder(&self) -> bool {
        let d = &self.dupes;
        if d.job.borrow().is_some() {
            return false;
        }
        if self.state.borrow().index.entries.is_empty() {
            self.set_status(tr!("انتظر حتى تكتمل الفهرسة.", "Wait until indexing is complete."));
            return false;
        }
        let mut slot = d.folder_dialog.borrow_mut();
        if slot.is_none() {
            let mut dlg = nwg::FileDialog::default();
            let built = nwg::FileDialog::builder()
                .title(tr!("اختر المجلد الذي تريد البحث فيه عن الملفات المكررة", "Choose the folder to search for duplicate files"))
                .action(nwg::FileDialogAction::OpenDirectory)
                .build(&mut dlg);
            if built.is_err() {
                return false;
            }
            *slot = Some(dlg);
        }
        let Some(dialog) = slot.as_ref() else { return false };
        let owner = if d.window.visible() { &d.window } else { &self.window };
        let chosen = if dialog.run(Some(owner)) { dialog.get_selected_item().ok() } else { None };
        drop(slot);
        let Some(dir) = chosen else { return false };
        let mut dir = dir.to_string_lossy().into_owned();
        if !dir.ends_with('\\') {
            dir.push('\\');
        }
        d.scope_label.set_text(&trf!("النطاق: {}", "Scope: {}", ltr_path(&dir)));
        *d.scope.borrow_mut() = Some(Scope::Folder(dir));
        true
    }

    fn start_dupes(&self) {
        let d = &self.dupes;
        if d.job.borrow().is_some() {
            return;
        }
        let Some(scope) = d.scope.borrow().clone() else { return };
        let index = self.state.borrow().index.clone();
        let min_size = MIN_SIZES[d.min_combo.selection().unwrap_or(DEFAULT_MIN).min(MIN_SIZES.len() - 1)];
        let mode = if d.mode_combo.selection() == Some(1) { Mode::NameAndSize } else { Mode::Content };
        let progress = Arc::new(Progress::default());
        let result = Arc::new(Mutex::new(None));
        {
            let (progress, result) = (progress.clone(), result.clone());
            std::thread::spawn(move || {
                background_priority();
                let r = dupes::find(&index, &scope, mode, min_size, &progress);
                if let Ok(mut slot) = result.lock() {
                    *slot = Some(r);
                }
            });
        }
        *d.job.borrow_mut() = Some(Job { progress, result });
        *d.report.borrow_mut() = Report::default();
        d.rows.borrow_mut().clear();
        d.list.set_item_count(0);
        d.list.invalidate();
        self.dupes_busy(true);
        d.summary.set_text(tr!("جارٍ التجميع حسب الحجم…", "Grouping by size…"));
        d.timer.start();
    }

    fn dupes_busy(&self, busy: bool) {
        let d = &self.dupes;
        d.stop_btn.set_enabled(busy);
        for b in [&d.start_btn, &d.folder_btn, &d.delete_btn] {
            b.set_enabled(!busy);
        }
        d.min_combo.set_enabled(!busy);
        d.mode_combo.set_enabled(!busy);
    }

    fn stop_dupes(&self) {
        if let Some(job) = &*self.dupes.job.borrow() {
            job.progress.cancel.store(true, Ordering::Relaxed);
        }
    }

    fn poll_dupes(&self) {
        let d = &self.dupes;
        let done = {
            let job = d.job.borrow();
            let Some(job) = job.as_ref() else {
                d.timer.stop();
                return;
            };
            let p = &job.progress;
            let taken = job.result.lock().ok().and_then(|mut r| r.take());
            if taken.is_none() {
                let (n, total) = (p.files_done.load(Ordering::Relaxed), p.files_total.load(Ordering::Relaxed));
                let (b, bt) = (p.bytes_done.load(Ordering::Relaxed), p.bytes_total.load(Ordering::Relaxed));
                let pct = (b * 100).checked_div(bt).unwrap_or(0).min(100);
                let text = match p.stage.load(Ordering::Relaxed) {
                    0 => tr!("جارٍ التجميع حسب الحجم…", "Grouping by size…").to_string(),
                    1 => trf!(
                        "مقارنة بداية الملفات ونهايتها: {} من {}",
                        "Comparing the start and end of files: {} of {}",
                        fmt_num(n),
                        fmt_num(total)
                    ),
                    _ => trf!(
                        "مقارنة المحتوى كاملاً: {} من {} ملف ({}%)",
                        "Comparing whole contents: {} of {} files ({}%)",
                        fmt_num(n),
                        fmt_num(total),
                        digits(pct.to_string())
                    ),
                };
                d.summary.set_text(&text);
            }
            taken
        };
        let Some(result) = done else { return };
        d.timer.stop();
        *d.job.borrow_mut() = None;
        self.dupes_busy(false);
        match result {
            Some(report) => {
                *d.report.borrow_mut() = report;
                let _ = self.dupes_show_report();
            }
            None => d.summary.set_text(tr!("أُوقف البحث.", "Search stopped.")),
        }
    }

    // ---------------------------------------------------------------- list

    /// Shows the report in the list; returns the totals line shown below it.
    fn dupes_show_report(&self) -> String {
        let d = &self.dupes;
        let report = d.report.borrow();
        let mut rows = Vec::with_capacity(report.file_count() + report.groups.len());
        for (g, group) in report.groups.iter().enumerate() {
            rows.push(Row::Group(g));
            rows.extend((0..group.files.len()).map(|f| Row::File(g, f)));
        }
        let n = rows.len();
        *d.rows.borrow_mut() = rows;
        for i in d.list.selected_items() {
            d.list.select_item(i, false);
        }
        d.list.set_item_count(n as u32);
        d.list.invalidate();
        let mut text = if report.groups.is_empty() {
            tr!("لا توجد ملفات مكررة.", "No duplicate files found.").to_string()
        } else {
            trf!(
                "{} مجموعة، {} ملف — المساحة المهدرة: {}",
                "{} groups, {} files — wasted space: {}",
                fmt_num(report.groups.len()),
                fmt_num(report.file_count()),
                size_text(report.wasted())
            )
        };
        if report.unreadable > 0 {
            text += &trf!(" (تعذرت قراءة {} ملف)", " ({} files could not be read)", fmt_num(report.unreadable));
        }
        d.summary.set_text(&text);
        text
    }

    fn dupes_cell(&self, row: usize, col: usize) -> String {
        let d = &self.dupes;
        let Some(&r) = d.rows.borrow().get(row) else { return String::new() };
        let report = d.report.borrow();
        match r {
            Row::Group(g) => {
                let Some(group) = report.groups.get(g) else { return String::new() };
                match col {
                    0 => trf!(
                        "المجموعة {}: {} نسخ",
                        "Group {}: {} copies",
                        fmt_num(g + 1),
                        fmt_num(group.files.len())
                    ),
                    1 => trf!("المهدر: {}", "Wasted: {}", size_text(group.wasted())),
                    _ => String::new(),
                }
            }
            Row::File(g, f) => {
                let Some((group, file)) = report.groups.get(g).and_then(|gr| Some((gr, gr.files.get(f)?))) else {
                    return String::new();
                };
                let (name, folder) = split_path(&file.path);
                match col {
                    0 => format!("    {name}"),
                    1 => ltr_path(folder),
                    2 => size_text(group.size),
                    3 => shell::format_time(file.mtime, true),
                    _ => String::new(),
                }
            }
        }
    }

    /// Paths of the selected file rows (group headings are skipped).
    fn dupes_selected_files(&self) -> Vec<String> {
        let d = &self.dupes;
        let rows = d.rows.borrow();
        let report = d.report.borrow();
        d.list
            .selected_items()
            .into_iter()
            .filter_map(|i| match rows.get(i)? {
                Row::File(g, f) => Some(report.groups.get(*g)?.files.get(*f)?.path.clone()),
                Row::Group(_) => None,
            })
            .collect()
    }

    fn dupes_open(&self, reveal: bool) {
        let Some(p) = self.dupes_selected_files().into_iter().next() else {
            self.dupes.summary.set_text(tr!("اختر ملفاً من القائمة أولاً.", "Select a file in the list first."));
            return;
        };
        if reveal {
            shell::reveal(&p);
        } else if !shell::open(&p) {
            self.dupes.summary.set_text(tr!("تعذر فتح الملف.", "Could not open the file."));
        }
    }

    /// Moves the selected files to the Recycle Bin after asking, keeping at
    /// least one copy of every group.
    fn dupes_delete(&self) {
        let d = &self.dupes;
        if d.job.borrow().is_some() {
            return;
        }
        let paths = self.dupes_selected_files();
        if paths.is_empty() {
            d.summary.set_text(tr!("اختر الملفات التي تريد حذفها أولاً.", "Select the files to delete first."));
            return;
        }
        let whole_group = d
            .report
            .borrow()
            .groups
            .iter()
            .position(|g| g.files.iter().all(|f| paths.contains(&f.path)));
        if let Some(g) = whole_group {
            nwg::modal_info_message(
                &d.window,
                title(),
                &trf!(
                    "حددت كل نسخ المجموعة {}. أبقِ نسخة واحدة على الأقل.",
                    "You selected every copy in group {}. Keep at least one copy.",
                    fmt_num(g + 1)
                ),
            );
            return;
        }
        if self.index_busy() {
            d.summary.set_text(tr!("انتظر انتهاء الفهرسة أو البحث داخل المستندات.", "Wait for indexing or the document search to finish."));
            return;
        }
        let shown: Vec<&str> = paths.iter().take(10).map(String::as_str).collect();
        let more = if paths.len() > 10 { trf!("\n… و{} غيرها", "\n… and {} more", paths.len() - 10) } else { String::new() };
        let text = trf!(
            "نقل {} ملف إلى سلة المحذوفات؟\n\n{}{}",
            "Move {} files to the Recycle Bin?\n\n{}{}",
            fmt_num(paths.len()),
            shown.join("\n"),
            more
        );
        let params = nwg::MessageParams {
            title: title(),
            content: &text,
            buttons: nwg::MessageButtons::YesNo,
            icons: nwg::MessageIcons::Question,
        };
        if nwg::modal_message(&d.window, &params) != nwg::MessageChoice::Yes {
            return;
        }
        let gone: Vec<String> = shell::recycle_many(&paths);
        if !gone.is_empty() {
            self.update_index(|idx| {
                for p in &gone {
                    idx.remove_path(p);
                }
            });
            let mut st = self.state.borrow_mut();
            for p in &gone {
                st.history.forget_path(p);
            }
            st.history.save();
        }
        dupes::forget_paths(&mut d.report.borrow_mut(), &gone);
        let totals = self.dupes_show_report();
        let left = paths.len() - gone.len();
        let msg = if left == 0 {
            trf!("نُقل {} ملف إلى سلة المحذوفات.", "Moved {} files to the Recycle Bin.", fmt_num(gone.len()))
        } else {
            trf!(
                "نُقل {} ملف إلى سلة المحذوفات، ولم يُحذف {}.",
                "Moved {} files to the Recycle Bin; {} were not deleted.",
                fmt_num(gone.len()),
                fmt_num(left)
            )
        };
        self.set_status(&msg);
        d.summary.set_text(&format!("{msg}  {totals}"));
    }
}
