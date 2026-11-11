//! The new interface's files, embedded in the exe, and how they are served:
//! only from the `instafind://` protocol (no network port, nothing from the
//! internet), each with a strict Content Security Policy.

use crate::i18n::Lang;

/// The custom protocol's name. WebView2 sees it as `http://instafind.localhost/`
/// and answers it inside the program; the request never reaches the network.
pub const SCHEME: &str = "instafind";
/// What the page is opened with (wry rewrites it to [`ORIGIN`]).
pub const START_URL: &str = "instafind://localhost/";
/// The only origin whose messages the program accepts.
pub const ORIGIN: &str = "http://instafind.localhost";

/// Whether `url` is the program's own page: exactly [`ORIGIN`] (alone or
/// followed by a path), or [`START_URL`] and its paths. A longer host
/// (`http://instafind.localhost.evil.example/`), a port or credentials are
/// someone else's. The messages, navigations and new windows the program
/// allows are checked with this.
pub fn own_url(url: &str) -> bool {
    url.strip_prefix(ORIGIN).is_some_and(|rest| rest.is_empty() || rest.starts_with('/')) || url.starts_with(START_URL)
}

/// Scripts, styles and fonts only from the program itself; no inline script,
/// no `eval`, no network, no plug-ins, no frames, no forms.
pub const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; font-src 'self'; \
img-src 'self' data:; connect-src 'none'; media-src 'none'; object-src 'none'; frame-src 'none'; \
worker-src 'none'; manifest-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

const PAGE: &str = include_str!("assets/index.html");

/// The served files other than the page: (path, contents, media type).
const FILES: &[(&str, &[u8], &str)] = &[
    ("/app.js", include_bytes!("assets/app.js"), "text/javascript; charset=utf-8"),
    ("/app.css", include_bytes!("assets/app.css"), "text/css; charset=utf-8"),
    ("/fonts/IBMPlexSans-Regular-Latin1.woff2", include_bytes!("assets/fonts/IBMPlexSans-Regular-Latin1.woff2"), "font/woff2"),
    ("/fonts/IBMPlexSans-Medium-Latin1.woff2", include_bytes!("assets/fonts/IBMPlexSans-Medium-Latin1.woff2"), "font/woff2"),
    ("/fonts/IBMPlexSans-SemiBold-Latin1.woff2", include_bytes!("assets/fonts/IBMPlexSans-SemiBold-Latin1.woff2"), "font/woff2"),
    ("/fonts/IBMPlexSansArabic-Regular.woff2", include_bytes!("assets/fonts/IBMPlexSansArabic-Regular.woff2"), "font/woff2"),
    ("/fonts/IBMPlexSansArabic-Medium.woff2", include_bytes!("assets/fonts/IBMPlexSansArabic-Medium.woff2"), "font/woff2"),
    ("/fonts/IBMPlexSansArabic-SemiBold.woff2", include_bytes!("assets/fonts/IBMPlexSansArabic-SemiBold.woff2"), "font/woff2"),
];

/// The page for `lang`, carrying this launch's `token`.
///
/// The token must be plain hex (as [`super::bridge::new_token`] makes it):
/// anything else is refused so it can't break out of its attribute.
pub fn page(lang: Lang, token: &str) -> Option<String> {
    if token.is_empty() || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let (code, dir) = match lang {
        Lang::Ar => ("ar", "rtl"),
        Lang::En => ("en", "ltr"),
    };
    Some(PAGE.replace("{{LANG}}", code).replace("{{DIR}}", dir).replace("{{CSP}}", &meta_csp()).replace("{{TOKEN}}", token))
}

/// [`CSP`] for the page's own `<meta>` copy, which may not hold
/// `frame-ancestors` (the header does).
fn meta_csp() -> String {
    CSP.replace("; frame-ancestors 'none'", "")
}

/// An answer of the protocol: status, media type and body.
pub struct Reply {
    pub status: u16,
    pub mime: &'static str,
    pub body: std::borrow::Cow<'static, [u8]>,
}

/// The answer for a request of `method` for `path` (the URL's path, without
/// the query). `page` is the filled page from [`page`]. Only GET (and HEAD)
/// of the known files; everything else is 404 or 405.
pub fn serve(method: &str, path: &str, page: &str) -> Reply {
    let not = |status| Reply { status, mime: "text/plain; charset=utf-8", body: std::borrow::Cow::Borrowed(b"") };
    if method != "GET" && method != "HEAD" {
        return not(405);
    }
    if path == "/" || path == "/index.html" {
        return Reply { status: 200, mime: "text/html; charset=utf-8", body: std::borrow::Cow::Owned(page.as_bytes().to_vec()) };
    }
    match FILES.iter().find(|(p, ..)| *p == path) {
        Some((_, body, mime)) => Reply { status: 200, mime, body: std::borrow::Cow::Borrowed(body) },
        None => not(404),
    }
}

/// The headers every answer carries besides its type.
pub const HEADERS: &[(&str, &str)] = &[
    ("Content-Security-Policy", CSP),
    ("X-Content-Type-Options", "nosniff"),
    ("Referrer-Policy", "no-referrer"),
    ("Cache-Control", "no-store"),
];

#[cfg(test)]
mod tests {
    #[test]
    fn only_the_programs_own_urls() {
        for ok in ["http://instafind.localhost", "http://instafind.localhost/", "http://instafind.localhost/index.html", "instafind://localhost/", "instafind://localhost/app.js"] {
            assert!(super::own_url(ok), "{ok}");
        }
        for bad in [
            "http://instafind.localhost.evil.example/",
            "http://instafind.localhost:8080/",
            "http://instafind.localhost@evil.example/",
            "http://instafind.localhostx/",
            "https://instafind.localhost/",
            "http://evil.example/http://instafind.localhost/",
            "instafind://localhost.evil.example/",
            "instafind://localhost@evil/",
            "file:///C:/x.html",
            "about:blank",
            "",
        ] {
            assert!(!super::own_url(bad), "{bad}");
        }
    }

    use super::*;

    fn text(path: &str) -> String {
        let r = serve("GET", path, "");
        assert_eq!(r.status, 200, "{path}");
        String::from_utf8(r.body.into_owned()).unwrap()
    }

    #[test]
    fn page_is_filled_and_token_is_checked() {
        let p = page(Lang::Ar, "0123abcdef").unwrap();
        assert!(p.contains(r#"<html lang="ar" dir="rtl">"#));
        assert!(p.contains(r#"content="0123abcdef""#));
        assert!(p.contains("default-src 'none'; script-src 'self';") && !p.contains("frame-ancestors"));
        assert!(CSP.ends_with("; frame-ancestors 'none'"));
        assert!(!p.contains("{{"));
        assert!(page(Lang::En, "abc").unwrap().contains(r#"<html lang="en" dir="ltr">"#));
        for bad in ["", "ab\"cd", "<script>", "12 34"] {
            assert!(page(Lang::Ar, bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn serves_only_known_files_by_get() {
        assert_eq!(serve("GET", "/", "x").status, 200);
        assert_eq!(&*serve("GET", "/index.html", "page").body, b"page");
        assert_eq!(serve("GET", "/app.js", "").mime, "text/javascript; charset=utf-8");
        assert_eq!(serve("HEAD", "/app.css", "").status, 200);
        assert_eq!(serve("GET", "/../Cargo.toml", "").status, 404);
        assert_eq!(serve("GET", "/fonts/", "").status, 404);
        assert_eq!(serve("POST", "/app.js", "").status, 405);
    }

    #[test]
    fn every_reference_is_embedded() {
        // The page's own files…
        let mut refs: Vec<String> = Vec::new();
        for attr in ["src=\"", "href=\""] {
            for part in PAGE.split(attr).skip(1) {
                refs.push(format!("/{}", &part[..part.find('"').unwrap()]));
            }
        }
        // …and the fonts of the style sheet.
        for part in text("/app.css").split("url(").skip(1) {
            refs.push(format!("/{}", &part[..part.find(')').unwrap()]));
        }
        assert!(refs.len() >= 8, "{refs:?}");
        for r in refs {
            assert_eq!(serve("GET", &r, "").status, 200, "{r} is referenced but not embedded");
        }
    }

    #[test]
    fn nothing_from_the_network_and_no_dynamic_code() {
        assert!(!CSP.contains("http") && !CSP.contains("unsafe") && !CSP.contains('*'));
        let js = text("/app.js");
        for bad in ["eval(", "new Function", "innerHTML", "outerHTML", "insertAdjacentHTML", "document.write", "fetch(", "XMLHttpRequest", "WebSocket", "import("] {
            assert!(!js.contains(bad), "app.js uses {bad}");
        }
        // Only the SVG namespace, which is a name, not a request.
        let urls: Vec<&str> = js.match_indices("http").map(|(i, _)| &js[i..(i + 30).min(js.len())]).collect();
        assert!(urls.iter().all(|u| u.starts_with("http://www.w3.org/2000/svg")), "{urls:?}");
        let css = text("/app.css");
        assert!(!css.contains("http:") && !css.contains("https:") && !css.contains("@import"));
        assert!(!PAGE.contains("http:") && !PAGE.contains("https:") && !PAGE.contains("<script>") && !PAGE.contains("style=\""));
    }
}
