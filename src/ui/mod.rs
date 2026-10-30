//! The Windows user interface, built on native Win32 controls so Arabic text
//! is shaped, joined and laid out right-to-left by Windows itself.

//!
//! The code is split by job: each submodule adds an `impl App` block.
//! - `build`: creating the controls and dispatching their events
//! - `search_jobs`: loading and scanning the index, filename and document searches
//! - `results`: the result list, selection, copying, sorting and export
//! - `files`: renaming and deleting files, and saving the edited index
//! - `history_menu`: the menu of recent and saved searches
//! - `updates`: checking for, downloading and installing new versions
//! - `content_index`: the stored document text and its background warm-up
//! - `keys`: keyboard handling and the global hotkey
//! - `quickswitch`: Ctrl+Alt+G moves an Open / Save As dialog to a result's folder
//! - `startup`: start-up timing, logging, single instance, fonts
//! - `format`, `shell`: text formatting and Windows shell helpers

mod autostart;
mod build;
mod content_index;
mod files;
mod format;
mod history_menu;
mod lang_menu;
mod keys;
mod quickswitch;
mod results;
mod search_jobs;
mod shell;
mod startup;
mod updates;

use native_windows_gui as nwg;
use nwg::NativeUi;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use winapi::shared::minwindef::LPARAM;
use winapi::shared::windef::{POINT, RECT};
use winapi::um::commctrl::{LVM_GETCOLUMNWIDTH, SB_SETPARTS};
use winapi::um::winuser::{
    GetCursorPos, GetWindowRect, IsIconic, IsZoomed, SW_RESTORE, SendMessageW, SetForegroundWindow, ShowWindow,
};

use crate::history::History;
use crate::index::{Entry, Index, ScanProgress};
use crate::search::{ContentHit, ContentProgress, Query};
use crate::settings::Settings;

use files::flush_index_save;
use format::ARABIC_DIGITS;
use history_menu::HistoryMenu;
use keys::HOTKEY_ID;
use search_jobs::NameResult;
use startup::{LOOP_STARTED, activate_running_copy, phase, ui_font_family};
use updates::UpdateEvent;

/// Mirrors the whole window so it reads right-to-left.
const WS_EX_LAYOUTRTL: u32 = 0x0040_0000;

/// The window title; also how a second copy of the program finds the first.
const TITLE: &str = "بحث سريع";
const TITLE_EN: &str = "Bahth Sari'";

/// The window title in the interface language.
fn title() -> &'static str {
    tr!(TITLE, TITLE_EN)
}

const COL_NAME: usize = 0;
const COL_PATH: usize = 1;
const COL_SIZE: usize = 2;
const COL_DATE: usize = 3;
const COL_SNIPPET: usize = 4;
const COLUMNS: [(&str, i32); 5] =
    [("الاسم", 260), ("المسار الكامل", 460), ("الحجم", 90), ("تاريخ التعديل", 140), ("النص المطابق", 360)];
const COLUMNS_EN: [&str; 5] = ["Name", "Full path", "Size", "Date modified", "Matching text"];

/// A result row: an index into `State::rows_index` plus an optional snippet.
struct Row {
    entry: u32,
    snippet: Option<String>,
    /// Every match inside the document (document search only).
    matches: Option<Box<Matches>>,
}

/// All the places a document matched, for the "all matches" window.
struct Matches {
    count: u32,
    /// (1-based line number, the line around its first match).
    lines: Vec<(u32, String)>,
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
    /// Cancels the background content-index warm-up.
    warm: Option<Arc<AtomicBool>>,
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

impl State {
    /// The index entry shown in result row `row`.
    fn row_entry(&self, row: usize) -> Option<&Entry> {
        self.rows_index.entries.get(self.rows.get(row)?.entry as usize)
    }
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
    menu_matches: nwg::MenuItem,
    /// Lists every matching line of one document.
    matches_window: nwg::Window,
    matches_text: nwg::TextBox,
    options_menu: nwg::Menu,
    menu_hotkey: nwg::MenuItem,
    menu_digits: nwg::MenuItem,
    menu_content_index: nwg::MenuItem,
    menu_update: nwg::MenuItem,
    menu_autostart: nwg::MenuItem,
    lang_menu: nwg::Menu,
    menu_lang_ar: nwg::MenuItem,
    menu_lang_en: nwg::MenuItem,
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
    /// Moving another program's Open / Save As dialog to a result's folder.
    quick: quickswitch::QuickSwitch,
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

    fn show_menu(&self, menu: &nwg::Menu) {
        let mut pt = POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut pt) };
        menu.popup(pt.x, pt.y);
    }

    fn set_status(&self, text: &str) {
        self.status.set_text(0, text);
    }

    // ---------------------------------------------------------------- tray & options

    fn hide_to_tray(&self) {
        self.remember_query();
        self.window.set_visible(false);
        if !self.tray_hint_shown.replace(true) {
            self.tray.show(
                tr!("البرنامج ما زال يعمل هنا. انقر الأيقونة أو شغّل البرنامج مرة أخرى لإظهاره فوراً.", "The program is still running here. Click the icon or start the program again to show it at once."),
                Some(title()),
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
        self.stop_warm();
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

pub fn run() {
    crate::search::init_local_utc_offset();
    let updated = std::env::args().any(|a| a == "--updated");
    if updated {
        // The previous version is still closing; let it go first. A big
        // index can take a while to free, so wait up to a minute.
        for _ in 0..6 {
            crate::update::wait_for_previous_instance_exit();
        }
        startup::log(&format!("start: relaunched after updating to {}", env!("CARGO_PKG_VERSION")));
    }
    if activate_running_copy(updated) {
        return;
    }
    // Only the copy that keeps running tidies up after an update.
    crate::update::cleanup_old();
    autostart::apply_default();
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
            nwg::fatal_message(title(), &trf!("تعذر إنشاء الواجهة: {e}", "Could not create the window: {e}"));
        }
    };
    phase("loop");
    LOOP_STARTED.store(true, Ordering::Relaxed);
    nwg::dispatch_thread_events();
    drop(ui);
}
