//! Keeps the main window on screen: centred in the work area (the monitor
//! without the taskbar) when it opens, and pulled back when the screen it
//! was placed for is gone or smaller. The arithmetic is in
//! `crate::window_fit`, tested on every platform.

use native_windows_gui as nwg;
use winapi::shared::windef::{HWND, RECT};
use winapi::um::winuser::{
    AdjustWindowRectEx, GetSystemMetrics, GetWindowRect, IsIconic, IsZoomed, SM_CXPADDEDBORDER, SM_CXSIZEFRAME,
    SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER, SetWindowPos, WS_OVERLAPPEDWINDOW,
};

use super::surface;
use crate::quick::Rect;
use crate::window_fit;

/// How far a resizable window's rectangle reaches past the frame one sees,
/// on the left, right and bottom: Windows 10 and 11 draw a 1-pixel border
/// and keep the rest of the resize frame invisible.
fn hidden_frame() -> i32 {
    (unsafe { GetSystemMetrics(SM_CXSIZEFRAME) + GetSystemMetrics(SM_CXPADDEDBORDER) } - 1).max(0)
}

/// Where the main window opens, for the content size it wants (logical
/// pixels): the content size to create it with and its top-left corner
/// (both logical, as nwg takes them), centred in the primary monitor's work
/// area and fully inside it. nwg's own centring uses the whole monitor,
/// which put the status line under the taskbar on a 768-pixel screen.
pub(super) fn opening(width: u32, height: u32) -> ((i32, i32), (i32, i32)) {
    let scale = nwg::scale_factor();
    let px = |v: u32| (v as f64 * scale).round() as i32;
    let (cw, ch) = (px(width), px(height));
    // The frame and title bar nwg adds around the content.
    let mut r = RECT { left: 0, top: 0, right: cw, bottom: ch };
    unsafe { AdjustWindowRectEx(&mut r, WS_OVERLAPPEDWINDOW, 0, 0) };
    let (fw, fh) = ((r.right - r.left - cw).max(0), (r.bottom - r.top - ch).max(0));
    let (_, work) = surface::primary_monitor();
    let area = window_fit::with_hidden_frame(&work, hidden_frame());
    let at = window_fit::place(&area, (cw + fw, ch + fh), None);
    let logical = |v: i32| (v as f64 / scale).floor() as i32;
    ((logical(at.width() - fw).max(100), logical(at.height() - fh).max(100)), (logical(at.left), logical(at.top)))
}

/// Moves (and, when it is larger than the screen, shrinks) the window so
/// all of it is in the work area of the monitor it is on. A maximized or
/// minimized window is left alone, and so is one that is already inside.
pub(super) fn keep_on_screen(hwnd: HWND) {
    if unsafe { IsZoomed(hwnd) != 0 || IsIconic(hwnd) != 0 } {
        return;
    }
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    if unsafe { GetWindowRect(hwnd, &mut r) } == 0 {
        return;
    }
    let now: Rect = surface::rect(&r);
    let (cx, cy) = now.center();
    let (_, work) = surface::monitor_at(cx, cy);
    let area = window_fit::with_hidden_frame(&work, hidden_frame());
    let at = window_fit::place(&area, (now.width(), now.height()), Some((now.left, now.top)));
    if at != now {
        let flags = SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER;
        unsafe { SetWindowPos(hwnd, std::ptr::null_mut(), at.left, at.top, at.width(), at.height(), flags) };
    }
}
