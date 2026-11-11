//! The Microsoft Store build's one-time rating question and its permanent
//! "Rate in the Store" menu items (`crate::rating` decides when). In any
//! other copy nothing here is built and every function returns at once.
//!
//! The question is a calm row under the results (the list shrinks from the
//! bottom, so no result moves under the mouse):
//! a sentence and two buttons, «قيّم InstaFind في المتجر» and «ليس الآن».
//! It never takes the focus and is never a message box.

use native_windows_gui as nwg;
use std::cell::Cell;
use winapi::um::winuser::{GetForegroundWindow, IsIconic};

use super::startup::log;
use super::{App, shell, style};
use crate::rating::{self, Moment};

#[derive(Default)]
pub(super) struct RatingBar {
    text: nwg::Label,
    rate: nwg::Button,
    later: nwg::Button,
    /// The row is on screen.
    open: Cell<bool>,
    /// Timer tick of the last keystroke in the search box.
    typed_tick: Cell<u32>,
    /// Timer tick when the window was first on screen this session.
    shown_tick: Cell<Option<u32>>,
    /// Options ▸ "Rate in the Store", and the same in the tray menu.
    menu_item: nwg::MenuItem,
    tray_item: nwg::MenuItem,
}

/// «التقييم في المتجر» / "Rate in the Store".
fn menu_label() -> &'static str {
    tr!("التقييم في المتجر", "Rate in the Store")
}

impl App {
    /// The tray menu's item, right under "About" (packaged builds only).
    pub(super) fn build_rating_tray_item(&mut self) -> Result<(), nwg::NwgError> {
        if rating::is_packaged() {
            nwg::MenuItem::builder().text(menu_label()).parent(&self.tray_menu).build(&mut self.rating.tray_item)?;
        }
        Ok(())
    }

    /// Builds the hidden row and the Options menu's item (after "Support"),
    /// in packaged builds only.
    pub(super) fn build_rating(&mut self) -> Result<(), nwg::NwgError> {
        if !rating::is_packaged() {
            return Ok(());
        }
        let r = &mut self.rating;
        nwg::MenuItem::builder().text(menu_label()).parent(&self.options_menu).build(&mut r.menu_item)?;
        nwg::Label::builder()
            .parent(&self.window)
            .font(Some(&self.font))
            .v_align(nwg::VTextAlign::Center)
            .text(tr!(
                "هل يفيدك InstaFind؟ تقييمك في المتجر يساعد غيرك على إيجاده.",
                "Is InstaFind useful to you? A rating in the Store helps others find it."
            ))
            .build(&mut r.text)?;
        nwg::Button::builder().parent(&self.window).font(Some(&self.font)).text(tr!("قيّم InstaFind في المتجر", "Rate InstaFind in the Store")).build(&mut r.rate)?;
        nwg::Button::builder().parent(&self.window).font(Some(&self.font)).text(tr!("ليس الآن", "Not now")).build(&mut r.later)?;
        r.text.set_visible(false);
        r.rate.set_visible(false);
        r.later.set_visible(false);
        Ok(())
    }

    /// A keystroke in the search box: the question waits until typing stops.
    pub(super) fn note_typing(&self) {
        self.rating.typed_tick.set(self.ticks.get());
    }

    /// One more file opened from the results (window or quick popup).
    /// Counted in packaged builds only, and only until the question was asked.
    pub(super) fn count_file_open(&self) {
        if !rating::is_packaged() {
            return;
        }
        {
            let mut st = self.state.borrow_mut();
            let s = &mut st.settings;
            let n = rating::count_open(s.file_opens, s.rating_asked);
            if n == s.file_opens {
                return;
            }
            s.file_opens = n;
        }
        self.save_settings();
    }

    /// From the timer: shows the row once it is due and the moment is calm.
    /// Classic window only: while the new look (`webui`) is the main window
    /// this window stays hidden, so `visible` is false and nothing is asked
    /// (the question waits, still unasked, for the classic window). Opens
    /// from the new look are not counted yet (TODO with its rating prompt).
    pub(super) fn rating_tick(&self, tick: u32) {
        if self.rating.open.get() || !rating::is_packaged() {
            return;
        }
        let visible = self.window.visible();
        if visible && self.rating.shown_tick.get().is_none() {
            self.rating.shown_tick.set(Some(tick));
        }
        let due = {
            let s = &self.state.borrow().settings;
            rating::rating_due(true, s.rating_asked, s.file_opens)
        };
        if !due {
            return;
        }
        let foreground = self.window.handle.hwnd().is_some_and(|h| unsafe { IsIconic(h) == 0 && GetForegroundWindow() == h });
        let moment = Moment {
            visible,
            foreground,
            modal: !self.window.enabled(),
            ticks_since_typing: tick.wrapping_sub(self.rating.typed_tick.get()),
            ticks_since_shown: self.rating.shown_tick.get().map(|t| tick.wrapping_sub(t)),
        };
        if !rating::show_now(due, moment) {
            return;
        }
        // Asked once: shown counts, whatever the answer (or none).
        self.state.borrow_mut().settings.rating_asked = true;
        self.save_settings();
        log("rating: asking once for a Store rating");
        let r = &self.rating;
        r.open.set(true);
        r.text.set_visible(true);
        r.rate.set_visible(true);
        r.later.set_visible(true);
        self.layout();
    }

    /// Places the row just above `bottom` (where the result list would end),
    /// from `x` in width `w`, when it is on screen; returns where the list
    /// ends now. Only the list's bottom moves, never its top rows.
    pub(super) fn place_rating(&self, x: i32, bottom: i32, w: i32) -> i32 {
        use style::{CTRL_H, GAP};
        let r = &self.rating;
        if !r.open.get() {
            return bottom;
        }
        let y = bottom - CTRL_H;
        let rate_w = style::button_width(&self.font, &r.rate);
        let later_w = style::button_width(&self.font, &r.later);
        let text_w = (w - rate_w - later_w - 2 * GAP).max(0);
        style::place(&r.text.handle, x, y, text_w, CTRL_H);
        style::place(&r.rate.handle, x + text_w + GAP, y, rate_w, CTRL_H);
        style::place(&r.later.handle, x + text_w + GAP + rate_w + GAP, y, later_w, CTRL_H);
        y - GAP
    }

    /// A click on one of the row's buttons; false for other controls.
    pub(super) fn on_rating_click(&self, handle: nwg::ControlHandle) -> bool {
        let r = &self.rating;
        if !r.open.get() || (handle != r.rate && handle != r.later) {
            return false;
        }
        let rate = handle == r.rate;
        log(&format!("rating: answered {}", if rate { "rate" } else { "not now" }));
        r.open.set(false);
        r.text.set_visible(false);
        r.rate.set_visible(false);
        r.later.set_visible(false);
        self.layout();
        self.focus_query();
        if rate {
            self.open_store_review();
        }
        true
    }

    /// Options ▸ / tray ▸ "Rate in the Store"; false for other items.
    pub(super) fn on_rating_item(&self, handle: nwg::ControlHandle) -> bool {
        let r = &self.rating;
        if !rating::is_packaged() || (handle != r.menu_item && handle != r.tray_item) {
            return false;
        }
        self.open_store_review();
        true
    }

    fn open_store_review(&self) {
        if shell::open(&rating::review_uri()) {
            self.set_status(tr!("فُتح متجر مايكروسوفت.", "The Microsoft Store opened."));
        } else {
            log("rating: could not open the Microsoft Store");
            self.set_status(tr!("تعذر فتح متجر Microsoft.", "Could not open the Microsoft Store."));
        }
    }
}
