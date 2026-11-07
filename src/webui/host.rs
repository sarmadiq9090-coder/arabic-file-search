//! The WebView2 control (through wry) filling a Win32 window: the page and
//! its files come from [`super::assets`] through the `instafind://` protocol,
//! messages go to the program's handler, and everything a browser would
//! otherwise allow (other pages, new windows, the context menu, browser
//! shortcuts, developer tools in release builds) is off.

use std::borrow::Cow;
use std::num::NonZeroIsize;

use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};
use wry::http::{Request, Response};
use wry::{MemoryUsageLevel, NewWindowResponse, WebContext, WebView, WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows};

use super::assets;
use crate::i18n::Lang;

/// The version of the installed WebView2 runtime, or `None` when there is
/// none (then the classic window is used).
pub fn runtime_version() -> Option<String> {
    use webview2_com::Microsoft::Web::WebView2::Win32::GetAvailableCoreWebView2BrowserVersionString;
    let mut info = windows::core::PWSTR::null();
    unsafe { GetAvailableCoreWebView2BrowserVersionString(windows::core::PCWSTR::null(), &mut info) }.ok()?;
    if info.is_null() {
        return None;
    }
    Some(webview2_com::take_pwstr(info)).filter(|v| !v.trim().is_empty())
}

/// The page inside a window.
pub struct Host {
    webview: WebView,
    // Kept for as long as the control lives.
    _context: WebContext,
}

/// A Win32 window handle as wry wants it.
struct Parent(NonZeroIsize);

impl HasWindowHandle for Parent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = RawWindowHandle::Win32(Win32WindowHandle::new(self.0));
        // The window outlives the control: `Host` is dropped before it.
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

impl Host {
    /// Fills window `hwnd` with the page in `lang`. `token` is this
    /// launch's token (hex); `on_message(body, source_url)` gets every
    /// message of the page, on the UI thread. `dark` picks the background
    /// shown before the page paints.
    pub fn create(hwnd: isize, lang: Lang, token: &str, dark: bool, on_message: impl Fn(&str, &str) + 'static) -> Result<Host, String> {
        let parent = Parent(NonZeroIsize::new(hwnd).ok_or("no window")?);
        let page = assets::page(lang, token).ok_or("bad token")?;
        // In the program's own data folder, not next to the exe (which may
        // be in Program Files, where it can't write).
        let data = crate::index::cache_path().with_file_name("WebView2");
        let mut context = WebContext::new(Some(data));
        let background = if dark { (15, 26, 31, 255) } else { (247, 250, 242, 255) };
        let webview = WebViewBuilder::new_with_web_context(&mut context)
            .with_custom_protocol(assets::SCHEME.to_string(), move |_id, request: Request<Vec<u8>>| {
                let reply = assets::serve(request.method().as_str(), request.uri().path(), &page);
                let mut response = Response::builder().status(reply.status).header("Content-Type", reply.mime);
                for (name, value) in assets::HEADERS {
                    response = response.header(*name, *value);
                }
                response.body(reply.body).unwrap_or_else(|_| Response::new(Cow::Borrowed(&b""[..])))
            })
            .with_url(assets::START_URL)
            .with_ipc_handler(move |request: Request<String>| on_message(request.body(), &request.uri().to_string()))
            // Only the program's own page; a link elsewhere goes nowhere.
            .with_navigation_handler(|url| url.starts_with(assets::ORIGIN) || url.starts_with(assets::START_URL))
            .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
            .with_background_color(background)
            .with_devtools(cfg!(debug_assertions))
            .with_default_context_menus(cfg!(debug_assertions))
            .with_browser_accelerator_keys(false)
            .with_focused(true)
            .build(&parent)
            .map_err(|e| e.to_string())?;
        Ok(Host { webview, _context: context })
    }

    /// Runs `script` in the page.
    pub fn eval(&self, script: &str) -> bool {
        self.webview.evaluate_script(script).is_ok()
    }

    /// Puts the keyboard focus in the page.
    pub fn focus(&self) {
        let _ = self.webview.focus();
    }

    /// Lowers the page's memory use while its window is hidden.
    pub fn set_idle(&self, idle: bool) {
        let level = if idle { MemoryUsageLevel::Low } else { MemoryUsageLevel::Normal };
        let _ = self.webview.set_memory_usage_level(level);
    }
}
