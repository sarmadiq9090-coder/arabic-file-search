//! Mends two kinds of PDF that every viewer shows but pdf-extract (0.12)
//! and lopdf (0.42) cannot read, so their text was never found (FIN1 F2):
//!
//! - [`simple_font_encodings`]: a font with `/Encoding /StandardEncoding`
//!   (or `/BaseEncoding /StandardEncoding`), the built-in encoding of the
//!   standard Type 1 fonts and a common choice of small PDF writers, makes
//!   pdf-extract panic ("unexpected encoding"), and so does any encoding
//!   name it does not know. Standard becomes WinAnsi plus the codes where
//!   the two differ (exact for every code Standard defines); an unknown
//!   name is dropped, as a viewer would ignore it.
//! - [`with_rebuilt_xref`]: lopdf refuses a whole file whose cross-reference
//!   table is malformed (19-byte entries ending in a bare line feed, offsets
//!   that are off). Viewers rebuild the table from the `N G obj` headers;
//!   so does this, on a copy, only when lopdf could not load the file.
//!
//! The mended document still goes through `pdf_guard` before pdf-extract
//! walks it, so the hostile-PDF protections are unchanged.

use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::HashMap;

/// The encodings pdf-extract knows by name.
const KNOWN: &[&[u8]] = &[b"WinAnsiEncoding", b"MacRomanEncoding", b"MacExpertEncoding"];

/// The codes where StandardEncoding names a glyph other than WinAnsiEncoding
/// does (PDF 32000-1, annex D.2). Codes Standard leaves undefined keep the
/// WinAnsi glyph, which a Standard-encoded font never draws.
const STANDARD_OVER_WIN_ANSI: &[(i64, &str)] = &[
    (39, "quoteright"),
    (96, "quoteleft"),
    (164, "fraction"),
    (166, "florin"),
    (168, "currency"),
    (169, "quotesingle"),
    (170, "quotedblleft"),
    (172, "guilsinglleft"),
    (173, "guilsinglright"),
    (174, "fi"),
    (175, "fl"),
    (177, "endash"),
    (178, "dagger"),
    (179, "daggerdbl"),
    (180, "periodcentered"),
    (183, "bullet"),
    (184, "quotesinglbase"),
    (185, "quotedblbase"),
    (186, "quotedblright"),
    (188, "ellipsis"),
    (189, "perthousand"),
    (193, "grave"),
    (194, "acute"),
    (195, "circumflex"),
    (196, "tilde"),
    (197, "macron"),
    (198, "breve"),
    (199, "dotaccent"),
    (200, "dieresis"),
    (202, "ring"),
    (203, "cedilla"),
    (205, "hungarumlaut"),
    (206, "ogonek"),
    (207, "caron"),
    (208, "emdash"),
    (225, "AE"),
    (227, "ordfeminine"),
    (232, "Lslash"),
    (233, "Oslash"),
    (234, "OE"),
    (235, "ordmasculine"),
    (241, "ae"),
    (245, "dotlessi"),
    (248, "lslash"),
    (249, "oslash"),
    (250, "oe"),
    (251, "germandbls"),
];

/// Dictionaries nested deeper than this are left as they are (lopdf's own
/// parser stops at 100 levels).
const MAX_DEPTH: u32 = 128;

/// The `/Differences` array that turns WinAnsi into Standard, before the
/// entries `rest` of the font's own array (which win, coming later).
fn standard_differences(rest: Vec<Object>) -> Object {
    let mut out = Vec::with_capacity(STANDARD_OVER_WIN_ANSI.len() * 2 + rest.len());
    for &(code, name) in STANDARD_OVER_WIN_ANSI {
        out.push(Object::Integer(code));
        out.push(Object::Name(name.as_bytes().to_vec()));
    }
    out.extend(rest);
    Object::Array(out)
}

/// A simple font, whose `/Encoding` pdf-extract reads by name or dictionary.
fn is_simple_font(d: &Dictionary) -> bool {
    matches!(d.get(b"Subtype").and_then(Object::as_name), Ok(b"Type1" | b"MMType1" | b"TrueType" | b"Type3"))
}

/// The name `o` is or points to (`names`: the document's objects that are names).
fn name_of<'a>(o: &'a Object, names: &'a HashMap<ObjectId, Vec<u8>>) -> Option<&'a [u8]> {
    match o {
        Object::Name(n) => Some(n),
        Object::Reference(id) => names.get(id).map(Vec::as_slice),
        _ => None,
    }
}

fn mend_dict(d: &mut Dictionary, names: &HashMap<ObjectId, Vec<u8>>) -> bool {
    let mut changed = false;
    // A font's encoding by name.
    if is_simple_font(d)
        && let Some(name) = d.get(b"Encoding").ok().and_then(|o| name_of(o, names)).map(<[u8]>::to_vec)
        && !KNOWN.contains(&name.as_slice())
    {
        if name == b"StandardEncoding" {
            let mut enc = Dictionary::new();
            enc.set("Type", Object::Name(b"Encoding".to_vec()));
            enc.set("BaseEncoding", Object::Name(b"WinAnsiEncoding".to_vec()));
            enc.set("Differences", standard_differences(Vec::new()));
            d.set("Encoding", Object::Dictionary(enc));
        } else {
            d.remove(b"Encoding");
        }
        changed = true;
    }
    // An encoding dictionary's base (in a font or an object of its own).
    if let Some(base) = d.get(b"BaseEncoding").ok().and_then(|o| name_of(o, names)).map(<[u8]>::to_vec)
        && !KNOWN.contains(&base.as_slice())
    {
        if base == b"StandardEncoding" {
            d.set("BaseEncoding", Object::Name(b"WinAnsiEncoding".to_vec()));
            match d.get(b"Differences") {
                Ok(Object::Array(own)) => {
                    let own = own.clone();
                    d.set("Differences", standard_differences(own));
                }
                // Its own differences elsewhere: WinAnsi as the base is close enough.
                Ok(_) => {}
                Err(_) => d.set("Differences", standard_differences(Vec::new())),
            }
        } else {
            // pdf-extract then starts from its default table.
            d.remove(b"BaseEncoding");
        }
        changed = true;
    }
    changed
}

fn mend(o: &mut Object, names: &HashMap<ObjectId, Vec<u8>>, depth: u32) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    let mut changed = false;
    match o {
        Object::Dictionary(d) => {
            changed |= mend_dict(d, names);
            for (_, v) in d.iter_mut() {
                changed |= mend(v, names, depth + 1);
            }
        }
        Object::Stream(s) => {
            for (_, v) in s.dict.iter_mut() {
                changed |= mend(v, names, depth + 1);
            }
        }
        Object::Array(a) => {
            for v in a {
                changed |= mend(v, names, depth + 1);
            }
        }
        _ => {}
    }
    changed
}

/// Rewrites the simple fonts' encodings pdf-extract would panic on (see the
/// module notes). True when something changed.
pub fn simple_font_encodings(doc: &mut Document) -> bool {
    let names: HashMap<ObjectId, Vec<u8>> =
        doc.objects.iter().filter_map(|(id, o)| o.as_name().ok().map(|n| (*id, n.to_vec()))).collect();
    let mut changed = false;
    for o in doc.objects.values_mut() {
        changed |= mend(o, &names, 0);
    }
    changed
}

// ---------------------------------------------------------------------------
// Cross-reference table
// ---------------------------------------------------------------------------

/// Files larger than this are not copied to rebuild their table.
const MAX_REBUILD: usize = 32 * 1024 * 1024;
/// Highest object number a rebuilt table lists.
const MAX_OBJECT: u32 = 2_000_000;

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'\x0C' | b'\0')
}

/// Whether `b` ends a PDF token (whitespace, a delimiter, or the end).
fn ends_token(b: Option<&u8>) -> bool {
    b.is_none_or(|&b| is_space(b) || b"()<>[]{}/%".contains(&b))
}

/// The digits that end at `end` (exclusive) in `s`, as a number, and where they start.
fn number_before(s: &[u8], end: usize) -> Option<(u64, usize)> {
    let start = s[..end].iter().rposition(|b| !b.is_ascii_digit()).map_or(0, |i| i + 1);
    if start == end || end - start > 10 {
        return None;
    }
    let n = std::str::from_utf8(&s[start..end]).ok()?.parse().ok()?;
    Some((n, start))
}

/// Every `N G obj` header in `s`: object number, generation, offset of N.
/// A later header of the same number wins (an update appended to the file).
fn object_headers(s: &[u8]) -> HashMap<u32, (u16, usize)> {
    let mut found = HashMap::new();
    let mut from = 0;
    while let Some(i) = s[from..].windows(3).position(|w| w == b"obj").map(|i| i + from) {
        from = i + 3;
        if !ends_token(s.get(i + 3)) {
            continue;
        }
        // Back over: spaces, the generation, spaces, the number.
        let mut j = i;
        let spaces = |j: &mut usize| {
            let k = *j;
            while *j > 0 && is_space(s[*j - 1]) {
                *j -= 1;
            }
            *j < k
        };
        if !spaces(&mut j) {
            continue;
        }
        let Some((generation, g_start)) = number_before(s, j) else { continue };
        j = g_start;
        if !spaces(&mut j) {
            continue;
        }
        let Some((number, n_start)) = number_before(s, j) else { continue };
        if n_start > 0 && !is_space(s[n_start - 1]) {
            continue;
        }
        let (Ok(number), Ok(generation)) = (u32::try_from(number), u16::try_from(generation)) else { continue };
        if number == 0 || number > MAX_OBJECT {
            continue;
        }
        found.insert(number, (generation, n_start));
    }
    found
}

/// `N G R` right after the last `key` in `s`, as (N, G).
fn reference_after(s: &[u8], key: &[u8]) -> Option<(u32, u16)> {
    let at = s.windows(key.len()).rposition(|w| w == key)? + key.len();
    if !ends_token(s.get(at)) {
        return None;
    }
    let mut parts = std::str::from_utf8(&s[at..s.len().min(at + 40)]).ok()?.split_ascii_whitespace();
    let n = parts.next()?.parse().ok()?;
    let g = parts.next()?.parse().ok()?;
    let r = parts.next()?;
    (r == "R" || r.starts_with("R/") || r.starts_with("R>")).then_some((n, g))
}

/// A copy of the PDF in `bytes` with a new cross-reference table made from
/// its object headers appended, when its own cannot be read: `None` when no
/// catalog is found, or the file is encrypted (its trailer holds the key's
/// details) or too large to copy.
pub fn with_rebuilt_xref(bytes: &[u8]) -> Option<Vec<u8>> {
    // lopdf counts offsets from the header, wherever it starts.
    let start = bytes.windows(5).position(|w| w == b"%PDF-")?;
    let s = &bytes[start..];
    if s.len() > MAX_REBUILD {
        return None;
    }
    let trailer = s.windows(7).rposition(|w| w == b"trailer").map(|i| &s[i..s.len().min(i + 4096)]);
    if trailer.is_some_and(|t| t.windows(8).any(|w| w == b"/Encrypt")) {
        return None;
    }
    let objects = object_headers(s);
    let max = *objects.keys().max()?;
    // A sparse table (a stray "999999 0 obj" in a stream) is not worth listing.
    if max as usize > objects.len().saturating_mul(8) + 1024 {
        return None;
    }
    let catalog = || {
        let mut ids: Vec<(&u32, &(u16, usize))> = objects.iter().collect();
        ids.sort_by_key(|(_, (_, off))| *off);
        ids.iter().enumerate().rev().find_map(|(k, (n, (g, off)))| {
            let end = ids.get(k + 1).map_or(s.len(), |(_, (_, o))| *o).min(off + 4096);
            let body = &s[*off..end];
            let catalog = body.windows(8).any(|w| w == b"/Catalog") && body.windows(5).any(|w| w == b"/Type");
            catalog.then_some((**n, *g))
        })
    };
    let (root, root_gen) = trailer.and_then(|t| reference_after(t, b"/Root")).filter(|(n, _)| objects.contains_key(n)).or_else(catalog)?;
    let mut out = Vec::with_capacity(bytes.len() + (max as usize + 1) * 20 + 128);
    out.extend_from_slice(s);
    out.extend_from_slice(b"\n");
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f\r\n", max + 1).bytes());
    for n in 1..=max {
        match objects.get(&n) {
            Some((g, off)) => out.extend(format!("{off:010} {g:05} n\r\n").bytes()),
            None => out.extend_from_slice(b"0000000000 65535 f\r\n"),
        }
    }
    out.extend(format!("trailer\n<< /Size {} /Root {root} {root_gen} R >>\nstartxref\n{xref}\n%%EOF\n", max + 1).bytes());
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font(enc: &str) -> Dictionary {
        let pdf = crate::extract::hostile::pdf_of(&[format!("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica {enc} >>")]);
        let doc = Document::load_mem(&pdf).unwrap();
        doc.get_object((1, 0)).unwrap().as_dict().unwrap().clone()
    }

    fn mended(enc: &str) -> (bool, Dictionary) {
        let mut o = Object::Dictionary(font(enc));
        let changed = mend(&mut o, &HashMap::new(), 0);
        (changed, o.as_dict().unwrap().clone())
    }

    #[test]
    fn standard_encoding_becomes_win_ansi_with_differences() {
        let (changed, d) = mended("/Encoding /StandardEncoding");
        assert!(changed);
        let enc = d.get(b"Encoding").unwrap().as_dict().unwrap();
        assert_eq!(enc.get(b"BaseEncoding").unwrap().as_name().unwrap(), b"WinAnsiEncoding");
        let diffs = enc.get(b"Differences").unwrap().as_array().unwrap();
        assert_eq!(diffs.len(), STANDARD_OVER_WIN_ANSI.len() * 2);
        assert_eq!(diffs[0], Object::Integer(39));
        assert_eq!(diffs[1], Object::Name(b"quoteright".to_vec()));
        // The font's own differences come after Standard's, so they win.
        let (_, d) = mended("/Encoding << /BaseEncoding /StandardEncoding /Differences [65 /B] >>");
        let enc = d.get(b"Encoding").unwrap().as_dict().unwrap();
        let diffs = enc.get(b"Differences").unwrap().as_array().unwrap();
        assert_eq!(&diffs[diffs.len() - 2..], &[Object::Integer(65), Object::Name(b"B".to_vec())]);
        // Unknown names go; the ones pdf-extract knows stay.
        let (changed, d) = mended("/Encoding /PDFDocEncoding");
        assert!(changed && d.get(b"Encoding").is_err());
        let (changed, d) = mended("/Encoding << /BaseEncoding /Nonsense >>");
        assert!(changed && d.get(b"Encoding").unwrap().as_dict().unwrap().get(b"BaseEncoding").is_err());
        for known in ["/Encoding /WinAnsiEncoding", "/Encoding /MacRomanEncoding", "", "/Encoding << /Differences [32 /space] >>"] {
            assert!(!mended(known).0, "{known}");
        }
        // Type 0 fonts name their CMap there: not touched.
        let mut t0 = font("/Encoding /Identity-H");
        t0.set("Subtype", Object::Name(b"Type0".to_vec()));
        assert!(!mend_dict(&mut t0, &HashMap::new()));
    }

    #[test]
    fn encodings_reached_through_references_are_mended() {
        let pdf = crate::extract::hostile::pdf_of(&[
            "<< /Type /Font /Subtype /TrueType /BaseFont /Arial /Encoding 2 0 R >>".to_owned(),
            "/StandardEncoding".to_owned(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /Encoding 4 0 R >>".to_owned(),
            "<< /Type /Encoding /BaseEncoding /StandardEncoding /Differences [32 /space] >>".to_owned(),
        ]);
        let mut doc = Document::load_mem(&pdf).unwrap();
        assert!(simple_font_encodings(&mut doc));
        let f = doc.get_object((1, 0)).unwrap().as_dict().unwrap();
        assert!(f.get(b"Encoding").unwrap().as_dict().is_ok());
        let e = doc.get_object((4, 0)).unwrap().as_dict().unwrap();
        assert_eq!(e.get(b"BaseEncoding").unwrap().as_name().unwrap(), b"WinAnsiEncoding");
        assert_eq!(e.get(b"Differences").unwrap().as_array().unwrap().len(), STANDARD_OVER_WIN_ANSI.len() * 2 + 2);
        // Nothing left to mend the second time.
        assert!(!simple_font_encodings(&mut doc));
    }

    #[test]
    fn rebuilt_xref_lists_every_object() {
        let objs = ["<< /Type /Catalog /Pages 2 0 R >>".to_owned(), "<< /Type /Pages /Kids [] /Count 0 >>".to_owned()];
        let good = crate::extract::hostile::pdf_of(&objs);
        // Entries of 19 bytes (a bare line feed): lopdf refuses the file.
        let bad = String::from_utf8(good.clone()).unwrap().replace(" n \n", " n\n").replace(" f \n", " f\n").into_bytes();
        assert!(Document::load_mem(&bad).is_err());
        let fixed = with_rebuilt_xref(&bad).unwrap();
        let doc = Document::load_mem(&fixed).unwrap();
        assert!(doc.catalog().is_ok());
        assert_eq!(doc.get_object((2, 0)).unwrap().as_dict().unwrap().get(b"Type").unwrap().as_name().unwrap(), b"Pages");
        // Offsets that point nowhere, and no trailer at all: the catalog is found by its type.
        let mut no_trailer = good.clone();
        let x = no_trailer.windows(4).rposition(|w| w == b"xref").unwrap();
        no_trailer.truncate(x);
        assert!(Document::load_mem(&fixed).is_ok());
        let doc = Document::load_mem(&with_rebuilt_xref(&no_trailer).unwrap()).unwrap();
        assert!(doc.catalog().is_ok());
        // Bytes before the header (a byte order mark) are skipped, as lopdf does.
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend(&bad);
        assert!(Document::load_mem(&with_rebuilt_xref(&bom).unwrap()).unwrap().catalog().is_ok());
    }

    #[test]
    fn rebuilt_xref_refuses_what_it_cannot_mend() {
        assert!(with_rebuilt_xref(b"").is_none());
        assert!(with_rebuilt_xref(b"not a pdf 1 0 obj << >> endobj").is_none());
        // No catalog anywhere.
        assert!(with_rebuilt_xref(b"%PDF-1.4\n1 0 obj\n<< /Type /Pages >>\nendobj\n%%EOF").is_none());
        // Encrypted: the trailer's /Encrypt cannot be carried over safely.
        let enc = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\ntrailer\n<< /Root 1 0 R /Encrypt 2 0 R >>\n%%EOF";
        assert!(with_rebuilt_xref(enc).is_none());
        // A huge stray object number does not make a huge table.
        let sparse = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\n1999999 0 obj\n<< >>\nendobj\n";
        assert!(with_rebuilt_xref(sparse).is_none());
        // "endobj", numbers glued to words, and numbers too large are not headers.
        let h = object_headers(b"x1 0 obj 12345678901 0 obj 7 70000 obj endobj 3 0 objx\n4 0 obj");
        assert_eq!(h.keys().copied().collect::<Vec<_>>(), vec![4]);
        assert_eq!(reference_after(b"/Root 12 0 R>>", b"/Root"), Some((12, 0)));
        assert_eq!(reference_after(b"/RootX 12 0 R", b"/Root"), None);
    }
}
