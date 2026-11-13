//! Options ▸ «الإبلاغ عن مشكلة…» / "Report a problem…" (also in the tray
//! menu and the new look's Settings ▸ More): a small window with a box for
//! what happened, a read-only preview of exactly what would be shared
//! (`crate::report` builds and filters it), and three buttons: copy it,
//! open a mail to the support address in the user's own mail program (the
//! full report on the clipboard, to paste), or open the bug form on GitHub
//! with it filled in (a public page, which the window says). The program
//! sends nothing by itself and makes no network call: the mail program or
//! the browser opens, and the user presses "Send" or "Submit" there.

use native_windows_gui as nwg;
use std::cell::RefCell;
use std::rc::Rc;

use winapi::um::winuser::{
    ES_CENTER, ES_RIGHT, GWL_EXSTYLE, GWL_STYLE, GetWindowLongW, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowLongW, SetWindowPos, WS_EX_LAYOUTRTL as EX_LAYOUTRTL,
    WS_EX_LEFTSCROLLBAR, WS_EX_RIGHT, WS_EX_RTLREADING,
};

use super::lazy::Lazy;
use super::startup::log;
use super::{App, WS_EX_LAYOUTRTL, shell};
use crate::feedback::{self, SystemInfo};
use crate::report::{self, Facts, Report};

/// The window and its menu items: one field of `App`.
#[derive(Default)]
pub struct ReportWindow {
    /// Options ▸ «الإبلاغ عن مشكلة…», and the same in the tray menu.
    menu_item: nwg::MenuItem,
    tray_item: nwg::MenuItem,
    /// Created the first time the window opens.
    ui: Lazy<ReportUi>,
    /// What was gathered when the window opened: the facts and `log.txt`
    /// (the preview is rebuilt from them as the user types).
    gathered: RefCell<Option<(Facts, String)>>,
    /// The report the preview shows now.
    current: RefCell<Option<Report>>,
}

impl std::ops::Deref for ReportWindow {
    type Target = ReportUi;
    fn deref(&self) -> &ReportUi {
        &self.ui
    }
}

#[derive(Default)]
pub struct ReportUi {
    window: nwg::Window,
    privacy: nwg::Label,
    what_label: nwg::Label,
    what: nwg::TextBox,
    preview_label: nwg::Label,
    /// Holds the preview: its children are not mirrored in the Arabic
    /// window (WS_EX_NOINHERITLAYOUT), so the English report reads left to
    /// right, aligned left.
    preview_frame: nwg::Frame,
    preview: nwg::TextBox,
    copy_btn: nwg::Button,
    /// «إرسال بالبريد»: the user's mail program (`feedback::report_mailto`).
    mail_btn: nwg::Button,
    open_btn: nwg::Button,
    note: nwg::Label,
}

fn menu_label() -> &'static str {
    tr!("الإبلاغ عن مشكلة…", "Report a problem…")
}

/// Line breaks as an edit control and the clipboard want them.
fn crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// The size of the files directly in `dir`.
fn folder_bytes(dir: &std::path::Path) -> Option<u64> {
    let mut total = 0;
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        if let Ok(m) = e.metadata()
            && m.is_file()
        {
            total += m.len();
        }
    }
    Some(total)
}

impl App {
    /// Options ▸ «الإبلاغ عن مشكلة…», in the help and support group.
    pub(super) fn build_report_item(&mut self) -> Result<(), nwg::NwgError> {
        nwg::MenuItem::builder().text(menu_label()).parent(&self.options_menu).build(&mut self.report.menu_item)
    }

    /// The tray menu's item, under "About".
    pub(super) fn build_report_tray_item(&mut self) -> Result<(), nwg::NwgError> {
        nwg::MenuItem::builder().text(menu_label()).parent(&self.tray_menu).build(&mut self.report.tray_item)
    }

    /// Binds the menu items. Called once from `build_ui`.
    pub(super) fn setup_report(self: &Rc<Self>) -> nwg::EventHandler {
        let weak = Rc::downgrade(self);
        self.bind_main_events(move |evt, _data, handle| {
            let Some(app) = weak.upgrade() else { return };
            if evt == nwg::Event::OnMenuItemSelected && (handle == app.report.menu_item || handle == app.report.tray_item) {
                app.show_report();
            }
        })
    }

    fn ensure_report(self: &Rc<Self>) -> bool {
        self.report.ui.is_built() || self.build_lazily("report", |app| {
            app.build_report_window()?;
            Ok((vec![app.bind_report_window()], Vec::new()))
        })
    }

    fn build_report_window(&self) -> Result<(), nwg::NwgError> {
        let mut ui = ReportUi::default();
        let f = Some(&self.font);
        let r = &mut ui;
        nwg::Window::builder()
            .title(tr!("الإبلاغ عن مشكلة", "Report a problem"))
            .size((640, 600))
            .center(true)
            .ex_flags(crate::i18n::rtl_only(WS_EX_LAYOUTRTL))
            .flags(nwg::WindowFlags::WINDOW)
            .icon(None)
            .parent(Some(&self.window))
            .build(&mut r.window)?;
        let p = &r.window;
        nwg::Label::builder()
            .parent(p)
            .font(f)
            .text(tr!(
                "لن يُرسل شيء إلا إذا ضغطت أنت. أسماء الملفات والمسارات تُحذف من التقرير. صفحة GitHub عامة يراها أي أحد؛ أما البريد فلا يقرؤه غيرنا.",
                "Nothing is sent unless you press a button. File names and paths are removed from the report. The GitHub page is public: anyone can read it. An email is read only by us."
            ))
            .position((12, 8))
            .size((616, 66))
            .build(&mut r.privacy)?;
        nwg::Label::builder()
            .parent(p)
            .font(f)
            .text(tr!("ماذا حدث؟ (اختياري، باختصار)", "What happened? (optional, briefly)"))
            .position((12, 78))
            .size((616, 24))
            .build(&mut r.what_label)?;
        nwg::TextBox::builder()
            .parent(p)
            .font(f)
            .limit(report::MAX_WHAT)
            .flags(nwg::TextBoxFlags::VISIBLE | nwg::TextBoxFlags::VSCROLL | nwg::TextBoxFlags::AUTOVSCROLL | nwg::TextBoxFlags::TAB_STOP)
            .position((12, 104))
            .size((616, 64))
            .build(&mut r.what)?;
        nwg::Label::builder()
            .parent(p)
            .font(f)
            .text(tr!("هذا بالضبط ما سيُشارَك:", "This is exactly what will be shared:"))
            .position((12, 174))
            .size((616, 24))
            .build(&mut r.preview_label)?;
        // The report is technical English and reads left to right, aligned
        // left, also in the Arabic window. Taking the right-to-left styles
        // off the box after it was made was not enough (an edit box keeps
        // the layout it was created with: on a real PC the text still sat
        // on the right), so the box is created inside a frame whose
        // children are not mirrored, with no right-to-left style at all.
        const WS_EX_NOINHERITLAYOUT: u32 = 0x0010_0000;
        nwg::Frame::builder()
            .parent(p)
            .flags(nwg::FrameFlags::VISIBLE)
            .ex_flags(crate::i18n::rtl_only(WS_EX_NOINHERITLAYOUT))
            .position((12, 200))
            .size((616, 282))
            .build(&mut r.preview_frame)?;
        nwg::TextBox::builder()
            .parent(&r.preview_frame)
            .font(f)
            .readonly(true)
            .flags(nwg::TextBoxFlags::VISIBLE | nwg::TextBoxFlags::VSCROLL | nwg::TextBoxFlags::AUTOVSCROLL | nwg::TextBoxFlags::TAB_STOP)
            .ex_flags(0)
            .position((0, 0))
            .size((616, 282))
            .build(&mut r.preview)?;
        if let Some(h) = r.preview.handle.hwnd() {
            unsafe {
                let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
                let rtl = EX_LAYOUTRTL | WS_EX_RTLREADING | WS_EX_RIGHT | WS_EX_LEFTSCROLLBAR;
                SetWindowLongW(h, GWL_EXSTYLE, (ex & !rtl) as i32);
                let style = GetWindowLongW(h, GWL_STYLE) as u32;
                SetWindowLongW(h, GWL_STYLE, (style & !(ES_RIGHT | ES_CENTER)) as i32);
                SetWindowPos(h, std::ptr::null_mut(), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED);
            }
        }
        nwg::Button::builder().parent(p).font(f).text(tr!("نسخ", "Copy")).position((12, 494)).size((120, 34)).build(&mut r.copy_btn)?;
        nwg::Button::builder()
            .parent(p)
            .font(f)
            .text(tr!("إرسال بالبريد", "Send by email"))
            .position((142, 494))
            .size((200, 34))
            .build(&mut r.mail_btn)?;
        nwg::Button::builder()
            .parent(p)
            .font(f)
            .text(tr!("صفحة GitHub (عامة)", "GitHub page (public)"))
            .position((352, 494))
            .size((240, 34))
            .build(&mut r.open_btn)?;
        nwg::Label::builder().parent(p).font(f).text("").position((12, 536)).size((616, 52)).build(&mut r.note)?;
        self.report.ui.set(ui);
        Ok(())
    }

    fn bind_report_window(self: &Rc<Self>) -> nwg::EventHandler {
        let weak = Rc::downgrade(self);
        nwg::full_bind_event_handler(&self.report.window.handle, crate::crash_log::guard_events(move |evt, data, handle| {
            let Some(app) = weak.upgrade() else { return };
            let r = &app.report;
            match evt {
                nwg::Event::OnWindowClose if handle == r.window => {
                    if let nwg::EventData::OnWindowClose(c) = &data {
                        c.close(false);
                    }
                    r.window.set_visible(false);
                }
                nwg::Event::OnKeyEsc => r.window.set_visible(false),
                nwg::Event::OnTextInput if handle == r.what => app.render_report(),
                nwg::Event::OnButtonClick if handle == r.copy_btn => app.report_copy(),
                nwg::Event::OnButtonClick if handle == r.mail_btn => app.report_mail(),
                nwg::Event::OnButtonClick if handle == r.open_btn => app.report_open(),
                _ => {}
            }
        }))
    }

    /// Opens the window, with the facts of this moment.
    pub(super) fn show_report(self: &Rc<Self>) {
        if !self.ensure_report() {
            return;
        }
        let log_file = crate::index::cache_path().with_file_name("log.txt");
        let log = std::fs::read(log_file).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        self.report.gathered.replace(Some((self.report_facts(), log)));
        self.report.note.set_text("");
        self.render_report();
        let r = &self.report;
        r.window.set_visible(true);
        r.what.set_focus();
    }

    /// What the report states about this copy now (no paths).
    fn report_facts(&self) -> Facts {
        let info = SystemInfo::current();
        let st = self.state.borrow();
        let (private, peak) = super::memory::private_and_peak().unzip();
        let data = crate::index::cache_path();
        let (phone_on, phone_running) = self.web_switch_state();
        Facts {
            version: info.version,
            os: info.os,
            arch: info.arch,
            lang: info.lang,
            build: crate::update::managed_by().map_or("portable", |pm| pm.name()).to_string(),
            new_look: self.webui_in_use(),
            new_look_chosen: st.settings.new_ui,
            entries: st.index.entries.len(),
            documents: super::content_index::opened().map(|c| c.documents_now()),
            index_bytes: std::fs::metadata(&data).ok().map(|m| m.len()),
            content_bytes: folder_bytes(&data.with_file_name("content")),
            private_bytes: private,
            peak_bytes: peak,
            uptime_secs: super::startup::ms_since_process_start() / 1000,
            auto_index: st.settings.auto_index,
            content_index: st.settings.content_index,
            phone_on,
            phone_running,
        }
    }

    /// Builds the report from what was gathered and the box's text, and
    /// shows it.
    fn render_report(&self) {
        let r = &self.report;
        let Some(built) = r.gathered.borrow().as_ref().map(|(facts, log)| {
            let names = report::local_names();
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            report::build(facts, &r.what.text(), log, &names)
        }) else {
            return;
        };
        r.preview.set_text(&crlf(&built.text()));
        r.current.replace(Some(built));
    }

    fn report_copy(&self) {
        let r = &self.report;
        let Some(text) = r.current.borrow().as_ref().map(Report::text) else { return };
        if shell::copy_text(&r.window, &crlf(&text)) {
            r.note.set_text(&trf!(
                "نُسخ التقرير. الصقه في رسالة إلى {} أو في صفحة الإبلاغ.",
                "The report is copied. Paste it in a message to {} or on the report page.",
                feedback::SUPPORT_EMAIL
            ));
        } else {
            r.note.set_text(shell::copy_failed_text());
        }
    }

    /// A new mail to the support address in the user's mail program, with
    /// the user's words in short; the whole report goes on the clipboard
    /// first (a `mailto:` link holds only a little), to paste in the mail.
    /// Nothing is sent: the user presses "Send" in the mail program.
    fn report_mail(&self) {
        let r = &self.report;
        let Some(built) = r.current.borrow().clone() else { return };
        let english = crate::i18n::current() == crate::i18n::Lang::En;
        let url = feedback::report_mailto(&SystemInfo::current(), &built.what, english);
        let copied = shell::copy_text(&r.window, &crlf(&built.text()));
        if !shell::open(&url) {
            log("report: could not open the mail program");
            r.note.set_text(&if copied {
                trf!(
                    "لا يوجد برنامج بريد يفتح هنا. نُسخ التقرير: الصقه في رسالة إلى {} من بريدك (من الهاتف أو المتصفح).",
                    "No mail program opened. The report is copied: paste it in a message to {} from your own mail (on your phone or in the browser).",
                    feedback::SUPPORT_EMAIL
                )
            } else {
                trf!("لا يوجد برنامج بريد يفتح هنا. اضغط «نسخ» وأرسل التقرير إلى {}.", "No mail program opened. Press “Copy” and send the report to {}.", feedback::SUPPORT_EMAIL)
            });
            return;
        }
        r.note.set_text(&if copied {
            trf!(
                "فُتحت رسالة إلى {} في برنامج البريد. التقرير الكامل منسوخ: الصقه في الرسالة (Ctrl+V) ثم اضغط «إرسال». لم يُرسل شيء بعد.",
                "A message to {} opened in your mail program. The full report is copied: paste it in the message (Ctrl+V), then press “Send”. Nothing was sent yet.",
                feedback::SUPPORT_EMAIL
            )
        } else {
            trf!(
                "فُتحت رسالة إلى {} في برنامج البريد (تعذر نسخ التقرير: اضغط «نسخ» ثم الصقه فيها). لم يُرسل شيء بعد.",
                "A message to {} opened in your mail program (the report could not be copied: press “Copy”, then paste it there). Nothing was sent yet.",
                feedback::SUPPORT_EMAIL
            )
        });
    }

    /// The bug form on GitHub with the report filled in; when it does not
    /// fit in the address, the whole report goes on the clipboard too.
    fn report_open(&self) {
        let r = &self.report;
        let Some(built) = r.current.borrow().clone() else { return };
        let (url, cut) = feedback::report_issue_url(&SystemInfo::current(), &built.what, &built.details);
        let copied = cut && shell::copy_text(&r.window, &crlf(&built.text()));
        if !shell::open(&url) {
            log("report: could not open the browser");
            let copied = copied || shell::copy_text(&r.window, &crlf(&built.text()));
            r.note.set_text(&if copied {
                trf!(
                    "تعذر فتح المتصفح. نُسخ التقرير: الصقه في رسالة إلى {}.",
                    "Could not open the browser. The report is copied: paste it in a message to {}.",
                    feedback::SUPPORT_EMAIL
                )
            } else {
                trf!("تعذر فتح المتصفح. اضغط «نسخ» وأرسل التقرير إلى {}.", "Could not open the browser. Press “Copy” and send the report to {}.", feedback::SUPPORT_EMAIL)
            });
            return;
        }
        r.note.set_text(if copied {
            tr!(
                "فُتحت صفحة GitHub. التقرير أطول مما تتسع له الصفحة، فنُسخ كاملاً: الصقه في خانة التقرير، ثم راجع الصفحة واضغط «Submit».",
                "GitHub opened. The report is longer than the page can take, so all of it is copied: paste it in the report box, then review the page and press “Submit”."
            )
        } else if cut {
            tr!(
                "فُتحت صفحة GitHub وفيها بداية التقرير (تعذر نسخ الباقي). راجعها ثم اضغط «Submit».",
                "GitHub opened with the start of the report (the rest could not be copied). Review it, then press “Submit”."
            )
        } else {
            tr!("فُتحت صفحة GitHub وفيها التقرير. راجعها ثم اضغط «Submit» لإرسالها. شكراً لك!", "GitHub opened with the report. Review it, then press “Submit” to send it. Thank you!")
        });
    }
}
