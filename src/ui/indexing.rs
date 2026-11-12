//! The indexing progress on the first day («أسماء الملفات جاهزة · تُقرأ
//! محتويات المستندات: 36,000 من 432,000») in a part of its own at the end of
//! the status bar and under the quick popup's results, read from the counters the running drive scan and
//! content warm-up already keep (no thread of its own). The texts are in
//! `crate::indexing_status`.

use std::sync::atomic::Ordering;

use super::App;
use super::format::fmt_num;
use crate::indexing_status::{self, Indexing};

/// The status bar part that shows the progress, while there is any.
pub(super) const STATUS_PART: u8 = 2;

impl App {
    /// What is being indexed now: the drive scan first, then the documents'
    /// texts going into the content index.
    pub(super) fn indexing(&self) -> Indexing {
        let st = self.state.borrow();
        if let Some(job) = &st.scan {
            return Indexing::Names { found: job.progress.files.load(Ordering::Relaxed) };
        }
        match &st.warm {
            Some(w) if !w.finished.load(Ordering::Relaxed) => Indexing::Contents {
                checked: w.progress.checked.load(Ordering::Relaxed),
                total: w.progress.total.load(Ordering::Relaxed),
            },
            _ => Indexing::Idle,
        }
    }

    /// "No results yet — results may be incomplete while indexing.", or
    /// `None` when nothing is being indexed.
    pub(super) fn no_results_while_indexing(&self) -> Option<&'static str> {
        indexing_status::no_results_text(crate::i18n::current(), self.indexing())
    }

    /// The status of a search inside documents that has found nothing yet
    /// while indexing runs, or `None` when nothing is being indexed.
    pub(super) fn content_search_while_indexing(&self, checked: usize, total: usize) -> Option<String> {
        indexing_status::content_search_text(crate::i18n::current(), self.indexing(), checked, total, &fmt_num)
    }

    /// What the empty result list says, or `None` for nothing: a search
    /// inside documents still reading with nothing found yet, or the very
    /// first index being built (no index to search yet).
    pub(super) fn empty_list_text(&self) -> Option<String> {
        let lang = crate::i18n::current();
        let st = self.state.borrow();
        if !st.rows.is_empty() {
            return None;
        }
        if let Some(job) = &st.content {
            let p = &job.progress;
            return Some(indexing_status::reading_text(lang, p.checked.load(Ordering::Relaxed), p.total.load(Ordering::Relaxed), &fmt_num));
        }
        (st.scan.is_some() && st.index.entries.is_empty()).then(|| indexing_status::first_scan_text(lang).to_string())
    }

    /// Whether the status bar has the progress part now, and whether it is
    /// the wider one (`indexing_status::status_parts`).
    pub(super) fn indexing_part(&self) -> Option<bool> {
        self.indexing_shown.borrow().is_some().then(|| self.indexing_wide.get())
    }

    /// Called by the timer: about once a second, brings the status bar's
    /// progress part and the quick popup's progress line up to date.
    pub(super) fn poll_indexing(&self, tick: u32) {
        if !tick.is_multiple_of(5) {
            return;
        }
        let state = self.indexing();
        let text = indexing_status::progress_text(crate::i18n::current(), state, &fmt_num);
        if *self.indexing_shown.borrow() == text {
            return;
        }
        let appeared_or_went = self.indexing_shown.replace(text.clone()).is_some() != text.is_some();
        // The line about the documents' texts is longer than the scan's.
        let wide = matches!(state, Indexing::Contents { .. });
        let resized = self.indexing_wide.replace(wide) != wide;
        if appeared_or_went || resized {
            // Two status bar parts or three, and how wide the third is.
            self.layout();
            // The empty list's sentence about the first index comes or goes.
            self.list.invalidate();
        }
        if let Some(t) = &text {
            self.status.set_text(STATUS_PART, t);
        }
        self.popup_indexing_changed();
    }
}
