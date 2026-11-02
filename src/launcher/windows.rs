//! Open top-level windows, for "switch to" results.

/// One visible top-level window.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenWindow {
    pub hwnd: isize,
    pub title: String,
    /// Executable file name ("WINWORD.EXE").
    pub exe: String,
    pub pid: u32,
}

use super::{Action, Context, Item, Kind};

/// Other names of common programs, by lower-case exe stem.
const ALIASES: &[(&str, &[&str])] = &[
    ("winword", &["وورد", "ورد", "word", "microsoft word"]),
    ("excel", &["اكسل", "إكسل", "excel"]),
    ("powerpnt", &["باوربوينت", "بوربوينت", "باور بوينت", "powerpoint"]),
    ("outlook", &["أوتلوك", "اوت لوك", "outlook", "البريد"]),
    ("onenote", &["ون نوت", "onenote"]),
    ("msaccess", &["أكسس", "access"]),
    ("chrome", &["كروم", "جوجل كروم", "google chrome"]),
    ("msedge", &["ايدج", "إيدج", "edge", "مايكروسوفت ايدج"]),
    ("firefox", &["فايرفوكس", "فيرفوكس", "firefox"]),
    ("opera", &["أوبرا", "opera"]),
    ("brave", &["بريف", "brave"]),
    ("telegram", &["تيليجرام", "تليجرام", "تلجرام", "تلغرام", "تيليغرام", "telegram"]),
    ("whatsapp", &["واتساب", "واتس اب", "واتس", "whatsapp"]),
    ("explorer", &["الملفات", "مستكشف الملفات", "مستكشف", "file explorer", "files"]),
    ("notepad", &["المفكرة", "مفكرة", "notepad"]),
    ("code", &["vs code", "vscode", "فيجوال ستوديو كود"]),
    ("devenv", &["visual studio", "فيجوال ستوديو"]),
    ("teams", &["تيمز", "teams"]),
    ("ms-teams", &["تيمز", "teams"]),
    ("zoom", &["زوم", "zoom"]),
    ("spotify", &["سبوتيفاي", "spotify"]),
    ("discord", &["ديسكورد", "discord"]),
    ("acrord32", &["أكروبات", "ادوبي", "adobe reader", "acrobat"]),
    ("acrobat", &["أكروبات", "ادوبي", "acrobat"]),
    ("vlc", &["في ال سي", "vlc"]),
    ("photoshop", &["فوتوشوب", "photoshop"]),
    ("calculatorapp", &["الحاسبة", "حاسبة", "calculator"]),
    ("windowsterminal", &["الطرفية", "terminal"]),
    ("cmd", &["موجه الأوامر", "command prompt"]),
    ("taskmgr", &["مدير المهام", "task manager"]),
    ("mspaint", &["الرسام", "paint"]),
    ("bahth", &["بحث سريع", "instafind"]),
];

/// Lower-case exe name without ".exe".
pub(crate) fn exe_stem(exe: &str) -> String {
    let lower = exe.to_lowercase();
    match lower.strip_suffix(".exe") {
        Some(s) => s.to_string(),
        None => lower,
    }
}

/// The extra names of a program, by exe name ("WINWORD.EXE" → وورد, word…).
pub(crate) fn aliases_for(exe: &str) -> &'static [&'static str] {
    let stem = exe_stem(exe);
    // Store apps: "WhatsApp.Root.exe", "Telegram Desktop" and the like.
    let head = stem.split(['.', ' ', '_']).next().unwrap_or(&stem);
    ALIASES.iter().find(|(s, _)| *s == stem || *s == head).map_or(&[], |(_, a)| *a)
}

fn norm(s: &str) -> String {
    crate::normalize::normalize(s).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Splits into words at anything that is not a letter or digit.
fn words(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty())
}

/// How well the normalised query `q` names a program (exe or alias), 0 = not at all.
pub(crate) fn program_match(q: &str, exe: &str) -> u32 {
    let stem = norm(&exe_stem(exe));
    let mut best = 0;
    let mut consider = |name: &str| {
        let n = norm(name);
        let s = if n == q {
            3
        } else if n.starts_with(q) || words(&n).any(|w| w.starts_with(q)) {
            2
        } else {
            0
        };
        best = best.max(s);
    };
    consider(&stem);
    for a in aliases_for(exe) {
        consider(a);
    }
    best
}

/// How well `q` matches a window title: 2 if a title word starts with it (or
/// it is a phrase found in the title), 0 otherwise.
fn title_match(q: &str, title: &str) -> u32 {
    let t = norm(title);
    if q.contains(' ') {
        return if t.contains(q) { 2 } else { 0 };
    }
    if words(&t).any(|w| w.starts_with(q) || (w.starts_with("ال") && w["ال".len()..].starts_with(q))) { 2 } else { 0 }
}

/// "Switch to" results for open windows whose title or program matches `query`.
pub fn matches(query: &str, ctx: &Context) -> Vec<Item> {
    let q = norm(query);
    if q.chars().count() < 2 || q.len() > 300 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for w in ctx.windows {
        if w.title.trim().is_empty() {
            continue;
        }
        let p = program_match(&q, &w.exe);
        let t = title_match(&q, &w.title);
        let score = match (p, t) {
            (3, _) => 700,
            (2, 2) => 650,
            (2, _) => 600,
            (_, 2) => 500,
            _ => continue,
        };
        let title = if ctx.english { format!("Switch to: {}", w.title.trim()) } else { format!("التبديل إلى: {}", w.title.trim()) };
        out.push(Item { kind: Kind::Window, title, subtitle: w.exe.clone(), action: Action::FocusWindow(w.hwnd), score, icon_path: None });
    }
    out
}

#[cfg(windows)]
mod win {
    use super::OpenWindow;
    use std::collections::HashMap;
    use winapi::shared::minwindef::{BOOL, DWORD, FALSE, LPARAM, TRUE};
    use winapi::shared::windef::HWND;
    use winapi::shared::winerror::S_OK;
    use winapi::um::dwmapi::{DWMWA_CLOAKED, DwmGetWindowAttribute};
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::processthreadsapi::{GetCurrentProcessId, GetCurrentThreadId, OpenProcess};
    use winapi::um::winbase::QueryFullProcessImageNameW;
    use winapi::um::winnt::PROCESS_QUERY_LIMITED_INFORMATION;
    use winapi::um::winuser::{
        ASFW_ANY, AllowSetForegroundWindow, AttachThreadInput, BringWindowToTop, EnumChildWindows, EnumWindows, GW_OWNER,
        GWL_EXSTYLE, GetClassNameW, GetForegroundWindow, GetWindow, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW,
        GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, SW_RESTORE, SetForegroundWindow, ShowWindow,
        WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };

    /// The executable file name of a process ("WINWORD.EXE"), if it can be read.
    pub fn process_exe(pid: u32) -> Option<String> {
        // SAFETY: plain Win32 calls; the handle is closed before returning and
        // the buffer outlives the call that fills it.
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
            if h.is_null() {
                return None;
            }
            let mut buf = [0u16; 1024];
            let mut len: DWORD = buf.len() as DWORD;
            let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len);
            CloseHandle(h);
            if ok == 0 {
                return None;
            }
            let path = String::from_utf16_lossy(&buf[..(len as usize).min(buf.len())]);
            path.rsplit(['\\', '/']).next().map(str::to_string)
        }
    }

    fn class_name(hwnd: HWND) -> String {
        let mut buf = [0u16; 256];
        // SAFETY: the buffer length is passed and outlives the call.
        let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }

    fn window_text(hwnd: HWND) -> String {
        // SAFETY: the buffer is one longer than the reported length, as the API needs.
        unsafe {
            let len = GetWindowTextLengthW(hwnd);
            if len <= 0 {
                return String::new();
            }
            let mut buf = vec![0u16; len as usize + 1];
            let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
            String::from_utf16_lossy(&buf[..n.max(0) as usize])
        }
    }

    fn pid_of(hwnd: HWND) -> u32 {
        let mut pid: DWORD = 0;
        // SAFETY: `pid` is a valid out pointer.
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        pid
    }

    fn is_cloaked(hwnd: HWND) -> bool {
        let mut cloaked: DWORD = 0;
        // SAFETY: the out buffer is a DWORD and its size is passed.
        let hr = unsafe {
            DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut DWORD as *mut _, std::mem::size_of::<DWORD>() as DWORD)
        };
        hr == S_OK && cloaked != 0
    }

    struct Scan {
        own_pid: u32,
        out: Vec<OpenWindow>,
        exes: HashMap<u32, Option<String>>,
    }

    /// A UWP app's frame belongs to ApplicationFrameHost.exe; its content
    /// window (a child) belongs to the real app.
    unsafe extern "system" fn child_pid(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` is the `(u32, u32)` passed by `real_pid`.
        let state = unsafe { &mut *(lparam as *mut (u32, u32)) };
        let pid = pid_of(hwnd);
        if pid != state.0 && pid != 0 {
            state.1 = pid;
            return FALSE;
        }
        TRUE
    }

    fn real_pid(hwnd: HWND, frame_pid: u32) -> u32 {
        let mut state: (u32, u32) = (frame_pid, frame_pid);
        // SAFETY: the callback only runs during this call, while `state` lives.
        unsafe { EnumChildWindows(hwnd, Some(child_pid), &mut state as *mut (u32, u32) as LPARAM) };
        state.1
    }

    unsafe extern "system" fn each_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` is the `Scan` passed by `list`, alive during EnumWindows.
        let scan = unsafe { &mut *(lparam as *mut Scan) };
        // SAFETY: plain queries on a window handle given by EnumWindows.
        let (visible, owner, ex) = unsafe {
            (IsWindowVisible(hwnd) != 0, GetWindow(hwnd, GW_OWNER), GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32)
        };
        if !visible {
            return TRUE;
        }
        let app = ex & WS_EX_APPWINDOW != 0;
        if (!owner.is_null() && !app) || (ex & WS_EX_TOOLWINDOW != 0 && !app) || (ex & WS_EX_NOACTIVATE != 0 && !app) {
            return TRUE;
        }
        let title = window_text(hwnd);
        if title.trim().is_empty() || is_cloaked(hwnd) {
            return TRUE;
        }
        let class = class_name(hwnd);
        if matches!(class.as_str(), "Progman" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "WorkerW") {
            return TRUE;
        }
        let mut pid = pid_of(hwnd);
        if pid == scan.own_pid || pid == 0 {
            return TRUE;
        }
        let mut exe = scan.exes.entry(pid).or_insert_with(|| process_exe(pid)).clone().unwrap_or_default();
        if exe.eq_ignore_ascii_case("ApplicationFrameHost.exe") {
            let inner = real_pid(hwnd, pid);
            if inner != pid {
                pid = inner;
                exe = scan.exes.entry(pid).or_insert_with(|| process_exe(pid)).clone().unwrap_or(exe);
            }
        }
        scan.out.push(OpenWindow { hwnd: hwnd as isize, title, exe, pid });
        TRUE
    }

    pub fn list() -> Vec<OpenWindow> {
        // SAFETY: GetCurrentProcessId has no preconditions.
        let own_pid = unsafe { GetCurrentProcessId() };
        let mut scan = Scan { own_pid, out: Vec::new(), exes: HashMap::new() };
        // SAFETY: the callback only runs during this call, while `scan` lives.
        unsafe { EnumWindows(Some(each_window), &mut scan as *mut Scan as LPARAM) };
        scan.out
    }

    pub fn focus(hwnd: isize) -> bool {
        let hwnd = hwnd as HWND;
        // SAFETY: Win32 calls on a window handle that may have closed meanwhile,
        // which they all tolerate; the thread input is detached before returning.
        unsafe {
            if IsWindow(hwnd) == 0 {
                return false;
            }
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            AllowSetForegroundWindow(ASFW_ANY);
            let fg = GetForegroundWindow();
            let me = GetCurrentThreadId();
            let fg_thread = if fg.is_null() { 0 } else { GetWindowThreadProcessId(fg, std::ptr::null_mut()) };
            let attached = fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, TRUE) != 0;
            let ok = SetForegroundWindow(hwnd) != 0;
            BringWindowToTop(hwnd);
            if attached {
                AttachThreadInput(me, fg_thread, FALSE);
            }
            ok || GetForegroundWindow() == hwnd
        }
    }
}

/// The visible top-level windows of other programs, front to back.
#[cfg(windows)]
pub fn list() -> Vec<OpenWindow> {
    win::list()
}

/// Brings a window to the front (restoring it if minimised). False if it is gone
/// or Windows refused.
#[cfg(windows)]
pub fn focus(hwnd: isize) -> bool {
    win::focus(hwnd)
}

#[cfg(windows)]
pub(crate) use win::process_exe;

#[cfg(test)]
mod tests {
    use super::*;

    fn w(hwnd: isize, title: &str, exe: &str, pid: u32) -> OpenWindow {
        OpenWindow { hwnd, title: title.into(), exe: exe.into(), pid }
    }

    fn sample() -> Vec<OpenWindow> {
        vec![
            w(1, "تقرير المبيعات.docx - Word", "WINWORD.EXE", 10),
            w(2, "الميزانية.xlsx - Excel", "EXCEL.EXE", 11),
            w(3, "YouTube - Google Chrome", "chrome.exe", 12),
            w(4, "Gmail - Google Chrome", "chrome.exe", 12),
            w(5, "التنزيلات", "explorer.exe", 13),
            w(6, "Telegram (3)", "Telegram.exe", 14),
            w(7, "WhatsApp", "WhatsApp.Root.exe", 15),
            w(8, "   ", "ghost.exe", 16),
            w(9, "عرض تقديمي.pptx - PowerPoint", "POWERPNT.EXE", 17),
        ]
    }

    fn run(q: &str, english: bool) -> Vec<Item> {
        static NO_LAUNCHES: fn(&str) -> u32 = |_| 0;
        let wins = sample();
        let ctx = Context { english, windows_build: 0, windows: &wins, launches: &NO_LAUNCHES, picked: &NO_LAUNCHES, today: (2026, 11, 1) };
        matches(q, &ctx)
    }

    fn hwnds(q: &str) -> Vec<isize> {
        run(q, false)
            .into_iter()
            .map(|i| match i.action {
                Action::FocusWindow(h) => h,
                other => panic!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn by_exe_and_alias() {
        assert_eq!(hwnds("وورد"), vec![1]);
        assert_eq!(hwnds("winword"), vec![1]);
        assert_eq!(hwnds("word"), vec![1]);
        assert_eq!(hwnds("اكسل"), vec![2]);
        assert_eq!(hwnds("إكسل"), vec![2]);
        assert_eq!(hwnds("كروم"), vec![3, 4]);
        assert_eq!(hwnds("chrome"), vec![3, 4]);
        assert_eq!(hwnds("الملفات"), vec![5]);
        assert_eq!(hwnds("تيليجرام"), vec![6]);
        assert_eq!(hwnds("tele"), vec![6]);
        assert_eq!(hwnds("واتساب"), vec![7]);
        assert_eq!(hwnds("باوربوينت"), vec![9]);
        assert_eq!(hwnds("powerpoint"), vec![9]);
    }

    #[test]
    fn by_title() {
        assert_eq!(hwnds("المبيعات"), vec![1]);
        assert_eq!(hwnds("مبيعات"), vec![1]); // without the article
        assert_eq!(hwnds("تقرير"), vec![1]);
        assert_eq!(hwnds("youtube"), vec![3]);
        assert_eq!(hwnds("gmail"), vec![4]);
        assert_eq!(hwnds("google chrome"), vec![3, 4]);
        assert_eq!(hwnds("الميزانيه"), vec![2]); // ta marbuta normalised
        assert_eq!(hwnds("تنزيلات"), vec![5]);
    }

    #[test]
    fn items_and_scores() {
        let v = run("كروم", false);
        assert_eq!(v[0].title, "التبديل إلى: YouTube - Google Chrome");
        assert_eq!(v[0].subtitle, "chrome.exe");
        assert_eq!(v[0].kind, Kind::Window);
        let e = run("chrome", true);
        assert_eq!(e[0].title, "Switch to: YouTube - Google Chrome");
        // An exact program name ranks above a title-word match.
        assert!(run("وورد", false)[0].score > run("المبيعات", false)[0].score);
    }

    #[test]
    fn no_matches() {
        for q in ["", " ", "a", "و", "zzz", "😀", "xlsx report", "\u{200F}"] {
            assert!(hwnds(q).is_empty(), "{q:?}");
        }
        // A window with an empty title is never offered.
        assert!(hwnds("ghost").is_empty());
    }

    #[test]
    fn alias_lookup() {
        assert_eq!(exe_stem("WINWORD.EXE"), "winword");
        assert_eq!(exe_stem("noext"), "noext");
        assert!(aliases_for("WhatsApp.Root.exe").contains(&"واتساب"));
        assert!(aliases_for("unknown.exe").is_empty());
        assert_eq!(program_match(&norm("كروم"), "chrome.exe"), 3);
        assert_eq!(program_match(&norm("كرو"), "chrome.exe"), 2);
        assert_eq!(program_match(&norm("فايرفوكس"), "chrome.exe"), 0);
    }
}
