//! Renaming and deleting files from the result list, and saving the edited
//! index in the background.

use native_windows_gui as nwg;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use winapi::um::winuser::SendMessageW;

use super::shell;
use super::startup::background_priority;
use super::{App, Row, State, title};
use crate::index::{self, Index};

impl App {
    // ---------------------------------------------------------------- rename & delete

    /// Results of a running document search point into the index, so the
    /// index must not change under them.
    pub(super) fn index_busy(&self) -> bool {
        let st = self.state.borrow();
        let msg = if st.content.is_some() {
            tr!("انتظر انتهاء البحث داخل المستندات أو أوقفه أولاً.", "Wait for the document search to finish, or stop it first.")
        } else if st.scan.is_some() || st.loading.is_some() {
            tr!("انتظر حتى تكتمل الفهرسة.", "Wait until indexing is complete.")
        } else {
            return false;
        };
        drop(st);
        self.set_status(msg);
        true
    }

    pub(super) fn start_rename(&self) {
        let Some(row) = self.list.selected_item() else {
            self.set_status(tr!("اختر ملفاً من القائمة أولاً.", "Select a file in the list first."));
            return;
        };
        if self.index_busy() {
            return;
        }
        if let Some(h) = self.list.handle.hwnd() {
            self.list.set_focus();
            unsafe { SendMessageW(h, winapi::um::commctrl::LVM_EDITLABELW, row, 0) };
        }
    }

    /// Starts an in-place rename. Returns true to refuse it.
    pub(super) fn begin_rename(&self, row: usize) -> bool {
        if self.index_busy() {
            return true;
        }
        let path = self.row_path(row);
        let refuse = path.is_none();
        *self.renaming_path.borrow_mut() = path;
        self.renaming.set(!refuse);
        refuse
    }

    /// Finishes an in-place rename. Returns true to accept the new label.
    pub(super) fn finish_rename(&self, new_name: Option<String>) -> bool {
        self.renaming.set(false);
        let old = self.renaming_path.borrow_mut().take();
        let Some(new_name) = new_name.map(|n| n.trim().to_string()) else { return false };
        let Some(old) = old else { return false };
        let old_name = old.rsplit('\\').next().unwrap_or(&old).to_string();
        if new_name.is_empty() || new_name == old_name {
            return false;
        }
        if new_name.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) {
            self.set_status(tr!("لا يمكن أن يحتوي الاسم على أي من هذه الرموز: \\ / : * ? \" < > |", "A name cannot contain any of these characters: \\ / : * ? \" < > |"));
            return false;
        }
        // Windows drops a trailing dot or space and reserves device names.
        let stem = new_name.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit());
        if new_name.ends_with(['.', ' ']) || reserved {
            self.set_status(tr!("هذا الاسم غير مسموح في ويندوز.", "Windows does not allow this name."));
            return false;
        }
        let new = match old.rfind('\\') {
            Some(i) => format!("{}\\{new_name}", &old[..i]),
            None => new_name.clone(),
        };
        // fs::rename replaces an existing file, so check first. Changing only
        // the letter case of the same file is fine.
        if std::path::Path::new(&new).exists() && new.to_lowercase() != old.to_lowercase() {
            nwg::modal_info_message(&self.window, title(), &trf!("يوجد ملف آخر بالاسم «{new_name}» في هذا المجلد.", "Another file named “{new_name}” already exists in this folder."));
            return false;
        }
        if let Err(e) = std::fs::rename(&old, &new) {
            nwg::modal_error_message(&self.window, tr!("خطأ", "Error"), &trf!("تعذر تغيير الاسم:\n{e}", "Could not rename:\n{e}"));
            return false;
        }
        self.update_index(|idx| {
            idx.rename_path(&old, &new);
        });
        {
            let mut st = self.state.borrow_mut();
            st.history.rename_path(&old, &new);
            st.history.save();
        }
        self.set_status(&trf!("تم تغيير الاسم إلى «{new_name}».", "Renamed to “{new_name}”."));
        // The list is refreshed from the updated index; the edit box text is not needed.
        false
    }

    pub(super) fn delete_selected(&self) {
        let Some(p) = self.selected_path() else {
            self.set_status(tr!("اختر ملفاً من القائمة أولاً.", "Select a file in the list first."));
            return;
        };
        if self.index_busy() {
            return;
        }
        if !shell::recycle(&p) {
            self.set_status(tr!("لم يُحذف الملف.", "The file was not deleted."));
            return;
        }
        self.update_index(|idx| {
            idx.remove_path(&p);
        });
        {
            let mut st = self.state.borrow_mut();
            st.history.forget_path(&p);
            st.history.save();
        }
        self.set_status(tr!("نُقل الملف إلى سلة المحذوفات.", "The file was moved to the Recycle Bin."));
    }

    /// Applies a change to the index, saves it in the background and shows
    /// the results again.
    pub(super) fn update_index(&self, change: impl FnOnce(&mut Index)) {
        self.edit_index(
            |idx| {
                change(idx);
                true
            },
            false,
        );
    }

    /// True when an edit would have to copy the whole index, because a
    /// holder other than the window's own (rows, a waiting save) shares it.
    pub(super) fn index_shared(&self, st: &State) -> bool {
        let rows = usize::from(Arc::ptr_eq(&st.rows_index, &st.index));
        let save = PENDING_SAVE.lock().map_or(0, |p| usize::from(p.as_ref().is_some_and(|p| Arc::ptr_eq(p, &st.index))));
        Arc::strong_count(&st.index) > 1 + rows + save
    }

    /// `update_index` for a change that returns whether it changed anything.
    /// `live` marks a change found on disk by the watcher: it keeps the
    /// selection, re-runs only a non-empty search, and saves less often.
    pub(super) fn edit_index(&self, change: impl FnOnce(&mut Index) -> bool, live: bool) {
        let selected = if live { self.selected_path() } else { None };
        let mut st = self.state.borrow_mut();
        // Paths of the current rows, to find them again after the change.
        let rows = std::mem::take(&mut st.rows);
        let row_paths: Vec<(String, Row)> = rows
            .into_iter()
            .filter_map(|r| st.rows_index.entries.get(r.entry as usize).map(|e| (e.path.to_string(), r)))
            .collect();
        st.rows_index = Arc::default();
        // A save still waiting holds the index too; it is about to be stale.
        let had_pending = PENDING_SAVE.lock().map(|mut p| p.take().is_some()).unwrap_or(false);
        let mut idx = std::mem::take(&mut st.index);
        let inner = Arc::get_mut(&mut idx);
        let mut owned = match inner {
            Some(i) => std::mem::take(i),
            None => {
                // Something else (a search, the web server, a save) still
                // reads this snapshot: copy it. `apply_watched` paces this.
                st.live.last_copy = Some(std::time::Instant::now());
                Index { entries: idx.entries.clone(), roots: idx.roots.clone(), built_at: idx.built_at }
            }
        };
        let changed = change(&mut owned);
        let idx = Arc::new(owned);
        st.index = idx.clone();
        st.rows_index = idx.clone();
        st.rows = row_paths
            .into_iter()
            .filter_map(|(p, row)| idx.find(&p).map(|i| Row { entry: i as u32, ..row }))
            .collect();
        drop(st);
        // Several quick edits only need the last state written; on_close
        // writes it at once if it is still waiting.
        if (changed || had_pending)
            && let Ok(mut p) = PENDING_SAVE.lock()
        {
            *p = Some(idx);
        }
        if !changed {
            return;
        }
        // Files change all the time; writing a big index after each batch
        // would cost more than it saves, so watcher edits wait half a minute.
        if !live || !LIVE_SAVE_QUEUED.swap(true, Ordering::Relaxed) {
            crate::crash_log::spawn_named("index-save", move || {
                background_priority();
                std::thread::sleep(std::time::Duration::from_millis(if live { 30_000 } else { 500 }));
                if live {
                    LIVE_SAVE_QUEUED.store(false, Ordering::Relaxed);
                }
                flush_index_save();
            });
        }
        if live {
            self.refresh_list();
            self.reselect(selected);
            if !self.content_mode() && !self.query.text().trim().is_empty() {
                self.state.borrow_mut().live_search = true;
                self.search_names();
            }
            return;
        }
        self.clear_selection();
        self.refresh_list();
        if !self.content_mode() {
            self.search_names();
        }
    }
}

/// A save of watcher edits is already scheduled.
static LIVE_SAVE_QUEUED: AtomicBool = AtomicBool::new(false);

/// The edited index waiting to be written to disk (see `update_index`).
pub(super) static PENDING_SAVE: Mutex<Option<Arc<Index>>> = Mutex::new(None);

pub(super) fn flush_index_save() {
    let idx = PENDING_SAVE.lock().ok().and_then(|mut p| p.take());
    if let Some(idx) = idx {
        let _ = idx.save(&index::cache_path());
    }
}
