//! The custom-drawn windows of the quick search (the popup and the floating
//! box): their window class and procedure, drawing with real transparency,
//! the DWM look (rounded corners, dark mode, acrylic), the Windows theme,
//! and the monitor and taskbar the box is placed by.
//!
//! GDI has no alpha channel, so translucent drawing is done in two passes:
//! the same text and icons are drawn on black and on white, and
//! [`crate::quick::matte`] recovers colour and coverage from the
//! difference. Backgrounds and highlights are computed per pixel in Rust.
//! Everything here falls back quietly: a call that fails leaves the window
//! with a plain, solid look.
//!
//! These windows are not mirrored with `WS_EX_LAYOUTRTL`: a mirrored DC
//! flips bitmaps copied into it, which would garble the double-buffered and
//! layered drawing. Their layout is mirrored by hand instead
//! ([`crate::quick::mirror_x`]) and Arabic text is drawn with
//! `DT_RTLREADING`, so the result reads exactly like a mirrored window.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use winapi::shared::minwindef::{BOOL, DWORD, HKEY, LPARAM, LRESULT, TRUE, UINT, WPARAM};
use winapi::shared::windef::{HBITMAP, HDC, HFONT, HGDIOBJ, HICON, HWND, POINT, RECT};
use winapi::um::wingdi::{
    ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleDC,
    CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiFlush, OUT_DEFAULT_PRECIS,
    SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use winapi::um::winuser::{
    DT_END_ELLIPSIS, DT_LEFT, DT_NOCLIP, DT_NOPREFIX, DT_RIGHT, DT_RTLREADING, DT_SINGLELINE, DT_VCENTER, DefWindowProcW, DrawTextW,
    GWLP_USERDATA, GetWindowLongPtrW, SetWindowLongPtrW,
};

use super::App;
use super::shell::wide;
use crate::quick::{Edge, Rect, Taskbar};

/// Posted to the popup when a background file search finished.
pub(super) const WM_APP_FILES: UINT = winapi::um::winuser::WM_APP + 0x51;
/// Posted to the popup when the launcher catalog (apps, settings) is ready.
pub(super) const WM_APP_CATALOG: UINT = winapi::um::winuser::WM_APP + 0x52;
/// Posted to the popup when a system command run in the background ended.
pub(super) const WM_APP_SYSTEM: UINT = winapi::um::winuser::WM_APP + 0x53;

/// Which window a message is for (kept in `GWLP_USERDATA`).
pub(super) const KIND_POPUP: isize = 1;
pub(super) const KIND_FLOATING: isize = 2;

thread_local! {
    /// The application, for the window procedure (UI thread only).
    static APP: RefCell<Weak<App>> = const { RefCell::new(Weak::new()) };
}

/// Lets the window procedure reach the application. Called once in `build_ui`.
pub(super) fn set_app(app: &Rc<App>) {
    APP.with(|a| *a.borrow_mut() = Rc::downgrade(app));
}

pub(super) fn app() -> Option<Rc<App>> {
    APP.with(|a| a.borrow().upgrade())
}

/// The shared window procedure. A panic is reported and the message gets
/// its default handling, never unwinding into Windows.
unsafe extern "system" fn proc(hwnd: HWND, msg: UINT, w: WPARAM, l: LPARAM) -> LRESULT {
    let kind = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as isize;
    let answer = crate::crash_log::catch(|| {
        let app = app()?;
        match kind {
            KIND_POPUP => app.popup_message(hwnd, msg, w, l),
            KIND_FLOATING => app.floating_message(hwnd, msg, w, l),
            _ => None,
        }
    })
    .flatten();
    match answer {
        Some(r) => r,
        None => unsafe { DefWindowProcW(hwnd, msg, w, l) },
    }
}

const POPUP_CLASS: &str = "InstaFindQuickSearch";
const BOX_CLASS: &str = "InstaFindFloatingBox";

/// Registers the two window classes once. The popup's class asks for a
/// drop shadow on Windows 10 (Windows 11 shadows rounded windows itself).
fn register_classes() -> bool {
    use std::sync::atomic::{AtomicU8, Ordering};
    use winapi::um::libloaderapi::GetModuleHandleW;
    use winapi::um::winuser::{CS_DROPSHADOW, IDC_ARROW, LoadCursorW, RegisterClassExW, WNDCLASSEXW};
    // 0 = not yet, 1 = registered, 2 = failed.
    static DONE: AtomicU8 = AtomicU8::new(0);
    match DONE.load(Ordering::Relaxed) {
        1 => return true,
        2 => return false,
        _ => {}
    }
    let mut ok = true;
    // Windows 11 shadows the rounded popup itself (a class shadow would
    // stay square there).
    let shadow = if crate::quick::has_round_corners(windows_build()) { 0 } else { CS_DROPSHADOW };
    for (name, style) in [(POPUP_CLASS, shadow), (BOX_CLASS, 0)] {
        let class = wide(name);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as UINT,
            style,
            lpfnWndProc: Some(proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: unsafe { GetModuleHandleW(std::ptr::null()) },
            hIcon: std::ptr::null_mut(),
            hCursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) },
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
            hIconSm: std::ptr::null_mut(),
        };
        ok &= unsafe { RegisterClassExW(&wc) } != 0;
    }
    DONE.store(if ok { 1 } else { 2 }, Ordering::Relaxed);
    ok
}

/// Creates a hidden popup window of `kind`; `None` when Windows refuses.
pub(super) fn create_window(kind: isize, ex_style: DWORD, title: &str) -> Option<HWND> {
    use winapi::um::libloaderapi::GetModuleHandleW;
    use winapi::um::winuser::{CreateWindowExW, WS_CLIPCHILDREN, WS_POPUP};
    if !register_classes() {
        return None;
    }
    let class = wide(if kind == KIND_POPUP { POPUP_CLASS } else { BOX_CLASS });
    let title = wide(title);
    let hwnd = unsafe {
        CreateWindowExW(
            ex_style,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP | WS_CLIPCHILDREN,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null_mut(),
        )
    };
    if hwnd.is_null() {
        return None;
    }
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, kind as _) };
    Some(hwnd)
}

/// Brings `hwnd` to the front with the keyboard focus. The hotkey, a click
/// on the box and the tray icon all give this program the right to; should
/// Windows refuse anyway, the input of the window in front is joined for a
/// moment, the usual way round the foreground lock.
pub(super) fn bring_to_front(hwnd: HWND) {
    use winapi::um::processthreadsapi::GetCurrentThreadId;
    use winapi::um::winuser::{AttachThreadInput, BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow};
    unsafe {
        SetForegroundWindow(hwnd);
        let front = GetForegroundWindow();
        if front == hwnd || front.is_null() {
            return;
        }
        let theirs = GetWindowThreadProcessId(front, std::ptr::null_mut());
        let mine = GetCurrentThreadId();
        if theirs != 0 && theirs != mine && AttachThreadInput(mine, theirs, TRUE) != 0 {
            BringWindowToTop(hwnd);
            SetForegroundWindow(hwnd);
            AttachThreadInput(mine, theirs, 0);
        }
    }
}

// ------------------------------------------------------------------ theme

fn reg_dword(root: HKEY, key: &str, name: &str) -> Option<u32> {
    use winapi::shared::winerror::ERROR_SUCCESS;
    use winapi::um::winreg::{RRF_RT_REG_DWORD, RegGetValueW};
    let (key, name) = (wide(key), wide(name));
    let mut value: DWORD = 0;
    let mut len = std::mem::size_of::<DWORD>() as DWORD;
    let r = unsafe {
        RegGetValueW(root, key.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&mut value as *mut DWORD).cast(), &mut len)
    };
    (r == ERROR_SUCCESS as i32).then_some(value)
}

const PERSONALIZE: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize";

/// Apps use the dark theme (Settings ▸ Personalisation ▸ Colours).
pub(super) fn apps_dark() -> bool {
    reg_dword(winapi::um::winreg::HKEY_CURRENT_USER, PERSONALIZE, "AppsUseLightTheme") == Some(0)
}

/// The taskbar is dark: it is unless Windows says it is light (Windows 10
/// before 1903 has no light taskbar and no value).
pub(super) fn taskbar_dark() -> bool {
    reg_dword(winapi::um::winreg::HKEY_CURRENT_USER, PERSONALIZE, "SystemUsesLightTheme") != Some(1)
}

/// Transparency effects are on (the default).
pub(super) fn transparency_on() -> bool {
    reg_dword(winapi::um::winreg::HKEY_CURRENT_USER, PERSONALIZE, "EnableTransparency") != Some(0)
}

/// High contrast is on: then nothing is translucent.
pub(super) fn high_contrast() -> bool {
    use winapi::um::winuser::{HCF_HIGHCONTRASTON, HIGHCONTRASTW, SPI_GETHIGHCONTRAST, SystemParametersInfoW};
    let mut hc: HIGHCONTRASTW = unsafe { std::mem::zeroed() };
    hc.cbSize = std::mem::size_of::<HIGHCONTRASTW>() as UINT;
    let ok = unsafe { SystemParametersInfoW(SPI_GETHIGHCONTRAST, hc.cbSize, (&mut hc as *mut HIGHCONTRASTW).cast(), 0) } != 0;
    ok && hc.dwFlags & HCF_HIGHCONTRASTON != 0
}

/// The Windows build number (19045, 22631…), 0 when unknown. Read once.
pub(super) fn windows_build() -> u32 {
    static BUILD: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *BUILD.get_or_init(|| {
        use winapi::shared::winerror::ERROR_SUCCESS;
        use winapi::um::winreg::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
        let key = wide("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion");
        let name = wide("CurrentBuildNumber");
        let mut buf = [0u16; 32];
        let mut len = std::mem::size_of_val(&buf) as DWORD;
        let r = unsafe {
            RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len)
        };
        if r != ERROR_SUCCESS as i32 {
            return 0;
        }
        let n = (len as usize / 2).min(buf.len());
        String::from_utf16_lossy(&buf[..n]).trim_end_matches('\0').trim().parse().unwrap_or(0)
    })
}

/// Whether `l` of a `WM_SETTINGCHANGE` names the light/dark switch.
pub(super) fn is_theme_change(l: LPARAM) -> bool {
    l != 0 && unsafe { super::format::wide_to_string(l as *const u16) } == "ImmersiveColorSet"
}

/// Asks DWM for the Windows 11 look: dark title colours, rounded corners
/// and, when `backdrop`, the transient (acrylic) backdrop behind the client
/// area. Older Windows ignores the attributes it does not know.
pub(super) fn dwm_style(hwnd: HWND, dark: bool, round: bool, backdrop: bool) {
    use winapi::um::dwmapi::DwmSetWindowAttribute;
    const DWMWA_USE_IMMERSIVE_DARK_MODE: DWORD = 20;
    const DWMWA_WINDOW_CORNER_PREFERENCE: DWORD = 33;
    const DWMWA_SYSTEMBACKDROP_TYPE: DWORD = 38;
    const DWMWCP_ROUND: u32 = 2;
    const DWMSBT_NONE: u32 = 1;
    const DWMSBT_TRANSIENTWINDOW: u32 = 3;
    let set = |attr: DWORD, value: u32| unsafe {
        DwmSetWindowAttribute(hwnd, attr, (&value as *const u32).cast(), std::mem::size_of::<u32>() as DWORD);
    };
    set(DWMWA_USE_IMMERSIVE_DARK_MODE, dark as u32);
    if round {
        set(DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND);
    }
    if crate::quick::has_backdrop(windows_build()) {
        set(DWMWA_SYSTEMBACKDROP_TYPE, if backdrop { DWMSBT_TRANSIENTWINDOW } else { DWMSBT_NONE });
    }
}

/// Lets the backdrop show through the top `height` pixels of the client
/// area (0 = none): there the alpha of what is drawn counts.
pub(super) fn extend_frame(hwnd: HWND, height: i32) {
    use winapi::um::dwmapi::DwmExtendFrameIntoClientArea;
    use winapi::um::uxtheme::MARGINS;
    let m = MARGINS { cxLeftWidth: 0, cxRightWidth: 0, cyTopHeight: height.max(0), cyBottomHeight: 0 };
    unsafe { DwmExtendFrameIntoClientArea(hwnd, &m) };
}

// ------------------------------------------------------------------ monitors and taskbar

pub(super) fn rect(r: &RECT) -> Rect {
    Rect::new(r.left, r.top, r.right, r.bottom)
}

pub(super) fn win_rect(r: &Rect) -> RECT {
    RECT { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
}

/// The monitor and work area (without the taskbar) at a screen point,
/// or of the nearest monitor.
pub(super) fn monitor_at(x: i32, y: i32) -> (Rect, Rect) {
    use winapi::um::winuser::{MONITOR_DEFAULTTONEAREST, MonitorFromPoint};
    monitor_info(unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) })
}

/// The primary monitor and its work area.
pub(super) fn primary_monitor() -> (Rect, Rect) {
    use winapi::um::winuser::{MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint};
    monitor_info(unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) })
}

/// The monitor and work area of the window the user is working in.
pub(super) fn active_monitor() -> (Rect, Rect) {
    use winapi::um::winuser::{GetCursorPos, GetForegroundWindow, MONITOR_DEFAULTTONEAREST, MonitorFromWindow};
    let fg = unsafe { GetForegroundWindow() };
    if !fg.is_null() {
        return monitor_info(unsafe { MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST) });
    }
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };
    monitor_at(pt.x, pt.y)
}

fn monitor_info(mon: winapi::shared::windef::HMONITOR) -> (Rect, Rect) {
    use winapi::um::winuser::{GetMonitorInfoW, GetSystemMetrics, MONITORINFO, SM_CXSCREEN, SM_CYSCREEN};
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<MONITORINFO>() as DWORD;
    if !mon.is_null() && unsafe { GetMonitorInfoW(mon, &mut info) } != 0 {
        return (rect(&info.rcMonitor), rect(&info.rcWork));
    }
    let r = unsafe { Rect::new(0, 0, GetSystemMetrics(SM_CXSCREEN).max(640), GetSystemMetrics(SM_CYSCREEN).max(480)) };
    (r, r)
}

/// The rectangles of all monitors.
pub(super) fn monitors() -> Vec<Rect> {
    use winapi::shared::windef::HMONITOR;
    use winapi::um::winuser::EnumDisplayMonitors;
    unsafe extern "system" fn add(_: HMONITOR, _: HDC, r: *mut RECT, data: LPARAM) -> BOOL {
        if !r.is_null() && data != 0 {
            let list = unsafe { &mut *(data as *mut Vec<Rect>) };
            list.push(rect(unsafe { &*r }));
        }
        TRUE
    }
    let mut list: Vec<Rect> = Vec::new();
    unsafe { EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), Some(add), &mut list as *mut Vec<Rect> as LPARAM) };
    list
}

/// The main taskbar window (`Shell_TrayWnd`), if Explorer runs.
pub(super) fn taskbar_window() -> Option<HWND> {
    use winapi::um::winuser::FindWindowW;
    let class = wide("Shell_TrayWnd");
    let h = unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) };
    (!h.is_null()).then_some(h)
}

/// Where the taskbar's window is now (an auto-hide taskbar moves).
pub(super) fn taskbar_window_rect() -> Option<Rect> {
    use winapi::um::winuser::GetWindowRect;
    let h = taskbar_window()?;
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    (unsafe { GetWindowRect(h, &mut r) } != 0).then(|| rect(&r))
}

/// The taskbar: its edge and rectangle (`ABM_GETTASKBARPOS`), and which
/// end the clock is at. `None` without Explorer.
pub(super) fn taskbar() -> Option<Taskbar> {
    use winapi::um::shellapi::{ABM_GETTASKBARPOS, APPBARDATA, SHAppBarMessage};
    use winapi::um::winuser::{FindWindowExW, GWL_EXSTYLE, GetWindowRect};
    let tray = taskbar_window()?;
    let mut abd: APPBARDATA = unsafe { std::mem::zeroed() };
    abd.cbSize = std::mem::size_of::<APPBARDATA>() as DWORD;
    abd.hWnd = tray;
    if unsafe { SHAppBarMessage(ABM_GETTASKBARPOS, &mut abd) } == 0 {
        return None;
    }
    let (rc, edge) = (abd.rc, abd.uEdge);
    let bar = rect(&rc);
    let edge = Edge::from_abe(edge);
    // The clock end: where the notification area is, else the taskbar's
    // own mirroring (a right-to-left Windows).
    let notify_class = wide("TrayNotifyWnd");
    let notify = unsafe { FindWindowExW(tray, std::ptr::null_mut(), notify_class.as_ptr(), std::ptr::null()) };
    let mut nr = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    let found = !notify.is_null() && unsafe { GetWindowRect(notify, &mut nr) } != 0;
    // Only a notification area that really lies on the taskbar tells.
    let clock_left = if edge.horizontal() && found && rect(&nr).intersect(&bar).width() > 0 {
        (nr.left + nr.right) / 2 < bar.center().0
    } else {
        let ex = unsafe { GetWindowLongPtrW(tray, GWL_EXSTYLE) } as u32;
        ex & super::WS_EX_LAYOUTRTL != 0
    };
    Some(Taskbar { rect: bar, edge, clock_left })
}

/// The taskbar hides itself until pointed at.
pub(super) fn taskbar_autohide() -> bool {
    use winapi::um::shellapi::{ABM_GETSTATE, ABS_AUTOHIDE, APPBARDATA, SHAppBarMessage};
    let mut abd: APPBARDATA = unsafe { std::mem::zeroed() };
    abd.cbSize = std::mem::size_of::<APPBARDATA>() as DWORD;
    (unsafe { SHAppBarMessage(ABM_GETSTATE, &mut abd) } as UINT) & ABS_AUTOHIDE != 0
}

/// A full-screen program, game, video or presentation is in front.
pub(super) fn busy_full_screen() -> bool {
    use winapi::um::shellapi::{QUERY_USER_NOTIFICATION_STATE, SHQueryUserNotificationState};
    let mut state: QUERY_USER_NOTIFICATION_STATE = 0;
    let hr = unsafe { SHQueryUserNotificationState(&mut state) };
    hr >= 0 && crate::quick::hides_for_notification_state(state as i32)
}

// ------------------------------------------------------------------ drawing

/// A 32-bit top-down DIB selected into a memory DC.
pub(super) struct Canvas {
    pub dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    pub w: i32,
    pub h: i32,
}

impl Canvas {
    pub fn new(w: i32, h: i32) -> Option<Canvas> {
        let (w, h) = (w.max(1), h.max(1));
        unsafe {
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as DWORD;
            info.bmiHeader.biWidth = w;
            info.bmiHeader.biHeight = -h; // top-down
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            if dc.is_null() {
                return None;
            }
            let mut bits: *mut winapi::ctypes::c_void = std::ptr::null_mut();
            let bmp = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            if bmp.is_null() || bits.is_null() {
                if !bmp.is_null() {
                    DeleteObject(bmp as HGDIOBJ);
                }
                DeleteDC(dc);
                return None;
            }
            let old = SelectObject(dc, bmp as HGDIOBJ);
            SetBkMode(dc, TRANSPARENT as i32);
            Some(Canvas { dc, bmp, old, bits: bits as *mut u32, w, h })
        }
    }

    /// The pixels (`0xAARRGGBB`), after GDI finished drawing.
    pub fn pixels(&mut self) -> &mut [u32] {
        unsafe {
            GdiFlush();
            std::slice::from_raw_parts_mut(self.bits, (self.w as usize) * (self.h as usize))
        }
    }

    pub fn fill(&mut self, p: u32) {
        self.pixels().fill(p);
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bmp as HGDIOBJ);
            DeleteDC(self.dc);
        }
    }
}

/// Puts what `content` draws with GDI over the premultiplied `base`
/// pixels into `out`. `opaque`: the base is solid and `content` is drawn on
/// it directly (ClearType); otherwise it is drawn twice and matted, so the
/// result keeps real transparency.
pub(super) fn compose(out: &mut Canvas, base: &[u32], opaque: bool, content: &dyn Fn(HDC)) {
    let n = out.pixels().len();
    if base.len() != n {
        return;
    }
    if opaque {
        out.pixels().copy_from_slice(base);
        content(out.dc);
        unsafe { GdiFlush() };
        return;
    }
    let (Some(mut black), Some(mut white)) = (Canvas::new(out.w, out.h), Canvas::new(out.w, out.h)) else {
        out.pixels().copy_from_slice(base);
        return;
    };
    black.fill(0);
    white.fill(0x00FF_FFFF);
    content(black.dc);
    content(white.dc);
    let (b, w) = (black.pixels(), white.pixels());
    let dst = out.pixels();
    for i in 0..n {
        dst[i] = crate::quick::over(crate::quick::matte(b[i], w[i]), base[i]);
    }
}

/// A GDI font, deleted with its owner.
pub(super) struct Font(pub HFONT);

impl Font {
    /// `ui_font_family` (or `family`) at `px` pixels; `smooth` asks for
    /// greyscale antialiasing, which mattes cleanly (ClearType colours the
    /// edges of text on a transparent background).
    pub fn new(family: &str, px: i32, weight: i32, smooth: bool) -> Font {
        let face = wide(family);
        let quality = if smooth { ANTIALIASED_QUALITY } else { CLEARTYPE_QUALITY };
        let f = unsafe {
            CreateFontW(-px.max(1), 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, quality, 0, face.as_ptr())
        };
        Font(f)
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { DeleteObject(self.0 as HGDIOBJ) };
        }
    }
}

impl Default for Font {
    fn default() -> Font {
        Font(std::ptr::null_mut())
    }
}

/// Draws one line of `text` in `r`, cut with "…" when too long: at the
/// reading start (right in Arabic), or at the end when `far`.
pub(super) fn draw_text(dc: HDC, font: &Font, color: u32, text: &str, r: &Rect, rtl: bool, far: bool) {
    if text.is_empty() || r.is_empty() {
        return;
    }
    let w: Vec<u16> = text.encode_utf16().collect();
    let mut rc = win_rect(r);
    let right = rtl != far;
    // Not clipped to `r`: the rows' text boxes are a half row high, and the
    // dots of a final ي (and other Arabic marks below the line) reach under
    // the font's descent, so a clipped box drew «الطبي» as «الطبى». The
    // ellipsis still keeps the text inside the box's width.
    let mut flags = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_NOCLIP | DT_END_ELLIPSIS | if right { DT_RIGHT } else { DT_LEFT };
    if rtl {
        flags |= DT_RTLREADING;
    }
    unsafe {
        let old = SelectObject(dc, font.0 as HGDIOBJ);
        SetTextColor(dc, bgr(color));
        SetBkMode(dc, TRANSPARENT as i32);
        DrawTextW(dc, w.as_ptr(), w.len() as i32, &mut rc, flags);
        SelectObject(dc, old);
    }
}

/// Draws a single glyph centred in `r`.
pub(super) fn draw_glyph(dc: HDC, font: &Font, color: u32, glyph: char, r: &Rect) {
    use winapi::um::winuser::DT_CENTER;
    let mut buf = [0u16; 2];
    let s = glyph.encode_utf16(&mut buf);
    let mut rc = win_rect(r);
    unsafe {
        let old = SelectObject(dc, font.0 as HGDIOBJ);
        SetTextColor(dc, bgr(color));
        SetBkMode(dc, TRANSPARENT as i32);
        DrawTextW(dc, s.as_ptr(), s.len() as i32, &mut rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        SelectObject(dc, old);
    }
}

/// Draws an icon at its own size with its top-left at (`x`, `y`).
pub(super) fn draw_icon(dc: HDC, icon: HICON, x: i32, y: i32, size: i32) {
    use winapi::um::winuser::DrawIconEx;
    const DI_NORMAL: UINT = 0x0003;
    if !icon.is_null() {
        unsafe { DrawIconEx(dc, x, y, icon, size, size, 0, std::ptr::null_mut(), DI_NORMAL) };
    }
}

/// `0x00RRGGBB` as a GDI `COLORREF` (`0x00BBGGRR`).
pub(super) fn bgr(rgb: u32) -> u32 {
    ((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF)
}

/// Fills `r` of a `w` wide pixel buffer with `p` over what is there,
/// rounded with corner radius `radius` (0 for square).
pub(super) fn fill_rounded(px: &mut [u32], w: i32, r: &Rect, radius: f32, p: u32) {
    let h = (px.len() as i32) / w.max(1);
    let clip = r.intersect(&Rect::new(0, 0, w, h));
    for y in clip.top..clip.bottom {
        for x in clip.left..clip.right {
            let cov = crate::quick::rounded_coverage(x - r.left, y - r.top, r.width(), r.height(), radius);
            if cov > 0 {
                let i = (y * w + x) as usize;
                px[i] = crate::quick::over(crate::quick::fade(p, cov), px[i]);
            }
        }
    }
}

/// The interface font family (Segoe UI…), as the main window uses.
pub(super) fn ui_family() -> &'static str {
    super::startup::ui_font_family()
}

/// The icon font (Segoe Fluent Icons or Segoe MDL2 Assets), if installed.
pub(super) fn icon_family() -> Option<&'static str> {
    super::style::icon_font()
}
