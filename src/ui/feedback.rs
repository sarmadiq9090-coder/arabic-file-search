//! Options ▸ "Send feedback", and the offer at start-up to report a crash of
//! the last run. Both only open the browser on a prefilled GitHub page: the
//! program itself sends nothing (see `crate::feedback`).

use native_windows_gui as nwg;

use super::startup::log;
use super::{App, shell, title};
use crate::feedback::{self, Kind, SystemInfo};

/// Adds the submenu to the Options menu.
pub(super) fn build(options: &nwg::Menu, menu: &mut nwg::Menu, items: [&mut nwg::MenuItem; 3]) -> Result<(), nwg::NwgError> {
    nwg::Menu::builder().text(tr!("أرسل ملاحظة", "Send feedback")).parent(options).build(menu)?;
    let [bug, idea, talk] = items;
    nwg::MenuItem::builder().text(tr!("أبلغ عن مشكلة…", "Report a problem…")).parent(&*menu).build(bug)?;
    nwg::MenuItem::builder().text(tr!("اقترح ميزة جديدة…", "Suggest a feature…")).parent(&*menu).build(idea)?;
    nwg::MenuItem::builder().text(tr!("اسأل أو ناقش مع المستخدمين…", "Ask or discuss with other users…")).parent(&*menu).build(talk)?;
    Ok(())
}

/// Adds the «الدعم» / "Support" submenu to the Options menu: write to the
/// support address, or open the support page.
pub(super) fn build_support(options: &nwg::Menu, menu: &mut nwg::Menu, items: [&mut nwg::MenuItem; 2]) -> Result<(), nwg::NwgError> {
    nwg::Menu::builder().text(tr!("الدعم", "Support")).parent(options).build(menu)?;
    let [mail, page] = items;
    let mail_text = trf!("راسل الدعم: {}", "Email support: {}", feedback::SUPPORT_EMAIL);
    nwg::MenuItem::builder().text(&mail_text).parent(&*menu).build(mail)?;
    nwg::MenuItem::builder().text(tr!("صفحة الدعم (fastkiwi.app)", "Support page (fastkiwi.app)")).parent(&*menu).build(page)?;
    Ok(())
}

impl App {
    /// Handles a click on one of the feedback items; false for any other item.
    /// "Report a problem…" opens the report window (`report.rs`), which
    /// shows what would be shared before the form opens.
    pub(super) fn on_feedback_item(self: &std::rc::Rc<Self>, handle: nwg::ControlHandle) -> bool {
        let url = if handle == self.menu_feedback_bug {
            self.show_report();
            return true;
        } else if handle == self.menu_feedback_idea {
            feedback::issue_url(Kind::Idea, &SystemInfo::current(), None)
        } else if handle == self.menu_feedback_talk {
            feedback::discussions_url()
        } else if handle == self.menu_support_mail || handle == self.menu_support_page {
            self.open_support(handle == self.menu_support_mail);
            return true;
        } else {
            return false;
        };
        self.open_feedback_page(&url);
        true
    }

    /// Options ▸ Support: the mail program on the support address, or the
    /// support page; both shown in a message when Windows can't open them.
    fn open_support(&self, mail: bool) {
        let page = feedback::support_page(crate::i18n::is_en());
        let target = if mail { feedback::support_mailto() } else { page.to_string() };
        if !shell::open(&target) {
            log("support: could not open the mail program or the browser");
            let owner = self.with_modal_owner(|h| h);
            nwg::modal_info_message(
                nwg::ControlHandle::Hwnd(owner),
                title(),
                &trf!(
                    "تعذر فتح البريد أو المتصفح. راسلنا على:\n{}\nأو افتح صفحة الدعم:\n{}",
                    "Could not open the mail program or the browser. Write to:\n{}\nor open the support page:\n{}",
                    feedback::SUPPORT_EMAIL,
                    page
                ),
            );
        }
    }

    pub(super) fn open_feedback_page(&self, url: &str) {
        if shell::open(url) {
            self.set_status(tr!(
                "فُتحت صفحة GitHub في المتصفح. راجعها ثم اضغط «Submit» لإرسالها. شكراً لك!",
                "GitHub opened in your browser. Review the page, then press “Submit” to send it. Thank you!"
            ));
        } else {
            log("feedback: could not open the browser");
            let text = trf!(
                "تعذر فتح المتصفح. يمكنك إرسال ملاحظتك من هذه الصفحة:\n{}",
                "Could not open the browser. You can send your feedback on this page:\n{}",
                format!("{}/issues", feedback::REPO_URL)
            );
            self.with_modal_owner(|owner| nwg::modal_info_message(nwg::ControlHandle::Hwnd(owner), title(), &text));
        }
    }

    /// Asks once whether to report the crash reports this version wrote
    /// since the last time it asked. Nothing is sent without "Yes", and even
    /// then only a browser page opens for the user to review and submit.
    pub(super) fn offer_crash_report(&self) {
        let seen = self.state.borrow().settings.crash_seen;
        let file = match std::fs::read(crate::crash_log::crash_path()) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(_) => return,
        };
        let reports = feedback::unseen(feedback::crash_reports(&file, env!("CARGO_PKG_VERSION")), seen);
        let Some(newest) = reports.last().map(|r| r.time) else { return };
        // Asked once per report, whatever the answer.
        self.state.borrow_mut().settings.crash_seen = newest;
        self.save_settings();
        log(&format!("feedback: offering {} crash report(s)", reports.len()));
        let text = tr!(
            "حدث خطأ داخلي في بحث سريع في المرة السابقة، وحُفظ تقرير عنه على جهازك فقط.\n\n\
             هل تريد إرساله للمطوّر لإصلاحه؟ ستُفتح صفحة GitHub فيها التقرير (رقم الإصدار، نظام ويندوز، ومكان الخطأ في البرنامج) لتراجعها قبل الضغط على «Submit». لا يُرسَل شيء تلقائياً.",
            "InstaFind ran into an internal error last time. A report about it was saved on your computer only.\n\n\
             Send it to the developer so it can be fixed? A GitHub page opens with the report (version, Windows version and where in the program it happened) for you to review before pressing “Submit”. Nothing is sent automatically."
        );
        let params = nwg::MessageParams {
            title: title(),
            content: text,
            buttons: nwg::MessageButtons::YesNo,
            icons: nwg::MessageIcons::Question,
        };
        if self.with_modal_owner(|owner| nwg::modal_message(nwg::ControlHandle::Hwnd(owner), &params)) != nwg::MessageChoice::Yes {
            return;
        }
        let crash = feedback::crash_text(&reports);
        self.open_feedback_page(&feedback::issue_url(Kind::Bug, &SystemInfo::current(), Some(&crash)));
    }
}
