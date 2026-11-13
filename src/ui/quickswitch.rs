//! Quick Switch: with an Open or Save As dialog in front of any program,
//! Ctrl+Alt+G brings up the search; Enter on a result moves that dialog to
//! the result's folder (Listary-style).
//!
//! The dialog is driven from outside through its file-name box: the folder
//! path is typed into it and OK is pressed, which makes every common file
//! dialog (old and Vista-style) navigate there, and then the original file
//! name is put back so a Save As keeps its name.

use native_windows_gui as nwg;
use std::cell::Cell;
use std::rc::Rc;

use winapi::shared::minwindef::{DWORD, LPARAM, UINT, WPARAM};
use winapi::shared::windef::HWND;
use winapi::shared::winerror::ERROR_SUCCESS;
use winapi::um::winnt::REG_DWORD;
use winapi::um::winreg::{RegQueryValueExW, RegSetValueExW};
use winapi::um::winuser::{
    BM_CLICK, GA_ROOTOWNER, GW_CHILD, GW_HWNDNEXT, GetAncestor, GetClassNameW, GetDlgCtrlID, GetDlgItem,
    GetForegroundWindow, GetLastActivePopup, GetWindow, IDOK, IsWindow, IsWindowEnabled, IsWindowVisible, MOD_ALT, MOD_CONTROL,
    MOD_NOREPEAT, PostMessageW, RegisterHotKey, SendMessageTimeoutW, SMTO_ABORTIFHUNG, SetForegroundWindow,
    UnregisterHotKey, WM_COMMAND, WM_GETTEXT, WM_GETTEXTLENGTH, WM_HOTKEY, WM_SETTEXT,
};

use super::App;
use super::autostart;
use super::shell::wide;
use super::startup::log;

/// Id of the Quick Switch hotkey (the "show" hotkey is 1).
pub(super) const HOTKEY_ID: i32 = 2;
pub(super) const HOTKEY_TEXT: &str = "Ctrl+Alt+G";
/// On/off switch, a DWORD under `HKCU\Software\BahthSari`.
const REG_VALUE: &str = "QuickSwitch";

fn hint() -> &'static str {
    tr!(
        "Enter: نقل النافذة إلى مجلد النتيجة · Esc: إلغاء",
        "Enter: move the dialog to the result's folder · Esc: cancel"
    )
}

/// Quick Switch state and controls, one field of `App`.
#[derive(Default)]
pub struct QuickSwitch {
    /// The dialog to move, as an `HWND` (0 when none).
    target: Cell<usize>,
    enabled: Cell<bool>,
    registered: Cell<bool>,
    menu_switch: nwg::MenuItem,
    menu_option: nwg::MenuItem,
}

// ---------------------------------------------------------------- registry

fn load_enabled() -> bool {
    let Some(key) = autostart::open(autostart::OWN_KEY, false) else { return true };
    let name = wide(REG_VALUE);
    let (mut value, mut size, mut kind): (DWORD, DWORD, DWORD) = (1, 4, 0);
    let ok = unsafe {
        RegQueryValueExW(
            key.0,
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut kind,
            (&mut value as *mut DWORD).cast(),
            &mut size,
        )
    } == ERROR_SUCCESS as i32;
    // Missing or unreadable means the default, on.
    !(ok && kind == REG_DWORD && value == 0)
}

fn save_enabled(on: bool) {
    let Some(key) = autostart::open(autostart::OWN_KEY, true) else { return };
    let name = wide(REG_VALUE);
    let value: DWORD = on as DWORD;
    unsafe {
        RegSetValueExW(key.0, name.as_ptr(), 0, REG_DWORD, (&value as *const DWORD).cast(), 4);
    }
}

// ---------------------------------------------------------------- window helpers

fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(h, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// Longest window text read from another program, in UTF-16 units: a
/// long path (`\\?\` prefix) is at most 32,767 units.
const MAX_TEXT: usize = 32_768;

/// The buffer length for a text of the length another program reported.
/// Its answer is untrusted: a negative or huge LRESULT must not overflow
/// `len + 1` nor make us allocate gigabytes.
fn text_len(reported: usize) -> usize {
    reported.min(MAX_TEXT)
}

/// Text of another program's window. A hung program can't stall us.
fn window_text(h: HWND) -> String {
    let mut len: usize = 0;
    unsafe {
        SendMessageTimeoutW(h, WM_GETTEXTLENGTH, 0, 0, SMTO_ABORTIFHUNG, 500, &mut len);
    }
    let len = text_len(len);
    let mut buf = vec![0u16; len + 1];
    let mut got: usize = 0;
    unsafe {
        SendMessageTimeoutW(
            h,
            WM_GETTEXT,
            buf.len() as WPARAM,
            buf.as_mut_ptr() as LPARAM,
            SMTO_ABORTIFHUNG,
            500,
            &mut got,
        );
    }
    String::from_utf16_lossy(&buf[..got.min(len)])
}

fn set_window_text(h: HWND, text: &str) -> bool {
    let w = wide(text);
    let mut r: usize = 0;
    let sent = unsafe { SendMessageTimeoutW(h, WM_SETTEXT, 0, w.as_ptr() as LPARAM, SMTO_ABORTIFHUNG, 1000, &mut r) };
    sent != 0 && r != 0
}

/// `h` and its child windows as `crate::file_dialog` reads them, at most
/// `budget` windows and 16 levels deep (a dialog has a few dozen).
fn describe(h: HWND, depth: u32, budget: &mut u32) -> crate::file_dialog::Node {
    let mut node = crate::file_dialog::Node {
        class: class_name(h),
        id: unsafe { GetDlgCtrlID(h) },
        hidden: unsafe { IsWindowVisible(h) } == 0,
        handle: h as usize,
        children: Vec::new(),
    };
    if depth < 16 {
        let mut c = unsafe { GetWindow(h, GW_CHILD) };
        while !c.is_null() && *budget > 0 {
            *budget -= 1;
            node.children.push(describe(c, depth + 1, budget));
            c = unsafe { GetWindow(c, GW_HWNDNEXT) };
        }
    }
    node
}

/// The editable file-name box of a common file dialog, if it has one
/// (`crate::file_dialog::filename_box`: the cmb13 combo of Open dialogs,
/// the floating combo of the Windows 11 Save dialog, or edt1).
fn filename_edit(dialog: HWND) -> Option<HWND> {
    if dialog.is_null() || unsafe { IsWindow(dialog) } == 0 {
        return None;
    }
    let tree = describe(dialog, 0, &mut 2000);
    crate::file_dialog::filename_box(&tree).map(|b| b.handle as HWND)
}

/// Whether `h` is a common Open / Save As dialog: a file-name box and a
/// view of the files (`crate::file_dialog`), not just any dialog with an
/// edt1 box (Find, Print) or a DirectUI child.
pub(super) fn is_file_dialog(h: HWND) -> bool {
    if h.is_null() || unsafe { IsWindow(h) } == 0 || class_name(h) != "#32770" {
        return false;
    }
    crate::file_dialog::is_file_dialog(&describe(h, 0, &mut 2000))
}

/// The class of the window in front, for the log when Ctrl+Alt+G finds no
/// file dialog (a class name says which dialog it was, and nothing private).
fn front_class() -> String {
    let front = unsafe { GetForegroundWindow() };
    if front.is_null() { "none".into() } else { class_name(front) }
}

/// The file dialog the user is looking at: the foreground window, or the
/// popup its program last activated.
fn foreground_file_dialog() -> Option<HWND> {
    let front = unsafe { GetForegroundWindow() };
    if is_file_dialog(front) {
        return Some(front);
    }
    if front.is_null() {
        return None;
    }
    let popup = unsafe { GetLastActivePopup(GetAncestor(front, GA_ROOTOWNER)) };
    (popup != front && is_file_dialog(popup)).then_some(popup)
}

/// The folder a result stands for: the folder itself, or the one holding the file.
pub(super) fn result_folder(path: &str, is_dir: bool) -> String {
    let path = path.trim_end_matches(['\\', '/']);
    let folder = if is_dir {
        path
    } else {
        match path.rfind(['\\', '/']) {
            Some(i) => &path[..i],
            None => path,
        }
    };
    // "C:" alone means the drive's current folder; the root needs its slash,
    // and a trailing slash makes the dialog read it as a folder, not a name.
    format!("{folder}\\")
}

/// Moves an Open / Save As dialog to `folder`: types the folder into its
/// file-name box, presses OK so it navigates there, then puts the original
/// file name back. Returns false when the dialog has no usable file-name box.
pub(super) fn switch_dialog_folder(dialog: HWND, folder: &str) -> bool {
    if unsafe { IsWindow(dialog) } == 0 {
        return false;
    }
    let Some(edit) = filename_edit(dialog) else { return false };
    let original = window_text(edit);
    unsafe { SetForegroundWindow(dialog) };
    if !set_window_text(edit, folder) {
        return false;
    }
    let ok = unsafe { GetDlgItem(dialog, IDOK) };
    unsafe {
        if !ok.is_null() && IsWindowVisible(ok) != 0 && IsWindowEnabled(ok) != 0 {
            // Posted, not sent: a click can open a message box in the other
            // program ("file exists"), which would block a sent message.
            PostMessageW(ok, BM_CLICK, 0, 0);
        } else {
            PostMessageW(dialog, WM_COMMAND, IDOK as WPARAM, 0);
        }
    }
    // The newer dialogs navigate asynchronously and may rewrite the box
    // meanwhile (a slow folder late), so the name goes back once the click
    // has been handled and again whenever the dialog takes it away, for
    // two seconds (`crate::file_dialog::after_switch`); the user's own
    // typing after that is left alone.
    let (dialog_addr, edit_addr) = (dialog as usize, edit as usize);
    let typed = folder.to_string();
    // `std::thread::spawn` would panic if no thread can be made.
    crate::crash_log::spawn_named("quick-switch", move || {
        use crate::file_dialog::{NameBox, after_switch};
        let dialog = dialog_addr as HWND;
        let mut edit = edit_addr as HWND;
        for step in 0..8 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            if unsafe { IsWindow(dialog) } == 0 {
                return;
            }
            // The dialog may have made a new box for the new folder.
            if unsafe { IsWindow(edit) } == 0 {
                match filename_edit(dialog) {
                    Some(e) => edit = e,
                    None => return,
                }
            }
            match after_switch(&window_text(edit), &original, &typed, step < 2) {
                NameBox::Fine => {}
                NameBox::Restore => {
                    set_window_text(edit, &original);
                }
                NameBox::Stop => return,
            }
        }
    });
    true
}

// ---------------------------------------------------------------- the app's side

impl App {
    /// Adds the context-menu and Options items. Called from `build_ui`
    /// while the app is still being built.
    pub(super) fn build_quick_switch_menus(&mut self) -> Result<(), nwg::NwgError> {
        let enabled = load_enabled();
        nwg::MenuItem::builder()
            .text(tr!("فتح مجلدها في نافذة الحفظ/الفتح\tEnter", "Open its folder in the Save/Open dialog\tEnter"))
            .disabled(true)
            .parent(&self.menu)
            .build(&mut self.quick.menu_switch)?;
        nwg::MenuItem::builder()
            .text(&trf!("القفز إلى مجلد النتيجة في نوافذ الحفظ والفتح ({HOTKEY_TEXT})", "Jump to the result's folder in Save/Open dialogs ({HOTKEY_TEXT})"))
            .check(enabled)
            .parent(&self.options_menu)
            .build(&mut self.quick.menu_option)?;
        self.quick.enabled.set(enabled);
        Ok(())
    }

    /// Registers the hotkey and binds the events. Called once from
    /// `build_ui`; the handlers are unbound with the others.
    pub(super) fn setup_quick_switch(
        self: &Rc<Self>,
    ) -> Result<(nwg::EventHandler, nwg::RawEventHandler), nwg::NwgError> {
        if self.quick.enabled.get() {
            self.register_quick_switch();
        }

        let weak = Rc::downgrade(self);
        let handler = nwg::full_bind_event_handler(&self.window.handle, crate::crash_log::guard_events(move |evt, _data, handle| {
            let Some(app) = weak.upgrade() else { return };
            match evt {
                nwg::Event::OnMenuItemSelected if handle == app.quick.menu_switch => {
                    app.quick_switch_selected();
                }
                nwg::Event::OnMenuItemSelected if handle == app.quick.menu_option => {
                    app.toggle_quick_switch();
                }
                // Picking a result is when the reminder helps most.
                nwg::Event::OnListViewItemChanged if handle == app.list && app.quick_target().is_some() => {
                    app.set_status(hint());
                }
                _ => {}
            }
        }));
        let weak = Rc::downgrade(self);
        let raw = nwg::bind_raw_event_handler(&self.window.handle, 0x1_0002, crate::crash_log::guard_raw(move |_h, msg: UINT, w: WPARAM, _l| {
            if msg == WM_HOTKEY && w as i32 == HOTKEY_ID {
                weak.upgrade()?.on_quick_switch_hotkey();
                return Some(0);
            }
            None
        }))?;
        Ok((handler, raw))
    }

    fn register_quick_switch(&self) {
        let Some(h) = self.window.handle.hwnd() else { return };
        if self.quick.registered.get() {
            return;
        }
        let mods = (MOD_CONTROL | MOD_ALT | MOD_NOREPEAT) as u32;
        let ok = unsafe { RegisterHotKey(h, HOTKEY_ID, mods, 'G' as u32) } != 0;
        self.quick.registered.set(ok);
        if !ok {
            log("hotkey Ctrl+Alt+G is taken by another program");
        }
    }

    fn unregister_quick_switch(&self) {
        if let Some(h) = self.window.handle.hwnd()
            && self.quick.registered.replace(false)
        {
            unsafe { UnregisterHotKey(h, HOTKEY_ID) };
        }
    }

    fn toggle_quick_switch(&self) {
        let on = !self.quick.menu_option.checked();
        self.quick.menu_option.set_checked(on);
        self.quick.enabled.set(on);
        save_enabled(on);
        if on {
            self.register_quick_switch();
            if self.quick.registered.get() {
                self.set_status(&trf!(
                    "في نافذة حفظ أو فتح بأي برنامج، اضغط {HOTKEY_TEXT} ثم اختر نتيجة لنقل النافذة إلى مجلدها.",
                    "In any program's Save or Open dialog, press {HOTKEY_TEXT}, then pick a result to move the dialog to its folder."
                ));
            } else {
                self.set_status(&trf!("الاختصار {HOTKEY_TEXT} مستخدم في برنامج آخر.", "The shortcut {HOTKEY_TEXT} is used by another program."));
            }
        } else {
            self.unregister_quick_switch();
            self.set_quick_target(None);
            self.set_status(tr!("أوقف القفز إلى مجلد النتيجة في نوافذ الحفظ والفتح.", "Jumping to the result's folder in Save/Open dialogs is off."));
        }
    }

    /// The dialog waiting to be moved, if it is still open.
    fn quick_target(&self) -> Option<HWND> {
        let h = self.quick.target.get() as HWND;
        if h.is_null() {
            return None;
        }
        if unsafe { IsWindow(h) } == 0 {
            self.set_quick_target(None);
            return None;
        }
        Some(h)
    }

    fn set_quick_target(&self, dialog: Option<HWND>) {
        self.quick.target.set(dialog.map_or(0, |h| h as usize));
        self.quick.menu_switch.set_enabled(dialog.is_some());
        // The new look shows the same mode: a line with the hint, Enter
        // moves the dialog, Esc cancels, and its result menu has the item.
        self.webui_quick_switch(dialog.map(|_| hint()));
    }

    /// Whether a Save/Open dialog is still waiting to be moved (forgets
    /// one that was closed meanwhile). Only the new look asks; mingw builds
    /// have none.
    #[cfg_attr(not(target_env = "msvc"), allow(dead_code))]
    pub(super) fn quick_switch_waiting(&self) -> bool {
        self.quick_target().is_some()
    }

    /// Ctrl+Alt+G: remember the file dialog in front, if any, and show the search.
    fn on_quick_switch_hotkey(&self) {
        if !self.quick.enabled.get() {
            return;
        }
        let dialog = foreground_file_dialog();
        match dialog {
            Some(d) => log(&format!(
                "quick switch: hotkey, file dialog found (box {})",
                if filename_edit(d).is_some() { "found" } else { "none" }
            )),
            None => log(&format!("quick switch: hotkey, file dialog not in front (front window class {})", front_class())),
        }
        self.set_quick_target(dialog);
        self.show_window();
        // (The new look shows the hint on its own line while the mode lasts.)
        if dialog.is_some() && !self.webui_in_use() {
            self.set_status(hint());
        }
    }

    /// Enter while a dialog is waiting: move it to the chosen result's
    /// folder. Returns false when there is no dialog or no result, so Enter
    /// does its usual job.
    pub(super) fn quick_switch_enter(&self) -> bool {
        if self.quick_target().is_none() {
            return false;
        }
        // From the search box, Enter takes the selected or the top result.
        let row = self.list.selected_item().or_else(|| (!self.state.borrow().rows.is_empty()).then_some(0));
        match row {
            Some(row) => {
                self.switch_to_row(row);
                true
            }
            None => false,
        }
    }

    /// Esc while a dialog is waiting forgets it. Returns true when it did.
    pub(super) fn quick_switch_cancel(&self) -> bool {
        if self.quick_target().is_none() {
            return false;
        }
        self.set_quick_target(None);
        self.set_status(tr!("أُلغي نقل نافذة الحفظ/الفتح.", "Save/Open dialog switch cancelled."));
        true
    }

    fn quick_switch_selected(&self) {
        if self.quick_target().is_none() {
            self.set_status(&press_first());
            return;
        }
        match self.list.selected_item() {
            Some(row) => self.switch_to_row(row),
            None => self.set_status(tr!("اختر نتيجة من القائمة أولاً.", "Pick a result from the list first.")),
        }
    }

    fn switch_to_row(&self, row: usize) {
        if self.quick_target().is_none() {
            return;
        }
        let Some((path, is_dir)) = self.state.borrow().row_entry(row).map(|e| (e.path.to_string(), e.is_dir)) else {
            return;
        };
        self.note_use(&path);
        self.remember_query();
        self.quick_switch_to(&path, is_dir);
    }

    /// Moves the waiting Save/Open dialog to the folder of `path` (a result
    /// picked in either look; the folder itself when `is_dir`). The main
    /// window in use gets out of the way first, so the dialog gets the
    /// focus back, and comes back with a word when the move fails. True
    /// when the dialog was moved.
    pub(super) fn quick_switch_to(&self, path: &str, is_dir: bool) -> bool {
        let Some(dialog) = self.quick_target() else {
            self.set_status(&press_first());
            return false;
        };
        let folder = result_folder(path, is_dir);
        if !self.webui_step_aside() {
            self.window.set_visible(false);
        }
        if switch_dialog_folder(dialog, &folder) {
            self.set_quick_target(None);
            log("quick switch: moved a file dialog");
            true
        } else {
            self.show_window();
            self.set_quick_target(None);
            self.set_status(tr!("تعذر نقل نافذة الحفظ/الفتح إلى هذا المجلد.", "Could not move the Save/Open dialog to this folder."));
            false
        }
    }
}

/// Said when a result is picked for a dialog that is no longer waiting.
fn press_first() -> String {
    trf!("اضغط {HOTKEY_TEXT} في نافذة حفظ أو فتح أولاً.", "Press {HOTKEY_TEXT} in a Save or Open dialog first.")
}

#[cfg(test)]
mod tests {
    use super::{MAX_TEXT, result_folder, text_len};

    #[test]
    fn untrusted_text_lengths_are_capped() {
        assert_eq!(text_len(0), 0);
        assert_eq!(text_len(260), 260);
        assert_eq!(text_len(32_767), 32_767);
        assert_eq!(text_len(usize::MAX), MAX_TEXT);
        // A negative LRESULT read as usize.
        assert_eq!(text_len(-1isize as usize), MAX_TEXT);
        assert!(text_len(usize::MAX).checked_add(1).is_some());
    }

    #[test]
    fn result_folder_never_panics() {
        assert_eq!(result_folder("", false), "\\");
        assert_eq!(result_folder("", true), "\\");
        assert_eq!(result_folder("\\\\", false), "\\");
        assert_eq!(result_folder("ملف", false), "ملف\\");
        let long = format!(r"\\?\C:\{}\f.txt", "مجلد\\".repeat(300));
        assert!(result_folder(&long, false).starts_with(r"\\?\C:\"));
        // Random names made of awkward pieces: always one folder, ending in a slash.
        const PIECES: &[&str] = &["\\", "/", "a", "ب", "\u{0651}", "\u{0640}", "\u{200F}", "👨‍👩‍👧", ".", " ", "C:", "\u{FFFD}"];
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = |n: usize| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % n as u64) as usize
        };
        for _ in 0..2000 {
            let path: String = (0..next(12)).map(|_| PIECES[next(PIECES.len())]).collect();
            let is_dir = next(2) == 0;
            let f = result_folder(&path, is_dir);
            assert!(f.ends_with('\\'), "{path:?} -> {f:?}");
            assert!(f.len() <= path.len() + 1, "{path:?} -> {f:?}");
        }
    }

    #[test]
    fn folder_of_a_result() {
        assert_eq!(result_folder(r"C:\Users\سيف\Documents\تقرير.pdf", false), r"C:\Users\سيف\Documents\");
        assert_eq!(result_folder(r"C:\Users\سيف\Documents", true), r"C:\Users\سيف\Documents\");
        assert_eq!(result_folder(r"C:\Users\سيف\Documents\", true), r"C:\Users\سيف\Documents\");
        assert_eq!(result_folder(r"C:\ملف.txt", false), r"C:\");
        assert_eq!(result_folder(r"D:\", true), r"D:\");
        assert_eq!(result_folder(r"\\server\share\a.doc", false), r"\\server\share\");
    }
}
