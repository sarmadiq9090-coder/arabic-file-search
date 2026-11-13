//! Handing over to a new copy of the program (after an update, a move to the
//! permanent place, a change of language), so that exactly one copy runs in
//! the end and the install always keeps a program that starts.
//!
//! - The old copy starts the new one with [`PREV_PID_ARG`] and its own
//!   process id, and watches it for [`TAKEOVER_GRACE`]: a new copy that ends
//!   at once (blocked by an antivirus, Smart App Control) means the handover
//!   failed; after an update the old program file is then put back
//!   ([`roll_back`]) and the old copy keeps running.
//! - The new copy waits for the old process to end, at most
//!   [`OLD_COPY_WAIT`], ends it if it is still there ([`end_previous`]), and
//!   only then takes the single-instance lock.
//! - The old program file (`bahth.exe.old`) is deleted by the new copy only
//!   once it has shown it runs (its first timer tick), with a few tries; one
//!   it cannot delete yet is deleted at a later start ([`remove_leftovers`]).
//!
//! The decisions and the file moves are free of Win32 and tested on Linux.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// `--prev-pid=<id>:<created>`: the process id of the copy that started
/// this one and when that process was created (FILETIME, 100 ns steps). Ids
/// are reused: a new copy that starts late must not wait for, or end, an
/// unrelated process that got the same id, so both must match.
pub const PREV_PID_ARG: &str = "--prev-pid";
/// How long the new copy waits for the old one to end by itself.
pub const OLD_COPY_WAIT: Duration = Duration::from_secs(10);
/// How long the old copy watches the new one before it leaves.
pub const TAKEOVER_GRACE: Duration = Duration::from_secs(3);

/// The copy that started this one: its process id and creation time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrevCopy {
    pub pid: u32,
    pub created: u64,
}

/// The argument that passes this copy on.
pub fn prev_pid_arg(pid: u32, created: u64) -> String {
    format!("{PREV_PID_ARG}={pid}:{created}")
}

/// The copy given with [`PREV_PID_ARG`], if any (never id 0 or our own; a
/// pid without its creation time is not trusted).
pub fn previous_pid(args: impl IntoIterator<Item = String>, own: u32) -> Option<PrevCopy> {
    args.into_iter()
        .find_map(|a| {
            let (pid, created) = a.strip_prefix(PREV_PID_ARG)?.strip_prefix('=')?.split_once(':')?;
            Some(PrevCopy { pid: pid.parse().ok()?, created: created.parse().ok()? })
        })
        .filter(|p| p.pid != 0 && p.pid != own && p.created != 0)
}

/// Whether the process found under the id is the copy that was named: it was
/// created exactly then (`actual`: its creation time, `None` if unknown).
pub fn is_that_copy(expected: &PrevCopy, actual: Option<u64>) -> bool {
    actual == Some(expected.created)
}

/// What became of the old copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Previous {
    /// It had already ended (or could not be found).
    Gone,
    /// It ended within the wait.
    Exited,
    /// It was still there after the wait and was ended.
    Terminated,
    /// It was still there and could not be ended.
    Stuck,
}

/// Waits for the old copy and ends it if it does not go (see the module
/// notes). `wait(budget)`: true once it has ended (`None`: no such process);
/// `terminate()`: true when it was ended. The line for the log is returned
/// with the outcome.
pub fn end_previous(
    pid: u32,
    wait: impl Fn(Duration) -> Option<bool>,
    terminate: impl FnOnce() -> bool,
) -> (Previous, String) {
    end_previous_within(pid, OLD_COPY_WAIT, wait, terminate)
}

/// [`end_previous`] with its own wait (`budget`) before the copy is ended.
pub fn end_previous_within(
    pid: u32,
    budget: Duration,
    wait: impl Fn(Duration) -> Option<bool>,
    terminate: impl FnOnce() -> bool,
) -> (Previous, String) {
    let started = Instant::now();
    let outcome = match wait(budget) {
        None => Previous::Gone,
        Some(true) => Previous::Exited,
        Some(false) if terminate() && wait(Duration::from_secs(2)) != Some(false) => Previous::Terminated,
        Some(false) => Previous::Stuck,
    };
    let ms = started.elapsed().as_millis();
    let line = match outcome {
        Previous::Gone => format!("handover: previous copy (pid {pid}) had already ended"),
        Previous::Exited => format!("handover: previous copy (pid {pid}) ended after {ms} ms"),
        Previous::Terminated => format!("handover: previous copy (pid {pid}) still running after {} s; ended it", budget.as_secs()),
        Previous::Stuck => format!("handover: previous copy (pid {pid}) still running and could not be ended; starting anyway"),
    };
    (outcome, line)
}

/// The old copy watches the new one for `grace`: `exited()` gives its exit
/// code once it has ended. True when it is still running at the end (it is
/// taking over); false when it ended (the handover failed).
pub fn took_over(mut exited: impl FnMut() -> Option<i32>, grace: Duration) -> Result<(), i32> {
    let deadline = Instant::now() + grace;
    loop {
        if let Some(code) = exited() {
            return Err(code);
        }
        if Instant::now() >= deadline {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// `exe` with `suffix` added to its file name (`bahth.exe.old`).
pub fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    exe.with_file_name(name)
}

/// How often each rename of a roll-back is tried, and the pause between: an
/// antivirus that blocked the new program may hold its file for a moment.
pub const ROLLBACK_TRIES: u32 = 10;
pub const ROLLBACK_PAUSE: Duration = Duration::from_millis(300);

/// An update was installed but the new program did not start: puts the old
/// program (`old`, the running one) back as `exe`, the new one going to
/// `exe.new` (removed at a later start). Each rename is tried
/// [`ROLLBACK_TRIES`] times. On failure `exe` is left as it was and the
/// caller keeps the old copy running.
pub fn roll_back(exe: &Path, old: &Path) -> Result<(), String> {
    roll_back_with(exe, old, &|a, b| std::fs::rename(a, b), ROLLBACK_TRIES, ROLLBACK_PAUSE)
}

fn roll_back_with(
    exe: &Path,
    old: &Path,
    rename: &dyn Fn(&Path, &Path) -> std::io::Result<()>,
    tries: u32,
    pause: Duration,
) -> Result<(), String> {
    let again = |from: &Path, to: &Path| {
        let mut last = Ok(());
        for i in 0..tries.max(1) {
            if i > 0 {
                std::thread::sleep(pause);
            }
            last = rename(from, to);
            if last.is_ok() {
                break;
            }
        }
        last
    };
    if !old.is_file() {
        return Err(format!("the previous version is not there ({})", old.display()));
    }
    let aside = sibling(exe, ".new");
    let _ = std::fs::remove_file(&aside);
    if exe.exists() {
        again(exe, &aside).map_err(|e| format!("cannot move the new version aside after {tries} tries: {e}"))?;
    }
    if let Err(e) = again(old, exe) {
        let _ = again(&aside, exe);
        return Err(format!("cannot put the previous version back after {tries} tries: {e}"));
    }
    Ok(())
}

/// Puts `exe.old` back as `exe` when an interrupted update left nothing
/// else (it is then the user's only program). True when it did.
pub fn restore_if_missing(exe: &Path) -> bool {
    let old = sibling(exe, ".old");
    !exe.exists() && old.is_file() && std::fs::rename(&old, exe).or_else(|_| std::fs::copy(&old, exe).map(drop)).is_ok()
}

/// Deletes `exe.old` and `exe.new` left by an update, never `exe` itself and
/// never while `exe` is missing; `tries` attempts, `pause` apart (the old
/// copy may still be letting go of its file). The line for the log, when
/// there was anything to delete.
pub fn remove_leftovers(exe: &Path, tries: u32, pause: Duration) -> Option<String> {
    if !exe.is_file() {
        return None;
    }
    let mut lines = Vec::new();
    for suffix in [".old", ".new"] {
        let p = sibling(exe, suffix);
        if !p.exists() {
            continue;
        }
        let mut last = None;
        for i in 0..tries.max(1) {
            if i > 0 {
                std::thread::sleep(pause);
            }
            match std::fs::remove_file(&p) {
                Ok(()) => {
                    last = None;
                    break;
                }
                Err(e) => last = Some(e),
            }
        }
        let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
        lines.push(match last {
            None => format!("removed {name}"),
            Some(e) => format!("kept {name} ({e}); trying again at the next start"),
        });
    }
    (!lines.is_empty()).then(|| format!("update leftovers: {}", lines.join("; ")))
}

/// What a start does about the trial of a new version (see [`trial`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trial {
    /// Nothing to do.
    Run,
    /// Started as the new version after an update: write the trial marker
    /// now, before any window; the first timer tick clears it.
    Begin,
    /// The marker is there and so is the old program: the new version did
    /// not reach its first tick last time (blocked, or ended while its window
    /// was being made). Put the old program back and start it instead.
    RollBack,
    /// A marker without an old program to go back to: just remove it.
    Forget,
}

/// The takeover grace (`TAKEOVER_GRACE`) covers only the start of the new
/// copy before it waits for the old one; window creation, WebView2 and the
/// index load come after the old copy has gone. So the new version is on
/// trial until its first timer tick: `updated` (started with `--updated`),
/// `marker` (the trial marker exists), `old` (`bahth.exe.old` exists).
pub fn trial(updated: bool, marker: bool, old: bool) -> Trial {
    match (updated, marker, old) {
        (true, _, _) => Trial::Begin,
        (false, true, true) => Trial::RollBack,
        (false, true, false) => Trial::Forget,
        (false, false, _) => Trial::Run,
    }
}

/// The titles the running copy's window may have: it keeps the language it
/// started with, which may not be the one saved since.
pub fn window_titles<'a>(current: &'a str, other: &'a str) -> [&'a str; 2] {
    [current, other]
}

// ------------------------------------------------- a newer copy starts

// A start that finds another copy running (the single-instance lock held)
// used to bring that copy to the front and leave, so the downloaded 1.0
// started while 0.10.0 sat by the clock left the user on 0.10.0 without a
// word. The owner's decision (2026-11-13): a newer copy replaces the older
// one, with no question:
//
// - The older copy is asked to leave ([`LEAVE_MESSAGE`], the same polite
//   way out as every handover, `Leaving::Restart`) and ended if it does not
//   go in time ([`end_previous_within`]). A copy of 0.10.0 or older knows
//   neither message ([`WayOut`]).
// - Its program file is replaced by this one ([`replace_installed`]: the
//   updater's swap, keeping `bahth.exe.old` for the trial and the roll-back
//   of an update), and the program starts again from there with
//   `--updated`, so the start-with-Windows entry, the Start-menu shortcut
//   and the data stay as they are. A copy started from Downloads or a temp
//   folder is deleted by the copy that took its place ([`download_to_remove`]).
// - A folder that must not be written to (Program Files, a package
//   manager's, read-only) or a copy in Downloads or a temp folder keeps its
//   file: the old copy is closed and this one runs from where it is.
// - An older or the same version, a Store copy and a portable one: the
//   running copy comes to the front, as before.
// - Whatever fails, a working copy is left: the file swap is undone, and an
//   old copy that cannot be ended keeps running.

/// Asked through the running copy's main window (`SendMessageTimeout`):
/// it answers with [`pack_identity`]. Copies before 1.0 answer 0.
pub const IDENTIFY_MESSAGE: &str = "BahthSari.Identify.v1";
/// Posted to the running copy's main window: save and leave, a newer copy
/// takes over (`Leaving::Restart`).
pub const LEAVE_MESSAGE: &str = "BahthSari.LeaveForNewerCopy.v1";
/// `--replaced=<version>`: this copy took the place of that version (for
/// the notice after the switch, [`replaced_notice`]).
pub const REPLACED_ARG: &str = "--replaced";
/// `--remove-download=<path>`: the downloaded copy this one replaced, to be
/// deleted once this copy runs ([`download_to_remove`]).
pub const REMOVE_DOWNLOAD_ARG: &str = "--remove-download";

/// How the running copy is asked to go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WayOut {
    /// [`LEAVE_MESSAGE`]: it saves and leaves (1.0 and later).
    Ask,
    /// WM_CLOSE: a copy before 1.0 whose close button really exits (no
    /// icon by the clock, no floating box) saves and leaves.
    Close,
    /// A copy before 1.0 whose close button only hides it (and shows a
    /// balloon "still running here"): nothing asks it to leave, so it is
    /// ended at once. It loses only what it had not saved yet (its settings
    /// since the last close; unsaved index changes are scanned again).
    End,
}

impl WayOut {
    /// `answered`: the copy answered [`IDENTIFY_MESSAGE`]; `close_exits`:
    /// its settings say the close button exits.
    pub fn for_copy(answered: bool, close_exits: bool) -> WayOut {
        match (answered, close_exits) {
            (true, _) => WayOut::Ask,
            (false, true) => WayOut::Close,
            (false, false) => WayOut::End,
        }
    }

    /// How long it is waited for before it is ended.
    pub fn wait(self) -> Duration {
        match self {
            WayOut::Ask | WayOut::Close => OLD_COPY_WAIT,
            WayOut::End => Duration::ZERO,
        }
    }
}

/// The answer to [`IDENTIFY_MESSAGE`]: a positive 32-bit value (an LRESULT
/// on 32-bit Windows too) with the version (9 bits major, 10 bits minor and
/// patch, each capped), whether this is the Store copy, and a marker bit.
pub fn pack_identity(version: &str, packaged: bool) -> isize {
    let v = crate::update::parse_version(version).unwrap_or_default();
    let part = |i: usize, max: u64| v.get(i).copied().unwrap_or(0).min(max) as isize;
    (1 << 30) | (isize::from(packaged) << 29) | (part(0, 511) << 20) | (part(1, 1023) << 10) | part(2, 1023)
}

/// The version and the Store flag of an [`IDENTIFY_MESSAGE`] answer; `None`
/// for no answer (0, a copy before 1.0) or anything else.
pub fn unpack_identity(v: isize) -> Option<(String, bool)> {
    if v < 0 || v >> 31 != 0 || v & (1 << 30) == 0 {
        return None;
    }
    let packaged = v & (1 << 29) != 0;
    Some((format!("{}.{}.{}", (v >> 20) & 511, (v >> 10) & 1023, v & 1023), packaged))
}

/// A copy of the program: its version (from its answer or its file's
/// version resource), its exe and whether it is the Store copy.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CopyInfo {
    pub version: Option<String>,
    pub exe: Option<String>,
    pub packaged: bool,
}

/// The folder of the running (older) copy's exe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Folder {
    /// Its own folder, which this copy can write to: the file is replaced.
    Writable,
    /// Downloads or a temp folder: not a place to put the new program in.
    Transient,
    /// Installed by winget, Scoop or Chocolatey, which own the file.
    Managed,
    /// Program Files or a folder that cannot be written to.
    Protected,
}

/// What a start does when another copy runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecondStart {
    /// Brings the running copy to the front and leaves (the reason, for
    /// the log).
    Front(&'static str),
    /// Closes the running copy, puts this program in place of its file and
    /// starts it from there.
    ReplaceInstalled,
    /// Closes the running copy and runs from here (the reason its file is
    /// kept, for the log).
    ReplaceProcess(&'static str),
}

/// Whether the running copy behind the program's window is really this
/// program, before it is ended or its file replaced: it answered
/// [`IDENTIFY_MESSAGE`], its file is named `bahth…`, or its version
/// resource names bahth or InstaFind (`resource_names`: its ProductName,
/// InternalName, OriginalFilename). Another program with the same window
/// title is only brought to the front.
#[cfg_attr(not(windows), allow(dead_code))] // older_copy (Windows)
pub fn is_this_program(answered: bool, exe: Option<&str>, resource_names: &[String]) -> bool {
    let file_name = exe.map(|e| e.rsplit(['\\', '/']).next().unwrap_or(e).to_lowercase());
    answered
        || file_name.is_some_and(|n| n.starts_with("bahth"))
        || resource_names.iter().any(|n| {
            let n = n.trim().to_lowercase();
            n == "bahth" || n == "bahth.exe" || n == "instafind"
        })
}

/// Before a running copy is replaced: `None` when it may be, else why it
/// is only brought to the front. `theirs` / `mine`: the PE machine types of
/// its file and of this one (`None` when unknown); a 32-bit or x64 build
/// never takes the place of another CPU's build, so the updater keeps
/// fetching the right one.
#[cfg_attr(not(windows), allow(dead_code))] // older_copy (Windows)
pub fn replace_refused(is_this_program: bool, theirs: Option<u16>, mine: Option<u16>) -> Option<&'static str> {
    if !is_this_program {
        return Some("the window with the program's title is not this program's");
    }
    match (theirs, mine) {
        (Some(t), Some(m)) if t != m => Some("the running copy is built for another CPU type"),
        _ => None,
    }
}

/// Whether two exe paths name the same file (case, `/`, `\\?\`). The image
/// of a copy that was updated in place may be `bahth.exe.old`: the same
/// install as `bahth.exe`.
pub fn same_exe(a: &str, b: &str) -> bool {
    let norm = |p: &str| {
        let n = crate::relocate::normalize(p);
        match n.strip_suffix(".exe.old\\") {
            Some(base) => format!("{base}.exe\\"),
            None => n,
        }
    };
    norm(a) == norm(b)
}

/// The decision when this copy (`this`; `this_managed`: installed by a
/// package manager) starts while `running` runs from a `folder` (see the
/// notes above).
pub fn second_start(this: &CopyInfo, this_managed: bool, running: &CopyInfo, folder: Folder) -> SecondStart {
    use crate::update::is_newer;
    let (Some(mine), Some(theirs)) = (this.version.as_deref(), running.version.as_deref()) else {
        return SecondStart::Front("the running copy's version is not known");
    };
    if this.packaged != running.packaged {
        return SecondStart::Front("a Store copy and a portable copy never replace each other");
    }
    if !is_newer(mine, theirs) {
        let same_place = matches!((&this.exe, &running.exe), (Some(a), Some(b)) if same_exe(a, b));
        return SecondStart::Front(if is_newer(theirs, mine) {
            "a newer copy is running"
        } else if same_place {
            "the same copy"
        } else {
            "the same version runs from another folder"
        });
    }
    if this.packaged {
        // (Two versions of the Store package: Windows replaces it itself.)
        return SecondStart::Front("the Store updates its own copy");
    }
    let Some(theirs_exe) = running.exe.as_deref() else {
        return SecondStart::ReplaceProcess("the running copy's file is not known");
    };
    if this.exe.as_deref().is_some_and(|mine| same_exe(mine, theirs_exe)) {
        return SecondStart::ReplaceProcess("this is already the program file of the running copy");
    }
    if this_managed {
        return SecondStart::ReplaceProcess("this copy is installed by a package manager, which owns its file");
    }
    match folder {
        Folder::Writable => SecondStart::ReplaceInstalled,
        Folder::Transient => SecondStart::ReplaceProcess("the running copy is in Downloads or a temp folder"),
        Folder::Managed => SecondStart::ReplaceProcess("the running copy is installed by a package manager, which owns its file"),
        Folder::Protected => SecondStart::ReplaceProcess("the running copy's folder is protected or cannot be written to"),
    }
}

/// No other copy runs, and this one (`this_transient`: started from
/// Downloads or a temp folder) is newer than the copy installed at
/// `installed` (the start-with-Windows entry's exe, or the permanent place
/// the move offer uses): true when that file is to be replaced by this one,
/// as when the older copy runs.
pub fn replaces_installed(this: &CopyInfo, this_transient: bool, installed: &CopyInfo, folder: Folder) -> bool {
    this_transient && second_start(this, false, installed, folder) == SecondStart::ReplaceInstalled
}

/// What tells Program Files apart (`var` reads an environment variable):
/// the file of a copy installed there is never replaced.
pub fn in_program_files(exe: &str, var: impl Fn(&str) -> Option<String>) -> bool {
    let dir = crate::relocate::normalize(exe);
    ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"]
        .iter()
        .filter_map(|k| var(k).filter(|v| !v.trim().is_empty()))
        .any(|root| dir.starts_with(&crate::relocate::normalize(&root)))
}

/// The folder kind of `exe` from what was found about it.
pub fn folder_kind(managed: bool, protected: bool, transient: bool, writable: bool) -> Folder {
    if managed {
        Folder::Managed
    } else if protected || !writable {
        Folder::Protected
    } else if transient {
        Folder::Transient
    } else {
        Folder::Writable
    }
}

/// Copies the program `src` (this one) next to `exe` as `exe.new`, a new
/// file (so a download's "from the internet" mark is not carried over), and
/// swaps it in with the updater's `swap_in`: `exe` becomes `exe.old` (the
/// update trial and its roll-back use it), the copy becomes `exe`. Each
/// swap is tried `tries` times, `pause` apart (the old copy, or a scanner,
/// may still hold the file). On failure `exe` is as it was and `exe.new` is
/// gone.
pub fn replace_installed(src: &Path, exe: &Path, tries: u32, pause: Duration) -> Result<(), String> {
    use std::io::{self, Write};
    let new = sibling(exe, ".new");
    let old = sibling(exe, ".old");
    let copied = (|| {
        let mut from = std::fs::File::open(src)?;
        let mut to = std::fs::File::create(&new)?;
        let n = io::copy(&mut from, &mut to)?;
        to.flush()?;
        to.sync_all()?;
        if n != std::fs::metadata(src)?.len() {
            return Err(io::Error::other("the copy is incomplete"));
        }
        Ok(())
    })();
    if let Err(e) = copied {
        let _ = std::fs::remove_file(&new);
        return Err(format!("cannot copy the program next to {}: {e}", exe.display()));
    }
    let mut last = Ok(());
    for i in 0..tries.max(1) {
        if i > 0 {
            std::thread::sleep(pause);
        }
        last = crate::update::swap_in(exe, &new, &old);
        if last.is_ok() {
            break;
        }
    }
    if last.is_err() {
        let _ = std::fs::remove_file(&new);
    }
    last.map_err(|e| format!("{e} (after {} tries)", tries.max(1)))
}

/// The value of `--name=<value>` in `args`, if any (not empty).
pub fn arg_value(args: impl IntoIterator<Item = String>, name: &str) -> Option<String> {
    args.into_iter().find_map(|a| Some(a.strip_prefix(name)?.strip_prefix('=')?.to_string())).filter(|v| !v.trim().is_empty())
}

/// The short notice after the switch.
pub fn replaced_notice(old: &str, new: &str) -> String {
    trf!("تم استبدال النسخة القديمة ({old}) بالنسخة {new}.", "Replaced the old copy ({old}) with {new}.")
}

/// The downloaded copy named by [`REMOVE_DOWNLOAD_ARG`] that this copy may
/// delete: only an exe in Downloads or a temp folder (`transient`), never
/// this copy's own file, and only when it is the same version of the
/// program as this one (`version_of`: its file's version resource).
pub fn download_to_remove(
    named: Option<&str>,
    own_exe: &str,
    transient: impl Fn(&str) -> bool,
    version_of: impl Fn(&str) -> Option<String>,
    own_version: &str,
) -> Option<PathBuf> {
    let p = named?.trim();
    let is_exe = Path::new(p).extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"));
    let same_version = || version_of(p).is_some_and(|v| !crate::update::is_newer(&v, own_version) && !crate::update::is_newer(own_version, &v));
    (is_exe && !same_exe(p, own_exe) && transient(p) && same_version()).then(|| PathBuf::from(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::fs;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(name: &str) -> TempDir {
            let d = std::env::temp_dir().join(format!("bahth_handover_{name}_{}", std::process::id()));
            let _ = fs::remove_dir_all(&d);
            fs::create_dir_all(&d).unwrap();
            TempDir(d)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_previous_copy_is_passed_and_read_back() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let arg = prev_pid_arg(4321, 133_000_000_000_000_000);
        assert_eq!(arg, "--prev-pid=4321:133000000000000000");
        let got = previous_pid(args(&["bahth.exe", "--updated", &arg]), 9);
        assert_eq!(got, Some(PrevCopy { pid: 4321, created: 133_000_000_000_000_000 }));
        assert_eq!(previous_pid(args(&["bahth.exe", "--moved"]), 9), None);
        // A bare pid (no creation time) is not trusted, nor id 0, nor our own.
        for bad in ["--prev-pid", "--prev-pid=", "--prev-pid=4321", "--prev-pid=x:1", "--prev-pid=0:5", "--prev-pid=9:5", "--prev-pid=7:0", "--prev-pid=7:y", "--prev-pidx=5:1"] {
            assert_eq!(previous_pid(args(&["bahth.exe", bad]), 9), None, "{bad}");
        }
    }

    /// Process ids are reused: only the process created at the given time
    /// is the old copy; any other process with that id is left alone.
    #[test]
    fn a_reused_process_id_is_not_the_old_copy() {
        let prev = PrevCopy { pid: 4321, created: 1000 };
        assert!(is_that_copy(&prev, Some(1000)));
        assert!(!is_that_copy(&prev, Some(2000)), "another process got the id");
        assert!(!is_that_copy(&prev, None), "unknown: hands off");
    }

    #[test]
    fn the_old_copy_is_waited_for_then_ended() {
        // Ended by itself.
        let (out, line) = end_previous(7, |_| Some(true), || panic!("must not be ended"));
        assert_eq!(out, Previous::Exited);
        assert!(line.contains("pid 7") && line.contains("ended after"), "{line}");
        // Gone already.
        assert_eq!(end_previous(7, |_| None, || panic!()).0, Previous::Gone);
        // Still there after the wait: ended, and then seen gone.
        let waits = Cell::new(0);
        let (out, line) = end_previous(
            7,
            |budget| {
                waits.set(waits.get() + 1);
                assert!(budget <= OLD_COPY_WAIT);
                Some(waits.get() > 1)
            },
            || true,
        );
        assert_eq!((out, waits.get()), (Previous::Terminated, 2));
        assert!(line.contains("ended it"), "{line}");
        // Cannot be ended: this copy starts anyway (never no program at all).
        assert_eq!(end_previous(7, |_| Some(false), || false).0, Previous::Stuck);
        assert_eq!(end_previous(7, |_| Some(false), || true).0, Previous::Stuck);
    }

    #[test]
    fn a_new_copy_that_ends_at_once_did_not_take_over() {
        assert!(TAKEOVER_GRACE < OLD_COPY_WAIT, "the old copy leaves before the new one gives up waiting");
        assert_eq!(took_over(|| None, Duration::from_millis(80)), Ok(()));
        let polls = Cell::new(0);
        let r = took_over(
            || {
                polls.set(polls.get() + 1);
                (polls.get() == 3).then_some(5)
            },
            Duration::from_secs(10),
        );
        assert_eq!(r, Err(5));
    }

    /// The update put a new bahth.exe in place, but it does not start: the
    /// old program goes back under its name, and the install still starts.
    #[test]
    fn a_failed_update_is_rolled_back() {
        let t = TempDir::new("rollback");
        let exe = t.0.join("bahth.exe");
        let old = sibling(&exe, ".old");
        fs::write(&exe, b"v2").unwrap();
        fs::write(&old, b"v1").unwrap();
        roll_back(&exe, &old).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"v1");
        assert!(!old.exists());
        assert_eq!(fs::read(sibling(&exe, ".new")).unwrap(), b"v2");
        // Nothing to go back to: refused, bahth.exe untouched.
        assert!(roll_back(&exe, &old).is_err());
        assert_eq!(fs::read(&exe).unwrap(), b"v1");
        // The leftovers then go, bahth.exe stays.
        assert_eq!(remove_leftovers(&exe, 1, Duration::ZERO).as_deref(), Some("update leftovers: removed bahth.exe.new"));
        assert!(exe.is_file());
    }

    /// The blocked new program is held by an antivirus for a while: the
    /// roll-back tries again and gets through; held for good, it fails with
    /// bahth.exe untouched (the old copy then keeps running).
    #[test]
    fn a_roll_back_retries_a_file_that_is_held() {
        let t = TempDir::new("rollback_held");
        let exe = t.0.join("bahth.exe");
        let old = sibling(&exe, ".old");
        fs::write(&exe, b"v2").unwrap();
        fs::write(&old, b"v1").unwrap();
        let held = Cell::new(3);
        let flaky = |a: &Path, b: &Path| {
            if held.get() > 0 {
                held.set(held.get() - 1);
                return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "in use"));
            }
            fs::rename(a, b)
        };
        roll_back_with(&exe, &old, &flaky, 5, Duration::from_millis(1)).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"v1");
        const { assert!(ROLLBACK_TRIES >= 5) };
        // Held for good: refused, and the new program is still in place.
        fs::write(&old, b"v1").unwrap();
        fs::write(&exe, b"v2").unwrap();
        let tries = Cell::new(0);
        let never = |_: &Path, _: &Path| {
            tries.set(tries.get() + 1);
            Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "in use"))
        };
        let e = roll_back_with(&exe, &old, &never, 4, Duration::ZERO).unwrap_err();
        assert!(e.contains("after 4 tries") && e.contains("in use"), "{e}");
        assert_eq!(tries.get(), 4);
        assert_eq!(fs::read(&exe).unwrap(), b"v2");
        assert!(old.is_file());
    }

    #[test]
    fn leftovers_go_only_while_the_program_is_there() {
        let t = TempDir::new("leftovers");
        let exe = t.0.join("bahth.exe");
        let (old, new) = (sibling(&exe, ".old"), sibling(&exe, ".new"));
        assert_eq!(remove_leftovers(&exe, 3, Duration::ZERO), None);
        // Only the old copy: it is the user's program, put back, never deleted.
        fs::write(&old, b"v1").unwrap();
        assert_eq!(remove_leftovers(&exe, 3, Duration::ZERO), None);
        assert!(old.exists());
        assert!(restore_if_missing(&exe));
        assert_eq!(fs::read(&exe).unwrap(), b"v1");
        assert!(!restore_if_missing(&exe));
        // After a good update both leftovers go.
        fs::write(&old, b"v0").unwrap();
        fs::write(&new, b"half").unwrap();
        let line = remove_leftovers(&exe, 3, Duration::ZERO).unwrap();
        assert_eq!(line, "update leftovers: removed bahth.exe.old; removed bahth.exe.new");
        assert!(exe.is_file() && !old.exists() && !new.exists());
        // One that cannot be deleted (here a folder) is kept, and said so.
        fs::create_dir(&old).unwrap();
        fs::write(old.join("x"), b"").unwrap();
        let line = remove_leftovers(&exe, 2, Duration::from_millis(1)).unwrap();
        assert!(line.starts_with("update leftovers: kept bahth.exe.old (") && line.ends_with("trying again at the next start"), "{line}");
        assert!(exe.is_file());
    }

    #[test]
    fn a_new_version_that_never_reached_its_first_tick_is_rolled_back() {
        // Started after an update: on trial, whatever was left before.
        for (marker, old) in [(false, false), (false, true), (true, true), (true, false)] {
            assert_eq!(trial(true, marker, old), Trial::Begin);
        }
        // The next start finds the trial unfinished: back to the old program.
        assert_eq!(trial(false, true, true), Trial::RollBack);
        // Nothing to go back to (cleaned up already): the marker just goes.
        assert_eq!(trial(false, true, false), Trial::Forget);
        // No trial: a normal start, an old file or not.
        assert_eq!(trial(false, false, true), Trial::Run);
        assert_eq!(trial(false, false, false), Trial::Run);
    }

    fn copy(version: &str, exe: &str, packaged: bool) -> CopyInfo {
        CopyInfo { version: (!version.is_empty()).then(|| version.into()), exe: (!exe.is_empty()).then(|| exe.into()), packaged }
    }

    const DL: &str = "C:\\Users\\Sara\\Downloads\\bahth.exe";
    const MOVED: &str = "C:\\Users\\Sara\\AppData\\Local\\Programs\\InstaFind\\bahth.exe";
    const STORE: &str = "C:\\Program Files\\WindowsApps\\FastKiwi.InstaFind_1.0.0.0_x64__abc\\bahth.exe";

    /// The PC test: the downloaded 1.0 starts while 0.10.0 runs by the clock.
    #[test]
    fn a_newer_copy_replaces_the_older_one() {
        let new = copy("1.0.0", DL, false);
        let old = copy("0.10.0", MOVED, false);
        assert_eq!(second_start(&new, false, &old, Folder::Writable), SecondStart::ReplaceInstalled);
        // Patch and minor steps count, and 1.0 is newer than 0.10.0.
        assert_eq!(second_start(&copy("1.0.1", DL, false), false, &copy("1.0.0", MOVED, false), Folder::Writable), SecondStart::ReplaceInstalled);
        assert_eq!(second_start(&copy("0.10.0", DL, false), false, &copy("0.9.1", MOVED, false), Folder::Writable), SecondStart::ReplaceInstalled);
        // Its file kept, the old copy closed, this one runs from here.
        for (folder, why) in [(Folder::Transient, "Downloads"), (Folder::Managed, "package manager"), (Folder::Protected, "protected")] {
            match second_start(&new, false, &old, folder) {
                SecondStart::ReplaceProcess(r) => assert!(r.contains(why), "{folder:?}: {r}"),
                other => panic!("{folder:?}: {other:?}"),
            }
        }
        // This copy is winget's: its own file stays the program.
        assert!(matches!(second_start(&new, true, &old, Folder::Writable), SecondStart::ReplaceProcess(_)));
        // The older copy's file was overwritten in place (same path): just close it.
        assert!(matches!(second_start(&copy("1.0.0", MOVED, false), false, &old, Folder::Writable), SecondStart::ReplaceProcess(_)));
        // The image of a copy updated in place.
        let renamed = copy("0.10.0", &format!("{MOVED}.old"), false);
        assert!(matches!(second_start(&copy("1.0.0", MOVED, false), false, &renamed, Folder::Writable), SecondStart::ReplaceProcess(_)));
        // Its file not known (QueryFullProcessImageName refused): not replaced.
        assert!(matches!(second_start(&new, false, &copy("0.10.0", "", false), Folder::Writable), SecondStart::ReplaceProcess(_)));
    }

    #[test]
    fn otherwise_the_running_copy_comes_to_the_front() {
        let front = |this: &CopyInfo, running: &CopyInfo| match second_start(this, false, running, Folder::Writable) {
            SecondStart::Front(why) => why,
            other => panic!("{this:?} / {running:?}: {other:?}"),
        };
        // An older copy started while a newer one runs.
        assert_eq!(front(&copy("0.10.0", DL, false), &copy("1.0.0", MOVED, false)), "a newer copy is running");
        // The same version: the same copy, or another folder (logged).
        assert_eq!(front(&copy("1.0.0", MOVED, false), &copy("1.0.0", MOVED, false)), "the same copy");
        assert_eq!(front(&copy("1.0.0", &MOVED.to_uppercase().replace('\\', "/"), false), &copy("1.0.0", MOVED, false)), "the same copy");
        assert_eq!(front(&copy("1.0.0", DL, false), &copy("1.0.0", MOVED, false)), "the same version runs from another folder");
        assert_eq!(front(&copy("1.0", DL, false), &copy("1.0.0.0", MOVED, false)), "the same version runs from another folder");
        // Store and portable never replace each other, either way.
        for (this, running) in [(copy("1.0.0", DL, false), copy("0.10.0", STORE, true)), (copy("1.1.0", STORE, true), copy("1.0.0", MOVED, false))] {
            assert_eq!(front(&this, &running), "a Store copy and a portable copy never replace each other");
        }
        assert_eq!(front(&copy("1.1.0", STORE, true), &copy("1.0.0", STORE, true)), "the Store updates its own copy");
        // The running copy's version not known.
        assert_eq!(front(&copy("1.0.0", DL, false), &copy("", MOVED, false)), "the running copy's version is not known");
        // A version that does not parse is never "newer".
        assert_eq!(front(&copy("1.0.0", DL, false), &CopyInfo { version: Some("x".into()), ..Default::default() }), "the same version runs from another folder");
    }

    #[test]
    fn a_newer_download_replaces_an_installed_copy_that_is_not_running() {
        let new = copy("1.0.0", DL, false);
        assert!(replaces_installed(&new, true, &copy("0.10.0", MOVED, false), Folder::Writable));
        // Not from a copy the user put somewhere of their own, not an older
        // or the same version, not a protected or managed folder.
        assert!(!replaces_installed(&new, false, &copy("0.10.0", MOVED, false), Folder::Writable));
        assert!(!replaces_installed(&new, true, &copy("1.0.0", MOVED, false), Folder::Writable));
        assert!(!replaces_installed(&new, true, &copy("1.1.0", MOVED, false), Folder::Writable));
        assert!(!replaces_installed(&new, true, &copy("0.10.0", MOVED, false), Folder::Protected));
        assert!(!replaces_installed(&new, true, &copy("0.10.0", MOVED, false), Folder::Managed));
        assert!(!replaces_installed(&new, true, &copy("0.10.0", DL, false), Folder::Writable), "its own file");
        assert!(!replaces_installed(&copy("1.0.0", DL, true), true, &copy("0.10.0", MOVED, false), Folder::Writable));
    }

    #[test]
    fn the_folder_of_the_running_copy() {
        assert_eq!(folder_kind(false, false, false, true), Folder::Writable);
        assert_eq!(folder_kind(false, false, true, true), Folder::Transient);
        assert_eq!(folder_kind(true, false, false, true), Folder::Managed);
        assert_eq!(folder_kind(false, true, false, true), Folder::Protected);
        assert_eq!(folder_kind(false, false, false, false), Folder::Protected);
        let env = |k: &str| match k {
            "ProgramFiles" => Some("C:\\Program Files".to_string()),
            "ProgramFiles(x86)" => Some("C:\\Program Files (x86)".to_string()),
            "ProgramW6432" => Some(String::new()),
            _ => None,
        };
        assert!(in_program_files("C:\\Program Files\\InstaFind\\bahth.exe", env));
        assert!(in_program_files("c:/program files (x86)/InstaFind/bahth.exe", env));
        assert!(!in_program_files("C:\\Program Files Old\\bahth.exe", env));
        assert!(!in_program_files(MOVED, env));
        assert!(!in_program_files(MOVED, |_| None));
    }

    #[test]
    fn the_running_copy_says_its_version() {
        for (v, packaged) in [("1.0.0", false), ("0.10.0", true), ("12.345.678", false), ("511.1023.1023", true)] {
            let n = pack_identity(v, packaged);
            assert!(n > 0 && n <= i32::MAX as isize, "fits a 32-bit LRESULT");
            assert_eq!(unpack_identity(n), Some((v.to_string(), packaged)));
        }
        assert_eq!(unpack_identity(pack_identity("1.2", false)), Some(("1.2.0".into(), false)));
        assert_eq!(unpack_identity(pack_identity("v2.0.1.7", false)), Some(("2.0.1".into(), false)));
        // A copy that does not know the message, or a strange answer.
        for bad in [0, 1, -1, 5 << 20, isize::MIN, 1 << 31] {
            assert_eq!(unpack_identity(bad), None, "{bad}");
        }
        assert_ne!(IDENTIFY_MESSAGE, LEAVE_MESSAGE);
    }

    #[test]
    fn the_older_copy_is_asked_to_go_as_it_can_be() {
        assert_eq!(WayOut::for_copy(true, false), WayOut::Ask);
        assert_eq!(WayOut::for_copy(true, true), WayOut::Ask);
        // 0.10.0 without an icon by the clock: its close button exits.
        assert_eq!(WayOut::for_copy(false, true), WayOut::Close);
        // 0.10.0 by the clock: WM_CLOSE would only hide it, with a balloon.
        assert_eq!(WayOut::for_copy(false, false), WayOut::End);
        assert_eq!(WayOut::Ask.wait(), OLD_COPY_WAIT);
        assert_eq!(WayOut::Close.wait(), OLD_COPY_WAIT);
        assert_eq!(WayOut::End.wait(), Duration::ZERO);
        // No wait: it is ended at once (the wait sees it still there).
        let (out, line) = end_previous_within(3, WayOut::End.wait(), |b| Some(b > Duration::ZERO && b != WayOut::End.wait()), || true);
        assert_eq!(out, Previous::Terminated, "{line}");
    }

    #[test]
    fn the_new_program_takes_the_old_ones_place() {
        let t = TempDir::new("replace");
        let exe = t.0.join("bahth.exe");
        let src = t.0.join("download.exe");
        fs::write(&exe, b"v0.10").unwrap();
        fs::write(&src, b"v1.0").unwrap();
        replace_installed(&src, &exe, 3, Duration::ZERO).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"v1.0");
        // The old program kept for the trial's roll-back, which works.
        assert_eq!(fs::read(sibling(&exe, ".old")).unwrap(), b"v0.10");
        assert!(!sibling(&exe, ".new").exists());
        assert!(src.is_file(), "the download is removed later, by the new copy");
        roll_back(&exe, &sibling(&exe, ".old")).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"v0.10");
        // A source that cannot be read: nothing changes.
        let _ = fs::remove_file(sibling(&exe, ".new"));
        assert!(replace_installed(&t.0.join("missing.exe"), &exe, 2, Duration::ZERO).is_err());
        assert_eq!(fs::read(&exe).unwrap(), b"v0.10");
        assert!(!sibling(&exe, ".new").exists());
        // A place where the swap cannot happen (no folder): refused after its tries, nothing left.
        let nowhere = t.0.join("no-such-folder").join("bahth.exe");
        let e = replace_installed(&src, &nowhere, 2, Duration::ZERO).unwrap_err();
        assert!(e.contains("cannot copy"), "{e}");
        // The old program file missing: the swap fails, tried again, refused.
        let lonely = t.0.join("other.exe");
        let e = replace_installed(&src, &lonely, 3, Duration::ZERO).unwrap_err();
        assert!(e.contains("after 3 tries"), "{e}");
        assert!(!lonely.exists() && !sibling(&lonely, ".new").exists());
    }

    #[test]
    fn the_switch_is_passed_on_and_told() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let a = args(&["bahth.exe", "--updated", "--replaced=0.10.0", "--remove-download=C:\\Users\\Sara\\Downloads\\bahth (1).exe"]);
        assert_eq!(arg_value(a.clone(), REPLACED_ARG).as_deref(), Some("0.10.0"));
        assert_eq!(arg_value(a, REMOVE_DOWNLOAD_ARG).as_deref(), Some("C:\\Users\\Sara\\Downloads\\bahth (1).exe"));
        assert_eq!(arg_value(args(&["bahth.exe", "--replaced", "--replaced="]), REPLACED_ARG), None);
        assert_eq!(arg_value(args(&["bahth.exe", "--replacedx=1"]), REPLACED_ARG), None);
        // (Tests run in Arabic.)
        assert_eq!(replaced_notice("0.10.0", "1.0.0"), "تم استبدال النسخة القديمة (0.10.0) بالنسخة 1.0.0.");
    }

    #[test]
    fn only_the_downloaded_copy_of_this_version_is_deleted() {
        let transient = |p: &str| p.to_lowercase().contains("\\downloads\\");
        let v = |ver: &'static str| move |_: &str| Some(ver.to_string());
        let pick = |named: Option<&str>, version: &dyn Fn(&str) -> Option<String>| download_to_remove(named, MOVED, transient, version, "1.0.0");
        assert_eq!(pick(Some(DL), &v("1.0.0")), Some(PathBuf::from(DL)));
        assert_eq!(pick(Some(DL), &v("1.0.0.0")), Some(PathBuf::from(DL)));
        assert_eq!(pick(None, &v("1.0.0")), None);
        assert_eq!(pick(Some(DL), &v("0.10.0")), None, "another version");
        assert_eq!(pick(Some(DL), &|_| None), None, "not the program");
        assert_eq!(pick(Some("C:\\Users\\Sara\\Downloads\\notes.txt"), &v("1.0.0")), None);
        assert_eq!(pick(Some("D:\\Tools\\bahth.exe"), &v("1.0.0")), None, "a folder of the user's own");
        assert_eq!(download_to_remove(Some(DL), DL, transient, v("1.0.0"), "1.0.0"), None, "never its own file");
    }

    #[test]
    fn the_running_window_is_looked_for_in_both_languages() {
        assert_eq!(window_titles("InstaFind", "بحث سريع"), ["InstaFind", "بحث سريع"]);
    }

    #[test]
    fn only_this_program_is_replaced() {
        let none: [String; 0] = [];
        assert!(is_this_program(true, None, &none));
        assert!(is_this_program(false, Some("C:\\Users\\a\\Downloads\\BAHTH-x86.exe"), &none));
        assert!(is_this_program(false, Some("C:/tools/bahth.exe.old"), &none));
        assert!(is_this_program(false, Some("C:\\x\\renamed.exe"), &["InstaFind".to_string()]));
        assert!(is_this_program(false, Some("C:\\x\\renamed.exe"), &[" bahth.exe ".to_string()]));
        assert!(!is_this_program(false, Some("C:\\x\\other.exe"), &["Other app".to_string()]));
        assert!(!is_this_program(false, Some("C:\\bahthdir\\other.exe"), &none));
        assert!(!is_this_program(false, None, &none));
        assert!(!is_this_program(false, Some("C:\\x\\notinstafind.exe"), &["InstaFind helper".to_string()]));
    }

    #[test]
    fn another_cpu_build_is_never_replaced() {
        assert_eq!(replace_refused(true, Some(0x8664), Some(0x8664)), None);
        assert_eq!(replace_refused(true, None, Some(0x8664)), None);
        assert!(replace_refused(true, Some(0x8664), Some(0x014c)).is_some());
        assert!(replace_refused(true, Some(0xaa64), Some(0x8664)).is_some());
        assert!(replace_refused(false, Some(0x8664), Some(0x8664)).is_some());
    }
}
