//! The look of the main window: the spacing grid, button widths fitted to
//! their text, small monochrome icons on the buttons, white backgrounds
//! behind labels and check boxes, and the window's minimum size.
//!
//! Everything here is plain Win32 and degrades quietly: if an icon font is
//! missing (Windows 7/8) the buttons keep their text only, and if a call
//! fails the control simply keeps its default look.

use native_windows_gui as nwg;

use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
use winapi::shared::windef::{HBRUSH, HDC, HGDIOBJ, HWND, RECT, SIZE};
use winapi::um::commctrl::{
    BCM_SETIMAGELIST, BUTTON_IMAGELIST, BUTTON_IMAGELIST_ALIGN_LEFT, HIMAGELIST, ILC_COLOR32, ImageList_Add,
    ImageList_Create,
};
use winapi::um::wingdi::{
    ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS, CreateCompatibleDC,
    CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DIB_RGB_COLORS, DeleteDC, DeleteObject, FW_NORMAL,
    GetTextExtentPoint32W, OUT_DEFAULT_PRECIS, SelectObject, SetBkColor, SetBkMode, SetTextColor, TRANSPARENT,
};
use winapi::um::winuser::{
    BS_DEFPUSHBUTTON, BM_SETSTYLE, COLOR_BTNTEXT, COLOR_GRAYTEXT, COLOR_WINDOW, DT_CENTER,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DrawTextW, GetDC, GetWindowRect, GetSysColor, GetSysColorBrush, GetWindowLongW,
    GWL_STYLE, MINMAXINFO, ReleaseDC, SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW, SetWindowLongW, SetWindowPos,
    SS_PATHELLIPSIS, WM_CTLCOLORSTATIC,
    WM_GETMINMAXINFO,
};

use super::shell::wide;
use super::startup::font_installed;

/// Outer margin of the window, in logical pixels (scaled by nwg for the DPI).
pub(super) const MARGIN: i32 = 12;
/// Gap between neighbouring controls.
pub(super) const GAP: i32 = 8;
/// Height of the search box and the buttons beside it.
pub(super) const QUERY_H: i32 = 36;
/// Height of every other control: buttons, the type filter, the check box.
pub(super) const CTRL_H: i32 = 32;
/// Narrowest a button gets, so short labels ("بحث") still give a good target.
const MIN_BUTTON_W: i32 = 88;
/// Space around a button's text (both sides together).
const BUTTON_PAD: i32 = 26;
/// Icon size on the buttons and the gap between icon and text.
const ICON: i32 = 16;
const ICON_GAP: i32 = 6;
/// Smallest window, so the controls never overlap.
const MIN_WINDOW: (i32, i32) = (640, 420);

/// Icons from Segoe Fluent Icons (Windows 11) or Segoe MDL2 Assets
/// (Windows 10); both fonts use the same code points.
pub(super) mod glyph {
    pub const SEARCH: char = '\u{E721}';
    pub const HISTORY: char = '\u{E81C}';
    pub const FOLDER: char = '\u{E8B7}';
    pub const DRIVE: char = '\u{EDA2}';
    pub const REFRESH: char = '\u{E72C}';
    pub const OPEN_FILE: char = '\u{E8E5}';
    pub const OPEN_FOLDER: char = '\u{E838}';
    pub const COPY: char = '\u{E8C8}';
    pub const EXPORT: char = '\u{EDE1}';
    pub const STOP: char = '\u{E71A}';
    pub const SETTINGS: char = '\u{E713}';
}

/// The icon font installed on this system, if any. Looked up once.
fn icon_font() -> Option<&'static str> {
    static FONT: std::sync::OnceLock<Option<&'static str>> = std::sync::OnceLock::new();
    *FONT.get_or_init(|| ["Segoe Fluent Icons", "Segoe MDL2 Assets"].into_iter().find(|f| font_installed(f)))
}

/// True when the buttons carry icons (an icon font is installed).
pub(super) fn has_icons() -> bool {
    icon_font().is_some()
}

/// Width of `text` drawn in `font`, in logical pixels.
pub(super) fn text_width(font: &nwg::Font, text: &str) -> i32 {
    text_size(font, text).0
}

/// Height of a line of text in `font`, in logical pixels.
pub(super) fn line_height(font: &nwg::Font) -> i32 {
    text_size(font, "Ay\u{0628}\u{0625}").1
}

/// Size of `text` drawn in `font`, in logical pixels.
fn text_size(font: &nwg::Font, text: &str) -> (i32, i32) {
    let w = wide(text);
    let mut size = SIZE { cx: 0, cy: 0 };
    unsafe {
        let dc = GetDC(std::ptr::null_mut());
        if dc.is_null() {
            return (text.chars().count() as i32 * 8, 20);
        }
        let old = SelectObject(dc, font.handle as HGDIOBJ);
        GetTextExtentPoint32W(dc, w.as_ptr(), (w.len().saturating_sub(1)) as i32, &mut size);
        SelectObject(dc, old);
        ReleaseDC(std::ptr::null_mut(), dc);
    }
    let s = nwg::scale_factor();
    ((size.cx as f64 / s).ceil() as i32, (size.cy as f64 / s).ceil() as i32)
}

/// The width a button needs for its text (and icon), in logical pixels.
pub(super) fn button_width(font: &nwg::Font, button: &nwg::Button) -> i32 {
    let icon = if has_icons() { ICON + ICON_GAP } else { 0 };
    (text_width(font, &button.text()) + BUTTON_PAD + icon).max(MIN_BUTTON_W)
}

/// The width a check box needs for its box and text.
pub(super) fn check_width(font: &nwg::Font, check: &nwg::CheckBox) -> i32 {
    // The box (13 px at 96 DPI) plus the gap Windows leaves after it.
    text_width(font, &check.text()) + 24
}

/// Gives `button` a small monochrome icon before its text. Does nothing
/// when no icon font is installed.
pub(super) fn set_button_icon(button: &nwg::Button, glyph: char) {
    let (Some(family), Some(hwnd)) = (icon_font(), button.handle.hwnd()) else { return };
    let px = (ICON as f64 * nwg::scale_factor()).round() as i32;
    let normal = unsafe { GetSysColor(COLOR_BTNTEXT) };
    let disabled = unsafe { GetSysColor(COLOR_GRAYTEXT) };
    let Some(list) = glyph_image_list(family, glyph, px, normal, disabled) else { return };
    let margin = (ICON_GAP as f64 * nwg::scale_factor()).round() as i32;
    let mut info = BUTTON_IMAGELIST {
        himl: list,
        margin: RECT { left: margin, top: 0, right: 0, bottom: 0 },
        uAlign: BUTTON_IMAGELIST_ALIGN_LEFT,
    };
    unsafe { SendMessageW(hwnd, BCM_SETIMAGELIST, 0, &mut info as *mut _ as LPARAM) };
}

/// An image list with the button states a themed button asks for (normal,
/// hot, pressed, disabled, default, stylus-hot), drawn from one glyph.
fn glyph_image_list(family: &str, glyph: char, px: i32, normal: u32, disabled: u32) -> Option<HIMAGELIST> {
    let list = unsafe { ImageList_Create(px, px, ILC_COLOR32, 6, 0) };
    if list.is_null() {
        return None;
    }
    for color in [normal, normal, normal, disabled, normal, normal] {
        let bmp = render_glyph(family, glyph, px, color)?;
        unsafe {
            ImageList_Add(list, bmp as _, std::ptr::null_mut());
            DeleteObject(bmp as HGDIOBJ);
        }
    }
    Some(list)
}

/// Draws `glyph` into a `px`×`px` 32-bit bitmap with per-pixel alpha.
///
/// GDI text has no alpha channel, so the glyph is drawn white on black with
/// grey-scale antialiasing and the brightness of each pixel becomes its
/// coverage; the colour is then premultiplied by it.
fn render_glyph(family: &str, glyph: char, px: i32, color: u32) -> Option<winapi::shared::windef::HBITMAP> {
    unsafe {
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = px;
        info.bmiHeader.biHeight = -px; // top-down
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let dc: HDC = CreateCompatibleDC(std::ptr::null_mut());
        if dc.is_null() {
            return None;
        }
        let mut bits: *mut winapi::ctypes::c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        if bmp.is_null() || bits.is_null() {
            DeleteDC(dc);
            return None;
        }
        let face = wide(family);
        let font = CreateFontW(
            -px,
            0,
            0,
            0,
            FW_NORMAL,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY,
            0,
            face.as_ptr(),
        );
        let old_bmp = SelectObject(dc, bmp as HGDIOBJ);
        let old_font = SelectObject(dc, font as HGDIOBJ);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, 0x00FF_FFFF);
        let mut buf = [0u16; 2];
        let text = glyph.encode_utf16(&mut buf);
        let mut r = RECT { left: 0, top: 0, right: px, bottom: px };
        DrawTextW(dc, text.as_ptr(), text.len() as i32, &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        SelectObject(dc, old_font);
        SelectObject(dc, old_bmp);
        DeleteObject(font as HGDIOBJ);
        DeleteDC(dc);

        let (cr, cg, cb) = (color & 0xFF, (color >> 8) & 0xFF, (color >> 16) & 0xFF);
        let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (px * px) as usize);
        for p in pixels.iter_mut() {
            // Coverage: the brightest channel of the white-on-black drawing.
            let a = (*p & 0xFF).max((*p >> 8) & 0xFF).max((*p >> 16) & 0xFF);
            *p = (a << 24) | ((cr * a / 255) << 16) | ((cg * a / 255) << 8) | (cb * a / 255);
        }
        Some(bmp)
    }
}

/// Marks `button` as the window's main action (the accent border Windows
/// draws around a default button).
pub(super) fn set_default_button(button: &nwg::Button) {
    if let Some(h) = button.handle.hwnd() {
        unsafe { SendMessageW(h, BM_SETSTYLE, BS_DEFPUSHBUTTON as WPARAM, 1) };
    }
}

/// Height of a control as Windows sized it, in logical pixels.
pub(super) fn height_of(control: &nwg::ControlHandle) -> Option<i32> {
    let hwnd = control.hwnd()?;
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    (unsafe { GetWindowRect(hwnd, &mut r) } != 0).then(|| ((r.bottom - r.top) as f64 / nwg::scale_factor()).round() as i32)
}

/// Shortens a long path in the middle ("C:\Users\…\Reports") instead of
/// cutting its end.
pub(super) fn set_path_ellipsis(label: &nwg::Label) {
    if let Some(h) = label.handle.hwnd() {
        unsafe {
            let style = GetWindowLongW(h, GWL_STYLE) as u32;
            SetWindowLongW(h, GWL_STYLE, (style | SS_PATHELLIPSIS) as i32);
        }
    }
}

/// Paints labels and check boxes on the window's own background (white)
/// instead of the grey dialog colour, and keeps the window from shrinking
/// below the size where its controls would overlap. `muted` labels are
/// drawn in a softer text colour.
pub(super) fn bind_window_style(
    window: &nwg::Window,
    muted: Vec<HWND>,
) -> Result<nwg::RawEventHandler, nwg::NwgError> {
    let main = window.handle.hwnd().unwrap_or(std::ptr::null_mut()) as usize;
    nwg::bind_raw_event_handler(
        &window.handle,
        0x1_0030,
        crate::crash_log::guard_raw(move |hwnd: HWND, msg: UINT, w: WPARAM, l: LPARAM| match msg {
            WM_CTLCOLORSTATIC if hwnd as usize == main => {
                let dc = w as HDC;
                unsafe {
                    SetBkMode(dc, TRANSPARENT as i32);
                    SetBkColor(dc, GetSysColor(COLOR_WINDOW));
                    if muted.contains(&(l as HWND)) {
                        SetTextColor(dc, muted_text_color());
                    }
                    Some(GetSysColorBrush(COLOR_WINDOW) as HBRUSH as LRESULT)
                }
            }
            WM_GETMINMAXINFO if l != 0 => {
                let scale = nwg::scale_factor();
                let info = unsafe { &mut *(l as *mut MINMAXINFO) };
                info.ptMinTrackSize.x = (MIN_WIDTH.load(std::sync::atomic::Ordering::Relaxed) as f64 * scale).round() as i32;
                info.ptMinTrackSize.y = (MIN_WINDOW.1 as f64 * scale).round() as i32;
                Some(0)
            }
            _ => None,
        }),
    )
}

/// Secondary text: halfway between the normal text and the background, so
/// it follows high-contrast and dark themes.
fn muted_text_color() -> u32 {
    let (t, b) = unsafe { (GetSysColor(COLOR_BTNTEXT), GetSysColor(COLOR_WINDOW)) };
    let mix = |shift: u32| ((((t >> shift) & 0xFF) * 3 + ((b >> shift) & 0xFF) * 2) / 5) << shift;
    mix(0) | mix(8) | mix(16)
}

/// Widths of the main window's controls, fitted to their text (logical px).
#[derive(Clone, Copy, Default)]
pub(super) struct Widths {
    pub search: i32,
    pub history: i32,
    pub check: i32,
    pub scope_label: i32,
    pub scope_btn: i32,
    pub all_btn: i32,
    pub reindex: i32,
    /// The action buttons along the bottom, left to right (in reading order).
    pub bottom: [i32; 5],
    pub options: i32,
}

/// Width of the file-type drop-down.
pub(super) const KIND_W: i32 = 168;
/// The scope text never gets narrower than this before it moves to its own row.
pub(super) const MIN_SCOPE_W: i32 = 140;

/// Smallest window width the layout needs; read by `WM_GETMINMAXINFO`.
static MIN_WIDTH: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(MIN_WINDOW.0);

/// Sets the smallest window width (logical px), never below the default.
pub(super) fn set_min_width(w: i32) {
    MIN_WIDTH.store(w.max(MIN_WINDOW.0), std::sync::atomic::Ordering::Relaxed);
}

/// Moves and sizes a control in one call (logical px, scaled for the DPI).
pub(super) fn place(control: &nwg::ControlHandle, x: i32, y: i32, w: i32, h: i32) {
    let Some(hwnd) = control.hwnd() else { return };
    let s = nwg::scale_factor();
    let px = |v: i32| (v as f64 * s).round() as i32;
    unsafe {
        SetWindowPos(hwnd, std::ptr::null_mut(), px(x), px(y), px(w.max(0)), px(h.max(0)), SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

/// Repaints a control's frame (non-client) area after a resize.
pub(super) fn repaint_frame(control: &nwg::ControlHandle) {
    use winapi::um::winuser::{RDW_FRAME, RDW_INVALIDATE, RedrawWindow};
    if let Some(hwnd) = control.hwnd() {
        unsafe { RedrawWindow(hwnd, std::ptr::null(), std::ptr::null_mut(), RDW_FRAME | RDW_INVALIDATE) };
    }
}
