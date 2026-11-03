//! Apps in the main window (0.9): when what is typed in the search box names
//! an installed program strongly (`launcher::STRONG_SCORE`), up to three
//! buttons above the results start it. They use the quick search's catalog
//! (built once by `start_catalog_once`, refreshed by the popup), and opening one counts like opening it from the popup.

use super::App;
use crate::launcher;

/// Most apps offered above the results.
pub(super) const APP_BUTTONS: usize = 3;

impl App {
    /// Shows the apps the search box names strongly as buttons above the
    /// results, or none (empty box, document search, catalog not built yet).
    pub(super) fn show_app_matches(&self) {
        let text = self.query.text();
        let items = if text.trim().is_empty() || self.content_mode() {
            Vec::new()
        } else {
            self.start_catalog_once();
            match self.catalog_now() {
                Some(c) => {
                    let st = self.state.borrow();
                    let now = crate::index::now_secs();
                    let history = &st.history;
                    let launches = move |key: &str| history.launch_count(key, now);
                    let picks = history.picks_for(&text, now);
                    let picked = move |key: &str| picks.iter().find(|p| p.0 == key).map_or(0, |p| p.1);
                    let ctx = launcher::Context {
                        english: crate::i18n::is_en(),
                        windows_build: super::surface::windows_build(),
                        windows: &[],
                        launches: &launches,
                        picked: &picked,
                        today: super::quick::today(),
                    };
                    c.strong_apps(&text, &ctx, APP_BUTTONS)
                }
                None => Vec::new(),
            }
        };
        let changed = {
            let old = self.app_items.borrow();
            old.len() != items.len() || old.iter().zip(&items).any(|(a, b)| a.action != b.action || a.title != b.title)
        };
        if !changed {
            return;
        }
        for (i, b) in self.app_btns.iter().enumerate() {
            match items.get(i) {
                Some(it) => {
                    b.set_text(&format!("{}: {}", tr!("تطبيق", "App"), it.title));
                    b.set_visible(true);
                }
                None => b.set_visible(false),
            }
        }
        *self.app_items.borrow_mut() = items;
        self.layout();
    }

    /// How many app buttons are shown, for the layout.
    pub(super) fn app_buttons_shown(&self) -> usize {
        self.app_items.borrow().len().min(APP_BUTTONS)
    }

    /// Starts the app behind a clicked app button. False when `handle` is
    /// not one of them.
    pub(super) fn on_app_button(&self, handle: &native_windows_gui::ControlHandle) -> bool {
        let Some(i) = self.app_btns.iter().position(|b| b.handle == *handle) else { return false };
        let Some(item) = self.app_items.borrow().get(i).cloned() else { return true };
        let text = self.query.text();
        match self.run_action(&item.action, false) {
            Some(true) => {
                let now = crate::index::now_secs();
                let key = launcher::action_key(&item.action);
                let mut st = self.state.borrow_mut();
                st.history.record_launch(&key, now);
                st.history.record_pick(&text, &key, now);
                st.history.add_recent(&text);
                st.history.save();
            }
            Some(false) => self.set_status(&trf!("تعذر فتح: {}", "Could not open: {}", item.title)),
            None => {}
        }
        true
    }
}
