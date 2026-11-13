//! Keyboard handling in the window, and the global hotkey.

use native_windows_gui as nwg;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use winapi::shared::minwindef::UINT;
use winapi::um::winuser::{EM_SETSEL, GetKeyState, SendMessageW, VK_CONTROL, VK_DOWN, VK_F3, VK_UP};

use super::App;
use super::startup::log;
use crate::settings::Hotkey;

/// Id of the global "show the program" hotkey (Ctrl+Alt+F unless the user
/// chose another in Settings: `Settings::hotkey`).
pub(super) const HOTKEY_ID: i32 = 1;

/// The chosen hotkey (its place in `Hotkey::ALL`), for the labels that name it.
static CURRENT: AtomicU8 = AtomicU8::new(0);
/// Another program held the chosen hotkey when it was registered.
static TAKEN: AtomicBool = AtomicBool::new(false);

/// The keys of the global hotkey, as the menus, tips and hints name them.
pub(super) fn hotkey_text() -> &'static str {
    current_hotkey().label()
}

pub(super) fn current_hotkey() -> Hotkey {
    Hotkey::ALL.get(CURRENT.load(Ordering::Relaxed) as usize).copied().unwrap_or_default()
}

/// Called with the settings before any label is built.
pub(super) fn set_current_hotkey(h: Hotkey) {
    CURRENT.store(Hotkey::ALL.iter().position(|&x| x == h).unwrap_or(0) as u8, Ordering::Relaxed);
}

/// Whether another program has the chosen hotkey (Settings says so).
pub(super) fn hotkey_taken() -> bool {
    TAKEN.load(Ordering::Relaxed)
}

/// `RegisterHotKey`'s modifiers and key for `h`.
fn mods_and_key(h: Hotkey) -> (u32, u32) {
    use winapi::um::winuser::{MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, VK_SPACE};
    let (mods, key) = match h {
        Hotkey::CtrlAltF => (MOD_CONTROL | MOD_ALT, 'F' as u32),
        Hotkey::CtrlAltSpace => (MOD_CONTROL | MOD_ALT, VK_SPACE as u32),
        Hotkey::CtrlShiftF => (MOD_CONTROL | MOD_SHIFT, 'F' as u32),
        Hotkey::WinAltF => (MOD_WIN | MOD_ALT, 'F' as u32),
    };
    ((mods | MOD_NOREPEAT) as u32, key)
}
/// Id of the optional Alt+Space hotkey (quick search), off by default.
/// Not 2: that is Ctrl+Alt+G (`quickswitch::HOTKEY_ID`) on the same window.
pub(super) const ALT_SPACE_ID: i32 = 3;
const _: () = assert!(ALT_SPACE_ID != HOTKEY_ID && ALT_SPACE_ID != super::quickswitch::HOTKEY_ID);

impl App {
    // ---------------------------------------------------------------- hotkey

    /// Registers the chosen hotkey. When another program has it, the
    /// Options item is greyed and Settings says so, offering the others.
    pub(super) fn register_hotkey(&self) {
        let chosen = self.state.borrow().settings.hotkey;
        set_current_hotkey(chosen);
        let ok = self.try_hotkey(chosen);
        TAKEN.store(!ok, Ordering::Relaxed);
        self.menu_hotkey.set_enabled(ok);
        if !ok {
            log(&format!("hotkey {} is taken by another program", chosen.label()));
        }
    }

    fn try_hotkey(&self, h: Hotkey) -> bool {
        let Some(hwnd) = self.window.handle.hwnd() else { return false };
        let (mods, key) = mods_and_key(h);
        unsafe { winapi::um::winuser::RegisterHotKey(hwnd, HOTKEY_ID, mods, key) != 0 }
    }

    /// Settings ▸ the hotkey: uses `h` from now on (saved) if Windows gives
    /// it; false when another program has it too (the old one is kept).
    #[cfg_attr(not(target_env = "msvc"), allow(dead_code))] // only the new window (MSVC) offers the choice
    pub(super) fn change_hotkey(&self, h: Hotkey) -> bool {
        let old = current_hotkey();
        if h == old && !hotkey_taken() {
            return true;
        }
        if let Some(hwnd) = self.window.handle.hwnd() {
            unsafe { winapi::um::winuser::UnregisterHotKey(hwnd, HOTKEY_ID) };
        }
        if !self.try_hotkey(h) {
            log(&format!("hotkey {} is taken by another program too; keeping {}", h.label(), old.label()));
            if !hotkey_taken() && !self.try_hotkey(old) {
                TAKEN.store(true, Ordering::Relaxed);
                self.menu_hotkey.set_enabled(false);
            }
            return false;
        }
        log(&format!("hotkey changed from {} to {}", old.label(), h.label()));
        set_current_hotkey(h);
        TAKEN.store(false, Ordering::Relaxed);
        self.menu_hotkey.set_enabled(true);
        self.state.borrow_mut().settings.hotkey = h;
        self.save_settings();
        let label = trf!("إظهار البرنامج من أي مكان\t{}", "Show the program from anywhere\t{}", h.label());
        set_menu_text(&self.menu_hotkey, &label);
        set_menu_text(&self.tray_quick, &trf!("فتح نافذة البحث\t{}", "Open quick search\t{}", h.label()));
        self.hotkey_labels_changed();
        true
    }

    /// Alt+Space for the quick search; false when another program has it.
    pub(super) fn register_alt_space(&self) -> bool {
        use winapi::um::winuser::{MOD_ALT, MOD_NOREPEAT, RegisterHotKey, VK_SPACE};
        let Some(h) = self.window.handle.hwnd() else { return false };
        unsafe { RegisterHotKey(h, ALT_SPACE_ID, (MOD_ALT | MOD_NOREPEAT) as u32, VK_SPACE as u32) != 0 }
    }

    pub(super) fn unregister_alt_space(&self) {
        if let Some(h) = self.window.handle.hwnd() {
            unsafe { winapi::um::winuser::UnregisterHotKey(h, ALT_SPACE_ID) };
        }
    }

    /// The global hotkey opens the quick search near the bottom of the
    /// active screen (or closes it); with "hotkey_opens=main" it shows the
    /// window, or hides it if it is already in front, as before 0.8.
    pub(super) fn on_hotkey(&self) {
        use winapi::um::winuser::GetForegroundWindow;
        if self.state.borrow().settings.hotkey_opens == crate::settings::HotkeyOpens::Quick {
            self.show_quick(crate::quick::Anchor::CenterBottom);
            return;
        }
        if self.webui_toggle() {
            return;
        }
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

/// A menu item's text, changed after it was built.
pub(super) fn set_menu_text(item: &nwg::MenuItem, text: &str) {
    use winapi::um::winuser::{MENUITEMINFOW, MIIM_STRING, SetMenuItemInfoW};
    let Some((menu, id)) = item.handle.hmenu_item() else { return };
    let text = super::shell::wide(text);
    let mut info: MENUITEMINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<MENUITEMINFOW>() as UINT;
    info.fMask = MIIM_STRING;
    info.dwTypeData = text.as_ptr() as *mut u16;
    unsafe { SetMenuItemInfoW(menu, id, 0, &info) };
}
