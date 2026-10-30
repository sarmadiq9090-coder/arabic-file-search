//! Keyboard handling in the window, and the global hotkey.

use native_windows_gui as nwg;
use winapi::shared::minwindef::UINT;
use winapi::um::winuser::{EM_SETSEL, GetKeyState, SendMessageW, VK_CONTROL, VK_DOWN, VK_F3, VK_UP};

use super::App;
use super::startup::log;

/// Id of the global "show the program" hotkey, Ctrl+Alt+F.
pub(super) const HOTKEY_ID: i32 = 1;
pub(super) const HOTKEY_TEXT: &str = "Ctrl+Alt+F";

impl App {
    // ---------------------------------------------------------------- hotkey

    pub(super) fn register_hotkey(&self) {
        use winapi::um::winuser::{MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey};
        let Some(h) = self.window.handle.hwnd() else { return };
        let ok = unsafe { RegisterHotKey(h, HOTKEY_ID, (MOD_CONTROL | MOD_ALT | MOD_NOREPEAT) as u32, 'F' as u32) } != 0;
        if !ok {
            self.menu_hotkey.set_enabled(false);
            log("hotkey Ctrl+Alt+F is taken by another program");
        }
    }

    /// The global hotkey shows the window, or hides it if it is already in front.
    pub(super) fn on_hotkey(&self) {
        use winapi::um::winuser::GetForegroundWindow;
        let front = self.window.handle.hwnd().is_some_and(|h| unsafe { GetForegroundWindow() } == h);
        if front && self.window.visible() {
            self.hide_to_tray();
        } else {
            self.show_window();
        }
    }

    // ---------------------------------------------------------------- keyboard

    pub(super) fn on_key(&self, handle: nwg::ControlHandle, key: u32) {
        let ctrl = unsafe { GetKeyState(VK_CONTROL) } < 0;
        if (ctrl && key == 'F' as u32) || key == VK_F3 as u32 {
            self.focus_query();
        } else if handle == self.query.handle && key == VK_DOWN as u32 {
            // Down arrow moves from the search box into the results, or
            // opens the search history when there are none.
            if self.state.borrow().rows.is_empty() {
                if self.query.text().trim().is_empty() {
                    self.show_history_menu();
                }
                return;
            }
            if self.list.selected_item().is_none() {
                self.list.select_item(0, true);
            }
            self.list.set_focus();
        } else if handle == self.list.handle && key == VK_UP as u32 && self.list.selected_item() == Some(0) {
            self.focus_query();
        } else if handle == self.list.handle && ctrl && key == 'C' as u32 {
            if unsafe { GetKeyState(winapi::um::winuser::VK_SHIFT) } < 0 {
                self.copy_path();
            } else {
                self.copy_file();
            }
        } else if handle == self.list.handle && key == winapi::um::winuser::VK_F4 as u32 {
            self.show_matches();
        } else if handle == self.list.handle && key == winapi::um::winuser::VK_F2 as u32 {
            self.start_rename();
        } else if handle == self.list.handle && key == winapi::um::winuser::VK_DELETE as u32 {
            self.delete_selected();
        } else if ctrl && key == 'H' as u32 {
            self.show_history_menu();
        }
    }

    pub(super) fn on_enter(&self) {
        if self.renaming.get() {
            return;
        }
        let ctrl = unsafe { GetKeyState(VK_CONTROL) } < 0;
        if !ctrl && self.quick_switch_enter() {
            return;
        }
        if ctrl {
            self.open_folder();
        } else if self.list.focus() {
            self.open_selected();
        } else {
            self.remember_query();
            self.on_search();
        }
    }

    /// Esc clears the search box; a second Esc hides the window to the tray.
    pub(super) fn on_escape(&self) {
        if self.renaming.get() || self.quick_switch_cancel() {
            return;
        }
        if self.state.borrow().content.is_some() {
            self.on_stop();
        } else if !self.query.text().is_empty() {
            self.remember_query();
            self.query.set_text("");
            self.focus_query();
        } else {
            self.hide_to_tray();
        }
    }

    pub(super) fn focus_query(&self) {
        self.query.set_focus();
        if let Some(h) = self.query.handle.hwnd() {
            unsafe { SendMessageW(h, EM_SETSEL as UINT, 0, -1) };
        }
    }
}
