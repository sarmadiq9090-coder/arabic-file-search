//! Checking for, downloading and installing a new version.

use native_windows_gui as nwg;
use std::sync::atomic::Ordering;

use super::startup::{background_priority, log};
use super::{App, TITLE};
use crate::index;

/// What a background update thread reports back.
pub(super) enum UpdateEvent {
    /// Result of a check; `manual` checks also report "up to date" and errors.
    Checked { manual: bool, result: Result<Option<crate::update::Release>, String> },
    Installed(Result<(), String>),
}

impl App {
    // ---------------------------------------------------------------- updates

    /// Looks for a newer release on GitHub in the background. The automatic
    /// check runs at most once a day and stays silent unless it finds one.
    pub(super) fn check_for_update(&self, manual: bool) {
        if self.update_busy.replace(true) {
            if manual {
                self.set_status("جاري التحقق من التحديثات…");
            }
            return;
        }
        if manual {
            self.set_status("جاري التحقق من التحديثات…");
        }
        let slot = self.update_result.clone();
        let notice = self.update_notice.sender();
        std::thread::spawn(move || {
            background_priority();
            let result = crate::update::check_latest();
            if let Ok(mut s) = slot.lock() {
                *s = Some(UpdateEvent::Checked { manual, result });
            }
            notice.notice();
        });
    }

    pub(super) fn maybe_check_for_update(&self) {
        let last = self.state.borrow().settings.update_check;
        if crate::update::should_check_now(last, index::now_secs().max(0) as u64) {
            self.check_for_update(false);
        }
    }

    pub(super) fn on_update_event(&self) {
        let Some(event) = self.update_result.lock().ok().and_then(|mut s| s.take()) else { return };
        match event {
            UpdateEvent::Checked { manual, result } => {
                self.update_busy.set(false);
                // A failed check (no network yet, rate limit) is retried at
                // the next hourly tick instead of a day later.
                match &result {
                    Ok(Some(rel)) => log(&format!("update: found {} (manual={manual})", rel.version)),
                    Ok(None) => log(&format!("update: up to date (manual={manual})")),
                    Err(e) => log(&format!("update: check failed (manual={manual}): {e}")),
                }
                if result.is_ok() {
                    let mut st = self.state.borrow_mut();
                    st.settings.update_check = index::now_secs().max(0) as u64;
                    st.settings.save();
                }
                match result {
                    Ok(Some(rel)) => self.offer_update(rel),
                    Ok(None) if manual => {
                        let v = env!("CARGO_PKG_VERSION");
                        self.set_status(&format!("لديك أحدث إصدار ({v})."));
                        nwg::modal_info_message(&self.window, TITLE, &format!("لديك أحدث إصدار من البرنامج ({v})."));
                    }
                    Err(e) if manual => {
                        self.set_status("تعذر التحقق من التحديثات.");
                        nwg::modal_error_message(&self.window, TITLE, &format!("تعذر التحقق من التحديثات:\n{e}"));
                    }
                    _ => {}
                }
            }
            UpdateEvent::Installed(Ok(())) => {
                self.set_status("اكتمل التحديث. جاري إعادة التشغيل…");
                match crate::update::restart_new_version() {
                    Ok(()) => {
                        // Save everything, then leave at once so the new copy
                        // gets the single-instance lock without waiting for
                        // the index to be freed.
                        self.on_close();
                        std::process::exit(0);
                    }
                    Err(e) => {
                        self.update_busy.set(false);
                        nwg::modal_info_message(
                            &self.window,
                            TITLE,
                            &format!("تم تثبيت الإصدار الجديد، لكن تعذرت إعادة التشغيل تلقائياً ({e}).\nأغلق البرنامج وافتحه من جديد."),
                        );
                    }
                }
            }
            UpdateEvent::Installed(Err(e)) => {
                log(&format!("update: install failed: {e}"));
                self.update_busy.set(false);
                self.set_status("تعذر التحديث.");
                nwg::modal_error_message(&self.window, TITLE, &format!("تعذر تنزيل التحديث أو تثبيته:\n{e}\n\nالإصدار الحالي يعمل كما هو."));
            }
        }
    }

    pub(super) fn offer_update(&self, rel: crate::update::Release) {
        // An automatic check must not pop up over a hidden window; show it.
        if !self.window.visible() {
            self.show_window();
        }
        let current = env!("CARGO_PKG_VERSION");
        let notes = if rel.notes.trim().is_empty() { String::new() } else { format!("\n\nالجديد:\n{}", rel.notes.trim()) };
        let text = format!(
            "يتوفر إصدار جديد من بحث سريع: {} (لديك {current}).{notes}\n\nهل تريد التحديث الآن؟ سيُعاد تشغيل البرنامج بعد التنزيل.",
            rel.version
        );
        let params = nwg::MessageParams {
            title: TITLE,
            content: &text,
            buttons: nwg::MessageButtons::YesNo,
            icons: nwg::MessageIcons::Question,
        };
        if nwg::modal_message(&self.window, &params) != nwg::MessageChoice::Yes {
            self.set_status(&format!("الإصدار {} متاح. يمكنك التحديث لاحقاً من «خيارات».", rel.version));
            return;
        }
        self.update_busy.set(true);
        self.save_settings();
        let total = rel.exe_size;
        self.update_progress.0.store(0, Ordering::Relaxed);
        self.update_progress.1.store(total, Ordering::Relaxed);
        let progress = self.update_progress.clone();
        let slot = self.update_result.clone();
        let notice = self.update_notice.sender();
        std::thread::spawn(move || {
            let result = crate::update::download_and_install(&rel, |done, total| {
                progress.0.store(done, Ordering::Relaxed);
                progress.1.store(total, Ordering::Relaxed);
            });
            progress.1.store(0, Ordering::Relaxed);
            if let Ok(mut s) = slot.lock() {
                *s = Some(UpdateEvent::Installed(result));
            }
            notice.notice();
        });
    }

    pub(super) fn poll_update_progress(&self) {
        let total = self.update_progress.1.load(Ordering::Relaxed);
        if total > 0 && self.update_busy.get() {
            let done = self.update_progress.0.load(Ordering::Relaxed);
            self.set_status(&format!("جاري تنزيل التحديث… {}٪", (done * 100 / total).min(100)));
        }
    }
}
