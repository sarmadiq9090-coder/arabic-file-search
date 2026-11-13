//! Watches the indexed drives so the index follows changes on disk by itself.
//!
//! The USN journal would be cheaper but needs admin rights, so each root gets
//! a thread with a recursive ReadDirectoryChangesW watch instead. Changes are
//! collected for about a second and handed over as one batch; when Windows
//! drops events (a full buffer) the root is reported for a rescan.

use std::cell::Cell;
use std::ptr::{null, null_mut};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use winapi::shared::minwindef::{BYTE, DWORD, FALSE, LPARAM, LRESULT, TRUE, UINT, WPARAM};
use winapi::shared::windef::HWND;
use winapi::shared::winerror::{ERROR_INVALID_PARAMETER, ERROR_NOTIFY_ENUM_DIR, WAIT_TIMEOUT};
use winapi::um::dbt::{DBT_DEVICEQUERYREMOVE, DBT_DEVICEREMOVECOMPLETE, DBT_DEVTYP_HANDLE, DEV_BROADCAST_HANDLE, DEV_BROADCAST_HDR};
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::fileapi::{CreateFileW, FindClose, FindFirstFileW, GetDriveTypeW, OPEN_EXISTING};
use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
use winapi::um::ioapiset::{CancelIoEx, GetOverlappedResult};
use winapi::um::libloaderapi::GetModuleHandleW;
use winapi::um::minwinbase::{OVERLAPPED, WIN32_FIND_DATAW};
use winapi::um::synchapi::{CreateEventW, ResetEvent, SetEvent, WaitForSingleObject};
use winapi::um::winbase::{
    DRIVE_REMOTE, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, INFINITE, ReadDirectoryChangesW, WAIT_OBJECT_0,
};
use winapi::um::winnt::{
    FILE_ACTION_ADDED, FILE_ACTION_MODIFIED, FILE_ACTION_REMOVED, FILE_ACTION_RENAMED_NEW_NAME,
    FILE_ACTION_RENAMED_OLD_NAME, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_LIST_DIRECTORY,
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE,
    FILE_NOTIFY_INFORMATION, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, HANDLE, IsReparseTagNameSurrogate,
};
use winapi::um::winuser::{
    CreateWindowExW, DEVICE_NOTIFY_WINDOW_HANDLE, DefWindowProcW, DestroyWindow, DispatchMessageW, HDEVNOTIFY,
    HWND_MESSAGE, MSG, MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW, QS_ALLINPUT, RegisterClassW,
    RegisterDeviceNotificationW, TranslateMessage, UnregisterDeviceNotification, WM_DEVICECHANGE, WNDCLASSW,
};

use crate::index::{Change, skipped_path};

/// Room for events between two reads. A full buffer drops them all and
/// costs a walk of the whole drive, so local drives get a large one; over
/// the network Windows refuses more than 64 KB.
const BUF_BYTES: usize = 1024 * 1024;
const NET_BUF_BYTES: usize = 64 * 1024;
/// How long changes are collected before they are handed over.
const BATCH: Duration = Duration::from_secs(1);
/// Pause before reopening a root that failed or disappeared.
const RETRY: Duration = Duration::from_secs(5);
const FILTER: DWORD = FILE_NOTIFY_CHANGE_FILE_NAME
    | FILE_NOTIFY_CHANGE_DIR_NAME
    | FILE_NOTIFY_CHANGE_SIZE
    | FILE_NOTIFY_CHANGE_LAST_WRITE;

/// A raw handle that may cross threads (event handles are thread-safe).
struct Shared(HANDLE);
unsafe impl Send for Shared {}

impl Shared {
    // A method, so closures capture the whole wrapper and not the raw pointer.
    fn get(&self) -> HANDLE {
        self.0
    }
}

/// Starts the watch thread of one root.
type SpawnRoot = Box<dyn Fn(&str) -> JoinHandle<()> + Send>;

pub struct Watcher {
    stop: Shared,
    threads: Vec<JoinHandle<()>>,
    /// The roots watched, as given.
    roots: Vec<String>,
    /// Starts the watch of one more root (with the same sink and stop event).
    spawn: SpawnRoot,
}

impl Watcher {
    /// Starts watching every root. `sink` gets each batch of changes, on a
    /// watcher thread.
    pub fn start<F>(roots: &[String], sink: F) -> Watcher
    where
        F: Fn(Vec<Change>) + Send + Clone + 'static,
    {
        let stop = unsafe { CreateEventW(null_mut(), TRUE, FALSE, null()) };
        let own = own_dirs();
        let event = Shared(stop);
        let spawn = Box::new(move |root: &str| {
            let (root, sink, own, stop) = (root.to_string(), sink.clone(), own.clone(), Shared(event.get()));
            std::thread::spawn(move || watch_root(&root, stop.get(), &own, &sink))
        });
        let threads = roots.iter().map(|root| spawn(root)).collect();
        Watcher { stop: Shared(stop), threads, roots: roots.to_vec(), spawn }
    }

    /// Watches one more root (a drive plugged in after the start); false
    /// when it is watched already.
    pub fn add_root(&mut self, root: &str) -> bool {
        if self.stop.0.is_null() || self.roots.iter().any(|r| r.eq_ignore_ascii_case(root)) {
            return false;
        }
        self.threads.push((self.spawn)(root));
        self.roots.push(root.to_string());
        true
    }

    /// Ends all watches and waits for the threads (they wake at once).
    pub fn stop(&mut self) {
        if self.stop.0.is_null() {
            return;
        }
        unsafe { SetEvent(self.stop.0) };
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
        unsafe { CloseHandle(self.stop.0) };
        self.stop.0 = null_mut();
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop();
    }
}

/// `%LOCALAPPDATA%\bahthsari\` in lower case: the index and log are written
/// there, and watching them would feed every save back as a change. In the
/// Microsoft Store version Windows keeps that folder under the package
/// (`store::real_data_dir`), the path the watcher sees, so that one too.
fn own_dirs() -> Vec<String> {
    let dir = crate::index::cache_path().parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    std::iter::once(dir.as_str())
        .chain(crate::store::real_data_dir())
        .map(|d| format!("{}\\", d.to_lowercase().trim_end_matches('\\')))
        .collect()
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `C:\…` as `\\?\C:\…` and `\\server\share\…` as `\\?\UNC\server\share\…`,
/// so paths longer than 260 characters work too. (A long path on a watched
/// share used to look gone, and its entry was dropped.)
fn long_path(path: &str) -> Vec<u16> {
    wide(&long_path_str(path))
}

fn long_path_str(path: &str) -> String {
    let b = path.as_bytes();
    if path.starts_with(r"\\?\") || path.starts_with(r"\\.\") {
        path.to_string()
    } else if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/') {
        // The prefix turns off the conversion of `/` to `\`.
        format!("\\\\?\\{}", path.replace('/', "\\"))
    } else if let Some(unc) = path.strip_prefix(r"\\").filter(|u| !u.is_empty() && !u.starts_with('\\')) {
        format!("\\\\?\\UNC\\{}", unc.replace('/', "\\"))
    } else {
        path.to_string()
    }
}

/// Size, mtime and folder flag of `path` as the walker would record them, or
/// `None` when it is gone, is a link or junction (never indexed), or exists
/// only under another letter case (a case-only rename of the old name).
pub fn stat(path: &str) -> Option<(u64, i64, bool)> {
    let name = path.rsplit(['\\', '/']).next().filter(|n| !n.is_empty())?;
    let mut fd: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
    let h = unsafe { FindFirstFileW(long_path(path).as_ptr(), &mut fd) };
    if h == INVALID_HANDLE_VALUE {
        return None;
    }
    unsafe { FindClose(h) };
    let len = fd.cFileName.iter().position(|&c| c == 0).unwrap_or(fd.cFileName.len());
    if String::from_utf16(&fd.cFileName[..len]).ok()? != name {
        return None;
    }
    let attrs = fd.dwFileAttributes;
    if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 && IsReparseTagNameSurrogate(fd.dwReserved0) {
        return None;
    }
    let is_dir = attrs & FILE_ATTRIBUTE_DIRECTORY != 0;
    let size = if is_dir { 0 } else { (fd.nFileSizeHigh as u64) << 32 | fd.nFileSizeLow as u64 };
    // FILETIME counts 100 ns steps since 1601; the index stores Unix seconds.
    let ft = ((fd.ftLastWriteTime.dwHighDateTime as u64) << 32 | fd.ftLastWriteTime.dwLowDateTime as u64) as i64;
    // Saturating: a damaged or odd time stamp (top bit set) must not overflow.
    let mtime = (ft.saturating_sub(116_444_736_000_000_000) / 10_000_000).max(0);
    Some((size, mtime, is_dir))
}

/// A share (`\\server\share\…`) or a folder on a mapped network drive.
fn is_network(root: &str) -> bool {
    if root.starts_with(r"\\") {
        return true;
    }
    let b = root.as_bytes();
    let drive = if b.len() >= 2 && b[1] == b':' { format!("{}\\", &root[..2]) } else { root.to_string() };
    unsafe { GetDriveTypeW(wide(&drive).as_ptr()) == DRIVE_REMOTE }
}

enum Outcome {
    Stop,
    /// The watch broke or the drive is going away: reopen later and rescan.
    Lost,
}

fn watch_root(root: &str, stop: HANDLE, own: &[String], sink: &dyn Fn(Vec<Change>)) {
    let window = message_window();
    let mut needs_rescan = false;
    loop {
        let dir = unsafe {
            CreateFileW(
                wide(root).as_ptr(),
                FILE_LIST_DIRECTORY,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                null_mut(),
            )
        };
        let outcome = if dir == INVALID_HANDLE_VALUE {
            // Not there (an unplugged disk, an empty card reader); its
            // contents are unknown when it comes back.
            Outcome::Lost
        } else {
            watch_dir(root, dir, stop, window, own, sink, &mut needs_rescan)
        };
        if let Outcome::Stop = outcome {
            break;
        }
        needs_rescan = true;
        if unsafe { WaitForSingleObject(stop, RETRY.as_millis() as DWORD) } == WAIT_OBJECT_0 {
            break;
        }
    }
    if !window.is_null() {
        unsafe { DestroyWindow(window) };
    }
}

thread_local! {
    /// The directory handle this thread watches, its device notification, and
    /// whether the window procedure released them for a drive removal.
    static DEVICE: Cell<(HANDLE, HDEVNOTIFY, bool)> = const { Cell::new((null_mut(), null_mut(), false)) };
}

/// Watches one open root until told to stop or the watch breaks. Closes `dir`.
fn watch_dir(
    root: &str,
    dir: HANDLE,
    stop: HANDLE,
    window: HWND,
    own: &[String],
    sink: &dyn Fn(Vec<Change>),
    needs_rescan: &mut bool,
) -> Outcome {
    // Asking to be told before the drive is ejected: an open handle would
    // otherwise make "safely remove" fail.
    let notify = if window.is_null() {
        null_mut()
    } else {
        let mut filter: DEV_BROADCAST_HANDLE = unsafe { std::mem::zeroed() };
        filter.dbch_size = std::mem::size_of::<DEV_BROADCAST_HANDLE>() as DWORD;
        filter.dbch_devicetype = DBT_DEVTYP_HANDLE;
        filter.dbch_handle = dir;
        unsafe { RegisterDeviceNotificationW(window as _, &mut filter as *mut _ as _, DEVICE_NOTIFY_WINDOW_HANDLE) }
    };
    DEVICE.with(|d| d.set((dir, notify, false)));

    let event = unsafe { CreateEventW(null_mut(), TRUE, FALSE, null()) };
    let mut buf_bytes = if is_network(root) { NET_BUF_BYTES } else { BUF_BYTES };
    // DWORD-aligned, as ReadDirectoryChangesW requires.
    let mut buf = vec![0u64; buf_bytes / 8];
    let mut ov: OVERLAPPED;
    let mut pending: Vec<Change> = Vec::new();
    let mut old_name: Option<String> = None;
    let mut deadline: Option<Instant> = None;
    let mut io_pending = false;

    let outcome = 'watch: loop {
        ov = unsafe { std::mem::zeroed() };
        ov.hEvent = event;
        unsafe { ResetEvent(event) };
        let ok = unsafe {
            ReadDirectoryChangesW(dir, buf.as_mut_ptr() as _, buf_bytes as DWORD, TRUE, FILTER, null_mut(), &mut ov, None)
        };
        if ok == 0 {
            // A network drive that did not look like one: try the small size.
            if buf_bytes > NET_BUF_BYTES && unsafe { GetLastError() } == ERROR_INVALID_PARAMETER {
                buf_bytes = NET_BUF_BYTES;
                continue;
            }
            break Outcome::Lost;
        }
        io_pending = true;
        if std::mem::take(needs_rescan) {
            // Watching again after a break: whatever happened meanwhile is unknown.
            pending.push(Change::Rescan(root.to_string()));
            deadline.get_or_insert_with(Instant::now);
        }

        // Wait for the read, handing over the batch when it is due.
        loop {
            let timeout = deadline.map_or(INFINITE, |d| d.saturating_duration_since(Instant::now()).as_millis() as DWORD);
            let handles = [event, stop];
            let r = unsafe { MsgWaitForMultipleObjects(2, handles.as_ptr(), FALSE, timeout, QS_ALLINPUT) };
            if r == WAIT_OBJECT_0 {
                break;
            } else if r == WAIT_OBJECT_0 + 1 {
                break 'watch Outcome::Stop;
            } else if r == WAIT_OBJECT_0 + 2 {
                pump_messages();
                if DEVICE.with(|d| d.get().2) {
                    break 'watch Outcome::Lost;
                }
            } else if r == WAIT_TIMEOUT {
                deadline = None;
                sink(std::mem::take(&mut pending));
            } else {
                break 'watch Outcome::Lost;
            }
        }

        io_pending = false;
        let mut bytes: DWORD = 0;
        if unsafe { GetOverlappedResult(dir, &mut ov, &mut bytes, FALSE) } == 0 {
            if unsafe { GetLastError() } != ERROR_NOTIFY_ENUM_DIR {
                break Outcome::Lost;
            }
            bytes = 0;
        }
        if bytes == 0 {
            // The buffer overflowed and events were dropped. What came before
            // is still right and is kept: the walk may be postponed a while.
            // A rename's old name waiting for its new one lost it.
            if let Some(old) = old_name.take() {
                push(&mut pending, own, Change::Removed(old));
            }
            pending.push(Change::Rescan(root.to_string()));
        } else {
            let data = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const BYTE, bytes as usize) };
            parse(data, root, own, &mut old_name, &mut pending);
        }
        if !pending.is_empty() {
            let due = *deadline.get_or_insert_with(|| Instant::now() + BATCH);
            // A file written non-stop would otherwise keep the wait from
            // ever timing out, and the batch would never be handed over.
            if Instant::now() >= due {
                deadline = None;
                sink(std::mem::take(&mut pending));
            }
        }
    };

    let (dir, notify, released) = DEVICE.with(|d| d.replace((null_mut(), null_mut(), false)));
    if released {
        // The handle is already closed; the read completes as cancelled.
        if io_pending && unsafe { WaitForSingleObject(event, 2000) } != WAIT_OBJECT_0 {
            // Never free a buffer the system may still write into.
            std::mem::forget(buf);
        }
    } else {
        if io_pending {
            let mut bytes = 0;
            unsafe {
                CancelIoEx(dir, &mut ov);
                GetOverlappedResult(dir, &mut ov, &mut bytes, TRUE);
            }
        }
        unsafe {
            if !notify.is_null() {
                UnregisterDeviceNotification(notify);
            }
            CloseHandle(dir);
        }
    }
    unsafe { CloseHandle(event) };
    if let Outcome::Lost = outcome
        && !pending.is_empty()
    {
        sink(pending);
    }
    outcome
}

/// Turns the FILE_NOTIFY_INFORMATION records in `data` into changes with full
/// paths. A rename's old name waits in `old_name` for its new name, which
/// may come in the next buffer.
fn parse(data: &[u8], root: &str, own: &[String], old_name: &mut Option<String>, out: &mut Vec<Change>) {
    // FILE_NOTIFY_INFORMATION: three DWORDs (next entry offset, action, name
    // length in bytes), then the UTF-16 name. Read field by field with bounds
    // checks, never through a struct reference: an offset that is out of
    // range or not DWORD-aligned is then just the end of the list.
    const HEADER: usize = 12;
    const _: () = assert!(std::mem::offset_of!(FILE_NOTIFY_INFORMATION, FileName) == HEADER);
    let dword = |at: usize| {
        let b = data.get(at..at.checked_add(4)?)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let mut off = 0usize;
    while let (Some(next), Some(action), Some(name_bytes)) = (dword(off), dword(off.saturating_add(4)), dword(off.saturating_add(8))) {
        // In bounds: the third DWORD ended there.
        let start = off + HEADER;
        let end = start.saturating_add(name_bytes as usize).min(data.len());
        let name: Vec<u16> = data[start..end].as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).collect();
        // Names that are not valid UTF-16 are not in the index either; an
        // empty one would stand for the root itself.
        let path = String::from_utf16(&name).ok().filter(|rel| !rel.is_empty()).map(|rel| {
            let sep = if root.ends_with('\\') { "" } else { "\\" };
            format!("{root}{sep}{rel}")
        });
        if action != FILE_ACTION_RENAMED_NEW_NAME
            && let Some(old) = old_name.take()
        {
            push(out, own, Change::Removed(old));
        }
        match (action, path) {
            (FILE_ACTION_ADDED, Some(p)) => push(out, own, Change::Added(p)),
            (FILE_ACTION_REMOVED, Some(p)) => push(out, own, Change::Removed(p)),
            (FILE_ACTION_MODIFIED, Some(p)) => push(out, own, Change::Modified(p)),
            // An old name the index can't hold: the new one is an addition.
            (FILE_ACTION_RENAMED_OLD_NAME, p) => *old_name = p,
            (FILE_ACTION_RENAMED_NEW_NAME, Some(p)) => match old_name.take() {
                Some(old) => push(out, own, Change::Renamed(old, p)),
                None => push(out, own, Change::Added(p)),
            },
            // Renamed to a name the index can't hold: the old one is gone.
            (FILE_ACTION_RENAMED_NEW_NAME, None) => {
                if let Some(old) = old_name.take() {
                    push(out, own, Change::Removed(old));
                }
            }
            _ => {}
        }
        if next == 0 || next % 4 != 0 {
            break;
        }
        let Some(o) = off.checked_add(next as usize) else { break };
        off = o;
    }
}

/// Adds `c` unless it is about a path the index leaves out; a rename
/// across that border becomes an add or a remove.
fn push(out: &mut Vec<Change>, own: &[String], c: Change) {
    let ignored = |p: &str| skipped_path(p) || {
        let p = p.to_lowercase();
        own.iter().any(|dir| p.starts_with(dir.as_str()))
    };
    let c = match c {
        Change::Renamed(old, new) => match (ignored(&old), ignored(&new)) {
            (true, true) => return,
            (true, false) => Change::Added(new),
            (false, true) => Change::Removed(old),
            (false, false) => Change::Renamed(old, new),
        },
        Change::Added(ref p) | Change::Removed(ref p) | Change::Modified(ref p) if ignored(p) => return,
        c => c,
    };
    // Each write of a file reports it a few times in a row.
    if out.last() != Some(&c) {
        out.push(c);
    }
}

fn pump_messages() {
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    while unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) } != 0 {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// A hidden message-only window that receives the drive-removal notices.
fn message_window() -> HWND {
    let class = wide("BahthSariWatch");
    unsafe {
        let hinst = GetModuleHandleW(null());
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(window_proc);
        wc.hInstance = hinst;
        wc.lpszClassName = class.as_ptr();
        // Fails harmlessly when another watcher thread registered it already.
        RegisterClassW(&wc);
        CreateWindowExW(0, class.as_ptr(), null(), 0, 0, 0, 0, 0, HWND_MESSAGE, null_mut(), hinst, null_mut())
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: UINT, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_DEVICECHANGE && (w == DBT_DEVICEQUERYREMOVE || w == DBT_DEVICEREMOVECOMPLETE) && l != 0 {
        let hdr = unsafe { &*(l as *const DEV_BROADCAST_HDR) };
        if hdr.dbch_devicetype == DBT_DEVTYP_HANDLE {
            // Let go of the drive before answering, or the removal fails.
            let (dir, notify, released) = DEVICE.with(|d| d.get());
            if !released && !dir.is_null() {
                unsafe {
                    if !notify.is_null() {
                        UnregisterDeviceNotification(notify);
                    }
                    CancelIoEx(dir, null_mut());
                    CloseHandle(dir);
                }
                DEVICE.with(|d| d.set((dir, null_mut(), true)));
            }
            return TRUE as LRESULT;
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One FILE_NOTIFY_INFORMATION record, padded to a DWORD boundary.
    fn record(action: DWORD, name: &[u16], last: bool) -> Vec<u8> {
        let len = (12 + name.len() * 2).div_ceil(4) * 4;
        let mut r = Vec::new();
        r.extend_from_slice(&(if last { 0 } else { len as u32 }).to_le_bytes());
        r.extend_from_slice(&action.to_le_bytes());
        r.extend_from_slice(&((name.len() * 2) as u32).to_le_bytes());
        for c in name {
            r.extend_from_slice(&c.to_le_bytes());
        }
        r.resize(len, 0);
        r
    }

    fn utf16(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn run(data: &[u8], old: &mut Option<String>) -> Vec<Change> {
        let mut out = Vec::new();
        parse(data, r"C:\", &[r"c:\own\".to_string()], old, &mut out);
        out
    }

    #[test]
    fn records_become_changes() {
        let mut d = record(FILE_ACTION_ADDED, &utf16(r"مجلد\ملف.txt"), false);
        d.extend(record(FILE_ACTION_RENAMED_OLD_NAME, &utf16("a.txt"), false));
        d.extend(record(FILE_ACTION_RENAMED_NEW_NAME, &utf16("A.TXT"), false));
        d.extend(record(FILE_ACTION_MODIFIED, &utf16(r"OWN\log.txt"), false));
        d.extend(record(FILE_ACTION_REMOVED, &utf16("😀"), true));
        let mut old = None;
        assert_eq!(
            run(&d, &mut old),
            vec![
                Change::Added(r"C:\مجلد\ملف.txt".into()),
                Change::Renamed(r"C:\a.txt".into(), r"C:\A.TXT".into()),
                Change::Removed(r"C:\😀".into()),
            ]
        );
        assert_eq!(old, None);
        // A rename split over two buffers.
        assert!(run(&record(FILE_ACTION_RENAMED_OLD_NAME, &utf16("x"), true), &mut old).is_empty());
        assert_eq!(old.as_deref(), Some(r"C:\x"));
        let got = run(&record(FILE_ACTION_RENAMED_NEW_NAME, &utf16("y"), true), &mut old);
        assert_eq!(got, vec![Change::Renamed(r"C:\x".into(), r"C:\y".into())]);
    }

    #[test]
    fn every_own_folder_is_left_out() {
        // The Store version: the usual folder and the package's real one.
        let own = [r"c:\users\u\appdata\local\bahthsari\".to_string(), r"c:\users\u\appdata\local\packages\f.i_p\localcache\local\bahthsari\".to_string()];
        let mut d = record(FILE_ACTION_MODIFIED, &utf16(r"Users\u\AppData\Local\Packages\F.I_p\LocalCache\Local\BahthSari\log.txt"), false);
        d.extend(record(FILE_ACTION_MODIFIED, &utf16(r"Users\u\AppData\Local\BahthSari\index.bin"), false));
        d.extend(record(FILE_ACTION_ADDED, &utf16(r"Users\u\AppData\Local\Packages\F.I_p\LocalCache\Local\BahthSariX.txt"), true));
        let mut out = Vec::new();
        parse(&d, r"C:\", &own, &mut None, &mut out);
        assert_eq!(out, vec![Change::Added(r"C:\Users\u\AppData\Local\Packages\F.I_p\LocalCache\Local\BahthSariX.txt".into())]);
    }

    #[test]
    fn odd_names_and_damaged_buffers() {
        let mut old = None;
        // Renamed to a name that is not valid UTF-16 (a lone surrogate): gone.
        let mut d = record(FILE_ACTION_RENAMED_OLD_NAME, &utf16("a"), false);
        d.extend(record(FILE_ACTION_RENAMED_NEW_NAME, &[0xD800, 0x61], true));
        assert_eq!(run(&d, &mut old), vec![Change::Removed(r"C:\a".into())]);
        // An empty name is not the root.
        assert!(run(&record(FILE_ACTION_REMOVED, &[], true), &mut old).is_empty());
        // A name length past the end is cut at the end.
        let mut d = record(FILE_ACTION_ADDED, &utf16("abcd"), true);
        d[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(run(&d, &mut old), vec![Change::Added(r"C:\abcd".into())]);
        // An offset that is not DWORD-aligned, or points past the end, ends the list.
        let mut d = record(FILE_ACTION_ADDED, &utf16("a"), false);
        d[..4].copy_from_slice(&13u32.to_le_bytes());
        d.extend(record(FILE_ACTION_ADDED, &utf16("b"), true));
        assert_eq!(run(&d, &mut old), vec![Change::Added(r"C:\a".into())]);
        let mut d = record(FILE_ACTION_ADDED, &utf16("a"), false);
        d[..4].copy_from_slice(&0xFFFF_FFFCu32.to_le_bytes());
        assert_eq!(run(&d, &mut old), vec![Change::Added(r"C:\a".into())]);
        // Every cut of a good buffer, and random bytes: never a panic, and
        // every path lies under the root.
        let mut good = record(FILE_ACTION_ADDED, &utf16("تقرير.docx"), false);
        good.extend(record(FILE_ACTION_RENAMED_OLD_NAME, &utf16("b"), false));
        good.extend(record(FILE_ACTION_RENAMED_NEW_NAME, &utf16("c"), true));
        for n in 0..=good.len() {
            for c in run(&good[..n], &mut None) {
                assert!(c.touches(r"C:\"), "{c:?}");
            }
        }
        let mut seed = 0x5151_7777_u64;
        for _ in 0..2000 {
            let mut b = Vec::new();
            for _ in 0..(seed % 120) {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                b.push(seed as u8);
            }
            for c in run(&b, &mut None) {
                assert!(c.touches(r"C:\"), "{c:?}");
            }
        }
    }

    #[test]
    fn long_paths_get_the_right_prefix() {
        assert_eq!(long_path_str(r"C:\a\b"), r"\\?\C:\a\b");
        assert_eq!(long_path_str("C:/a/b"), r"\\?\C:\a\b");
        assert_eq!(long_path_str(r"\\srv\share\مجلد"), r"\\?\UNC\srv\share\مجلد");
        assert_eq!(long_path_str(r"\\?\C:\x"), r"\\?\C:\x");
        assert_eq!(long_path_str(r"\\.\pipe\x"), r"\\.\pipe\x");
        for odd in ["", r"\\", r"\\\x", "C:", "relative"] {
            assert_eq!(long_path_str(odd), odd);
        }
    }
}
