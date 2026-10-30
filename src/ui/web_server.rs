//! Options ▸ "Search from phone (home network)…": the window that turns the
//! read-only web server of `crate::web` on and off, and shows the address,
//! the PIN and the direct link. Off by default; the switch, the port, the
//! download switch and the secrets are kept under `HKCU\Software\BahthSari`.

use native_windows_gui as nwg;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use winapi::shared::minwindef::DWORD;
use winapi::shared::winerror::ERROR_SUCCESS;
use winapi::um::winnt::{REG_DWORD, REG_SZ};
use winapi::um::winreg::{RegQueryValueExW, RegSetValueExW};
use winapi::um::winuser::{GWL_EXSTYLE, GetWindowLongW, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowLongW, SetWindowPos, WS_EX_LAYOUTRTL as EX_LAYOUTRTL, WS_EX_LEFTSCROLLBAR, WS_EX_RIGHT, WS_EX_RTLREADING};

use super::lazy::Lazy;
use super::shell::wide;
use super::startup::log;
use super::{App, WS_EX_LAYOUTRTL, autostart};
use crate::web;

const REG_ON: &str = "WebServer";
const REG_PORT: &str = "WebPort";
const REG_DOWNLOAD: &str = "WebDownload";
const REG_TOKEN: &str = "WebToken";
const REG_PIN: &str = "WebPin";

/// The server, its window and its menu item: one field of `App`.
#[derive(Default)]
pub struct WebUi {
    menu_item: nwg::MenuItem,
    /// Created the first time the window opens.
    ui: Lazy<WebWindow>,
    shared: RefCell<Option<Arc<web::Shared>>>,
    server: RefCell<Option<web::Server>>,
    /// The last error, shown in the details until the next try.
    error: RefCell<Option<String>>,
}

impl std::ops::Deref for WebUi {
    type Target = WebWindow;
    fn deref(&self) -> &WebWindow {
        &self.ui
    }
}

/// The window's controls.
#[derive(Default)]
pub struct WebWindow {
    window: nwg::Window,
    info: nwg::Label,
    on_chk: nwg::CheckBox,
    download_chk: nwg::CheckBox,
    port_label: nwg::Label,
    port_input: nwg::TextInput,
    details: nwg::TextBox,
    new_btn: nwg::Button,
    close_btn: nwg::Button,
}

// ---------------------------------------------------------------- registry

fn get_dword(name: &str) -> Option<u32> {
    let key = autostart::open(autostart::OWN_KEY, false)?;
    let name = wide(name);
    let (mut value, mut size, mut kind): (DWORD, DWORD, DWORD) = (0, 4, 0);
    let ok = unsafe {
        RegQueryValueExW(key.0, name.as_ptr(), std::ptr::null_mut(), &mut kind, (&mut value as *mut DWORD).cast(), &mut size)
    } == ERROR_SUCCESS as i32;
    (ok && kind == REG_DWORD).then_some(value)
}

fn set_dword(name: &str, value: u32) {
    let Some(key) = autostart::open(autostart::OWN_KEY, true) else { return };
    let name = wide(name);
    unsafe { RegSetValueExW(key.0, name.as_ptr(), 0, REG_DWORD, (&value as *const DWORD).cast(), 4) };
}

fn get_string(name: &str) -> Option<String> {
    let key = autostart::open(autostart::OWN_KEY, false)?;
    let name = wide(name);
    let mut buf = [0u16; 64];
    let (mut size, mut kind): (DWORD, DWORD) = (std::mem::size_of_val(&buf) as DWORD, 0);
    let ok = unsafe {
        RegQueryValueExW(key.0, name.as_ptr(), std::ptr::null_mut(), &mut kind, buf.as_mut_ptr().cast(), &mut size)
    } == ERROR_SUCCESS as i32;
    if !ok || kind != REG_SZ {
        return None;
    }
    let n = (size as usize / 2).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..n]).trim_end_matches('\0').to_string())
}

fn set_string(name: &str, value: &str) {
    let Some(key) = autostart::open(autostart::OWN_KEY, true) else { return };
    let (name, value) = (wide(name), wide(value));
    unsafe { RegSetValueExW(key.0, name.as_ptr(), 0, REG_SZ, value.as_ptr().cast(), (value.len() * 2) as DWORD) };
}

fn saved_port() -> u16 {
    get_dword(REG_PORT).and_then(|p| u16::try_from(p).ok()).filter(|p| *p >= 1024).unwrap_or(web::DEFAULT_PORT)
}

/// The saved token and PIN, or new ones (saved) when missing or malformed.
fn load_secrets() -> Option<(String, String)> {
    let token = get_string(REG_TOKEN).filter(|t| web::valid_token(t));
    let pin = get_string(REG_PIN).filter(|p| web::valid_pin(p));
    match (token, pin) {
        (Some(t), Some(p)) => Some((t, p)),
        _ => new_secrets(),
    }
}

fn new_secrets() -> Option<(String, String)> {
    let (t, p) = (web::new_token()?, web::new_pin()?);
    set_string(REG_TOKEN, &t);
    set_string(REG_PIN, &p);
    Some((t, p))
}

// ---------------------------------------------------------------- UI

fn info_text() -> &'static str {
    tr!(
        "ابحث في ملفات هذا الحاسوب من متصفح هاتفك المتصل بشبكة المنزل (Wi-Fi) نفسها: افتح العنوان أدناه واكتب الرمز السري. \
للقراءة فقط، ولا يقبل إلا أجهزة الشبكة المحلية.\r\n\
قد يسألك جدار حماية ويندوز في أول مرة: اسمح بالشبكات الخاصة فقط، لا العامة. أوقفه حين لا تحتاجه.",
        "Search this PC's files from your phone's browser on the same home network (Wi-Fi): open the address below and type the PIN. \
Read-only, and only devices on the local network are accepted.\r\n\
Windows Firewall may ask the first time: allow private networks only, not public ones. Turn it off when you don't need it."
    )
}

impl App {
    /// Builds the Options menu item. Called once from `build_ui`; the
    /// window is created when it first opens.
    pub(super) fn build_web_ui(&mut self) -> Result<(), nwg::NwgError> {
        nwg::MenuItem::builder()
            .text(tr!("البحث من الهاتف (الشبكة المنزلية)…", "Search from phone (home network)…"))
            .parent(&self.options_menu)
            .build(&mut self.web.menu_item)
    }

    /// Builds the (hidden) window and binds its events, once.
    fn ensure_web_window(self: &Rc<Self>) -> bool {
        self.web.ui.is_built() || self.build_lazily("web", |app| {
            app.build_web_window()?;
            Ok((vec![app.bind_web_window()], Vec::new()))
        })
    }

    fn build_web_window(&self) -> Result<(), nwg::NwgError> {
        let mut ui = WebWindow::default();
        let f = Some(&self.font);
        let w = &mut ui;
        nwg::Window::builder()
            .title(tr!("البحث من الهاتف", "Search from phone"))
            .size((560, 520))
            .center(true)
            .ex_flags(crate::i18n::rtl_only(WS_EX_LAYOUTRTL))
            .flags(nwg::WindowFlags::WINDOW)
            .icon(None)
            .parent(Some(&self.window))
            .build(&mut w.window)?;
        let p = &w.window;
        nwg::Label::builder().parent(p).font(f).text(info_text()).position((12, 10)).size((536, 112)).build(&mut w.info)?;
        nwg::CheckBox::builder()
            .parent(p)
            .font(f)
            .text(tr!("تشغيل البحث من الهاتف", "Turn on search from phone"))
            .position((12, 128))
            .size((330, 28))
            .build(&mut w.on_chk)?;
        nwg::Label::builder().parent(p).font(f).text(tr!("المنفذ:", "Port:")).position((360, 133)).size((70, 24)).build(&mut w.port_label)?;
        nwg::TextInput::builder()
            .parent(p)
            .font(f)
            .text(&saved_port().to_string())
            .limit(5)
            .position((432, 128))
            .size((116, 28))
            .build(&mut w.port_input)?;
        nwg::CheckBox::builder()
            .parent(p)
            .font(f)
            .text(tr!("السماح بتنزيل ملفات النتائج إلى الهاتف", "Allow downloading result files to the phone"))
            .check_state(if get_dword(REG_DOWNLOAD) == Some(1) {
                nwg::CheckBoxState::Checked
            } else {
                nwg::CheckBoxState::Unchecked
            })
            .position((12, 160))
            .size((536, 28))
            .build(&mut w.download_chk)?;
        nwg::TextBox::builder()
            .parent(p)
            .font(f)
            .readonly(true)
            .flags(nwg::TextBoxFlags::VISIBLE | nwg::TextBoxFlags::VSCROLL | nwg::TextBoxFlags::AUTOVSCROLL)
            .position((12, 196))
            .size((536, 214))
            .build(&mut w.details)?;
        // Addresses and links read left to right, also in the Arabic window.
        if let Some(h) = w.details.handle.hwnd() {
            unsafe {
                let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
                let rtl = EX_LAYOUTRTL | WS_EX_RTLREADING | WS_EX_RIGHT | WS_EX_LEFTSCROLLBAR;
                SetWindowLongW(h, GWL_EXSTYLE, (ex & !rtl) as i32);
                SetWindowPos(h, std::ptr::null_mut(), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED);
            }
        }
        nwg::Button::builder()
            .parent(p)
            .font(f)
            .text(tr!("رمز ورابط جديدان", "New PIN and link"))
            .position((12, 422))
            .size((200, 34))
            .build(&mut w.new_btn)?;
        nwg::Button::builder().parent(p).font(f).text(tr!("إغلاق", "Close")).position((428, 422)).size((120, 34)).build(&mut w.close_btn)?;
        self.web.ui.set(ui);
        Ok(())
    }

    /// Binds the menu item's events and starts the server if it was left
    /// on. Called once from `build_ui`; the handler is unbound with the others.
    pub(super) fn setup_web(self: &Rc<Self>) -> nwg::EventHandler {
        let weak = Rc::downgrade(self);
        let main = self.bind_main_events(move |evt, _data, handle| {
            let Some(app) = weak.upgrade() else { return };
            match evt {
                nwg::Event::OnMenuItemSelected if handle == app.web.menu_item && app.ensure_web_window() => app.show_web_window(),
                nwg::Event::OnTimerTick if handle == app.timer => app.sync_web_index(),
                nwg::Event::OnWindowClose if handle == app.window => app.stop_web(),
                _ => {}
            }
        });
        if get_dword(REG_ON) == Some(1) {
            self.start_web();
        }
        main
    }

    /// Binds the window's events, when it is created.
    fn bind_web_window(self: &Rc<Self>) -> nwg::EventHandler {
        let weak = Rc::downgrade(self);
        nwg::full_bind_event_handler(&self.web.window.handle, crate::crash_log::guard_events(move |evt, data, handle| {
            let Some(app) = weak.upgrade() else { return };
            match evt {
                nwg::Event::OnWindowClose if handle == app.web.window => {
                    if let nwg::EventData::OnWindowClose(d) = &data {
                        d.close(false);
                    }
                    app.web.window.set_visible(false);
                }
                nwg::Event::OnButtonClick if handle == app.web.on_chk => app.on_web_switch(),
                nwg::Event::OnButtonClick if handle == app.web.download_chk => app.on_web_download_switch(),
                nwg::Event::OnButtonClick if handle == app.web.new_btn => app.new_web_secrets(),
                nwg::Event::OnButtonClick if handle == app.web.close_btn => app.web.window.set_visible(false),
                _ => {}
            }
        }))
    }

    fn show_web_window(&self) {
        self.refresh_web_window();
        self.web.window.set_visible(true);
        self.web.window.set_focus();
    }

    fn web_shared(&self) -> Option<Arc<web::Shared>> {
        if let Some(s) = self.web.shared.borrow().as_ref() {
            return Some(s.clone());
        }
        let (token, pin) = load_secrets()?;
        // Before the window exists, the switch is as saved.
        let allow = if self.web.ui.is_built() {
            self.web.download_chk.check_state() == nwg::CheckBoxState::Checked
        } else {
            get_dword(REG_DOWNLOAD) == Some(1)
        };
        let s = Arc::new(web::Shared::new(token, pin, allow));
        *self.web.shared.borrow_mut() = Some(s.clone());
        Some(s)
    }

    fn start_web(&self) -> bool {
        self.stop_web();
        let port = if self.web.ui.is_built() {
            self.web.port_input.text().trim().parse::<u16>().ok().filter(|p| *p >= 1024)
        } else {
            Some(saved_port())
        };
        let Some(port) = port else {
            *self.web.error.borrow_mut() =
                Some(tr!("المنفذ يجب أن يكون رقماً بين 1024 و65535.", "The port must be a number from 1024 to 65535.").into());
            return false;
        };
        let Some(shared) = self.web_shared() else {
            *self.web.error.borrow_mut() = Some(tr!("تعذر إنشاء رمز سري آمن.", "Could not create a secure PIN.").into());
            return false;
        };
        shared.set_index(self.state.borrow().index.clone());
        match web::Server::start(port, shared) {
            Ok(server) => {
                log(&format!("web: listening on {} address(es), port {port}", server.addrs.len()));
                set_dword(REG_PORT, port as u32);
                *self.web.server.borrow_mut() = Some(server);
                *self.web.error.borrow_mut() = None;
                true
            }
            Err(e) => {
                log(&format!("web: could not listen on port {port}: {e}"));
                *self.web.error.borrow_mut() = Some(trf!(
                    "تعذر التشغيل على المنفذ {port} ({e}). جرّب منفذاً آخر.",
                    "Could not start on port {port} ({e}). Try another port."
                ));
                false
            }
        }
    }

    /// Stops the server, if running. Also called when the program closes.
    fn stop_web(&self) {
        if let Some(mut s) = self.web.server.borrow_mut().take() {
            s.stop();
            log("web: stopped");
        }
    }

    fn on_web_switch(&self) {
        let on = self.web.on_chk.check_state() == nwg::CheckBoxState::Checked;
        if on {
            let ok = self.start_web();
            set_dword(REG_ON, ok as u32);
        } else {
            self.stop_web();
            *self.web.error.borrow_mut() = None;
            set_dword(REG_ON, 0);
        }
        self.refresh_web_window();
    }

    fn on_web_download_switch(&self) {
        let on = self.web.download_chk.check_state() == nwg::CheckBoxState::Checked;
        set_dword(REG_DOWNLOAD, on as u32);
        if let Some(s) = self.web.shared.borrow().as_ref() {
            s.allow_download.store(on, Ordering::Relaxed);
        }
    }

    fn new_web_secrets(&self) {
        let Some((t, p)) = new_secrets() else { return };
        if let Some(s) = self.web.shared.borrow().as_ref() {
            s.set_secrets(t, p);
        }
        self.refresh_web_window();
    }

    /// Keeps the server on the index the window shows (it is replaced, not
    /// changed, after a load, a scan, a rename or a delete).
    fn sync_web_index(&self) {
        if self.web.server.borrow().is_none() {
            return;
        }
        let (Some(shared), Ok(st)) = (self.web.shared.borrow().clone(), self.state.try_borrow()) else { return };
        if !Arc::ptr_eq(&shared.index(), &st.index) {
            shared.set_index(st.index.clone());
        }
    }

    fn refresh_web_window(&self) {
        let w = &self.web;
        let server = w.server.borrow();
        let running = server.is_some();
        w.on_chk.set_check_state(if running { nwg::CheckBoxState::Checked } else { nwg::CheckBoxState::Unchecked });
        w.port_input.set_readonly(running);
        let mut text = String::new();
        if let Some(e) = w.error.borrow().as_ref() {
            text.push_str(e);
            text.push_str("\r\n\r\n");
        }
        let secrets = running.then(load_secrets).flatten();
        match (server.as_ref(), secrets) {
            (Some(s), Some((token, pin))) => {
                let urls: Vec<String> = s.lan_addrs().map(|a| format!("http://{a}/")).collect();
                if urls.is_empty() {
                    text.push_str(tr!(
                        "لم يُعثر على اتصال بشبكة منزلية، يعمل على هذا الحاسوب فقط",
                        "No home network connection found; it works on this PC only:"
                    ));
                    text.push_str("\r\n");
                    for a in s.addrs.iter() {
                        text.push_str(&format!("http://{a}/\r\n"));
                    }
                } else {
                    text.push_str(tr!("افتح في متصفح الهاتف", "Open in the phone's browser:"));
                    text.push_str("\r\n");
                    for u in &urls {
                        text.push_str(u);
                        text.push_str("\r\n");
                    }
                }
                text.push_str("\r\n");
                text.push_str(tr!("الرمز السري", "PIN:"));
                text.push_str("\r\n");
                text.push_str(&web::pin_display(&pin));
                text.push_str("\r\n\r\n");
                text.push_str(tr!(
                    "أو رابط مباشر دون رمز، لا تشاركه مع أحد",
                    "Or a direct link without the PIN (don't share it):"
                ));
                text.push_str("\r\n");
                let first = s.lan_addrs().next().or(s.addrs.first());
                if let Some(a) = first {
                    text.push_str(&format!("http://{a}/?t={token}\r\n"));
                }
            }
            _ => text.push_str(tr!("متوقف. فعّل الخيار أعلاه للبدء.", "Off. Tick the box above to start.")),
        }
        w.details.set_text(&text);
    }
}
