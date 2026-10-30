//! Options ▸ "Language / اللغة": Arabic or English. The choice is saved in
//! the registry and applies when the program starts again; it offers to
//! restart at once (the window is not re-labelled live).

use native_windows_gui as nwg;

use super::{App, title};
use crate::i18n::{self, Lang};

/// Adds the submenu to the Options menu, the current language checked.
pub(super) fn build(
    options: &nwg::Menu,
    menu: &mut nwg::Menu,
    ar: &mut nwg::MenuItem,
    en: &mut nwg::MenuItem,
) -> Result<(), nwg::NwgError> {
    nwg::Menu::builder().text("Language / اللغة").parent(options).build(menu)?;
    let current = i18n::current();
    nwg::MenuItem::builder().text("العربية").check(current == Lang::Ar).parent(&*menu).build(ar)?;
    nwg::MenuItem::builder().text("English").check(current == Lang::En).parent(&*menu).build(en)?;
    Ok(())
}

impl App {
    /// Handles a click on one of the language items; false for any other item.
    pub(super) fn on_lang_item(&self, handle: nwg::ControlHandle) -> bool {
        let lang = if handle == self.menu_lang_ar {
            Lang::Ar
        } else if handle == self.menu_lang_en {
            Lang::En
        } else {
            return false;
        };
        if !i18n::save(lang) {
            self.set_status(tr!("تعذر حفظ اللغة.", "Could not save the language."));
            return true;
        }
        self.menu_lang_ar.set_checked(lang == Lang::Ar);
        self.menu_lang_en.set_checked(lang == Lang::En);
        if lang == i18n::current() {
            return true;
        }
        // Worded in the language just chosen: that is the one the user reads.
        let params = nwg::MessageParams {
            title: title(),
            content: lang.pick(
                "ستظهر الواجهة بالعربية بعد إعادة تشغيل البرنامج.\n\nهل تريد إعادة التشغيل الآن؟",
                "The interface will be in English after the program restarts.\n\nRestart now?",
            ),
            buttons: nwg::MessageButtons::YesNo,
            icons: nwg::MessageIcons::Question,
        };
        if nwg::modal_message(&self.window, &params) != nwg::MessageChoice::Yes {
            self.set_status(lang.pick(
                "ستظهر الواجهة بالعربية عند تشغيل البرنامج في المرة القادمة.",
                "The interface will be in English the next time the program starts.",
            ));
            return true;
        }
        match crate::update::restart_new_version() {
            Ok(()) => {
                // Same as after an update: save everything and leave at once
                // so the new copy gets the single-instance lock.
                self.on_close();
                std::process::exit(0);
            }
            Err(e) => {
                nwg::modal_error_message(&self.window, title(), &trf!(
                    "تعذرت إعادة التشغيل تلقائياً ({e}).\nأغلق البرنامج وافتحه من جديد.",
                    "The program could not restart itself ({e}).\nClose the program and open it again."
                ));
            }
        }
        true
    }
}
