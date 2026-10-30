//! Thumbnail view (Options ▸ "Thumbnail view", Ctrl+T): the same virtual
//! result list switched to large icons, each file shown by its picture.
//!
//! Thumbnails come from the Windows shell (`IShellItemImageFactory`, the
//! same pictures Explorer shows) on one background thread, only for the rows
//! the list actually paints, newest request first. Until one arrives, and for
//! files that have none, the file-type icon is shown. Pictures live in the
//! list's own image list, managed by a bounded LRU (`crate::thumb_cache`),
//! so memory stays flat however far the user scrolls. The details view is
//! untouched: while it is shown, nothing here runs but one flag test.
//!
//! The choice is saved as a DWORD under `HKCU\Software\BahthSari`.

use native_windows_gui as nwg;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use winapi::ctypes::c_void;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex};

use winapi::shared::guiddef::GUID;
use winapi::shared::minwindef::{DWORD, LPARAM, UINT, WPARAM};
use winapi::shared::windef::{HBITMAP, HGDIOBJ, HICON, HWND, SIZE};
use winapi::shared::winerror::{ERROR_SUCCESS, HRESULT};
use winapi::um::commctrl::{
    HIMAGELIST, ILC_COLOR32, ILD_TRANSPARENT, ImageList_Add, ImageList_Create, ImageList_GetIcon,
    ImageList_GetImageCount, ImageList_Replace, LVIF_IMAGE, LVIF_TEXT, LVM_SETICONSPACING, LVM_SETIMAGELIST,
    LVN_GETDISPINFOW, LVSIL_NORMAL, NM_CUSTOMDRAW, NMLVDISPINFOW,
};
use winapi::um::wingdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDIBits, GetObjectW,
};
use winapi::um::winnt::REG_DWORD;
use winapi::um::winreg::{RegQueryValueExW, RegSetValueExW};
use winapi::um::winuser::{DestroyIcon, GetIconInfo, GetKeyState, ICONINFO, NMHDR, SendMessageW, VK_CONTROL, WM_NOTIFY};

use super::shell::wide;
use super::{App, COL_NAME, autostart};
use crate::thumb_cache::{Cached, ThumbCache};

/// On/off, a DWORD under `HKCU\Software\BahthSari`.
const REG_VALUE: &str = "ThumbnailView";
/// Tile edge in logical pixels (scaled for the screen's DPI).
const TILE: f64 = 96.0;
/// Thumbnails kept in memory: about 36 KB each at 96 px, 9 MB in all.
const MAX_IMAGES: usize = 256;
/// Files remembered in all, including those that have no thumbnail.
const MAX_ENTRIES: usize = 4096;
/// Requests waiting for the thread; the oldest (scrolled past) are dropped.
const MAX_QUEUE: usize = 200;

/// `IShellItemImageFactory` {BCC18B79-BA16-442F-80C4-8A59C30C463B}.
const IID_IMAGE_FACTORY: GUID =
    GUID { Data1: 0xbcc1_8b79, Data2: 0xba16, Data3: 0x442f, Data4: [0x80, 0xc4, 0x8a, 0x59, 0xc3, 0x0c, 0x46, 0x3b] };
/// `IImageList` {46EB5926-582E-4017-9FDF-E8998DAA0950}.
const IID_IMAGE_LIST: GUID =
    GUID { Data1: 0x46eb_5926, Data2: 0x582e, Data3: 0x4017, Data4: [0x9f, 0xdf, 0xe8, 0x99, 0x8d, 0xaa, 0x09, 0x50] };
/// Smaller "thumbnails" are placeholders, not pictures.
const MIN_THUMB: i32 = 16;
/// `SIIGBF_THUMBNAILONLY`: a real thumbnail or nothing (icons are ours).
const SIIGBF_THUMBNAILONLY: i32 = 0x8;

#[repr(C)]
struct ImageFactory {
    vtbl: *const ImageFactoryVtbl,
}

#[repr(C)]
struct ImageFactoryVtbl {
    query_interface: unsafe extern "system" fn(*mut ImageFactory, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut ImageFactory) -> u32,
    release: unsafe extern "system" fn(*mut ImageFactory) -> u32,
    get_image: unsafe extern "system" fn(*mut ImageFactory, SIZE, i32, *mut HBITMAP) -> HRESULT,
}

/// One file waiting for its thumbnail.
struct Request {
    key: String,
    path: String,
}

#[derive(Default)]
struct Queue {
    /// Served last-in first-out: the rows painted most recently.
    waiting: Vec<Request>,
    /// Keys asked for and not yet taken back by the UI thread.
    pending: HashSet<String>,
    /// Finished: key and tile bitmap (0 when the file has no thumbnail).
    done: Vec<(String, usize)>,
}

#[derive(Default)]
struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
}

/// Thumbnail-view state and controls, one field of `App`.
#[derive(Default)]
pub struct Thumbs {
    on: Cell<bool>,
    menu_item: nwg::MenuItem,
    /// Wakes the UI thread when thumbnails are ready.
    notice: nwg::Notice,
    /// The list's large-icon image list (an `HIMAGELIST`), made on first use.
    images: Cell<usize>,
    /// Tile edge in physical pixels.
    size: Cell<i32>,
    cache: RefCell<Option<ThumbCache>>,
    /// Image-list slot of the file-type icon of each extension ("/" folders).
    type_icons: RefCell<HashMap<String, i32>>,
    /// Number of results last arranged.
    count: Cell<usize>,
    /// The thread's queue, once it has been started.
    shared: RefCell<Option<Arc<Shared>>>,
}

// ---------------------------------------------------------------- registry

fn load_enabled() -> bool {
    let Some(key) = autostart::open(autostart::OWN_KEY, false) else { return false };
    let name = wide(REG_VALUE);
    let (mut value, mut size, mut kind): (DWORD, DWORD, DWORD) = (0, 4, 0);
    let ok = unsafe {
        RegQueryValueExW(key.0, name.as_ptr(), std::ptr::null_mut(), &mut kind, (&mut value as *mut DWORD).cast(), &mut size)
    } == ERROR_SUCCESS as i32;
    ok && kind == REG_DWORD && value != 0
}

fn save_enabled(on: bool) {
    let Some(key) = autostart::open(autostart::OWN_KEY, true) else { return };
    let name = wide(REG_VALUE);
    let value: DWORD = on as DWORD;
    unsafe {
        RegSetValueExW(key.0, name.as_ptr(), 0, REG_DWORD, (&value as *const DWORD).cast(), 4);
    }
}

// ---------------------------------------------------------------- pictures

/// The 32-bit pixels of a bitmap, top row first.
fn bitmap_pixels(hbm: HBITMAP) -> Option<(i32, i32, Vec<u32>)> {
    if hbm.is_null() {
        return None;
    }
    unsafe {
        let mut bm: BITMAP = std::mem::zeroed();
        if GetObjectW(hbm as HGDIOBJ, std::mem::size_of::<BITMAP>() as i32, (&mut bm as *mut BITMAP).cast()) == 0 {
            return None;
        }
        let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
        if w <= 0 || h <= 0 || w > 4096 || h > 4096 {
            return None;
        }
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = header(w, h);
        let mut px = vec![0u32; (w * h) as usize];
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        if dc.is_null() {
            return None;
        }
        let lines = GetDIBits(dc, hbm, 0, h as UINT, px.as_mut_ptr().cast(), &mut info, DIB_RGB_COLORS);
        DeleteDC(dc);
        (lines == h).then_some((w, h, px))
    }
}

fn header(w: i32, h: i32) -> BITMAPINFOHEADER {
    BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: w,
        biHeight: -h, // top-down
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        biSizeImage: 0,
        biXPelsPerMeter: 0,
        biYPelsPerMeter: 0,
        biClrUsed: 0,
        biClrImportant: 0,
    }
}

/// An `n`×`n` 32-bit tile with the picture centred on a transparent
/// background (larger pictures are cropped to their middle). A picture
/// without any alpha is taken as opaque.
fn make_tile(w: i32, h: i32, src: &[u32], n: i32) -> HBITMAP {
    let opaque = src.iter().all(|p| p >> 24 == 0);
    unsafe {
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = header(n, n);
        let mut bits: *mut c_void = std::ptr::null_mut();
        let hbm = CreateDIBSection(std::ptr::null_mut(), &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        if hbm.is_null() || bits.is_null() {
            return std::ptr::null_mut();
        }
        let dst = std::slice::from_raw_parts_mut(bits as *mut u32, (n * n) as usize);
        dst.fill(0);
        let (dx, dy) = ((n - w) / 2, (n - h) / 2);
        for y in 0..h {
            let ty = y + dy;
            if !(0..n).contains(&ty) {
                continue;
            }
            for x in 0..w {
                let tx = x + dx;
                if (0..n).contains(&tx) {
                    let p = src[(y * w + x) as usize];
                    dst[(ty * n + tx) as usize] = if opaque { p | 0xFF00_0000 } else { p };
                }
            }
        }
        hbm
    }
}

/// A tile from an icon, at the icon's own size.
fn icon_tile(icon: HICON, n: i32) -> HBITMAP {
    if icon.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        let mut ii: ICONINFO = std::mem::zeroed();
        if GetIconInfo(icon, &mut ii) == 0 {
            return std::ptr::null_mut();
        }
        let color = bitmap_pixels(ii.hbmColor);
        let mut tile = std::ptr::null_mut();
        if let Some((w, h, mut px)) = color {
            // An icon without alpha is cut out by its mask (black = shown).
            if px.iter().all(|p| p >> 24 == 0)
                && let Some((mw, mh, mask)) = bitmap_pixels(ii.hbmMask)
                && (mw, mh) == (w, h)
            {
                for (p, m) in px.iter_mut().zip(&mask) {
                    if m & 0x00FF_FFFF == 0 {
                        *p |= 0xFF00_0000;
                    }
                }
                if px.iter().all(|p| p >> 24 == 0) {
                    px.iter_mut().for_each(|p| *p |= 0xFF00_0000);
                }
            }
            tile = make_tile(w, h, &px, n);
        }
        for b in [ii.hbmColor, ii.hbmMask] {
            if !b.is_null() {
                DeleteObject(b as HGDIOBJ);
            }
        }
        tile
    }
}

/// The extra-large (48 px) shell icon of a file type, or the large one.
fn type_icon_handle(ext: &str) -> HICON {
    use winapi::um::shellapi::{SHGFI_ICON, SHGFI_LARGEICON, SHGFI_USEFILEATTRIBUTES, SHGetImageList, SHIL_EXTRALARGE};
    let index = super::shell::type_icon(ext);
    unsafe {
        let mut list: *mut c_void = std::ptr::null_mut();
        if SHGetImageList(SHIL_EXTRALARGE as i32, &IID_IMAGE_LIST, &mut list) >= 0 && !list.is_null() {
            // An IImageList is also usable as an HIMAGELIST.
            let icon = ImageList_GetIcon(list as HIMAGELIST, index, ILD_TRANSPARENT);
            if !icon.is_null() {
                return icon;
            }
        }
        let (name, attrs) = if ext == "/" {
            ("folder".to_string(), winapi::um::winnt::FILE_ATTRIBUTE_DIRECTORY)
        } else {
            (format!("file.{ext}"), winapi::um::winnt::FILE_ATTRIBUTE_NORMAL)
        };
        let name = wide(&name);
        let mut info: winapi::um::shellapi::SHFILEINFOW = std::mem::zeroed();
        let flags = SHGFI_ICON | SHGFI_LARGEICON | SHGFI_USEFILEATTRIBUTES;
        winapi::um::shellapi::SHGetFileInfoW(name.as_ptr(), attrs, &mut info, std::mem::size_of_val(&info) as u32, flags);
        info.hIcon
    }
}

/// The shell's thumbnail of a file as an `n`×`n` tile, or null when it has
/// none (or the shell can't make thumbnails, as under Wine).
fn shell_thumbnail(path: &str, n: i32) -> HBITMAP {
    unsafe extern "system" {
        fn SHCreateItemFromParsingName(path: *const u16, bc: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> HRESULT;
    }
    let wpath = wide(path);
    unsafe {
        let mut f: *mut ImageFactory = std::ptr::null_mut();
        let hr = SHCreateItemFromParsingName(
            wpath.as_ptr(),
            std::ptr::null_mut(),
            &IID_IMAGE_FACTORY,
            (&mut f as *mut *mut ImageFactory).cast(),
        );
        if hr < 0 || f.is_null() {
            return std::ptr::null_mut();
        }
        let mut hbm: HBITMAP = std::ptr::null_mut();
        let hr = ((*(*f).vtbl).get_image)(f, SIZE { cx: n, cy: n }, SIIGBF_THUMBNAILONLY, &mut hbm);
        ((*(*f).vtbl).release)(f);
        if hr < 0 || hbm.is_null() {
            return std::ptr::null_mut();
        }
        // Wine answers with a 1×1 bitmap; a real thumbnail is bigger than
        // a few pixels on its long side.
        let tile = match bitmap_pixels(hbm) {
            Some((w, h, px)) if w.max(h) >= MIN_THUMB => make_tile(w, h, &px, n),
            _ => std::ptr::null_mut(),
        };
        DeleteObject(hbm as HGDIOBJ);
        tile
    }
}

/// The background thread: makes thumbnails, newest request first.
fn worker(shared: Arc<Shared>, notice: nwg::NoticeSender, n: i32) {
    unsafe {
        winapi::um::combaseapi::CoInitializeEx(std::ptr::null_mut(), winapi::um::objbase::COINIT_APARTMENTTHREADED);
    }
    loop {
        let req = {
            let Ok(mut q) = shared.queue.lock() else { return };
            loop {
                if let Some(r) = q.waiting.pop() {
                    break r;
                }
                q = match shared.wake.wait(q) {
                    Ok(q) => q,
                    Err(_) => return,
                };
            }
        };
        let tile = shell_thumbnail(&req.path, n) as usize;
        let Ok(mut q) = shared.queue.lock() else { return };
        q.done.push((req.key, tile));
        drop(q);
        notice.notice();
    }
}

/// Writes `text` into a list-view text buffer of `max` units with its NUL.
fn copy_text(text: &str, buf: *mut u16, max: i32) {
    if buf.is_null() || max <= 0 {
        return;
    }
    let max = max as usize - 1;
    let out = unsafe { std::slice::from_raw_parts_mut(buf, max + 1) };
    let mut n = 0;
    for u in text.encode_utf16().take(max) {
        out[n] = u;
        n += 1;
    }
    out[n] = 0;
}

// ---------------------------------------------------------------- the app's side

impl App {
    /// Adds the Options item and the notice. Called from `build_ui` while
    /// the app is still being built.
    pub(super) fn build_thumbs(&mut self) -> Result<(), nwg::NwgError> {
        let on = load_enabled();
        nwg::MenuItem::builder()
            .text(tr!("عرض المصغرات\tCtrl+T", "Thumbnail view\tCtrl+T"))
            .check(on)
            .parent(&self.options_menu)
            .build(&mut self.thumbs.menu_item)?;
        nwg::Notice::builder().parent(&self.window).build(&mut self.thumbs.notice)?;
        Ok(())
    }

    /// Binds the events and applies the saved view. Called once from
    /// `build_ui`, after the other list handlers so this one runs first.
    pub(super) fn setup_thumbs(self: &Rc<Self>) -> Result<(nwg::EventHandler, nwg::RawEventHandler), nwg::NwgError> {
        let weak = Rc::downgrade(self);
        let handler = nwg::full_bind_event_handler(&self.window.handle, move |evt, data, handle| {
            let Some(app) = weak.upgrade() else { return };
            match evt {
                nwg::Event::OnMenuItemSelected if handle == app.thumbs.menu_item => app.toggle_thumbs(),
                nwg::Event::OnNotice if handle == app.thumbs.notice => app.on_thumbs_ready(),
                nwg::Event::OnKeyPress
                    if data.on_key() == 'T' as u32
                        && unsafe { GetKeyState(VK_CONTROL) } < 0
                        && !app.renaming.get() =>
                {
                    app.toggle_thumbs()
                }
                _ => {}
            }
        });
        let weak = Rc::downgrade(self);
        let list_hwnd = self.list.handle.hwnd().unwrap_or(std::ptr::null_mut()) as usize;
        let raw = nwg::bind_raw_event_handler(&self.window.handle, 0x1_0029, move |_h: HWND, msg: UINT, _w: WPARAM, l: LPARAM| {
            if msg != WM_NOTIFY || l == 0 {
                return None;
            }
            let hdr = unsafe { &*(l as *const NMHDR) };
            if hdr.hwndFrom as usize != list_hwnd {
                return None;
            }
            let app = weak.upgrade()?;
            if !app.thumbs.on.get() {
                return None;
            }
            match hdr.code {
                LVN_GETDISPINFOW => {
                    app.arrange_if_count_changed();
                    let item = unsafe { &mut (*(l as *mut NMLVDISPINFOW)).item };
                    let row = item.iItem.max(0) as usize;
                    if item.mask & LVIF_TEXT != 0 {
                        copy_text(&app.cell_text(row, COL_NAME), item.pszText, item.cchTextMax);
                    }
                    if item.mask & LVIF_IMAGE != 0 {
                        item.iImage = app.thumb_image(row);
                    }
                    Some(0)
                }
                // The match underline is placed for the details view's label.
                NM_CUSTOMDRAW => Some(winapi::um::commctrl::CDRF_DODEFAULT),
                _ => None,
            }
        })?;
        if self.thumbs.menu_item.checked() {
            // Switched from the message loop (see `on_thumbs_ready`), once
            // the window is laid out.
            self.thumbs.notice.sender().notice();
        }
        Ok((handler, raw))
    }

    /// Re-arranges the tiles when the number of results changed. Wine keeps
    /// a position per item in an owner-data icon view and leaves new ones at
    /// the origin, so a longer result list would be drawn in one pile.
    fn arrange_if_count_changed(&self) {
        let n = self.list.len();
        if self.thumbs.count.replace(n) != n
            && let Some(h) = self.list.handle.hwnd()
        {
            use winapi::um::commctrl::{LVA_DEFAULT, LVM_ARRANGE};
            unsafe { winapi::um::winuser::PostMessageW(h, LVM_ARRANGE, LVA_DEFAULT as WPARAM, 0) };
        }
    }

    /// Whether the results are shown as thumbnails.
    pub(super) fn thumbs_on(&self) -> bool {
        self.thumbs.on.get()
    }

    fn toggle_thumbs(&self) {
        let on = !self.thumbs.on.get();
        self.set_thumbs(on);
        save_enabled(on);
        self.set_status(if on {
            tr!("عرض المصغرات. اضغط Ctrl+T للعودة إلى عرض التفاصيل.", "Thumbnail view. Press Ctrl+T to go back to the details view.")
        } else {
            tr!("عرض التفاصيل.", "Details view.")
        });
    }

    fn set_thumbs(&self, on: bool) {
        if on && !self.ensure_image_list() {
            return;
        }
        self.thumbs.on.set(on);
        self.thumbs.menu_item.set_checked(on);
        if !on && let Some(shared) = self.thumbs.shared.borrow().as_ref()
            && let Ok(mut q) = shared.queue.lock()
        {
            // Nothing scrolled past is worth making any more.
            let q = &mut *q;
            for r in q.waiting.drain(..) {
                q.pending.remove(&r.key);
            }
        }
        let selected = self.list.selected_item();
        self.list.set_list_style(if on { nwg::ListViewStyle::Icon } else { nwg::ListViewStyle::Detailed });
        if let Some(h) = self.list.handle.hwnd() {
            // Kept arranged in rows whatever the number of results.
            use winapi::um::winuser::{GWL_STYLE, GetWindowLongW, SetWindowLongW};
            let auto = winapi::um::commctrl::LVS_AUTOARRANGE as i32;
            unsafe {
                let style = GetWindowLongW(h, GWL_STYLE);
                SetWindowLongW(h, GWL_STYLE, if on { style | auto } else { style & !auto });
            }
        }
        if let Some(i) = selected {
            self.list.select_item(i, true);
            if let Some(h) = self.list.handle.hwnd() {
                unsafe { SendMessageW(h, winapi::um::commctrl::LVM_ENSUREVISIBLE, i, 0) };
            }
        }
        self.list.invalidate();
    }

    /// Makes the image list and gives it to the list. False if it can't.
    fn ensure_image_list(&self) -> bool {
        if self.thumbs.images.get() != 0 {
            return true;
        }
        let Some(h) = self.list.handle.hwnd() else { return false };
        let scale = nwg::scale_factor();
        let n = (TILE * scale).round() as i32;
        let images = unsafe { ImageList_Create(n, n, ILC_COLOR32, 32, 32) };
        if images.is_null() {
            return false;
        }
        self.thumbs.images.set(images as usize);
        self.thumbs.size.set(n);
        *self.thumbs.cache.borrow_mut() = Some(ThumbCache::new(MAX_IMAGES, MAX_ENTRIES));
        let (cx, cy) = (n + (28.0 * scale) as i32, n + (48.0 * scale) as i32);
        unsafe {
            // The list was made with LVS_SHAREIMAGELISTS: it never frees it.
            SendMessageW(h, LVM_SETIMAGELIST, LVSIL_NORMAL as WPARAM, images as LPARAM);
            SendMessageW(h, LVM_SETICONSPACING, 0, ((cy as u32) << 16 | (cx as u32 & 0xFFFF)) as LPARAM);
        }
        true
    }

    /// Puts a tile into image-list `slot`; frees the bitmap.
    fn put_tile(&self, slot: i32, tile: HBITMAP) {
        let images = self.thumbs.images.get() as HIMAGELIST;
        unsafe {
            if slot >= ImageList_GetImageCount(images) {
                ImageList_Add(images, tile, std::ptr::null_mut());
            } else {
                ImageList_Replace(images, slot, tile, std::ptr::null_mut());
            }
            DeleteObject(tile as HGDIOBJ);
        }
    }

    /// Image-list slot of the file-type icon for an extension ("/" folders).
    fn type_tile(&self, ext: &str) -> i32 {
        if let Some(&i) = self.thumbs.type_icons.borrow().get(ext) {
            return i;
        }
        let Some(slot) = self.thumbs.cache.borrow_mut().as_mut().map(|c| c.pin_slot()) else { return 0 };
        let n = self.thumbs.size.get();
        let icon = type_icon_handle(ext);
        let mut tile = icon_tile(icon, n);
        if !icon.is_null() {
            unsafe { DestroyIcon(icon) };
        }
        if tile.is_null() {
            // Keep the slot numbering in step with the image list.
            tile = make_tile(0, 0, &[], n);
        }
        self.put_tile(slot, tile);
        self.thumbs.type_icons.borrow_mut().insert(ext.to_string(), slot);
        slot
    }

    /// The picture for a row: its thumbnail if ready, else its type icon
    /// (and the thumbnail is asked for).
    fn thumb_image(&self, row: usize) -> i32 {
        let (key, path, ext) = {
            let Ok(st) = self.state.try_borrow() else { return 0 };
            let Some(e) = st.row_entry(row) else { return 0 };
            if e.is_dir {
                drop(st);
                return self.type_tile("/");
            }
            (ThumbCache::key(&e.path, e.size, e.mtime), e.path.to_string(), e.ext())
        };
        let cached = self.thumbs.cache.borrow_mut().as_mut().and_then(|c| c.get(&key));
        match cached {
            Some(Cached::Image(slot)) => slot,
            Some(Cached::NoThumb) => self.type_tile(&ext),
            None => {
                self.request_thumb(key, path);
                self.type_tile(&ext)
            }
        }
    }

    fn request_thumb(&self, key: String, path: String) {
        let mut slot = self.thumbs.shared.borrow_mut();
        let shared = match slot.as_ref() {
            Some(s) => s.clone(),
            None => {
                let s = Arc::new(Shared::default());
                let (t, sender, n) = (s.clone(), self.thumbs.notice.sender(), self.thumbs.size.get());
                let started = std::thread::Builder::new().name("thumbs".into()).spawn(move || worker(t, sender, n));
                if started.is_err() {
                    return;
                }
                *slot = Some(s.clone());
                s
            }
        };
        drop(slot);
        let Ok(mut q) = shared.queue.lock() else { return };
        if !q.pending.insert(key.clone()) {
            return;
        }
        q.waiting.push(Request { key, path });
        if q.waiting.len() > MAX_QUEUE {
            let old = q.waiting.remove(0);
            q.pending.remove(&old.key);
        }
        drop(q);
        shared.wake.notify_one();
    }

    /// Takes the finished thumbnails into the image list and repaints.
    fn on_thumbs_ready(&self) {
        if self.thumbs.menu_item.checked() && !self.thumbs.on.get() {
            self.set_thumbs(true);
        }
        let Some(shared) = self.thumbs.shared.borrow().clone() else { return };
        let done = match shared.queue.lock() {
            Ok(mut q) => {
                let done = std::mem::take(&mut q.done);
                for (key, _) in &done {
                    q.pending.remove(key);
                }
                done
            }
            Err(_) => return,
        };
        if done.is_empty() {
            return;
        }
        let mut any = false;
        for (key, tile) in done {
            let tile = tile as HBITMAP;
            if tile.is_null() {
                if let Some(c) = self.thumbs.cache.borrow_mut().as_mut() {
                    c.insert_none(key);
                }
                continue;
            }
            let Some(slot) = self.thumbs.cache.borrow_mut().as_mut().map(|c| c.insert_image(key)) else {
                unsafe { DeleteObject(tile as HGDIOBJ) };
                continue;
            };
            self.put_tile(slot, tile);
            any = true;
        }
        if any && self.thumbs.on.get() {
            self.list.invalidate();
        }
    }
}
