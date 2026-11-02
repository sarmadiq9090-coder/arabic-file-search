//! Panic reports, so a crash on a user's computer can be diagnosed.
//!
//! The program has no console, so the default panic message goes nowhere.
//! [`install`] adds a panic hook that appends the message, where it
//! happened, the thread and a backtrace to `log.txt` (and to `crash.txt`,
//! which the size cap on the log does not empty). On Windows the backtrace
//! has no symbol names (the release build is stripped); each frame inside
//! the program also gets its offset (`bahth.exe+0x…`), which a build of the
//! same version with symbols turns back into a function.
//!
//! A panic in a window procedure cannot unwind into Windows: the process
//! aborts. [`guard3`] and [`guard4`] wrap the event handlers so a bug in one
//! of them is logged and that one event is dropped instead.
//!
//! Parsers of documents from the disk are expected to panic now and then on
//! damaged files; [`quietly`] contains those and logs one short line each,
//! without the cost of a backtrace.

use std::cell::Cell;
use std::fmt::Write as _;
use std::io::Write as _;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::JoinHandle;

#[cfg(windows)]
use native_windows_gui as nwg;
#[cfg(windows)]
use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
#[cfg(windows)]
use winapi::shared::windef::HWND;

/// `crash.txt` starts afresh past this size.
const CRASH_MAX: u64 = 256 * 1024;
/// `log.txt` is emptied by the regular log past this size (see
/// `ui::startup::log`); a report that would cross it moves the old log aside
/// first, so the report survives.
const LOG_MAX: u64 = 64 * 1024;
/// Expected (quiet) panics logged per run; later ones are only counted.
const QUIET_LINES: usize = 20;

thread_local! {
    /// Inside [`quietly`]: a panic here is expected.
    static QUIET: Cell<bool> = const { Cell::new(false) };
    /// Inside the hook already (a panic while reporting one).
    static IN_HOOK: Cell<bool> = const { Cell::new(false) };
}

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static QUIET_COUNT: AtomicUsize = AtomicUsize::new(0);

/// `%LOCALAPPDATA%\BahthSari\log.txt`, next to the index.
#[cfg(windows)]
pub fn default_log_path() -> PathBuf {
    crate::index::cache_path().with_file_name("log.txt")
}

/// `crash.txt`, where the panic reports are kept (see `feedback`).
#[cfg(windows)]
pub fn crash_path() -> PathBuf {
    default_log_path().with_file_name("crash.txt")
}

/// Installs the panic hook, writing to `log` (and `crash.txt` beside it).
/// The previous hook still runs after it. Only the first call counts.
pub fn install(log: PathBuf) {
    if LOG_PATH.set(log).is_err() {
        return;
    }
    #[cfg(windows)]
    native::install(&crash_path());
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        // Never let reporting a panic cause another one.
        if IN_HOOK.try_with(|h| h.replace(true)).unwrap_or(true) {
            return;
        }
        let msg = info.payload_as_str().unwrap_or("(no message)");
        let location = info.location().map(|l| (l.file(), l.line(), l.column()));
        if QUIET.try_with(Cell::get).unwrap_or(false) {
            let n = QUIET_COUNT.fetch_add(1, Ordering::Relaxed);
            if n < QUIET_LINES
                && let Some(path) = LOG_PATH.get()
            {
                append_log(path, &quiet_line(msg, location, n + 1 == QUIET_LINES));
            }
        } else {
            let thread = std::thread::current();
            let name = thread_label(thread.name(), &format!("{:?}", thread.id()));
            let bt = std::backtrace::Backtrace::force_capture();
            let bt = annotate_frames(&format!("{bt:#}"), image_base(), exe_name());
            let text = report(msg, location, &name, &bt);
            if let Some(path) = LOG_PATH.get() {
                append_log(path, &text);
                append_crash(&path.with_file_name("crash.txt"), &text);
            }
            previous(info);
        }
        let _ = IN_HOOK.try_with(|h| h.set(false));
    }));
}

/// Names the threads of rayon's global pool (`rayon-N`), so a report shows
/// that a panic came from one. Does nothing when the pool already exists.
#[cfg(windows)]
pub fn name_global_pool() {
    let _ = rayon::ThreadPoolBuilder::new().thread_name(|i| format!("rayon-{i}")).build_global();
}

/// `std::thread::spawn` with a thread name, which panic reports show.
/// `None` (and a log line) when Windows refuses a new thread, where
/// `std::thread::spawn` would panic.
pub fn spawn_named<F, T>(name: &str, f: F) -> Option<JoinHandle<T>>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    match std::thread::Builder::new().name(name.to_string()).spawn(f) {
        Ok(h) => Some(h),
        Err(e) => {
            if let Some(path) = LOG_PATH.get() {
                append_log(path, &format!("could not start thread {name}: {e}"));
            }
            None
        }
    }
}

/// Runs `f`; a panic in it (already reported by the hook) gives `None`.
pub fn catch<R>(f: impl FnOnce() -> R) -> Option<R> {
    panic::catch_unwind(AssertUnwindSafe(f)).ok()
}

/// [`catch`] for code where a panic is expected (third-party parsers on
/// damaged files): logged as one short line, without a backtrace.
pub fn quietly<R>(f: impl FnOnce() -> R) -> std::thread::Result<R> {
    let was = QUIET.try_with(|q| q.replace(true)).unwrap_or(false);
    let r = panic::catch_unwind(AssertUnwindSafe(f));
    let _ = QUIET.try_with(|q| q.set(was));
    r
}

/// Wraps an nwg event handler: a panic is reported and the event dropped,
/// instead of unwinding into the window procedure, which aborts.
pub fn guard3<A, B, C>(f: impl Fn(A, B, C) + 'static) -> impl Fn(A, B, C) + 'static {
    move |a, b, c| {
        let _ = catch(|| f(a, b, c));
    }
}

/// [`guard3`] for a raw window-message handler; a panic answers `R::default()`
/// (`None` for an nwg raw handler: the message gets its default handling).
pub fn guard4<A, B, C, D, R: Default>(f: impl Fn(A, B, C, D) -> R + 'static) -> impl Fn(A, B, C, D) -> R + 'static {
    move |a, b, c, d| catch(|| f(a, b, c, d)).unwrap_or_default()
}

/// [`guard3`] typed for an nwg event handler, so the closure's arguments
/// need no annotations.
#[cfg(windows)]
pub fn guard_events(
    f: impl Fn(nwg::Event, nwg::EventData, nwg::ControlHandle) + 'static,
) -> impl Fn(nwg::Event, nwg::EventData, nwg::ControlHandle) + 'static {
    guard3(f)
}

/// [`guard4`] typed for an nwg raw event handler.
#[cfg(windows)]
pub fn guard_raw(f: impl Fn(HWND, UINT, WPARAM, LPARAM) -> Option<LRESULT> + 'static) -> impl Fn(HWND, UINT, WPARAM, LPARAM) -> Option<LRESULT> + 'static {
    guard4(f)
}

/// The name a report gives a thread: its name, or its id.
fn thread_label(name: Option<&str>, id: &str) -> String {
    match name {
        Some(n) => n.to_string(),
        None => format!("unnamed {id}"),
    }
}

/// The text of a panic report.
fn report(msg: &str, location: Option<(&str, u32, u32)>, thread: &str, backtrace: &str) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "PANIC (v{}) in thread '{thread}': {msg}", env!("CARGO_PKG_VERSION"));
    match location {
        Some((file, line, col)) => {
            let _ = writeln!(s, "  at {file}:{line}:{col}");
        }
        None => s.push_str("  at an unknown place\n"),
    }
    if let Some(base) = image_base() {
        let _ = writeln!(s, "  image base {base:#x}");
    }
    s.push_str("  backtrace:\n");
    for line in backtrace.lines() {
        let _ = writeln!(s, "  {line}");
    }
    s.push_str("END PANIC");
    s
}

/// The one line for an expected panic.
fn quiet_line(msg: &str, location: Option<(&str, u32, u32)>, last: bool) -> String {
    let mut msg: String = msg.chars().take(200).collect();
    msg = msg.replace(['\r', '\n'], " ");
    let at = location.map_or(String::new(), |(f, l, _)| format!(" at {f}:{l}"));
    let more = if last { " (later ones are not logged)" } else { "" };
    format!("contained a parser panic: {msg}{at}{more}")
}

/// Adds `exe+0xOFFSET` after each frame address of a `{:#}` backtrace that
/// lies inside the program's image (starting at `base`).
fn annotate_frames(bt: &str, base: Option<usize>, exe: &str) -> String {
    let Some(base) = base else { return bt.to_string() };
    let mut out = String::with_capacity(bt.len() + bt.len() / 4);
    for line in bt.lines() {
        out.push_str(line);
        if let Some(ip) = frame_address(line)
            && let Some(off) = ip.checked_sub(base)
            && off < 0x1000_0000
        {
            let _ = write!(out, " ({exe}+{off:#x})");
        }
        out.push('\n');
    }
    out
}

/// The address of a backtrace line such as `   3:     0x7ff6a1b2c3d4 - name`.
fn frame_address(line: &str) -> Option<usize> {
    let (num, rest) = line.trim_start().split_once(':')?;
    if num.is_empty() || !num.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hex = rest.trim_start().strip_prefix("0x")?;
    let end = hex.find(|c: char| !c.is_ascii_hexdigit()).unwrap_or(hex.len());
    usize::from_str_radix(&hex[..end], 16).ok()
}

#[cfg(windows)]
fn image_base() -> Option<usize> {
    let h = unsafe { winapi::um::libloaderapi::GetModuleHandleW(std::ptr::null()) };
    (!h.is_null()).then_some(h as usize)
}

#[cfg(not(windows))]
fn image_base() -> Option<usize> {
    None
}

/// The release asset name, so offsets name the right build (x64, x86 or ARM64).
fn exe_name() -> &'static str {
    crate::update::ASSET_NAME
}

/// Appends a timestamped entry to the log; never fails loudly. Moves a log
/// near its cap to `log.old.txt` first, so the regular log's cap doesn't
/// throw this entry away.
fn append_log(path: &Path, text: &str) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len().saturating_add(text.len() as u64) > LOG_MAX) {
        let _ = std::fs::rename(path, path.with_file_name("log.old.txt"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{} {text}", crate::index::now_secs());
    }
}

fn append_crash(path: &Path, text: &str) {
    if std::fs::metadata(path).is_ok_and(|m| m.len() > CRASH_MAX) {
        let _ = std::fs::remove_file(path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{} {text}\n", crate::index::now_secs());
    }
}

/// Crashes that are not Rust panics (a stack overflow, an access violation
/// in Windows or a driver hook): one line in `crash.txt`, written without
/// allocating, so the next start offers the report like a panic's. Before
/// 0.8.2 these left no trace at all.
#[cfg(windows)]
mod native {
    use std::path::Path;
    use std::sync::OnceLock;
    use winapi::shared::minwindef::DWORD;
    use winapi::um::errhandlingapi::{AddVectoredExceptionHandler, SetUnhandledExceptionFilter};
    use winapi::um::fileapi::{CreateFileW, OPEN_ALWAYS, WriteFile};
    use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
    use winapi::um::processthreadsapi::SetThreadStackGuarantee;
    use winapi::um::winnt::{EXCEPTION_POINTERS, FILE_APPEND_DATA, FILE_SHARE_READ, FILE_SHARE_WRITE, LONG};

    const STACK_OVERFLOW: u32 = 0xC000_00FD;
    const CONTINUE_SEARCH: LONG = 0;
    static PATH: OnceLock<Vec<u16>> = OnceLock::new();

    pub(super) fn install(crash: &Path) {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = crash.as_os_str().encode_wide().chain(Some(0)).collect();
        if PATH.set(wide).is_err() {
            return;
        }
        unsafe {
            // Room for the handler to run once the stack has overflowed.
            let mut guarantee: u32 = 64 * 1024;
            SetThreadStackGuarantee(&mut guarantee);
            AddVectoredExceptionHandler(1, Some(vectored));
            SetUnhandledExceptionFilter(Some(unhandled));
        }
    }

    unsafe extern "system" fn vectored(info: *mut EXCEPTION_POINTERS) -> LONG {
        if let Some((code, at)) = unsafe { code_and_address(info) }
            && code == STACK_OVERFLOW
        {
            write_line(code, at);
        }
        CONTINUE_SEARCH
    }

    unsafe extern "system" fn unhandled(info: *mut EXCEPTION_POINTERS) -> LONG {
        if let Some((code, at)) = unsafe { code_and_address(info) }
            && code != STACK_OVERFLOW
        {
            write_line(code, at);
        }
        CONTINUE_SEARCH
    }

    unsafe fn code_and_address(info: *mut EXCEPTION_POINTERS) -> Option<(u32, usize)> {
        let rec = unsafe { info.as_ref()?.ExceptionRecord.as_ref()? };
        Some((rec.ExceptionCode, rec.ExceptionAddress as usize))
    }

    /// Appends "<secs> PANIC (v0.8.2) native crash: exception 0xC00000FD at
    /// bahth.exe+0x1234" and "END PANIC", the shape `feedback::crash_reports`
    /// reads, so the next start offers to report it.
    fn write_line(code: u32, at: usize) {
        let Some(path) = PATH.get() else { return };
        let mut buf = [0u8; 200];
        let mut n = 0;
        let mut put = |bytes: &[u8], n: &mut usize| {
            for &b in bytes {
                if *n < buf.len() {
                    buf[*n] = b;
                    *n += 1;
                }
            }
        };
        put(&decimal(now_secs()), &mut n);
        put(b" PANIC (v", &mut n);
        put(env!("CARGO_PKG_VERSION").as_bytes(), &mut n);
        put(b") native crash: exception 0x", &mut n);
        put(&hex(code as u64, 8), &mut n);
        put(b" at ", &mut n);
        let base = super::image_base().unwrap_or(0);
        if base != 0 && at >= base && at - base < 0x1000_0000 {
            put(super::exe_name().as_bytes(), &mut n);
            put(b"+0x", &mut n);
            put(&hex((at - base) as u64, 1), &mut n);
        } else {
            put(b"0x", &mut n);
            put(&hex(at as u64, 1), &mut n);
        }
        put(b"\r\nEND PANIC\r\n\r\n", &mut n);
        unsafe {
            let h = CreateFileW(
                path.as_ptr(),
                FILE_APPEND_DATA,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null_mut(),
                OPEN_ALWAYS,
                0,
                std::ptr::null_mut(),
            );
            if h != INVALID_HANDLE_VALUE {
                let mut written: DWORD = 0;
                WriteFile(h, buf.as_ptr().cast(), n as DWORD, &mut written, std::ptr::null_mut());
                CloseHandle(h);
            }
        }
    }

    fn now_secs() -> u64 {
        let mut ft = winapi::shared::minwindef::FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        unsafe { winapi::um::sysinfoapi::GetSystemTimeAsFileTime(&mut ft) };
        let t = ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64;
        (t / 10_000_000).saturating_sub(11_644_473_600)
    }

    /// `v` in hex, at least `min` digits, as ASCII in a fixed buffer.
    pub(super) fn hex(v: u64, min: usize) -> Digits {
        let mut d = Digits::default();
        let mut tmp = [0u8; 16];
        let mut len = 0;
        let mut x = v;
        loop {
            tmp[len] = b"0123456789ABCDEF"[(x & 0xF) as usize];
            len += 1;
            x >>= 4;
            if x == 0 && len >= min {
                break;
            }
        }
        for i in 0..len {
            d.push(tmp[len - 1 - i]);
        }
        d
    }

    pub(super) fn decimal(v: u64) -> Digits {
        let mut d = Digits::default();
        let mut tmp = [0u8; 20];
        let mut len = 0;
        let mut x = v;
        loop {
            tmp[len] = b'0' + (x % 10) as u8;
            len += 1;
            x /= 10;
            if x == 0 {
                break;
            }
        }
        for i in 0..len {
            d.push(tmp[len - 1 - i]);
        }
        d
    }

    /// Up to 20 ASCII digits, on the stack.
    #[derive(Default)]
    pub(super) struct Digits {
        buf: [u8; 20],
        len: usize,
    }

    impl Digits {
        fn push(&mut self, b: u8) {
            if self.len < self.buf.len() {
                self.buf[self.len] = b;
                self.len += 1;
            }
        }
    }

    impl std::ops::Deref for Digits {
        type Target = [u8];
        fn deref(&self) -> &[u8] {
            &self.buf[..self.len]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_has_message_place_thread_and_backtrace() {
        let r = report("index out of bounds", Some(("src\\search.rs", 1185, 9)), "content-search", "   0: 0x1 - <unknown>\n   1: 0x2 - main");
        assert!(r.starts_with(&format!("PANIC (v{}) in thread 'content-search': index out of bounds", env!("CARGO_PKG_VERSION"))));
        assert!(r.contains("\n  at src\\search.rs:1185:9\n"));
        assert!(r.contains("  backtrace:\n     0: 0x1 - <unknown>\n     1: 0x2 - main\n"));
        assert!(r.ends_with("END PANIC"));
        assert!(report("x", None, "main", "").contains("at an unknown place"));
        assert_eq!(thread_label(None, "ThreadId(7)"), "unnamed ThreadId(7)");
        assert_eq!(thread_label(Some("main"), "ThreadId(1)"), "main");
    }

    #[test]
    #[cfg(target_pointer_width = "64")] // 64-bit addresses
    fn frames_get_offsets_into_the_program() {
        let bt = "   0:     0x140489519 - <unknown>\n   1:     0x7ffa00001000 - <unknown>\n  at src/x.rs:1\n";
        let out = annotate_frames(bt, Some(0x1_4000_0000), "bahth.exe");
        assert_eq!(out, "   0:     0x140489519 - <unknown> (bahth.exe+0x489519)\n   1:     0x7ffa00001000 - <unknown>\n  at src/x.rs:1\n");
        assert_eq!(annotate_frames(bt, None, "bahth.exe"), bt);
        assert_eq!(frame_address("  12: 0xabc - f"), Some(0xabc));
        assert_eq!(frame_address("note: 0x12"), None);
    }

    #[test]
    fn quiet_line_is_short() {
        let long = "x\n".repeat(500);
        let l = quiet_line(&long, Some(("lopdf/src/a.rs", 3, 1)), true);
        assert!(l.chars().count() < 300 && !l.contains('\n') && l.ends_with("at lopdf/src/a.rs:3 (later ones are not logged)"));
    }

    #[test]
    fn guards_turn_panics_into_defaults() {
        let h = guard3(|a: u32, _b: (), _c: ()| assert!(a < 5, "handler bug"));
        h(1, (), ());
        h(9, (), ()); // must not unwind out
        let raw = guard4(|_: (), msg: u32, _: (), _: ()| -> Option<isize> {
            let v: Vec<isize> = Vec::new();
            Some(v[msg as usize])
        });
        assert_eq!(raw((), 3, (), ()), None);
        assert_eq!(catch(|| 7), Some(7));
        assert!(quietly(|| panic!("damaged file")).is_err());
        assert_eq!(quietly(|| 1).ok(), Some(1));
        // The flag is restored after the panic.
        assert!(!QUIET.with(Cell::get));
    }

    /// The hook's helpers run while a panic is being handled, where a
    /// second panic aborts the program: odd input must not make them panic.
    #[test]
    fn report_helpers_take_any_input() {
        const PIECES: &[&str] = &["0", "9", ":", " ", "0x", "ff", "FFFFFFFFFFFFFFFFFFFF", "-", "\n", "\r", "ب", "😀", "\u{0}", "\u{200F}", "x"];
        let mut seed = 0xFACE_B00C_u64;
        let mut below = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        // The usual image base of a 64-bit exe; 32-bit images load lower.
        #[cfg(target_pointer_width = "64")]
        const IMAGE_BASE: usize = 0x1_4000_0000;
        #[cfg(not(target_pointer_width = "64"))]
        const IMAGE_BASE: usize = 0x40_0000;
        for _ in 0..3000 {
            let s: String = (0..below(12)).map(|_| PIECES[below(PIECES.len())]).collect();
            let _ = frame_address(&s);
            let base = [None, Some(0), Some(usize::MAX), Some(IMAGE_BASE)][below(4)];
            let _ = annotate_frames(&s, base, "bahth.exe");
            let r = report(&s, Some((&s, u32::MAX, 0)), &s, &s);
            assert!(r.ends_with("END PANIC"));
            let _ = quiet_line(&s, Some((&s, 0, 0)), below(2) == 0);
            assert!(!quiet_line(&s, None, false).contains(['\n', '\r']));
        }
        assert_eq!(frame_address("1: 0xFFFFFFFFFFFFFFFFFFFFFFFF - f"), None);
        assert_eq!(frame_address(":"), None);
        assert_eq!(frame_address(""), None);
    }

    #[test]
    fn writing_a_report_where_it_cannot_go_is_harmless() {
        let dir = std::env::temp_dir().join(format!("bahth_crash_nowhere_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("log.txt")).unwrap();
        std::fs::write(dir.join("file"), "x").unwrap();
        // The log path is a folder; its parent is a file; a name that isn't one.
        append_log(&dir.join("log.txt"), "a");
        append_crash(&dir.join("log.txt"), "a");
        append_log(&dir.join("file").join("log.txt"), "b");
        append_crash(&dir.join("file").join("crash.txt"), "b");
        append_log(Path::new(""), "c");
        append_crash(Path::new(""), "c");
        // A report bigger than the cap is still written whole.
        let log = dir.join("big").join("log.txt");
        let big = "ب".repeat(LOG_MAX as usize);
        append_log(&log, &big);
        append_log(&log, "after");
        assert!(std::fs::read_to_string(dir.join("big").join("log.old.txt")).unwrap().contains(&big));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn panics_with_odd_payloads_are_contained() {
        assert!(quietly(|| std::panic::panic_any(42u8)).is_err());
        assert!(catch(|| std::panic::panic_any(vec![1, 2, 3])).is_none());
        assert!(quietly(|| panic!("{}", "ب\u{0}\n".repeat(10_000))).is_err());
        // Nested: a contained panic inside a contained panic's handler.
        assert!(catch(|| {
            let inner = quietly(|| panic!("inner"));
            assert!(inner.is_err());
            panic!("outer")
        })
        .is_none());
        assert!(!QUIET.with(Cell::get));
    }

    #[test]
    fn hook_writes_the_report() {
        let dir = std::env::temp_dir().join(format!("bahth_crash_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let log = dir.join("log.txt");
        install(log.clone());
        let installed_here = LOG_PATH.get() == Some(&log);
        let t = spawn_named("probe-thread", || {
            let words = ["a"];
            let i = words.len() + 2;
            words[i % 4]
        })
        .unwrap();
        assert!(t.join().is_err());
        assert!(quietly(|| panic!("bad pdf")).is_err());
        if !installed_here {
            return; // another test installed the hook first
        }
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.contains("in thread 'probe-thread': index out of bounds"), "{text}");
        assert!(text.contains("crash_log.rs:"), "{text}");
        assert!(text.contains("backtrace:"), "{text}");
        assert!(text.contains("contained a parser panic: bad pdf"), "{text}");
        let crash = std::fs::read_to_string(dir.join("crash.txt")).unwrap();
        assert!(crash.contains("probe-thread") && !crash.contains("bad pdf"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
