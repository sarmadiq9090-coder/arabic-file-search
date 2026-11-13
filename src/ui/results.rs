//! The result list: rows, selection, copying, sorting, export and the
//! virtual list's cell text, icons and match highlighting.

use native_windows_gui as nwg;
use std::io::Write;

use winapi::shared::minwindef::LPARAM;

use super::format::{csv_field, digits, fmt_num, ltr_path, shown_digits, ui_rtl};
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

    /// Selects the row showing `path` again after the rows changed, or
    /// nothing when it is no longer among them.
    pub(super) fn reselect(&self, path: Option<String>) {
        self.clear_selection();
        let Some(path) = path else { return };
        let row = {
            let st = self.state.borrow();
            (0..st.rows.len()).find(|&i| st.row_entry(i).is_some_and(|e| *e.path == *path))
        };
        if let Some(i) = row {
            self.list.select_item(i, true);
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
                if shell::open(&p) {
                    // Files only: opening a folder is not the success that counts.
                    if !std::path::Path::new(&p).is_dir() {
                        self.count_file_open();
                    }
                } else {
                    nwg::modal_error_message(&self.window, tr!("خطأ", "Error"), &trf!("تعذر فتح الملف:\n{p}", "Could not open the file:\n{p}"));
                }
            }
            None => self.set_status(tr!("اختر ملفاً من القائمة أولاً.", "Select a file in the list first.")),
        }
    }

    pub(super) fn open_folder(&self) {
        match self.selected_path() {
            Some(p) => {
                self.note_use(&p);
                shell::reveal(&p);
            }
            None => self.set_status(tr!("اختر ملفاً من القائمة أولاً.", "Select a file in the list first.")),
        }
    }

    /// Windows' Properties window ("properties") or "Open with" choice
    /// ("openas") for the selected result.
    pub(super) fn shell_verb_selected(&self, verb: &str) {
        match self.selected_path() {
            Some(p) => {
                if !shell::invoke_verb(self.window.handle.hwnd(), &p, verb) {
                    self.set_status(&trf!("تعذّر تنفيذ ذلك على: {}", "Windows could not do that for: {}", ltr_path(&p)));
                }
            }
            None => self.set_status(tr!("اختر ملفاً من القائمة أولاً.", "Select a file in the list first.")),
        }
    }

    pub(super) fn copy_path(&self) {
        match self.selected_path() {
            Some(p) => {
                if shell::copy_text(&self.window, &p) {
                    self.set_status(tr!("تم نسخ المسار.", "Path copied."));
                } else {
                    self.set_status(shell::copy_failed_text());
                }
            }
            None => self.set_status(tr!("اختر ملفاً من القائمة أولاً.", "Select a file in the list first.")),
        }
    }

    pub(super) fn copy_all_paths(&self) {
        let st = self.state.borrow();
        let paths: Vec<&str> =
            st.rows.iter().filter_map(|r| st.rows_index.entries.get(r.entry as usize)).map(|e| &*e.path).collect();
        if paths.is_empty() {
            drop(st);
            self.set_status(tr!("لا توجد نتائج لنسخها.", "There are no results to copy."));
            return;
        }
        let n = paths.len();
        let text = paths.join("\r\n");
        // Not borrowed while the clipboard notifies its previous owner.
        drop(st);
        if shell::copy_text(&self.window, &text) {
            self.set_status(&trf!("تم نسخ المسارات. عددها: {}.", "Copied {} paths.", fmt_num(n)));
        } else {
            self.set_status(shell::copy_failed_text());
        }
    }

    /// Puts the selected file itself on the clipboard, ready to paste into
    /// Explorer, an e-mail or a chat.
    pub(super) fn copy_file(&self) {
        let Some(p) = self.selected_path() else {
            self.set_status(tr!("اختر ملفاً من القائمة أولاً.", "Select a file in the list first."));
            return;
        };
        let ok = self.window.handle.hwnd().is_some_and(|h| crate::dragdrop::copy_files(h, std::slice::from_ref(&p)));
        self.set_status(if ok { tr!("تم نسخ الملف. الصقه حيث تريد بـ Ctrl+V.", "File copied. Paste it anywhere with Ctrl+V.") } else { tr!("تعذر نسخ الملف.", "Could not copy the file.") });
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
            self.sort_rows_in_place(None);
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

    /// Sorts the current rows (used for document-search results) and
    /// returns where row `selected` went, for [`App::select_row`].
    pub(super) fn sort_rows_in_place(&self, selected: Option<usize>) -> Option<usize> {
        let (key, desc) = self.sort_key();
        let mut st = self.state.borrow_mut();
        let st = &mut *st;
        search::sort_rows(&st.rows_index.entries, &mut st.rows, |r: &Row| r.entry, key, desc, selected)
    }

    /// Moves the selection to `row` (to nothing for `None`) after the rows
    /// were put in another order, so it stays on the same file.
    pub(super) fn select_row(&self, row: Option<usize>) {
        self.clear_selection();
        if let Some(i) = row {
            self.list.select_item(i, true);
        }
        self.show_selected_path();
    }

    /// Opens a window listing every matching line of the selected document.
    pub(super) fn show_matches(&self) {
        let Some(row) = self.list.selected_item() else {
            self.set_status(tr!("اختر ملفاً من القائمة أولاً.", "Select a file in the list first."));
            return;
        };
        // Lines not kept for this result are read again in the background,
        // and the window opens when they are there (`on_matches_read`).
        if self.read_matches_again(row) {
            self.set_status(tr!("جارٍ قراءة التطابقات من الملف…", "Reading the matches from the file…"));
            return;
        }
        self.show_matches_of(row);
    }

    fn show_matches_of(&self, row: usize) {
        let text = {
            let st = self.state.borrow();
            let (Some(r), Some(e)) = (st.rows.get(row), st.row_entry(row)) else { return };
            let Some(m) = &r.matches else {
                drop(st);
                self.set_status(tr!("كل التطابقات متاحة لنتائج البحث داخل المستندات.", "All matches are available for results of a document search."));
                return;
            };
            let mut t = trf!("{}\r\nعدد التطابقات: {}\r\n", "{}\r\nMatches: {}\r\n", ltr_path(&e.path), digits(m.count.to_string()));
            if m.lines.len() < m.count as usize {
                t.push_str(&trf!("(أول الأسطر فقط، عددها: {})\r\n", "(first {} lines)\r\n", digits(m.lines.len().to_string())));
            }
            for (line, s) in &m.lines {
                t.push_str(&trf!("\r\nسطر {}:  {}", "\r\nLine {}:  {}", digits(line.to_string()), shown_digits(&s.replace(['\r', '\n'], " "))));
            }
            self.matches_window.set_text(&trf!("كل التطابقات — {}", "All matches — {}", shown_digits(e.name())));
            t
        };
        self.matches_text.set_text(&text);
        self.matches_window.set_visible(true);
        self.matches_text.set_focus();
    }

    /// A search inside documents keeps the matching lines of its first
    /// results only (`search::LINED_HITS`); for a later one they are read
    /// again from the document (or the content index) on a background
    /// thread, never on this one: a big document can take a long time. True
    /// when that read was started.
    fn read_matches_again(&self, row: usize) -> bool {
        let (path, q, use_index) = {
            let st = self.state.borrow();
            let Some(m) = st.rows.get(row).and_then(|r| r.matches.as_ref()) else { return false };
            if !m.lines.is_empty() || m.count == 0 {
                return false;
            }
            let Some(path) = st.row_entry(row).map(|e| e.path.to_string()) else { return false };
            let Some(q) = st.shown_query.clone() else { return false };
            (path, q, st.settings.content_index)
        };
        let slot = self.matches_read.clone();
        let notice = self.matches_notice.sender();
        crate::crash_log::spawn_named("matches-read", move || {
            let cache = if use_index { super::content_index::shared() } else { None };
            let found = crate::crash_log::catch(|| search::document_matches(&path, &q, cache)).flatten();
            if let Ok(mut s) = slot.lock() {
                s.push(super::MatchesRead::Classic { path, found });
            }
            notice.notice();
        })
        .is_some()
    }

    /// Matches read again in the background: kept with their row, and shown
    /// (classic window: when that row is still the selected one).
    pub(super) fn on_matches_read(&self) {
        let done = self.matches_read.lock().map(|mut s| std::mem::take(&mut *s)).unwrap_or_default();
        for read in done {
            match read {
                super::MatchesRead::Classic { path, found } => {
                    let row = {
                        let mut st = self.state.borrow_mut();
                        let Some(row) = (0..st.rows.len()).find(|&r| st.row_entry(r).is_some_and(|e| *e.path == *path)) else { continue };
                        if let (Some(m), Some((count, lines))) = (st.rows.get_mut(row).and_then(|r| r.matches.as_mut()), found) {
                            m.count = count;
                            m.lines = lines;
                        }
                        row
                    };
                    if self.list.selected_item() == Some(row) {
                        self.show_matches_of(row);
                    }
                }
                super::MatchesRead::Web { seq, ver, id, found } => self.webui_matches_read(seq, ver, id, found),
            }
        }
    }

    /// Saves the result list as a CSV file that Excel opens with Arabic intact,
    /// in columns. The file starts with the UTF-8 BOM (Excel reads Arabic
    /// only with it) and uses Windows' list separator, as Excel does when
    /// it opens a CSV: `,` in most regions, `;` where the decimal mark is a
    /// comma. A `sep=,` first line is not used: the BOM has to come first,
    /// and Excel then ignores the BOM and shows the Arabic as gibberish.
    pub(super) fn export_csv(&self) {
        // The results on screen: the new look's while it is the main window.
        let rows = self.webui_results().unwrap_or_else(|| {
            let st = self.state.borrow();
            st.rows
                .iter()
                .filter_map(|r| {
                    let e = st.rows_index.entries.get(r.entry as usize)?;
                    Some(crate::webui::bridge::ExportRow {
                        path: e.path.to_string(),
                        size: (!e.is_dir).then_some(e.size),
                        mtime: e.mtime,
                        snippet: r.snippet.clone().unwrap_or_default(),
                    })
                })
                .collect()
        });
        if rows.is_empty() {
            self.set_status(tr!("لا توجد نتائج لتصديرها.", "There are no results to export."));
            return;
        }
        // Taken out while it runs: the dialog pumps messages, and a handler
        // that runs meanwhile must not find the cell borrowed.
        let mut slot = self.export_dialog.take();
        if slot.is_none() {
            let mut d = nwg::FileDialog::default();
            let built = nwg::FileDialog::builder()
                .title(tr!("حفظ النتائج", "Save results"))
                .action(nwg::FileDialogAction::Save)
                .filters("CSV(*.csv)")
                .build(&mut d);
            if built.is_err() {
                return;
            }
            slot = Some(d);
        }
        let Some(dialog) = slot.as_ref() else { return };
        let chosen = if self.with_modal_owner(|owner| dialog.run(Some(nwg::ControlHandle::Hwnd(owner)))) { dialog.get_selected_item().ok() } else { None };
        *self.export_dialog.borrow_mut() = slot;
        let Some(path) = chosen else { return };
        let mut path = std::path::PathBuf::from(path);
        if path.extension().is_none() {
            path.set_extension("csv");
        }
        let sep = csv_separator(&list_separator());
        let write = || -> std::io::Result<usize> {
            let mut f = std::io::BufWriter::new(std::fs::File::create(&path)?);
            let head = [
                tr!("الاسم", "Name"),
                tr!("المسار", "Path"),
                tr!("الحجم بالبايت", "Size in bytes"),
                tr!("تاريخ التعديل", "Date modified"),
                tr!("النص المطابق", "Matching text"),
            ];
            f.write_all(format!("\u{FEFF}{}", csv_line(&head, sep)).as_bytes())?;
            for r in &rows {
                let name = r.path.rsplit('\\').next().unwrap_or(&r.path);
                let size = r.size.map(|s| s.to_string()).unwrap_or_default();
                let fields = [name, &r.path, &size, &shell::format_time(r.mtime, false), &r.snippet];
                f.write_all(csv_line(&fields, sep).as_bytes())?;
            }
            f.flush()?;
            Ok(rows.len())
        };
        match write() {
            Ok(n) => self.set_status(&trf!("تم التصدير. عدد النتائج: {}.", "Exported {} results.", fmt_num(n))),
            Err(e) => {
                let text = trf!("تعذر حفظ الملف:\n{e}", "Could not save the file:\n{e}");
                self.with_modal_owner(|owner| nwg::modal_error_message(nwg::ControlHandle::Hwnd(owner), tr!("خطأ", "Error"), &text));
            }
        }
    }

    /// Text for one cell of the virtual list.
    pub(super) fn cell_text(&self, row: usize, col: usize) -> String {
        let Ok(st) = self.state.try_borrow() else { return String::new() };
        let Some(r) = st.rows.get(row) else { return String::new() };
        let Some(e) = st.rows_index.entries.get(r.entry as usize) else { return String::new() };
        match col {
            // An Arabic name's extension kept whole (".xlsx") at its end,
            // its dates and phone numbers as typed.
            COL_NAME => crate::bidi_layout::list_name(e.name(), e.is_dir, ui_rtl()).0,
            // A drive that is not connected keeps its entries; a quiet note says so.
            COL_PATH if crate::drives::is_absent(&e.path) => format!("{}  ({})", ltr_path(&e.path), crate::drives::absent_note()),
            COL_PATH => ltr_path(&e.path),
            COL_SIZE => self.size_cell(&st.rows_index, r.entry, e),
            COL_DATE => shell::format_time(e.mtime, true),
            COL_SNIPPET => {
                let snippet = shown_digits(r.snippet.as_deref().unwrap_or_default());
                match &r.matches {
                    // "(12) …the first match…": how many places matched.
                    Some(m) if m.count > 1 => format!("({}) {snippet}", digits(m.count.to_string())),
                    _ => snippet,
                }
            }
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

    /// Byte ranges of the query's matches in a row's file name, as the
    /// list shows it (`cell_text`, with the marks `bidi_layout::list_name` puts in).
    pub(super) fn match_ranges(&self, row: usize) -> Vec<(usize, usize)> {
        let Ok(st) = self.state.try_borrow() else { return Vec::new() };
        let Some(q) = &st.shown_query else { return Vec::new() };
        let Some(e) = st.row_entry(row) else { return Vec::new() };
        let ranges = search::match_ranges(e.name(), q);
        if ranges.is_empty() {
            return ranges;
        }
        crate::bidi_layout::shift_ranges(ranges, &crate::bidi_layout::list_name(e.name(), e.is_dir, ui_rtl()).1)
    }

    pub(super) fn custom_draw(&self, l: LPARAM) -> Option<isize> {
        crate::draw::handle_custom_draw(l, |row| self.match_ranges(row), |row| self.cell_text(row, COL_NAME))
    }
}

/// Windows' list separator for the user (Region ▸ Additional settings ▸
/// List separator), the one Excel splits a CSV it opens by. Empty when it
/// can't be read.
fn list_separator() -> String {
    // Declared here: winapi's binding needs the "winnls" feature.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLocaleInfoW(locale: u32, lctype: u32, data: *mut u16, len: i32) -> i32;
    }
    const LOCALE_USER_DEFAULT: u32 = 0x0400;
    const LOCALE_SLIST: u32 = 0x000C;
    let mut buf = [0u16; 8];
    // SAFETY: `buf` holds `buf.len()` units; the call writes at most that many.
    let n = unsafe { GetLocaleInfoW(LOCALE_USER_DEFAULT, LOCALE_SLIST, buf.as_mut_ptr(), buf.len() as i32) };
    let n = usize::try_from(n).unwrap_or(0).min(buf.len());
    String::from_utf16_lossy(&buf[..n]).trim_end_matches('\0').to_string()
}

/// The separator for an exported CSV: Windows' list separator when it is
/// one character Excel can split by, else a comma.
fn csv_separator(list: &str) -> char {
    let mut chars = list.chars();
    match (chars.next(), chars.next()) {
        (Some(c @ (',' | ';' | '\t' | '|')), None) => c,
        _ => ',',
    }
}

/// One CSV line, ending with CRLF: each field through `csv_field` (quotes,
/// and the apostrophe that stops formulas), and quoted when it holds `sep`.
fn csv_line(fields: &[&str], sep: char) -> String {
    let mut line = String::new();
    for (i, s) in fields.iter().enumerate() {
        if i > 0 {
            line.push(sep);
        }
        let f = csv_field(s);
        // `csv_field` quotes for commas; an unquoted field has no `"` in it.
        if !f.starts_with('"') && f.contains(sep) {
            line.push('"');
            line.push_str(&f);
            line.push('"');
        } else {
            line.push_str(&f);
        }
    }
    line.push_str("\r\n");
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_uses_the_list_separator_and_keeps_the_formula_guard() {
        assert_eq!(csv_separator(";"), ';');
        assert_eq!(csv_separator(","), ',');
        for odd in ["", "ab", ";;", "x", "\u{060C}"] {
            assert_eq!(csv_separator(odd), ',', "{odd:?}");
        }
        assert_eq!(csv_line(&["تقرير.pdf", "C:\\a;b", "=1+1", "x,y"], ';'), "تقرير.pdf;\"C:\\a;b\";'=1+1;\"x,y\"\r\n");
        assert_eq!(csv_line(&["a", "say \"hi\"", "-1"], ','), "a,\"say \"\"hi\"\"\",'-1\r\n");
        assert_eq!(csv_line(&["=a;b"], ';'), "\"'=a;b\"\r\n");
        // The locale call answers something (a comma on CI runners).
        let _ = csv_separator(&list_separator());
    }
}
