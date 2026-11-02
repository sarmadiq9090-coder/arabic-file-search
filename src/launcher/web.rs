//! Web shortcuts: "يوتيوب قرآن" searches YouTube, plus a "search the web" fallback.

use super::{Action, Context, Item, Kind};

/// Score of a site shortcut: the user named the site, so it leads.
pub const SHORTCUT_SCORE: u32 = 900;
/// Score of the generic "search the web" row: always last.
pub const FALLBACK_SCORE: u32 = 1;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Site {
    YouTube,
    Google,
    Wikipedia,
    Maps,
    Translate,
    Images,
    Amazon,
    X,
}

/// Words that start a shortcut (compared after Arabic normalisation).
const PREFIXES: &[(&str, Site)] = &[
    ("يوتيوب", Site::YouTube),
    ("يوتوب", Site::YouTube),
    ("youtube", Site::YouTube),
    ("yt", Site::YouTube),
    ("جوجل", Site::Google),
    ("قوقل", Site::Google),
    ("غوغل", Site::Google),
    ("google", Site::Google),
    ("g", Site::Google),
    ("ويكيبيديا", Site::Wikipedia),
    ("ويكي", Site::Wikipedia),
    ("wikipedia", Site::Wikipedia),
    ("wiki", Site::Wikipedia),
    ("خرائط", Site::Maps),
    ("خريطة", Site::Maps),
    ("maps", Site::Maps),
    ("map", Site::Maps),
    ("ترجم", Site::Translate),
    ("ترجمة", Site::Translate),
    ("translate", Site::Translate),
    ("صور", Site::Images),
    ("images", Site::Images),
    ("image", Site::Images),
    ("img", Site::Images),
    ("أمازون", Site::Amazon),
    ("امازون", Site::Amazon),
    ("amazon", Site::Amazon),
    ("x", Site::X),
    ("تويتر", Site::X),
    ("twitter", Site::X),
];

fn site_name(site: Site, english: bool) -> &'static str {
    match (site, english) {
        (Site::YouTube, false) => "يوتيوب",
        (Site::YouTube, true) => "YouTube",
        (Site::Google, false) => "جوجل",
        (Site::Google, true) => "Google",
        (Site::Wikipedia, false) => "ويكيبيديا",
        (Site::Wikipedia, true) => "Wikipedia",
        (Site::Maps, false) => "خرائط جوجل",
        (Site::Maps, true) => "Google Maps",
        (Site::Translate, false) => "ترجمة جوجل",
        (Site::Translate, true) => "Google Translate",
        (Site::Images, false) => "صور جوجل",
        (Site::Images, true) => "Google Images",
        (Site::Amazon, false) => "أمازون",
        (Site::Amazon, true) => "Amazon",
        (Site::X, false) => "X (تويتر)",
        (Site::X, true) => "X (Twitter)",
    }
}

/// Percent-encodes `s` for a URL query value (UTF-8, RFC 3986 unreserved
/// characters kept, everything else `%XX`, spaces as `%20`).
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            out.push('%');
            out.push(HEX[usize::from(b >> 4)] as char);
            out.push(HEX[usize::from(b & 15)] as char);
        }
    }
    out
}

fn has_arabic(s: &str) -> bool {
    s.chars().any(|c| matches!(c, '\u{0600}'..='\u{06FF}' | '\u{0750}'..='\u{077F}' | '\u{08A0}'..='\u{08FF}' | '\u{FB50}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}'))
}

fn url(site: Site, terms: &str) -> String {
    let q = percent_encode(terms);
    match site {
        Site::YouTube => format!("https://www.youtube.com/results?search_query={q}"),
        Site::Google => format!("https://www.google.com/search?q={q}"),
        Site::Wikipedia => {
            let lang = if has_arabic(terms) { "ar" } else { "en" };
            format!("https://{lang}.wikipedia.org/w/index.php?search={q}")
        }
        Site::Maps => format!("https://www.google.com/maps/search/?api=1&query={q}"),
        Site::Translate => {
            let (sl, tl) = if has_arabic(terms) { ("ar", "en") } else { ("auto", "ar") };
            format!("https://translate.google.com/?sl={sl}&tl={tl}&text={q}&op=translate")
        }
        Site::Images => format!("https://www.google.com/search?tbm=isch&q={q}"),
        Site::Amazon => format!("https://www.amazon.com/s?k={q}"),
        Site::X => format!("https://x.com/search?q={q}"),
    }
}

fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let h = rest.split(['/', '?']).next().unwrap_or(rest);
    h.strip_prefix("www.").unwrap_or(h)
}

/// Strips invisible direction marks and trims, so that a pasted "‏يوتيوب ‏قرآن" still works.
fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{061C}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}'))
        .collect::<String>()
        .trim()
        .to_string()
}

/// A site shortcut: the site and the words to search for.
fn shortcut(query: &str) -> Option<(Site, String)> {
    let (first, rest) = query.split_once(char::is_whitespace)?;
    let first = first.trim_end_matches([':', '：']);
    let key = crate::normalize::normalize(first);
    let site = PREFIXES.iter().find(|(p, _)| crate::normalize::normalize(p) == key)?.1;
    let terms = rest.split_whitespace().collect::<Vec<_>>().join(" ");
    if terms.is_empty() { None } else { Some((site, terms)) }
}

fn web_item(title: String, url: String, score: u32) -> Item {
    Item { kind: Kind::Web, title, subtitle: host(&url).to_string(), action: Action::OpenUri(url), score, icon_path: None }
}

/// Web-search results for `query` (a site shortcut, or the generic fallback when `fallback`).
pub fn shortcuts(query: &str, ctx: &Context, fallback: bool) -> Vec<Item> {
    let q = clean(query);
    if q.is_empty() || q.len() > 2000 {
        return Vec::new();
    }
    let en = ctx.english;
    if let Some((site, terms)) = shortcut(&q) {
        let name = site_name(site, en);
        let title = match (site, en) {
            (Site::Translate, false) => format!("ترجم «{terms}»"),
            (Site::Translate, true) => format!("Translate “{terms}”"),
            (Site::Maps, false) => format!("ابحث في الخرائط عن «{terms}»"),
            (Site::Maps, true) => format!("Search Google Maps for “{terms}”"),
            (_, false) => format!("ابحث في {name} عن «{terms}»"),
            (_, true) => format!("Search {name} for “{terms}”"),
        };
        return vec![web_item(title, url(site, &terms), SHORTCUT_SCORE)];
    }
    // A bare prefix ("يوتيوب") is not a search yet.
    let key = crate::normalize::normalize(q.trim_end_matches([':', '：']));
    let bare_prefix = PREFIXES.iter().any(|(p, _)| crate::normalize::normalize(p) == key);
    if fallback && !bare_prefix {
        let terms = q.split_whitespace().collect::<Vec<_>>().join(" ");
        let title = if en { format!("Search the web for “{terms}”") } else { format!("ابحث في الويب عن «{terms}»") };
        return vec![web_item(title, url(Site::Google, &terms), FALLBACK_SCORE)];
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(english: bool) -> Context<'static> {
        static NO_LAUNCHES: fn(&str) -> u32 = |_| 0;
        Context { english, windows_build: 0, windows: &[], launches: &NO_LAUNCHES, picked: &NO_LAUNCHES, today: (2026, 11, 1) }
    }

    fn one(q: &str, fallback: bool) -> Option<Item> {
        let v = shortcuts(q, &ctx(false), fallback);
        assert!(v.len() <= 1);
        v.into_iter().next()
    }

    fn uri(q: &str) -> Option<String> {
        one(q, false).map(|i| match i.action {
            Action::OpenUri(u) => u,
            other => panic!("{other:?}"),
        })
    }

    #[test]
    fn encoding() {
        assert_eq!(percent_encode("قرآن"), "%D9%82%D8%B1%D8%A2%D9%86");
        assert_eq!(percent_encode("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
        assert_eq!(percent_encode("A-z_0.9~"), "A-z_0.9~");
        assert_eq!(percent_encode("😀"), "%F0%9F%98%80");
        assert_eq!(percent_encode("+%#?"), "%2B%25%23%3F");
        assert_eq!(percent_encode(""), "");
    }

    #[test]
    fn site_shortcuts() {
        assert_eq!(uri("يوتيوب قرآن").unwrap(), "https://www.youtube.com/results?search_query=%D9%82%D8%B1%D8%A2%D9%86");
        assert_eq!(uri("youtube lofi music").unwrap(), "https://www.youtube.com/results?search_query=lofi%20music");
        assert_eq!(uri("YT cats").unwrap(), "https://www.youtube.com/results?search_query=cats");
        assert_eq!(uri("جوجل الطقس").unwrap(), "https://www.google.com/search?q=%D8%A7%D9%84%D8%B7%D9%82%D8%B3");
        assert_eq!(uri("قوقل rust").unwrap(), "https://www.google.com/search?q=rust");
        assert_eq!(uri("g rust lang").unwrap(), "https://www.google.com/search?q=rust%20lang");
        assert_eq!(uri("ويكي مكة").unwrap(), "https://ar.wikipedia.org/w/index.php?search=%D9%85%D9%83%D8%A9");
        assert_eq!(uri("wiki Mecca").unwrap(), "https://en.wikipedia.org/w/index.php?search=Mecca");
        assert_eq!(uri("ويكيبيديا  ابن   سينا").unwrap(), "https://ar.wikipedia.org/w/index.php?search=%D8%A7%D8%A8%D9%86%20%D8%B3%D9%8A%D9%86%D8%A7");
        assert_eq!(uri("خرائط الرياض").unwrap(), "https://www.google.com/maps/search/?api=1&query=%D8%A7%D9%84%D8%B1%D9%8A%D8%A7%D8%B6");
        assert_eq!(uri("ترجم hello").unwrap(), "https://translate.google.com/?sl=auto&tl=ar&text=hello&op=translate");
        assert_eq!(uri("ترجمة مرحبا").unwrap(), "https://translate.google.com/?sl=ar&tl=en&text=%D9%85%D8%B1%D8%AD%D8%A8%D8%A7&op=translate");
        assert_eq!(uri("translate good morning").unwrap(), "https://translate.google.com/?sl=auto&tl=ar&text=good%20morning&op=translate");
        assert_eq!(uri("صور قطط").unwrap(), "https://www.google.com/search?tbm=isch&q=%D9%82%D8%B7%D8%B7");
        assert_eq!(uri("images cats").unwrap(), "https://www.google.com/search?tbm=isch&q=cats");
        assert_eq!(uri("أمازون كتب").unwrap(), "https://www.amazon.com/s?k=%D9%83%D8%AA%D8%A8");
        assert_eq!(uri("امازون kindle").unwrap(), "https://www.amazon.com/s?k=kindle");
        assert_eq!(uri("x rust").unwrap(), "https://x.com/search?q=rust");
        assert_eq!(uri("تويتر اخبار").unwrap(), "https://x.com/search?q=%D8%A7%D8%AE%D8%A8%D8%A7%D8%B1");
        assert_eq!(uri("youtube: cats").unwrap(), "https://www.youtube.com/results?search_query=cats");
        assert_eq!(uri("\u{200F}يوتيوب \u{200F}قرآن").unwrap(), "https://www.youtube.com/results?search_query=%D9%82%D8%B1%D8%A2%D9%86");
        assert_eq!(uri("youtube a&b=c").unwrap(), "https://www.youtube.com/results?search_query=a%26b%3Dc");
    }

    #[test]
    fn titles() {
        let i = one("يوتيوب قرآن", false).unwrap();
        assert_eq!(i.kind, Kind::Web);
        assert_eq!(i.title, "ابحث في يوتيوب عن «قرآن»");
        assert_eq!(i.subtitle, "youtube.com");
        let e = shortcuts("youtube cats", &ctx(true), false).remove(0);
        assert_eq!(e.title, "Search YouTube for “cats”");
        assert_eq!(one("ترجم hello", false).unwrap().title, "ترجم «hello»");
        assert_eq!(one("wiki Mecca", false).unwrap().subtitle, "en.wikipedia.org");
    }

    #[test]
    fn fallback_and_non_shortcuts() {
        assert!(one("يوتيوب", false).is_none());
        assert!(one("يوتيوب", true).is_none());
        assert!(one("youtube   ", true).is_none());
        assert!(one("youtube:", true).is_none());
        assert!(one("", true).is_none());
        assert!(one("   ", true).is_none());
        assert!(one("تقرير المبيعات", false).is_none());
        assert!(one("youtuber notes", false).is_none());
        assert!(one("gx rust", false).is_none());
        let f = one("تقرير المبيعات", true).unwrap();
        assert_eq!(f.title, "ابحث في الويب عن «تقرير المبيعات»");
        assert_eq!(f.action, Action::OpenUri("https://www.google.com/search?q=%D8%AA%D9%82%D8%B1%D9%8A%D8%B1%20%D8%A7%D9%84%D9%85%D8%A8%D9%8A%D8%B9%D8%A7%D8%AA".into()));
        assert_eq!(f.score, FALLBACK_SCORE);
        let e = shortcuts("rust book", &ctx(true), true).remove(0);
        assert_eq!(e.title, "Search the web for “rust book”");
        // A shortcut wins over the fallback, and there is only one row.
        assert_eq!(shortcuts("يوتيوب قرآن", &ctx(false), true).len(), 1);
        assert!(one("😀", true).is_some());
    }

    #[test]
    fn host_parsing() {
        assert_eq!(host("https://www.google.com/search?q=a"), "google.com");
        assert_eq!(host("https://x.com/search?q=a"), "x.com");
        assert_eq!(host("nothing"), "nothing");
    }
}
