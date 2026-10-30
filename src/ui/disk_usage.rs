//! Folder sizes ("what uses my disk"): folder totals in the result list's
//! Size column, and a window listing the largest items inside a folder or
//! drive, with drill-down. The sizes come from the index
//! ([`crate::folder_sizes`]), computed on a background thread the first time
//! they are needed for each index snapshot.

use native_windows_gui as nwg;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use winapi::shared::minwindef::{LPARAM, UINT, WPARAM};
use winapi::shared::windef::{HWND, RECT};
use winapi::um::winuser::{NMHDR, SendMessageW, VK_BACK, VK_F5, WM_NOTIFY, WM_SETFONT};

use super::format::{digits, fmt_bytes, fmt_num, fmt_size, ltr_path};
use super::{App, WS_EX_LAYOUTRTL, shell};
use crate::folder_sizes::{self, Node};
use crate::index::{Entry, Index};

/// At most this many items are listed; the rest are summed in one last row.
const MAX_ROWS: usize = 1000;
/// The column with the drawn bar.
const COL_BAR: i32 = 3;
/// Id of the raw handler that draws the bars.
const RAW_ID: usize = 0x1_0D15;

const COLS: [(&str, &str, i32); 5] = [
    ("الاسم", "Name", 300),
    ("الحجم", "Size", 100),
    ("النسبة", "Share", 80),
    ("", "", 180),
    ("عدد الملفات", "Files", 110),
];

/// One listed item: its path (empty for the "other items" row).
struct UsageRow {
    path: String,
    is_dir: bool,
    /// Part of the folder's total, 0 to 1, drawn as a bar.
    share: f64,
}

/// The folder-sizes window and its menu items, one field of `App`.
#[derive(Default)]
pub struct DiskUsage {
    menu_options: nwg::MenuItem,
    menu_row: nwg::MenuItem,
    /// Wakes the UI thread when sizes for an index snapshot are ready.
    notice: nwg::Notice,
    /// The index snapshot whose sizes are being computed (pointer, 0 = none).
    pending: Cell<usize>,
    window: nwg::Window,
    up_btn: nwg::Button,
    explorer_btn: nwg::Button,
    path_box: nwg::TextInput,
    total_label: nwg::Label,
    list: nwg::ListView,
    /// The folder shown; empty means all drives.
    location: RefCell<String>,
    rows: RefCell<Vec<UsageRow>>,
}

/// Width in pixels of the filled part of a bar `width` wide; any share
/// above zero shows at least one pixel.
fn bar_width(share: f64, width: i32) -> i32 {
    let w = (share.clamp(0.0, 1.0) * width as f64).round() as i32;
    if share > 0.0 { w.max(1) } else { w }
}

/// Custom draw for the list: paints the bar column as a filled bar.
fn draw_bar(l: LPARAM, share_of: impl Fn(usize) -> Option<f64>) -> Option<isize> {
    use winapi::um::commctrl::{
        CDDS_ITEMPREPAINT, CDDS_PREPAINT, CDDS_SUBITEM, CDRF_DODEFAULT, CDRF_NOTIFYITEMDRAW,
        CDRF_NOTIFYSUBITEMDRAW, CDRF_SKIPDEFAULT, LVIR_BOUNDS, LVM_GETSUBITEMRECT, NMLVCUSTOMDRAW,
    };
    use winapi::um::wingdi::{CreateSolidBrush, DeleteObject, RGB};
    use winapi::um::winuser::{COLOR_WINDOW, FillRect, GetSysColorBrush};
    let cd = unsafe { &*(l as *const NMLVCUSTOMDRAW) };
    let stage = cd.nmcd.dwDrawStage;
    if stage == CDDS_PREPAINT {
        return Some(CDRF_NOTIFYITEMDRAW);
    }
    if stage == CDDS_ITEMPREPAINT {
        return Some(CDRF_NOTIFYSUBITEMDRAW);
    }
    if stage != CDDS_ITEMPREPAINT | CDDS_SUBITEM || cd.iSubItem != COL_BAR {
        return Some(CDRF_DODEFAULT);
    }
    let row = cd.nmcd.dwItemSpec;
    let Some(share) = share_of(row) else { return Some(CDRF_DODEFAULT) };
    let mut rc = RECT { left: LVIR_BOUNDS, top: COL_BAR, right: 0, bottom: 0 };
    let hdc = cd.nmcd.hdc;
    unsafe {
        if SendMessageW(cd.nmcd.hdr.hwndFrom, LVM_GETSUBITEMRECT, row, &mut rc as *mut RECT as LPARAM) == 0 {
            return Some(CDRF_DODEFAULT);
        }
        FillRect(hdc, &rc, GetSysColorBrush(COLOR_WINDOW));
        let inner = RECT { left: rc.left + 4, top: rc.top + 4, right: rc.right - 4, bottom: rc.bottom - 4 };
        if inner.right > inner.left && inner.bottom > inner.top {
            let track = CreateSolidBrush(RGB(0xE6, 0xE6, 0xE6));
            FillRect(hdc, &inner, track);
            DeleteObject(track as _);
            let filled = RECT { right: inner.left + bar_width(share, inner.right - inner.left), ..inner };
            let fill = CreateSolidBrush(RGB(0x26, 0x7C, 0xD6));
            FillRect(hdc, &filled, fill);
            DeleteObject(fill as _);
        }
    }
    Some(CDRF_SKIPDEFAULT)
}

/// The last part of a path (the whole path for a root).
fn name_of(path: &str) -> &str {
    let t = path.trim_end_matches(['\\', '/']);
    match t.rfind(['\\', '/']) {
        Some(i) if i + 1 < t.len() => &t[i + 1..],
        _ => path,
    }
}

fn percent(share: f64) -> String {
    digits(format!("{:.1}%", share * 100.0))
}

impl App {
    /// Adds the menu items and builds the (hidden) window. Called from
    /// `build_ui` while the app is still being built.
    pub(super) fn build_disk_usage(&mut self) -> Result<(), nwg::NwgError> {
        nwg::MenuItem::builder()
            .text(tr!("أحجام المجلدات…", "Folder sizes…"))
            .parent(&self.options_menu)
            .build(&mut self.disk.menu_options)?;
        nwg::MenuItem::builder()
            .text(tr!("أحجام المجلدات هنا…", "Folder sizes here…"))
            .parent(&self.menu)
            .build(&mut self.disk.menu_row)?;
        nwg::Notice::builder().parent(&self.window).build(&mut self.disk.notice)?;

        let f = Some(&self.font);
        let d = &mut self.disk;
        nwg::Window::builder()
            .title(tr!("أحجام المجلدات", "Folder sizes"))
            .size((800, 540))
            .center(true)
            .ex_flags(crate::i18n::rtl_only(WS_EX_LAYOUTRTL))
            .flags(nwg::WindowFlags::WINDOW | nwg::WindowFlags::RESIZABLE)
            .icon(None)
            .parent(Some(&self.window))
            .build(&mut d.window)?;
        let w = &d.window;
        nwg::Button::builder().parent(w).font(f).text(tr!("أعلى", "Up")).build(&mut d.up_btn)?;
        nwg::Button::builder().parent(w).font(f).text(tr!("فتح في المستكشف", "Open in Explorer")).build(&mut d.explorer_btn)?;
        nwg::TextInput::builder().parent(w).font(f).readonly(true).build(&mut d.path_box)?;
        nwg::Label::builder().parent(w).font(f).text("").build(&mut d.total_label)?;
        nwg::ListView::builder()
            .parent(w)
            .flags(
                nwg::ListViewFlags::VISIBLE
                    | nwg::ListViewFlags::TAB_STOP
                    | nwg::ListViewFlags::SINGLE_SELECTION
                    | nwg::ListViewFlags::ALWAYS_SHOW_SELECTION,
            )
            .list_style(nwg::ListViewStyle::Detailed)
            .ex_flags(nwg::ListViewExFlags::FULL_ROW_SELECT)
            .build(&mut d.list)?;
        if let Some(h) = d.list.handle.hwnd() {
            unsafe {
                SendMessageW(h, WM_SETFONT, self.font.handle as WPARAM, 1);
                let theme = shell::wide("Explorer");
                winapi::um::uxtheme::SetWindowTheme(h, theme.as_ptr(), std::ptr::null());
            }
        }
        let scale = nwg::scale_factor();
        for (i, (ar, en, width)) in COLS.iter().enumerate() {
            d.list.insert_column(nwg::InsertListViewColumn {
                index: Some(i as i32),
                fmt: None,
                width: Some((*width as f64 * scale).round() as i32),
                text: Some(tr!(*ar, *en).to_string()),
            });
        }
        d.list.set_headers_enabled(true);
        Ok(())
    }

    /// Binds the window's and the menu items' events. Called once from
    /// `build_ui`; the handlers are unbound with the others.
    pub(super) fn setup_disk_usage(self: &Rc<Self>) -> Result<(Vec<nwg::EventHandler>, nwg::RawEventHandler), nwg::NwgError> {
        let weak = Rc::downgrade(self);
        let main = nwg::full_bind_event_handler(&self.window.handle, move |evt, _data, handle| {
            let Some(app) = weak.upgrade() else { return };
            match evt {
                nwg::Event::OnMenuItemSelected if handle == app.disk.menu_options => {
                    let scope = app.state.borrow().scope.clone();
                    app.show_disk_usage(scope);
                }
                nwg::Event::OnMenuItemSelected if handle == app.disk.menu_row => {
                    let folder = {
                        let st = app.state.borrow();
                        app.list.selected_item().and_then(|r| st.row_entry(r)).map(|e| {
                            if e.is_dir {
                                e.path.to_string()
                            } else {
                                folder_sizes::parent_path(&e.path).unwrap_or_default().to_string()
                            }
                        })
                    };
                    match folder {
                        Some(f) => app.show_disk_usage(f),
                        None => app.set_status(tr!("اختر ملفاً من القائمة أولاً.", "Select a file in the list first.")),
                    }
                }
                nwg::Event::OnNotice if handle == app.disk.notice => app.on_sizes_ready(),
                _ => {}
            }
        });

        let weak = Rc::downgrade(self);
        let window = nwg::full_bind_event_handler(&self.disk.window.handle, move |evt, data, handle| {
            let Some(app) = weak.upgrade() else { return };
            let d = &app.disk;
            match evt {
                nwg::Event::OnWindowClose if handle == d.window => {
                    if let nwg::EventData::OnWindowClose(c) = &data {
                        c.close(false);
                    }
                    d.window.set_visible(false);
                }
                nwg::Event::OnResize if handle == d.window => app.layout_disk_usage(),
                nwg::Event::OnKeyEsc => d.window.set_visible(false),
                nwg::Event::OnKeyEnter => app.disk_open_row(),
                nwg::Event::OnButtonClick if handle == d.up_btn => app.disk_up(),
                nwg::Event::OnButtonClick if handle == d.explorer_btn => app.disk_explorer(),
                nwg::Event::OnListViewItemActivated if handle == d.list => app.disk_open_row(),
                nwg::Event::OnKeyPress if handle == d.list => {
                    let key = data.on_key();
                    if key == VK_BACK as u32 {
                        app.disk_up();
                    } else if key == VK_F5 as u32 {
                        app.render_disk_usage();
                    }
                }
                _ => {}
            }
        });
        let weak = Rc::downgrade(self);
        let list_hwnd = self.disk.list.handle.hwnd().unwrap_or(std::ptr::null_mut());
        let raw = nwg::bind_raw_event_handler(&self.disk.window.handle, RAW_ID, move |_h: HWND, msg: UINT, _w: WPARAM, l: LPARAM| {
            if msg != WM_NOTIFY {
                return None;
            }
            let hdr = unsafe { &*(l as *const NMHDR) };
            if hdr.hwndFrom != list_hwnd || hdr.code != winapi::um::commctrl::NM_CUSTOMDRAW {
                return None;
            }
            let app = weak.upgrade()?;
            draw_bar(l, |row| app.disk.rows.try_borrow().ok()?.get(row).map(|r| r.share))
        })?;
        Ok((vec![main, window], raw))
    }

    /// Size-column text of a result row: a folder shows everything inside it
    /// once the sizes are ready.
    pub(super) fn size_cell(&self, index: &Arc<Index>, i: u32, e: &Entry) -> String {
        if !e.is_dir {
            return fmt_size(e);
        }
        match folder_sizes::cached(index) {
            Some(s) => fmt_bytes(s.total(i as usize)),
            None => {
                self.request_sizes(index.clone());
                fmt_size(e)
            }
        }
    }

    /// Computes the sizes for `index` on a background thread, once.
    fn request_sizes(&self, index: Arc<Index>) {
        let key = Arc::as_ptr(&index) as usize;
        if self.disk.pending.get() == key {
            return;
        }
        self.disk.pending.set(key);
        let notice = self.disk.notice.sender();
        std::thread::spawn(move || {
            folder_sizes::for_index(&index);
            drop(index);
            notice.notice();
        });
    }

    fn on_sizes_ready(&self) {
        self.disk.pending.set(0);
        self.list.invalidate();
        if self.disk.window.visible() {
            self.render_disk_usage();
        }
    }

    /// Opens the window on `folder` (empty for all drives).
    pub(super) fn show_disk_usage(&self, folder: String) {
        *self.disk.location.borrow_mut() = folder;
        self.disk.window.set_visible(true);
        self.layout_disk_usage();
        self.render_disk_usage();
        self.disk.list.set_focus();
    }

    fn layout_disk_usage(&self) {
        let d = &self.disk;
        let (w, h) = d.window.size();
        let (w, h) = (w as i32, h as i32);
        let (m, row_h, up_w, ex_w) = (8, 30, 90, 190);
        d.up_btn.set_position(m, m);
        d.up_btn.set_size(up_w as u32, row_h as u32);
        d.explorer_btn.set_position(w - m - ex_w, m);
        d.explorer_btn.set_size(ex_w as u32, row_h as u32);
        d.path_box.set_position(2 * m + up_w, m + 1);
        d.path_box.set_size((w - 4 * m - up_w - ex_w).max(50) as u32, (row_h - 2) as u32);
        let y = m * 2 + row_h;
        d.total_label.set_position(m, y);
        d.total_label.set_size((w - 2 * m).max(50) as u32, 24);
        let y = y + 24 + m / 2;
        d.list.set_position(m, y);
        d.list.set_size((w - 2 * m).max(50) as u32, (h - y - m).max(50) as u32);
    }

    /// Fills the window with what is inside the current folder.
    fn render_disk_usage(&self) {
        let d = &self.disk;
        let index = self.state.borrow().index.clone();
        let Some(sizes) = folder_sizes::cached(&index) else {
            d.list.clear();
            d.rows.borrow_mut().clear();
            d.total_label.set_text(tr!("جاري حساب أحجام المجلدات…", "Calculating folder sizes…"));
            self.request_sizes(index);
            return;
        };
        let entries = &index.entries;
        // A folder that left the index shows its nearest remaining parent.
        let mut loc = d.location.borrow().clone();
        let node = loop {
            if let Some(n) = sizes.locate(entries, &loc) {
                break n;
            }
            match folder_sizes::parent_path(&loc) {
                Some(p) if p != loc => loc = p.to_string(),
                _ => break Node::All,
            }
        };
        let path = sizes.node_path(entries, node).to_string();
        *d.location.borrow_mut() = path.clone();

        let (total, files) = sizes.node_total(node);
        let kids = sizes.children(entries, node);
        d.path_box.set_text(&if path.is_empty() { tr!("كل الأقراص", "All drives").to_string() } else { ltr_path(&path) });
        d.window.set_text(&if path.is_empty() {
            tr!("أحجام المجلدات", "Folder sizes").to_string()
        } else {
            trf!("أحجام المجلدات — {}", "Folder sizes — {}", name_of(&path))
        });
        d.total_label.set_text(&trf!(
            "الحجم الكلي: {} في {} ملف · العناصر: {} · Backspace للرجوع، نقرتان للدخول",
            "Total: {} in {} files · items: {} · Backspace goes up, double-click opens",
            fmt_bytes(total),
            fmt_num(files as usize),
            fmt_num(kids.len())
        ));
        d.up_btn.set_enabled(node != Node::All);

        let share = |size: u64| if total == 0 { 0.0 } else { size as f64 / total as f64 };
        let mut rows = Vec::with_capacity(kids.len().min(MAX_ROWS + 1));
        d.list.set_redraw(false);
        d.list.clear();
        for (r, c) in kids.iter().take(MAX_ROWS).enumerate() {
            let p = sizes.node_path(entries, c.node);
            let name = match c.node {
                Node::Entry(_) => name_of(p).to_string(),
                _ => ltr_path(p),
            };
            let files = if c.is_dir { fmt_num(c.files as usize) } else { String::new() };
            let s = share(c.size);
            d.list.insert_items_row(Some(r as i32), &[name, fmt_bytes(c.size), percent(s), String::new(), files]);
            rows.push(UsageRow { path: p.to_string(), is_dir: c.is_dir, share: s });
        }
        if kids.len() > MAX_ROWS {
            let rest = &kids[MAX_ROWS..];
            let size: u64 = rest.iter().map(|c| c.size).sum();
            let files: u64 = rest.iter().map(|c| c.files).sum();
            let s = share(size);
            let label = trf!("… و{} عنصراً آخر", "… and {} more items", fmt_num(rest.len()));
            d.list.insert_items_row(Some(MAX_ROWS as i32), &[label, fmt_bytes(size), percent(s), String::new(), fmt_num(files as usize)]);
            rows.push(UsageRow { path: String::new(), is_dir: false, share: s });
        }
        d.list.set_redraw(true);
        *d.rows.borrow_mut() = rows;
        if !kids.is_empty() {
            d.list.select_item(0, true);
        }
    }

    fn disk_selected(&self) -> Option<(String, bool)> {
        let r = self.disk.list.selected_item()?;
        self.disk.rows.borrow().get(r).filter(|u| !u.path.is_empty()).map(|u| (u.path.clone(), u.is_dir))
    }

    /// Enters a folder; a file is shown in Explorer.
    fn disk_open_row(&self) {
        match self.disk_selected() {
            Some((p, true)) => {
                *self.disk.location.borrow_mut() = p;
                self.render_disk_usage();
            }
            Some((p, false)) => shell::reveal(&p),
            None => {}
        }
    }

    fn disk_up(&self) {
        let here = self.disk.location.borrow().clone();
        if here.is_empty() {
            return;
        }
        let up = {
            let index = self.state.borrow().index.clone();
            folder_sizes::cached(&index).and_then(|s| {
                let node = s.locate(&index.entries, &here)?;
                Some(s.node_path(&index.entries, s.parent(node)?).to_string())
            })
        };
        *self.disk.location.borrow_mut() = up.unwrap_or_default();
        self.render_disk_usage();
        // Keep the folder we came from selected.
        if let Some(i) = self.disk.rows.borrow().iter().position(|u| u.path == here) {
            self.disk.list.select_item(0, false);
            self.disk.list.select_item(i, true);
        }
    }

    /// Shows the selected item in Explorer, or opens the current folder.
    fn disk_explorer(&self) {
        if let Some((p, _)) = self.disk_selected() {
            shell::reveal(&p);
            return;
        }
        let here = self.disk.location.borrow().clone();
        if !here.is_empty() && !shell::open(&here) {
            self.set_status(&trf!("تعذر فتح المجلد:\n{here}", "Could not open the folder:\n{here}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars() {
        assert_eq!(bar_width(0.0, 100), 0);
        assert_eq!(bar_width(1.0, 100), 100);
        assert_eq!(bar_width(0.0001, 100), 1, "a tiny share still shows");
        assert_eq!(bar_width(0.5, 100), 50);
        assert_eq!(bar_width(2.0, 100), 100);
    }
}
