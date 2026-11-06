//! «نقل البرنامج إلى مكان دائم»: InstaFind is one portable exe, and most
//! people run it straight from Downloads (or from inside a zip, which is a
//! temp folder). Once, when that is so, the program offers to copy itself to
//! `%LOCALAPPDATA%\Programs\InstaFind\bahth.exe`, add a Start-menu shortcut,
//! point "start with Windows" at the new place when it is on, and restart
//! from there. Settings, history and the index live in
//! `%LOCALAPPDATA%\BahthSari` and stay as they are. Copies installed by
//! winget, Scoop or Chocolatey, or in Program Files, are never offered (see
//! `crate::relocate`).

use std::io;
use std::path::{Path, PathBuf};

use winapi::um::winuser::{IDYES, MB_ICONERROR, MB_ICONQUESTION, MB_OK, MB_RIGHT, MB_RTLREADING, MB_YESNO, MessageBoxW};

use super::format::ltr_path;
use super::startup::log;
use super::{App, autostart, shell, title};
use crate::relocate::{self, Folders, Place};

/// The folders the exe's place is compared with, as Windows reports them now.
fn folders() -> Folders {
    Folders::from_env(&|name| std::env::var(name).ok(), downloads_folder())
}

/// The Downloads folder, also when the user moved it (its Properties ▸ Location).
fn downloads_folder() -> Option<String> {
    use std::ptr::null_mut;
    use winapi::shared::winerror::SUCCEEDED;
    use winapi::um::combaseapi::CoTaskMemFree;
    use winapi::um::knownfolders::FOLDERID_Downloads;
    use winapi::um::shlobj::SHGetKnownFolderPath;
    let mut p = null_mut();
    // SAFETY: `p` receives a string the shell allocated; it is freed below
    // even when the call fails.
    let hr = unsafe { SHGetKnownFolderPath(&FOLDERID_Downloads, 0, null_mut(), &mut p) };
    if p.is_null() {
        return None;
    }
    let s = unsafe { super::format::wide_to_string(p) };
    unsafe { CoTaskMemFree(p.cast()) };
    SUCCEEDED(hr).then_some(s)
}

/// The Start menu's Programs folder of this user.
fn start_menu_folder() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Microsoft\\Windows\\Start Menu\\Programs"))
}

impl App {
    /// Once (the first time it applies), offers to move a copy that runs
    /// from Downloads or a temp folder to a permanent place.
    pub(super) fn offer_move(&self) {
        if self.state.borrow().settings.move_asked {
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        let place = relocate::classify(&exe.to_string_lossy(), &folders());
        if !place.offer_move() {
            return;
        }
        let Some(local) = std::env::var_os("LOCALAPPDATA") else { return };
        let dir = PathBuf::from(relocate::install_dir(&local.to_string_lossy()));
        // Asked once, whatever the answer (and even if the copy fails).
        self.state.borrow_mut().settings.move_asked = true;
        self.save_settings();
        let where_now = match place {
            Place::Temp => tr!(
                "يعمل «InstaFind» الآن من مجلد مؤقت (ربما من داخل ملف مضغوط)، وقد يحذفه ويندوز في أي وقت.",
                "InstaFind is running from a temporary folder (perhaps from inside a zip file), which Windows may empty at any time."
            ),
            _ => tr!(
                "يعمل «InstaFind» الآن من مجلد التنزيلات. إذا حُذف الملف أو نُظّف المجلد سيتوقف البرنامج.",
                "InstaFind is running from your Downloads folder. If the file is deleted or the folder is cleaned up, the program stops working."
            ),
        };
        let autostart_line = if autostart::is_enabled() {
            tr!("\nالتشغيل مع ويندوز سيستخدم المكان الجديد.", "\nStarting with Windows will use the new place.")
        } else {
            ""
        };
        let shown_dir = ltr_path(&dir.to_string_lossy());
        let text = trf!(
            "{where_now}\n\nهل تريد نقل البرنامج إلى مكان دائم؟ سيُنسخ إلى:\n{shown_dir}\nوتُضاف له أيقونة في قائمة «ابدأ»، ثم يُعاد تشغيله من هناك. إعداداتك وفهرسك يبقيان كما هما.{autostart_line}",
            "{where_now}\n\nMove the program to a permanent place? It is copied to:\n{shown_dir}\nadded to the Start menu, and restarted from there. Your settings and index stay as they are.{autostart_line}"
        );
        log(&format!("move offer: running from {place:?} ({})", exe.display()));
        if !self.move_message(&text, MB_YESNO | MB_ICONQUESTION) {
            log("move offer: declined");
            return;
        }
        let target = match copy_to(&exe, &dir) {
            Ok(t) => t,
            Err(e) => {
                log(&format!("move offer: copy failed: {e}"));
                self.move_message(
                    &trf!(
                        "تعذر نسخ البرنامج إلى المكان الدائم:\n{e}\n\nالبرنامج يعمل كما هو من مكانه الحالي.",
                        "Could not copy the program to the permanent place:\n{e}\n\nThe program keeps working from where it is."
                    ),
                    MB_OK | MB_ICONERROR,
                );
                return;
            }
        };
        // Both are extras: the moved copy works without them.
        if let Some(menu) = start_menu_folder() {
            let lnk = menu.join(format!("{}.lnk", title()));
            if let Err(e) = create_shortcut(&target, &lnk) {
                log(&format!("move offer: Start-menu shortcut failed: {e}"));
            }
        }
        if !autostart::point_to(&target) {
            log("move offer: could not point start-with-Windows at the new place");
        }
        log(&format!("move offer: copied to {}; restarting from there", target.display()));
        match restart_from(&target) {
            Ok(()) => {
                // As after an update: save everything and leave at once, so
                // the new copy gets the single-instance lock.
                self.on_close();
                std::process::exit(0);
            }
            Err(e) => {
                log(&format!("move offer: restart failed: {e}"));
                self.move_message(
                    &trf!(
                        "نُسخ البرنامج إلى المكان الدائم، لكن تعذرت إعادة تشغيله من هناك ({e}).\nافتحه من قائمة «ابدأ».",
                        "The program was copied to the permanent place, but could not restart from there ({e}).\nOpen it from the Start menu."
                    ),
                    MB_OK | MB_ICONERROR,
                );
            }
        }
    }

    /// A message box over the main window, right to left in Arabic; true
    /// when "Yes" was chosen.
    fn move_message(&self, text: &str, flags: u32) -> bool {
        let flags = flags | if crate::i18n::is_en() { 0 } else { MB_RTLREADING | MB_RIGHT };
        let caption = tr!("نقل البرنامج إلى مكان دائم", "Move the program to a permanent place");
        let (t, c) = (shell::wide(text), shell::wide(caption));
        let owner = self.window.handle.hwnd().unwrap_or(std::ptr::null_mut());
        unsafe { MessageBoxW(owner, t.as_ptr(), c.as_ptr(), flags) == IDYES }
    }
}

/// Copies the running exe to `dir\bahth.exe` (replacing an older copy
/// there) and returns that path. The bytes are written to a new file, so
/// the download's "from the internet" mark is not carried over: like an
/// installer's files, the permanent copy then starts without the warning
/// Windows shows for downloaded programs.
fn copy_to(exe: &Path, dir: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let target = dir.join(relocate::EXE_NAME);
    let part = dir.join(format!("{}.moving", relocate::EXE_NAME));
    let copied = (|| {
        let mut from = std::fs::File::open(exe)?;
        let mut to = std::fs::File::create(&part)?;
        let n = io::copy(&mut from, &mut to)?;
        to.sync_all()?;
        if n != std::fs::metadata(exe)?.len() {
            return Err(io::Error::other("the copy is incomplete"));
        }
        drop(to);
        std::fs::rename(&part, &target)
    })();
    if copied.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    copied.map(|()| target)
}

/// Starts `exe` with [`relocate::MOVED_ARG`], detached from this process.
fn restart_from(exe: &Path) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg(relocate::MOVED_ARG).creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    cmd.spawn().map(drop)
}

/// Releases a COM interface when dropped.
struct Com<T>(*mut T);

impl<T> Drop for Com<T> {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: every `Com` holds an interface pointer the caller owns.
            unsafe { (*(self.0 as *mut winapi::um::unknwnbase::IUnknown)).Release() };
        }
    }
}

/// Writes the shortcut `lnk` to `target` (the Start menu lists it at once).
/// COM is already initialised on the UI thread (`ui::run`).
fn create_shortcut(target: &Path, lnk: &Path) -> Result<(), String> {
    use std::ptr::null_mut;
    use winapi::shared::minwindef::TRUE;
    use winapi::shared::winerror::{FAILED, HRESULT};
    use winapi::shared::wtypesbase::CLSCTX_INPROC_SERVER;
    use winapi::um::combaseapi::CoCreateInstance;
    use winapi::um::objidl::IPersistFile;
    use winapi::um::shobjidl_core::{IShellLinkW, ShellLink};
    use winapi::{Class, Interface};
    let check = |what: &str, hr: HRESULT| if FAILED(hr) { Err(format!("{what}: 0x{:08x}", hr as u32)) } else { Ok(()) };
    let (target_w, lnk_w) = (shell::wide(&target.to_string_lossy()), shell::wide(&lnk.to_string_lossy()));
    let dir_w = shell::wide(&target.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default());
    let about = shell::wide(tr!(
        "ابحث عن ملفاتك فوراً بالعربية والإنجليزية، حتى داخل المستندات",
        "Find your files as you type, in Arabic or English, even inside documents"
    ));
    let mut link: *mut IShellLinkW = null_mut();
    // SAFETY: plain COM calls on interfaces created here; each is released by `Com`.
    unsafe {
        check(
            "CoCreateInstance(ShellLink)",
            CoCreateInstance(&ShellLink::uuidof(), null_mut(), CLSCTX_INPROC_SERVER, &IShellLinkW::uuidof(), (&mut link as *mut *mut IShellLinkW).cast()),
        )?;
        let link = Com(link);
        let l = &*link.0;
        check("SetPath", l.SetPath(target_w.as_ptr()))?;
        check("SetWorkingDirectory", l.SetWorkingDirectory(dir_w.as_ptr()))?;
        check("SetDescription", l.SetDescription(about.as_ptr()))?;
        check("SetIconLocation", l.SetIconLocation(target_w.as_ptr(), 0))?;
        let mut file: *mut IPersistFile = null_mut();
        check("QueryInterface(IPersistFile)", l.QueryInterface(&IPersistFile::uuidof(), (&mut file as *mut *mut IPersistFile).cast()))?;
        let file = Com(file);
        check("Save", (*file.0).Save(lnk_w.as_ptr(), TRUE))
    }
}
