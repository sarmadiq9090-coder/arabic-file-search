//! What the quick search finds besides files: installed apps, Windows
//! settings pages, open windows, instant answers (calculator, units, Hijri
//! date, currencies), web shortcuts and system commands.
//!
//! Everything here is platform independent and unit-tested on Linux, except
//! the parts that ask Windows for data (`apps::scan`, `windows::list`), which
//! are `cfg(windows)` inside their modules.
//!
//! The UI calls [`Catalog::query`] on every keystroke, so it must stay well
//! under a millisecond for a few hundred entries.

pub mod aliases;
pub mod answers;
pub mod apps;
mod calc;
pub mod currency;
mod hijri;
mod matching;
mod phonetic;
pub mod settings_pages;
pub mod system;
mod units;
pub mod web;
pub mod windows;

/// Which group a result is shown in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// An instant answer (calculation, unit conversion, date).
    Answer,
    /// An open window of a running program.
    Window,
    /// An installed program (Start menu shortcut or Store app).
    App,
    /// A Windows settings page.
    Setting,
    /// A system command (shut down, lock…); always confirmed first.
    Command,
    /// Search a website for the query.
    Web,
}

/// A command that changes the state of the computer. The UI must ask for
/// confirmation before running any of these.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SystemCommand {
    Shutdown,
    Restart,
    Sleep,
    Lock,
    SignOut,
    EmptyRecycleBin,
    /// End a program: process id and its executable name, for the prompt.
    EndProcess { pid: u32, name: String },
}

/// What happens when a result is opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Start a program: a `.lnk`/`.exe` path, or `shell:AppsFolder\<AUMID>`
    /// for a Store app.
    Launch { target: String, args: String },
    /// Open a URI with the shell: `ms-settings:…`, `https://…`.
    OpenUri(String),
    /// Copy text to the clipboard (calculator result).
    CopyText(String),
    /// Bring an open window to the front (HWND as an integer).
    FocusWindow(isize),
    /// Run a system command after the user confirms it.
    System(SystemCommand),
}

/// One row of the quick-search results.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub kind: Kind,
    /// Main text, shown in the interface language where one exists.
    pub title: String,
    /// Second line: a path, the settings section, "Store app"…
    pub subtitle: String,
    pub action: Action,
    /// Higher is better; only comparable within one query.
    pub score: u32,
    /// File the icon is taken from (exe, lnk), if any.
    pub icon_path: Option<String>,
    /// An app named strongly by the query: its match score plus the habit
    /// for these very letters reaches [`STRONG_SCORE`]. How often it was
    /// launched in general does not count, so a favourite does not turn a
    /// weak match strong.
    pub strong: bool,
}

/// The data the quick search looks through besides files, built once in the
/// background and then queried on every keystroke.
#[derive(Default)]
pub struct Catalog {
    pub apps: Vec<apps::App>,
    pub settings: Vec<settings_pages::Page>,
    /// Normalised names and keywords of `apps` and `settings`, in order.
    index: matching::Index,
}

/// A score at least this high is a strong match: the whole name or an
/// alias, the start of one ("سبوت" for سبوتيفاي), the name said in Arabic,
/// or a habit. Such an app leads the quick-search list over files and is
/// offered in the main window too; a word somewhere in an alias ("شاشة" in
/// "لقطة شاشة") is not enough.
pub const STRONG_SCORE: u32 = 850;

/// What the caller knows about the moment of the query.
pub struct Context<'a> {
    /// The interface language is English (titles and answers follow it).
    pub english: bool,
    /// Windows build number, to hide settings pages that do not exist (0 = unknown, show all).
    pub windows_build: u32,
    /// Open windows, listed by the UI just before querying (empty on Linux tests).
    pub windows: &'a [windows::OpenWindow],
    /// How often the user opened a target before (`Action` key → count), for the boost.
    pub launches: &'a dyn Fn(&str) -> u32,
    /// How often the user opened a target (`Action` key) after typing this
    /// very query, or a longer one that starts with it (0 = never); see
    /// `History::picks_for`. A strong boost: the habit for these letters.
    pub picked: &'a dyn Fn(&str) -> u32,
    /// Today's date (year, month, day), for the Hijri answer.
    pub today: (i32, u32, u32),
}

impl Catalog {
    /// A catalog of `apps` and `settings` (usually `settings_pages::PAGES`),
    /// with their names and keywords normalised once for [`Catalog::query`].
    pub fn new(apps: Vec<apps::App>, settings: Vec<settings_pages::Page>) -> Catalog {
        let index = matching::Index::build(&apps, &settings);
        Catalog { apps, settings, index }
    }

    /// Results for `query`, best first, at most `limit`.
    ///
    /// Apps and settings pages are matched here (exact name, then the start
    /// of the name, then the start of a word, then anywhere; Arabic letter
    /// variants and case ignored; also the query typed in the wrong keyboard
    /// layout, Arabizi and small typos when nothing else matches), boosted by
    /// how often each was opened. Open windows, system commands and web
    /// shortcuts come from their modules, and an instant answer always comes
    /// first.
    pub fn query(&self, query: &str, ctx: &Context, limit: usize) -> Vec<Item> {
        let q = query.trim();
        if q.is_empty() || limit == 0 {
            return Vec::new();
        }
        let answer = answers::answer(q, ctx);

        // A catalog filled field by field (Default, then pushes) has no index yet.
        let rebuilt;
        let index = if self.index.matches(&self.apps, &self.settings) {
            &self.index
        } else {
            rebuilt = matching::Index::build(&self.apps, &self.settings);
            &rebuilt
        };

        let mut items: Vec<Item> = Vec::new();
        let (app_scores, setting_scores) = index.score(q, ctx, &self.settings);
        for (i, s) in app_scores {
            let app = &self.apps[i];
            let action = Action::Launch { target: app.target.clone(), args: String::new() };
            items.push(Item {
                kind: Kind::App,
                title: app.name.clone(),
                subtitle: if ctx.english { "App" } else { "تطبيق" }.to_string(),
                score: s + boost(ctx, &action),
                strong: s + picked_boost(ctx, &action) >= STRONG_SCORE,
                action,
                icon_path: app.icon_path.clone(),
            });
        }
        for (i, s) in setting_scores {
            let page = &self.settings[i];
            let action = page_action(page);
            let subtitle = if page.is_settings_uri() {
                format!("{} › {}", if ctx.english { "Settings" } else { "الإعدادات" }, page.section(ctx.english))
            } else {
                page.section(ctx.english).to_string()
            };
            items.push(Item {
                kind: Kind::Setting,
                title: page.title(ctx.english).to_string(),
                subtitle,
                score: s + boost(ctx, &action),
                strong: false,
                action,
                icon_path: None,
            });
        }
        items.extend(windows::matches(q, ctx));
        items.extend(system::commands(q, ctx));
        let fallback = items.is_empty() && answer.is_none();
        items.extend(web::shortcuts(q, ctx, fallback));

        // Best first; on a tie the group order, then the shorter title ("Word" before "WordPad").
        items.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then(a.kind.cmp(&b.kind))
                .then(a.title.chars().count().cmp(&b.title.chars().count()))
                .then_with(|| a.title.cmp(&b.title))
        });
        let mut seen = std::collections::HashSet::new();
        items.retain(|it| {
            let key = action_key(&it.action);
            let key = if key.is_empty() { format!("{:?}:{}", it.kind, it.title) } else { key };
            seen.insert(key)
        });
        if let Some(a) = answer {
            items.insert(0, a);
        }
        items.truncate(limit);
        items
    }
}

impl Catalog {
    /// The apps `query` names strongly ([`Item::strong`]),
    /// best first, at most `n`: what the main window offers above the files.
    pub fn strong_apps(&self, query: &str, ctx: &Context, n: usize) -> Vec<Item> {
        self.query(query, ctx, 40).into_iter().filter(|i| i.kind == Kind::App && i.strong).take(n).collect()
    }
}

/// What opening a settings page does: `ms-settings:` URIs go to the shell,
/// classic tools ("control printers") are started with their arguments.
pub fn page_action(page: &settings_pages::Page) -> Action {
    if page.is_settings_uri() {
        Action::OpenUri(page.uri.to_string())
    } else {
        let (target, args) = page.uri.split_once(' ').unwrap_or((page.uri, ""));
        Action::Launch { target: target.to_string(), args: args.trim().to_string() }
    }
}

/// Score added for an item opened `n` times before: grows with the
/// logarithm of the count, so a habit beats a slightly better match
/// without letting one favourite swamp every query.
fn launch_boost(n: u32) -> u32 {
    if n == 0 { 0 } else { (60 * (32 - n.leading_zeros())).min(400) }
}

/// Score added for an item picked `n` times after the same letters were
/// typed: more than any tier gap, so the habit leads.
fn pick_boost(n: u32) -> u32 {
    if n == 0 { 0 } else { (500 + 50 * (32 - n.leading_zeros())).min(700) }
}

fn boost(ctx: &Context, action: &Action) -> u32 {
    let key = action_key(action);
    if key.is_empty() { 0 } else { launch_boost((ctx.launches)(&key)) + pick_boost((ctx.picked)(&key)) }
}

/// The part of [`boost`] that belongs to this query: picks after the same letters.
fn picked_boost(ctx: &Context, action: &Action) -> u32 {
    let key = action_key(action);
    if key.is_empty() { 0 } else { pick_boost((ctx.picked)(&key)) }
}

/// Whether opening `action` after typing a query is learnt as a pick for
/// those letters: an app or a settings page. Web searches (whose key holds
/// the typed text) and system commands would only fill the list.
pub fn learns_pick(action: &Action) -> bool {
    match action {
        Action::Launch { .. } => true,
        Action::OpenUri(u) => u.starts_with("ms-settings:"),
        Action::CopyText(_) | Action::FocusWindow(_) | Action::System(_) => false,
    }
}

/// A stable key for an action, used to count launches for the boost.
pub fn action_key(action: &Action) -> String {
    match action {
        Action::Launch { target, .. } => format!("launch:{}", target.to_lowercase()),
        Action::OpenUri(u) => format!("uri:{u}"),
        Action::CopyText(_) => String::new(),
        Action::FocusWindow(_) => String::new(),
        Action::System(c) => format!("sys:{c:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_launches(_: &str) -> u32 {
        0
    }

    fn ctx<'a>(english: bool, build: u32, launches: &'a dyn Fn(&str) -> u32) -> Context<'a> {
        Context { english, windows_build: build, windows: &[], launches, picked: &no_launches, today: (2026, 11, 1) }
    }

    fn app(name: &str) -> apps::App {
        apps::App::new(name.to_string(), format!("shell:AppsFolder\\{}", name.replace(' ', "")), None)
    }

    fn catalog() -> Catalog {
        let names = [
            "Google Chrome",
            "Microsoft Edge",
            "Word",
            "WordPad",
            "Excel",
            "Calculator",
            "Notepad",
            "Notepad++",
            "Spotify",
            "Telegram",
            "WhatsApp",
            "Visual Studio Code",
            "Snipping Tool",
            "Paint",
            "Google Play Games",
            "Discord",
            "Netflix",
            "Adobe Photoshop 2024",
            "Steam",
        ];
        Catalog::new(names.iter().map(|n| app(n)).collect(), settings_pages::PAGES.to_vec())
    }

    fn titles(c: &Catalog, q: &str, cx: &Context) -> Vec<String> {
        c.query(q, cx, 8).into_iter().map(|i| i.title).collect()
    }

    fn first(c: &Catalog, q: &str, cx: &Context) -> Item {
        c.query(q, cx, 8).into_iter().next().unwrap_or_else(|| panic!("nothing for {q}"))
    }

    fn uri(item: &Item) -> &str {
        match &item.action {
            Action::OpenUri(u) => u,
            Action::Launch { target, .. } => target,
            _ => "",
        }
    }

    #[test]
    fn arabic_and_latin_names_find_chrome() {
        let c = catalog();
        let cx = ctx(false, 0, &no_launches);
        for q in ["كروم", "chrome", "Chrome", "google", "جوجل كروم", "chr"] {
            assert_eq!(first(&c, q, &cx).title, "Google Chrome", "{q}");
        }
        assert_eq!(first(&c, "chrome", &cx).kind, Kind::App);
    }

    #[test]
    fn arabic_spelling_of_any_latin_name() {
        let c = catalog();
        let cx = ctx(false, 0, &no_launches);
        for q in ["كوكل بلي", "جوجل بلاي", "قوقل بلاي قيمز"] {
            assert_eq!(first(&c, q, &cx).title, "Google Play Games", "{q}");
        }
        // A name in the Arabic table still comes first.
        assert_eq!(first(&c, "جوجل كروم", &cx).title, "Google Chrome");
        assert!(titles(&c, "سبوتيفاي", &cx).contains(&"Spotify".to_string()));
    }

    #[test]
    fn wrong_keyboard_layout() {
        let c = catalog();
        let cx = ctx(false, 0, &no_launches);
        // "chrome" typed with the Arabic layout on.
        assert_eq!(first(&c, "ؤاقخةث", &cx).title, "Google Chrome");
        // "الحاسبة" typed with the English layout on.
        assert_eq!(first(&c, "hgphsfm", &cx).title, "Calculator");
    }

    #[test]
    fn arabizi_and_typos() {
        let c = catalog();
        let cx = ctx(false, 0, &no_launches);
        assert_eq!(first(&c, "kroom", &cx).title, "Google Chrome");
        assert_eq!(first(&c, "spotfy", &cx).title, "Spotify");
        assert_eq!(first(&c, "telgram", &cx).title, "Telegram");
    }

    #[test]
    fn whole_name_beats_longer_names() {
        let c = catalog();
        let cx = ctx(false, 0, &no_launches);
        assert_eq!(first(&c, "word", &cx).title, "Word");
        assert_eq!(first(&c, "وورد", &cx).title, "Word");
        let t = titles(&c, "وورد", &cx);
        assert!(t.iter().position(|t| t == "WordPad").is_none_or(|i| i > 0));
        assert_eq!(first(&c, "notepad", &cx).title, "Notepad");
        assert_eq!(first(&c, "المفكرة", &cx).title, "Notepad");
        assert_eq!(first(&c, "الحاسبه", &cx).title, "Calculator");
    }

    #[test]
    fn wifi_settings() {
        let c = catalog();
        let cx = ctx(false, 0, &no_launches);
        for q in ["واي فاي", "wifi", "وايفاي", "Wi-Fi", "الواي فاي"] {
            assert_eq!(uri(&first(&c, q, &cx)), "ms-settings:network-wifi", "{q}");
        }
        assert_eq!(first(&c, "wifi", &cx).kind, Kind::Setting);
        assert_eq!(first(&c, "wifi", &cx).action, Action::OpenUri("ms-settings:network-wifi".into()));
    }

    #[test]
    fn settings_words_without_hamza_or_ta_marbuta() {
        let c = catalog();
        let cx = ctx(false, 0, &no_launches);
        for q in ["اعدادات الشاشه", "إعدادات الشاشة", "الشاشة", "شاشه", "display", "display settings"] {
            assert_eq!(uri(&first(&c, q, &cx)), "ms-settings:display", "{q}");
        }
        assert_eq!(uri(&first(&c, "بلوتوث", &cx)), "ms-settings:bluetooth");
        assert_eq!(uri(&first(&c, "الوضع الداكن", &cx)), "ms-settings:personalization-colors");
        assert_eq!(uri(&first(&c, "المتصفح الافتراضي", &cx)), "ms-settings:defaultapps");
        assert_eq!(uri(&first(&c, "حذف برنامج", &cx)), "ms-settings:appsfeatures");
        assert_eq!(uri(&first(&c, "تحديث ويندوز", &cx)), "ms-settings:windowsupdate");
        assert_eq!(uri(&first(&c, "إدارة الأجهزة", &cx)), "devmgmt.msc");
        assert_eq!(uri(&first(&c, "محرر التسجيل", &cx)), "regedit");
    }

    #[test]
    fn classic_tools_launch_with_arguments() {
        let page = settings_pages::PAGES.iter().find(|p| p.uri == "control printers").unwrap();
        assert_eq!(page_action(page), Action::Launch { target: "control".into(), args: "printers".into() });
        let page = settings_pages::PAGES.iter().find(|p| p.uri == "ms-settings:about").unwrap();
        assert_eq!(page_action(page), Action::OpenUri("ms-settings:about".into()));
    }

    fn score_of(c: &Catalog, q: &str, cx: &Context, title: &str) -> u32 {
        c.query(q, cx, 40).into_iter().find(|i| i.title == title).map_or(0, |i| i.score)
    }

    #[test]
    fn arabic_starts_and_slips() {
        let c = catalog();
        let cx = ctx(false, 0, &no_launches);
        // The start of an Arabic spelling.
        for (q, t) in [("سبوت", "Spotify"), ("فوتو", "Adobe Photoshop 2024"), ("تيليج", "Telegram"), ("ديسك", "Discord")] {
            assert_eq!(first(&c, q, &cx).title, t, "{q}");
        }
        // Several words, the last one still being typed.
        assert_eq!(first(&c, "مايكروسوفت ور", &cx).title, "Word");
        // Common spellings, Iraqi ones too.
        for (q, t) in [("كوكل كروم", "Google Chrome"), ("تلكرام", "Telegram"), ("وتساب", "WhatsApp"), ("ستيم", "Steam")] {
            assert_eq!(first(&c, q, &cx).title, t, "{q}");
        }
        // One sound off, not in the table.
        for (q, t) in [("ديسكود", "Discord"), ("نتفليس", "Netflix"), ("فوتشب", "Adobe Photoshop 2024")] {
            assert_eq!(first(&c, q, &cx).title, t, "{q}");
        }
        // A slip ranks below the exact sound, which is a strong match.
        let exact = score_of(&c, "سبوتفاي", &cx, "Spotify");
        assert!(exact >= STRONG_SCORE, "{exact}");
        assert!(score_of(&c, "ديسكود", &cx, "Discord") < exact);
        assert!(score_of(&c, "ديسكود", &cx, "Discord") > 0);
    }

    #[test]
    fn typos_find_apps() {
        let c = catalog();
        let cx = ctx(true, 0, &no_launches);
        assert_eq!(first(&c, "notepda", &cx).title, "Notepad");
        assert_eq!(first(&c, "discrod", &cx).title, "Discord");
    }

    #[test]
    fn whole_name_then_start_then_typo() {
        let c = catalog();
        let cx = ctx(true, 0, &no_launches);
        let whole = score_of(&c, "notepad", &cx, "Notepad");
        let start = score_of(&c, "notep", &cx, "Notepad");
        let typo = score_of(&c, "notepda", &cx, "Notepad");
        assert!(whole > start && start > typo && typo > 0, "{whole} {start} {typo}");
        // Same tier: the shorter name first.
        let t = titles(&c, "notep", &cx);
        assert_eq!(&t[..2], ["Notepad", "Notepad++"]);
    }

    #[test]
    fn strong_apps_for_the_main_window() {
        let c = catalog();
        let cx = ctx(false, 0, &no_launches);
        let names = |q: &str| c.strong_apps(q, &cx, 3).into_iter().map(|i| i.title).collect::<Vec<_>>();
        assert_eq!(names("كروم"), ["Google Chrome"]);
        assert_eq!(names("سبوت"), ["Spotify"]);
        assert_eq!(names("نتفلكس"), ["Netflix"]);
        assert_eq!(names("notepad"), ["Notepad", "Notepad++"]);
        // Weak matches stay in the quick search: a typo, a slip, a piece of a word, a setting.
        for q in ["notepda", "ديسكود", "otepa", "واي فاي", "تقرير", "2+2", "كل"] {
            assert!(names(q).is_empty(), "{q}: {:?}", names(q));
        }
        assert!(c.strong_apps("note", &cx, 1).len() == 1);
        // Launched often in general: a piece of its name is still weak.
        let notepad = action_key(&Action::Launch { target: "shell:AppsFolder\\Notepad".into(), args: String::new() });
        let often = move |k: &str| if k == notepad { u32::MAX } else { 0 };
        let cx = ctx(true, 0, &often);
        assert!(c.strong_apps("otepa", &cx, 3).is_empty());
        // Picked after these very letters: that habit makes it strong.
        let notepad = action_key(&Action::Launch { target: "shell:AppsFolder\\Notepad".into(), args: String::new() });
        let picked = move |k: &str| if k == notepad { 1 } else { 0 };
        let cx = Context { picked: &picked, ..ctx(true, 0, &no_launches) };
        assert_eq!(c.strong_apps("otepa", &cx, 3).into_iter().map(|i| i.title).collect::<Vec<_>>(), ["Notepad"]);
    }

    #[test]
    fn picks_lead_for_their_letters() {
        let c = catalog();
        let pp = action_key(&Action::Launch { target: "shell:AppsFolder\\Notepad++".into(), args: String::new() });
        let picked = move |k: &str| if k == pp { 1 } else { 0 };
        let cx = Context { picked: &picked, ..ctx(true, 0, &no_launches) };
        assert_eq!(first(&c, "notepad", &cx).title, "Notepad++");
        assert!(pick_boost(1) >= 500 && pick_boost(u32::MAX) <= 700 && pick_boost(0) == 0);
    }

    #[test]
    fn picks_learnt_for_apps_and_settings_only() {
        assert!(learns_pick(&Action::Launch { target: "C:\\x.exe".into(), args: String::new() }));
        assert!(learns_pick(&Action::OpenUri("ms-settings:bluetooth".into())));
        assert!(!learns_pick(&Action::OpenUri("https://www.google.com/search?q=x".into())));
        assert!(!learns_pick(&Action::CopyText("4".into())));
        assert!(!learns_pick(&Action::FocusWindow(1)));
        let c = catalog();
        let cx = ctx(true, 0, &no_launches);
        for it in c.query("restart", &cx, 40).into_iter().filter(|i| matches!(i.kind, Kind::Command | Kind::Web)) {
            assert!(!learns_pick(&it.action), "{:?}", it.action);
        }
    }

    #[test]
    fn launches_boost_reorders() {
        let c = catalog();
        let cx = ctx(true, 0, &no_launches);
        assert_eq!(titles(&c, "note", &cx)[0], "Notepad");
        let pp = action_key(&Action::Launch { target: "shell:AppsFolder\\Notepad++".into(), args: String::new() });
        let often = move |k: &str| if k == pp { 12 } else { 0 };
        let cx = ctx(true, 0, &often);
        assert_eq!(titles(&c, "note", &cx)[0], "Notepad++");
        assert!(launch_boost(1) > 0 && launch_boost(100) > launch_boost(3) && launch_boost(u32::MAX) <= 400);
    }

    #[test]
    fn english_flag_picks_titles() {
        let c = catalog();
        let item = first(&c, "bluetooth", &ctx(true, 0, &no_launches));
        assert_eq!(item.title, "Bluetooth");
        assert_eq!(item.subtitle, "Settings › Bluetooth & devices");
        let item = first(&c, "bluetooth", &ctx(false, 0, &no_launches));
        assert_eq!(item.title, "البلوتوث");
        assert_eq!(item.subtitle, "الإعدادات › البلوتوث والأجهزة");
        assert_eq!(first(&c, "chrome", &ctx(false, 0, &no_launches)).subtitle, "تطبيق");
    }

    #[test]
    fn build_filter_hides_windows_11_pages() {
        let c = catalog();
        let find = |q: &str, build: u32, page: &str| {
            c.query(q, &ctx(true, build, &no_launches), 20).iter().any(|i| uri(i) == page)
        };
        let disks = "ms-settings:disksandvolumes";
        assert!(find("disks volumes", 0, disks) && find("disks volumes", 22631, disks));
        assert!(!find("disks volumes", 19045, disks));
        let tablet = "ms-settings:tabletmode";
        assert!(find("tablet mode", 19045, tablet) && !find("tablet mode", 22631, tablet));
    }

    #[test]
    fn empty_query_limit_and_dedupe() {
        let c = catalog();
        let cx = ctx(false, 0, &no_launches);
        assert!(c.query("", &cx, 8).is_empty());
        assert!(c.query("   \t", &cx, 8).is_empty());
        // Nothing matches: only the "search the web" fallback is left.
        assert!(c.query("zzqqxxvv", &cx, 8).iter().all(|i| i.kind == Kind::Web));
        assert!(c.query("a", &cx, 3).len() <= 3);
        let all = c.query("الصوت", &cx, 50);
        let mut keys: Vec<String> = all.iter().map(|i| action_key(&i.action)).collect();
        let n = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), n);
        assert!(all.windows(2).all(|w| w[0].score >= w[1].score));
    }

    #[test]
    fn catalog_filled_without_new_still_works() {
        let mut c = Catalog::default();
        c.apps.push(app("Google Chrome"));
        let cx = ctx(false, 0, &no_launches);
        assert_eq!(first(&c, "كروم", &cx).title, "Google Chrome");
    }

    #[test]
    fn query_is_fast() {
        let mut apps: Vec<apps::App> = (0..286).map(|i| app(&format!("Program Number {i} Tools"))).collect();
        apps.extend(catalog().apps);
        let c = Catalog::new(apps, settings_pages::PAGES.to_vec());
        let cx = ctx(false, 0, &no_launches);
        let queries = ["ك", "كر", "كروم", "chrome", "واي فاي", "اعدادات الشاشه", "ؤاقخةث", "kroom", "spotfy", "zzzz", "ديسكود", "مايكروسوفت ور"];
        let start = std::time::Instant::now();
        let mut n = 0;
        for i in 0..1000 {
            n += c.query(queries[i % queries.len()], &cx, 8).len();
        }
        let took = start.elapsed();
        assert!(n > 0);
        // Release: well under 1 ms a query. Debug builds are only checked loosely.
        let bound = if cfg!(debug_assertions) { 30 } else { 1 };
        assert!(took < std::time::Duration::from_secs(bound), "{took:?} for 1000 queries");
    }
}
