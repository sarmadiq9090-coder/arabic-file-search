//! EPUB: a zip whose `META-INF/container.xml` points at an OPF package file;
//! the OPF spine gives the reading order of the XHTML content documents.

use std::io::{Read, Seek};
use std::path::Path;

use quick_xml::events::Event;

use super::{MAX_OUTPUT, open_zip, read_zip_entry, strip_html};

/// Upper bound for the number of spine items read.
const MAX_ITEMS: usize = 10_000;

pub(super) fn extract_epub(path: &Path) -> Option<String> {
    let mut zip = open_zip(path)?;
    epub_to_text(&mut zip)
}

pub(super) fn epub_to_text<R: Read + Seek>(zip: &mut zip::ZipArchive<R>) -> Option<String> {
    let mut items = spine_items(zip).unwrap_or_default();
    // A spine whose hrefs don't match any member (case, encoding, base path)
    // is as useless as no spine at all.
    if !items.iter().any(|n| zip.index_for_name(n).is_some()) {
        items.clear();
    }
    if items.is_empty() {
        // No usable OPF: every HTML member, in archive order.
        items = zip
            .file_names()
            .filter_map(|n| n.ok().map(|n| n.into_owned()))
            .filter(|n| {
                let l = n.to_ascii_lowercase();
                l.ends_with(".xhtml") || l.ends_with(".html") || l.ends_with(".htm")
            })
            .take(MAX_ITEMS)
            .collect();
    }
    if items.is_empty() {
        return None;
    }
    let mut out = String::new();
    let mut seen = std::collections::HashSet::new();
    for name in items {
        if out.len() > MAX_OUTPUT {
            break;
        }
        if !seen.insert(name.clone()) {
            continue;
        }
        if let Some(html) = read_zip_entry(zip, &name) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(strip_html(&html).trim());
        }
    }
    Some(out)
}

/// Zip member names of the spine's content documents, in reading order.
fn spine_items<R: Read + Seek>(zip: &mut zip::ZipArchive<R>) -> Option<Vec<String>> {
    let container = read_zip_entry(zip, "META-INF/container.xml")?;
    let opf_path = first_attr(&container, "rootfile", "full-path")?;
    let opf_path = percent_decode(opf_path.trim_start_matches('/'));
    let opf = read_zip_entry(zip, &opf_path)?;
    let base = opf_path.rfind('/').map_or("", |i| &opf_path[..=i]);

    // manifest: id -> (href, media-type); spine: idrefs.
    let mut manifest: std::collections::HashMap<String, (String, String)> = std::collections::HashMap::new();
    let mut spine: Vec<String> = Vec::new();
    let mut reader = quick_xml::Reader::from_str(&opf);
    loop {
        let e = match reader.read_event() {
            Ok(Event::Eof) | Err(_) => break,
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => e,
            Ok(_) => continue,
        };
        let attr = |key: &str| {
            e.attributes()
                .flatten()
                .find(|a| a.key.local_name().as_ref() == key)
                .and_then(|a| {
                    a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .ok()
                        .map(|v| v.into_owned())
                })
        };
        match e.local_name().as_ref() {
            "item" if manifest.len() < MAX_ITEMS * 4 => {
                if let (Some(id), Some(href)) = (attr("id"), attr("href")) {
                    manifest.entry(id).or_insert((href, attr("media-type").unwrap_or_default()));
                }
            }
            "itemref" if spine.len() < MAX_ITEMS => {
                if let Some(idref) = attr("idref") {
                    spine.push(idref);
                }
            }
            _ => {}
        }
    }
    let items = spine
        .iter()
        .filter_map(|idref| manifest.get(idref))
        .filter(|(href, mt)| {
            let h = href.to_ascii_lowercase();
            mt.contains("html")
                || h.ends_with(".xhtml")
                || h.ends_with(".html")
                || h.ends_with(".htm")
        })
        .map(|(href, _)| {
            let href = href.split('#').next().unwrap_or(href);
            resolve(base, &percent_decode(href))
        })
        .collect();
    Some(items)
}

/// Value of `attr` on the first `elem` element of `xml`.
fn first_attr(xml: &str, elem: &str, attr: &str) -> Option<String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    loop {
        match reader.read_event() {
            Ok(Event::Eof) | Err(_) => return None,
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if e.local_name().as_ref() == elem => {
                return e
                    .attributes()
                    .flatten()
                    .find(|a| a.key.local_name().as_ref() == attr)
                    .and_then(|a| {
                        a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                            .ok()
                            .map(|v| v.into_owned())
                    });
            }
            Ok(_) => {}
        }
    }
}

/// Joins a relative href onto the OPF directory, resolving `.` and `..`.
fn resolve(base: &str, href: &str) -> String {
    let joined = if href.starts_with('/') {
        href.trim_start_matches('/').to_owned()
    } else {
        format!("{base}{href}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_owned();
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    fn zip_bytes(entries: &[(&str, &str)]) -> zip::ZipArchive<Cursor<Vec<u8>>> {
        let mut zw = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (n, content) in entries {
            zw.start_file(*n, zip::write::SimpleFileOptions::default())
                .unwrap();
            zw.write_all(content.as_bytes()).unwrap();
        }
        let cur = zw.finish().unwrap();
        zip::ZipArchive::new(Cursor::new(cur.into_inner())).unwrap()
    }

    const CONTAINER: &str = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;

    #[test]
    fn epub_spine_order() {
        let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <manifest>
    <item id="c1" href="text/ch%201.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="../OEBPS/text/ch2.xhtml#top" media-type="application/xhtml+xml"/>
    <item id="css" href="style.css" media-type="text/css"/>
  </manifest>
  <spine><itemref idref="c2"/><itemref idref="css"/><itemref idref="c1"/></spine>
</package>"#;
        let mut zip = zip_bytes(&[
            ("mimetype", "application/epub+zip"),
            ("META-INF/container.xml", CONTAINER),
            ("OEBPS/content.opf", opf),
            (
                "OEBPS/text/ch 1.xhtml",
                "<html><body><p>الفصل الأول</p></body></html>",
            ),
            (
                "OEBPS/text/ch2.xhtml",
                "<html><body><p>الفصل الثاني &amp; التمهيد</p></body></html>",
            ),
            ("OEBPS/style.css", "body{color:red}"),
        ]);
        let text = epub_to_text(&mut zip).unwrap();
        let first = text.find("الفصل الأول").unwrap();
        let second = text.find("الفصل الثاني & التمهيد").unwrap();
        assert!(second < first, "{text}");
        assert!(!text.contains("color"));
    }

    #[test]
    fn epub_fallback_without_opf() {
        let mut zip = zip_bytes(&[
            (
                "META-INF/container.xml",
                "<container><rootfiles></rootfiles",
            ),
            ("a.xhtml", "<p>one</p>"),
            ("b.html", "<p>two</p>"),
            ("c.txt", "three"),
        ]);
        let text = epub_to_text(&mut zip).unwrap();
        assert!(text.contains("one") && text.contains("two") && !text.contains("three"));
        let mut empty = zip_bytes(&[("x.txt", "x")]);
        assert!(epub_to_text(&mut empty).is_none());
    }

    #[test]
    fn resolve_paths() {
        assert_eq!(resolve("OEBPS/", "../a/./b.xhtml"), "a/b.xhtml");
        assert_eq!(resolve("", "/x.html"), "x.html");
        assert_eq!(percent_decode("a%20b%zz"), "a b%zz");
    }
}
