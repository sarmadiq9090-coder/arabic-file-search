//! System commands (shut down, restart, lock…), always confirmed by the UI before running.

use std::sync::OnceLock;

use super::windows::{exe_stem, program_match};
use crate::i18n::Lang;
use super::{Action, Context, Item, Kind, SystemCommand};

/// Words for each command, as typed (normalised before matching).
const COMMANDS: &[(SystemCommand, &[&str])] = &[
    (
        SystemCommand::Shutdown,
        &[
            "إيقاف التشغيل", "ايقاف تشغيل", "إيقاف تشغيل الجهاز", "إطفاء", "اطفاء الجهاز", "إطفاء الكمبيوتر", "اغلاق الجهاز",
            "إغلاق الكمبيوتر", "إغلاق الحاسوب", "طفي الجهاز", "shutdown", "shut down", "power off", "turn off",
        ],
    ),
    (SystemCommand::Restart, &["إعادة التشغيل", "إعادة تشغيل", "اعادة تشغيل الجهاز", "ريستارت", "restart", "reboot"]),
    (SystemCommand::Sleep, &["سكون", "وضع السكون", "نوم", "sleep", "suspend"]),
    (SystemCommand::Lock, &["قفل", "قفل الجهاز", "قفل الشاشة", "lock", "lock screen", "lock computer"]),
    (
        SystemCommand::SignOut,
        &["تسجيل الخروج", "تسجيل خروج", "sign out", "signout", "log off", "logoff", "log out", "logout"],
    ),
    (
        SystemCommand::EmptyRecycleBin,
        &[
            "إفراغ سلة المحذوفات", "تفريغ سلة المحذوفات", "افراغ السلة", "سلة المحذوفات", "empty recycle bin",
            "recycle bin", "empty trash", "empty bin",
        ],
    ),
];

/// Verbs of "end <program>".
const END_VERBS: &[&str] = &["إنهاء", "انهاء", "اغلاق", "إغلاق", "اقفال", "kill", "end", "close", "قتل"];

/// Words after "اغلاق" that mean the computer, not a program.
const COMPUTER_WORDS: &[&str] = &["الجهاز", "الكمبيوتر", "الحاسوب", "الحاسب", "computer", "pc"];

/// Processes Windows needs; ending them would crash or log off the session.
/// ApplicationFrameHost draws the frame of every Microsoft Store app: ending
/// it closes all of them at once.
pub const CRITICAL: &[&str] = &[
    "csrss", "wininit", "winlogon", "lsass", "smss", "services", "system", "registry", "svchost", "dwm", "fontdrvhost",
    "applicationframehost",
];

/// The host of Store apps' frames ([`CRITICAL`]). A Store app's window
/// belongs to it when the app's own process could not be found (a minimized
/// Store app has no content window inside its frame).
const FRAME_HOST: &str = "applicationframehost";

/// Why `cmd` must not run, as the message shown instead of the question:
/// "end" on a Store app whose own process is unknown would end the host of
/// every Store app's window.
pub fn refusal(cmd: &SystemCommand, english: bool) -> Option<String> {
    let SystemCommand::EndProcess { name, .. } = cmd else { return None };
    if exe_stem(name) != FRAME_HOST {
        return None;
    }
    Some(
        Lang::from_english(english)
            .pick(
                "هذا تطبيق من متجر Microsoft لا يمكن معرفة برنامجه وهو مصغّر، وإنهاء إطاره يغلق كل تطبيقات المتجر معاً. أعد نافذته إلى الشاشة ثم جرّب مرة أخرى، أو أغلقه من نافذته.",
                "This is a Microsoft Store app whose own program can't be found while its window is minimized, and ending its frame would close every Store app. Restore its window and try again, or close it from its own window.",
            )
            .to_string(),
    )
}

/// [`COMMANDS`] with the words normalised once.
fn command_words() -> &'static [(SystemCommand, Vec<String>)] {
    static W: OnceLock<Vec<(SystemCommand, Vec<String>)>> = OnceLock::new();
    W.get_or_init(|| COMMANDS.iter().map(|(c, words)| (c.clone(), words.iter().map(|w| norm(w)).collect())).collect())
}

fn norm(s: &str) -> String {
    crate::normalize::normalize(s).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Restricted Damerau–Levenshtein distance between two short strings.
fn edit_distance(a: &[char], b: &[char]) -> usize {
    let (n, m) = (a.len(), b.len());
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = v;
        }
    }
    d[n][m]
}

/// How well the normalised query names `word`: 3 exact, 2 a prefix of at
/// least three letters, 1 a prefix with one typo (five letters or more), 0 no.
fn word_match(q: &str, word: &str) -> u32 {
    if q == word {
        return 3;
    }
    let qc: Vec<char> = q.chars().collect();
    let letters = qc.iter().filter(|c| c.is_alphabetic()).count();
    if letters >= 3 && word.starts_with(q) {
        return 2;
    }
    // Typos: not in the first letter, which keeps this cheap on every keystroke.
    if letters >= 5 && word.chars().next() == qc.first().copied() {
        let wc: Vec<char> = word.chars().collect();
        // Compare with the word's prefix of about the same length.
        for len in [qc.len().saturating_sub(1), qc.len(), qc.len() + 1] {
            if len >= 4 && len <= wc.len() && edit_distance(&qc, &wc[..len]) <= 1 {
                return 1;
            }
        }
    }
    0
}

fn title(cmd: &SystemCommand, english: bool) -> String {
    let lang = Lang::from_english(english);
    match cmd {
        SystemCommand::Shutdown => lang.pick("إيقاف تشغيل الجهاز", "Shut down").into(),
        SystemCommand::Restart => lang.pick("إعادة تشغيل الجهاز", "Restart").into(),
        SystemCommand::Sleep => lang.pick("السكون", "Sleep").into(),
        SystemCommand::Lock => lang.pick("قفل الجهاز", "Lock").into(),
        SystemCommand::SignOut => lang.pick("تسجيل الخروج", "Sign out").into(),
        SystemCommand::EmptyRecycleBin => lang.pick("إفراغ سلة المحذوفات", "Empty Recycle Bin").into(),
        SystemCommand::EndProcess { name, .. } => end_title(name, english),
    }
}

/// "End <name>": the title of ending a program.
fn end_title(name: &str, english: bool) -> String {
    trf!(in Lang::from_english(english); "إنهاء {name}", "End {name}")
}

/// The question the UI shows before running `cmd`.
pub fn confirm_text(cmd: &SystemCommand, english: bool) -> String {
    let lang = Lang::from_english(english);
    match cmd {
        SystemCommand::Shutdown => lang.pick("هل تريد إيقاف تشغيل الجهاز الآن؟", "Shut down the computer now?").into(),
        SystemCommand::Restart => lang.pick("هل تريد إعادة تشغيل الجهاز الآن؟", "Restart the computer now?").into(),
        SystemCommand::Sleep => lang.pick("هل تريد وضع الجهاز في وضع السكون الآن؟", "Put the computer to sleep now?").into(),
        SystemCommand::Lock => lang.pick("هل تريد قفل الجهاز الآن؟", "Lock the computer now?").into(),
        SystemCommand::SignOut => {
            lang.pick("هل تريد تسجيل الخروج الآن؟ ستُغلق البرامج المفتوحة.", "Sign out now? Open programs will be closed.").into()
        }
        SystemCommand::EmptyRecycleBin => {
            lang.pick("هل تريد إفراغ سلة المحذوفات؟ لا يمكن التراجع عن ذلك.", "Empty the Recycle Bin? This cannot be undone.").into()
        }
        SystemCommand::EndProcess { name, .. } => {
            if exe_stem(name) == "explorer" {
                trf!(
                    in lang;
                    "هل تريد إنهاء {name}؟ سيختفي شريط المهام وسطح المكتب وتُغلق نوافذ المجلدات حتى يُعاد تشغيله.",
                    "End {name}? The taskbar, desktop and folder windows disappear until it is restarted."
                )
            } else {
                trf!(in lang; "هل تريد إنهاء {name}؟ قد يضيع العمل غير المحفوظ.", "End {name}? Unsaved work may be lost.")
            }
        }
    }
}

/// Whether `cmd` runs on a background thread: emptying a large Recycle Bin
/// takes seconds to minutes, and the window must keep answering meanwhile.
/// The others return at once (or end the session).
pub fn runs_in_background(cmd: &SystemCommand) -> bool {
    matches!(cmd, SystemCommand::EmptyRecycleBin)
}

/// The message when `cmd` failed.
pub fn failed_text(cmd: &SystemCommand, english: bool) -> String {
    trf!(in Lang::from_english(english); "تعذر تنفيذ: {}.", "Could not do this: {}.", title(cmd, english))
}

fn item(cmd: SystemCommand, subtitle: String, score: u32, english: bool) -> Item {
    Item { kind: Kind::Command, title: title(&cmd, english), subtitle, action: Action::System(cmd), score, icon_path: None, strong: false }
}

/// "end chrome" → `chrome`, if the query starts with an end verb.
fn end_target(q: &str) -> Option<&str> {
    let (verb, rest) = q.split_once(' ')?;
    let rest = rest.trim();
    if rest.chars().count() < 2 || !END_VERBS.iter().any(|v| norm(v) == verb) {
        return None;
    }
    if COMPUTER_WORDS.iter().any(|w| norm(w) == rest) {
        return None;
    }
    Some(rest)
}

/// Commands whose words match `query`, including "end <program>" for open windows.
pub fn commands(query: &str, ctx: &Context) -> Vec<Item> {
    let q = norm(query);
    if q.is_empty() || q.chars().count() > 60 {
        return Vec::new();
    }
    let en = ctx.english;
    let mut out = Vec::new();
    let sub = Lang::from_english(en).pick("أمر نظام · يُطلب التأكيد قبل التنفيذ", "System command · asks to confirm");
    for (cmd, words) in command_words() {
        let best = words.iter().map(|w| word_match(&q, w)).max().unwrap_or(0);
        if best > 0 {
            out.push(item(cmd.clone(), sub.to_string(), 300 + 100 * best, en));
        }
    }
    if let Some(target) = end_target(&q) {
        let own = std::process::id();
        let mut seen: Vec<u32> = Vec::new();
        for w in ctx.windows {
            if w.pid == own || w.pid == 0 || seen.contains(&w.pid) {
                continue;
            }
            let stem = exe_stem(&w.exe);
            // A Store app whose own process is unknown: offered by its title,
            // and refused with an explanation (`refusal`) when chosen.
            let frame = stem == FRAME_HOST;
            if CRITICAL.contains(&stem.as_str()) && !frame {
                continue;
            }
            let p = if frame { 0 } else { program_match(target, &w.exe) };
            let t = norm(&w.title).contains(target);
            if p == 0 && !t {
                continue;
            }
            seen.push(w.pid);
            let name = if w.exe.is_empty() { w.title.clone() } else { w.exe.clone() };
            let cmd = SystemCommand::EndProcess { pid: w.pid, name };
            let score = if p >= 2 { 600 } else { 450 };
            let mut it = item(cmd, w.title.trim().to_string(), score, en);
            if frame {
                it.title = end_title(w.title.trim(), en);
            }
            out.push(it);
        }
    }
    out
}

#[cfg(windows)]
mod win {
    use super::{CRITICAL, SystemCommand};
    use crate::launcher::windows::{exe_stem, process_exe};
    use winapi::shared::minwindef::{DWORD, FALSE};
    use winapi::shared::winerror::{E_UNEXPECTED, S_OK};
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::powerbase::GetPwrCapabilities;
    use winapi::um::powrprof::SetSuspendState;
    use winapi::um::processthreadsapi::{GetCurrentProcess, OpenProcess, OpenProcessToken, TerminateProcess};
    use winapi::um::reason::{SHTDN_REASON_FLAG_PLANNED, SHTDN_REASON_MAJOR_OTHER, SHTDN_REASON_MINOR_OTHER};
    use winapi::um::securitybaseapi::AdjustTokenPrivileges;
    use winapi::um::shellapi::{SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND, SHEmptyRecycleBinW};
    use winapi::um::winbase::LookupPrivilegeValueW;
    use winapi::um::winnt::{
        HANDLE, LUID_AND_ATTRIBUTES, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, SE_PRIVILEGE_ENABLED,
        SE_SHUTDOWN_NAME, SYSTEM_POWER_CAPABILITIES, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
    };
    use winapi::um::winuser::{
        EWX_LOGOFF, EWX_POWEROFF, EWX_REBOOT, EWX_SHUTDOWN, ExitWindowsEx, HWND_BROADCAST, LockWorkStation, PostMessageW,
        SC_MONITORPOWER, WM_SYSCOMMAND,
    };

    fn last_error() -> String {
        std::io::Error::last_os_error().to_string()
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Turns on SeShutdownPrivilege for this process (needed by ExitWindowsEx
    /// for shutdown and restart, and by SetSuspendState).
    fn enable_shutdown_privilege() -> Result<(), String> {
        // SAFETY: the token handle is closed before returning; all pointers
        // point to locals that outlive the calls.
        unsafe {
            let mut token: HANDLE = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token) == 0 {
                return Err(last_error());
            }
            let mut tp: TOKEN_PRIVILEGES = std::mem::zeroed();
            tp.PrivilegeCount = 1;
            let name = wide(SE_SHUTDOWN_NAME);
            if LookupPrivilegeValueW(std::ptr::null(), name.as_ptr(), &mut tp.Privileges[0].Luid) == 0 {
                let e = last_error();
                CloseHandle(token);
                return Err(e);
            }
            tp.Privileges[0] = LUID_AND_ATTRIBUTES { Luid: tp.Privileges[0].Luid, Attributes: SE_PRIVILEGE_ENABLED };
            let ok = AdjustTokenPrivileges(token, FALSE, &mut tp, 0, std::ptr::null_mut(), std::ptr::null_mut());
            // AdjustTokenPrivileges "succeeds" without the privilege; check the error too.
            let err = std::io::Error::last_os_error();
            CloseHandle(token);
            if ok == 0 || err.raw_os_error() == Some(1300) {
                return Err(err.to_string());
            }
            Ok(())
        }
    }

    const REASON: DWORD = SHTDN_REASON_MAJOR_OTHER | SHTDN_REASON_MINOR_OTHER | SHTDN_REASON_FLAG_PLANNED;

    fn exit_windows(flags: u32) -> Result<(), String> {
        // SAFETY: no pointers involved.
        if unsafe { ExitWindowsEx(flags, REASON) } == 0 { Err(last_error()) } else { Ok(()) }
    }

    fn sleep() -> Result<(), String> {
        // SAFETY: the capabilities struct is a plain C struct written by the call.
        let caps = unsafe {
            let mut caps: SYSTEM_POWER_CAPABILITIES = std::mem::zeroed();
            if GetPwrCapabilities(&mut caps) == 0 {
                return Err(last_error());
            }
            caps
        };
        if caps.AoAc != 0 {
            // Modern Standby (S0 low-power idle): SetSuspendState would hibernate
            // or fail; turning the display off is how Windows enters standby.
            // SAFETY: posting a broadcast message has no memory preconditions.
            let ok = unsafe { PostMessageW(HWND_BROADCAST, WM_SYSCOMMAND, SC_MONITORPOWER, 2) };
            return if ok == 0 { Err(last_error()) } else { Ok(()) };
        }
        if caps.SystemS1 == 0 && caps.SystemS2 == 0 && caps.SystemS3 == 0 {
            return Err("this computer does not support sleep".into());
        }
        let _ = enable_shutdown_privilege();
        // SAFETY: plain call. bHibernate = FALSE (sleep, never hibernate),
        // bForce = FALSE, bWakeupEventsDisabled = FALSE.
        if unsafe { SetSuspendState(0, 0, 0) } == 0 { Err(last_error()) } else { Ok(()) }
    }

    fn end_process(pid: u32, name: &str) -> Result<(), String> {
        if pid == std::process::id() || pid == 0 || pid == 4 {
            return Err("refusing to end this process".into());
        }
        // The pid may have been reused since the list was taken: check the exe again.
        let actual = process_exe(pid).ok_or_else(|| "the program is no longer running".to_string())?;
        let stem = exe_stem(&actual);
        if CRITICAL.contains(&stem.as_str()) {
            return Err(format!("{actual} is a system process"));
        }
        if !name.is_empty() && !actual.eq_ignore_ascii_case(name) {
            return Err("the program is no longer running".into());
        }
        // SAFETY: the handle is checked and closed.
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
            if h.is_null() {
                return Err(last_error());
            }
            let ok = TerminateProcess(h, 1);
            let e = last_error();
            CloseHandle(h);
            if ok == 0 { Err(e) } else { Ok(()) }
        }
    }

    pub fn run(cmd: &SystemCommand) -> Result<(), String> {
        match cmd {
            SystemCommand::Shutdown => {
                enable_shutdown_privilege()?;
                exit_windows(EWX_SHUTDOWN | EWX_POWEROFF)
            }
            SystemCommand::Restart => {
                enable_shutdown_privilege()?;
                exit_windows(EWX_REBOOT)
            }
            SystemCommand::Sleep => sleep(),
            SystemCommand::Lock => {
                // SAFETY: no arguments.
                if unsafe { LockWorkStation() } == 0 { Err(last_error()) } else { Ok(()) }
            }
            SystemCommand::SignOut => exit_windows(EWX_LOGOFF),
            SystemCommand::EmptyRecycleBin => {
                // SAFETY: null window and null root (all drives) are documented values.
                let hr = unsafe {
                    SHEmptyRecycleBinW(std::ptr::null_mut(), std::ptr::null(), SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND)
                };
                // E_UNEXPECTED: the bin was already empty.
                if hr == S_OK || hr == E_UNEXPECTED { Ok(()) } else { Err(format!("SHEmptyRecycleBinW failed: 0x{:08X}", hr as u32)) }
            }
            SystemCommand::EndProcess { pid, name } => end_process(*pid, name),
        }
    }
}

/// Runs a system command. The UI must have asked [`confirm_text`] first.
#[cfg(windows)]
pub fn run(cmd: &SystemCommand) -> Result<(), String> {
    win::run(cmd)
}

#[cfg(test)]
mod tests {
    use super::super::windows::OpenWindow;
    use super::*;

    #[test]
    fn texts_follow_the_language() {
        let end = |name: &str| SystemCommand::EndProcess { pid: 5, name: name.into() };
        assert_eq!(confirm_text(&SystemCommand::Shutdown, true), "Shut down the computer now?");
        assert_eq!(confirm_text(&SystemCommand::Lock, false), "هل تريد قفل الجهاز الآن؟");
        assert_eq!(
            confirm_text(&end("Explorer.exe"), true),
            "End Explorer.exe? The taskbar, desktop and folder windows disappear until it is restarted."
        );
        assert_eq!(confirm_text(&end("a.exe"), false), "هل تريد إنهاء a.exe؟ قد يضيع العمل غير المحفوظ.");
        assert_eq!(failed_text(&SystemCommand::Restart, true), "Could not do this: Restart.");
        assert_eq!(failed_text(&end("a.exe"), false), "تعذر تنفيذ: إنهاء a.exe.");
        assert_eq!(title(&SystemCommand::EmptyRecycleBin, true), "Empty Recycle Bin");
    }

    fn ctx_with<'a>(english: bool, windows: &'a [OpenWindow]) -> Context<'a> {
        static NO_LAUNCHES: fn(&str) -> u32 = |_| 0;
        Context { english, windows_build: 0, windows, launches: &NO_LAUNCHES, picked: &NO_LAUNCHES, today: (2026, 11, 1) }
    }

    fn cmds(q: &str) -> Vec<SystemCommand> {
        commands(q, &ctx_with(false, &[]))
            .into_iter()
            .map(|i| match i.action {
                Action::System(c) => c,
                other => panic!("{other:?}"),
            })
            .collect()
    }

    use SystemCommand::*;

    #[test]
    fn arabic_and_english_words() {
        assert_eq!(cmds("إيقاف التشغيل"), vec![Shutdown]);
        assert_eq!(cmds("ايقاف"), vec![Shutdown]);
        assert_eq!(cmds("اطفاء"), vec![Shutdown]);
        assert_eq!(cmds("اغلاق الجهاز"), vec![Shutdown]);
        assert_eq!(cmds("shutdown"), vec![Shutdown]);
        assert_eq!(cmds("Shut"), vec![Shutdown]);
        assert_eq!(cmds("إعادة التشغيل"), vec![Restart]);
        assert_eq!(cmds("اعاده"), vec![Restart]);
        assert_eq!(cmds("ريستارت"), vec![Restart]);
        assert_eq!(cmds("restart"), vec![Restart]);
        assert_eq!(cmds("reboot"), vec![Restart]);
        assert_eq!(cmds("سكون"), vec![Sleep]);
        assert_eq!(cmds("نوم"), vec![Sleep]);
        assert_eq!(cmds("sleep"), vec![Sleep]);
        assert_eq!(cmds("قفل"), vec![Lock]);
        assert_eq!(cmds("lock"), vec![Lock]);
        assert_eq!(cmds("تسجيل الخروج"), vec![SignOut]);
        assert_eq!(cmds("sign out"), vec![SignOut]);
        assert_eq!(cmds("log off"), vec![SignOut]);
        assert_eq!(cmds("إفراغ سلة المحذوفات"), vec![EmptyRecycleBin]);
        assert_eq!(cmds("سلة المحذوفات"), vec![EmptyRecycleBin]);
        assert_eq!(cmds("empty recycle bin"), vec![EmptyRecycleBin]);
        assert_eq!(cmds("recycle"), vec![EmptyRecycleBin]);
    }

    #[test]
    fn typos_and_prefixes() {
        assert_eq!(cmds("shutdwon"), vec![Shutdown]);
        assert_eq!(cmds("restrat"), vec![Restart]);
        assert_eq!(cmds("ريستارتت"), vec![Restart]);
        assert_eq!(cmds("اعادة التشغيل"), vec![Restart]);
        assert_eq!(cmds("res"), vec![Restart]);
        assert_eq!(cmds("slee"), vec![Sleep]);
        assert_eq!(cmds("SLEEP"), vec![Sleep]);
        assert_eq!(cmds("تسجيل"), vec![SignOut]);
        assert_eq!(cmds("  lock  "), vec![Lock]);
    }

    #[test]
    fn not_commands() {
        for q in [
            "", " ", "s", "sh", "re", "تق", "تقرير", "shutdown_log.txt", "sleeping beauty.mp4", "lockdown", "تقرير السكون",
            "report", "😀", "\u{200F}", "lock.png", "نومينا", "الجهاز", "restaurant", "صور العيد",
        ] {
            assert!(cmds(q).is_empty(), "{q:?}: {:?}", cmds(q));
        }
    }

    #[test]
    fn end_process() {
        let wins = vec![
            OpenWindow { hwnd: 1, title: "YouTube - Google Chrome".into(), exe: "chrome.exe".into(), pid: 50 },
            OpenWindow { hwnd: 2, title: "Gmail - Google Chrome".into(), exe: "chrome.exe".into(), pid: 50 },
            OpenWindow { hwnd: 3, title: "تقرير.docx - Word".into(), exe: "WINWORD.EXE".into(), pid: 60 },
            OpenWindow { hwnd: 4, title: "Our own window".into(), exe: "bahth.exe".into(), pid: std::process::id() },
            OpenWindow { hwnd: 5, title: "Windows Logon".into(), exe: "winlogon.exe".into(), pid: 70 },
            OpenWindow { hwnd: 6, title: "التنزيلات".into(), exe: "explorer.exe".into(), pid: 80 },
        ];
        let get = |q: &str| -> Vec<SystemCommand> {
            commands(q, &ctx_with(false, &wins))
                .into_iter()
                .filter_map(|i| match i.action {
                    Action::System(c @ EndProcess { .. }) => Some(c),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(get("إنهاء كروم"), vec![EndProcess { pid: 50, name: "chrome.exe".into() }]);
        assert_eq!(get("kill chrome"), vec![EndProcess { pid: 50, name: "chrome.exe".into() }]);
        assert_eq!(get("end chr"), vec![EndProcess { pid: 50, name: "chrome.exe".into() }]);
        assert_eq!(get("اغلاق وورد"), vec![EndProcess { pid: 60, name: "WINWORD.EXE".into() }]);
        assert_eq!(get("انهاء تقرير"), vec![EndProcess { pid: 60, name: "WINWORD.EXE".into() }]);
        assert_eq!(get("kill youtube"), vec![EndProcess { pid: 50, name: "chrome.exe".into() }]);
        assert_eq!(get("kill bahth"), vec![]); // ourselves
        assert_eq!(get("kill winlogon"), vec![]); // critical
        assert_eq!(get("إنهاء الملفات"), vec![EndProcess { pid: 80, name: "explorer.exe".into() }]);
        assert_eq!(get("kill"), vec![]);
        assert_eq!(get("kill x"), vec![]);
        assert_eq!(get("kill nothing"), vec![]);
        assert_eq!(get("اغلاق الجهاز"), vec![]);
        // "اغلاق الجهاز" is still the shutdown command.
        assert!(commands("اغلاق الجهاز", &ctx_with(false, &wins)).iter().any(|i| i.action == Action::System(Shutdown)));
        let items = commands("kill chrome", &ctx_with(true, &wins));
        assert_eq!(items[0].title, "End chrome.exe");
        assert_eq!(items[0].kind, Kind::Command);
    }

    /// A minimized Store app: its frame belongs to ApplicationFrameHost and
    /// its own process is unknown. It is offered by its title but refused,
    /// never ending the host of every Store app; a restored one (its own exe
    /// found) is ended as usual.
    #[test]
    fn store_app_frames_are_never_ended() {
        let wins = vec![
            OpenWindow { hwnd: 1, title: "Calculator".into(), exe: "ApplicationFrameHost.exe".into(), pid: 90 },
            OpenWindow { hwnd: 2, title: "Photos".into(), exe: "ApplicationFrameHost.exe".into(), pid: 90 },
            OpenWindow { hwnd: 3, title: "Microsoft Store".into(), exe: "WinStore.App.exe".into(), pid: 91 },
        ];
        let items = |q: &str, en: bool| commands(q, &ctx_with(en, &wins));
        let calc = items("end calculator", true);
        assert_eq!(calc.len(), 1);
        assert_eq!(calc[0].title, "End Calculator");
        let Action::System(cmd) = &calc[0].action else { panic!("{:?}", calc[0].action) };
        assert!(refusal(cmd, true).is_some_and(|m| m.contains("Store app")));
        assert!(refusal(cmd, false).is_some_and(|m| m.contains("المتجر")));
        assert_eq!(items("انهاء calculator", false)[0].title, "إنهاء Calculator");
        // Not by the host's own name.
        assert!(items("kill applicationframehost", true).is_empty());
        assert!(items("end frame", true).is_empty());
        // The app's own process: ended as usual, no refusal.
        let store = items("end store", true);
        assert_eq!(store.len(), 1);
        let Action::System(cmd) = &store[0].action else { panic!() };
        assert_eq!(*cmd, EndProcess { pid: 91, name: "WinStore.App.exe".into() });
        assert!(refusal(cmd, true).is_none());
        assert!(refusal(&Shutdown, true).is_none());
        assert!(refusal(&EndProcess { pid: 5, name: "chrome.exe".into() }, false).is_none());
        assert!(CRITICAL.contains(&"applicationframehost"));
    }

    #[test]
    fn titles_and_confirmations() {
        let i = commands("shutdown", &ctx_with(true, &[])).remove(0);
        assert_eq!(i.title, "Shut down");
        assert_eq!(i.kind, Kind::Command);
        assert_eq!(commands("اطفاء", &ctx_with(false, &[]))[0].title, "إيقاف تشغيل الجهاز");
        assert_eq!(confirm_text(&Shutdown, false), "هل تريد إيقاف تشغيل الجهاز الآن؟");
        assert_eq!(confirm_text(&Shutdown, true), "Shut down the computer now?");
        for c in [Shutdown, Restart, Sleep, Lock, SignOut, EmptyRecycleBin, EndProcess { pid: 1, name: "a.exe".into() }] {
            assert!(confirm_text(&c, false).ends_with('؟') || confirm_text(&c, false).ends_with('.'));
            assert!(!confirm_text(&c, true).is_empty());
            assert!(!title(&c, false).is_empty());
        }
        let ex = EndProcess { pid: 1, name: "explorer.exe".into() };
        assert!(confirm_text(&ex, true).contains("taskbar"));
        assert!(confirm_text(&ex, false).contains("شريط المهام"));
        assert!(commands("shutdown", &ctx_with(false, &[]))[0].score > commands("shut", &ctx_with(false, &[]))[0].score);
    }

    #[test]
    fn only_slow_commands_leave_the_ui_thread() {
        assert!(runs_in_background(&EmptyRecycleBin));
        for c in [Shutdown, Restart, Sleep, Lock, SignOut, EndProcess { pid: 1, name: "a.exe".into() }] {
            assert!(!runs_in_background(&c), "{c:?}");
        }
        assert_eq!(failed_text(&EmptyRecycleBin, true), "Could not do this: Empty Recycle Bin.");
        assert_eq!(failed_text(&EmptyRecycleBin, false), "تعذر تنفيذ: إفراغ سلة المحذوفات.");
    }

    #[test]
    fn distance() {
        let c = |s: &str| s.chars().collect::<Vec<_>>();
        assert_eq!(edit_distance(&c("abc"), &c("abc")), 0);
        assert_eq!(edit_distance(&c("abc"), &c("acb")), 1);
        assert_eq!(edit_distance(&c("abc"), &c("")), 3);
        assert_eq!(edit_distance(&c(""), &c("")), 0);
    }
}
