//! The WebView2 control (through wry) filling a Win32 window: the page and
//! its files come from [`super::assets`] through the `instafind://` protocol,
//! messages go to the program's handler, and everything a browser would
//! otherwise allow (other pages, new windows, the context menu, browser
//! shortcuts, developer tools in release builds) is off.

use std::borrow::Cow;
use std::num::NonZeroIsize;

use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL, COREWEBVIEW2_PROCESS_FAILED_KIND,
    COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED, COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED, COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
    ICoreWebView2_3, ICoreWebView2_19, ICoreWebView2Controller4,
};
use webview2_com::{ProcessFailedEventHandler, TrySuspendCompletedHandler};
use windows::core::Interface;
use wry::http::{Request, Response};
use wry::{NewWindowResponse, WebContext, WebView, WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows};

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

// `super::failed_kind` is WebView2's numbering.
const _: () = assert!(COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED.0 == super::failed_kind::BROWSER_PROCESS_EXITED);
const _: () = assert!(COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED.0 == super::failed_kind::RENDER_PROCESS_EXITED);
const _: () = assert!(COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE.0 == super::failed_kind::RENDER_PROCESS_UNRESPONSIVE);

/// How the program answers a thumbnail request: the PNG, or none (404).
pub type Thumb = Box<dyn FnOnce(Option<std::sync::Arc<Vec<u8>>>) + Send>;

/// The HTTP response for `reply`, with the page's security headers.
fn response(reply: assets::Reply) -> Response<Cow<'static, [u8]>> {
    let mut response = Response::builder().status(reply.status).header("Content-Type", reply.mime);
    for (name, value) in assets::HEADERS {
        response = response.header(*name, *value);
    }
    response.body(reply.body).unwrap_or_else(|_| Response::new(Cow::Borrowed(&b""[..])))
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
    /// message of the page, on the UI thread. `on_failed(browser_gone)` is
    /// told when a WebView2 process failed (`browser_gone`: the whole
    /// browser, so only a new `Host` can show the page again; otherwise a
    /// reload can). Both are called from inside WebView2's own event
    /// handlers: they must not drop the `Host`, and a panic in them is
    /// caught here (it must not unwind into WebView2). `dark` picks the
    /// background shown before the page paints.
    pub fn create(
        hwnd: isize,
        lang: Lang,
        token: &str,
        dark: bool,
        on_message: impl Fn(&str, &str) + 'static,
        on_failed: impl Fn(bool) + 'static,
        thumbs: impl Fn(&str, Thumb) + Send + Sync + 'static,
    ) -> Result<Host, String> {
        let parent = Parent(NonZeroIsize::new(hwnd).ok_or("no window")?);
        let page = assets::page(lang, token).ok_or("bad token")?;
        // In the program's own data folder, not next to the exe (which may
        // be in Program Files, where it can't write).
        let data = crate::index::cache_path().with_file_name("WebView2");
        let mut context = WebContext::new(Some(data));
        let background = if dark { (15, 26, 31, 255) } else { (247, 250, 242, 255) };
        let webview = WebViewBuilder::new_with_web_context(&mut context)
            // The page and its files at once; a thumbnail (`/thumb/<key>`)
            // when a worker has made it (`thumbs` answers through `Thumb`).
            .with_asynchronous_custom_protocol(assets::SCHEME.to_string(), move |_id, request: Request<Vec<u8>>, responder| {
                let path = request.uri().path();
                if request.method() == "GET"
                    && let Some(key) = assets::thumb_key(path)
                {
                    let answer: Thumb = Box::new(move |png| {
                        let reply = match png {
                            Some(png) => assets::Reply { status: 200, mime: "image/png", body: Cow::Owned(png.to_vec()) },
                            None => assets::Reply { status: 404, mime: "text/plain; charset=utf-8", body: Cow::Borrowed(b"") },
                        };
                        responder.respond(response(reply));
                    });
                    let _ = crate::crash_log::catch(|| thumbs(key, answer));
                    return;
                }
                responder.respond(response(assets::serve(request.method().as_str(), path, &page)));
            })
            .with_url(assets::START_URL)
            .with_ipc_handler(move |request: Request<String>| {
                let _ = crate::crash_log::catch(|| on_message(request.body(), &request.uri().to_string()));
            })
            // Only the program's own page (exactly its origin, see
            // `assets::own_url`); a link elsewhere goes nowhere, and nothing
            // opens a new window or downloads.
            .with_navigation_handler(|url| assets::own_url(&url))
            .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
            .with_download_started_handler(|_, _| false)
            .with_background_color(background)
            .with_devtools(cfg!(debug_assertions))
            .with_default_context_menus(cfg!(debug_assertions))
            .with_browser_accelerator_keys(false)
            .with_focused(true)
            .build(&parent)
            .map_err(|e| e.to_string())?;
        // A link or file dragged onto the window is not opened (the
        // navigation check would refuse a link anyway).
        if let Ok(c) = webview.controller().cast::<ICoreWebView2Controller4>() {
            let _ = unsafe { c.SetAllowExternalDrop(false) };
        }
        // Only the failures that concern the page (`super::failure`); an
        // event whose kind can't be read is left alone.
        let failed = ProcessFailedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
            if unsafe { args.ProcessFailedKind(&mut kind) }.is_err() {
                return Ok(());
            }
            if let Some(failure) = super::failure(kind.0) {
                let _ = crate::crash_log::catch(|| on_failed(failure == super::Failure::Browser));
            }
            Ok(())
        }));
        let mut token = 0i64;
        unsafe { webview.webview().add_ProcessFailed(&failed, &mut token) }.map_err(|e| e.to_string())?;
        Ok(Host { webview, _context: context })
    }

    /// Loads the page again (after its renderer failed).
    pub fn reload(&self) -> bool {
        self.webview.reload().is_ok()
    }

    /// Runs `script` in the page.
    pub fn eval(&self, script: &str) -> bool {
        self.webview.evaluate_script(script).is_ok()
    }

    /// Puts the keyboard focus in the page.
    pub fn focus(&self) {
        let _ = self.webview.focus();
    }

    /// The window went out of sight (`true`, to the tray) or came back.
    /// Hidden: WebView2 is told the page is not visible (it stops painting,
    /// and only then can it be suspended, [`Host::suspend`]) and to keep as
    /// little memory as it can (`ICoreWebView2_19`, when the installed
    /// runtime has it). Back: visible, normal memory, and resumed if it was
    /// suspended (showing it would resume it anyway). Errors are left
    /// alone: an older runtime lacks some of this.
    pub fn set_hidden(&self, hidden: bool) {
        let controller = self.webview.controller();
        let webview = self.webview.webview();
        let level = if hidden { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW } else { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL };
        unsafe {
            let _ = controller.SetIsVisible(!hidden);
            if let Ok(w) = webview.cast::<ICoreWebView2_19>() {
                let _ = w.SetMemoryUsageTargetLevel(level);
            }
            if !hidden && let Ok(w) = webview.cast::<ICoreWebView2_3>() {
                let mut suspended = windows::core::BOOL::default();
                if w.IsSuspended(&mut suspended).is_ok() && suspended.as_bool() {
                    let _ = w.Resume();
                }
            }
        }
    }

    /// Freezes the hidden page's processes (`ICoreWebView2_3::TrySuspend`;
    /// after `set_hidden(true)`, a visible page can't be). `done(suspended)`
    /// is told how it went, later, on the UI thread. False when it could not
    /// even be asked (an old runtime).
    pub fn suspend(&self, done: impl FnOnce(bool) + 'static) -> bool {
        let Ok(w) = self.webview.webview().cast::<ICoreWebView2_3>() else { return false };
        let handler = TrySuspendCompletedHandler::create(Box::new(move |result, suspended| {
            let _ = crate::crash_log::catch(|| done(result.is_ok() && suspended));
            Ok(())
        }));
        unsafe { w.TrySuspend(&handler) }.is_ok()
    }
}
