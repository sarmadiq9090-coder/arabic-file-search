//! The pop-up menu of recent and saved searches.

use native_windows_gui as nwg;

use super::App;
use crate::history::SavedSearch;
use crate::search;

/// The pop-up menu of saved and recent searches and what each item does.
#[derive(Default)]
pub(super) struct HistoryMenu {
    menu: nwg::Menu,
    items: Vec<(nwg::MenuItem, HistoryAction)>,
    separators: Vec<nwg::MenuSeparator>,
}

pub(super) enum HistoryAction {
    Saved(SavedSearch),
    Recent(String),
    SaveCurrent,
    ClearRecent,
    ClearSaved,
    Nothing,
}

impl App {
    // ---------------------------------------------------------------- history

    pub(super) fn show_history_menu(&self) {
        let st = self.state.borrow();
        let mut hm = HistoryMenu::default();
        let w = &self.window;
        if nwg::Menu::builder().popup(true).parent(w).build(&mut hm.menu).is_err() {
            return;
        }
        let add = |hm: &mut HistoryMenu, text: &str, action: HistoryAction, disabled: bool| {
            let mut item = nwg::MenuItem::default();
            if nwg::MenuItem::builder().text(text).disabled(disabled).parent(&hm.menu).build(&mut item).is_ok() {
                hm.items.push((item, action));
            }
        };
        let sep = |hm: &mut HistoryMenu| {
            let mut s = nwg::MenuSeparator::default();
            if nwg::MenuSeparator::builder().parent(&hm.menu).build(&mut s).is_ok() {
                hm.separators.push(s);
            }
        };
        if !st.history.saved.is_empty() {
            add(&mut hm, "البحوث المحفوظة", HistoryAction::Nothing, true);
            for s in &st.history.saved {
                add(&mut hm, &format!("★  {}", s.name), HistoryAction::Saved(s.clone()), false);
            }
            sep(&mut hm);
        }
        if st.history.recent.is_empty() {
            add(&mut hm, "لا توجد عمليات بحث سابقة بعد", HistoryAction::Nothing, true);
        } else {
            add(&mut hm, "عمليات البحث الأخيرة", HistoryAction::Nothing, true);
            for q in st.history.recent.iter().take(15) {
                add(&mut hm, q, HistoryAction::Recent(q.clone()), false);
            }
        }
        sep(&mut hm);
        let has_query = !self.query.text().trim().is_empty();
        add(&mut hm, "حفظ البحث الحالي", HistoryAction::SaveCurrent, !has_query);
        add(&mut hm, "مسح عمليات البحث الأخيرة", HistoryAction::ClearRecent, st.history.recent.is_empty());
        add(&mut hm, "حذف كل البحوث المحفوظة", HistoryAction::ClearSaved, st.history.saved.is_empty());
        drop(st);
        *self.history_menu.borrow_mut() = hm;
        self.show_menu(&self.history_menu.borrow().menu);
    }

    /// Runs the history-menu item behind `handle`; false if it is not one.
    pub(super) fn on_history_item(&self, handle: nwg::ControlHandle) -> bool {
        let action = {
            let hm = self.history_menu.borrow();
            let Some((_, a)) = hm.items.iter().find(|(i, _)| i.handle == handle) else { return false };
            match a {
                HistoryAction::Saved(s) => HistoryAction::Saved(s.clone()),
                HistoryAction::Recent(q) => HistoryAction::Recent(q.clone()),
                HistoryAction::SaveCurrent => HistoryAction::SaveCurrent,
                HistoryAction::ClearRecent => HistoryAction::ClearRecent,
                HistoryAction::ClearSaved => HistoryAction::ClearSaved,
                HistoryAction::Nothing => HistoryAction::Nothing,
            }
        };
        match action {
            HistoryAction::Saved(s) => self.apply_saved(&s),
            HistoryAction::Recent(q) => {
                self.query.set_text(&q);
                self.focus_query();
                self.on_search();
            }
            HistoryAction::SaveCurrent => {
                let text = self.query.text().trim().to_string();
                let mut st = self.state.borrow_mut();
                let saved = SavedSearch {
                    name: text.clone(),
                    query: text.clone(),
                    kind: self.kind(),
                    scope: st.scope.clone(),
                    content: self.content_mode(),
                };
                st.history.add_saved(saved);
                st.history.save();
                drop(st);
                self.set_status(&format!("حُفظ البحث «{text}». تجده في قائمة «السجل»."));
            }
            HistoryAction::ClearRecent => {
                let mut st = self.state.borrow_mut();
                st.history.recent.clear();
                st.history.save();
            }
            HistoryAction::ClearSaved => {
                let mut st = self.state.borrow_mut();
                st.history.saved.clear();
                st.history.save();
            }
            HistoryAction::Nothing => {}
        }
        true
    }

    pub(super) fn apply_saved(&self, s: &SavedSearch) {
        self.kind_combo.set_selection(Some(s.kind.min(search::KINDS.len() - 1)));
        self.state.borrow_mut().settings.kind = self.kind();
        self.content_chk.set_check_state(if s.content {
            nwg::CheckBoxState::Checked
        } else {
            nwg::CheckBoxState::Unchecked
        });
        self.scope_input.set_text(if s.scope.is_empty() { "كل الأقراص" } else { &s.scope });
        self.state.borrow_mut().scope = s.scope.clone();
        self.query.set_text(&s.query);
        self.focus_query();
        self.on_search();
    }
}
