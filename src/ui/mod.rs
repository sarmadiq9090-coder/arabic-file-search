//! The Windows user interface, built on native Win32 controls so Arabic text
//! is shaped, joined and laid out right-to-left by Windows itself.

//!
//! The code is split by job: each submodule adds an `impl App` block.
//! - `build`: creating the controls and dispatching their events
//! - `search_jobs`: loading and scanning the index, filename and document searches
//! - `results`: the result list, selection, copying, sorting and export
//! - `files`: renaming and deleting files, and saving the edited index
//! - `history_menu`: the menu of recent and saved searches
//! - `empty_state`: the files opened lately (empty box) and what to try when nothing is found
//! - `updates`: checking for, downloading and installing new versions
//! - `feedback`: the "Send feedback" menu and the offer to report a crash
//! - `rating`: the Store build's one-time rating question and "Rate in the Store" items
//! - `content_index`: the stored document text and its background warm-up
//! - `watching`: applying changes from the drive watcher to the index
//! - `keys`: keyboard handling and the global hotkey
//! - `quickswitch`: Ctrl+Alt+G moves an Open / Save As dialog to a result's folder
//! - `dupes_window`: the duplicate-file finder window
//! - `replace_window`: find and replace in the plain text files of the results
//! - `thumbs`: the thumbnail view of the results (Ctrl+T)
//! - `startup`: start-up timing, logging, single instance, fonts
//! - `lazy`: secondary windows created when they first open, for a quick start
//! - `quick`: the quick search popup (Ctrl+Alt+F, the floating box, the tray icon)
//! - `floating`: the small search box above the taskbar clock
//! - `tray`: the monochrome tray icon and menu, the new options, first-run question
//! - `surface`: drawing, theme and DWM helpers of those custom-drawn windows
//! - `format`, `shell`: text formatting and Windows shell helpers
//! - `webui`: the new main window (WebView2), opt-in while it is built

mod app_matches;
mod autostart;
mod build;
mod content_index;
mod dupes_window;
mod disk_usage;
mod empty_state;
mod feedback;
mod files;
mod floating;
mod format;
mod history_menu;
mod indexing;
mod lang_menu;
mod keys;
mod launcher_shim; // thin glue to crate::launcher, see the file
mod lazy;
mod quick;
mod quickswitch;
mod rating;
mod relocate;
mod replace_window;
mod results;
mod search_jobs;
mod shell;
mod startup;
mod style;
mod surface;
mod thumbs;
mod tray;
mod updates;
mod watching;
mod web_server;
mod webui;

use native_windows_gui as nwg;
use nwg::NativeUi;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
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

use files::SAVES;
use format::ARABIC_DIGITS;
use history_menu::HistoryMenu;
use keys::HOTKEY_ID;
use search_jobs::NameResult;
use startup::{LOOP_STARTED, activate_running_copy, phase, ui_font_family};
use updates::UpdateEvent;
use watching::SubtreeScan;

/// Mirrors the whole window so it reads right-to-left.
const WS_EX_LAYOUTRTL: u32 = 0x0040_0000;

/// The window title; also how a second copy of the program finds the first.
const TITLE: &str = "بحث سريع";
const TITLE_EN: &str = "InstaFind";

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
    /// `Some(None)` when the scan failed (a panic, logged).
    result: Arc<Mutex<Option<Option<Index>>>>,
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
    /// The background content-index warm-up: cancels it, and its progress.
    warm: Option<Arc<content_index::Warm>>,
    /// A filename search is running on a background thread.
    name_running: bool,
    /// The query changed while a search was running; search again when it ends.
    name_pending: bool,
    /// The query behind the current rows, used to underline the matches.
    shown_query: Option<Query>,
    /// Watches the drives so the index follows changes (see `watching`).
    watcher: Option<crate::watch::Watcher>,
    /// Watcher changes waiting until the index may change (limited in size).
    changes: crate::watch_pace::ChangeQueue,
    /// Folders being walked because their contents are unknown.
    subtree_scans: Vec<SubtreeScan>,
    /// Stats from the watcher, rescan pacing and index-copy pacing.
    live: watching::Live,
    /// The running filename search refreshes results after a change on
    /// disk, rather than answering the user's typing.
    live_search: bool,
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
    /// The folder searched, or "All drives"; long paths are shortened in the middle.
    scope_text: nwg::Label,
    scope_btn: nwg::Button,
    all_btn: nwg::Button,
    reindex_btn: nwg::Button,
    list: nwg::ListView,
    /// Apps the search box names strongly, above the results (hidden when none).
    app_btns: [nwg::Button; app_matches::APP_BUTTONS],
    /// The launcher results behind `app_btns`, in order.
    app_items: RefCell<Vec<crate::launcher::Item>>,
    open_btn: nwg::Button,
    folder_btn: nwg::Button,
    copy_btn: nwg::Button,
    export_btn: nwg::Button,
    stop_btn: nwg::Button,
    options_btn: nwg::Button,
    status: nwg::StatusBar,
    /// Button and check box widths fitted to their text, measured once.
    widths: Cell<Option<style::Widths>>,
    timer: nwg::AnimationTimer,
    /// Wakes the UI thread when a background filename search finishes.
    name_notice: nwg::Notice,
    name_result: Arc<Mutex<Option<NameResult>>>,
    /// Wakes the UI thread when an update check or download finishes.
    update_notice: nwg::Notice,
    update_result: Arc<Mutex<Option<UpdateEvent>>>,
    /// Wakes the UI thread when the drive watcher has a batch of changes.
    watch_notice: nwg::Notice,
    watch_inbox: Arc<Mutex<watching::Inbox>>,
    /// The saved index misses changes of the last session (its marker file
    /// was found): scan the drives again once it is loaded.
    reconcile: Cell<bool>,
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
    /// «جرّب الشكل الجديد»: only built when WebView2 is there (MSVC builds).
    menu_new_ui: nwg::MenuItem,
    menu_new_ui_sep: nwg::MenuSeparator,
    menu_hotkey: nwg::MenuItem,
    menu_digits: nwg::MenuItem,
    menu_content_index: nwg::MenuItem,
    menu_auto_index: nwg::MenuItem,
    menu_hide_system: nwg::MenuItem,
    menu_update: nwg::MenuItem,
    menu_autostart: nwg::MenuItem,
    lang_menu: nwg::Menu,
    menu_lang_ar: nwg::MenuItem,
    menu_lang_en: nwg::MenuItem,
    /// Options ▸ "Send feedback": report a problem, suggest an idea, discuss.
    feedback_menu: nwg::Menu,
    menu_feedback_bug: nwg::MenuItem,
    menu_feedback_idea: nwg::MenuItem,
    menu_feedback_talk: nwg::MenuItem,
    /// Options ▸ «الدعم» / "Support": the support address and page.
    support_menu: nwg::Menu,
    menu_support_mail: nwg::MenuItem,
    menu_support_page: nwg::MenuItem,
    /// The crash reports of the last run were already offered (or none).
    crash_checked: Cell<bool>,
    menu_export: nwg::MenuItem,
    menu_hide: nwg::MenuItem,
    menu_exit: nwg::MenuItem,
    tray_icon: nwg::Icon,
    tray: nwg::TrayNotification,
    tray_menu: nwg::Menu,
    tray_quick: nwg::MenuItem,
    tray_floating: nwg::MenuItem,
    tray_show: nwg::MenuItem,
    tray_settings: nwg::MenuItem,
    tray_about: nwg::MenuItem,
    tray_exit: nwg::MenuItem,
    /// The tray icon in white and in black, for dark and light taskbars.
    mono_icons: tray::MonoIcons,
    /// Options ▸ "Search from anywhere".
    quick_menu: tray::QuickMenu,
    /// The quick search popup (built when it first opens).
    popup: quick::Popup,
    /// What to try when a search finds nothing, above the result list.
    suggest: empty_state::SuggestBar,
    /// The Store build's one-time rating question above the result list.
    rating: rating::RatingBar,
    /// The search box above the clock.
    floating: floating::Floating,
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
    /// "Exit" was chosen: closing the window really quits.
    quitting: Cell<bool>,
    startup_logged: Cell<bool>,
    /// The indexing progress shown in the status bar's last part, if any.
    indexing_shown: RefCell<Option<String>>,
    /// The offer to move a copy out of Downloads or a temp folder was
    /// already considered this run (see `relocate`).
    move_checked: Cell<bool>,
    state: RefCell<State>,
    /// Moving another program's Open / Save As dialog to a result's folder.
    quick: quickswitch::QuickSwitch,
    /// The duplicate-file finder window.
    dupes: dupes_window::Dupes,
    /// Find and replace across the plain text files of the results.
    replace: replace_window::ReplaceWindow,
    /// Folder sizes: the Size column of folders and the folder-sizes window.
    disk: disk_usage::DiskUsage,
    /// Searching from a phone over the home network (off by default).
    web: web_server::WebUi,
    /// The thumbnail view of the results (Ctrl+T).
    thumbs: thumbs::Thumbs,
    /// Event handlers of the windows created on first use.
    lazy_handlers: lazy::LazyHandlers,
    /// The new main window, when it replaces this one (see `webui`).
    #[cfg_attr(not(target_env = "msvc"), allow(dead_code))]
    webui: webui::WebUi,
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

    /// The widths of the buttons and the check box, measured on first use.
    fn widths(&self) -> style::Widths {
        if let Some(w) = self.widths.get() {
            return w;
        }
        let f = &self.font;
        let b = |btn: &nwg::Button| style::button_width(f, btn);
        let w = style::Widths {
            search: b(&self.search_btn),
            history: b(&self.history_btn),
            check: style::check_width(f, &self.content_chk),
            scope_label: style::text_width(f, &self.scope_label.text()) + 6,
            scope_btn: b(&self.scope_btn),
            all_btn: b(&self.all_btn),
            reindex: b(&self.reindex_btn),
            bottom: [b(&self.open_btn), b(&self.folder_btn), b(&self.copy_btn), b(&self.export_btn), b(&self.stop_btn)],
            options: b(&self.options_btn),
        };
        // The narrowest window in which nothing overlaps: the bottom button
        // row, or the scope row when it has a row of its own.
        use style::{GAP, MARGIN};
        // (Export, also in the Options menu, steps aside when there is no room.)
        let bottom = w.bottom.iter().sum::<i32>() - w.bottom[3] + w.options + 5 * GAP + 2 * MARGIN;
        let scope_row =
            w.scope_label + style::MIN_SCOPE_W + w.scope_btn + w.all_btn + w.reindex + 4 * GAP + 2 * MARGIN;
        let filter_row = style::KIND_W + w.check + 3 * GAP + 2 * MARGIN;
        style::set_min_width(bottom.max(scope_row).max(filter_row));
        self.widths.set(Some(w));
        w
    }

    /// Shows the folder searched (or "All drives"); the "All drives" button
    /// only appears while a folder is chosen.
    fn show_scope(&self, scope: &str) {
        if scope.is_empty() {
            self.scope_text.set_text(tr!("كل الأقراص", "All drives"));
        } else {
            self.scope_text.set_text(&format::ltr_path(scope));
        }
        self.all_btn.set_visible(!scope.is_empty());
        self.layout();
    }

    fn layout(&self) {
        use style::{CTRL_H, GAP, KIND_W, MARGIN as M, MIN_SCOPE_W, QUERY_H};
        let (w, h) = self.window.size();
        let (w, h) = (w as i32, h as i32);
        let wd = self.widths();

        // Row 1: a large search box, then History and Search, all one height.
        let right = w - M;
        style::place(&self.search_btn.handle, right - wd.search, M, wd.search, QUERY_H);
        let hist_x = right - wd.search - GAP - wd.history;
        style::place(&self.history_btn.handle, hist_x, M, wd.history, QUERY_H);
        style::place(&self.query.handle, M, M, (hist_x - GAP - M).max(100), QUERY_H);
        // The box centres its text with a frame area; repaint that too.
        style::repaint_frame(&self.query.handle);

        // Row 2: file type and the content check box, then where to search
        // and the index button. When the window is narrow the scope gets a
        // row of its own.
        let y2 = M + QUERY_H + GAP + 2;
        // A drop-down's height is that of its open list; the closed box
        // keeps the height Windows gives it for its font, centred in the row.
        let combo_h = style::height_of(&self.kind_combo.handle).unwrap_or(CTRL_H).min(CTRL_H);
        style::place(&self.kind_combo.handle, M, y2 + (CTRL_H - combo_h) / 2, KIND_W, 320);
        let chk_x = M + KIND_W + GAP + 4;
        style::place(&self.content_chk.handle, chk_x, y2, wd.check, CTRL_H);

        let all_visible = self.all_btn.visible();
        let buttons_w = wd.scope_btn + wd.reindex + GAP + if all_visible { wd.all_btn + GAP } else { 0 };
        let scope_start = chk_x + wd.check + 2 * GAP;
        let (y_scope, scope_x) = if right - buttons_w - GAP - wd.scope_label - MIN_SCOPE_W >= scope_start {
            (y2, scope_start)
        } else {
            (y2 + CTRL_H + GAP, M)
        };
        // The labels are exactly one line tall and centred in the row, so
        // they have no frame area left to paint.
        let line_h = style::line_height(&self.font).min(CTRL_H);
        let y_text = y_scope + (CTRL_H - line_h) / 2;
        style::place(&self.scope_label.handle, scope_x, y_text, wd.scope_label, line_h);
        let text_x = scope_x + wd.scope_label;
        let mut x = right - buttons_w;
        style::place(&self.scope_text.handle, text_x, y_text, (x - GAP - text_x).max(40), line_h);
        for (b, bw, shown) in [
            (&self.scope_btn, wd.scope_btn, true),
            (&self.all_btn, wd.all_btn, all_visible),
            (&self.reindex_btn, wd.reindex, true),
        ] {
            if shown {
                style::place(&b.handle, x, y_scope, bw, CTRL_H);
                x += bw + GAP;
            }
        }

        // Bottom: action buttons above the status bar, options at the far end.
        let status_h = self.status_height();
        let y_btn = h - status_h - GAP - CTRL_H;
        let mut x = M;
        // Export results is also in the Options menu, so in a narrow window
        // it gives up its place rather than overlap Options.
        let export_fits = wd.bottom.iter().sum::<i32>() + wd.options + 6 * GAP + 2 * M <= w;
        self.export_btn.set_visible(export_fits);
        let bottom = [&self.open_btn, &self.folder_btn, &self.copy_btn, &self.export_btn, &self.stop_btn];
        for (i, (b, bw)) in bottom.into_iter().zip(wd.bottom).enumerate() {
            if i == 3 && !export_fits {
                continue;
            }
            style::place(&b.handle, x, y_btn, bw, CTRL_H);
            x += bw + GAP;
        }
        style::place(&self.options_btn.handle, right - wd.options, y_btn, wd.options, CTRL_H);

        // The result list fills the rest.
        let mut y3 = y_scope + CTRL_H + GAP + 2;
        // Apps the query names, in a row of their own above the results.
        let apps = self.app_buttons_shown();
        if apps > 0 {
            let mut x = M;
            for b in &self.app_btns[..apps] {
                let bw = style::button_width(&self.font, b).min(((w - 2 * M) / apps as i32 - GAP).max(40));
                style::place(&b.handle, x, y3, bw, CTRL_H);
                x += bw + GAP;
            }
            y3 += CTRL_H + GAP;
        }
        let y3 = self.place_suggestions(M, y3, w - 2 * M);
        // The Store rating question goes under the list, so its top rows stay put.
        let list_bottom = self.place_rating(M, y_btn - GAP, w - 2 * M);
        style::place(&self.list.handle, M, y3, (w - 2 * M).max(100), (list_bottom - y3).max(50));

        // Status bar: messages on one side, the selected file's path on the
        // other, and while indexing runs its progress at the far end.
        if let Some(sb) = self.status.handle.hwnd() {
            let px = (w as f64 * nwg::scale_factor()) as i32;
            if self.indexing_part_shown() {
                let parts: [i32; 3] = [px * 9 / 20, px * 3 / 4, -1];
                unsafe { SendMessageW(sb, SB_SETPARTS, 3, parts.as_ptr() as LPARAM) };
            } else {
                let parts: [i32; 2] = [px / 2, -1];
                unsafe { SendMessageW(sb, SB_SETPARTS, 2, parts.as_ptr() as LPARAM) };
            }
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
        if self.state.borrow().settings.tray_icon && !self.tray_hint_shown.replace(true) {
            self.tray.show(
                tr!("البرنامج ما زال يعمل هنا. انقر الأيقونة أو شغّل البرنامج مرة أخرى لإظهاره فوراً.", "The program is still running here. Click the icon or start the program again to show it at once."),
                Some(title()),
                None,
                None,
            );
        }
    }

    /// Whether the window's close button only hides it: there is a way back
    /// (the icon by the clock or the floating box), and "Exit" was not chosen.
    fn close_hides(&self) -> bool {
        if self.quitting.get() {
            return false;
        }
        let s = &self.state.borrow().settings;
        s.tray_icon || s.floating_mode != crate::settings::FloatingMode::Off
    }

    /// Really closes the program (the tray menu's and Options menu's "Exit").
    fn quit(&self) {
        startup::log("exit chosen from the menu");
        self.quitting.set(true);
        self.window.close();
    }

    /// Whether the main window is on screen: the classic one, or the new
    /// one when it replaces it (the first-run offers wait for this).
    fn main_shown(&self) -> bool {
        self.window.visible() || self.webui_visible()
    }

    fn show_window(&self) {
        if self.webui_show() {
            return;
        }
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
        // The icon view has no columns to measure.
        if let Some(h) = self.list.handle.hwnd().filter(|_| !self.thumbs_on()) {
            let scale = nwg::scale_factor();
            s.columns = (0..COLUMNS.len())
                .map(|i| (unsafe { SendMessageW(h, LVM_GETCOLUMNWIDTH, i, 0) } as f64 / scale).round() as i32)
                .collect();
        }
        s.save();
    }

    fn on_close(&self) {
        self.stop_watching();
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
        self.unregister_alt_space();
        self.hide_quick();
        self.webui_close();
        // Gone at once, not left on screen while the index is freed.
        self.destroy_floating();
        // The index is written last, by the saving thread, with every window
        // already off the screen; this thread only waits for it, and only
        // for `EXIT_SAVE_WAIT`. A save not done by then (a huge index, a
        // disk that is busy or stuck) is given up, which leaves `index.bin`
        // as the last finished save wrote it. Before 0.10.1 the UI thread
        // wrote it here itself, behind every save that was queued, for as
        // long as that took, with the window frozen.
        self.window.set_visible(false);
        let started = std::time::Instant::now();
        let outcome = SAVES.flush_within(EXIT_SAVE_WAIT);
        if outcome != crate::index_save::Flush::Nothing {
            startup::log(&format!("exit: last index save {outcome:?} after {} ms", started.elapsed().as_millis()));
        }
        nwg::stop_thread_dispatch();
    }
}

/// How long leaving the program waits for the index to be written.
const EXIT_SAVE_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

pub fn run() {
    crate::search::init_local_utc_offset();
    let updated = std::env::args().any(|a| a == "--updated");
    let moved = std::env::args().any(|a| a == crate::relocate::MOVED_ARG);
    if moved {
        // Started from the new place by the copy that is now exiting.
        for _ in 0..6 {
            crate::update::wait_for_previous_instance_exit();
        }
        startup::log("start: relaunched from the permanent place");
    }
    if updated {
        // The previous version is still closing; let it go first. A big
        // index can take a while to free, so wait up to a minute.
        for _ in 0..6 {
            crate::update::wait_for_previous_instance_exit();
        }
        startup::log(&format!("start: relaunched after updating to {}", env!("CARGO_PKG_VERSION")));
    }
    if activate_running_copy(updated || moved) {
        return;
    }
    // Only the copy that keeps running tidies up after an update.
    crate::update::cleanup_old();
    // Temp files of index saves that never finished (a copy closed or
    // killed in the middle), unless written to in the last two minutes.
    let stale = crate::index::remove_stale_tmp(&crate::index::cache_path(), std::time::Duration::from_secs(120));
    if stale > 0 {
        startup::log(&format!("start: removed {stale} unfinished index save file(s)"));
    }
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
