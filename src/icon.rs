//! Loads the application icon (resource id 1, embedded by build.rs) for windows and the tray.

#![cfg(windows)]

use std::ptr::null;
use winapi::shared::minwindef::{LPARAM, WPARAM};
use winapi::shared::windef::{HICON, HWND};
use winapi::um::libloaderapi::GetModuleHandleW;
use winapi::um::winuser::{
    GetSystemMetrics, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, MAKEINTRESOURCEW, SM_CXICON,
    SM_CXSMICON, SM_CYICON, SM_CYSMICON, SendMessageW, WM_SETICON,
};

/// Loads icon resource 1 from the executable at the system small or large icon size.
/// Returns a null handle on failure.
pub fn load_icon(small: bool) -> HICON {
    unsafe {
        let (cx, cy) = if small {
            (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON))
        } else {
            (GetSystemMetrics(SM_CXICON), GetSystemMetrics(SM_CYICON))
        };
        let module = GetModuleHandleW(null());
        if module.is_null() {
            return std::ptr::null_mut();
        }
        LoadImageW(module, MAKEINTRESOURCEW(1), IMAGE_ICON, cx, cy, LR_DEFAULTCOLOR) as HICON
    }
}

/// Sets both the title-bar/Alt-Tab (small) and taskbar (big) icons of a window.
pub fn set_window_icon(hwnd: HWND) {
    if hwnd.is_null() {
        return;
    }
    let big = load_icon(false);
    let small = load_icon(true);
    unsafe {
        if !big.is_null() {
            SendMessageW(hwnd, WM_SETICON, ICON_BIG as WPARAM, big as LPARAM);
        }
        if !small.is_null() {
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL as WPARAM, small as LPARAM);
        }
    }
}
