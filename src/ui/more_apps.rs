//! Options ▸ «المزيد من FastKiwi» / "More from FastKiwi": the other FastKiwi
//! apps from the embedded list (`crate::more_apps`). A click opens the app's
//! page on fastkiwi.app, or its Microsoft Store page, in the default browser
//! or Store app; nothing is downloaded or installed.

use native_windows_gui as nwg;

use super::startup::log;
use super::{App, shell, title};
use crate::more_apps;

/// Adds the submenu to the Options menu, one item per link; no submenu when
/// the list is empty.
pub(super) fn build(options: &nwg::Menu, menu: &mut nwg::Menu, items: &mut Vec<(nwg::MenuItem, String)>) -> Result<(), nwg::NwgError> {
    let links = more_apps::links(crate::i18n::is_en());
    if links.is_empty() {
        return Ok(());
    }
    nwg::Menu::builder().text(tr!("المزيد من FastKiwi", "More from FastKiwi")).parent(options).build(menu)?;
    for link in links {
        let mut item = nwg::MenuItem::default();
        nwg::MenuItem::builder().text(&link.label).parent(&*menu).build(&mut item)?;
        items.push((item, link.target));
    }
    Ok(())
}

impl App {
    /// Handles a click on one of the items; false for any other item.
    pub(super) fn on_more_apps_item(&self, handle: nwg::ControlHandle) -> bool {
        let Some((_, target)) = self.more_apps_items.iter().find(|(item, _)| handle == *item) else { return false };
        // Only fastkiwi.app pages and Store product pages, checked again here.
        if !more_apps::is_allowed(target) {
            log("more apps: refused a link that is not a fastkiwi.app or Store page");
            return true;
        }
        if !shell::open(target) {
            log("more apps: could not open the browser or the Store");
            nwg::modal_info_message(
                &self.window,
                title(),
                &trf!("تعذر فتح الرابط. افتحه في المتصفح:\n{}", "Could not open the link. Open it in your browser:\n{}", target),
            );
        }
        true
    }
}
