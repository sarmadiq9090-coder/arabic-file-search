//! Renaming and deleting files from the result list, and saving the edited
//! index in the background.

use native_windows_gui as nwg;
use std::sync::{Arc, Mutex};

use winapi::um::winuser::SendMessageW;

use super::shell;
use super::startup::background_priority;
use super::{App, Row, TITLE};
use crate::index::{self, Index};

impl App {
    // ---------------------------------------------------------------- rename & delete

    /// Results of a running document search point into the index, so the
    /// index must not change under them.
    pub(super) fn index_busy(&self) -> bool {
        let st = self.state.borrow();
        let msg = if st.content.is_some() {
            "انتظر انتهاء البحث داخل المستندات أو أوقفه أولاً."
        } else if st.scan.is_some() || st.loading.is_some() {
            "انتظر حتى تكتمل الفهرسة."
        } else {
            return false;
        };
        drop(st);
        self.set_status(msg);
        true
    }

    pub(super) fn start_rename(&self) {
        let Some(row) = self.list.selected_item() else {
            self.set_status("اختر ملفاً من القائمة أولاً.");
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
            self.set_status("لا يمكن أن يحتوي الاسم على أي من هذه الرموز: \\ / : * ? \" < > |");
            return false;
        }
        // Windows drops a trailing dot or space and reserves device names.
        let stem = new_name.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit());
        if new_name.ends_with(['.', ' ']) || reserved {
            self.set_status("هذا الاسم غير مسموح في ويندوز.");
            return false;
        }
        let new = match old.rfind('\\') {
            Some(i) => format!("{}\\{new_name}", &old[..i]),
            None => new_name.clone(),
        };
        // fs::rename replaces an existing file, so check first. Changing only
        // the letter case of the same file is fine.
        if std::path::Path::new(&new).exists() && new.to_lowercase() != old.to_lowercase() {
            nwg::modal_info_message(&self.window, TITLE, &format!("يوجد ملف آخر بالاسم «{new_name}» في هذا المجلد."));
            return false;
        }
        if let Err(e) = std::fs::rename(&old, &new) {
            nwg::modal_error_message(&self.window, "خطأ", &format!("تعذر تغيير الاسم:\n{e}"));
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
        self.set_status(&format!("تم تغيير الاسم إلى «{new_name}»."));
        // The list is refreshed from the updated index; the edit box text is not needed.
        false
    }

    pub(super) fn delete_selected(&self) {
        let Some(p) = self.selected_path() else {
            self.set_status("اختر ملفاً من القائمة أولاً.");
            return;
        };
        if self.index_busy() {
            return;
        }
        if !shell::recycle(&p) {
            self.set_status("لم يُحذف الملف.");
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
        self.set_status("نُقل الملف إلى سلة المحذوفات.");
    }

    /// Applies a change to the index, saves it in the background and shows
    /// the results again.
    pub(super) fn update_index(&self, change: impl FnOnce(&mut Index)) {
        let mut st = self.state.borrow_mut();
        // Paths of the current rows, to find them again after the change.
        let rows = std::mem::take(&mut st.rows);
        let row_paths: Vec<(String, Row)> = rows
            .into_iter()
            .filter_map(|r| st.rows_index.entries.get(r.entry as usize).map(|e| (e.path.to_string(), r)))
            .collect();
        st.rows_index = Arc::default();
        // A save still waiting holds the index too; it is about to be stale.
        PENDING_SAVE.lock().map(|mut p| p.take()).ok();
        let mut idx = std::mem::take(&mut st.index);
        let inner = Arc::get_mut(&mut idx);
        let mut owned = match inner {
            Some(i) => std::mem::take(i),
            None => Index { entries: idx.entries.clone(), roots: idx.roots.clone(), built_at: idx.built_at },
        };
        change(&mut owned);
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
        if let Ok(mut p) = PENDING_SAVE.lock() {
            *p = Some(idx);
        }
        std::thread::spawn(|| {
            background_priority();
            std::thread::sleep(std::time::Duration::from_millis(500));
            flush_index_save();
        });
        self.clear_selection();
        self.refresh_list();
        if !self.content_mode() {
            self.search_names();
        }
    }
}

/// The edited index waiting to be written to disk (see `update_index`).
pub(super) static PENDING_SAVE: Mutex<Option<Arc<Index>>> = Mutex::new(None);

pub(super) fn flush_index_save() {
    let idx = PENDING_SAVE.lock().ok().and_then(|mut p| p.take());
    if let Some(idx) = idx {
        let _ = idx.save(&index::cache_path());
    }
}
