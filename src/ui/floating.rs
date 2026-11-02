//! The floating search box: a small, rounded, translucent pill just above
//! the taskbar beside the clock (`crate::quick::default_box_rect`). A click
//! opens the quick search right above it; the grip along its top drags it
//! anywhere (the place is saved per monitor); its right-click menu resets it.
//!
//! It is a layered, never-activated tool window drawn with per-pixel alpha.
//! In the "fade" mode it is nearly invisible until the cursor comes close,
//! in the "hidden" mode fully invisible until the cursor reaches it. It
//! steps aside for full-screen programs, games, videos and presentations,
//! and for an auto-hide taskbar that is tucked away. The cursor is polled by
//! a slow timer (a faster one only while fading), so it costs nothing idle.

use std::cell::{Cell, RefCell};
use std::time::Instant;

use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
use winapi::shared::windef::{HWND, POINT, RECT, SIZE};
use winapi::um::wingdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, FW_NORMAL};
use winapi::um::winuser::*;

use super::App;
use super::quick::{nwg_scale, point_of};
use super::shell;
use super::startup::log;
use super::surface::{self, Canvas, Font, KIND_FLOATING};
use crate::quick::{self, Anchor, Palette, Rect, Taskbar, px};
use crate::settings::{FloatPos, FloatingMode};

/// Polls the cursor this often while the box rests.
const POLL_MS: u32 = 120;
/// Animation frames while fading.
const FRAME_MS: u32 = 16;
/// Checks less often while a full-screen program hides the box.
const ASLEEP_MS: u32 = 1000;
const TIMER_POLL: usize = 1;
/// Re-places the box a moment after the screen or the taskbar changed.
const TIMER_REPLACE: usize = 2;
/// The grip: this many logical px along the top drag the box.
const GRIP_H: i32 = 8;

/// The box's state, one field of `App`.
#[derive(Default)]
pub struct Floating {
    hwnd: Cell<usize>,
    rect: Cell<Rect>,
    alpha: Cell<f32>,
    last_step: Cell<Option<Instant>>,
    /// Stays fully visible until then (the hotkey was used, the popup closed).
    awake_until: Cell<Option<Instant>>,
    interval: Cell<u32>,
    /// Hidden for a full-screen program or a tucked-away taskbar.
    asleep: Cell<bool>,
    /// A full-screen program, game, video or presentation is in front
    /// (checked about once a second).
    full_screen: Cell<bool>,
    ticks: Cell<u32>,
    dragging: Cell<bool>,
    /// The user dragged it: it no longer follows the taskbar.
    custom: Cell<bool>,
    autohide: Cell<bool>,
    taskbar: Cell<Option<Taskbar>>,
    monitor: Cell<Rect>,
    /// `RegisterWindowMessage("TaskbarCreated")`: Explorer restarted.
    taskbar_created: Cell<u32>,
    /// The box at full look (premultiplied), redrawn on changes only.
    image: RefCell<Option<Canvas>>,
}

fn rtl() -> bool {
    !crate::i18n::is_en()
}

impl App {
    pub(super) fn floating_hwnd(&self) -> HWND {
        self.floating.hwnd.get() as HWND
    }

    /// Shows, hides or removes the box after its setting changed (or at start).
    pub(super) fn apply_floating(&self) {
        let mode = self.state.borrow().settings.floating_mode;
        if mode == FloatingMode::Off {
            self.destroy_floating();
            return;
        }
        if self.floating.hwnd.get() == 0 {
            let started = Instant::now();
            let ex = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE;
            let Some(hwnd) = surface::create_window(KIND_FLOATING, ex, "") else {
                log("floating box: could not create the window");
                return;
            };
            self.floating.hwnd.set(hwnd as usize);
            let name = shell::wide("TaskbarCreated");
            self.floating.taskbar_created.set(unsafe { RegisterWindowMessageW(name.as_ptr()) });
            self.floating.alpha.set(0.0);
            self.floating_place();
            unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
            log(&format!("floating box built in {} ms", started.elapsed().as_millis()));
        }
        // Wake it briefly so the user sees where it is.
        self.floating.awake_until.set(Some(Instant::now() + std::time::Duration::from_millis(1500)));
        self.floating_timer(FRAME_MS);
    }

    pub(super) fn destroy_floating(&self) {
        let h = self.floating_hwnd();
        if !h.is_null() {
            unsafe { DestroyWindow(h) };
            self.floating.hwnd.set(0);
            self.floating.interval.set(0);
            self.floating.asleep.set(false);
            *self.floating.image.borrow_mut() = None;
        }
    }

    /// The box stays fully visible while the popup is open, and a moment after.
    pub(super) fn wake_floating(&self, open: bool) {
        if self.floating.hwnd.get() == 0 {
            return;
        }
        let ms = if open { 1500 } else { 700 };
        self.floating.awake_until.set(Some(Instant::now() + std::time::Duration::from_millis(ms)));
        self.floating_timer(FRAME_MS);
    }

    fn floating_timer(&self, ms: u32) {
        let h = self.floating_hwnd();
        if !h.is_null() && self.floating.interval.replace(ms) != ms {
            unsafe { SetTimer(h, TIMER_POLL, ms, None) };
        }
    }

    /// Puts the box where the user left it, or by the clock.
    fn floating_place(&self) {
        let scale = nwg_scale();
        let saved = self.state.borrow().settings.floating_pos;
        let restored = saved.and_then(|p| quick::restore_box_rect(&p, &surface::monitors(), scale));
        let taskbar = surface::taskbar();
        let monitor = match &taskbar {
            Some(t) => surface::monitor_at(t.rect.center().0, t.rect.center().1).0,
            None => surface::primary_monitor().0,
        };
        self.floating.custom.set(restored.is_some());
        self.floating.taskbar.set(taskbar);
        self.floating.monitor.set(monitor);
        self.floating.autohide.set(surface::taskbar_autohide());
        let r = restored.unwrap_or_else(|| quick::default_box_rect(&monitor, taskbar.as_ref(), scale));
        self.floating.rect.set(r);
        self.floating_render();
        self.floating_update(true);
    }

    /// Draws the box at full look: the tinted rounded pill with a fine
    /// outline, the grip, the magnifier, "Search..." and the hotkey hint.
    fn floating_render(&self) {
        let r = self.floating.rect.get();
        let (w, h) = (r.width(), r.height());
        let Some(mut canvas) = Canvas::new(w, h) else { return };
        let s = nwg_scale();
        let dark = surface::taskbar_dark() && !surface::high_contrast();
        let p = Palette::new(dark);
        let radius = h as f32 / 2.0;
        let fill = quick::premul(p.box_tint, p.box_alpha);
        let outline = if dark { quick::premul(0xFF_FF_FF, 46) } else { quick::premul(0x00_00_00, 30) };
        let mut base = vec![0u32; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let outer = quick::rounded_coverage(x, y, w, h, radius);
                if outer == 0 {
                    continue;
                }
                let inner = quick::rounded_coverage(x - 1, y - 1, w - 2, h - 2, radius - 1.0);
                let ring = outer.saturating_sub(inner);
                base[(y * w + x) as usize] = quick::over(quick::fade(outline, ring), quick::fade(fill, inner));
            }
        }
        // The grip: a short bar centred along the top.
        let grip = Rect::at((w - px(18, s)) / 2, px(3, s), px(18, s), px(2, s).max(2));
        surface::fill_rounded(&mut base, w, &grip, 1.0, quick::premul(p.muted, 110));

        let ui = surface::ui_family();
        let text_font = Font::new(ui, px(13, s), FW_NORMAL, true);
        let hint_font = Font::new(ui, px(11, s), FW_NORMAL, true);
        let glyph_font = Font::new(surface::icon_family().unwrap_or(ui), px(14, s), FW_NORMAL, true);
        let rtl = rtl();
        let icon_w = px(20, s);
        let ix = quick::mirror_x(px(12, s), icon_w, w, rtl);
        let hint_w = px(70, s);
        let hx = quick::mirror_x(w - px(14, s) - hint_w, hint_w, w, rtl);
        let text_x = px(38, s);
        let text_w = (w - text_x - hint_w - px(18, s)).max(1);
        let tx = quick::mirror_x(text_x, text_w, w, rtl);
        // The hint is fainter than the text: halfway to the tint.
        let faint = mix(p.muted, p.box_tint);
        surface::compose(&mut canvas, &base, false, &|dc| {
            surface::draw_glyph(dc, &glyph_font, p.text, quick::glyph::SEARCH, &Rect::at(ix, 0, icon_w, h));
            surface::draw_text(dc, &text_font, p.muted, tr!("ابحث...", "Search..."), &Rect::at(tx, 0, text_w, h), rtl, false);
            // Always left-to-right, at the far end.
            surface::draw_text(dc, &hint_font, faint, super::keys::HOTKEY_TEXT, &Rect::at(hx, 0, hint_w, h), false, !rtl);
        });
        *self.floating.image.borrow_mut() = Some(canvas);
    }

    /// Shows the image at the current opacity (and, unless the user is
    /// dragging it, at its place).
    fn floating_update(&self, move_it: bool) {
        let hwnd = self.floating_hwnd();
        let image = self.floating.image.borrow();
        let Some(c) = image.as_ref() else { return };
        let r = self.floating.rect.get();
        let pos = POINT { x: r.left, y: r.top };
        let size = SIZE { cx: c.w, cy: c.h };
        let src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER,
            BlendFlags: 0,
            SourceConstantAlpha: quick::alpha_byte(self.floating.alpha.get()),
            AlphaFormat: AC_SRC_ALPHA,
        };
        let at = if move_it && !self.floating.dragging.get() { &pos as *const POINT as *mut POINT } else { std::ptr::null_mut() };
        unsafe {
            UpdateLayeredWindow(hwnd, std::ptr::null_mut(), at, &size as *const SIZE as *mut SIZE, c.dc, &src as *const POINT as *mut POINT, 0, &blend as *const BLENDFUNCTION as *mut BLENDFUNCTION, ULW_ALPHA);
        }
    }

    /// One tick: steps aside for full-screen programs and a hidden taskbar,
    /// then fades towards what the cursor's distance asks for.
    fn floating_tick(&self) {
        let hwnd = self.floating_hwnd();
        let t = self.floating.ticks.get().wrapping_add(1);
        self.floating.ticks.set(t);
        let now = Instant::now();
        let s = nwg_scale();
        if t % 8 == 1 {
            self.floating.autohide.set(surface::taskbar_autohide());
        }
        if t % 8 == 1 || self.floating.asleep.get() {
            self.floating.full_screen.set(surface::busy_full_screen());
        }
        let mut asleep = self.floating.full_screen.get();
        if !asleep
            && self.floating.autohide.get()
            && !self.floating.custom.get()
            && let (Some(tb), Some(now_rect)) = (self.floating.taskbar.get(), surface::taskbar_window_rect())
        {
            asleep = !tb.is_out(&now_rect, &self.floating.monitor.get(), s);
        }
        // The popup keeps the box awake whatever else is going on.
        let popup = self.quick_visible();
        asleep &= !popup;
        if asleep != self.floating.asleep.replace(asleep) {
            unsafe { ShowWindow(hwnd, if asleep { SW_HIDE } else { SW_SHOWNOACTIVATE }) };
        }
        if asleep {
            self.floating.alpha.set(0.0);
            self.floating.last_step.set(None);
            self.floating_timer(if self.floating.autohide.get() { POLL_MS } else { ASLEEP_MS });
            return;
        }
        let mut pt = POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut pt) };
        let distance = self.floating.rect.get().distance(pt.x, pt.y);
        let awake = popup || self.floating.dragging.get() || self.floating.awake_until.get().is_some_and(|u| u > now);
        let mode = self.state.borrow().settings.floating_mode;
        let target = quick::target_alpha(mode, distance, s, awake);
        let dt = self.floating.last_step.replace(Some(now)).map_or(FRAME_MS as f32, |l| now.duration_since(l).as_secs_f32() * 1000.0);
        let old = self.floating.alpha.get();
        let new = quick::step_alpha(old, target, dt.min(100.0));
        if quick::alpha_byte(new) != quick::alpha_byte(old) {
            self.floating.alpha.set(new);
            self.floating_update(false);
        } else {
            self.floating.alpha.set(new);
        }
        self.floating_timer(if (new - target).abs() > f32::EPSILON { FRAME_MS } else { POLL_MS });
    }

    /// Re-places the box shortly, after the screen, the DPI or the taskbar changed.
    fn floating_replace_soon(&self) {
        let h = self.floating_hwnd();
        if !h.is_null() {
            unsafe { SetTimer(h, TIMER_REPLACE, 400, None) };
        }
    }

    /// The box's window procedure; `None` for the default handling.
    pub(super) fn floating_message(&self, hwnd: HWND, msg: UINT, w: WPARAM, l: LPARAM) -> Option<LRESULT> {
        if msg != 0 && msg == self.floating.taskbar_created.get() {
            self.floating_replace_soon();
            return Some(0);
        }
        match msg {
            WM_TIMER if w == TIMER_POLL => {
                self.floating_tick();
                Some(0)
            }
            WM_TIMER if w == TIMER_REPLACE => {
                unsafe { KillTimer(hwnd, TIMER_REPLACE) };
                self.floating_place();
                Some(0)
            }
            WM_MOUSEACTIVATE => Some(MA_NOACTIVATE as LRESULT),
            WM_NCHITTEST => {
                let (_, y) = point_of(l);
                let mut wr = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                let top = if unsafe { GetWindowRect(hwnd, &mut wr) } != 0 { wr.top } else { self.floating.rect.get().top };
                Some(if y < top + px(GRIP_H, nwg_scale()) { HTCAPTION } else { HTCLIENT } as LRESULT)
            }
            WM_MOUSEMOVE => {
                self.floating_timer(FRAME_MS);
                Some(0)
            }
            WM_LBUTTONUP => {
                self.show_quick(Anchor::Box(self.floating.rect.get()));
                Some(0)
            }
            WM_RBUTTONUP | WM_NCRBUTTONUP | WM_CONTEXTMENU => {
                self.floating_menu();
                Some(0)
            }
            WM_NCLBUTTONDBLCLK => Some(0),
            WM_ENTERSIZEMOVE => {
                self.floating.dragging.set(true);
                None
            }
            WM_EXITSIZEMOVE => {
                self.floating.dragging.set(false);
                self.floating_dropped(hwnd);
                None
            }
            WM_DISPLAYCHANGE | WM_DPICHANGED => {
                self.floating_replace_soon();
                Some(0)
            }
            WM_SETTINGCHANGE => {
                // The work area (the taskbar moved or resized), the theme, taskbar settings.
                self.floating_replace_soon();
                None
            }
            _ => None,
        }
    }

    /// Saves where the box was dragged to, with the monitor it is on.
    fn floating_dropped(&self, hwnd: HWND) {
        let mut wr = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        if unsafe { GetWindowRect(hwnd, &mut wr) } == 0 {
            return;
        }
        let r = surface::rect(&wr);
        if r == self.floating.rect.get() {
            return;
        }
        let (mon, _) = surface::monitor_at(r.center().0, r.center().1);
        let r = r.clamp_into(&mon);
        self.floating.rect.set(r);
        self.floating.custom.set(true);
        self.state.borrow_mut().settings.floating_pos = Some(FloatPos { monitor: [mon.left, mon.top, mon.right, mon.bottom], x: r.left, y: r.top });
        self.save_settings();
        self.floating_update(true);
    }

    /// Back beside the clock.
    pub(super) fn reset_floating_position(&self) {
        self.state.borrow_mut().settings.floating_pos = None;
        self.save_settings();
        if self.floating.hwnd.get() != 0 {
            self.floating_place();
            self.wake_floating(false);
        }
    }

    fn floating_menu(&self) {
        let items: [(usize, &str); 4] = [
            (1, tr!("فتح نافذة البحث\tCtrl+Alt+F", "Open quick search\tCtrl+Alt+F")),
            (2, tr!("النافذة الكاملة", "Full window")),
            (3, tr!("إعادة المربع إلى مكانه", "Reset position")),
            (4, tr!("إخفاء المربع العائم", "Hide the floating box")),
        ];
        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            return;
        }
        for (id, text) in items {
            if id == 3 {
                unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null()) };
            }
            let t = shell::wide(text);
            let flags = if id == 3 && !self.floating.custom.get() { MF_STRING | MF_GRAYED } else { MF_STRING };
            unsafe { AppendMenuW(menu, flags, id, t.as_ptr()) };
        }
        let mut pt = POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut pt) };
        // The box never takes the focus, so the (hidden) main window owns
        // the menu, as for the tray icon.
        let owner = self.window.handle.hwnd().unwrap_or(std::ptr::null_mut());
        let align = if rtl() { TPM_LAYOUTRTL | TPM_RIGHTALIGN } else { TPM_LEFTALIGN };
        let chosen = unsafe {
            SetForegroundWindow(owner);
            let c = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | align, pt.x, pt.y, 0, owner, std::ptr::null());
            PostMessageW(owner, WM_NULL, 0, 0);
            DestroyMenu(menu);
            c
        };
        match chosen {
            1 => self.show_quick(Anchor::Box(self.floating.rect.get())),
            2 => self.show_window(),
            3 => self.reset_floating_position(),
            4 => self.set_floating_mode(FloatingMode::Off),
            _ => {}
        }
    }
}

/// The colour halfway between two `0x00RRGGBB` colours.
fn mix(a: u32, b: u32) -> u32 {
    let ch = |shift: u32| ((((a >> shift) & 0xFF) + ((b >> shift) & 0xFF)) / 2) << shift;
    ch(16) | ch(8) | ch(0)
}
