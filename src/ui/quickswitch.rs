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

use winapi::shared::minwindef::{BOOL, DWORD, LPARAM, UINT, WPARAM};
use winapi::shared::windef::HWND;
use winapi::shared::winerror::ERROR_SUCCESS;
use winapi::um::winnt::REG_DWORD;
use winapi::um::winreg::{RegQueryValueExW, RegSetValueExW};
use winapi::um::winuser::{
    BM_CLICK, EnumChildWindows, GA_ROOTOWNER, GetAncestor, GetClassNameW, GetDlgCtrlID, GetDlgItem,
    GetForegroundWindow, GetLastActivePopup, IDOK, IsWindow, IsWindowEnabled, IsWindowVisible, MOD_ALT, MOD_CONTROL,
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

/// Control ids of the file-name box in the common file dialogs: the combo
/// box (cmb13) of the Vista-style and newer old-style dialogs, and the plain
/// edit (edt1) of the oldest ones.
const ID_FILENAME_COMBO: i32 = 0x47C;
const ID_FILENAME_EDIT: i32 = 0x480;

fn hint() -> &'static str {
    tr!(
        "اختر نتيجة واضغط Enter لنقل نافذة الحفظ إلى مجلدها (Esc للإلغاء)",
        "Pick a result and press Enter to move the Save dialog to its folder (Esc to cancel)"
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

/// Every descendant window of `parent`, in Z order, depth first.
fn descendants(parent: HWND) -> Vec<HWND> {
    unsafe extern "system" fn collect(h: HWND, l: LPARAM) -> BOOL {
        unsafe { (*(l as *mut Vec<HWND>)).push(h) };
        1
    }
    let mut out: Vec<HWND> = Vec::new();
    unsafe { EnumChildWindows(parent, Some(collect), &mut out as *mut Vec<HWND> as LPARAM) };
    out
}

/// Text of another program's window. A hung program can't stall us.
fn window_text(h: HWND) -> String {
    let mut len: usize = 0;
    unsafe {
        SendMessageTimeoutW(h, WM_GETTEXTLENGTH, 0, 0, SMTO_ABORTIFHUNG, 500, &mut len);
    }
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

/// The editable file-name box of a common file dialog, if it has one.
fn filename_edit(dialog: HWND) -> Option<HWND> {
    let all = descendants(dialog);
    let id = |h: HWND| unsafe { GetDlgCtrlID(h) };
    // The combo box (ComboBoxEx32 in a Save dialog, ComboBox in Open): its
    // edit is a child of it, or of its inner ComboBox.
    for &combo in all.iter().filter(|&&h| id(h) == ID_FILENAME_COMBO) {
        if class_name(combo).eq_ignore_ascii_case("Edit") {
            return Some(combo);
        }
        if let Some(&edit) = descendants(combo).iter().find(|&&h| class_name(h).eq_ignore_ascii_case("Edit")) {
            return Some(edit);
        }
    }
    all.into_iter().find(|&h| id(h) == ID_FILENAME_EDIT && class_name(h).eq_ignore_ascii_case("Edit"))
}

/// Whether `h` is a common Open / Save As dialog: a dialog-class window
/// with a file-name box, or with the Vista-style dialog's DirectUI view.
pub(super) fn is_file_dialog(h: HWND) -> bool {
    if h.is_null() || unsafe { IsWindow(h) } == 0 || class_name(h) != "#32770" {
        return false;
    }
    filename_edit(h).is_some() || descendants(h).into_iter().any(|c| class_name(c) == "DirectUIHWND")
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
    // meanwhile, so give the name back once the click has been handled.
    let edit_addr = edit as usize;
    std::thread::spawn(move || {
        let edit = edit_addr as HWND;
        for _ in 0..2 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            if unsafe { IsWindow(edit) } == 0 {
                return;
            }
            let now = window_text(edit);
            if now != original {
                set_window_text(edit, &original);
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
        let handler = nwg::full_bind_event_handler(&self.window.handle, move |evt, _data, handle| {
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
        });
        let weak = Rc::downgrade(self);
        let raw = nwg::bind_raw_event_handler(&self.window.handle, 0x1_0002, move |_h, msg: UINT, w: WPARAM, _l| {
            if msg == WM_HOTKEY && w as i32 == HOTKEY_ID {
                weak.upgrade()?.on_quick_switch_hotkey();
                return Some(0);
            }
            None
        })?;
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
    }

    /// Ctrl+Alt+G: remember the file dialog in front, if any, and show the search.
    fn on_quick_switch_hotkey(&self) {
        if !self.quick.enabled.get() {
            return;
        }
        let dialog = foreground_file_dialog();
        log(&format!(
            "quick switch: hotkey, file dialog {} (box {})",
            if dialog.is_some() { "found" } else { "not in front" },
            if dialog.and_then(filename_edit).is_some() { "found" } else { "none" },
        ));
        self.set_quick_target(dialog);
        self.show_window();
        if dialog.is_some() {
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
            self.set_status(&trf!("اضغط {HOTKEY_TEXT} في نافذة حفظ أو فتح أولاً.", "Press {HOTKEY_TEXT} in a Save or Open dialog first."));
            return;
        }
        match self.list.selected_item() {
            Some(row) => self.switch_to_row(row),
            None => self.set_status(tr!("اختر نتيجة من القائمة أولاً.", "Pick a result from the list first.")),
        }
    }

    fn switch_to_row(&self, row: usize) {
        let Some(dialog) = self.quick_target() else { return };
        let Some((path, is_dir)) = self.state.borrow().row_entry(row).map(|e| (e.path.to_string(), e.is_dir)) else {
            return;
        };
        let folder = result_folder(&path, is_dir);
        self.note_use(&path);
        self.remember_query();
        // Out of the way first, so the dialog gets the focus back.
        self.window.set_visible(false);
        if switch_dialog_folder(dialog, &folder) {
            self.set_quick_target(None);
            log("quick switch: moved a file dialog");
        } else {
            self.show_window();
            self.set_quick_target(None);
            self.set_status(tr!("تعذر نقل نافذة الحفظ/الفتح إلى هذا المجلد.", "Could not move the Save/Open dialog to this folder."));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::result_folder;

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
