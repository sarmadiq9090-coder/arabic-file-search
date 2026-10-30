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

/// Installs the panic hook, writing to `log` (and `crash.txt` beside it).
/// The previous hook still runs after it. Only the first call counts.
pub fn install(log: PathBuf) {
    if LOG_PATH.set(log).is_err() {
        return;
    }
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

fn exe_name() -> &'static str {
    "bahth.exe"
}

/// Appends a timestamped entry to the log; never fails loudly. Moves a log
/// near its cap to `log.old.txt` first, so the regular log's cap doesn't
/// throw this entry away.
fn append_log(path: &Path, text: &str) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() + text.len() as u64 > LOG_MAX) {
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
