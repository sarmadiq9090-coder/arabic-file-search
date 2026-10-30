//! Text recognition in images with the OCR engine built into Windows 10+
//! (`Windows.Media.Ocr`). Nothing is downloaded: the engine uses the OCR
//! languages Windows has installed, so Arabic needs the Arabic language pack.
//!
//! Every entry point is blocking and meant for background threads (content
//! search). WinRT is initialised (multithreaded apartment) on each calling
//! thread, and at most [`MAX_CONCURRENT`] recognitions run at once so OCR
//! does not starve the rest of the content search. Where the engine does not
//! exist (older Windows, Wine) [`available`] is false and everything returns
//! `None`.

use std::cell::RefCell;
use std::path::Path;
use std::sync::{Condvar, Mutex, OnceLock};

use windows::Globalization::Language;
use windows::Graphics::Imaging::{
    BitmapAlphaMode, BitmapDecoder, BitmapInterpolationMode, BitmapPixelFormat, BitmapTransform,
    ColorManagementMode, ExifOrientationMode, SoftwareBitmap,
};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows::core::HSTRING;

/// Image files larger than this are not recognised.
pub const MAX_IMAGE_FILE: u64 = 30 * 1024 * 1024;
/// At most this many recognitions run at the same time.
const MAX_CONCURRENT: usize = 2;
/// Frames of a multi-page image (TIFF) that are recognised.
const MAX_FRAMES: u32 = 50;
/// Images smaller than this (either side) hold no readable text.
const MIN_SIDE: u32 = 16;

/// Whether Windows OCR works here with at least one language.
pub fn available() -> bool {
    !languages().is_empty()
}

/// BCP-47 tags of the OCR languages used, Arabic first: Arabic and English
/// when installed, plus the user-profile language the engine picks.
pub fn languages() -> Vec<String> {
    static LANGS: OnceLock<Vec<String>> = OnceLock::new();
    LANGS
        .get_or_init(|| {
            // Probe on a fresh thread so the caller's COM apartment (the UI
            // thread is single-threaded) does not matter, and a failure in
            // the probe cannot take the caller down.
            std::thread::spawn(|| {
                init_thread();
                pick_languages().unwrap_or_default()
            })
            .join()
            .unwrap_or_default()
        })
        .clone()
}

/// Recognises the text in an encoded image (JPEG, PNG, BMP, GIF, TIFF…).
/// Lines from each OCR language are merged without duplicates.
pub fn ocr_image_bytes(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || !available() {
        return None;
    }
    init_thread();
    let _permit = Permit::acquire();
    let text = recognize(bytes).ok()?;
    (!text.trim().is_empty()).then_some(text)
}

/// [`ocr_image_bytes`] on the file at `path` (at most [`MAX_IMAGE_FILE`]).
pub fn ocr_image_file(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_IMAGE_FILE || !available() {
        return None;
    }
    ocr_image_bytes(&std::fs::read(path).ok()?)
}

// ---------------------------------------------------------------------------

fn init_thread() {
    thread_local!(static INIT: () = {
        // S_FALSE (already initialised) and RPC_E_CHANGED_MODE (an STA
        // thread) are both fine: the OCR objects are agile.
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
    });
    INIT.with(|_| {});
}

fn tag_of(lang: &Language) -> Option<String> {
    Some(lang.LanguageTag().ok()?.to_string_lossy())
}

fn pick_languages() -> windows::core::Result<Vec<String>> {
    let avail = OcrEngine::AvailableRecognizerLanguages()?;
    let tags: Vec<String> = (0..avail.Size()?)
        .filter_map(|i| avail.GetAt(i).ok())
        .filter_map(|l| tag_of(&l))
        .collect();
    let primary = |t: &str| t.split('-').next().unwrap_or("").to_ascii_lowercase();
    let mut out: Vec<String> = Vec::new();
    for want in ["ar", "en"] {
        if let Some(t) = tags.iter().find(|t| primary(t) == want) {
            out.push(t.clone());
        }
    }
    if let Ok(engine) = OcrEngine::TryCreateFromUserProfileLanguages()
        && let Ok(lang) = engine.RecognizerLanguage()
        && let Some(t) = tag_of(&lang)
        && !out.iter().any(|o| primary(o) == primary(&t))
    {
        out.push(t);
    }
    // Only keep languages an engine can really be created for.
    out.retain(|t| create_engine(t).is_some());
    Ok(out)
}

fn create_engine(tag: &str) -> Option<OcrEngine> {
    let lang = Language::CreateLanguage(&HSTRING::from(tag)).ok()?;
    OcrEngine::TryCreateFromLanguage(&lang).ok()
}

/// Per-thread engines, one per language of [`languages`].
fn with_engines<R>(f: impl FnOnce(&[OcrEngine]) -> R) -> R {
    thread_local!(static ENGINES: RefCell<Option<Vec<OcrEngine>>> = const { RefCell::new(None) });
    ENGINES.with(|cell| {
        let mut slot = cell.borrow_mut();
        let engines =
            slot.get_or_insert_with(|| languages().iter().filter_map(|t| create_engine(t)).collect());
        f(engines)
    })
}

fn recognize(bytes: &[u8]) -> windows::core::Result<String> {
    let stream = InMemoryRandomAccessStream::new()?;
    let writer = DataWriter::CreateDataWriter(&stream)?;
    writer.WriteBytes(bytes)?;
    writer.StoreAsync()?.join()?;
    writer.FlushAsync()?.join()?;
    writer.DetachStream()?;
    stream.Seek(0)?;

    let decoder = BitmapDecoder::CreateAsync(&stream)?.join()?;
    let max = OcrEngine::MaxImageDimension().unwrap_or(10_000).max(1);
    let mut lines: Vec<String> = Vec::new();
    for i in 0..decoder.FrameCount()?.min(MAX_FRAMES) {
        let frame = decoder.GetFrameAsync(i)?.join()?;
        let (w, h) = (frame.OrientedPixelWidth()?, frame.OrientedPixelHeight()?);
        if w < MIN_SIDE || h < MIN_SIDE {
            continue;
        }
        let transform = BitmapTransform::new()?;
        if w > max || h > max {
            let (sw, sh) = scaled_size(w, h, max);
            transform.SetScaledWidth(sw)?;
            transform.SetScaledHeight(sh)?;
            transform.SetInterpolationMode(BitmapInterpolationMode::Fant)?;
        }
        let bitmap = frame
            .GetSoftwareBitmapTransformedAsync(
                BitmapPixelFormat::Bgra8,
                BitmapAlphaMode::Premultiplied,
                &transform,
                ExifOrientationMode::RespectExifOrientation,
                ColorManagementMode::DoNotColorManage,
            )?
            .join()?;
        recognize_bitmap(&bitmap, &mut lines);
    }
    Ok(lines.join("\n"))
}

/// Runs every engine on `bitmap`, appending lines not seen yet.
fn recognize_bitmap(bitmap: &SoftwareBitmap, lines: &mut Vec<String>) {
    with_engines(|engines| {
        for engine in engines {
            let Ok(result) = engine.RecognizeAsync(bitmap).and_then(|op| op.join()) else {
                continue;
            };
            let Ok(found) = result.Lines() else { continue };
            for line in (0..found.Size().unwrap_or(0)).filter_map(|i| found.GetAt(i).ok()) {
                let Ok(text) = line.Text() else { continue };
                let text = text.to_string_lossy();
                let text = text.trim();
                if !text.is_empty() && !lines.iter().any(|l| l == text) {
                    lines.push(text.to_string());
                }
            }
        }
    });
}

/// Fits `w`×`h` into `max`×`max`, keeping the aspect ratio.
fn scaled_size(w: u32, h: u32, max: u32) -> (u32, u32) {
    let scale = max as f64 / w.max(h) as f64;
    let fit = |v: u32| ((v as f64 * scale).floor() as u32).clamp(1, max);
    (fit(w), fit(h))
}

/// A counting semaphore permit limiting concurrent recognitions.
struct Permit;

static SLOTS: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());

impl Permit {
    fn acquire() -> Permit {
        let (lock, cv) = &SLOTS;
        let mut busy = lock.lock().unwrap_or_else(|e| e.into_inner());
        while *busy >= MAX_CONCURRENT {
            busy = cv.wait(busy).unwrap_or_else(|e| e.into_inner());
        }
        *busy += 1;
        Permit
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        let (lock, cv) = &SLOTS;
        let mut busy = lock.lock().unwrap_or_else(|e| e.into_inner());
        *busy = busy.saturating_sub(1);
        cv.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_fits_max() {
        assert_eq!(scaled_size(20_000, 10_000, 10_000), (10_000, 5_000));
        assert_eq!(scaled_size(100, 30_000, 10_000), (33, 10_000));
        assert_eq!(scaled_size(1, 1_000_000, 10_000), (1, 10_000));
    }
}
