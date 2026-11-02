//! The quick search popup ("search from anywhere"): a small borderless
//! window with one search box at the bottom and the results above it,
//! grouped as `crate::quick::arrange` orders them. Files come from the name
//! search over the loaded index, on a background thread so typing never
//! waits; apps, settings, open windows, answers, commands and web shortcuts
//! from `crate::launcher::Catalog`, built once in the background.
//!
//! The window is created the first time it opens and then only hidden, so
//! it opens again at once. It closes when it loses the focus.

use native_windows_gui as nwg;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Clears a flag when dropped, also when a panic unwinds past it.
struct Reset<'a>(&'a Cell<bool>);

impl Drop for Reset<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

/// How long a list of installed apps is used before it is made again.
const CATALOG_FRESH: Duration = Duration::from_secs(120);

use winapi::shared::basetsd::{DWORD_PTR, UINT_PTR};
use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
use winapi::shared::windef::{HBRUSH, HDC, HGDIOBJ, HICON, HWND, RECT};
use winapi::um::wingdi::{BitBlt, CreateSolidBrush, DeleteObject, FW_NORMAL, FW_SEMIBOLD, SRCCOPY, SetBkColor, SetTextColor};
use winapi::um::winuser::*;

use super::launcher_shim as shim;
use super::startup::{background_priority, log};
use super::surface::{self, Canvas, Font, KIND_POPUP, WM_APP_CATALOG, WM_APP_FILES};
use super::{App, shell, title};
use crate::feedback::{self, SystemInfo};
use crate::launcher::{self, Action, Catalog, windows::OpenWindow};
use crate::quick::{self, Anchor, FeedbackKind, Group, Icon, ListMetrics, Palette, Parts, Rect, Row, Slot, SlotKind, Target, px};

/// Width of the popup, logical px.
const WIDTH: i32 = 480;
/// The search strip at the bottom.
const STRIP_H: i32 = 52;
/// Opacity (of 255) of the list's tint over the acrylic backdrop.
const LIST_TINT: u8 = 0xA8;
/// At most this many file rows (the full window has the rest).
const FILE_ROWS: usize = 6;
/// Id of the search box among the popup's children.
const EDIT_ID: usize = 100;
/// A click on the tray icon right after the popup closed (by that very
/// click taking the focus) means "close", not "open again".
const REOPEN_GAP: Duration = Duration::from_millis(350);

fn metrics(scale: f64) -> ListMetrics {
    ListMetrics { row: px(40, scale), more: px(34, scale), header: px(26, scale), separator: px(9, scale), pad: px(6, scale) }
}

/// The popup's state, one field of `App`.
#[derive(Default)]
pub struct Popup {
    hwnd: Cell<usize>,
    edit: Cell<usize>,
    rows: RefCell<Vec<Row>>,
    slots: RefCell<Vec<Slot>>,
    /// Height of the list part (above the search strip), physical px.
    list_h: Cell<i32>,
    selected: Cell<Option<usize>>,
    /// Where the mouse was last seen, so a list that moves under a still
    /// cursor does not change the selection.
    mouse: Cell<(i32, i32)>,
    rect: Cell<Rect>,
    work: Cell<Rect>,
    /// It opened upwards (its bottom edge stays put as it grows).
    upward: Cell<bool>,
    windows: RefCell<Vec<OpenWindow>>,
    files: RefCell<Files>,
    file_slot: Arc<Mutex<Option<FileResult>>>,
    file_running: Cell<bool>,
    file_pending: Cell<bool>,
    /// Enter was pressed before the file results came; open when they do.
    enter_pending: Cell<bool>,
    catalog: Arc<Mutex<Option<Arc<Catalog>>>>,
    /// When the catalog was last (re)built, to list new apps now and then.
    catalog_started: Cell<Option<Instant>>,
    /// The popup's HWND for background threads to post to (0 = none yet).
    post_to: Arc<AtomicUsize>,
    look: RefCell<Look>,
    /// Shell icons by path or extension.
    icons: RefCell<HashMap<String, usize>>,
    /// A menu or message box of the popup is open: losing the focus to it
    /// must not close the popup.
    busy: Cell<u32>,
    /// Inside `show_quick` or `hide_quick`: a nested call (Windows sends
    /// activation messages while the popup is shown or hidden) does nothing.
    toggling: Cell<bool>,
    hidden_at: Cell<Option<Instant>>,
}

/// File rows of the latest finished search.
#[derive(Default)]
struct Files {
    query: String,
    rows: Vec<Row>,
    done: bool,
}

struct FileResult {
    query: String,
    rows: Vec<(String, bool)>,
    /// False while the index is still loading: "nothing found" isn't final.
    done: bool,
}

/// Fonts, colours and the backdrop choice for the current theme.
struct Look {
    dark: bool,
    backdrop: bool,
    scale: f64,
    palette: Palette,
    title: Font,
    small: Font,
    header: Font,
    glyph: Font,
    edit_font: Font,
    brush: usize,
}

impl Default for Look {
    fn default() -> Look {
        Look {
            dark: false,
            backdrop: false,
            scale: 1.0,
            palette: Palette::new(false),
            title: Font::default(),
            small: Font::default(),
            header: Font::default(),
            glyph: Font::default(),
            edit_font: Font::default(),
            brush: 0,
        }
    }
}

impl Drop for Look {
    fn drop(&mut self) {
        if self.brush != 0 {
            unsafe { DeleteObject(self.brush as HGDIOBJ) };
        }
    }
}

impl Drop for Popup {
    fn drop(&mut self) {
        for (_, icon) in self.icons.get_mut().drain() {
            unsafe { DestroyIcon(icon as HICON) };
        }
    }
}

fn rtl() -> bool {
    !crate::i18n::is_en()
}

/// `GET_X_LPARAM` / `GET_Y_LPARAM`.
pub(super) fn point_of(l: LPARAM) -> (i32, i32) {
    let v = l as usize as u32;
    ((v & 0xFFFF) as u16 as i16 as i32, (v >> 16) as u16 as i16 as i32)
}

fn window_text(hwnd: HWND) -> String {
    let n = unsafe { GetWindowTextLengthW(hwnd) }.max(0) as usize;
    let mut buf = vec![0u16; n + 1];
    let got = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) }.max(0) as usize;
    String::from_utf16_lossy(&buf[..got.min(n)])
}

/// `ShellExecuteW` with a verb (`None` = the default, "runas" = as
/// administrator) and arguments. True when it started.
fn shell_execute(verb: Option<&str>, file: &str, args: &str) -> bool {
    use winapi::um::shellapi::ShellExecuteW;
    let verb = verb.map(shell::wide);
    let (file, args) = (shell::wide(file), shell::wide(args));
    let r = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()),
            file.as_ptr(),
            if args.len() > 1 { args.as_ptr() } else { std::ptr::null() },
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    r as isize > 32
}

/// A classic tool named without a folder ("taskmgr", "devmgmt.msc", the
/// settings table's non-`ms-settings:` pages) as its file in the Windows
/// system folder, when it is there. `ShellExecute` looks in the current
/// directory first, which for a portable copy started from Downloads is
/// Downloads, where a program of that name could have been planted.
fn system_tool(target: &str) -> String {
    if target.contains(['\\', '/', ':']) {
        return target.to_string();
    }
    let mut buf = [0u16; 260];
    let n = unsafe { winapi::um::sysinfoapi::GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as UINT) } as usize;
    if n == 0 || n >= buf.len() {
        return target.to_string();
    }
    let name = if std::path::Path::new(target).extension().is_some() { target.to_string() } else { format!("{target}.exe") };
    let path = std::path::PathBuf::from(String::from_utf16_lossy(&buf[..n])).join(name);
    if path.is_file() { path.to_string_lossy().into_owned() } else { target.to_string() }
}

/// Whether `path` is something Windows can start as administrator.
fn runs_as_admin(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    !p.starts_with("shell:") && [".exe", ".lnk", ".bat", ".cmd", ".msc"].iter().any(|e| p.ends_with(e))
}

/// Today's local date, for the Hijri answer.
fn today() -> (i32, u32, u32) {
    let mut t: winapi::um::minwinbase::SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { winapi::um::sysinfoapi::GetLocalTime(&mut t) };
    (t.wYear as i32, t.wMonth as u32, t.wDay as u32)
}

/// The search box's subclass: arrows, Enter, Tab and Esc work the list
/// instead of the box. `WM_GETDLGCODE` keeps the dialog-style message loop
/// from taking those keys first.
unsafe extern "system" fn edit_proc(hwnd: HWND, msg: UINT, w: WPARAM, l: LPARAM, _id: UINT_PTR, _data: DWORD_PTR) -> LRESULT {
    use winapi::um::commctrl::DefSubclassProc;
    if msg == WM_GETDLGCODE {
        return unsafe { DefSubclassProc(hwnd, msg, w, l) } | DLGC_WANTALLKEYS as LRESULT;
    }
    let answer = crate::crash_log::catch(|| surface::app()?.popup_edit_key(msg, w)).flatten();
    match answer {
        Some(r) => r,
        None => unsafe { DefSubclassProc(hwnd, msg, w, l) },
    }
}

impl App {
    fn popup_hwnd(&self) -> HWND {
        self.popup.hwnd.get() as HWND
    }

    fn popup_edit(&self) -> HWND {
        self.popup.edit.get() as HWND
    }

    /// Whether the popup is on screen.
    pub(super) fn quick_visible(&self) -> bool {
        let h = self.popup_hwnd();
        !h.is_null() && unsafe { IsWindowVisible(h) } != 0
    }

    /// Creates the (hidden) popup once. False when Windows refused.
    fn ensure_popup(&self) -> bool {
        if self.popup.hwnd.get() != 0 {
            return true;
        }
        let started = Instant::now();
        let Some(hwnd) = surface::create_window(KIND_POPUP, WS_EX_TOOLWINDOW | WS_EX_TOPMOST, title()) else {
            log("quick search: could not create the window");
            return false;
        };
        let class = shell::wide("EDIT");
        let empty = shell::wide("");
        let (style, ex) = if rtl() {
            (ES_RIGHT, WS_EX_RTLREADING | WS_EX_RIGHT | WS_EX_LEFTSCROLLBAR)
        } else {
            (ES_LEFT, 0)
        };
        let edit = unsafe {
            CreateWindowExW(
                ex,
                class.as_ptr(),
                empty.as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL | style,
                0,
                0,
                0,
                0,
                hwnd,
                EDIT_ID as winapi::shared::windef::HMENU,
                winapi::um::libloaderapi::GetModuleHandleW(std::ptr::null()),
                std::ptr::null_mut(),
            )
        };
        if edit.is_null() {
            log("quick search: could not create the search box");
            unsafe { DestroyWindow(hwnd) };
            return false;
        }
        unsafe {
            winapi::um::commctrl::SetWindowSubclass(edit, Some(edit_proc), 1, 0);
            let cue = shell::wide(tr!("ابحث...", "Search..."));
            SendMessageW(edit, winapi::um::commctrl::EM_SETCUEBANNER, 1, cue.as_ptr() as LPARAM);
        }
        self.popup.hwnd.set(hwnd as usize);
        self.popup.edit.set(edit as usize);
        self.popup.post_to.store(hwnd as usize, Ordering::Relaxed);
        self.popup_apply_theme();
        log(&format!("quick search window built in {} ms", started.elapsed().as_millis()));
        true
    }

    /// Fonts, colours and the DWM look for the current Windows theme.
    fn popup_apply_theme(&self) {
        let hwnd = self.popup_hwnd();
        if hwnd.is_null() {
            return;
        }
        let build = surface::windows_build();
        let hc = surface::high_contrast();
        let dark = surface::apps_dark() && !hc;
        let backdrop = quick::has_backdrop(build) && surface::transparency_on() && !hc;
        let scale = nwg_scale();
        let ui = surface::ui_family();
        let palette = Palette::new(dark);
        let smooth = backdrop;
        let look = Look {
            dark,
            backdrop,
            scale,
            palette,
            title: Font::new(ui, px(14, scale), FW_NORMAL, smooth),
            small: Font::new(ui, px(12, scale), FW_NORMAL, smooth),
            header: Font::new(ui, px(12, scale), FW_SEMIBOLD, smooth),
            glyph: Font::new(surface::icon_family().unwrap_or(ui), px(16, scale), FW_NORMAL, smooth),
            edit_font: Font::new(ui, px(17, scale), FW_NORMAL, false),
            brush: unsafe { CreateSolidBrush(surface::bgr(palette.background)) } as usize,
        };
        surface::dwm_style(hwnd, dark, quick::has_round_corners(build), backdrop);
        unsafe { SendMessageW(self.popup_edit(), WM_SETFONT, look.edit_font.0 as WPARAM, 1) };
        *self.popup.look.borrow_mut() = look;
    }

    /// Opens the quick search at `anchor`, or closes it if it is open and
    /// in front (the hotkey and the box toggle it).
    pub(super) fn show_quick(&self, anchor: Anchor) {
        if self.popup.toggling.replace(true) {
            return;
        }
        let _reset = Reset(&self.popup.toggling);
        self.show_quick_inner(anchor);
    }

    fn show_quick_inner(&self, anchor: Anchor) {
        if !self.ensure_popup() {
            return;
        }
        let hwnd = self.popup_hwnd();
        if self.quick_visible() {
            if unsafe { GetForegroundWindow() } == hwnd {
                self.hide_quick_inner();
                return;
            }
        } else if matches!(anchor, Anchor::Tray { .. }) && self.popup.hidden_at.get().is_some_and(|t| t.elapsed() < REOPEN_GAP) {
            return;
        }
        let started = Instant::now();
        {
            let look = self.popup.look.borrow();
            let changed = look.dark != (surface::apps_dark() && !surface::high_contrast()) || look.scale != nwg_scale();
            drop(look);
            if changed {
                self.popup_apply_theme();
            }
        }
        // The open windows, minus our own.
        let ours = [hwnd as isize, self.floating_hwnd() as isize, self.window.handle.hwnd().map_or(0, |h| h as isize)];
        *self.popup.windows.borrow_mut() = shim::list_windows().into_iter().filter(|w| !ours.contains(&w.hwnd)).collect();
        self.start_catalog();
        let work = match anchor {
            Anchor::Box(b) => surface::monitor_at(b.center().0, b.center().1).1,
            Anchor::CenterBottom => surface::active_monitor().1,
            Anchor::Tray { .. } => surface::primary_monitor().1,
        };
        self.popup.work.set(work);
        let scale = self.popup.look.borrow().scale;
        // A first guess of the size; the rows set the real height.
        let rect = quick::popup_rect(anchor, &work, px(WIDTH, scale), px(STRIP_H, scale), scale);
        self.popup.upward.set(!matches!(anchor, Anchor::Box(b) if b.center().1 < work.center().1));
        self.popup.rect.set(rect);
        self.popup.selected.set(None);
        self.popup.enter_pending.set(false);
        // Where the cursor is now: only a real move selects a row.
        let mut cur = winapi::shared::windef::POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut cur) };
        self.popup.mouse.set((cur.x, cur.y));
        // Clearing the box runs the (empty) search, which sizes the window.
        let empty = shell::wide("");
        unsafe { SetWindowTextW(self.popup_edit(), empty.as_ptr()) };
        self.popup_text_changed();
        unsafe {
            ShowWindow(hwnd, SW_SHOW);
            surface::bring_to_front(hwnd);
            SetFocus(self.popup_edit());
        }
        self.wake_floating(true);
        log(&format!("quick search shown in {} ms", started.elapsed().as_millis()));
    }

    pub(super) fn hide_quick(&self) {
        if self.popup.toggling.replace(true) {
            return;
        }
        let _reset = Reset(&self.popup.toggling);
        self.hide_quick_inner();
    }

    fn hide_quick_inner(&self) {
        let hwnd = self.popup_hwnd();
        if hwnd.is_null() || unsafe { IsWindowVisible(hwnd) } == 0 {
            return;
        }
        unsafe { ShowWindow(hwnd, SW_HIDE) };
        self.popup.hidden_at.set(Some(Instant::now()));
        self.popup.enter_pending.set(false);
        self.wake_floating(false);
    }

    /// Builds the launcher catalog (installed apps take a moment to list)
    /// on a background thread. Built again when the popup opens and the list
    /// is over [`CATALOG_FRESH`] old, so programs installed since show up;
    /// the old list serves until the new one is ready.
    pub(super) fn start_catalog(&self) {
        if self.popup.catalog_started.get().is_some_and(|t| t.elapsed() < CATALOG_FRESH) {
            return;
        }
        self.popup.catalog_started.set(Some(Instant::now()));
        let slot = self.popup.catalog.clone();
        let post_to = self.popup.post_to.clone();
        crate::crash_log::spawn_named("launcher-catalog", move || {
            background_priority();
            let started = Instant::now();
            let catalog = crate::crash_log::catch(shim::new_catalog).unwrap_or_default();
            log(&format!(
                "launcher catalog: {} apps, {} settings pages in {} ms",
                catalog.apps.len(),
                catalog.settings.len(),
                started.elapsed().as_millis()
            ));
            if let Ok(mut s) = slot.lock() {
                *s = Some(Arc::new(catalog));
            }
            let h = post_to.load(Ordering::Relaxed);
            if h != 0 {
                unsafe { PostMessageW(h as HWND, WM_APP_CATALOG, 0, 0) };
            }
        });
    }

    // ---------------------------------------------------------------- searching

    fn popup_query(&self) -> String {
        window_text(self.popup_edit())
    }

    fn popup_text_changed(&self) {
        self.popup.enter_pending.set(false);
        let q = self.popup_query();
        self.popup_edit_direction(&q);
        if q.trim().is_empty() {
            *self.popup.files.borrow_mut() = Files::default();
        } else {
            self.popup_search_files(q);
        }
        self.popup_refresh();
    }

    /// Gives the search box the reading order of what is typed ("250*12"
    /// or "5 kg to lb" left to right in the Arabic interface, Arabic right
    /// to left in the English one); the interface's own when it is empty.
    fn popup_edit_direction(&self, text: &str) {
        let edit = self.popup_edit();
        let want = if text.trim().is_empty() { rtl() } else { quick::first_strong_rtl(text).unwrap_or(false) };
        let ex = unsafe { GetWindowLongW(edit, GWL_EXSTYLE) } as u32;
        let has = ex & WS_EX_RTLREADING != 0;
        if has != want {
            let ex = if want { ex | WS_EX_RTLREADING } else { ex & !WS_EX_RTLREADING };
            unsafe {
                SetWindowLongW(edit, GWL_EXSTYLE, ex as i32);
                InvalidateRect(edit, std::ptr::null(), 1);
            }
        }
    }

    /// Starts the name search for `text` on a background thread; while one
    /// runs, later keystrokes fold into one more search when it ends.
    fn popup_search_files(&self, text: String) {
        if text.trim().is_empty() {
            return;
        }
        if self.popup.file_running.get() {
            self.popup.file_pending.set(true);
            return;
        }
        let st = self.state.borrow();
        let index = st.index.clone();
        let boosted = st.history.boosted(&index, crate::index::now_secs());
        let show_system = crate::system_files::show_system(st.settings.hide_system, "");
        let done = st.loading.is_none() && !(st.scan.is_some() && index.entries.is_empty());
        drop(st);
        let slot = self.popup.file_slot.clone();
        let post_to = self.popup.post_to.clone();
        self.popup.file_running.set(true);
        self.popup.file_pending.set(false);
        let started = crate::crash_log::spawn_named("quick-search", move || {
            let parse = move |t: &str| {
                let mut q = crate::search::Query::parse(t);
                q.show_system |= show_system;
                q
            };
            let run = || {
                let mut hits = crate::search::search_names(&index.entries, &parse(&text), "");
                if hits.is_empty()
                    && let Some((alt, _)) = crate::fuzzy::name_fallback(&index.entries, &text, "", &parse)
                {
                    hits = alt;
                }
                crate::search::promote(&mut hits, &boosted);
                hits.iter().take(FILE_ROWS).filter_map(|&i| index.entries.get(i as usize)).map(|e| (e.path.to_string(), e.is_dir)).collect()
            };
            let rows = crate::crash_log::catch(run).unwrap_or_default();
            if let Ok(mut s) = slot.lock() {
                *s = Some(FileResult { query: text, rows, done });
            }
            let h = post_to.load(Ordering::Relaxed);
            if h != 0 {
                unsafe { PostMessageW(h as HWND, WM_APP_FILES, 0, 0) };
            }
        });
        if started.is_none() {
            self.popup.file_running.set(false);
        }
    }

    fn popup_files_found(&self) {
        self.popup.file_running.set(false);
        let Some(res) = self.popup.file_slot.lock().ok().and_then(|mut s| s.take()) else { return };
        let current = self.popup_query();
        if self.popup.file_pending.replace(false) && res.query != current {
            // Typed on meanwhile: show these for now and search again.
            self.popup_search_files(current.clone());
        }
        if !current.starts_with(res.query.trim_end()) && res.query != current {
            // Stale, and not even a start of what is typed now.
            return;
        }
        *self.popup.files.borrow_mut() = Files {
            done: res.done && res.query == current,
            query: res.query,
            rows: res.rows.iter().map(|(p, d)| Row::file(p, *d)).collect(),
        };
        self.popup_refresh();
        // The results of what is typed now (final or, while the index still
        // loads, what was found so far).
        if self.popup.enter_pending.get() && self.popup.files.borrow().query == current {
            self.popup.enter_pending.set(false);
            self.popup_open_selected(How::Open);
        }
    }

    /// Puts the list together again from the pins, the catalog and the file
    /// rows, keeps the selection on the same result, and resizes the popup.
    fn popup_refresh(&self) {
        let query = self.popup_query();
        let catalog = self.popup.catalog.lock().ok().and_then(|c| c.clone());
        let windows = self.popup.windows.borrow();
        let (pins, items) = {
            let st = self.state.borrow();
            let pins: Vec<Row> = st.history.pinned_matching(&query).into_iter().map(pin_row).collect();
            let items = match &catalog {
                Some(c) if !query.trim().is_empty() => {
                    let now = crate::index::now_secs();
                    let history = &st.history;
                    let launches = move |key: &str| history.launch_count(key, now);
                    let ctx = launcher::Context {
                        english: crate::i18n::is_en(),
                        windows_build: surface::windows_build(),
                        windows: &windows,
                        launches: &launches,
                        today: today(),
                    };
                    c.query(&query, &ctx, 40)
                }
                _ => Vec::new(),
            };
            (pins, items)
        };
        drop(windows);
        let (files, files_done) = {
            let f = self.popup.files.borrow();
            (f.rows.clone(), f.done && f.query == query)
        };
        let mut rows = quick::arrange(Parts { query: query.clone(), pins, items, files, files_done });
        for r in rows.iter_mut() {
            label_more_row(r);
        }
        let look = self.popup.look.borrow();
        let m = metrics(look.scale);
        let work = self.popup.work.get();
        let max_list = (work.height() - px(STRIP_H, look.scale) - px(24, look.scale)).max(m.row * 3);
        quick::fit_rows(&mut rows, &m, max_list);
        let groups: Vec<Group> = rows.iter().map(|r| r.group).collect();
        let (slots, mut list_h) = quick::layout_list(&groups, &m);
        if rows.is_empty() {
            // The hint line.
            list_h = px(44, look.scale);
        }
        let scale = look.scale;
        drop(look);
        let previous = self.popup.selected.get().and_then(|i| self.popup.rows.borrow().get(i).map(|r| r.target.clone()));
        let selected = quick::reselect(&rows, previous.as_ref());
        *self.popup.rows.borrow_mut() = rows;
        *self.popup.slots.borrow_mut() = slots;
        self.popup.selected.set(selected);
        self.popup.list_h.set(list_h);
        self.popup_resize(list_h + px(STRIP_H, scale));
    }

    /// Gives the popup `height` (keeping its anchored edge) and lays out the
    /// search box.
    fn popup_resize(&self, height: i32) {
        let hwnd = self.popup_hwnd();
        let look = self.popup.look.borrow();
        let s = look.scale;
        let r = quick::regrow(&self.popup.rect.get(), height, self.popup.upward.get(), &self.popup.work.get());
        self.popup.rect.set(r);
        let list_h = self.popup.list_h.get().min(r.height());
        unsafe {
            SetWindowPos(hwnd, HWND_TOPMOST, r.left, r.top, r.width(), r.height(), SWP_NOACTIVATE);
        }
        // The backdrop shows through the list; a solid Windows 11 popup still
        // extends one (opaque) row, which gets it DWM's shadow.
        if look.backdrop {
            surface::extend_frame(hwnd, list_h);
        } else if quick::has_round_corners(surface::windows_build()) {
            surface::extend_frame(hwnd, 1);
        }
        // The box: after the magnifier, one line of its font, centred in the strip.
        let edit_h = px(26, s);
        let x = px(48, s);
        let w = (r.width() - x - px(16, s)).max(px(40, s));
        let x = quick::mirror_x(x, w, r.width(), rtl());
        let y = list_h + (r.height() - list_h - edit_h) / 2;
        unsafe {
            SetWindowPos(self.popup_edit(), std::ptr::null_mut(), x, y, w, edit_h, SWP_NOZORDER | SWP_NOACTIVATE);
            InvalidateRect(hwnd, std::ptr::null(), 0);
        }
    }

    // ---------------------------------------------------------------- messages

    /// The popup's window procedure; `None` for the default handling.
    pub(super) fn popup_message(&self, hwnd: HWND, msg: UINT, w: WPARAM, l: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_PAINT => {
                self.popup_paint(hwnd);
                Some(0)
            }
            WM_ERASEBKGND => Some(1),
            WM_CTLCOLOREDIT => {
                let look = self.popup.look.borrow();
                let dc = w as HDC;
                unsafe {
                    SetTextColor(dc, surface::bgr(look.palette.text));
                    SetBkColor(dc, surface::bgr(look.palette.background));
                }
                (look.brush != 0).then_some(look.brush as HBRUSH as LRESULT)
            }
            WM_COMMAND => {
                let code = ((w >> 16) & 0xFFFF) as u16;
                if code == EN_CHANGE && l as HWND == self.popup_edit() {
                    self.popup_text_changed();
                    return Some(0);
                }
                None
            }
            WM_ACTIVATE => {
                if (w & 0xFFFF) as u16 == WA_INACTIVE && self.popup.busy.get() == 0 {
                    self.hide_quick();
                }
                None
            }
            WM_SETFOCUS => {
                unsafe { SetFocus(self.popup_edit()) };
                Some(0)
            }
            WM_MOUSEMOVE => {
                let p = point_of(l);
                // Compared on the screen: Windows also sends WM_MOUSEMOVE
                // when the popup grows or moves under a cursor that stays
                // put, and the row under it must not take the selection
                // from the best match then (Enter would open that row).
                let mut sp = winapi::shared::windef::POINT { x: 0, y: 0 };
                unsafe { GetCursorPos(&mut sp) };
                if self.popup.mouse.replace((sp.x, sp.y)) != (sp.x, sp.y) {
                    let row = quick::row_at(&self.popup.slots.borrow(), p.1);
                    if row.is_some() && row != self.popup.selected.get() {
                        self.popup.selected.set(row);
                        unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
                    }
                }
                Some(0)
            }
            WM_LBUTTONUP => {
                // Not borrowed inside: opening a row lays the list out again.
                let row = quick::row_at(&self.popup.slots.borrow(), point_of(l).1);
                if let Some(i) = row {
                    self.popup.selected.set(Some(i));
                    self.popup_open_selected(How::Open);
                }
                Some(0)
            }
            WM_RBUTTONUP => {
                // Not borrowed inside: opening a row lays the list out again.
                let row = quick::row_at(&self.popup.slots.borrow(), point_of(l).1);
                if let Some(i) = row {
                    self.popup.selected.set(Some(i));
                    unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
                    self.popup_action_menu();
                }
                Some(0)
            }
            WM_APP_FILES => {
                self.popup_files_found();
                Some(0)
            }
            WM_APP_CATALOG => {
                if self.quick_visible() {
                    self.popup_refresh();
                }
                Some(0)
            }
            WM_SETTINGCHANGE if surface::is_theme_change(l) => {
                self.popup_apply_theme();
                unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
                None
            }
            WM_CLOSE => {
                self.hide_quick();
                Some(0)
            }
            _ => None,
        }
    }

    /// Keys typed in the search box; `None` lets the box have them.
    fn popup_edit_key(&self, msg: UINT, w: WPARAM) -> Option<LRESULT> {
        match msg {
            WM_KEYDOWN => {
                let ctrl = unsafe { GetKeyState(VK_CONTROL) } < 0;
                match w as i32 {
                    VK_DOWN => self.popup_move(1),
                    VK_UP => self.popup_move(-1),
                    VK_NEXT => self.popup_move(4),
                    VK_PRIOR => self.popup_move(-4),
                    VK_RETURN => self.popup_open_selected(if ctrl { How::Folder } else { How::Enter }),
                    VK_ESCAPE => self.hide_quick(),
                    VK_TAB => self.popup_action_menu(),
                    _ => return None,
                }
                Some(0)
            }
            // The characters of the keys above (and Ctrl+Backspace's box) would beep or show.
            WM_CHAR if matches!(w, 0x0D | 0x0A | 0x09 | 0x1B | 0x7F) => Some(0),
            _ => None,
        }
    }

    fn popup_move(&self, delta: i32) {
        let n = self.popup.rows.borrow().len();
        let cur = self.popup.selected.get();
        let next = if delta.abs() > 1 {
            cur.map(|c| (c as i64 + delta as i64).clamp(0, n as i64 - 1) as usize).or((n > 0).then_some(0))
        } else {
            quick::move_selection(cur, delta, n)
        };
        self.popup.selected.set(next);
        unsafe { InvalidateRect(self.popup_hwnd(), std::ptr::null(), 0) };
    }

    // ---------------------------------------------------------------- painting

    fn popup_paint(&self, hwnd: HWND) {
        let mut ps: PAINTSTRUCT = unsafe { std::mem::zeroed() };
        let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
        if hdc.is_null() {
            return;
        }
        let mut cr = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe { GetClientRect(hwnd, &mut cr) };
        let (w, h) = (cr.right - cr.left, cr.bottom - cr.top);
        if let Some(mut canvas) = Canvas::new(w, h) {
            self.popup_draw(&mut canvas);
            unsafe { BitBlt(hdc, 0, 0, w, h, canvas.dc, 0, 0, SRCCOPY) };
        }
        unsafe { EndPaint(hwnd, &ps) };
    }

    fn popup_draw(&self, canvas: &mut Canvas) {
        let look = self.popup.look.borrow();
        let rows = self.popup.rows.borrow();
        let slots = self.popup.slots.borrow();
        let (w, h) = (canvas.w, canvas.h);
        let s = look.scale;
        let p = &look.palette;
        let rtl = rtl();
        let list_h = self.popup.list_h.get().min(h);
        let opaque = !look.backdrop;

        // Backgrounds: the list (solid, or clear for the backdrop), the
        // search strip, the line between them and the selection.
        // Over the backdrop the list keeps a tint: where Windows draws it
        // without its blur (a virtual machine, a remote session) what is
        // behind would show through crisp and fight with the rows.
        let list_bg = quick::premul(p.background, if opaque { 255 } else { LIST_TINT });
        let mut base = vec![list_bg; (w * h) as usize];
        let strip = quick::premul(p.background, 255);
        base[(list_h * w) as usize..].fill(strip);
        if !rows.is_empty() && list_h > 0 && list_h < h {
            let line = quick::premul(p.line, 255);
            base[((list_h - 1) * w) as usize..(list_h * w) as usize].fill(line);
        }
        if let Some(sel) = self.popup.selected.get()
            && let Some(slot) = slots.iter().find(|x| x.kind == SlotKind::Row(sel))
        {
            let m = px(6, s);
            let r = Rect::new(m, slot.y + 1, w - m, slot.y + slot.h - 1);
            surface::fill_rounded(&mut base, w, &r, px(5, s) as f32, quick::premul(p.select, p.select_alpha));
            // The accent mark at the reading start of the row.
            let bar_h = (slot.h / 2).max(4);
            let bx = quick::mirror_x(m, px(3, s), w, rtl);
            let bar = Rect::at(bx, slot.y + (slot.h - bar_h) / 2, px(3, s), bar_h);
            surface::fill_rounded(&mut base, w, &bar, px(2, s) as f32, quick::premul(p.accent, 255));
        }

        // Icons first, outside the drawing closure (it can't borrow mutably).
        let icon_px = unsafe { GetSystemMetrics(SM_CXSMICON) };
        // Trimmed before this paint looks icons up, never while it holds some.
        self.popup_trim_icons();
        let icons: Vec<Option<HICON>> = rows.iter().map(|r| self.popup_icon(&r.icon)).collect();

        let text_x = px(48, s);
        let text_w = (w - text_x - px(16, s)).max(1);
        let tx = quick::mirror_x(text_x, text_w, w, rtl);
        let icon_box = px(24, s);
        let ix = quick::mirror_x(px(14, s), icon_box, w, rtl);
        surface::compose(canvas, &base, opaque, &|dc| {
            if rows.is_empty() {
                let hint = tr!("اكتب للبحث في الملفات والتطبيقات والإعدادات", "Type to search files, apps and settings");
                surface::draw_text(dc, &look.small, p.muted, hint, &Rect::at(tx, 0, text_w, list_h), rtl, false);
            }
            for slot in slots.iter() {
                match slot.kind {
                    SlotKind::Header(g) => {
                        let r = Rect::at(quick::mirror_x(px(16, s), w - px(32, s), w, rtl), slot.y, w - px(32, s), slot.h);
                        surface::draw_text(dc, &look.header, p.muted, group_label(g), &r, rtl, false);
                    }
                    SlotKind::Separator => {}
                    SlotKind::Row(i) => {
                        let Some(row) = rows.get(i) else { continue };
                        let icon_r = Rect::at(ix, slot.y, icon_box, slot.h);
                        match (&row.icon, icons.get(i).copied().flatten()) {
                            (_, Some(icon)) => {
                                let (cx, cy) = icon_r.center();
                                surface::draw_icon(dc, icon, cx - icon_px / 2, cy - icon_px / 2, icon_px);
                            }
                            (Icon::Glyph(g), None) => surface::draw_glyph(dc, &look.glyph, p.text, *g, &icon_r),
                            (Icon::Shell { is_dir, .. }, None) => {
                                let g = if *is_dir { super::style::glyph::FOLDER } else { super::style::glyph::OPEN_FILE };
                                surface::draw_glyph(dc, &look.glyph, p.text, g, &icon_r);
                            }
                        }
                        let font = if row.group == Group::More { &look.small } else { &look.title };
                        let title = quick::embed_quoted(&row.title);
                        if row.subtitle.is_empty() || row.group == Group::More {
                            surface::draw_text(dc, font, p.text, &title, &Rect::at(tx, slot.y, text_w, slot.h), rtl, false);
                        } else {
                            let half = slot.h / 2;
                            let top = Rect::at(tx, slot.y + px(3, s), text_w, half - px(2, s));
                            let bottom = Rect::at(tx, slot.y + half, text_w, half - px(3, s));
                            surface::draw_text(dc, font, p.text, &title, &top, rtl, false);
                            // A folder reads left to right; other second lines as written.
                            let sub = if matches!(row.target, Target::File { .. }) { super::format::ltr_path(&row.subtitle) } else { row.subtitle.clone() };
                            surface::draw_text(dc, &look.small, p.muted, &sub, &bottom, rtl, false);
                        }
                    }
                }
            }
            // The magnifier at the start of the search strip.
            let mag = Rect::at(ix, list_h, icon_box, h - list_h);
            surface::draw_glyph(dc, &look.glyph, p.muted, quick::glyph::SEARCH, &mag);
        });
    }

    /// The shell's small icon for a file, folder, program or shortcut;
    /// `None` for glyph rows. Programs and shortcuts have their own icons
    /// (looked up by path), other files share their type's icon.
    fn popup_icon(&self, icon: &Icon) -> Option<HICON> {
        use winapi::um::shellapi::{SHFILEINFOW, SHGFI_ICON, SHGFI_SMALLICON, SHGFI_USEFILEATTRIBUTES, SHGetFileInfoW};
        use winapi::um::winnt::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
        let Icon::Shell { path, is_dir } = icon else { return None };
        let lower = path.to_lowercase();
        if lower.starts_with("shell:") {
            return None;
        }
        let ext = lower.rsplit_once('.').map(|(_, e)| e).filter(|e| !e.contains(['\\', '/'])).unwrap_or("");
        let own = matches!(ext, "exe" | "lnk" | "ico" | "url" | "msc" | "appref-ms");
        let key = if *is_dir {
            "<folder>".to_string()
        } else if own {
            lower.clone()
        } else {
            format!(".{ext}")
        };
        if let Some(&h) = self.popup.icons.borrow().get(&key) {
            return (h != 0).then_some(h as HICON);
        }
        let (name, attrs, flags) = if *is_dir {
            ("folder".to_string(), FILE_ATTRIBUTE_DIRECTORY, SHGFI_USEFILEATTRIBUTES)
        } else if own {
            (path.clone(), FILE_ATTRIBUTE_NORMAL, 0)
        } else {
            (format!("file.{ext}"), FILE_ATTRIBUTE_NORMAL, SHGFI_USEFILEATTRIBUTES)
        };
        let w = shell::wide(&name);
        let mut info: SHFILEINFOW = unsafe { std::mem::zeroed() };
        unsafe { SHGetFileInfoW(w.as_ptr(), attrs, &mut info, std::mem::size_of::<SHFILEINFOW>() as UINT, SHGFI_ICON | SHGFI_SMALLICON | flags) };
        self.popup.icons.borrow_mut().insert(key, info.hIcon as usize);
        (!info.hIcon.is_null()).then_some(info.hIcon)
    }

    /// Empties the icon cache when it grew large. Only called before a paint
    /// looks up its icons: emptying it in the middle would destroy icons the
    /// same paint is about to draw.
    fn popup_trim_icons(&self) {
        let mut icons = self.popup.icons.borrow_mut();
        if icons.len() > 400 {
            for (_, h) in icons.drain() {
                if h != 0 {
                    unsafe { DestroyIcon(h as HICON) };
                }
            }
        }
    }

    // ---------------------------------------------------------------- opening

    fn popup_selected_row(&self) -> Option<Row> {
        let i = self.popup.selected.get()?;
        self.popup.rows.borrow().get(i).cloned()
    }

    fn popup_open_selected(&self, how: How) {
        let Some(row) = self.popup_selected_row() else { return };
        // Enter before the files for what is typed arrived: wait for them.
        // The top row is only chosen by default, and the files can still
        // replace it: it may be a file of an earlier, shorter query, the web
        // fallback, or a weak match that a file will beat. Then open the top
        // row of the final list instead. (Pins, answers and names that start
        // with the query stay on top, so they open at once.)
        if how == How::Enter && self.popup.file_running.get() {
            let first = self.popup.selected.get() == Some(0);
            let may_change = match row.group {
                Group::Pinned => false,
                Group::More | Group::Web => true,
                _ if matches!(row.target, Target::Launcher(Action::CopyText(_))) => false,
                _ => matches!(row.target, Target::File { .. }) || self.popup.files.borrow().rows.is_empty(),
            };
            if (first && may_change) || row.group == Group::More {
                if first {
                    self.popup.selected.set(None);
                }
                self.popup.enter_pending.set(true);
                return;
            }
        }
        self.popup_open(&row, how);
    }

    /// Opens `row` the way `how` says, then closes the popup.
    fn popup_open(&self, row: &Row, how: How) {
        let query = self.popup_query();
        let now = crate::index::now_secs();
        let ok = match (&row.target, how) {
            (t, How::Folder) => match t.path() {
                Some(p) => {
                    self.popup_note_file(p, &query);
                    shell::reveal(p);
                    true
                }
                None => return,
            },
            (t, How::CopyPath) => match t.path() {
                Some(p) => {
                    nwg::Clipboard::set_data_text(&self.window, p);
                    true
                }
                None => return,
            },
            (t, How::Pin) => {
                self.popup_toggle_pin(row, t);
                return;
            }
            (Target::File { path, .. }, How::Admin) => shell_execute(Some("runas"), path, ""),
            (Target::File { path, .. }, _) => {
                self.popup_note_file(path, &query);
                shell::open(path)
            }
            (Target::Launcher(action), how) => {
                // A system command the user said "No" to: back to the list.
                let Some(ok) = self.run_action(action, how == How::Admin) else { return };
                if ok {
                    let mut st = self.state.borrow_mut();
                    st.history.record_launch(&launcher::action_key(action), now);
                    st.history.add_recent(&query);
                    st.history.save();
                }
                ok
            }
            (Target::Feedback { kind, query }, _) => {
                let info = SystemInfo::current();
                let url = match kind {
                    FeedbackKind::Bug => feedback::issue_url(feedback::Kind::Bug, &info, None),
                    FeedbackKind::Idea => feedback::issue_url(feedback::Kind::Idea, &info, None),
                    FeedbackKind::WrongResult => feedback::search_issue_url(&info, query, crate::i18n::is_en()),
                };
                self.hide_quick();
                self.open_feedback_page(&url);
                return;
            }
            (Target::OpenInMain(q), _) => {
                self.query.set_text(q);
                self.show_window();
                self.hide_quick();
                self.on_query_changed();
                return;
            }
        };
        if ok {
            self.hide_quick();
        } else {
            self.popup_error(&trf!("تعذر فتح:\n{}", "Could not open:\n{}", row.title));
        }
    }

    /// A file opened from the popup counts like one opened in the window.
    fn popup_note_file(&self, path: &str, query: &str) {
        let mut st = self.state.borrow_mut();
        st.history.add_recent(query);
        st.history.record_open(path, crate::index::now_secs());
        st.history.save();
    }

    /// Runs a launcher action: whether it worked, or `None` when the user
    /// declined. System commands are confirmed first, with "No" as the
    /// default button.
    fn run_action(&self, action: &Action, admin: bool) -> Option<bool> {
        Some(match action {
            Action::Launch { target, args } => shell_execute(admin.then_some("runas"), &system_tool(target), args),
            Action::OpenUri(uri) => shell::open(uri),
            Action::CopyText(text) => {
                nwg::Clipboard::set_data_text(&self.window, text);
                true
            }
            Action::FocusWindow(h) => shim::focus_window(*h),
            Action::System(cmd) => {
                if !self.popup_confirm(&shim::confirm_text(cmd)) {
                    return None;
                }
                let ok = shim::run_system(cmd);
                if !ok {
                    log(&format!("quick search: system command {cmd:?} failed"));
                }
                ok
            }
        })
    }

    fn popup_toggle_pin(&self, row: &Row, target: &Target) {
        let Some(t) = target.pin_target() else { return };
        let pin = crate::history::Pin {
            title: row.title.clone(),
            target: t.to_string(),
            app: matches!(target, Target::Launcher(_)),
            dir: matches!(target, Target::File { is_dir: true, .. }),
        };
        let mut st = self.state.borrow_mut();
        let was = st.history.is_pinned(t);
        let now = st.history.toggle_pin(pin);
        st.history.save();
        drop(st);
        if !was && !now {
            self.popup_error(&trf!(
                "قائمة المثبّتة ممتلئة ({} عنصراً). ألغِ تثبيت عنصر أولاً.",
                "The pinned list is full ({} items). Unpin one first.",
                crate::history::MAX_PINS
            ));
        }
        self.popup_refresh();
    }

    /// A Yes/No question over the popup; "No" is the default.
    fn popup_confirm(&self, text: &str) -> bool {
        let flags = MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2 | MB_TOPMOST | if rtl() { MB_RTLREADING | MB_RIGHT } else { 0 };
        self.popup_message_box(text, flags) == IDYES
    }

    fn popup_error(&self, text: &str) {
        let flags = MB_OK | MB_ICONERROR | MB_TOPMOST | if rtl() { MB_RTLREADING | MB_RIGHT } else { 0 };
        self.popup_message_box(text, flags);
    }

    fn popup_message_box(&self, text: &str, flags: UINT) -> i32 {
        let (t, c) = (shell::wide(text), shell::wide(title()));
        self.popup.busy.set(self.popup.busy.get() + 1);
        let r = unsafe { MessageBoxW(self.popup_hwnd(), t.as_ptr(), c.as_ptr(), flags) };
        self.popup.busy.set(self.popup.busy.get().saturating_sub(1));
        // Back to typing.
        if self.quick_visible() {
            unsafe {
                SetForegroundWindow(self.popup_hwnd());
                SetFocus(self.popup_edit());
            }
        }
        r
    }

    /// Tab (or a right click): what can be done with the selected row.
    fn popup_action_menu(&self) {
        let Some(row) = self.popup_selected_row() else { return };
        let Some(sel) = self.popup.selected.get() else { return };
        let path = row.target.path().map(str::to_string);
        let admin = match &row.target {
            Target::File { path, is_dir: false } => runs_as_admin(path),
            Target::Launcher(Action::Launch { target, .. }) => runs_as_admin(target),
            _ => false,
        };
        let pinned = row.target.pin_target().map(|t| self.state.borrow().history.is_pinned(t));
        let mut items: Vec<(How, String)> = vec![(How::Open, tr!("فتح\tEnter", "Open\tEnter").to_string())];
        if admin {
            items.push((How::Admin, tr!("تشغيل كمسؤول", "Run as administrator").to_string()));
        }
        if path.is_some() {
            items.push((How::CopyPath, tr!("نسخ المسار", "Copy path").to_string()));
            items.push((How::Folder, tr!("فتح المجلد الذي يحتويه\tCtrl+Enter", "Open containing folder\tCtrl+Enter").to_string()));
        }
        if let Some(p) = pinned {
            let text = if p { tr!("إلغاء التثبيت من المفضلة", "Unpin from favourites") } else { tr!("تثبيت في المفضلة", "Pin to favourites") };
            items.push((How::Pin, text.to_string()));
        }
        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            return;
        }
        for (i, (_, text)) in items.iter().enumerate() {
            let t = shell::wide(text);
            unsafe { AppendMenuW(menu, MF_STRING, i + 1, t.as_ptr()) };
        }
        // Below the selected row, at its reading start.
        let r = self.popup.rect.get();
        let y = self.popup.slots.borrow().iter().find(|x| x.kind == SlotKind::Row(sel)).map_or(0, |x| x.y + x.h);
        let s = self.popup.look.borrow().scale;
        let (x, align) = if rtl() { (r.right - px(48, s), TPM_RIGHTALIGN | TPM_LAYOUTRTL) } else { (r.left + px(48, s), TPM_LEFTALIGN) };
        self.popup.busy.set(self.popup.busy.get() + 1);
        let chosen = unsafe { TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | align, x, r.top + y, 0, self.popup_hwnd(), std::ptr::null()) };
        self.popup.busy.set(self.popup.busy.get().saturating_sub(1));
        unsafe { DestroyMenu(menu) };
        if chosen > 0
            && let Some((how, _)) = items.get(chosen as usize - 1)
        {
            self.popup_open(&row, *how);
        }
    }
}

/// How a row is opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum How {
    /// Enter: may wait for the file results.
    Enter,
    Open,
    Admin,
    Folder,
    CopyPath,
    Pin,
}

/// The system (not the per-monitor) DPI scale, which the whole program uses.
pub(super) fn nwg_scale() -> f64 {
    nwg::scale_factor()
}

/// A pinned favourite as a row.
fn pin_row(p: &crate::history::Pin) -> Row {
    if p.app {
        let shell_app = p.target.to_ascii_lowercase().starts_with("shell:");
        Row {
            group: Group::Pinned,
            title: p.title.clone(),
            subtitle: tr!("تطبيق", "App").to_string(),
            target: Target::Launcher(Action::Launch { target: p.target.clone(), args: String::new() }),
            icon: if shell_app { Icon::Glyph(quick::glyph::APPS) } else { Icon::Shell { path: p.target.clone(), is_dir: false } },
        }
    } else {
        let mut r = Row::file(&p.target, p.dir);
        if !p.title.is_empty() {
            r.title = p.title.clone();
        }
        r
    }
}

/// The texts of the feedback and "open in InstaFind" rows.
fn label_more_row(r: &mut Row) {
    match &r.target {
        Target::Feedback { kind: FeedbackKind::Bug, .. } => r.title = tr!("أبلغ عن مشكلة في بحث سريع…", "Report a problem with InstaFind…").to_string(),
        Target::Feedback { kind: FeedbackKind::Idea, .. } => r.title = tr!("اقترح ميزة جديدة…", "Suggest a feature…").to_string(),
        Target::Feedback { kind: FeedbackKind::WrongResult, .. } => r.title = tr!("النتيجة غلط؟ أخبرنا", "Not what you expected? Tell us").to_string(),
        Target::OpenInMain(q) => r.title = trf!("افتح «{q}» في نافذة البحث الكاملة", "Open “{q}” in the full InstaFind window"),
        _ => {}
    }
}

/// The name shown above a section.
fn group_label(g: Group) -> &'static str {
    match g {
        Group::Pinned => tr!("المثبّتة", "Pinned"),
        Group::Best => tr!("أفضل نتيجة", "Best match"),
        Group::Windows => tr!("النوافذ المفتوحة", "Open windows"),
        Group::Apps => tr!("التطبيقات", "Apps"),
        Group::Files => tr!("الملفات", "Files"),
        Group::Settings => tr!("الإعدادات", "Settings"),
        Group::Answers => tr!("إجابات", "Answers"),
        Group::Commands => tr!("أوامر", "Commands"),
        Group::Web => tr!("الويب", "Web"),
        Group::More => "",
    }
}

#[cfg(test)]
mod dpi_leak_tests {
    /// Issue #10: the scale used on every floating-box frame leaked a screen
    /// DC per call (native-windows-gui's `dpi()`), until GDI ran out and a
    /// size conversion panicked dividing by a DPI of 0.
    #[test]
    fn scale_does_not_leak_gdi_handles() {
        use winapi::um::processthreadsapi::GetCurrentProcess;
        #[link(name = "user32")]
        unsafe extern "system" {
            fn GetGuiResources(process: winapi::um::winnt::HANDLE, flags: u32) -> u32;
        }
        const GR_GDIOBJECTS: u32 = 0;
        let gdi = || unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) };
        let _ = super::nwg_scale();
        let before = gdi();
        for _ in 0..5000 {
            assert!(super::nwg_scale() > 0.0);
            assert!(unsafe { native_windows_gui::dpi() } > 0);
        }
        let after = gdi();
        assert!(after <= before + 16, "GDI objects grew from {before} to {after}");
    }
}
