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
    let started = Instant::now();
    let outcome = match wait(OLD_COPY_WAIT) {
        None => Previous::Gone,
        Some(true) => Previous::Exited,
        Some(false) if terminate() && wait(Duration::from_secs(2)) != Some(false) => Previous::Terminated,
        Some(false) => Previous::Stuck,
    };
    let ms = started.elapsed().as_millis();
    let line = match outcome {
        Previous::Gone => format!("handover: previous copy (pid {pid}) had already ended"),
        Previous::Exited => format!("handover: previous copy (pid {pid}) ended after {ms} ms"),
        Previous::Terminated => format!("handover: previous copy (pid {pid}) still running after {} s; ended it", OLD_COPY_WAIT.as_secs()),
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

    #[test]
    fn the_running_window_is_looked_for_in_both_languages() {
        assert_eq!(window_titles("InstaFind", "بحث سريع"), ["InstaFind", "بحث سريع"]);
    }
}
