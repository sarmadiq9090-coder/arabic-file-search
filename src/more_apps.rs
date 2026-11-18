//! Options ▸ «المزيد من FastKiwi» / "More from FastKiwi": the other FastKiwi
//! apps, each opening its page on fastkiwi.app (and the Microsoft Store when
//! its listing is live). It installs and downloads nothing.
//!
//! The list is `data/apps.json`, a copy of the site's `data/apps.json`
//! (repo `sarmadiq9090-coder.github.io`, docs/apps-data.md), embedded at
//! build time. It is never fetched at runtime: no request tells a server that
//! the program was opened, and the menu works offline.
//!
//! Vendored from site commit 2f13a0cbdda57e2f2f108eb65f47c21e52122959
//! (branch `claude/apps-data`). To refresh it, from this repo's root with the
//! site repo checked out next to it:
//!
//! ```sh
//! git -C ../sarmadiq9090-coder.github.io show <commit>:data/apps.json > data/apps.json
//! ```
//!
//! then update the commit above. Changes reach users with the next release.

use serde::Deserialize;

/// The vendored list (see the module comment).
const APPS_JSON: &str = include_str!("../data/apps.json");

/// This program's `id` in the list; it is left out.
pub const SELF_ID: &str = "instafind";

/// The schema version this code reads (`schema` in the file).
const SCHEMA: u32 = 1;

#[derive(Deserialize)]
struct Catalog {
    schema: u32,
    apps: Vec<Entry>,
}

/// One app of the file. Fields this menu does not show (description, winget,
/// icon) are ignored.
#[derive(Deserialize)]
struct Entry {
    id: String,
    status: String,
    name: Text,
    tagline: Text,
    url: Text,
    store: Option<Listing>,
}

#[derive(Deserialize)]
struct Text {
    ar: String,
    en: String,
}

impl Text {
    fn pick(&self, english: bool) -> &str {
        if english { &self.en } else { &self.ar }
    }
}

#[derive(Deserialize)]
struct Listing {
    id: String,
    live: bool,
}

/// One menu item: its text and what it opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// Ready for a Win32 menu (`&` doubled, no control characters).
    pub label: String,
    /// An `https://fastkiwi.app/` page or a Microsoft Store product page;
    /// always passes [`is_allowed`].
    pub target: String,
}

/// The menu items for the embedded list, in the interface language.
pub fn links(english: bool) -> Vec<Link> {
    links_from(APPS_JSON, SELF_ID, english)
}

/// The menu items for `json`, without `self_id`. An app whose page fails
/// [`is_allowed`] is left out; a file that doesn't parse, or has another
/// schema, gives no items (the menu is then not shown).
fn links_from(json: &str, self_id: &str, english: bool) -> Vec<Link> {
    let Ok(catalog) = serde_json::from_str::<Catalog>(json) else { return Vec::new() };
    if catalog.schema != SCHEMA {
        return Vec::new();
    }
    let mut out = Vec::new();
    for app in catalog.apps.iter().filter(|a| a.id != self_id) {
        let page = app.url.pick(english);
        if !is_allowed(page) {
            continue;
        }
        let name = app.name.pick(english);
        let tagline = app.tagline.pick(english);
        let coming_soon = app.status == "coming-soon";
        let label = if coming_soon {
            let soon = if english { "Coming soon" } else { "قريباً" };
            format!("{name} — {tagline} ({soon})")
        } else {
            format!("{name} — {tagline}")
        };
        out.push(Link { label: menu_text(&label), target: page.to_string() });
        // Only a released app with a public listing gets a Store item; an
        // app that is coming soon never has an install action.
        let store = app.store.as_ref().filter(|s| s.live && !coming_soon).and_then(|s| store_uri(&s.id));
        if let Some(uri) = store {
            let label = if english { format!("{name} in the Microsoft Store") } else { format!("{name} في متجر Microsoft") };
            out.push(Link { label: menu_text(&label), target: uri });
        }
    }
    out
}

/// The Microsoft Store page of a product id (12 upper-case letters and
/// digits, e.g. `9NGB1F7T7JGN`); None for anything else.
fn store_uri(id: &str) -> Option<String> {
    let ok = id.len() == 12 && id.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
    ok.then(|| format!("ms-windows-store://pdp/?productid={id}"))
}

/// Whether the menu may open `target`: a page on https://fastkiwi.app/ (plain
/// URL characters only, no `..`) or a Microsoft Store product page. Checked
/// again right before opening.
pub fn is_allowed(target: &str) -> bool {
    if let Some(id) = target.strip_prefix("ms-windows-store://pdp/?productid=") {
        return store_uri(id).is_some();
    }
    let Some(path) = target.strip_prefix("https://fastkiwi.app/") else { return false };
    !path.contains("..") && path.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~/#?=&%".contains(&b))
}

/// Menu text: `&` would underline the next letter, a tab would start the
/// shortcut column.
fn menu_text(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect::<String>().replace('&', "&&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vendored_file_parses_and_leaves_this_app_out() {
        let catalog: Catalog = serde_json::from_str(APPS_JSON).expect("data/apps.json parses");
        assert_eq!(catalog.schema, SCHEMA);
        assert!(catalog.apps.iter().any(|a| a.id == SELF_ID), "InstaFind is in the list under its id");
        for english in [false, true] {
            let links = links(english);
            assert!(links.len() >= 4, "{links:?}");
            assert!(links.iter().all(|l| !l.label.starts_with("InstaFind")), "{links:?}");
            assert!(links.iter().all(|l| !l.target.contains("/arabic-file-search/")), "{links:?}");
            assert!(links.iter().all(|l| is_allowed(&l.target)), "{links:?}");
        }
    }

    #[test]
    fn labels_and_pages_follow_the_language() {
        let ar = links(false);
        let en = links(true);
        assert!(ar.iter().any(|l| l.label == "ذكرني — المهام والتذكيرات" && l.target == "https://fastkiwi.app/Remember-Me/ar/"), "{ar:?}");
        // "Tasks & reminders": the & is doubled, or Windows would hide it.
        assert!(en.iter().any(|l| l.label == "Remember Me — Tasks && reminders" && l.target == "https://fastkiwi.app/Remember-Me/"), "{en:?}");
        assert!(ar.iter().any(|l| l.label.ends_with("(قريباً)")));
        assert!(en.iter().any(|l| l.label.ends_with("(Coming soon)")));
    }

    #[test]
    fn store_items_only_for_live_listings_and_never_for_coming_soon() {
        let catalog: Catalog = serde_json::from_str(APPS_JSON).unwrap();
        let stores: Vec<String> = links(true).into_iter().filter(|l| l.target.starts_with("ms-windows-store:")).map(|l| l.target).collect();
        let live: Vec<String> = catalog
            .apps
            .iter()
            .filter(|a| a.id != SELF_ID && a.status != "coming-soon")
            .filter_map(|a| a.store.as_ref().filter(|s| s.live))
            .map(|s| format!("ms-windows-store://pdp/?productid={}", s.id))
            .collect();
        assert_eq!(stores, live);
        // A coming-soon app has only its page, even with a live listing.
        let json = r#"{"schema":1,"apps":[{"id":"x","status":"coming-soon","name":{"ar":"س","en":"X"},
            "tagline":{"ar":"ت","en":"T"},"url":{"ar":"https://fastkiwi.app/ar/x/","en":"https://fastkiwi.app/x/"},
            "store":{"id":"9NGB1F7T7JGN","live":true}}]}"#;
        let links = links_from(json, SELF_ID, true);
        assert_eq!(links, vec![Link { label: "X — T (Coming soon)".into(), target: "https://fastkiwi.app/x/".into() }]);
        // A listing that is not live yet gives no Store item.
        let json = json.replace("coming-soon", "released").replace("true", "false");
        assert_eq!(links_from(&json, SELF_ID, true).len(), 1);
    }

    #[test]
    fn only_fastkiwi_pages_and_store_pages_are_opened() {
        for ok in [
            "https://fastkiwi.app/",
            "https://fastkiwi.app/Remember-Me/ar/",
            "https://fastkiwi.app/downloads/#instaconvert",
            "ms-windows-store://pdp/?productid=9NGB1F7T7JGN",
        ] {
            assert!(is_allowed(ok), "{ok}");
        }
        for bad in [
            "http://fastkiwi.app/",
            "https://fastkiwi.app",
            "https://fastkiwi.app.evil.example/",
            "https://fastkiwi.app@evil.example/",
            "https://evil.example/https://fastkiwi.app/",
            "https://fastkiwi.app/../x",
            "https://fastkiwi.app/a b",
            "https://fastkiwi.app/\"x",
            "file:///C:/Windows/System32/calc.exe",
            "C:\\Windows\\System32\\calc.exe",
            "ms-windows-store://pdp/?productid=9ngb1f7t7jgn",
            "ms-windows-store://pdp/?productid=9NGB1F7T7JGN&x=1",
            "ms-windows-store://search/?query=x",
            "",
        ] {
            assert!(!is_allowed(bad), "{bad}");
        }
        // A page outside fastkiwi.app in the file is left out, not opened.
        let json = r#"{"schema":1,"apps":[{"id":"x","status":"released","name":{"ar":"س","en":"X"},
            "tagline":{"ar":"ت","en":"T"},"url":{"ar":"https://evil.example/","en":"https://evil.example/"},"store":null}]}"#;
        assert!(links_from(json, SELF_ID, true).is_empty());
    }

    #[test]
    fn a_broken_file_or_another_schema_gives_no_menu() {
        assert!(links_from("", SELF_ID, true).is_empty());
        assert!(links_from("{", SELF_ID, true).is_empty());
        assert!(links_from(&APPS_JSON.replacen("\"schema\": 1", "\"schema\": 2", 1), SELF_ID, true).is_empty());
    }
}
