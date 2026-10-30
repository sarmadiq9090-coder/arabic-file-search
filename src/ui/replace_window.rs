//! The "Replace text in files" window: find and replace across the plain
//! text files of the current results, with a preview, a confirmation, a
//! backup of every changed file and an Undo of the last replace. The work
//! itself is in `crate::replace`; it runs on a background thread.

use native_windows_gui as nwg;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use winapi::um::commctrl::{LVIS_STATEIMAGEMASK, LVITEMW, LVM_GETITEMSTATE, LVM_SETITEMSTATE, LVS_EX_CHECKBOXES};
use winapi::um::winuser::SendMessageW;

use super::format::{digits, fmt_num, ltr_path};
use super::{App, WS_EX_LAYOUTRTL};
use crate::replace::{self, ApplyReport, Matcher, Options, Outcome, Sample, UndoReport};

/// At most this many result files are offered.
const MAX_FILES: usize = 10_000;

enum Done {
    Preview(Options, Vec<(String, Outcome)>),
    Applied(Result<ApplyReport, String>),
    Undone(Result<UndoReport, String>),
}

/// A line of the preview list.
struct PreviewRow {
    path: String,
    /// False for a skipped file: never replaced, even when checked.
    can: bool,
    count: usize,
    samples: Vec<Sample>,
    note: String,
}

#[derive(Default)]
struct ReplaceState {
    /// The plain-text files of the results when the window was opened.
    files: Vec<String>,
    rows: Vec<PreviewRow>,
    /// The options the preview was made with; Replace uses exactly these.
    previewed: Option<Options>,
    busy: bool,
}

/// The window, its controls and its state: one field of `App`.
#[derive(Default)]
pub struct ReplaceWindow {
    menu_item: nwg::MenuItem,
    options_item: nwg::MenuItem,
    window: nwg::Window,
    find_label: nwg::Label,
    find: nwg::TextInput,
    repl_label: nwg::Label,
    repl: nwg::TextInput,
    ignore_case: nwg::CheckBox,
    regex: nwg::CheckBox,
    preview_btn: nwg::Button,
    apply_btn: nwg::Button,
    undo_btn: nwg::Button,
    list: nwg::ListView,
    detail: nwg::TextBox,
    status: nwg::Label,
    notice: nwg::Notice,
    done: Arc<Mutex<Option<Done>>>,
    progress: Arc<AtomicUsize>,
    state: RefCell<ReplaceState>,
}

/// `%LOCALAPPDATA%\BahthSari\backups`.
fn backups_root() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("BahthSari")
        .join("backups")
}

fn checked(v: &nwg::CheckBox) -> bool {
    v.check_state() == nwg::CheckBoxState::Checked
}

impl ReplaceWindow {
    fn options(&self) -> Options {
        Options {
            find: self.find.text(),
            replace: self.repl.text(),
            ignore_case: checked(&self.ignore_case),
            regex: checked(&self.regex),
        }
    }

    fn is_checked(&self, row: usize) -> bool {
        let Some(h) = self.list.handle.hwnd() else { return false };
        let state = unsafe { SendMessageW(h, LVM_GETITEMSTATE, row, LVIS_STATEIMAGEMASK as isize) } as u32;
        state >> 12 == 2
    }

    fn set_checked(&self, row: usize, on: bool) {
        let Some(h) = self.list.handle.hwnd() else { return };
        let mut item: LVITEMW = unsafe { std::mem::zeroed() };
        item.stateMask = LVIS_STATEIMAGEMASK;
        item.state = (if on { 2 } else { 1 }) << 12;
        unsafe { SendMessageW(h, LVM_SETITEMSTATE, row, &mut item as *mut LVITEMW as isize) };
    }

    fn set_busy(&self, busy: bool) {
        self.state.borrow_mut().busy = busy;
        for b in [&self.preview_btn, &self.undo_btn] {
            b.set_enabled(!busy);
        }
        self.apply_btn.set_enabled(!busy && self.state.borrow().previewed.is_some());
    }

    fn layout(&self) {
        let (w, h) = self.window.size();
        let (w, h) = (w as i32, h as i32);
        let m = 10;
        let lw = 120;
        let fw = (w - 3 * m - lw).max(100) as u32;
        self.find_label.set_position(m, m + 4);
        self.find_label.set_size(lw as u32, 24);
        self.find.set_position(2 * m + lw, m);
        self.find.set_size(fw, 28);
        self.repl_label.set_position(m, m + 42);
        self.repl_label.set_size(lw as u32, 24);
        self.repl.set_position(2 * m + lw, m + 38);
        self.repl.set_size(fw, 28);
        self.ignore_case.set_position(2 * m + lw, m + 74);
        self.ignore_case.set_size(220, 26);
        self.regex.set_position(3 * m + lw + 220, m + 74);
        self.regex.set_size(220, 26);
        let by = m + 108;
        let mut x = m;
        for (b, bw) in [(&self.preview_btn, 130), (&self.apply_btn, 130), (&self.undo_btn, 200)] {
            b.set_position(x, by);
            b.set_size(bw, 32);
            x += bw as i32 + m;
        }
        let ly = by + 32 + m;
        let status_h = 24;
        let detail_h = 110;
        let lh = (h - ly - detail_h - status_h - 3 * m).max(60);
        self.list.set_position(m, ly);
        self.list.set_size((w - 2 * m) as u32, lh as u32);
        self.detail.set_position(m, ly + lh + m);
        self.detail.set_size((w - 2 * m) as u32, detail_h as u32);
        self.status.set_position(m, ly + lh + 2 * m + detail_h);
        self.status.set_size((w - 2 * m) as u32, status_h as u32);
    }

    fn set_status(&self, text: &str) {
        self.status.set_text(text);
    }

    fn clear_preview(&self) {
        let mut st = self.state.borrow_mut();
        st.rows.clear();
        st.previewed = None;
        drop(st);
        self.list.clear();
        self.detail.set_text("");
        self.apply_btn.set_enabled(false);
    }

    /// Shows the before/after lines of the selected file.
    fn show_detail(&self) {
        let st = self.state.borrow();
        let Some(row) = self.list.selected_item().and_then(|i| st.rows.get(i)) else {
            self.detail.set_text("");
            return;
        };
        let mut t = ltr_path(&row.path);
        if !row.can {
            t.push_str(&format!("\r\n{}", row.note));
        }
        for s in &row.samples {
            t.push_str(&trf!("\r\n\r\nسطر {}:", "\r\n\r\nLine {}:", digits(s.line.to_string())));
            t.push_str(&format!("\r\n  − {}\r\n  + {}", s.before, s.after));
        }
        self.detail.set_text(&t);
    }
}

impl App {
    /// Adds the menu items and the (hidden) window. Called from `build_ui`.
    pub(super) fn build_replace_window(&mut self) -> Result<(), nwg::NwgError> {
        let r = &mut self.replace;
        let label = tr!("استبدال نص في الملفات…", "Replace text in files…");
        nwg::MenuItem::builder().text(label).parent(&self.menu).build(&mut r.menu_item)?;
        nwg::MenuItem::builder().text(label).parent(&self.options_menu).build(&mut r.options_item)?;

        let f = Some(&self.font);
        nwg::Window::builder()
            .title(tr!("استبدال نص في الملفات", "Replace text in files"))
            .size((820, 600))
            .center(true)
            .ex_flags(crate::i18n::rtl_only(WS_EX_LAYOUTRTL))
            .flags(nwg::WindowFlags::WINDOW | nwg::WindowFlags::RESIZABLE)
            .icon(None)
            .parent(Some(&self.window))
            .build(&mut r.window)?;
        let w = &r.window;
        nwg::Label::builder().parent(w).font(f).text(tr!("البحث عن:", "Find:")).build(&mut r.find_label)?;
        nwg::TextInput::builder().parent(w).font(f).build(&mut r.find)?;
        nwg::Label::builder().parent(w).font(f).text(tr!("استبداله بـ:", "Replace with:")).build(&mut r.repl_label)?;
        nwg::TextInput::builder().parent(w).font(f).build(&mut r.repl)?;
        nwg::CheckBox::builder().parent(w).font(f).text(tr!("تجاهل حالة الأحرف", "Ignore case")).build(&mut r.ignore_case)?;
        nwg::CheckBox::builder().parent(w).font(f).text(tr!("تعبير نمطي (Regex)", "Regular expression")).build(&mut r.regex)?;
        nwg::Button::builder().parent(w).font(f).text(tr!("معاينة", "Preview")).build(&mut r.preview_btn)?;
        nwg::Button::builder().parent(w).font(f).text(tr!("استبدال…", "Replace…")).enabled(false).build(&mut r.apply_btn)?;
        nwg::Button::builder().parent(w).font(f).text(tr!("تراجع عن آخر استبدال…", "Undo last replace…")).build(&mut r.undo_btn)?;
        nwg::ListView::builder()
            .parent(w)
            .list_style(nwg::ListViewStyle::Detailed)
            .flags(nwg::ListViewFlags::VISIBLE | nwg::ListViewFlags::TAB_STOP | nwg::ListViewFlags::SINGLE_SELECTION | nwg::ListViewFlags::ALWAYS_SHOW_SELECTION)
            .ex_flags(unsafe {
                nwg::ListViewExFlags::from_bits_unchecked(nwg::ListViewExFlags::FULL_ROW_SELECT.bits() | LVS_EX_CHECKBOXES)
            })
            .build(&mut r.list)?;
        if let Some(h) = r.list.handle.hwnd() {
            unsafe { SendMessageW(h, winapi::um::winuser::WM_SETFONT, self.font.handle as usize, 1) };
        }
        let scale = nwg::scale_factor();
        for (i, (text, width)) in [(tr!("الملف", "File"), 500), (tr!("التطابقات", "Matches"), 90), (tr!("الترميز / الحالة", "Encoding / status"), 190)]
            .into_iter()
            .enumerate()
        {
            r.list.insert_column(nwg::InsertListViewColumn {
                index: Some(i as i32),
                fmt: None,
                width: Some((width as f64 * scale).round() as i32),
                text: Some(text.to_string()),
            });
        }
        r.list.set_headers_enabled(true);
        nwg::TextBox::builder()
            .parent(w)
            .font(f)
            .readonly(true)
            .flags(nwg::TextBoxFlags::VISIBLE | nwg::TextBoxFlags::VSCROLL | nwg::TextBoxFlags::AUTOVSCROLL)
            .build(&mut r.detail)?;
        nwg::Label::builder().parent(w).font(f).text("").build(&mut r.status)?;
        nwg::Notice::builder().parent(w).build(&mut r.notice)?;
        r.layout();
        Ok(())
    }

    /// Binds the window's events, and the menu items on the main window.
    /// Called once from `build_ui`; the handlers are unbound with the others.
    pub(super) fn setup_replace_window(self: &Rc<Self>) -> (nwg::EventHandler, nwg::EventHandler) {
        let weak = Rc::downgrade(self);
        let main = nwg::full_bind_event_handler(&self.window.handle, move |evt, _data, handle| {
            let Some(app) = weak.upgrade() else { return };
            let r = &app.replace;
            match evt {
                nwg::Event::OnMenuItemSelected if handle == r.menu_item || handle == r.options_item => app.open_replace(),
                nwg::Event::OnTimerTick if handle == app.timer && r.state.borrow().busy => {
                    let n = r.progress.load(Ordering::Relaxed);
                    r.set_status(&trf!("جارٍ العمل… ({} ملف)", "Working… ({} files)", fmt_num(n)));
                }
                _ => {}
            }
        });
        let weak = Rc::downgrade(self);
        let own = nwg::full_bind_event_handler(&self.replace.window.handle, move |evt, data, handle| {
            let Some(app) = weak.upgrade() else { return };
            let r = &app.replace;
            match evt {
                nwg::Event::OnWindowClose if handle == r.window => {
                    if let nwg::EventData::OnWindowClose(d) = &data {
                        d.close(false);
                    }
                    r.window.set_visible(false);
                }
                nwg::Event::OnResize if handle == r.window => r.layout(),
                nwg::Event::OnKeyEnter if !r.state.borrow().busy => app.replace_preview(),
                nwg::Event::OnNotice if handle == r.notice => app.replace_done(),
                nwg::Event::OnListViewItemChanged if handle == r.list => r.show_detail(),
                nwg::Event::OnButtonClick if handle == r.preview_btn => app.replace_preview(),
                nwg::Event::OnButtonClick if handle == r.apply_btn => app.replace_apply(),
                nwg::Event::OnButtonClick if handle == r.undo_btn => app.replace_undo(),
                // Changing what to find or how makes the preview stale.
                nwg::Event::OnTextInput | nwg::Event::OnButtonClick
                    if (handle == r.find || handle == r.repl || handle == r.ignore_case || handle == r.regex)
                        && !r.state.borrow().busy
                        && r.state.borrow().previewed.is_some() =>
                {
                    r.clear_preview();
                    r.set_status(tr!("تغيّرت الخيارات؛ اضغط «معاينة» مرة أخرى.", "The options changed; press Preview again."));
                }
                _ => {}
            }
        });
        (main, own)
    }

    /// Opens the window on the plain-text files of the current results.
    fn open_replace(&self) {
        let r = &self.replace;
        if !r.state.borrow().busy {
            let (files, others) = {
                let st = self.state.borrow();
                let mut files = Vec::new();
                let mut others = 0;
                for e in st.rows.iter().filter_map(|row| st.rows_index.entries.get(row.entry as usize)) {
                    if e.is_dir {
                        continue;
                    }
                    let ext = std::path::Path::new(&*e.path).extension().and_then(|x| x.to_str()).map(str::to_ascii_lowercase);
                    if ext.is_some_and(|x| crate::extract::is_plain_text(&x)) && files.len() < MAX_FILES {
                        files.push(e.path.to_string());
                    } else {
                        others += 1;
                    }
                }
                (files, others)
            };
            r.clear_preview();
            let n = files.len();
            r.state.borrow_mut().files = files;
            let mut msg = if n == 0 {
                tr!("لا توجد ملفات نصية عادية في النتائج الحالية. ابحث أولاً (مثلاً: ‎.txt).", "No plain text files in the current results. Search first (e.g. .txt).").to_string()
            } else {
                trf!("سيُبحث في {} ملفاً نصياً من النتائج الحالية.", "{} plain text files from the current results will be searched.", fmt_num(n))
            };
            if others > 0 {
                msg.push_str(&trf!(" ({} نتيجة أخرى ليست ملفات نصية عادية وستُترك.)", " ({} other results are not plain text files and are left alone.)", fmt_num(others)));
            }
            r.set_status(&msg);
        }
        r.window.set_visible(true);
        r.find.set_focus();
    }

    fn spawn_replace_job(&self, job: impl FnOnce(&AtomicUsize) -> Done + Send + 'static) {
        let r = &self.replace;
        r.progress.store(0, Ordering::Relaxed);
        r.set_busy(true);
        r.set_status(tr!("جارٍ العمل…", "Working…"));
        let done = r.done.clone();
        let progress = r.progress.clone();
        let sender = r.notice.sender();
        std::thread::spawn(move || {
            let result = job(&progress);
            *done.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
            sender.notice();
        });
    }

    fn replace_preview(&self) {
        let r = &self.replace;
        let opts = r.options();
        let matcher = match Matcher::new(&opts) {
            Ok(m) => m,
            Err(e) => return r.set_status(&e),
        };
        let files = r.state.borrow().files.clone();
        if files.is_empty() {
            return r.set_status(tr!("لا توجد ملفات نصية عادية في النتائج الحالية.", "No plain text files in the current results."));
        }
        r.clear_preview();
        self.spawn_replace_job(move |progress| {
            let out = files
                .into_iter()
                .enumerate()
                .map(|(i, p)| {
                    progress.store(i + 1, Ordering::Relaxed);
                    let o = replace::preview_file(std::path::Path::new(&p), &matcher);
                    (p, o)
                })
                .collect();
            Done::Preview(opts, out)
        });
    }

    fn replace_apply(&self) {
        let r = &self.replace;
        let Some(opts) = r.state.borrow().previewed.clone() else { return };
        if opts != r.options() {
            r.clear_preview();
            return r.set_status(tr!("تغيّرت الخيارات؛ اضغط «معاينة» مرة أخرى.", "The options changed; press Preview again."));
        }
        let (paths, total): (Vec<String>, usize) = {
            let st = r.state.borrow();
            let chosen: Vec<&PreviewRow> = st.rows.iter().enumerate().filter(|(i, row)| row.can && r.is_checked(*i)).map(|(_, row)| row).collect();
            (chosen.iter().map(|row| row.path.clone()).collect(), chosen.iter().map(|row| row.count).sum())
        };
        if paths.is_empty() {
            return r.set_status(tr!("لم يُحدَّد أي ملف. ضع علامة على الملفات المراد تعديلها.", "No file is checked. Check the files to change."));
        }
        let params = nwg::MessageParams {
            title: tr!("استبدال نص في الملفات", "Replace text in files"),
            content: &trf!(
                "استبدال «{}» بـ «{}» في {} ملفاً ({} تطابقاً)؟\n\nتُحفظ نسخة احتياطية من كل ملف أولاً، ويمكنك التراجع عن آخر استبدال.",
                "Replace “{}” with “{}” in {} files ({} matches)?\n\nA backup copy of each file is saved first, and you can undo the last replace.",
                opts.find,
                opts.replace,
                fmt_num(paths.len()),
                fmt_num(total)
            ),
            buttons: nwg::MessageButtons::YesNo,
            icons: nwg::MessageIcons::Warning,
        };
        if nwg::modal_message(&r.window, &params) != nwg::MessageChoice::Yes {
            return;
        }
        let matcher = match Matcher::new(&opts) {
            Ok(m) => m,
            Err(e) => return r.set_status(&e),
        };
        self.spawn_replace_job(move |progress| {
            Done::Applied(replace::apply(&paths, &matcher, &opts, &backups_root(), &|n| progress.store(n, Ordering::Relaxed)))
        });
    }

    fn replace_undo(&self) {
        let r = &self.replace;
        let Some((dir, index)) = replace::last_operation(&backups_root()) else {
            return r.set_status(tr!("لا توجد عملية استبدال يمكن التراجع عنها.", "There is no replace to undo."));
        };
        let when = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let params = nwg::MessageParams {
            title: tr!("التراجع عن آخر استبدال", "Undo last replace"),
            content: &trf!(
                "إرجاع {} ملفاً إلى ما كانت عليه قبل استبدال «{}» بـ «{}» ({})؟\n\nالملفات التي عُدّلت بعد الاستبدال لن تُمسّ.",
                "Restore {} files to how they were before “{}” was replaced with “{}” ({})?\n\nFiles edited since the replace are left as they are.",
                fmt_num(index.files.len()),
                index.find,
                index.replace,
                when
            ),
            buttons: nwg::MessageButtons::YesNo,
            icons: nwg::MessageIcons::Question,
        };
        if nwg::modal_message(&r.window, &params) != nwg::MessageChoice::Yes {
            return;
        }
        r.clear_preview();
        self.spawn_replace_job(move |_| Done::Undone(replace::undo(&dir)));
    }

    /// A background job finished: show what it did.
    fn replace_done(&self) {
        let r = &self.replace;
        let Some(done) = r.done.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
        r.state.borrow_mut().busy = false;
        match done {
            Done::Preview(opts, outcomes) => {
                let mut rows = Vec::new();
                let mut skipped = Vec::new();
                let mut none = 0;
                for (path, o) in outcomes {
                    match o {
                        Outcome::Matches(p) => rows.push(PreviewRow {
                            path,
                            can: true,
                            count: p.count,
                            samples: p.samples,
                            note: p.encoding.label().to_string(),
                        }),
                        Outcome::NoMatch => none += 1,
                        Outcome::Skipped(s) => skipped.push(PreviewRow { path, can: false, count: 0, samples: Vec::new(), note: s.reason() }),
                    }
                }
                let files = rows.len();
                let total: usize = rows.iter().map(|x| x.count).sum();
                let skipped_n = skipped.len();
                rows.extend(skipped);
                for (i, row) in rows.iter().enumerate() {
                    let count = if row.can { fmt_num(row.count) } else { "—".into() };
                    r.list.insert_items_row(None, &[ltr_path(&row.path), count, row.note.clone()]);
                    r.set_checked(i, row.can);
                }
                let mut msg = trf!(
                    "{} تطابقاً في {} ملفاً. ملفات بلا تطابق: {}.",
                    "{} matches in {} files. Files without a match: {}.",
                    fmt_num(total),
                    fmt_num(files),
                    fmt_num(none)
                );
                if skipped_n > 0 {
                    msg.push_str(&trf!(" تُركت {} ملفات (السبب في القائمة).", " {} files skipped (reason in the list).", fmt_num(skipped_n)));
                }
                r.set_status(&msg);
                let mut st = r.state.borrow_mut();
                st.previewed = (files > 0).then_some(opts);
                st.rows = rows;
            }
            Done::Applied(Err(e)) => r.set_status(&e),
            Done::Applied(Ok(rep)) => {
                r.clear_preview();
                let total: usize = rep.done.iter().map(|(_, n)| n).sum();
                let mut msg = trf!(
                    "تم: {} استبدالاً في {} ملفاً.",
                    "Done: {} replacements in {} files.",
                    fmt_num(total),
                    fmt_num(rep.done.len())
                );
                if !rep.skipped.is_empty() {
                    msg.push_str(&trf!(" لم يُعدَّل {} ملفاً.", " {} files were not changed.", fmt_num(rep.skipped.len())));
                }
                r.set_status(&msg);
                let mut t = String::new();
                if let Some(dir) = &rep.dir {
                    t.push_str(&trf!("النسخ الاحتياطية في: {}", "Backups in: {}", ltr_path(&dir.to_string_lossy())));
                }
                for (p, s) in &rep.skipped {
                    t.push_str(&format!("\r\n{} — {}", ltr_path(p), s.reason()));
                }
                r.detail.set_text(&t);
                for (p, _) in &rep.done {
                    r.list.insert_items_row(None, &[ltr_path(p), String::new(), tr!("تم", "Done").to_string()]);
                }
            }
            Done::Undone(Err(e)) => r.set_status(&e),
            Done::Undone(Ok(rep)) => {
                let mut msg = trf!("أُرجع {} ملفاً.", "{} files restored.", fmt_num(rep.restored.len()));
                if !rep.conflicts.is_empty() {
                    msg.push_str(&trf!(" {} ملفاً تغيّر بعد الاستبدال فتُرك كما هو.", " {} files changed after the replace and were left as they are.", fmt_num(rep.conflicts.len())));
                }
                if !rep.failed.is_empty() {
                    msg.push_str(&trf!(" تعذر إرجاع {} ملفاً.", " {} files could not be restored.", fmt_num(rep.failed.len())));
                }
                r.set_status(&msg);
                let mut t = String::new();
                for p in &rep.conflicts {
                    t.push_str(&trf!("{} — تغيّر بعد الاستبدال\r\n", "{} — changed after the replace\r\n", ltr_path(p)));
                }
                for (p, e) in &rep.failed {
                    t.push_str(&format!("{} — {e}\r\n", ltr_path(p)));
                }
                r.detail.set_text(&t);
            }
        }
        r.set_busy(false);
    }
}
