//! Checking for, downloading and installing a new version.

use native_windows_gui as nwg;
use std::sync::atomic::Ordering;

use super::startup::{background_priority, log};
use super::{App, title};
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
        // Installed with winget, Scoop or Chocolatey, or from the Store: they
        // update it, never the program itself (that would break their
        // records of the file, or cannot work at all).
        if let Some(pm) = crate::update::managed_by() {
            if manual && pm == crate::update::PackageManager::Store {
                self.show_store_update_note();
            } else if manual {
                log(&format!("update: skipped, installed with {}", pm.name()));
                self.set_status(&trf!("يُحدَّث عبر {}.", "Updated by {}.", pm.name()));
                self.modal_info(title(), &crate::update::managed_message(pm));
            }
            return;
        }
        // In a folder this copy cannot write to (Program Files): a
        // self-update would always fail, so none is offered.
        if let Some(dir) = crate::update::read_only_folder() {
            if manual {
                log(&format!("update: skipped, cannot write to {dir}"));
                nwg::modal_info_message(&self.window, title(), &crate::update::read_only_message(&dir));
            }
            return;
        }
        if self.update_busy.replace(true) {
            if manual {
                self.set_status(tr!("جارٍ التحقق من التحديثات…", "Checking for updates…"));
            }
            return;
        }
        if manual {
            self.set_status(tr!("جارٍ التحقق من التحديثات…", "Checking for updates…"));
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
        // The Store updates its own copy; no check against GitHub.
        if crate::store::is_packaged() {
            return;
        }
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
                        self.set_status(&trf!("لديك أحدث إصدار ({v}).", "You have the latest version ({v})."));
                        self.modal_info(title(), &trf!("لديك أحدث إصدار من البرنامج ({v}).", "You have the latest version of the program ({v})."));
                    }
                    Err(e) if manual => {
                        self.set_status(tr!("تعذر التحقق من التحديثات.", "Could not check for updates."));
                        self.modal_error(title(), &trf!("تعذر التحقق من التحديثات:\n{e}", "Could not check for updates:\n{e}"));
                    }
                    _ => {}
                }
            }
            UpdateEvent::Installed(Ok(())) => {
                self.set_status(tr!("اكتمل التحديث. جارٍ إعادة التشغيل…", "Update complete. Restarting…"));
                // Leaves only when the new version runs; otherwise the old
                // program file goes back in place, so the install always has
                // a program that starts.
                let failed = match crate::update::restart_new_version() {
                    Ok(child) => {
                        self.hand_over(child, "update");
                        tr!("الإصدار الجديد لم يعمل (ربما أوقفه برنامج الحماية).", "The new version did not start (perhaps blocked by security software).").to_string()
                    }
                    Err(e) => e,
                };
                match crate::update::roll_back_update() {
                    Ok(()) => {
                        log(&format!("update: rolled back to {} ({failed})", env!("CARGO_PKG_VERSION")));
                        self.update_busy.set(false);
                        nwg::modal_error_message(
                            &self.window,
                            title(),
                            &trf!("تعذر تطبيق التحديث: {failed}\nأُعيد الإصدار الحالي ({}) وهو يعمل كما هو.", "The update could not be applied: {failed}\nThe current version ({}) was put back and keeps working.", env!("CARGO_PKG_VERSION")),
                        );
                    }
                    Err(rb) => {
                        log(&format!("update: the new version did not start ({failed}); rolling back failed: {rb}"));
                        let e = failed;
                        self.update_busy.set(false);
                        self.modal_info(
                            title(),
                            &trf!("تم تثبيت الإصدار الجديد، لكن تعذرت إعادة التشغيل تلقائياً ({e}).\nأغلق البرنامج وافتحه من جديد.", "The new version is installed, but the program could not restart itself ({e}).\nClose the program and open it again."),
                        );
                    }
                }
            }
            UpdateEvent::Installed(Err(e)) => {
                log(&format!("update: install failed: {e}"));
                self.update_busy.set(false);
                self.set_status(tr!("تعذر التحديث.", "The update failed."));
                self.modal_error(title(), &trf!("تعذر تنزيل التحديث أو تثبيته:\n{e}\n\nالإصدار الحالي يعمل كما هو.", "Could not download or install the update:\n{e}\n\nThe current version keeps working as before."));
            }
        }
    }

    pub(super) fn offer_update(&self, rel: crate::update::Release) {
        // An automatic check must not pop up over a hidden window; show the
        // main window in use (the new look's, or the classic one).
        if !self.main_shown() {
            self.show_window();
        }
        let current = env!("CARGO_PKG_VERSION");
        let notes = if rel.notes.trim().is_empty() { String::new() } else { trf!("\n\nالجديد:\n{}", "\n\nWhat's new:\n{}", rel.notes.trim()) };
        let text = trf!(
            "يتوفر إصدار جديد من بحث سريع: {} (لديك {current}).{notes}\n\nهل تريد التحديث الآن؟ سيُعاد تشغيل البرنامج بعد التنزيل.",
            "A new version of InstaFind is available: {} (you have {current}).{notes}\n\nUpdate now? The program restarts after the download.",
            rel.version
        );
        let params = nwg::MessageParams {
            title: title(),
            content: &text,
            buttons: nwg::MessageButtons::YesNo,
            icons: nwg::MessageIcons::Question,
        };
        if self.with_modal_owner(|owner| nwg::modal_message(nwg::ControlHandle::Hwnd(owner), &params)) != nwg::MessageChoice::Yes {
            self.set_status(&trf!("الإصدار {} متاح. يمكنك التحديث لاحقاً من «خيارات».", "Version {} is available. You can update later from “Options”.", rel.version));
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

    /// A message box over the main window in use: the new look's when it
    /// is on screen (the classic window is hidden then), else the classic one.
    fn modal_info(&self, title: &str, text: &str) {
        self.with_modal_owner(|owner| nwg::modal_info_message(nwg::ControlHandle::Hwnd(owner), title, text));
    }

    fn modal_error(&self, title: &str, text: &str) {
        self.with_modal_owner(|owner| nwg::modal_error_message(nwg::ControlHandle::Hwnd(owner), title, text));
    }

    pub(super) fn poll_update_progress(&self) {
        let total = self.update_progress.1.load(Ordering::Relaxed);
        if total > 0 && self.update_busy.get() {
            let done = self.update_progress.0.load(Ordering::Relaxed);
            self.set_status(&trf!("جارٍ تنزيل التحديث… {}٪", "Downloading the update… {}%", (done * 100 / total).min(100)));
        }
    }

    /// Options ▸ Updates in the Microsoft Store version: the Store, not the
    /// program, installs new versions, so nothing is downloaded; the user
    /// is offered InstaFind's page in the Store instead.
    pub(super) fn show_store_update_note(&self) {
        log("update: Microsoft Store version; offering the Store page");
        self.set_status(tr!("التحديثات تأتي من Microsoft Store.", "Updates come from Microsoft Store."));
        let text = crate::store::update_message();
        let params = nwg::MessageParams {
            title: title(),
            content: &text,
            buttons: nwg::MessageButtons::YesNo,
            icons: nwg::MessageIcons::Info,
        };
        if self.with_modal_owner(|owner| nwg::modal_message(nwg::ControlHandle::Hwnd(owner), &params)) == nwg::MessageChoice::Yes
            && !super::shell::open(&crate::store::store_page_uri())
        {
            self.set_status(tr!("تعذر فتح Microsoft Store.", "Could not open Microsoft Store."));
        }
    }
}
