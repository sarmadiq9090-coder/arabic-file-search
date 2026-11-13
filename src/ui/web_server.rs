//! Options ▸ "Search from phone (home network)…": the window that turns the
//! read-only web server of `crate::web` on and off, and shows the address,
//! the PIN and the direct link. Off by default; the switch, the port, the
//! download switch, the secrets and the home network's id are kept under
//! `HKCU\Software\BahthSari`.
//!
//! With the switch on, the server follows the network ([`web::net`]): a
//! look every few seconds on a thread of its own, and it listens only on
//! the addresses of the home network (the one it was turned on at, while
//! Windows marks it private) or of a network the user allowed in this run.
//! It stops when the PC leaves that network and moves with a new address.
//! While it runs, the tray icon's tip and a check mark on the Options item
//! say so. After two hours with no request from any phone ([`web::IDLE_OFF`])
//! it turns itself off, as if the user had (logged, and said in the window
//! and the new look's Settings).

use native_windows_gui as nwg;
use std::cell::{Cell, RefCell};
use std::net::Ipv4Addr;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use winapi::shared::minwindef::DWORD;
use winapi::shared::winerror::ERROR_SUCCESS;
use winapi::um::winnt::{REG_DWORD, REG_SZ};
use winapi::um::winreg::{RegQueryValueExW, RegSetValueExW};
use winapi::um::winuser::{GWL_EXSTYLE, GetWindowLongW, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowLongW, SetWindowPos, WS_EX_LAYOUTRTL as EX_LAYOUTRTL, WS_EX_LEFTSCROLLBAR, WS_EX_RIGHT, WS_EX_RTLREADING};

use super::lazy::Lazy;
use super::shell::wide;
use super::startup::log;
use super::keys::hotkey_text;
use super::{App, WS_EX_LAYOUTRTL, autostart, title};
use crate::web;
use crate::web::net::{self, Refusal};

const REG_ON: &str = "WebServer";
const REG_PORT: &str = "WebPort";
const REG_DOWNLOAD: &str = "WebDownload";
const REG_TOKEN: &str = "WebToken";
const REG_PIN: &str = "WebPin";
/// The Network List Manager id of the network the user turned it on at.
const REG_HOME: &str = "WebHomeNetwork";
/// How long a click waits for Windows to say which network this is.
const LOOK_TIMEOUT: Duration = Duration::from_secs(3);

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
    /// The switch is on (saved as `WebServer`): the server runs whenever
    /// the network allows it.
    wanted: Cell<bool>,
    /// Looks at the network while the switch is on.
    watcher: RefCell<Option<net::Watcher>>,
    /// What the user allowed in this run of the program.
    session: RefCell<net::Session>,
    /// The LAN addresses the running server was asked to listen on.
    listening: RefCell<Option<Vec<Ipv4Addr>>>,
    /// Addresses a start failed on (the port is taken): not retried until
    /// the network or the user's choice changes.
    failed_for: RefCell<Option<Vec<Ipv4Addr>>>,
    /// Why the server is off although the switch is on.
    refusal: Cell<Option<Refusal>>,
    /// The network seen last, for the window.
    seen: RefCell<Vec<net::Candidate>>,
    /// It turned itself off after [`web::IDLE_OFF`] with no request (until
    /// it is turned on again).
    idle_off: Cell<bool>,
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
    allow_btn: nwg::Button,
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
    ///
    /// Not in the Microsoft Store version: the page serves file names over
    /// plain HTTP on the home network, which Store policy 10.5.4 does not
    /// allow. The item is then never built, so it never matches an event.
    pub(super) fn build_web_ui(&mut self) -> Result<(), nwg::NwgError> {
        if crate::store::is_packaged() {
            return Ok(());
        }
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
        nwg::Button::builder()
            .parent(p)
            .font(f)
            .text(tr!("السماح بهذه الشبكة", "Allow this network"))
            .position((220, 422))
            .size((200, 34))
            .build(&mut w.allow_btn)?;
        w.allow_btn.set_visible(false);
        nwg::Button::builder().parent(p).font(f).text(tr!("إغلاق", "Close")).position((428, 422)).size((120, 34)).build(&mut w.close_btn)?;
        self.web.ui.set(ui);
        Ok(())
    }

    /// Binds the menu item's events and, if the switch was left on, starts
    /// looking at the network (the server starts when the network allows
    /// it). Called once from `build_ui`; the handler is unbound with the others.
    pub(super) fn setup_web(self: &Rc<Self>) -> nwg::EventHandler {
        let weak = Rc::downgrade(self);
        let main = self.bind_main_events(move |evt, _data, handle| {
            let Some(app) = weak.upgrade() else { return };
            match evt {
                nwg::Event::OnMenuItemSelected if handle == app.web.menu_item && app.ensure_web_window() => app.show_web_window(),
                nwg::Event::OnTimerTick if handle == app.timer => {
                    app.check_web_idle();
                    app.poll_web_network();
                    app.sync_web_index();
                }
                // Only a real exit stops it; the close button keeps the
                // program running in the tray.
                nwg::Event::OnWindowClose if handle == app.window && !app.close_hides() => {
                    app.web.watcher.borrow_mut().take();
                    app.stop_web();
                }
                _ => {}
            }
        });
        if get_dword(REG_ON) == Some(1) && !crate::store::is_packaged() {
            self.web.wanted.set(true);
            self.watch_web_network();
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
                nwg::Event::OnButtonClick if handle == app.web.allow_btn => {
                    app.allow_web_network();
                    app.refresh_web_window();
                }
                nwg::Event::OnButtonClick if handle == app.web.close_btn => app.web.window.set_visible(false),
                _ => {}
            }
        }))
    }

    fn show_web_window(&self) {
        // A fresh look, so the window soon shows the network as it is now.
        if let Some(w) = self.web.watcher.borrow().as_ref() {
            w.look_now();
        }
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

    /// Starts the server on `ips` (LAN addresses [`net::plan`] allows) and
    /// 127.0.0.1.
    fn start_web(&self, ips: &[Ipv4Addr]) -> bool {
        if crate::store::is_packaged() {
            return false; // see build_web_ui
        }
        self.stop_web();
        *self.web.failed_for.borrow_mut() = Some(ips.to_vec());
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
        self.set_web_show_system(&shared);
        // The idle time counts from the start.
        shared.note_request(std::time::Instant::now());
        let mut all = ips.to_vec();
        all.push(Ipv4Addr::LOCALHOST);
        match web::Server::start_on(&all, port, shared) {
            Ok(server) => {
                log(&format!("web: listening on {} address(es), port {port}", server.addrs.len()));
                set_dword(REG_PORT, port as u32);
                *self.web.server.borrow_mut() = Some(server);
                *self.web.listening.borrow_mut() = Some(ips.to_vec());
                *self.web.failed_for.borrow_mut() = None;
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

    /// Phone searches follow the "hide system files" option.
    pub(super) fn set_web_show_system(&self, shared: &web::Shared) {
        shared.show_system.store(!self.state.borrow().settings.hide_system, Ordering::Relaxed);
    }

    /// Search from phone: switched on, and running now (for the problem
    /// report).
    pub(super) fn web_switch_state(&self) -> (bool, bool) {
        (self.web.wanted.get(), self.web.server.borrow().is_some())
    }

    /// The running server's shared state, if any.
    pub(super) fn web_running_shared(&self) -> Option<Arc<web::Shared>> {
        self.web.shared.borrow().clone()
    }

    /// Stops the server, if running. Also called when the program closes.
    fn stop_web(&self) {
        *self.web.listening.borrow_mut() = None;
        let stopped = self.web.server.borrow_mut().take();
        if let Some(mut s) = stopped {
            s.stop();
            log("web: stopped");
        }
        self.update_web_indicator();
    }

    fn watch_web_network(&self) {
        let mut w = self.web.watcher.borrow_mut();
        if w.is_none() {
            *w = net::Watcher::start();
            if w.is_none() {
                log("web: could not start watching the network");
            }
        }
    }

    /// The newest look at the network, from the watcher's thread.
    fn poll_web_network(&self) {
        let seen = self.web.watcher.borrow().as_ref().and_then(|w| w.latest());
        if let Some(seen) = seen {
            self.apply_web_network(seen);
        }
    }

    /// Starts, stops or moves the server as the network allows (the rule is
    /// [`net::plan`] and [`net::step`]).
    fn apply_web_network(&self, seen: Vec<net::Candidate>) {
        if !self.web.wanted.get() {
            return;
        }
        let home = get_string(REG_HOME).filter(|h| !h.is_empty());
        let plan = net::plan(&seen, home.as_deref(), &self.web.session.borrow());
        *self.web.seen.borrow_mut() = seen;
        if plan.bind.is_empty() {
            *self.web.failed_for.borrow_mut() = None;
        }
        let listening = self.web.listening.borrow().clone();
        let mut changed = match net::step(listening.as_deref(), &plan) {
            net::Step::Keep => false,
            net::Step::Stop => {
                log("web: this network is not allowed; stopping");
                self.stop_web();
                true
            }
            net::Step::Listen(ips) => {
                if self.web.failed_for.borrow().as_ref() == Some(&ips) {
                    false
                } else {
                    self.start_web(&ips);
                    self.update_web_indicator();
                    true
                }
            }
        };
        let refusal = if plan.bind.is_empty() { plan.refusal } else { None };
        if self.web.refusal.replace(refusal) != refusal {
            if let Some(r) = refusal {
                log(&format!("web: not started: {r:?}"));
            }
            changed = true;
        }
        if changed && self.web.ui.is_built() && self.web.window.visible() {
            self.refresh_web_window();
        }
        if changed {
            self.webui_settings_changed();
        }
    }

    /// The user turned it on or pressed "Allow this network": this network
    /// is allowed for this run, and remembered as home if Windows marks it
    /// private.
    fn allow_web_network(&self) {
        let seen = net::look_within(LOOK_TIMEOUT);
        let (home, session) = net::allow_now(&seen);
        if let Some(id) = home {
            set_string(REG_HOME, &id);
        }
        *self.web.session.borrow_mut() = session;
        *self.web.failed_for.borrow_mut() = None;
        *self.web.error.borrow_mut() = None;
        self.apply_web_network(seen);
    }

    /// The tray icon's tip and the Options item's check mark say whether
    /// the server runs.
    pub(super) fn update_web_indicator(&self) {
        let on = self.web.server.borrow().is_some();
        // Not built in the Microsoft Store version (`build_web_ui`).
        if !self.web.menu_item.handle.blank() {
            self.web.menu_item.set_checked(on);
        }
        let base = format!("{} ({})", title(), hotkey_text());
        let tip = if on { format!("{base} — {}", tr!("البحث من الهاتف يعمل", "Phone search is on")) } else { base };
        self.tray.set_tip(&tip);
        self.webui_settings_changed();
    }

    fn on_web_switch(&self) {
        self.set_web_on(self.web.on_chk.check_state() == nwg::CheckBoxState::Checked);
    }

    /// Turns search from phone on or off (the window's box, or the new
    /// look's Settings switch). Never in the Microsoft Store version.
    pub(super) fn set_web_on(&self, on: bool) {
        if crate::store::is_packaged() {
            return; // see build_web_ui
        }
        self.web.idle_off.set(false);
        if on {
            self.web.wanted.set(true);
            set_dword(REG_ON, 1);
            self.allow_web_network();
            self.watch_web_network();
        } else {
            self.web.wanted.set(false);
            set_dword(REG_ON, 0);
            self.web.watcher.borrow_mut().take();
            self.stop_web();
            // Off means off: every phone's cookie and the old link stop
            // working, also for a request still being answered, and the
            // next time it is turned on there is a new PIN.
            self.rotate_web_secrets();
            *self.web.session.borrow_mut() = net::Session::Nothing;
            *self.web.failed_for.borrow_mut() = None;
            self.web.refusal.set(None);
            *self.web.error.borrow_mut() = None;
        }
        self.refresh_web_window();
        self.webui_settings_changed();
    }

    /// Off after two hours with no request from a phone: an open door
    /// nobody uses is closed (the user turns it on again when needed).
    fn check_web_idle(&self) {
        if self.web.server.borrow().is_none() {
            return;
        }
        let expired = self.web.shared.borrow().as_ref().is_some_and(|s| s.idle_expired(std::time::Instant::now()));
        if expired {
            log(&format!("web: no request for {} minutes; turning phone search off", web::IDLE_OFF.as_secs() / 60));
            self.set_web_on(false);
            self.web.idle_off.set(true);
            self.refresh_web_window();
        }
    }

    /// The new look's «السماح بهذه الشبكة» / "Allow this network".
    #[cfg_attr(not(target_env = "msvc"), allow(dead_code))] // only the new look (MSVC) shows it
    pub(super) fn web_allow_now(&self) {
        if !self.web.wanted.get() {
            return;
        }
        self.allow_web_network();
        self.refresh_web_window();
        self.webui_settings_changed();
    }

    /// The new look's "More phone settings…": the window with the port,
    /// downloads and a new PIN.
    #[cfg_attr(not(target_env = "msvc"), allow(dead_code))] // only the new look (MSVC) shows it
    pub(super) fn open_web_window(self: &Rc<Self>) {
        if !crate::store::is_packaged() && self.ensure_web_window() {
            self.show_web_window();
        }
    }

    /// Search from phone as the new look's Settings shows it; `None` in the
    /// Microsoft Store version, which has none.
    #[cfg_attr(not(target_env = "msvc"), allow(dead_code))] // only the new look (MSVC) shows it
    pub(super) fn phone_settings(&self) -> Option<crate::webui::bridge::PhoneOut> {
        if crate::store::is_packaged() {
            return None;
        }
        let w = &self.web;
        let server = w.server.borrow();
        let wanted = w.wanted.get();
        let mut out = crate::webui::bridge::PhoneOut { on: wanted, running: server.is_some(), idle_off: w.idle_off.get(), ..Default::default() };
        // What the server checks (see refresh_web_window).
        let secrets = if server.is_some() { w.shared.borrow().as_ref().map(|s| s.current_secrets()) } else { None };
        if let (Some(s), Some((token, pin))) = (server.as_ref(), secrets) {
            let lan = s.lan_addrs().next();
            if let Some(a) = lan.or(s.addrs.first()) {
                out.url = format!("http://{a}/");
                out.link = format!("http://{a}/?t={token}");
            }
            out.pin = web::pin_display(&pin);
            if lan.is_none() {
                out.note = tr!(
                    "لم يُعثر على اتصال بشبكة منزلية، يعمل على هذا الحاسوب فقط.",
                    "No home network connection found; it works on this PC only."
                )
                .into();
            }
        } else if wanted {
            let refusal = w.refusal.get();
            let mut note = w.error.borrow().clone().map(|e| e + "\n").unwrap_or_default();
            note.push_str(&off_reason(refusal, &w.seen.borrow(), w.error.borrow().is_some()).replace("\r\n", "\n"));
            out.note = note.trim().to_string();
            out.allow = matches!(refusal, Some(Refusal::NotHome | Refusal::NotPrivate | Refusal::Unknown));
        }
        Some(out)
    }

    fn on_web_download_switch(&self) {
        let on = self.web.download_chk.check_state() == nwg::CheckBoxState::Checked;
        set_dword(REG_DOWNLOAD, on as u32);
        if let Some(s) = self.web.shared.borrow().as_ref() {
            s.allow_download.store(on, Ordering::Relaxed);
        }
    }

    /// New secrets for the running (or next) server; false if the OS
    /// generator failed (the old ones then stay).
    fn rotate_web_secrets(&self) -> bool {
        let Some((t, p)) = new_secrets() else {
            log("web: could not make new secrets");
            return false;
        };
        if let Some(s) = self.web.shared.borrow().as_ref() {
            s.set_secrets(t, p);
        }
        true
    }

    fn new_web_secrets(&self) {
        if !self.rotate_web_secrets() {
            *self.web.error.borrow_mut() = Some(tr!("تعذر إنشاء رمز سري آمن.", "Could not create a secure PIN.").into());
        }
        self.refresh_web_window();
        self.webui_settings_changed();
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
        if !self.web.ui.is_built() {
            return; // (the new look's Settings switch, before the window was ever opened)
        }
        let w = &self.web;
        let server = w.server.borrow();
        let running = server.is_some();
        let wanted = w.wanted.get();
        w.on_chk.set_check_state(if wanted { nwg::CheckBoxState::Checked } else { nwg::CheckBoxState::Unchecked });
        w.port_input.set_readonly(running);
        let refusal = if wanted && !running { w.refusal.get() } else { None };
        w.allow_btn.set_visible(matches!(refusal, Some(Refusal::NotHome | Refusal::NotPrivate | Refusal::Unknown)));
        let mut text = String::new();
        if let Some(e) = w.error.borrow().as_ref() {
            text.push_str(e);
            text.push_str("\r\n\r\n");
        }
        // What the server checks, not a fresh registry read: if saving new
        // secrets failed, `load_secrets` would make up others that the
        // running server does not know.
        let secrets = if running { w.shared.borrow().as_ref().map(|s| s.current_secrets()) } else { None };
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
            _ if wanted => text.push_str(&off_reason(refusal, &w.seen.borrow(), w.error.borrow().is_some())),
            _ if w.idle_off.get() => text.push_str(idle_off_text()),
            _ => text.push_str(tr!("متوقف. فعّل الخيار أعلاه للبدء.", "Off. Tick the box above to start.")),
        }
        w.details.set_text(&text);
    }
}

/// It turned itself off after two hours unused (the window; the page has
/// its own words).
fn idle_off_text() -> &'static str {
    tr!(
        "توقّف البحث من الهاتف وحده بعد ساعتين دون أي طلب من الهاتف، للأمان. فعّل الخيار أعلاه لتشغيله من جديد.",
        "Phone search turned itself off after two hours with no request from a phone, to be safe. Tick the box above to turn it on again."
    )
}

/// Why the switch is on but the server is not running, for the window.
fn off_reason(refusal: Option<Refusal>, seen: &[net::Candidate], failed: bool) -> String {
    let mut text = String::from(match refusal {
        Some(Refusal::NotHome) => tr!("لم يبدأ البحث من الهاتف: هذه ليست شبكتك المنزلية.", "Phone search did not start: this is not your home network."),
        Some(Refusal::NotPrivate) => tr!(
            "لم يبدأ البحث من الهاتف: ويندوز لا يعدّ هذه الشبكة خاصة (منزلية).",
            "Phone search did not start: Windows does not mark this network as private (home)."
        ),
        Some(Refusal::Unknown) => tr!(
            "لم يبدأ البحث من الهاتف: تعذّر على ويندوز تحديد هذه الشبكة.",
            "Phone search did not start: Windows could not tell which network this is."
        ),
        Some(Refusal::NoNetwork) => tr!(
            "لم يبدأ البحث من الهاتف: لا يوجد اتصال بشبكة. يبدأ وحده حين تتصل بشبكتك المنزلية.",
            "Phone search did not start: no network connection. It starts by itself when you join your home network."
        ),
        None if failed => "",
        None => tr!("جارٍ التحقق من الشبكة…", "Checking the network…"),
    });
    let names: Vec<&str> = seen.iter().filter_map(|c| c.network.as_ref()).map(|n| n.name.as_str()).filter(|n| !n.is_empty()).collect();
    if !names.is_empty() && refusal.is_some() {
        text.push_str("\r\n");
        text.push_str(&trf!("الشبكة الحالية: {}", "This network: {}", names.join("، ")));
    }
    if matches!(refusal, Some(Refusal::NotHome | Refusal::NotPrivate | Refusal::Unknown)) {
        text.push_str("\r\n\r\n");
        text.push_str(tr!(
            "إن كانت هذه شبكتك المنزلية فاضغط «السماح بهذه الشبكة». الشبكة التي يعدّها ويندوز عامة تُسمح حتى إغلاق البرنامج فقط.",
            "If this is your home network, press \"Allow this network\". A network Windows marks as public is allowed only until InstaFind closes."
        ));
    }
    text
}
