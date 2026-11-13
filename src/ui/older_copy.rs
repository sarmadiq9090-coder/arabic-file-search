//! A newer copy takes the place of an older one: the one that runs when this
//! copy starts (the single-instance lock is held), or, with none running,
//! the installed one a download is newer than. The decisions and the file
//! swap are in `crate::handover` (free of Win32, tested on Linux); this file
//! asks the running copy who it is, closes it and starts the program from
//! its place.
//!
//! The running copy is found by its main window (`startup::other_copy_window`).
//! Its version comes from its answer to `handover::IDENTIFY_MESSAGE` (1.0 and
//! later), else from its program file's version resource (0.10.0 has one);
//! its file from `QueryFullProcessImageNameW`; whether it is the Store copy
//! from its answer, else from `GetPackageFullName`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use winapi::shared::minwindef::{DWORD, FALSE, LPARAM, UINT, WPARAM};
use winapi::shared::windef::HWND;
use winapi::um::handleapi::CloseHandle;
use winapi::um::winnt::HANDLE;

use super::autostart;
use super::shell::wide;
use super::startup::log;
use crate::handover::{self, CopyInfo, Folder, SecondStart, WayOut};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn registered(cell: &'static OnceLock<UINT>, name: &str) -> UINT {
    *cell.get_or_init(|| {
        let name = wide(name);
        unsafe { winapi::um::winuser::RegisterWindowMessageW(name.as_ptr()) }
    })
}

/// `handover::IDENTIFY_MESSAGE`, registered (0 if Windows refused).
pub(super) fn identify_message() -> UINT {
    static ID: OnceLock<UINT> = OnceLock::new();
    registered(&ID, handover::IDENTIFY_MESSAGE)
}

/// `handover::LEAVE_MESSAGE`, registered (0 if Windows refused).
pub(super) fn leave_message() -> UINT {
    static ID: OnceLock<UINT> = OnceLock::new();
    registered(&ID, handover::LEAVE_MESSAGE)
}

/// This copy's answer to `identify_message`.
pub(super) fn identity() -> isize {
    handover::pack_identity(VERSION, crate::store::is_packaged())
}

/// The version of the copy this process replaced, when it did so itself
/// (the old copy's file was kept): for the notice and the Run entry.
static REPLACED: OnceLock<String> = OnceLock::new();

/// The version this copy replaced: by itself, or as the copy started in the
/// old one's place (`--replaced`).
pub(super) fn replaced_version() -> Option<String> {
    REPLACED.get().cloned().or_else(|| handover::arg_value(std::env::args(), handover::REPLACED_ARG))
}

/// The notice after a switch, once the main window shows.
pub(super) fn notice() -> Option<String> {
    replaced_version().map(|old| handover::replaced_notice(&old, VERSION))
}

struct Process(HANDLE);

impl Drop for Process {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// The exe a process runs, as Windows names it.
fn image_path(h: HANDLE) -> Option<String> {
    let mut buf = vec![0u16; 32_768];
    let mut len = buf.len() as DWORD;
    let ok = unsafe { winapi::um::winbase::QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) } != 0;
    (ok && len > 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
}

/// `VS_FIXEDFILEINFO` (verrsrc.h), which winapi does not define.
#[repr(C)]
struct FixedFileInfo {
    signature: DWORD,
    struc_version: DWORD,
    file_version_ms: DWORD,
    file_version_ls: DWORD,
    product_version_ms: DWORD,
    product_version_ls: DWORD,
    file_flags_mask: DWORD,
    file_flags: DWORD,
    file_os: DWORD,
    file_type: DWORD,
    file_subtype: DWORD,
    file_date_ms: DWORD,
    file_date_ls: DWORD,
}

/// The file version of the exe at `path` (`major.minor.patch`), from its
/// version resource; `None` without one.
pub(super) fn file_version(path: &str) -> Option<String> {
    use winapi::um::winver::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
    let p = wide(path);
    let mut handle: DWORD = 0;
    let size = unsafe { GetFileVersionInfoSizeW(p.as_ptr(), &mut handle) };
    if size == 0 {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    if unsafe { GetFileVersionInfoW(p.as_ptr(), 0, size, data.as_mut_ptr().cast()) } == 0 {
        return None;
    }
    let root = wide("\\");
    let mut info: *mut winapi::ctypes::c_void = std::ptr::null_mut();
    let mut len: UINT = 0;
    let ok = unsafe { VerQueryValueW(data.as_ptr().cast(), root.as_ptr(), &mut info, &mut len) } != 0;
    if !ok || info.is_null() || (len as usize) < std::mem::size_of::<FixedFileInfo>() {
        return None;
    }
    // SAFETY: VerQueryValueW points into `data`, which outlives this read,
    // at a VS_FIXEDFILEINFO of `len` bytes.
    let f = unsafe { std::ptr::read_unaligned(info as *const FixedFileInfo) };
    (f.signature == 0xFEEF_04BD).then(|| {
        format!("{}.{}.{}", f.file_version_ms >> 16, f.file_version_ms & 0xFFFF, f.file_version_ls >> 16)
    })
}

/// The names in the version resource of the exe at `path` (ProductName,
/// InternalName, OriginalFilename, for each language it lists).
fn resource_names(path: &str) -> Vec<String> {
    use winapi::um::winver::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
    // A value's bytes (`unit`: bytes per counted unit), kept inside `data`.
    fn query<'a>(data: &'a [u8], sub: &str, unit: usize) -> Option<&'a [u8]> {
        let w = wide(sub);
        let mut ptr: *mut winapi::ctypes::c_void = std::ptr::null_mut();
        let mut len: UINT = 0;
        let ok = unsafe { VerQueryValueW(data.as_ptr().cast(), w.as_ptr(), &mut ptr, &mut len) } != 0;
        let at = (ptr as usize).checked_sub(data.as_ptr() as usize)?;
        if !ok || len == 0 || at >= data.len() {
            return None;
        }
        let n = (len as usize).saturating_mul(unit).min(data.len() - at);
        Some(&data[at..at + n])
    }
    let p = wide(path);
    let mut handle: DWORD = 0;
    let size = unsafe { GetFileVersionInfoSizeW(p.as_ptr(), &mut handle) };
    if size == 0 {
        return Vec::new();
    }
    let mut data = vec![0u8; size as usize];
    if unsafe { GetFileVersionInfoW(p.as_ptr(), 0, size, data.as_mut_ptr().cast()) } == 0 {
        return Vec::new();
    }
    let mut langs = Vec::new();
    if let Some(bytes) = query(&data, "\\VarFileInfo\\Translation", 1) {
        // Pairs of u16: language, code page.
        for pair in bytes.as_chunks::<4>().0.iter().take(8) {
            langs.push(format!("{:04x}{:04x}", u16::from_le_bytes([pair[0], pair[1]]), u16::from_le_bytes([pair[2], pair[3]])));
        }
    }
    let mut names = Vec::new();
    for lang in langs {
        for key in ["ProductName", "InternalName", "OriginalFilename"] {
            if let Some(bytes) = query(&data, &format!("\\StringFileInfo\\{lang}\\{key}"), 2) {
                // UTF-16 units, read as bytes (`data` has no u16 alignment).
                let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|&c| c != 0).collect();
                names.push(String::from_utf16_lossy(&units));
            }
        }
    }
    names
}

/// Before the copy at `exe` (`answered`: it answered the identify
/// message) is ended or its file replaced: `None` when it may be, else why
/// it is only brought to the front (`handover::replace_refused`).
fn replace_refused(answered: bool, exe: Option<&str>) -> Option<&'static str> {
    let names = match exe {
        Some(e) if !answered => resource_names(e),
        _ => Vec::new(),
    };
    let ours = handover::is_this_program(answered, exe, &names);
    let theirs = exe.and_then(|e| crate::update::file_machine(Path::new(e)));
    let mine = own_exe().and_then(|p| crate::update::file_machine(&p)).or_else(crate::update::this_machine);
    handover::replace_refused(ours, theirs, mine)
}

/// Whether the process `h` runs from an MSIX package (the Store copy):
/// `GetPackageFullName`, looked up at run time (not before Windows 8).
fn process_packaged(h: HANDLE) -> bool {
    use winapi::um::libloaderapi::{GetModuleHandleW, GetProcAddress};
    type GetPackageFullName = unsafe extern "system" fn(HANDLE, *mut u32, *mut u16) -> i32;
    let kernel32 = wide("kernel32.dll");
    let module = unsafe { GetModuleHandleW(kernel32.as_ptr()) };
    if module.is_null() {
        return false;
    }
    let f = unsafe { GetProcAddress(module, c"GetPackageFullName".as_ptr()) };
    if f.is_null() {
        return false;
    }
    // SAFETY: the documented signature of GetPackageFullName.
    let f = unsafe { std::mem::transmute::<winapi::shared::minwindef::FARPROC, GetPackageFullName>(f) };
    let mut len = 0u32;
    crate::store::packaged_from_status(unsafe { f(h, &mut len, std::ptr::null_mut()) })
}

/// This program's own file.
fn own_exe() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

fn this_copy() -> CopyInfo {
    CopyInfo {
        version: Some(VERSION.to_string()),
        exe: own_exe().map(|p| p.to_string_lossy().into_owned()),
        packaged: crate::store::is_packaged(),
    }
}

/// In Downloads or a temp folder.
fn transient(exe: &str) -> bool {
    matches!(crate::relocate::classify(exe, &autostart::folders()), crate::relocate::Place::Downloads | crate::relocate::Place::Temp)
}

/// Installed by winget, Scoop or Chocolatey (judged by its place).
fn managed(exe: &str) -> bool {
    crate::update::package_manager_for(exe, |k| std::env::var(k).ok()).is_some()
}

/// The kind of folder `exe` is in.
fn folder_of(exe: &str) -> Folder {
    let protected = handover::in_program_files(exe, |k| std::env::var(k).ok());
    let writable = Path::new(exe).parent().is_some_and(crate::update::folder_writable);
    handover::folder_kind(managed(exe), protected, transient(exe), writable)
}

/// The running copy behind `hwnd`.
struct Running {
    hwnd: HWND,
    pid: u32,
    process: Process,
    info: CopyInfo,
    /// It answered `identify_message`: it knows `leave_message`.
    answered: bool,
}

fn running_copy(hwnd: HWND) -> Result<Running, String> {
    use winapi::um::processthreadsapi::OpenProcess;
    use winapi::um::winnt::{PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, SYNCHRONIZE};
    use winapi::um::winuser::{SMTO_ABORTIFHUNG, SMTO_BLOCK, SendMessageTimeoutW};
    let pid = super::startup::window_pid(hwnd).ok_or("its window has no other process")?;
    // The rights to wait for it and end it: without them (a copy run as
    // administrator) it is not touched at all.
    let h = unsafe { OpenProcess(SYNCHRONIZE | PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
    if h.is_null() {
        return Err(format!("pid {pid} cannot be opened (error {})", unsafe { winapi::um::errhandlingapi::GetLastError() }));
    }
    let process = Process(h);
    let mut answer: usize = 0;
    let msg = identify_message();
    let sent = msg != 0
        && unsafe { SendMessageTimeoutW(hwnd, msg, 0, 0, SMTO_ABORTIFHUNG | SMTO_BLOCK, 2000, &mut answer) } != 0;
    let told = if sent { handover::unpack_identity(answer as isize) } else { None };
    let exe = image_path(h);
    let info = match &told {
        Some((version, packaged)) => CopyInfo { version: Some(version.clone()), exe: exe.clone(), packaged: *packaged },
        None => CopyInfo {
            version: exe.as_deref().and_then(file_version),
            exe: exe.clone(),
            packaged: process_packaged(h),
        },
    };
    Ok(Running { hwnd, pid, process, info, answered: told.is_some() })
}

/// Asks the running copy to leave and ends it if it does not go in time
/// (`handover::WayOut`). False when it is still there.
fn close_running(r: &Running) -> bool {
    use winapi::um::processthreadsapi::TerminateProcess;
    use winapi::um::synchapi::WaitForSingleObject;
    use winapi::um::winbase::WAIT_OBJECT_0;
    use winapi::um::winuser::{PostMessageW, WM_CLOSE};
    let close_exits = {
        let s = crate::settings::Settings::load();
        !s.tray_icon && s.floating_mode == crate::settings::FloatingMode::Off
    };
    let way = WayOut::for_copy(r.answered, close_exits);
    let post = |msg: UINT| unsafe { PostMessageW(r.hwnd, msg, 0, 0) } != 0;
    let posted = match way {
        WayOut::Ask => post(leave_message()),
        WayOut::Close => post(WM_CLOSE),
        WayOut::End => true,
    };
    log(&format!("replace: asking the running copy (pid {}) to leave: {way:?}{}", r.pid, if posted { "" } else { " (the message was not posted)" }));
    let h = r.process.0;
    let (outcome, line) = handover::end_previous_within(
        r.pid,
        way.wait(),
        |budget| Some(unsafe { WaitForSingleObject(h, budget.as_millis() as u32) } == WAIT_OBJECT_0),
        || unsafe { TerminateProcess(h, 1) } != 0,
    );
    log(&line.replace("handover: previous copy", "replace: the older copy"));
    if outcome == handover::Previous::Terminated {
        // Ended from outside: what it was reading did not crash it.
        crate::content_cache::clear_markers_in(&crate::index::cache_path().with_file_name("content"));
    }
    outcome != handover::Previous::Stuck
}

/// Starts the program at `exe` as the one that replaced `old_version`,
/// detached from this process, waiting for this one to end (`--updated`,
/// `--prev-pid`), and naming the download to delete.
fn start_from(exe: &Path, old_version: &str, download: Option<&Path>) -> std::io::Result<std::process::Child> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--updated")
        .arg(super::startup::this_copy_arg())
        .arg(format!("{}={old_version}", handover::REPLACED_ARG))
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    if let Some(d) = download {
        cmd.arg(format!("{}={}", handover::REMOVE_DOWNLOAD_ARG, d.display()));
    }
    if autostart::launched_at_sign_in() {
        cmd.arg(autostart::STARTUP_ARG);
    }
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    cmd.spawn()
}

/// This program was put in place of `target` (the old file kept as
/// `.old`): starts it from there and watches it (`handover::took_over`).
/// True when it runs (this process must leave now). Otherwise the old file
/// is put back and false is returned.
fn run_from(target: &Path, old_version: &str) -> bool {
    let download = own_exe().filter(|p| transient(&p.to_string_lossy()));
    let failed = match start_from(target, old_version, download.as_deref()) {
        Ok(mut child) => {
            let pid = child.id();
            match handover::took_over(|| child.try_wait().ok().flatten().map(|s| s.code().unwrap_or(-1)), handover::TAKEOVER_GRACE) {
                Ok(()) => {
                    log(&format!("replace: the new version (pid {pid}) runs from {}; leaving", target.display()));
                    return true;
                }
                Err(code) => format!("it ended at once with code {code}"),
            }
        }
        Err(e) => format!("cannot start it: {e}"),
    };
    let back = handover::roll_back(target, &handover::sibling(target, ".old"));
    log(&format!(
        "replace: the new version at {} did not run ({failed}); {}",
        target.display(),
        match back {
            Ok(()) => "the old program file is back".to_string(),
            Err(e) => format!("putting the old program file back failed: {e}"),
        }
    ));
    false
}

/// What the start does after [`on_running_copy`].
pub(super) enum Outcome {
    /// Bring the running copy to the front and leave (as before 1.0).
    Front,
    /// The program now runs from the old copy's place: leave at once.
    Leave,
    /// The old copy is gone: this copy goes on starting.
    Run,
}

/// Another copy holds the lock and its window is `hwnd`: replaces it when
/// this one is newer (`handover::second_start`).
pub(super) fn on_running_copy(hwnd: HWND) -> Outcome {
    let r = match running_copy(hwnd) {
        Ok(r) => r,
        Err(e) => {
            log(&format!("start: another copy is running; it cannot be asked ({e})"));
            return Outcome::Front;
        }
    };
    let this = this_copy();
    let their_exe = r.info.exe.clone().unwrap_or_default();
    let their_version = r.info.version.clone().unwrap_or_else(|| "?".into());
    let folder = r.info.exe.as_deref().map_or(Folder::Protected, folder_of);
    let this_managed = this.exe.as_deref().is_some_and(managed);
    let decision = handover::second_start(&this, this_managed, &r.info, folder);
    log(&format!(
        "start: another copy is running: {their_version} (pid {}, {}{}, {}), this is {VERSION}{}: {decision:?}",
        r.pid,
        their_exe,
        if r.info.packaged { ", Store" } else { "" },
        if r.answered { "answered" } else { "from its file" },
        if this.packaged { " (Store)" } else { "" },
    ));
    if !matches!(decision, SecondStart::Front(_))
        && let Some(why) = replace_refused(r.answered, r.info.exe.as_deref())
    {
        log(&format!("replace: not replacing the running copy: {why}"));
        return Outcome::Front;
    }
    match decision {
        SecondStart::Front(_) => Outcome::Front,
        SecondStart::ReplaceProcess(why) => {
            if !close_running(&r) {
                log("replace: the older copy is still running; it stays");
                return Outcome::Front;
            }
            log(&format!("replace: running {VERSION} from here; the file at {their_exe} is kept ({why})"));
            let _ = REPLACED.set(their_version);
            Outcome::Run
        }
        SecondStart::ReplaceInstalled => {
            let target = PathBuf::from(&their_exe);
            let Some(src) = own_exe() else { return Outcome::Front };
            // While the old copy still runs: a running program's file can be
            // renamed. If this fails the old copy is not touched at all.
            if let Err(e) = handover::replace_installed(&src, &target, handover::ROLLBACK_TRIES, handover::ROLLBACK_PAUSE) {
                log(&format!("replace: cannot put this version in place of {their_exe}: {e}"));
                if !close_running(&r) {
                    return Outcome::Front;
                }
                log(&format!("replace: running {VERSION} from here instead; the file at {their_exe} is kept"));
                let _ = REPLACED.set(their_version);
                return Outcome::Run;
            }
            log(&format!("replace: {VERSION} put in place of {their_exe} (the old file kept as .old)"));
            if !close_running(&r) {
                let back = handover::roll_back(&target, &handover::sibling(&target, ".old"));
                log(&format!("replace: the older copy is still running; it stays, its file put back: {back:?}"));
                return Outcome::Front;
            }
            if run_from(&target, &their_version) {
                return Outcome::Leave;
            }
            // The old copy is gone and its file is back: this copy runs.
            let _ = REPLACED.set(their_version);
            Outcome::Run
        }
    }
}

/// No other copy runs. When this copy runs from Downloads or a temp folder
/// and is newer than the copy installed in its permanent place (the
/// start-with-Windows entry's exe, else the move offer's folder), that file
/// is replaced by this one and the program starts from there. True when
/// this process must leave now.
pub(super) fn replace_installed_elsewhere() -> bool {
    let this = this_copy();
    let Some(mine) = this.exe.clone() else { return false };
    // (Not again after `on_running_copy` closed an older copy this run.)
    if REPLACED.get().is_some() || this.packaged || !transient(&mine) {
        return false;
    }
    let moved = std::env::var("LOCALAPPDATA")
        .ok()
        .map(|l| format!("{}\\{}", crate::relocate::install_dir(&l), crate::relocate::EXE_NAME));
    let candidates = autostart::entry_exe().into_iter().chain(moved);
    for exe in candidates {
        if handover::same_exe(&exe, &mine) || !Path::new(&exe).is_file() {
            continue;
        }
        let installed = CopyInfo { version: file_version(&exe), exe: Some(exe.clone()), packaged: false };
        let folder = folder_of(&exe);
        if !handover::replaces_installed(&this, true, &installed, folder) {
            continue;
        }
        if let Some(why) = replace_refused(false, Some(&exe)) {
            log(&format!("replace: the installed copy at {exe} is left as it is: {why}"));
            continue;
        }
        let old_version = installed.version.clone().unwrap_or_default();
        log(&format!("replace: {VERSION} from {mine} is newer than the installed {old_version} at {exe}"));
        let target = PathBuf::from(&exe);
        if let Err(e) = handover::replace_installed(Path::new(&mine), &target, handover::ROLLBACK_TRIES, handover::ROLLBACK_PAUSE) {
            log(&format!("replace: cannot put this version in place of {exe}: {e}; running from here"));
            return false;
        }
        return run_from(&target, &old_version);
    }
    false
}

/// Once this copy runs (its first timer tick): deletes the downloaded copy
/// it replaced (`--remove-download`), with a few tries while that process
/// ends. Off the UI thread.
pub(super) fn remove_download_later() {
    let Some(named) = handover::arg_value(std::env::args(), handover::REMOVE_DOWNLOAD_ARG) else { return };
    crate::crash_log::spawn_named("remove-download", move || {
        let own = own_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(path) = handover::download_to_remove(Some(&named), &own, transient, file_version, VERSION) else {
            log(&format!("replace: the download {named} is left where it is (not a copy of this version in Downloads or a temp folder)"));
            return;
        };
        let mut last = None;
        for i in 0..10 {
            if i > 0 {
                std::thread::sleep(Duration::from_millis(500));
            }
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    log(&format!("replace: removed the download {}", path.display()));
                    return;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                Err(e) => last = Some(e),
            }
        }
        log(&format!("replace: could not remove the download {}: {}", path.display(), last.map(|e| e.to_string()).unwrap_or_default()));
    });
}

/// The main window's answer to the two messages (`build.rs`): `Some` when
/// `msg` was one of them. `leave` saves and ends this copy.
pub(super) fn on_message(msg: UINT, _w: WPARAM, _l: LPARAM, leave: impl FnOnce()) -> Option<isize> {
    if msg == 0 {
        return None;
    }
    if msg == identify_message() {
        return Some(identity());
    }
    if msg == leave_message() {
        log("replace: a newer copy asked this one to leave");
        leave();
        return Some(0);
    }
    None
}
