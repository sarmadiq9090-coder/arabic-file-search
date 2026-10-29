//! The result list: rows, selection, copying, sorting, export and the
//! virtual list's cell text, icons and match highlighting.

use native_windows_gui as nwg;
use std::collections::HashMap;
use std::io::Write;

use winapi::shared::minwindef::LPARAM;

use super::format::{csv_field, fmt_num, fmt_size, ltr_path};
use super::shell;
use super::{App, COL_DATE, COL_NAME, COL_PATH, COL_SIZE, COL_SNIPPET, COLUMNS, Row};
use crate::index;
use crate::search::{self, SortKey};

impl App {
    // ---------------------------------------------------------------- results

    /// Drops the selection, so a row number never points at a different file
    /// after the results change.
    pub(super) fn clear_selection(&self) {
        for i in self.list.selected_items() {
            self.list.select_item(i, false);
        }
    }

    pub(super) fn refresh_list(&self) {
        let n = self.state.borrow().rows.len();
        self.list.set_item_count(n as u32);
        self.list.invalidate();
        self.show_selected_path();
    }

    /// Full path of the file in result row `row`.
    pub(super) fn row_path(&self, row: usize) -> Option<String> {
        Some(self.state.borrow().row_entry(row)?.path.to_string())
    }

    pub(super) fn selected_path(&self) -> Option<String> {
        self.row_path(self.list.selected_item()?)
    }

    /// Shows the full path of the selected file in the status bar.
    pub(super) fn show_selected_path(&self) {
        let text = self.selected_path().map(|p| ltr_path(&p)).unwrap_or_default();
        self.status.set_text(1, &text);
    }

    /// Remembers that a result was used: its search goes into the history and
    /// the file ranks higher next time.
    pub(super) fn note_use(&self, path: &str) {
        let text = self.query.text();
        let mut st = self.state.borrow_mut();
        st.history.add_recent(&text);
        st.history.record_open(path, index::now_secs());
        st.history.save();
    }

    /// Keeps a search that found something in the recent list, so it can be
    /// picked again from the history menu.
    pub(super) fn remember_query(&self) {
        let text = self.query.text();
        let mut st = self.state.borrow_mut();
        if !st.rows.is_empty() {
            st.history.add_recent(&text);
            st.history.save();
        }
    }

    pub(super) fn open_selected(&self) {
        match self.selected_path() {
            Some(p) => {
                self.note_use(&p);
                if !shell::open(&p) {
                    nwg::modal_error_message(&self.window, "خطأ", &format!("تعذر فتح الملف:\n{p}"));
                }
            }
            None => self.set_status("اختر ملفاً من القائمة أولاً."),
        }
    }

    pub(super) fn open_folder(&self) {
        match self.selected_path() {
            Some(p) => {
                self.note_use(&p);
                shell::reveal(&p);
            }
            None => self.set_status("اختر ملفاً من القائمة أولاً."),
        }
    }

    pub(super) fn copy_path(&self) {
        match self.selected_path() {
            Some(p) => {
                nwg::Clipboard::set_data_text(&self.window, &p);
                self.set_status("تم نسخ المسار.");
            }
            None => self.set_status("اختر ملفاً من القائمة أولاً."),
        }
    }

    pub(super) fn copy_all_paths(&self) {
        let st = self.state.borrow();
        let paths: Vec<&str> =
            st.rows.iter().filter_map(|r| st.rows_index.entries.get(r.entry as usize)).map(|e| &*e.path).collect();
        if paths.is_empty() {
            drop(st);
            self.set_status("لا توجد نتائج لنسخها.");
            return;
        }
        let n = paths.len();
        nwg::Clipboard::set_data_text(&self.window, &paths.join("\r\n"));
        drop(st);
        self.set_status(&format!("تم نسخ {} مسار.", fmt_num(n)));
    }

    /// Puts the selected file itself on the clipboard, ready to paste into
    /// Explorer, an e-mail or a chat.
    pub(super) fn copy_file(&self) {
        let Some(p) = self.selected_path() else {
            self.set_status("اختر ملفاً من القائمة أولاً.");
            return;
        };
        let ok = self.window.handle.hwnd().is_some_and(|h| crate::dragdrop::copy_files(h, std::slice::from_ref(&p)));
        self.set_status(if ok { "تم نسخ الملف. الصقه حيث تريد بـ Ctrl+V." } else { "تعذر نسخ الملف." });
    }

    pub(super) fn drag_row(&self, row: usize) {
        if let (Some(p), Some(h)) = (self.row_path(row), self.list.handle.hwnd()) {
            crate::dragdrop::drag_files_from(h, std::slice::from_ref(&p));
        }
    }

    // ---------------------------------------------------------------- sorting

    pub(super) fn sort_key(&self) -> (SortKey, bool) {
        let st = self.state.borrow();
        let col = st.settings.sort_col;
        (if col < 0 { SortKey::Relevance } else { SortKey::from_column(col as usize) }, st.settings.sort_desc)
    }

    /// A click on a column header sorts by it; a second click reverses the
    /// order, and a third goes back to best match first.
    pub(super) fn on_column_click(&self, col: usize) {
        let key = SortKey::from_column(col);
        if key == SortKey::Relevance {
            return;
        }
        {
            let mut st = self.state.borrow_mut();
            let s = &mut st.settings;
            if s.sort_col == col as i32 {
                if s.sort_desc {
                    s.sort_col = -1;
                    s.sort_desc = false;
                } else {
                    s.sort_desc = true;
                }
            } else {
                s.sort_col = col as i32;
                s.sort_desc = false;
            }
        }
        self.show_sort_arrow();
        if self.content_mode() || self.state.borrow().content.is_some() {
            self.sort_rows_in_place();
            self.clear_selection();
            self.refresh_list();
        } else {
            self.search_names();
        }
    }

    pub(super) fn show_sort_arrow(&self) {
        let (key, desc) = self.sort_key();
        for c in 0..COLUMNS.len() {
            let arrow = match key.column() {
                Some(k) if k == c => {
                    Some(if desc { nwg::ListViewColumnSortArrow::Down } else { nwg::ListViewColumnSortArrow::Up })
                }
                _ => None,
            };
            self.list.set_column_sort_arrow(c, arrow);
        }
    }

    /// Sorts the current rows (used for document-search results).
    pub(super) fn sort_rows_in_place(&self) {
        let (key, desc) = self.sort_key();
        let mut st = self.state.borrow_mut();
        let mut hits: Vec<u32> = st.rows.iter().map(|r| r.entry).collect();
        if key == SortKey::Relevance {
            // Document hits have no score; path order is the neutral order.
            hits.sort_unstable();
        } else {
            search::sort_hits(&st.rows_index.entries, &mut hits, key, desc);
        }
        let mut snippets: HashMap<u32, Option<String>> = st.rows.drain(..).map(|r| (r.entry, r.snippet)).collect();
        st.rows = hits.into_iter().map(|entry| Row { entry, snippet: snippets.remove(&entry).flatten() }).collect();
    }

    /// Saves the result list as a CSV file that Excel opens with Arabic intact.
    pub(super) fn export_csv(&self) {
        if self.state.borrow().rows.is_empty() {
            self.set_status("لا توجد نتائج لتصديرها.");
            return;
        }
        let mut slot = self.export_dialog.borrow_mut();
        if slot.is_none() {
            let mut d = nwg::FileDialog::default();
            let built = nwg::FileDialog::builder()
                .title("حفظ النتائج")
                .action(nwg::FileDialogAction::Save)
                .filters("CSV(*.csv)")
                .build(&mut d);
            if built.is_err() {
                return;
            }
            *slot = Some(d);
        }
        let Some(dialog) = slot.as_ref() else { return };
        let chosen = if dialog.run(Some(&self.window)) { dialog.get_selected_item().ok() } else { None };
        drop(slot);
        let Some(path) = chosen else { return };
        let mut path = std::path::PathBuf::from(path);
        if path.extension().is_none() {
            path.set_extension("csv");
        }
        let st = self.state.borrow();
        let write = || -> std::io::Result<usize> {
            let mut f = std::io::BufWriter::new(std::fs::File::create(&path)?);
            f.write_all("\u{FEFF}الاسم,المسار,الحجم بالبايت,تاريخ التعديل,النص المطابق\r\n".as_bytes())?;
            let mut n = 0;
            for r in &st.rows {
                let Some(e) = st.rows_index.entries.get(r.entry as usize) else { continue };
                let size = if e.is_dir { String::new() } else { e.size.to_string() };
                let fields = [e.name(), &e.path, &size, &shell::format_time(e.mtime, false), r.snippet.as_deref().unwrap_or("")];
                let line: Vec<String> = fields.iter().map(|s| csv_field(s)).collect();
                f.write_all(line.join(",").as_bytes())?;
                f.write_all(b"\r\n")?;
                n += 1;
            }
            f.flush()?;
            Ok(n)
        };
        let result = write();
        drop(st);
        match result {
            Ok(n) => self.set_status(&format!("تم تصدير {} نتيجة.", fmt_num(n))),
            Err(e) => {
                nwg::modal_error_message(&self.window, "خطأ", &format!("تعذر حفظ الملف:\n{e}"));
            }
        }
    }

    /// Text for one cell of the virtual list.
    pub(super) fn cell_text(&self, row: usize, col: usize) -> String {
        let Ok(st) = self.state.try_borrow() else { return String::new() };
        let Some(r) = st.rows.get(row) else { return String::new() };
        let Some(e) = st.rows_index.entries.get(r.entry as usize) else { return String::new() };
        match col {
            COL_NAME => e.name().to_string(),
            COL_PATH => ltr_path(&e.path),
            COL_SIZE => fmt_size(e),
            COL_DATE => shell::format_time(e.mtime, true),
            COL_SNIPPET => r.snippet.clone().unwrap_or_default(),
            _ => String::new(),
        }
    }

    /// System image-list index of the small icon for a row's file type.
    pub(super) fn row_icon(&self, row: usize) -> i32 {
        let key = {
            let Ok(st) = self.state.try_borrow() else { return 0 };
            let Some(e) = st.row_entry(row) else { return 0 };
            if e.is_dir { String::from("/") } else { e.ext() }
        };
        if let Some(&i) = self.icons.borrow().get(&key) {
            return i;
        }
        let i = shell::type_icon(&key);
        self.icons.borrow_mut().insert(key, i);
        i
    }

    /// Byte ranges of the query's matches in a row's file name.
    pub(super) fn match_ranges(&self, row: usize) -> Vec<(usize, usize)> {
        let Ok(st) = self.state.try_borrow() else { return Vec::new() };
        let Some(q) = &st.shown_query else { return Vec::new() };
        let Some(e) = st.row_entry(row) else { return Vec::new() };
        search::match_ranges(e.name(), q)
    }

    pub(super) fn custom_draw(&self, l: LPARAM) -> Option<isize> {
        crate::draw::handle_custom_draw(l, |row| self.match_ranges(row), |row| self.cell_text(row, COL_NAME))
    }
}
