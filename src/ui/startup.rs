//! Start-up timing and logging, background thread priority, the single
//! running copy, the screen work area and the user-interface font.

use native_windows_gui as nwg;
use std::cell::RefCell;
use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use winapi::shared::windef::RECT;
use winapi::um::winuser::{IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow};

use super::{TITLE, shell};
use crate::index;

/// Set once the message loop runs, so start-up timing ends at a real tick.
pub(super) static LOOP_STARTED: AtomicBool = AtomicBool::new(false);

/// The first installed font from a list of fonts known to render Arabic well.
/// Each name is looked up directly: listing every font on the system takes
/// seconds on a computer with many fonts installed.
pub(super) fn ui_font_family() -> &'static str {
    static FAMILY: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    const PREFERRED: [&str; 5] = ["Segoe UI", "Tahoma", "Arial", "Noto Sans Arabic", "Noto Naskh Arabic"];
    FAMILY.get_or_init(|| PREFERRED.iter().copied().find(|f| font_installed(f)).unwrap_or(PREFERRED[0]))
}

pub(super) fn font_installed(name: &str) -> bool {
    use winapi::shared::minwindef::{DWORD, LPARAM};
    use winapi::um::wingdi::{DEFAULT_CHARSET, EnumFontFamiliesExW, LOGFONTW, TEXTMETRICW};
    use winapi::um::winuser::{GetDC, ReleaseDC};

    unsafe extern "system" fn found(_: *const LOGFONTW, _: *const TEXTMETRICW, _: DWORD, flag: LPARAM) -> i32 {
        unsafe { *(flag as *mut bool) = true };
        0 // one match is enough
    }
    let mut lf: LOGFONTW = unsafe { std::mem::zeroed() };
    lf.lfCharSet = DEFAULT_CHARSET as u8;
    // The last unit stays 0 as the terminator.
    for (dst, src) in lf.lfFaceName.iter_mut().take(31).zip(shell::wide(name)) {
        *dst = src;
    }
    let mut ok = false;
    unsafe {
        let hdc = GetDC(std::ptr::null_mut());
        EnumFontFamiliesExW(hdc, &mut lf, Some(found), &mut ok as *mut bool as LPARAM, 0);
        ReleaseDC(std::ptr::null_mut(), hdc);
    }
    ok
}

thread_local! {
    /// Start-up phase timings, written to the log after the first paint.
    static PHASES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Milliseconds since Windows created this process, so the timings include
/// loading the program before `main`.
pub(super) fn ms_since_process_start() -> u64 {
    use winapi::shared::minwindef::FILETIME;
    use winapi::um::processthreadsapi::{GetCurrentProcess, GetProcessTimes};
    use winapi::um::sysinfoapi::GetSystemTimeAsFileTime;
    let as_u64 = |f: &FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    unsafe {
        let zero = || std::mem::zeroed::<FILETIME>();
        let (mut created, mut exited, mut kernel, mut user, mut now) = (zero(), zero(), zero(), zero(), zero());
        if GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut kernel, &mut user) == 0 {
            return 0;
        }
        GetSystemTimeAsFileTime(&mut now);
        as_u64(&now).saturating_sub(as_u64(&created)) / 10_000
    }
}

/// Records how long start-up took to reach `name`.
pub(super) fn phase(name: &str) {
    let ms = ms_since_process_start();
    PHASES.with(|p| p.borrow_mut().push(format!("{name}={ms}")));
}

/// Writes the recorded phases to the log, off the UI thread.
pub(super) fn log_phases() {
    let marks = PHASES.with(|p| std::mem::take(&mut *p.borrow_mut()).join(" "));
    if !marks.is_empty() {
        std::thread::spawn(move || log(&format!("startup ms (v{}): {marks}", env!("CARGO_PKG_VERSION"))));
    }
}

/// The usable screen size (without the taskbar) in logical pixels.
pub(super) fn work_area() -> (u32, u32) {
    use winapi::um::winuser::{SPI_GETWORKAREA, SystemParametersInfoW};
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    let ok = unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut r as *mut RECT as _, 0) } != 0;
    if !ok || r.right <= r.left {
        return (u32::MAX, u32::MAX);
    }
    let scale = nwg::scale_factor();
    // Leave room for the window frame and title bar.
    let w = ((r.right - r.left) as f64 / scale) as u32;
    let h = ((r.bottom - r.top) as f64 / scale) as u32;
    (w.saturating_sub(16).max(400), h.saturating_sub(40).max(300))
}

/// Lets the user interface win when a background thread competes for the CPU.
pub(super) fn background_priority() {
    use winapi::um::processthreadsapi::{GetCurrentThread, SetThreadPriority};
    use winapi::um::winbase::THREAD_PRIORITY_BELOW_NORMAL;
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL as i32) };
}

/// Appends a timestamped line to `%LOCALAPPDATA%\BahthSari\log.txt`, kept short.
pub(super) fn log(line: &str) {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock();
    let path = index::cache_path().with_file_name("log.txt");
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 64 * 1024) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{} {line}", index::now_secs());
    }
}

/// If the program is already running, brings its window forward and returns true.
pub(super) fn activate_running_copy(updated: bool) -> bool {
    use winapi::shared::winerror::ERROR_ALREADY_EXISTS;
    use winapi::um::errhandlingapi::GetLastError;
    use winapi::um::synchapi::CreateMutexW;
    use winapi::um::winuser::{FindWindowW, SW_SHOW};

    let name = shell::wide(crate::update::SINGLE_INSTANCE_MUTEX);
    // The mutex lives as long as this process; Windows frees it on exit.
    let mutex = unsafe { CreateMutexW(std::ptr::null_mut(), 0, name.as_ptr()) };
    if mutex.is_null() || unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
        return false;
    }
    // The first copy may still be creating its window; give it a few seconds.
    let (class, title) = (shell::wide("NativeWindowsGuiWindow"), shell::wide(TITLE));
    let mut hwnd = std::ptr::null_mut();
    for _ in 0..50 {
        hwnd = unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) };
        if !hwnd.is_null() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if hwnd.is_null() {
        // Right after an update the old copy may hold the mutex without a
        // window while it exits; never leave the user with no program then.
        log(&format!("start: another copy holds the lock, no window found (updated={updated})"));
        return !updated;
    }
    log("start: another copy is running; brought it to the front");
    unsafe {
        ShowWindow(hwnd, if IsIconic(hwnd) != 0 { SW_RESTORE } else { SW_SHOW });
        SetForegroundWindow(hwnd);
    }
    true
}
