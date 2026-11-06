//! What the main window and the quick popup say while the index is still
//! being built, so a first-day search that finds nothing reads as "not
//! ready yet" instead of "broken". Platform-independent and unit-tested; the
//! UI reads the counters of the running scan or content warm-up
//! (`ui/indexing.rs`).

use crate::i18n::Lang;

/// What the background indexing is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Indexing {
    #[default]
    Idle,
    /// The drives are being scanned for names: `found` so far (the total is
    /// not known until the scan ends).
    Names { found: usize },
    /// Document texts are going into the content index: `checked` of the
    /// `total` documents that were not in it yet.
    Contents { checked: usize, total: usize },
}

impl Indexing {
    /// Whether a search may miss files because of it.
    pub fn running(self) -> bool {
        match self {
            Indexing::Idle => false,
            Indexing::Names { .. } => true,
            Indexing::Contents { checked, total } => checked < total,
        }
    }

    /// Whether a search by name may miss files: the drive scan is not done.
    pub fn names_incomplete(self) -> bool {
        matches!(self, Indexing::Names { .. })
    }
}

/// `n` with a comma between thousands: 432000 → "432,000".
pub fn group_digits(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "جارِ الفهرسة… 36,000 من 432,000 ملف" / "Indexing… 36,000 of 432,000
/// files", or `None` when nothing is being indexed. `num` writes a count
/// (thousands separators, and the digits the user chose).
pub fn progress_text(lang: Lang, state: Indexing, num: &dyn Fn(usize) -> String) -> Option<String> {
    if !state.running() {
        return None;
    }
    Some(match state {
        Indexing::Names { found } => match lang {
            Lang::Ar => format!("جارِ الفهرسة… {} ملف حتى الآن", num(found)),
            Lang::En => format!("Indexing… {} files so far", num(found)),
        },
        Indexing::Contents { checked, total } => {
            let (checked, total) = (num(checked.min(total)), num(total));
            match lang {
                Lang::Ar => format!("جارِ الفهرسة… {checked} من {total} ملف"),
                Lang::En => format!("Indexing… {checked} of {total} files"),
            }
        }
        Indexing::Idle => return None,
    })
}

/// Said instead of "no results" while indexing runs.
pub fn incomplete_text(lang: Lang) -> &'static str {
    lang.pick("قد تكون النتائج ناقصة أثناء الفهرسة", "Results may be incomplete while indexing")
}

/// What a search that found nothing says while indexing runs, instead of
/// "no results" (the progress itself is shown beside it); `None` when
/// nothing is being indexed, so the usual "no results" applies.
pub fn no_results_text(lang: Lang, state: Indexing) -> Option<&'static str> {
    state.running().then(|| {
        lang.pick(
            "لا نتائج حتى الآن — قد تكون النتائج ناقصة أثناء الفهرسة.",
            "No results yet — results may be incomplete while indexing.",
        )
    })
}

/// A search inside documents still running with nothing found yet, while
/// indexing runs: the "may be incomplete" note first (the status bar cuts a
/// long text at its end), then how far the search got.
pub fn content_search_text(lang: Lang, state: Indexing, checked: usize, total: usize, num: &dyn Fn(usize) -> String) -> Option<String> {
    let lead = no_results_text(lang, state)?;
    let (checked, total) = (num(checked.min(total)), num(total));
    Some(match lang {
        Lang::Ar => format!("{lead} تم فحص {checked} من {total} مستند…"),
        Lang::En => format!("{lead} Checked {checked} of {total} documents…"),
    })
}

/// The heading of the quick popup's "No results — try" section while
/// indexing runs.
pub fn incomplete_try(lang: Lang) -> &'static str {
    lang.pick("قد تكون النتائج ناقصة أثناء الفهرسة — جرّب", "Results may be incomplete while indexing — try")
}

/// The quick popup's line under its results while indexing runs: the
/// progress, led by the "may be incomplete" note when no file name matched
/// while the names are still being scanned (the popup searches names only,
/// so the content index filling up doesn't make its results incomplete).
pub fn popup_note(lang: Lang, state: Indexing, no_files: bool, num: &dyn Fn(usize) -> String) -> Option<String> {
    let progress = progress_text(lang, state, num)?;
    Some(if no_files && state.names_incomplete() { format!("{} — {progress}", incomplete_text(lang)) } else { progress })
}

/// True when the OCR languages Windows offers (BCP-47 tags such as
/// "en-US") leave out Arabic: images are then searched without their
/// Arabic text, which the user is told about ([`ocr_arabic_missing_text`])
/// rather than finding nothing silently.
pub fn ocr_lacks_arabic(tags: &[String]) -> bool {
    !tags.iter().any(|t| t.split('-').next().is_some_and(|p| p.eq_ignore_ascii_case("ar")))
}

/// The notice shown after a search inside documents when Windows has no
/// Arabic OCR language ([`ocr_lacks_arabic`]).
pub fn ocr_arabic_missing_text(lang: Lang) -> &'static str {
    lang.pick(
        "تنبيه: لا يمكن قراءة النص العربي داخل الصور لأن حزمة اللغة العربية غير مثبّتة في ويندوز.",
        "Note: Arabic text in images can't be read: the Windows Arabic language pack is not installed.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(n: usize) -> String {
        group_digits(n)
    }

    #[test]
    fn thousands_are_grouped() {
        let cases = [(0, "0"), (7, "7"), (999, "999"), (1000, "1,000"), (36_000, "36,000"), (432_000, "432,000"), (1_234_567, "1,234,567")];
        for (n, want) in cases {
            assert_eq!(group_digits(n), want, "{n}");
        }
    }

    #[test]
    fn content_progress_reads_as_in_the_user_study() {
        let s = Indexing::Contents { checked: 36_000, total: 432_000 };
        assert_eq!(progress_text(Lang::Ar, s, &num).as_deref(), Some("جارِ الفهرسة… 36,000 من 432,000 ملف"));
        assert_eq!(progress_text(Lang::En, s, &num).as_deref(), Some("Indexing… 36,000 of 432,000 files"));
    }

    #[test]
    fn name_scan_has_no_total_yet() {
        let s = Indexing::Names { found: 120_500 };
        assert_eq!(progress_text(Lang::Ar, s, &num).as_deref(), Some("جارِ الفهرسة… 120,500 ملف حتى الآن"));
        assert_eq!(progress_text(Lang::En, s, &num).as_deref(), Some("Indexing… 120,500 files so far"));
        // A scan that has found nothing yet is still running.
        assert!(Indexing::Names { found: 0 }.running());
    }

    #[test]
    fn nothing_is_said_when_idle_or_finished() {
        for s in [Indexing::Idle, Indexing::Contents { checked: 5, total: 5 }, Indexing::Contents { checked: 0, total: 0 }] {
            assert!(!s.running(), "{s:?}");
            assert_eq!(progress_text(Lang::En, s, &num), None);
            assert_eq!(no_results_text(Lang::Ar, s), None);
            assert_eq!(content_search_text(Lang::En, s, 1, 2, &num), None);
            assert_eq!(popup_note(Lang::Ar, s, true, &num), None);
        }
    }

    #[test]
    fn checked_never_shows_above_total() {
        let s = Indexing::Contents { checked: 12, total: 10 };
        assert!(!s.running());
        assert_eq!(progress_text(Lang::En, Indexing::Contents { checked: 9, total: 10 }, &num).as_deref(), Some("Indexing… 9 of 10 files"));
    }

    #[test]
    fn empty_result_while_indexing_says_incomplete() {
        let s = Indexing::Contents { checked: 36_000, total: 432_000 };
        assert_eq!(no_results_text(Lang::Ar, s), Some("لا نتائج حتى الآن — قد تكون النتائج ناقصة أثناء الفهرسة."));
        assert_eq!(no_results_text(Lang::En, Indexing::Names { found: 0 }), Some("No results yet — results may be incomplete while indexing."));
        // The search inside documents that found nothing in its first minute.
        assert_eq!(
            content_search_text(Lang::En, s, 36_000, 432_000, &num).as_deref(),
            Some("No results yet — results may be incomplete while indexing. Checked 36,000 of 432,000 documents…")
        );
        assert_eq!(
            content_search_text(Lang::Ar, s, 36_000, 432_000, &num).as_deref(),
            Some("لا نتائج حتى الآن — قد تكون النتائج ناقصة أثناء الفهرسة. تم فحص 36,000 من 432,000 مستند…")
        );
    }

    #[test]
    fn popup_note_leads_with_incomplete_only_without_files() {
        let s = Indexing::Names { found: 1_000 };
        assert_eq!(popup_note(Lang::En, s, false, &num).as_deref(), Some("Indexing… 1,000 files so far"));
        assert_eq!(
            popup_note(Lang::En, s, true, &num).as_deref(),
            Some("Results may be incomplete while indexing — Indexing… 1,000 files so far")
        );
        assert!(popup_note(Lang::Ar, s, true, &num).unwrap().starts_with("قد تكون النتائج ناقصة أثناء الفهرسة — جارِ الفهرسة…"));
        // The names are all in while the content index fills: only the progress.
        let c = Indexing::Contents { checked: 10, total: 20 };
        assert!(!c.names_incomplete() && s.names_incomplete());
        assert_eq!(popup_note(Lang::En, c, true, &num).as_deref(), Some("Indexing… 10 of 20 files"));
    }

    #[test]
    fn the_counts_use_the_given_digits() {
        // The window can show Arabic-Indic digits (Options); the formatter is passed in.
        let indic = |n: usize| group_digits(n).chars().map(|c| if c.is_ascii_digit() { char::from_u32(c as u32 - 48 + 0x0660).unwrap() } else { c }).collect::<String>();
        let s = Indexing::Contents { checked: 36_000, total: 432_000 };
        assert_eq!(progress_text(Lang::Ar, s, &indic).as_deref(), Some("جارِ الفهرسة… ٣٦,٠٠٠ من ٤٣٢,٠٠٠ ملف"));
    }

    #[test]
    fn arabic_texts_start_with_an_arabic_letter() {
        // The status bar and the popup take the paragraph direction from the
        // first strong letter: the Arabic texts must not start with a digit
        // or a Latin letter, or they would lay out left to right.
        let s = Indexing::Contents { checked: 1, total: 2 };
        let texts = [
            progress_text(Lang::Ar, s, &num).unwrap(),
            no_results_text(Lang::Ar, s).unwrap().to_string(),
            content_search_text(Lang::Ar, s, 1, 2, &num).unwrap(),
            popup_note(Lang::Ar, s, true, &num).unwrap(),
            incomplete_try(Lang::Ar).to_string(),
            ocr_arabic_missing_text(Lang::Ar).to_string(),
        ];
        for t in texts {
            let first = t.chars().next().unwrap();
            assert!(('\u{0600}'..='\u{06FF}').contains(&first), "{t}");
        }
    }

    #[test]
    fn arabic_ocr_language_detection() {
        let tags = |t: &[&str]| t.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(ocr_lacks_arabic(&tags(&[])));
        assert!(ocr_lacks_arabic(&tags(&["en-US"])));
        assert!(ocr_lacks_arabic(&tags(&["en-US", "fr-FR", "arn"])));
        assert!(!ocr_lacks_arabic(&tags(&["ar-SA", "en-US"])));
        assert!(!ocr_lacks_arabic(&tags(&["en-GB", "AR"])));
        assert!(ocr_arabic_missing_text(Lang::En).contains("Arabic text in images can't be read"));
        assert!(ocr_arabic_missing_text(Lang::Ar).contains("لا يمكن قراءة النص العربي داخل الصور"));
    }
}
