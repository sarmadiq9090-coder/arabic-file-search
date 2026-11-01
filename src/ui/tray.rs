//! The quick search's place in the rest of the program: the monochrome tray
//! icon (white on a dark taskbar, black on a light one) and its menu, the
//! new items of the Options menu, Alt+Space, the one-time question about
//! the floating box, and starting it all after the main window is up.

use native_windows_gui as nwg;
use std::cell::Cell;

use winapi::shared::minwindef::{LPARAM, UINT, WPARAM};
use winapi::shared::windef::{HGDIOBJ, HICON, HWND};
use winapi::um::wingdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject};
use winapi::um::winuser::{CreateIconIndirect, DestroyIcon, GetDC, GetSystemMetrics, ICONINFO, ReleaseDC, SM_CXSMICON, WM_HOTKEY, WM_SETTINGCHANGE};

use super::keys::{ALT_SPACE_ID, HOTKEY_TEXT};
use super::startup::log;
use super::{App, autostart, surface, title};
use crate::quick::Anchor;
use crate::settings::{FloatingMode, HotkeyOpens};

/// Raw handler id for the main window's theme and Alt+Space messages.
const RAW_ID: usize = 0x1_0F10;

/// The tray icon in both colours, made at run time from
/// [`crate::quick::magnifier_icon`] (the app icon stays as it is).
#[derive(Default)]
pub struct MonoIcons {
    white: Cell<usize>,
    black: Cell<usize>,
}

impl Drop for MonoIcons {
    fn drop(&mut self) {
        for h in [self.white.get(), self.black.get()] {
            if h != 0 {
                unsafe { DestroyIcon(h as HICON) };
            }
        }
    }
}

impl MonoIcons {
    /// The icon for a dark (white glyph) or light (black glyph) taskbar.
    pub fn get(&self, dark_taskbar: bool) -> HICON {
        let cell = if dark_taskbar { &self.white } else { &self.black };
        if cell.get() == 0 {
            cell.set(make_icon(if dark_taskbar { 0xFF_FF_FF } else { 0x00_00_00 }) as usize);
        }
        cell.get() as HICON
    }
}

/// A small-icon-sized icon of the magnifier in `rgb`, with soft edges.
fn make_icon(rgb: u32) -> HICON {
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.clamp(16, 64);
    // Icons take straight (not premultiplied) alpha.
    let pixels: Vec<u32> = crate::quick::magnifier_icon(size, rgb).into_iter().map(|p| if p >> 24 == 0 { 0 } else { (p & 0xFF00_0000) | rgb }).collect();
    unsafe {
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = size;
        info.bmiHeader.biHeight = -size;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let screen = GetDC(std::ptr::null_mut());
        let mut bits: *mut winapi::ctypes::c_void = std::ptr::null_mut();
        let color = CreateDIBSection(screen, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        ReleaseDC(std::ptr::null_mut(), screen);
        if color.is_null() || bits.is_null() {
            return std::ptr::null_mut();
        }
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u32, pixels.len());
        // The AND mask is ignored for 32-bit icons with alpha, but required.
        let row_bytes = (((size + 15) / 16) * 2) as usize;
        let zeros = vec![0u8; row_bytes * size as usize];
        let mask = CreateBitmap(size, size, 1, 1, zeros.as_ptr().cast());
        let mut ii = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&mut ii);
        DeleteObject(color as HGDIOBJ);
        if !mask.is_null() {
            DeleteObject(mask as HGDIOBJ);
        }
        icon
    }
}

/// Options ▸ "Search from anywhere".
#[derive(Default)]
pub struct QuickMenu {
    menu: nwg::Menu,
    fade: nwg::MenuItem,
    hidden: nwg::MenuItem,
    off: nwg::MenuItem,
    reset: nwg::MenuItem,
    hotkey_quick: nwg::MenuItem,
    alt_space: nwg::MenuItem,
    tray_icon: nwg::MenuItem,
}

/// Whether this run started with Windows (hidden, in the background).
fn started_hidden() -> bool {
    std::env::args().any(|a| a == autostart::STARTUP_ARG)
}

impl App {
    /// The Options submenu and the tray menu's new items. Called from
    /// `build_ui` while the app is being built.
    pub(super) fn build_quick_menus(&mut self, settings: &crate::settings::Settings) -> Result<(), nwg::NwgError> {
        let q = &mut self.quick_menu;
        nwg::Menu::builder().text(tr!("البحث من أي مكان", "Search from anywhere")).parent(&self.options_menu).build(&mut q.menu)?;
        let mode = settings.floating_mode;
        let item = |text: &str, check: bool, out: &mut nwg::MenuItem, parent: &nwg::Menu| {
            nwg::MenuItem::builder().text(text).check(check).parent(parent).build(out)
        };
        item(tr!("خانة بحث شفافة فوق الساعة (تظهر عند الاقتراب)", "Search box above the clock (fades in when you come near)"), mode == FloatingMode::Fade, &mut q.fade, &q.menu)?;
        item(tr!("خانة بحث مخفية حتى تصل إلى زاويتها", "Search box hidden until you point at its corner"), mode == FloatingMode::Hidden, &mut q.hidden, &q.menu)?;
        item(tr!("بدون خانة بحث فوق الساعة", "No search box above the clock"), mode == FloatingMode::Off, &mut q.off, &q.menu)?;
        item(tr!("إعادة الخانة إلى مكانها", "Put the box back by the clock"), false, &mut q.reset, &q.menu)?;
        item(
            &trf!("{HOTKEY_TEXT} يفتح نافذة البحث الصغيرة (وإلا النافذة الكاملة)", "{HOTKEY_TEXT} opens the quick search (otherwise the full window)"),
            settings.hotkey_opens == HotkeyOpens::Quick,
            &mut q.hotkey_quick,
            &q.menu,
        )?;
        item(tr!("Alt+Space يفتح نافذة البحث الصغيرة", "Alt+Space opens the quick search"), settings.alt_space, &mut q.alt_space, &q.menu)?;
        item(tr!("أيقونة بجانب الساعة", "Icon in the notification area"), settings.tray_icon, &mut q.tray_icon, &q.menu)?;
        Ok(())
    }

    /// Binds the new menu items and the main window's theme and Alt+Space
    /// messages. Called once from `build_ui`.
    pub(super) fn setup_quick(self: &std::rc::Rc<Self>) -> Result<(nwg::EventHandler, nwg::RawEventHandler), nwg::NwgError> {
        surface::set_app(self);
        let weak = std::rc::Rc::downgrade(self);
        let events = self.bind_main_events(move |evt, _data, handle| {
            let Some(app) = weak.upgrade() else { return };
            if evt != nwg::Event::OnMenuItemSelected {
                return;
            }
            let q = &app.quick_menu;
            if handle == q.fade {
                app.set_floating_mode(FloatingMode::Fade);
            } else if handle == q.hidden {
                app.set_floating_mode(FloatingMode::Hidden);
            } else if handle == q.off {
                app.set_floating_mode(FloatingMode::Off);
            } else if handle == q.reset {
                app.reset_floating_position();
            } else if handle == q.hotkey_quick {
                let quick = !q.hotkey_quick.checked();
                q.hotkey_quick.set_checked(quick);
                app.state.borrow_mut().settings.hotkey_opens = if quick { HotkeyOpens::Quick } else { HotkeyOpens::Main };
                app.save_settings();
            } else if handle == q.alt_space {
                app.toggle_alt_space();
            } else if handle == q.tray_icon {
                let on = !q.tray_icon.checked();
                q.tray_icon.set_checked(on);
                app.tray.set_visibility(on);
                app.state.borrow_mut().settings.tray_icon = on;
                app.save_settings();
            } else if handle == app.tray_quick {
                app.show_quick(app.tray_anchor());
            } else if handle == app.tray_floating {
                let off = app.state.borrow().settings.floating_mode == FloatingMode::Off;
                app.set_floating_mode(if off { FloatingMode::Fade } else { FloatingMode::Off });
            } else if handle == app.tray_settings {
                app.show_menu(&app.options_menu);
            } else if handle == app.tray_about {
                app.show_about();
            }
        });
        let weak = std::rc::Rc::downgrade(self);
        let raw = nwg::bind_raw_event_handler(
            &self.window.handle,
            RAW_ID,
            crate::crash_log::guard_raw(move |_hwnd: HWND, msg: UINT, w: WPARAM, l: LPARAM| {
                let app = weak.upgrade()?;
                match msg {
                    WM_HOTKEY if w as i32 == ALT_SPACE_ID => {
                        app.show_quick(Anchor::CenterBottom);
                        Some(0)
                    }
                    WM_SETTINGCHANGE if surface::is_theme_change(l) => {
                        app.refresh_tray_icon();
                        None
                    }
                    _ => None,
                }
            }),
        )?;
        Ok((events, raw))
    }

    /// Where a popup opened from the tray goes: beside the clock.
    fn tray_anchor(&self) -> Anchor {
        Anchor::Tray { clock_left: surface::taskbar().is_some_and(|t| t.clock_left) }
    }

    /// A left click on the tray icon opens the quick search.
    pub(super) fn on_tray_click(&self) {
        self.show_quick(self.tray_anchor());
    }

    /// The tray menu, with the box's check mark up to date.
    pub(super) fn show_tray_menu(&self) {
        let on = self.state.borrow().settings.floating_mode != FloatingMode::Off;
        self.tray_floating.set_checked(on);
        self.show_menu(&self.tray_menu);
    }

    pub(super) fn refresh_tray_icon(&self) {
        let h = self.mono_icons.get(surface::taskbar_dark());
        if h.is_null() {
            return;
        }
        let mut icon = nwg::Icon::default();
        icon.handle = h as _;
        self.tray.set_icon(&icon);
    }

    /// Switches the floating box's mode, keeping the menus in step.
    pub(super) fn set_floating_mode(&self, mode: FloatingMode) {
        {
            let mut st = self.state.borrow_mut();
            st.settings.floating_mode = mode;
            st.settings.floating_asked = true;
        }
        self.save_settings();
        let q = &self.quick_menu;
        q.fade.set_checked(mode == FloatingMode::Fade);
        q.hidden.set_checked(mode == FloatingMode::Hidden);
        q.off.set_checked(mode == FloatingMode::Off);
        self.tray_floating.set_checked(mode != FloatingMode::Off);
        self.apply_floating();
        if mode == FloatingMode::Off {
            self.set_status(tr!(
                "أُخفيت خانة البحث العائمة. تقدر ترجعها من خيارات ← البحث من أي مكان.",
                "The floating search box is off. Turn it back on in Options ▸ Search from anywhere."
            ));
        }
    }

    fn toggle_alt_space(&self) {
        let on = !self.quick_menu.alt_space.checked();
        if on && !self.register_alt_space() {
            nwg::modal_info_message(
                &self.window,
                title(),
                tr!(
                    "الاختصار Alt+Space مستخدم من برنامج آخر، فلا يمكن استخدامه هنا. يبقى Ctrl+Alt+F متاحاً.",
                    "Alt+Space is already taken by another program, so it can't be used here. Ctrl+Alt+F still works."
                ),
            );
            return;
        }
        if !on {
            self.unregister_alt_space();
        }
        self.quick_menu.alt_space.set_checked(on);
        self.state.borrow_mut().settings.alt_space = on;
        self.save_settings();
    }

    fn show_about(&self) {
        nwg::modal_info_message(
            &self.window,
            tr!("حول بحث سريع", "About InstaFind"),
            &trf!(
                "بحث سريع (InstaFind) — الإصدار {}\nبحث فوري عن الملفات وداخل المستندات، بالعربية والإنجليزية.\n\nاضغط {HOTKEY_TEXT} من أي مكان لتبحث.\n{}",
                "InstaFind — version {}\nInstant search for files and inside documents, in Arabic and English.\n\nPress {HOTKEY_TEXT} anywhere to search.\n{}",
                env!("CARGO_PKG_VERSION"),
                crate::feedback::REPO_URL
            ),
        );
    }

    /// Timer work of the quick search, from the main window's tick: the
    /// floating box shortly after start (so the main window's start-up does
    /// not wait for it), the catalog a few seconds later, and the one-time
    /// question once the window is on screen.
    pub(super) fn quick_tick(&self, tick: u32) {
        let (mode, asked, alt_space, hotkey) = {
            let s = &self.state.borrow().settings;
            (s.floating_mode, s.floating_asked, s.alt_space, s.hotkey_opens)
        };
        if tick == 2 {
            if mode != FloatingMode::Off && (asked || started_hidden()) {
                self.apply_floating();
            }
            if alt_space && !self.register_alt_space() {
                log("hotkey Alt+Space is taken by another program");
                self.quick_menu.alt_space.set_checked(false);
                self.state.borrow_mut().settings.alt_space = false;
            }
        }
        // Warm the catalog when anything can open the quick search.
        let used = mode != FloatingMode::Off || alt_space || hotkey == HotkeyOpens::Quick;
        if tick == 25 && used {
            self.start_catalog();
        }
        if !asked && tick >= 14 && self.crash_checked.get() && self.window.visible() {
            self.ask_floating_once();
        }
    }

    fn ask_floating_once(&self) {
        self.state.borrow_mut().settings.floating_asked = true;
        self.save_settings();
        let params = nwg::MessageParams {
            title: title(),
            content: tr!(
                "هل تريد خانة البحث الصغيرة فوق الساعة؟ تقدر تغيّر هذا لاحقًا من الإعدادات.",
                "Would you like the small search box above the clock? You can change this later in Options."
            ),
            buttons: nwg::MessageButtons::YesNo,
            icons: nwg::MessageIcons::Question,
        };
        let yes = nwg::modal_message(&self.window, &params) == nwg::MessageChoice::Yes;
        log(&format!("first run: floating box {}", if yes { "on" } else { "off" }));
        self.set_floating_mode(if yes { FloatingMode::Fade } else { FloatingMode::Off });
        if yes {
            self.set_status(&trf!(
                "خانة البحث فوق الساعة: انقرها أو اضغط {HOTKEY_TEXT}. اسحبها من حافتها العليا لتنقلها.",
                "The search box is above the clock: click it or press {HOTKEY_TEXT}. Drag its top edge to move it."
            ));
        }
    }
}
