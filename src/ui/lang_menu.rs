//! Options ▸ "Language / اللغة": the interface languages (Arabic, English,
//! and every translation this build has, `i18n::available`), each by its
//! own name, a translation marked as a machine translation. The choice is
//! saved in the registry and applies when the program starts again; it
//! offers to restart at once (the window is not re-labelled live).
//!
//! With many languages the submenu is split into columns
//! (`MFT_MENUBARBREAK`), so it never grows taller than the screen.

use native_windows_gui as nwg;

use super::{App, title};
use crate::i18n::{self, Lang, Locale};

/// Items per column of the submenu.
const COLUMN: usize = 14;

/// "(machine translation)" after a translated language's name, in the
/// current interface language.
fn label(l: &Locale) -> String {
    if l.machine { format!("{} {}", l.native, tr!("(ترجمة آلية)", "(machine translation)")) } else { l.native.to_string() }
}

/// Adds the submenu to the Options menu, the current language checked.
pub(super) fn build(options: &nwg::Menu, menu: &mut nwg::Menu, items: &mut Vec<(nwg::MenuItem, &'static Locale)>) -> Result<(), nwg::NwgError> {
    nwg::Menu::builder().text("Language / اللغة").parent(options).build(menu)?;
    let current = i18n::locale();
    items.clear();
    for (i, l) in i18n::available().into_iter().enumerate() {
        let mut item = nwg::MenuItem::default();
        nwg::MenuItem::builder().text(&label(l)).check(std::ptr::eq(l, current)).parent(&*menu).build(&mut item)?;
        if i > 0 && i % COLUMN == 0 {
            column_break(&item);
        }
        items.push((item, l));
    }
    Ok(())
}

/// Starts a new column of the menu at `item`.
fn column_break(item: &nwg::MenuItem) {
    use winapi::um::winuser::{MENUITEMINFOW, MFT_MENUBARBREAK, MIIM_FTYPE, SetMenuItemInfoW};
    let Some((hmenu, id)) = item.handle.hmenu_item() else { return };
    let mut mii: MENUITEMINFOW = unsafe { std::mem::zeroed() };
    mii.cbSize = std::mem::size_of::<MENUITEMINFOW>() as u32;
    mii.fMask = MIIM_FTYPE;
    mii.fType = MFT_MENUBARBREAK;
    unsafe { SetMenuItemInfoW(hmenu, id, 0, &mii) };
}

impl App {
    /// Checks `chosen` and unchecks the other languages.
    pub(super) fn check_lang_item(&self, chosen: &Locale) {
        for (item, l) in &self.menu_langs {
            item.set_checked(std::ptr::eq(*l, chosen));
        }
    }

    /// Handles a click on one of the language items; false for any other item.
    pub(super) fn on_lang_item(&self, handle: nwg::ControlHandle) -> bool {
        let Some(&(_, chosen)) = self.menu_langs.iter().find(|(item, _)| handle == *item) else {
            return false;
        };
        if !i18n::save_locale(chosen) {
            self.set_status(tr!("تعذر حفظ اللغة.", "Could not save the language."));
            return true;
        }
        self.check_lang_item(chosen);
        if std::ptr::eq(chosen, i18n::locale()) {
            return true;
        }
        // Worded in the language just chosen: that is the one the user
        // reads. A translated language has no texts of its own here (they
        // come with its file at the next start), so it reads English with
        // the language's own name.
        let (restart_q, next_time) = match chosen.base {
            Lang::Ar => (
                "ستظهر الواجهة بالعربية بعد إعادة تشغيل البرنامج.\n\nهل تريد إعادة التشغيل الآن؟".to_string(),
                "ستظهر الواجهة بالعربية عند تشغيل البرنامج في المرة القادمة.".to_string(),
            ),
            Lang::En => (
                format!("The interface will be in {} after the program restarts.\n\nRestart now?", chosen.native),
                format!("The interface will be in {} the next time the program starts.", chosen.native),
            ),
        };
        let params = nwg::MessageParams { title: title(), content: &restart_q, buttons: nwg::MessageButtons::YesNo, icons: nwg::MessageIcons::Question };
        if self.with_modal_owner(|owner| nwg::modal_message(nwg::ControlHandle::Hwnd(owner), &params)) != nwg::MessageChoice::Yes {
            self.set_status(&next_time);
            return true;
        }
        let started = crate::update::restart_new_version();
        let failed = match started {
            // Leaves when the new copy runs; keeps running otherwise.
            Ok(child) => {
                self.hand_over(child, "language");
                Err(chosen.base.pick("لم تعمل النسخة الجديدة", "the new copy did not start").to_string())
            }
            Err(e) => Err(e),
        };
        match failed {
            Ok(()) => {}
            Err(e) => {
                self.modal_error(title(), &trf!(
                    "تعذرت إعادة التشغيل تلقائياً ({e}).\nأغلق البرنامج وافتحه من جديد.",
                    "The program could not restart itself ({e}).\nClose the program and open it again."
                ));
            }
        }
        true
    }
}
