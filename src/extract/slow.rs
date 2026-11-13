//! Inputs that used to take hours (quadratic or exponential work) and were
//! never stored, so the warm-up met them again at every start. Each runs on
//! a thread with a deadline: a regression fails the test instead of hanging
//! the test run (the stuck thread is left behind).

use std::fmt::Write as _;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::*;

/// Runs `f` on its own thread; panics when it takes longer than `limit`.
fn within<R: Send + 'static>(what: &str, limit: Duration, f: impl FnOnce() -> R + Send + 'static) -> R {
    let (tx, rx) = mpsc::channel();
    let started = Instant::now();
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || {
            let _ = tx.send(f());
        })
        .unwrap();
    match rx.recv_timeout(limit) {
        Ok(r) => {
            eprintln!("{what}: {} ms", started.elapsed().as_millis());
            r
        }
        Err(_) => panic!("{what}: still running after {limit:?}"),
    }
}

const LIMIT: Duration = Duration::from_secs(20);

#[test]
fn rtf_with_a_huge_font_table() {
    // Every \fN\fcharset178 went through a linear list of fonts (retain +
    // push, then a linear lookup per character): quadratic in the fonts.
    let mut rtf = String::from("{\\rtf1\\ansi{\\fonttbl");
    for i in 0..200_000 {
        let _ = write!(rtf, "{{\\f{i}\\fcharset178 Arial;}}");
    }
    rtf.push_str("}\\f199999 \\'e3\\'d1\\'cd\\'c8\\'c7}");
    let t = within("rtf fonts", LIMIT, move || rtf_to_text(rtf.as_bytes()));
    assert!(t.contains("مرحبا"), "{t:?}");
}

#[test]
fn eml_boundary_of_dashes_over_a_body_of_dashes() {
    // A 500,000-character boundary of dashes (ending in "x") over 8 MB of
    // dashes: each position compared up to 500,000 bytes.
    let boundary = format!("{}x", "-".repeat(500_000));
    let eml = format!(
        "Subject: s\r\nContent-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\r\n{}\r\n--{boundary}\r\nContent-Type: text/plain\r\n\r\nنص الرسالة\r\n--{boundary}--\r\n",
        "-".repeat(8 << 20)
    );
    let t = within("eml boundary", LIMIT, move || mail::eml_to_text(eml.as_bytes()));
    assert!(t.is_some_and(|t| t.contains("نص الرسالة")));
}

#[test]
fn eml_subject_of_unfinished_encoded_words() {
    // `=?a?b?x` 300,000 times: every one searched the rest of the header
    // for its `?=`.
    let subject = "=?a?b?x".repeat(300_000);
    let t = within("eml subject", LIMIT, move || mail::decode_header_value(subject.as_bytes()));
    assert!(t.starts_with("=?a?b?x"));
    // Ordinary encoded words still decode, split ones joined.
    assert_eq!(mail::decode_header_value(b"=?utf-8?B?2YXYsdit?= =?utf-8?B?2KjYpw==?= x"), "مرحبا x");
}

/// A one-page PDF whose resources (object 6) list `forms` forms, each of
/// which has those same resources, plus `images` 40x40 gray images.
fn pdf_shared_resources(forms: usize, images: usize) -> Vec<u8> {
    use super::hostile::{pdf_of, pdf_stream};
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources 6 0 R >>".to_owned(),
        pdf_stream("", "q Q"),
        "<< >>".to_owned(),
    ];
    let mut xobjects = String::new();
    for i in 0..forms {
        let _ = write!(xobjects, "/F{i} {} 0 R ", 7 + i);
    }
    for i in 0..images {
        let _ = write!(xobjects, "/I{i} {} 0 R ", 7 + forms + i);
    }
    objects.push(format!("<< /XObject << {xobjects}>> >>"));
    for _ in 0..forms {
        objects.push(pdf_stream("/Type /XObject /Subtype /Form /BBox [0 0 1 1] /Resources 6 0 R", "q Q"));
    }
    let pixels: String = (0..1600).map(|i| if i % 3 == 0 { 'a' } else { 'z' }).collect();
    for _ in 0..images {
        objects.push(pdf_stream("/Type /XObject /Subtype /Image /Width 40 /Height 40 /ColorSpace /DeviceGray /BitsPerComponent 8", &pixels));
    }
    pdf_of(&objects)
}

#[test]
fn pdf_forms_sharing_one_resources_dictionary() {
    // 60 forms whose resources are the page's own: every level re-entered
    // all 60, 60^5 visits looking for images.
    let pdf = pdf_shared_resources(60, 1);
    let n = within("pdf shared resources", LIMIT, move || {
        let doc = lopdf::Document::load_mem(&pdf).unwrap();
        pdf_images::page_images(&doc, pdf_images::MAX_PAGES).len()
    });
    assert_eq!(n, 1);
}

#[test]
fn pdf_page_images_sent_to_ocr_are_capped() {
    let pdf = pdf_shared_resources(0, 3000);
    let n = within("pdf many images", LIMIT, move || {
        let doc = lopdf::Document::load_mem(&pdf).unwrap();
        pdf_images::page_images(&doc, pdf_images::MAX_PAGES).len()
    });
    assert_eq!(n, pdf_images::MAX_IMAGES);
}

#[test]
fn a_file_over_its_time_budget_gives_no_text() {
    // Whatever is slow next: the extraction stops at its time budget, and
    // the file is stored as "no text" instead of being tried at every start.
    let mut rtf = String::from("{\\rtf1\\ansi ");
    for _ in 0..400_000 {
        rtf.push_str("{\\b \\'e3\\'d1\\'cd\\'c8\\'c7 }");
    }
    rtf.push('}');
    let p = std::env::temp_dir().join(format!("bahth_slow_{}.rtf", std::process::id()));
    let _tmp = crate::test_temp::TempPath::at(p.clone());
    std::fs::write(&p, &rtf).unwrap();
    assert!(extract_text(&p).is_some(), "fits the normal budget");
    let q = p.clone();
    let t = within("budget", LIMIT, move || {
        let _short = budget::with_limit(Duration::from_millis(1));
        extract_text(&q)
    });
    assert_eq!(t, None);
    let _ = std::fs::remove_file(&p);
}

/// A docx whose 1 KB-per-megabyte parts unpack to `headers` MB of headers.
fn docx_with_headers(headers: usize) -> Vec<u8> {
    let part = format!("<w:hdr xmlns:w=\"w\"><w:p>{}</w:p></w:hdr>", "<w:r><w:t>ترويسة</w:t></w:r>".repeat(1 << 15));
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    zw.start_file("word/document.xml", opts).unwrap();
    std::io::Write::write_all(&mut zw, "<w:document xmlns:w=\"w\"><w:body><w:p><w:r><w:t>المتن</w:t></w:r></w:p></w:body></w:document>".as_bytes()).unwrap();
    for i in 1..=headers {
        zw.start_file(format!("word/header{i}.xml"), opts).unwrap();
        std::io::Write::write_all(&mut zw, part.as_bytes()).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

#[test]
fn nested_documents_share_one_inflation_budget() {
    // 600 headers of 1 MB each (every part under the 1 MB floor of the
    // ratio check) unpack to 600 MB from a few megabytes; inside an archive
    // every member could do it again. About 90 s in a debug build before
    // (all 600 read), under 10 s now (64 headers).
    let docx = docx_with_headers(600);
    eprintln!("docx with headers: {} bytes", docx.len());
    assert!(docx.len() < 16 << 20, "{}", docx.len());
    let p = std::env::temp_dir().join(format!("bahth_slow_{}.docx", std::process::id()));
    let _tmp = crate::test_temp::TempPath::at(p.clone());
    std::fs::write(&p, &docx).unwrap();
    let q = p.clone();
    let t = within("docx headers", Duration::from_secs(40), move || extract_text(&q));
    // The body and some headers come out; most of the headers do not.
    let t = t.unwrap();
    assert!(t.contains("المتن") && t.contains("ترويسة"));
    assert!(t.len() < 100 << 20, "{}", t.len());
    let _ = std::fs::remove_file(&p);
}

#[test]
fn inflation_budget_spans_archive_members() {
    // Members (30 MB each, stored, no text in them) are charged to the same
    // budget as the parts of documents inside them.
    let member = vec![b' '; 30 << 20];
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for i in 0..20 {
        zw.start_file(format!("m{i}.txt"), opts).unwrap();
        std::io::Write::write_all(&mut zw, &member).unwrap();
    }
    let zip = zw.finish().unwrap().into_inner();
    let p = std::env::temp_dir().join(format!("bahth_slow_{}.zip", std::process::id()));
    let _tmp = crate::test_temp::TempPath::at(p.clone());
    std::fs::write(&p, &zip).unwrap();
    drop(zip);
    let _b = budget::start();
    let members = archive::archive_texts(&p);
    // MAX_ARCHIVE_TOTAL (300 MB) stops it first here; the budget is charged.
    assert!(members.is_empty());
    assert!(budget::room(u64::MAX) <= budget::INFLATE - (290 << 20));
    let _ = std::fs::remove_file(&p);
}
