//! What the main window shows besides results (0.9): the files opened
//! lately while the search box is empty, and a row of buttons with what to
//! try when a search finds nothing (`crate::suggest` decides which).

use native_windows_gui as nwg;
use std::cell::{Cell, RefCell};

use super::{App, Row, shell, style};
use crate::history::History;
use crate::index::Index;
use crate::suggest::{self, Situation, Suggestion};

/// At most this many suggestion buttons (one per kind of suggestion).
const BUTTONS: usize = 5;

/// The row of suggestion buttons above the result list, hidden while there
/// is nothing to suggest.
#[derive(Default)]
pub(super) struct SuggestBar {
    /// The plain sentence above the buttons ("No files match your search. Try:").
    lead: nwg::Label,
    buttons: [nwg::Button; BUTTONS],
    /// What each shown button does, in order.
    list: RefCell<Vec<Suggestion>>,
    /// Widths of the buttons fitted to their texts, measured when shown.
    widths: Cell<[i32; BUTTONS]>,
}

/// The files opened lately that are still in `index`, as result rows.
pub(super) fn recent_rows(history: &History, index: &Index) -> Vec<Row> {
    let probe = |p: &str| index.find(p).map(|i| i as u32);
    suggest::pick_recent(history.recent_opens(), suggest::RECENT_ROWS, usize::MAX, probe)
        .into_iter()
        .map(|(_, entry)| Row { entry, snippet: None, matches: None })
        .collect()
}

impl App {
    pub(super) fn build_suggest_bar(&mut self) -> Result<(), nwg::NwgError> {
        nwg::Label::builder().parent(&self.window).font(Some(&self.font)).text("").build(&mut self.suggest.lead)?;
        self.suggest.lead.set_visible(false);
        for b in self.suggest.buttons.iter_mut() {
            nwg::Button::builder().parent(&self.window).font(Some(&self.font)).text("").build(b)?;
            b.set_visible(false);
        }
        Ok(())
    }

    /// With the search box empty, lists the files opened lately (unless the
    /// list already shows something).
    pub(super) fn show_recent_files(&self) {
        if !self.query.text().trim().is_empty() || self.content_mode() {
            return;
        }
        let mut st = self.state.borrow_mut();
        if !st.rows.is_empty() || st.content.is_some() {
            return;
        }
        let index = st.index.clone();
        st.rows = recent_rows(&st.history, &index);
        st.rows_index = index;
        st.shown_query = None;
        let any = !st.rows.is_empty();
        drop(st);
        if any {
            self.clear_selection();
            self.refresh_list();
        }
    }

    /// The suggestions for the name search of `text` that found nothing.
    pub(super) fn suggest_for_names(&self, text: &str, did_you_mean: Option<String>) {
        let scoped = !self.state.borrow().scope.is_empty();
        let situation = Situation {
            did_you_mean,
            kind_filtered: self.kind() != 0,
            scoped,
            contents: self.parse_query(text).has_text_criteria(),
            web: true,
        };
        self.show_suggestions(suggest::suggestions(text, &situation));
    }

    /// The suggestions for the document search of `text` that found nothing.
    pub(super) fn suggest_for_content(&self, text: &str) {
        let scoped = !self.state.borrow().scope.is_empty();
        let situation = Situation { kind_filtered: self.kind() != 0, scoped, web: true, ..Situation::default() };
        self.show_suggestions(suggest::suggestions(text, &situation));
    }

    /// Shows the buttons of `list` (none hides the row) and lays out again
    /// when the row appears or goes.
    pub(super) fn show_suggestions(&self, list: Vec<Suggestion>) {
        if *self.suggest.list.borrow() == list {
            return;
        }
        let was = !self.suggest.list.borrow().is_empty();
        let mut widths = [0; BUTTONS];
        for (i, b) in self.suggest.buttons.iter().enumerate() {
            match list.get(i) {
                Some(s) => {
                    b.set_text(&s.label());
                    widths[i] = style::button_width(&self.font, b);
                }
                None => b.set_visible(false),
            }
        }
        self.suggest.widths.set(widths);
        let now = !list.is_empty();
        if now {
            self.suggest.lead.set_text(crate::indexing_status::nothing_found_lead(crate::i18n::current(), self.indexing()));
        }
        self.suggest.lead.set_visible(now);
        *self.suggest.list.borrow_mut() = list;
        if was || now {
            self.layout();
        }
    }

    /// Places the shown buttons in a row at `y` from `x`, as many as fit in
    /// `w`; returns where the content below goes.
    pub(super) fn place_suggestions(&self, x: i32, y: i32, w: i32) -> i32 {
        use style::{CTRL_H, GAP};
        let n = self.suggest.list.borrow().len();
        if n == 0 {
            return y;
        }
        // The sentence on a line of its own, then the buttons.
        let line_h = style::line_height(&self.font).min(CTRL_H);
        style::place(&self.suggest.lead.handle, x, y, w.max(40), line_h);
        let y = y + line_h + GAP / 2;
        let widths = self.suggest.widths.get();
        let mut at = x;
        for (i, b) in self.suggest.buttons.iter().enumerate() {
            let bw = widths[i];
            let fits = i < n && at + bw <= x + w;
            if fits {
                style::place(&b.handle, at, y, bw, CTRL_H);
                at += bw + GAP;
            }
            b.set_visible(fits);
        }
        y + CTRL_H + GAP
    }

    /// A click on a suggestion button: applies it. False for other controls.
    pub(super) fn on_suggestion_click(&self, handle: nwg::ControlHandle) -> bool {
        let Some(i) = self.suggest.buttons.iter().position(|b| handle == *b) else { return false };
        let Some(s) = self.suggest.list.borrow().get(i).cloned() else { return true };
        self.apply_suggestion(&s);
        true
    }

    fn apply_suggestion(&self, s: &Suggestion) {
        match s {
            Suggestion::Query(text) => {
                self.query.set_text(text);
                self.focus_query();
                self.on_search();
            }
            Suggestion::AllKinds => {
                self.kind_combo.set_selection(Some(0));
                self.focus_query();
                self.on_kind_changed();
                if self.content_mode() {
                    self.on_search();
                }
            }
            Suggestion::AllDrives => {
                self.focus_query();
                self.set_scope(String::new());
                if self.content_mode() {
                    self.on_search();
                }
            }
            Suggestion::Contents => {
                self.content_chk.set_check_state(nwg::CheckBoxState::Checked);
                self.focus_query();
                self.on_search();
            }
            Suggestion::Web => {
                let url = suggest::web_url(&self.query.text());
                if !shell::open(&url) {
                    self.set_status(tr!("تعذر فتح المتصفح.", "Could not open the browser."));
                }
            }
        }
    }
}
