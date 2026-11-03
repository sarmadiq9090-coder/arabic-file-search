//! The indexing progress on the first day: «جارِ الفهرسة… 36,000 من 432,000
//! ملف» in a part of its own at the end of the status bar and under the quick
//! popup's results, read from the counters the running drive scan and
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

    /// Whether the status bar has the progress part now.
    pub(super) fn indexing_part_shown(&self) -> bool {
        self.indexing_shown.borrow().is_some()
    }

    /// Called by the timer: about once a second, brings the status bar's
    /// progress part and the quick popup's progress line up to date.
    pub(super) fn poll_indexing(&self, tick: u32) {
        if !tick.is_multiple_of(5) {
            return;
        }
        let text = indexing_status::progress_text(crate::i18n::current(), self.indexing(), &fmt_num);
        if *self.indexing_shown.borrow() == text {
            return;
        }
        let appeared_or_went = self.indexing_shown.replace(text.clone()).is_some() != text.is_some();
        if appeared_or_went {
            // Two status bar parts or three.
            self.layout();
        }
        if let Some(t) = &text {
            self.status.set_text(STATUS_PART, t);
        }
        self.popup_indexing_changed();
    }
}
