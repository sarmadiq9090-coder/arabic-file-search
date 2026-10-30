//! Page images of scanned PDFs, for OCR.
//!
//! A scanned PDF has (almost) no text layer: each page is one or more image
//! XObjects. [`page_images`] finds them (also inside form XObjects) and turns
//! each into an encoded image file the OS decoder understands: JPEG streams
//! (DCTDecode) pass through as they are, raw Flate/uncompressed samples in
//! DeviceGray (1 or 8 bits) or DeviceRGB (8 bits) become BMP files. Anything
//! else (JPEG 2000, CCITT, JBIG2, indexed or CMYK raw samples) is skipped.
//! This part is platform independent; the OCR glue is Windows-only.

use std::collections::HashSet;
use std::io::Read;

use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

/// Pages whose images are looked at.
pub const MAX_PAGES: usize = 50;
/// A PDF whose text averages fewer non-space characters per page than this
/// counts as scanned.
pub const SCANNED_CHARS_PER_PAGE: usize = 20;
/// Upper bound for one image's decoded samples (decompression-bomb guard).
const MAX_RAW_BYTES: usize = 64 * 1024 * 1024;
/// Images with a side shorter than this (rules, bullets, logos) are skipped.
const MIN_SIDE: i64 = 32;
/// Form XObjects nested deeper than this are not entered.
const MAX_FORM_DEPTH: u32 = 4;

/// Whether `text` is too sparse for a PDF of `pages` pages to have a real
/// text layer.
pub fn looks_scanned(text: &str, pages: usize) -> bool {
    let chars = text.chars().filter(|c| !c.is_whitespace()).count();
    pages > 0 && chars < SCANNED_CHARS_PER_PAGE * pages
}

/// Number of pages in `doc`.
pub fn page_count(doc: &Document) -> usize {
    doc.get_pages().len()
}

/// Encoded images (JPEG or BMP files) of the first `max_pages` pages, in page
/// order. An image shared by several pages is returned once.
#[cfg_attr(not(test), allow(dead_code))]
pub fn page_images(doc: &Document, max_pages: usize) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for_each_page_image(doc, max_pages, |img| {
        out.push(img);
        true
    });
    out
}

/// Like [`page_images`] but hands each image to `f` as soon as it is found,
/// stopping when `f` returns false.
pub fn for_each_page_image(doc: &Document, max_pages: usize, mut f: impl FnMut(Vec<u8>) -> bool) {
    let mut seen = HashSet::new();
    for (_, page_id) in doc.get_pages().into_iter().take(max_pages) {
        let mut ids = Vec::new();
        if let Some(res) = page_resources(doc, page_id) {
            collect_images(doc, res, 0, &mut ids);
        }
        for id in ids {
            if !seen.insert(id) {
                continue;
            }
            if let Ok(Object::Stream(s)) = doc.get_object(id)
                && let Some(img) = encode_image(doc, s)
                && !f(img)
            {
                return;
            }
        }
    }
}

fn deref<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Object> {
    doc.dereference(obj).ok().map(|(_, o)| o)
}

fn dict_entry<'a>(doc: &'a Document, dict: &'a Dictionary, key: &[u8]) -> Option<&'a Object> {
    deref(doc, dict.get(key).ok()?)
}

/// The page's /Resources, inherited from the page tree when absent.
fn page_resources(doc: &Document, page_id: ObjectId) -> Option<&Dictionary> {
    let mut node = doc.get_dictionary(page_id).ok()?;
    for _ in 0..32 {
        if let Some(Object::Dictionary(res)) = dict_entry(doc, node, b"Resources") {
            return Some(res);
        }
        node = match dict_entry(doc, node, b"Parent")? {
            Object::Dictionary(d) => d,
            _ => return None,
        };
    }
    None
}

/// Object ids of the image XObjects in `res`, descending into forms.
fn collect_images(doc: &Document, res: &Dictionary, depth: u32, ids: &mut Vec<ObjectId>) {
    let Some(Object::Dictionary(xobjects)) = dict_entry(doc, res, b"XObject") else {
        return;
    };
    for (_, value) in xobjects.iter() {
        let Ok(id) = value.as_reference() else { continue };
        let Ok(Object::Stream(s)) = doc.get_object(id) else { continue };
        match s.dict.get(b"Subtype").and_then(Object::as_name) {
            Ok(b"Image") => {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            Ok(b"Form") if depth < MAX_FORM_DEPTH => {
                if let Some(Object::Dictionary(form_res)) = dict_entry(doc, &s.dict, b"Resources") {
                    collect_images(doc, form_res, depth + 1, ids);
                }
            }
            _ => {}
        }
    }
}

fn int(doc: &Document, dict: &Dictionary, key: &[u8]) -> Option<i64> {
    dict_entry(doc, dict, key)?.as_i64().ok()
}

fn filters(doc: &Document, dict: &Dictionary) -> Option<Vec<Vec<u8>>> {
    match dict_entry(doc, dict, b"Filter") {
        None => Some(Vec::new()),
        Some(Object::Name(n)) => Some(vec![n.clone()]),
        Some(Object::Array(a)) => a
            .iter()
            .map(|o| deref(doc, o)?.as_name().ok().map(<[u8]>::to_vec))
            .collect(),
        Some(_) => None,
    }
}

/// The decode parameters for the filter at `index` of the chain.
fn decode_parms<'a>(doc: &'a Document, dict: &'a Dictionary, index: usize) -> Option<&'a Dictionary> {
    match dict_entry(doc, dict, b"DecodeParms")? {
        Object::Dictionary(d) => (index == 0).then_some(d),
        Object::Array(a) => match deref(doc, a.get(index)?)? {
            Object::Dictionary(d) => Some(d),
            _ => None,
        },
        _ => None,
    }
}

/// Number of colour components of a supported colour space.
fn components(doc: &Document, cs: Option<&Object>) -> Option<u8> {
    match cs? {
        Object::Name(n) => match n.as_slice() {
            b"DeviceGray" | b"CalGray" | b"G" => Some(1),
            b"DeviceRGB" | b"CalRGB" | b"RGB" => Some(3),
            _ => None,
        },
        Object::Array(a) => match deref(doc, a.first()?)?.as_name().ok()? {
            b"ICCBased" => {
                let Object::Stream(icc) = deref(doc, a.get(1)?)? else { return None };
                match int(doc, &icc.dict, b"N")? {
                    1 => Some(1),
                    3 => Some(3),
                    _ => None,
                }
            }
            b"CalGray" => Some(1),
            b"CalRGB" => Some(3),
            _ => None,
        },
        _ => None,
    }
}

/// Turns one image XObject into an encoded image file, or None if its
/// encoding is not supported.
fn encode_image(doc: &Document, s: &Stream) -> Option<Vec<u8>> {
    let d = &s.dict;
    let width = int(doc, d, b"Width")?;
    let height = int(doc, d, b"Height")?;
    if width < MIN_SIDE || height < MIN_SIDE || width > 1 << 16 || height > 1 << 16 {
        return None;
    }
    let chain = filters(doc, d)?;
    // Only Flate may come before the image codec.
    let (codec, pre) = match chain.split_last() {
        Some((last, pre)) if last.as_slice() == b"DCTDecode" || last.as_slice() == b"DCT" => {
            (Some(()), pre)
        }
        _ => (None, chain.as_slice()),
    };
    if pre.iter().any(|f| f.as_slice() != b"FlateDecode" && f.as_slice() != b"Fl") {
        return None;
    }

    if codec.is_some() {
        // JPEG: the decoder handles colour spaces and sizes itself.
        let mut data = s.content.clone();
        for (i, _) in pre.iter().enumerate() {
            data = inflate(&data, MAX_RAW_BYTES, decode_parms(doc, d, i))?;
        }
        return data.starts_with(&[0xFF, 0xD8]).then_some(data);
    }

    let mask = matches!(dict_entry(doc, d, b"ImageMask"), Some(Object::Boolean(true)));
    let (comps, bpc) = if mask {
        (1, 1)
    } else {
        (components(doc, dict_entry(doc, d, b"ColorSpace"))?, int(doc, d, b"BitsPerComponent")?)
    };
    if !matches!((comps, bpc), (1, 1) | (1, 8) | (3, 8)) {
        return None;
    }
    let (w, h) = (width as usize, height as usize);
    let row_bytes = (w * comps as usize * bpc as usize).div_ceil(8);
    let raw_len = row_bytes.checked_mul(h)?;
    if raw_len > MAX_RAW_BYTES {
        return None;
    }
    let mut data = s.content.clone();
    for (i, _) in pre.iter().enumerate() {
        data = inflate(&data, raw_len, decode_parms(doc, d, i))?;
    }
    if data.len() < raw_len {
        return None;
    }
    // /Decode [1 0] inverts the samples; image masks paint where 0 by default.
    let inverted = match dict_entry(doc, d, b"Decode") {
        Some(Object::Array(a)) => {
            a.first().and_then(|o| deref(doc, o)).and_then(|o| o.as_float().ok()).unwrap_or(0.0) > 0.5
        }
        _ => false,
    };
    let gray = |v: u8| if inverted { 255 - v } else { v };
    Some(match (comps, bpc) {
        (1, 1) => {
            let px: Vec<u8> = (0..h)
                .flat_map(|y| {
                    let row = &data[y * row_bytes..(y + 1) * row_bytes];
                    (0..w).map(move |x| if row[x / 8] & (0x80 >> (x % 8)) != 0 { 255 } else { 0 })
                })
                .map(gray)
                .collect();
            bmp_gray(w, h, &px)
        }
        (1, _) => bmp_gray(w, h, &data[..raw_len].iter().map(|&v| gray(v)).collect::<Vec<_>>()),
        _ => bmp_rgb(w, h, &data[..raw_len]),
    })
}

/// Zlib-inflates `data` to at most `cap` bytes (after undoing a PNG
/// predictor). TIFF predictors are not supported.
fn inflate(data: &[u8], cap: usize, parms: Option<&Dictionary>) -> Option<Vec<u8>> {
    let predictor = parms.and_then(|p| p.get(b"Predictor").ok()?.as_i64().ok()).unwrap_or(1);
    let param = |k: &[u8], def: i64| parms.and_then(|p| p.get(k).ok()?.as_i64().ok()).unwrap_or(def);
    let (colors, bpc, columns) = (param(b"Colors", 1), param(b"BitsPerComponent", 8), param(b"Columns", 1));
    if !(1..=4).contains(&colors) || !matches!(bpc, 1 | 2 | 4 | 8 | 16) || !(1..=1 << 16).contains(&columns) {
        return None;
    }
    let row = (colors * bpc * columns) as usize;
    let row = row.div_ceil(8);
    // With a PNG predictor every row carries one extra filter byte.
    let limit = match predictor {
        1 => cap,
        10..=15 => cap + cap / row + 1,
        _ => return None,
    };
    let mut out = Vec::new();
    let mut z = flate2::read::ZlibDecoder::new(data).take(limit as u64);
    if z.read_to_end(&mut out).is_err() && out.is_empty() {
        return None;
    }
    if predictor >= 10 {
        out = png_unpredict(&out, row, ((colors * bpc) as usize).div_ceil(8))?;
    }
    Some(out)
}

/// Undoes PNG row filters (each row: filter byte + `row` bytes).
fn png_unpredict(data: &[u8], row: usize, bpp: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len());
    let mut prev = vec![0u8; row];
    for chunk in data.chunks(row + 1) {
        if chunk.len() < row + 1 {
            break;
        }
        let (kind, src) = (chunk[0], &chunk[1..]);
        let mut cur = vec![0u8; row];
        for i in 0..row {
            let a = if i >= bpp { cur[i - bpp] } else { 0 };
            let b = prev[i];
            let c = if i >= bpp { prev[i - bpp] } else { 0 };
            let pred = match kind {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((a as u16 + b as u16) / 2) as u8,
                4 => {
                    let p = a as i16 + b as i16 - c as i16;
                    let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
                    if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
                _ => return None,
            };
            cur[i] = src[i].wrapping_add(pred);
        }
        out.extend_from_slice(&cur);
        prev = cur;
    }
    Some(out)
}

fn bmp_header(w: usize, h: usize, bits: u16, palette: usize, stride: usize) -> Vec<u8> {
    let offset = 14 + 40 + palette * 4;
    let size = offset + stride * h;
    let mut b = Vec::with_capacity(size);
    b.extend_from_slice(b"BM");
    b.extend_from_slice(&(size as u32).to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&(offset as u32).to_le_bytes());
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&(w as i32).to_le_bytes());
    // Negative height: rows top-down, as in the PDF.
    b.extend_from_slice(&(-(h as i32)).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&bits.to_le_bytes());
    b.extend_from_slice(&[0; 4]); // BI_RGB
    b.extend_from_slice(&((stride * h) as u32).to_le_bytes());
    b.extend_from_slice(&2835i32.to_le_bytes());
    b.extend_from_slice(&2835i32.to_le_bytes());
    b.extend_from_slice(&(palette as u32).to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b
}

/// An 8-bit grayscale BMP file from `w`×`h` samples.
fn bmp_gray(w: usize, h: usize, px: &[u8]) -> Vec<u8> {
    let stride = w.div_ceil(4) * 4;
    let mut b = bmp_header(w, h, 8, 256, stride);
    for v in 0..=255u8 {
        b.extend_from_slice(&[v, v, v, 0]);
    }
    for row in px.chunks(w).take(h) {
        b.extend_from_slice(row);
        b.resize(b.len() + stride - w, 0);
    }
    b
}

/// A 24-bit BMP file from `w`×`h` RGB samples.
fn bmp_rgb(w: usize, h: usize, px: &[u8]) -> Vec<u8> {
    let stride = (w * 3).div_ceil(4) * 4;
    let mut b = bmp_header(w, h, 24, 0, stride);
    for row in px.chunks(w * 3).take(h) {
        for p in row.chunks(3) {
            b.extend_from_slice(&[p[2], p[1], p[0]]);
        }
        b.resize(b.len() + stride - w * 3, 0);
    }
    b
}

/// OCR text of a scanned PDF's page images when `text` (what the text layer
/// gave) is too sparse; otherwise, or when OCR finds nothing, `text` as is.
#[cfg(windows)]
pub fn ocr_if_scanned(bytes: &[u8], text: String) -> String {
    if !crate::ocr::available() {
        return text;
    }
    let Ok(doc) = Document::load_mem(bytes) else { return text };
    if !looks_scanned(&text, page_count(&doc)) {
        return text;
    }
    let mut out = text;
    for_each_page_image(&doc, MAX_PAGES, |img| {
        if let Some(t) = crate::ocr::ocr_image_bytes(&img) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&t);
        }
        true
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{Object, Stream, dictionary};
    use std::io::Write;

    fn zlib(data: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    /// A PDF whose pages each show the given image XObjects; the last page
    /// wraps its images in a form XObject.
    fn pdf_with(pages: Vec<Vec<Stream>>) -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let mut kids = Vec::new();
        let n = pages.len();
        for (pi, images) in pages.into_iter().enumerate() {
            let mut xobj = lopdf::Dictionary::new();
            for (i, img) in images.into_iter().enumerate() {
                let id = doc.add_object(img);
                xobj.set(format!("Im{i}"), Object::Reference(id));
            }
            let xobj = if pi + 1 == n && n > 1 {
                let form = Stream::new(
                    dictionary! { "Type" => "XObject", "Subtype" => "Form",
                    "BBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
                    "Resources" => dictionary! { "XObject" => xobj } },
                    b"/Im0 Do".to_vec(),
                );
                let fid = doc.add_object(form);
                dictionary! { "Fm0" => fid }
            } else {
                xobj
            };
            let content = doc.add_object(Stream::new(dictionary! {}, b"q 100 0 0 100 0 0 cm /Im0 Do Q".to_vec()));
            let page = doc.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id, "Contents" => content,
                "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
                "Resources" => dictionary! { "XObject" => xobj },
            });
            kids.push(Object::Reference(page));
        }
        let count = kids.len() as i64;
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    fn image(w: i64, h: i64, extra: lopdf::Dictionary, data: Vec<u8>) -> Stream {
        let mut d = dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => w, "Height" => h };
        for (k, v) in extra.into_iter() {
            d.set(k, v);
        }
        Stream::new(d, data).with_compression(false)
    }

    fn fake_jpeg() -> Vec<u8> {
        let mut j = vec![0xFF, 0xD8, 0xFF, 0xE0];
        j.extend_from_slice(b"\0\x10JFIF\0rest-of-jpeg");
        j.extend_from_slice(&[0xFF, 0xD9]);
        j
    }

    /// Pixel (x, y) of an 8-bit gray or 24-bit BMP made here, as RGB.
    fn bmp_pixel(b: &[u8], x: usize, y: usize) -> [u8; 3] {
        assert_eq!(&b[..2], b"BM");
        let off = u32::from_le_bytes(b[10..14].try_into().unwrap()) as usize;
        let w = i32::from_le_bytes(b[18..22].try_into().unwrap()) as usize;
        let h = i32::from_le_bytes(b[22..26].try_into().unwrap());
        assert!(h < 0, "top-down");
        let bits = u16::from_le_bytes(b[28..30].try_into().unwrap());
        assert_eq!(b.len(), u32::from_le_bytes(b[2..6].try_into().unwrap()) as usize);
        if bits == 8 {
            let stride = w.div_ceil(4) * 4;
            let v = b[off + y * stride + x];
            [v, v, v]
        } else {
            let stride = (w * 3).div_ceil(4) * 4;
            let p = off + y * stride + x * 3;
            [b[p + 2], b[p + 1], b[p]]
        }
    }

    #[test]
    fn scanned_heuristic() {
        assert!(looks_scanned("", 1));
        assert!(looks_scanned(" \n\x0c 12 ", 3));
        assert!(!looks_scanned(&"نص ".repeat(30), 1));
        assert!(looks_scanned(&"a".repeat(30), 2));
        assert!(!looks_scanned("", 0));
    }

    #[test]
    fn jpeg_passes_through() {
        let jpeg = fake_jpeg();
        let pdf = pdf_with(vec![vec![image(
            640,
            480,
            dictionary! { "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "Filter" => "DCTDecode" },
            jpeg.clone(),
        )]]);
        let doc = Document::load_mem(&pdf).unwrap();
        assert_eq!(page_count(&doc), 1);
        assert_eq!(page_images(&doc, MAX_PAGES), vec![jpeg]);
    }

    #[test]
    fn flate_then_jpeg() {
        let jpeg = fake_jpeg();
        let filters: Vec<Object> = vec!["FlateDecode".into(), "DCTDecode".into()];
        let pdf = pdf_with(vec![vec![image(
            100,
            100,
            dictionary! { "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8, "Filter" => filters },
            zlib(&jpeg),
        )]]);
        let doc = Document::load_mem(&pdf).unwrap();
        assert_eq!(page_images(&doc, MAX_PAGES), vec![jpeg]);
    }

    #[test]
    fn raw_gray_rgb_and_bilevel_become_bmp() {
        let (w, h) = (37usize, 33usize); // odd sizes exercise row padding
        let gray: Vec<u8> = (0..w * h).map(|i| (i % 251) as u8).collect();
        let rgb: Vec<u8> = (0..w * h).flat_map(|i| [i as u8, 7, 200]).collect();
        let bilevel_row = (w).div_ceil(8);
        let mut bilevel = vec![0u8; bilevel_row * h];
        bilevel[0] = 0x80; // (0,0) white, (1,0) black
        let pdf = pdf_with(vec![
            vec![image(
                w as i64,
                h as i64,
                dictionary! { "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8, "Filter" => "FlateDecode" },
                zlib(&gray),
            )],
            vec![image(w as i64, h as i64, dictionary! { "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8 }, rgb)],
            vec![image(
                w as i64,
                h as i64,
                dictionary! { "ColorSpace" => "DeviceGray", "BitsPerComponent" => 1, "Filter" => "FlateDecode" },
                zlib(&bilevel),
            )],
        ]);
        let doc = Document::load_mem(&pdf).unwrap();
        let imgs = page_images(&doc, MAX_PAGES);
        assert_eq!(imgs.len(), 3);
        assert_eq!(bmp_pixel(&imgs[0], 5, 2), [((2 * w + 5) % 251) as u8; 3]);
        assert_eq!(bmp_pixel(&imgs[0], w - 1, h - 1), [((w * h - 1) % 251) as u8; 3]);
        let i = 3 * w + 4;
        assert_eq!(bmp_pixel(&imgs[1], 4, 3), [i as u8, 7, 200]);
        // Third page's image sits inside a form XObject.
        assert_eq!(bmp_pixel(&imgs[2], 0, 0), [255; 3]);
        assert_eq!(bmp_pixel(&imgs[2], 1, 0), [0; 3]);
    }

    #[test]
    fn png_predictor_and_decode_inversion() {
        let (w, h) = (40usize, 40usize);
        // Filter type 2 (Up): first row raw, every next row repeats it.
        let mut filtered = Vec::new();
        for y in 0..h {
            filtered.push(if y == 0 { 0 } else { 2 });
            filtered.extend((0..w).map(|x| if y == 0 { x as u8 } else { 0 }));
        }
        let pdf = pdf_with(vec![vec![image(
            w as i64,
            h as i64,
            dictionary! { "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8, "Filter" => "FlateDecode",
            "DecodeParms" => dictionary! { "Predictor" => 15, "Columns" => w as i64 },
            "Decode" => vec![1.into(), 0.into()] },
            zlib(&filtered),
        )]]);
        let doc = Document::load_mem(&pdf).unwrap();
        let imgs = page_images(&doc, MAX_PAGES);
        assert_eq!(bmp_pixel(&imgs[0], 9, 30), [255 - 9; 3]);
    }

    #[test]
    fn unsupported_and_tiny_images_are_skipped() {
        let pdf = pdf_with(vec![vec![
            // JPEG 2000, CCITT, CMYK raw, 16-bit, tiny icon, truncated data.
            image(100, 100, dictionary! { "Filter" => "JPXDecode" }, vec![0; 10]),
            image(100, 100, dictionary! { "Filter" => "CCITTFaxDecode", "ColorSpace" => "DeviceGray", "BitsPerComponent" => 1 }, vec![0; 10]),
            image(40, 40, dictionary! { "ColorSpace" => "DeviceCMYK", "BitsPerComponent" => 8 }, vec![0; 6400]),
            image(40, 40, dictionary! { "ColorSpace" => "DeviceGray", "BitsPerComponent" => 16 }, vec![0; 3200]),
            image(16, 16, dictionary! { "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8 }, vec![0; 256]),
            image(40, 40, dictionary! { "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8 }, vec![0; 100]),
            image(40, 40, dictionary! { "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8, "Filter" => "DCTDecode" }, b"not a jpeg".to_vec()),
        ]]);
        let doc = Document::load_mem(&pdf).unwrap();
        assert!(page_images(&doc, MAX_PAGES).is_empty());
    }

    #[test]
    fn shared_images_once_and_page_limit() {
        let mut doc = Document::with_version("1.5");
        let img = doc.add_object(image(
            50,
            50,
            dictionary! { "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8 },
            vec![128; 2500],
        ));
        let pages_id = doc.new_object_id();
        let kids: Vec<Object> = (0..3)
            .map(|_| {
                Object::Reference(doc.add_object(dictionary! {
                    "Type" => "Page", "Parent" => pages_id,
                    "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
                }))
            })
            .collect();
        // Resources inherited from the page tree.
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => 3,
                "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => img } } }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        let mut pdf = Vec::new();
        doc.save_to(&mut pdf).unwrap();
        let doc = Document::load_mem(&pdf).unwrap();
        assert_eq!(page_count(&doc), 3);
        assert_eq!(page_images(&doc, MAX_PAGES).len(), 1);
        assert!(page_images(&doc, 0).is_empty());
    }

    #[test]
    fn sparse_text_pdf_round_trip() {
        // pdf-extract on an image-only PDF yields (almost) no text, which is
        // what triggers OCR.
        let pdf = pdf_with(vec![vec![image(
            64,
            64,
            dictionary! { "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8 },
            vec![200; 64 * 64],
        )]]);
        let text = pdf_extract::extract_text_from_mem(&pdf).unwrap_or_default();
        let doc = Document::load_mem(&pdf).unwrap();
        assert!(looks_scanned(&text, page_count(&doc)));
        assert_eq!(page_images(&doc, MAX_PAGES).len(), 1);
    }
}
