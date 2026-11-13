//! Creating the window and its controls, and dispatching their events.

use native_windows_gui as nwg;
use nwg::NativeUi;
use std::cell::RefCell;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use winapi::shared::minwindef::{LPARAM, UINT, WPARAM};
use winapi::shared::windef::HWND;
use winapi::um::commctrl::{
    LVIF_IMAGE, LVIF_TEXT, LVM_SETIMAGELIST, LVN_GETDISPINFOW, LVS_EX_DOUBLEBUFFER, LVS_OWNERDATA,
    LVS_SHAREIMAGELISTS, LVSIL_SMALL, NMLVDISPINFOW,
};
use winapi::um::winuser::{NMHDR, SendMessageW, WM_NOTIFY, WM_SETFONT};

use super::format::{ARABIC_DIGITS, wide_to_string};
use super::keys::{HOTKEY_ID, HOTKEY_TEXT};
use super::{autostart, placement, shell, style};
use super::startup::{phase, ui_font_family, work_area};
use super::{App, COLUMNS, WS_EX_LAYOUTRTL, title};
use crate::history::History;
use crate::search;
use crate::settings::Settings;

pub struct AppUi {
    inner: Rc<App>,
    handlers: RefCell<Vec<nwg::EventHandler>>,
    raw_handlers: RefCell<Vec<nwg::RawEventHandler>>,
}

impl Deref for AppUi {
    type Target = App;
    fn deref(&self) -> &App {
        &self.inner
    }
}

impl Drop for AppUi {
    fn drop(&mut self) {
        for h in self.handlers.borrow().iter() {
            nwg::unbind_event_handler(h);
        }
        for h in self.raw_handlers.borrow().iter() {
            let _ = nwg::unbind_raw_event_handler(h);
        }
        self.inner.lazy_handlers.unbind();
    }
}

impl NativeUi<AppUi> for App {
    fn build_ui(mut app: App) -> Result<AppUi, nwg::NwgError> {
        let had_file = Settings::exists();
        let mut settings = Settings::load();
        // 0.10.1: the new look is the default, once (see the method); a
        // classic user is told so once in the new look, with the way back.
        let was_on = settings.new_ui;
        let switched = settings.make_new_look_default();
        if switched {
            super::startup::log("settings: the new look is now the default (once)");
            settings.save();
        }
        if Settings::switch_to_tell(had_file, was_on, switched) {
            app.webui_tell_switch();
        }
        // A size saved on a bigger screen must still fit this one.
        let (max_w, max_h) = work_area();
        settings.width = settings.width.min(max_w);
        settings.height = settings.height.min(max_h);
        ARABIC_DIGITS.store(settings.arabic_digits, Ordering::Relaxed);

        let family = ui_font_family();
        phase("settings");
        // Sizes in logical pixels: nwg scales fonts for the DPI itself
        // (scaling here too made the text 2.25× too big at 150 %).
        nwg::Font::builder().family(family).size_absolute(15).build(&mut app.font)?;
        nwg::Font::builder().family(family).size_absolute(19).build(&mut app.query_font)?;
        let f = Some(&app.font);

        // Centred in the work area, not on the whole screen: on a 768-pixel
        // screen the status line and the bottom buttons were under the
        // taskbar. Given at creation, so it is also where a window that
        // opens maximized goes when restored.
        let (size, position) = placement::opening(settings.width, settings.height);
        nwg::Window::builder()
            .title(title())
            .size(size)
            .position(position)
            .ex_flags(crate::i18n::rtl_only(WS_EX_LAYOUTRTL))
            .flags({
                // Started with Windows: wait in the tray until called. The
                // new main window, when used, shows itself instead.
                let visible = if autostart::launched_at_sign_in() || super::webui::wanted(settings.new_ui) {
                    nwg::WindowFlags::empty()
                } else {
                    nwg::WindowFlags::VISIBLE
                };
                if settings.maximized {
                    nwg::WindowFlags::MAIN_WINDOW | visible | nwg::WindowFlags::MAXIMIZED
                } else {
                    nwg::WindowFlags::MAIN_WINDOW | visible
                }
            })
            .build(&mut app.window)?;
        let w = &app.window;
        if let Some(h) = w.handle.hwnd() {
            crate::icon::set_window_icon(h);
            // With the real frame, on the monitor it opened on.
            placement::keep_on_screen(h);
        }
        phase("window");

        nwg::TextInput::builder()
            .parent(w)
            .font(Some(&app.query_font))
            .focus(true)
            .build(&mut app.query)?;
        // The hint, shown even while the box has focus (nwg's placeholder
        // hides it then, and the box has focus from the start, so it was
        // never seen), as the quick search's box does.
        if let Some(h) = app.query.handle.hwnd() {
            let cue = shell::wide(tr!(
                "اكتب اسم الملف أو جزءاً منه…   مثال: تقرير \u{200E}.pdf",
                "Type a file name or part of it…   e.g. report .pdf"
            ));
            unsafe { SendMessageW(h, winapi::um::commctrl::EM_SETCUEBANNER, 1, cue.as_ptr() as LPARAM) };
        }
        nwg::Button::builder().parent(w).font(f).text(tr!("بحث", "Search")).build(&mut app.search_btn)?;
        nwg::Button::builder().parent(w).font(f).text(tr!("السجل", "History")).build(&mut app.history_btn)?;
        let kinds: Vec<&'static str> = search::KINDS.iter().zip(crate::i18n::KIND_LABELS_EN).map(|((label, _), en)| tr!(*label, en)).collect();
        let kind = settings.kind.min(kinds.len().saturating_sub(1));
        nwg::ComboBox::builder()
            .parent(w)
            .font(f)
            .collection(kinds)
            .selected_index(Some(kind))
            .build(&mut app.kind_combo)?;
        nwg::CheckBox::builder()
            .parent(w)
            .font(f)
            .text(tr!("البحث داخل المستندات أيضاً", "Also search inside documents"))
            .build(&mut app.content_chk)?;
        nwg::Label::builder().parent(w).font(f).text(tr!("البحث في", "Look in")).build(&mut app.scope_label)?;
        nwg::Label::builder().parent(w).font(f).text(tr!("كل الأقراص", "All drives")).build(&mut app.scope_text)?;
        style::set_path_ellipsis(&app.scope_text);
        nwg::Button::builder().parent(w).font(f).text(tr!("اختيار مجلد…", "Choose folder…")).build(&mut app.scope_btn)?;
        nwg::Button::builder().parent(w).font(f).text(tr!("كل الأقراص", "All drives")).build(&mut app.all_btn)?;
        nwg::Button::builder().parent(w).font(f).text(tr!("تحديث الفهرس", "Update index")).build(&mut app.reindex_btn)?;
        // Only shown while a folder is chosen (see `show_scope`).
        app.all_btn.set_visible(false);
        style::set_default_button(&app.search_btn);

        // Owner-data (virtual) list: rows are drawn on demand, so even a
        // million results display instantly.
        let list_flags = unsafe {
            nwg::ListViewFlags::from_bits_unchecked(
                nwg::ListViewFlags::VISIBLE.bits()
                    | nwg::ListViewFlags::TAB_STOP.bits()
                    | nwg::ListViewFlags::SINGLE_SELECTION.bits()
                    | nwg::ListViewFlags::ALWAYS_SHOW_SELECTION.bits()
                    | LVS_OWNERDATA
                    | winapi::um::commctrl::LVS_EDITLABELS
                    | LVS_SHAREIMAGELISTS,
            )
        };
        nwg::ListView::builder()
            .parent(w)
            .flags(list_flags)
            .list_style(nwg::ListViewStyle::Detailed)
            // Native double buffering (LVS_EX_DOUBLEBUFFER). nwg's own
            // `double_buffer` paints through an unmirrored memory DC, which
            // draws text backwards in a right-to-left window.
            .ex_flags(unsafe {
                nwg::ListViewExFlags::from_bits_unchecked(
                    nwg::ListViewExFlags::FULL_ROW_SELECT.bits() | LVS_EX_DOUBLEBUFFER,
                )
            })
            .build(&mut app.list)?;
        if let Some(h) = app.list.handle.hwnd() {
            unsafe {
                SendMessageW(h, WM_SETFONT, app.font.handle as WPARAM, 1);
                // Explorer-style rows: soft hover and selection colours.
                let theme = shell::wide("Explorer");
                winapi::um::uxtheme::SetWindowTheme(h, theme.as_ptr(), std::ptr::null());
                // File-type icons from the shared system image list.
                SendMessageW(h, LVM_SETIMAGELIST, LVSIL_SMALL as WPARAM, shell::system_image_list() as LPARAM);
            }
        }
        phase("list");
        // Until the user sets widths of their own, the columns share the
        // list's width (the window's, less the margins, the list's border
        // and its scroll bar): the defaults are wider than the default
        // window, which cut the last header to "Mat…".
        let defaults: Vec<(i32, i32)> = COLUMNS.iter().zip(super::COLUMN_MIN).map(|((_, w), min)| (*w, min)).collect();
        let fitted = crate::window_fit::columns_untouched(&settings.columns, &defaults)
            .then(|| crate::window_fit::fit_columns(&defaults, w.size().0 as i32 - 2 * style::MARGIN - 22));
        for (i, (title, width)) in COLUMNS.iter().enumerate() {
            let width = match &fitted {
                Some(f) => f[i],
                None => settings.columns.get(i).copied().filter(|w| (20..3000).contains(w)).unwrap_or(*width),
            };
            app.list.insert_column(nwg::InsertListViewColumn {
                index: Some(i as i32),
                fmt: None,
                width: Some(width), // nwg scales it for the DPI
                text: Some(tr!(*title, super::COLUMNS_EN[i]).to_string()),
            });
        }
        app.list.set_headers_enabled(true);

        // Apps the query names (hidden until one does).
        for b in app.app_btns.iter_mut() {
            nwg::Button::builder().parent(w).font(f).text("").flags(nwg::ButtonFlags::TAB_STOP).build(b)?;
        }
        nwg::Button::builder().parent(w).font(f).text(tr!("فتح الملف", "Open file")).build(&mut app.open_btn)?;
        nwg::Button::builder().parent(w).font(f).text(tr!("فتح المجلد", "Open folder")).build(&mut app.folder_btn)?;
        nwg::Button::builder().parent(w).font(f).text(tr!("نسخ المسار", "Copy path")).build(&mut app.copy_btn)?;
        nwg::Button::builder().parent(w).font(f).text(tr!("تصدير النتائج", "Export results")).build(&mut app.export_btn)?;
        nwg::Button::builder().parent(w).font(f).text(tr!("إيقاف البحث", "Stop search")).enabled(false).build(&mut app.stop_btn)?;
        nwg::Button::builder().parent(w).font(f).text(tr!("خيارات…", "Options…")).build(&mut app.options_btn)?;
        nwg::StatusBar::builder().parent(w).font(f).text("").build(&mut app.status)?;
        {
            use style::glyph as g;
            for (b, glyph) in [
                (&app.search_btn, g::SEARCH),
                (&app.history_btn, g::HISTORY),
                (&app.scope_btn, g::FOLDER),
                (&app.all_btn, g::DRIVE),
                (&app.reindex_btn, g::REFRESH),
                (&app.open_btn, g::OPEN_FILE),
                (&app.folder_btn, g::OPEN_FOLDER),
                (&app.copy_btn, g::COPY),
                (&app.export_btn, g::EXPORT),
                (&app.stop_btn, g::STOP),
                (&app.options_btn, g::SETTINGS),
            ] {
                style::set_button_icon(b, glyph);
            }
        }

        nwg::AnimationTimer::builder()
            .parent(w)
            .interval(Duration::from_millis(200))
            .active(true)
            .build(&mut app.timer)?;
        nwg::Notice::builder().parent(w).build(&mut app.name_notice)?;
        nwg::Notice::builder().parent(w).build(&mut app.update_notice)?;
        nwg::Notice::builder().parent(w).build(&mut app.watch_notice)?;
        nwg::Notice::builder().parent(w).build(&mut app.matches_notice)?;
        phase("controls");
        nwg::Menu::builder().popup(true).parent(w).build(&mut app.menu)?;
        nwg::MenuItem::builder().text(tr!("فتح الملف\tEnter", "Open file\tEnter")).parent(&app.menu).build(&mut app.menu_open)?;
        nwg::MenuItem::builder().text(tr!("فتح المجلد الذي يحتويه\tCtrl+Enter", "Open containing folder\tCtrl+Enter")).parent(&app.menu).build(&mut app.menu_folder)?;
        nwg::MenuItem::builder().text(tr!("فتح باستخدام…", "Open with…")).parent(&app.menu).build(&mut app.menu_open_with)?;
        nwg::MenuItem::builder().text(tr!("نسخ الملف\tCtrl+C", "Copy file\tCtrl+C")).parent(&app.menu).build(&mut app.menu_copy_file)?;
        nwg::MenuItem::builder().text(tr!("نسخ المسار\tCtrl+Shift+C", "Copy path\tCtrl+Shift+C")).parent(&app.menu).build(&mut app.menu_copy)?;
        nwg::MenuItem::builder().text(tr!("نسخ مسارات كل النتائج", "Copy paths of all results")).parent(&app.menu).build(&mut app.menu_copy_all)?;
        // Here rather than in Options: a narrow window hides the Export button.
        nwg::MenuItem::builder().text(tr!("تصدير النتائج إلى Excel (CSV)", "Export results to Excel (CSV)")).parent(&app.menu).build(&mut app.menu_export)?;
        nwg::MenuItem::builder().text(tr!("تغيير الاسم\tF2", "Rename\tF2")).parent(&app.menu).build(&mut app.menu_rename)?;
        nwg::MenuItem::builder().text(tr!("نقل إلى سلة المحذوفات\tDelete", "Move to Recycle Bin\tDelete")).parent(&app.menu).build(&mut app.menu_delete)?;
        nwg::MenuItem::builder().text(tr!("عرض كل التطابقات داخل الملف\tF4", "Show all matches in the file\tF4")).parent(&app.menu).build(&mut app.menu_matches)?;

        // Window with every matching line of one document (hidden until used).
        nwg::Window::builder()
            .title(tr!("كل التطابقات", "All matches"))
            .size((720, 460))
            .center(true)
            .ex_flags(crate::i18n::rtl_only(WS_EX_LAYOUTRTL))
            .flags(nwg::WindowFlags::WINDOW | nwg::WindowFlags::RESIZABLE)
            .icon(None)
            .parent(Some(&app.window))
            .build(&mut app.matches_window)?;
        nwg::TextBox::builder()
            .parent(&app.matches_window)
            .font(f)
            .readonly(true)
            .flags(nwg::TextBoxFlags::VISIBLE | nwg::TextBoxFlags::VSCROLL | nwg::TextBoxFlags::AUTOVSCROLL)
            .position((8, 8))
            .size((704, 444))
            .build(&mut app.matches_text)?;


        // Tray icon, so a hidden window comes back instantly: a monochrome
        // magnifier in the taskbar's colours (the app icon if that fails).
        app.tray_icon.handle = app.mono_icons.get(super::surface::taskbar_dark()) as _;
        if app.tray_icon.handle.is_null() {
            app.tray_icon.handle = crate::icon::load_icon(true) as _;
        }
        if app.tray_icon.handle.is_null() {
            nwg::Icon::builder().source_system(Some(nwg::OemIcon::Information)).build(&mut app.tray_icon)?;
        }
        nwg::TrayNotification::builder()
            .parent(w)
            .icon(Some(&app.tray_icon))
            .tip(Some(&format!("{} ({HOTKEY_TEXT})", title())))
            .visible(settings.tray_icon)
            .build(&mut app.tray)?;
        nwg::Menu::builder().popup(true).parent(w).build(&mut app.tray_menu)?;
        nwg::MenuItem::builder().text(&trf!("فتح نافذة البحث\t{HOTKEY_TEXT}", "Open quick search\t{HOTKEY_TEXT}")).parent(&app.tray_menu).build(&mut app.tray_quick)?;
        nwg::MenuItem::builder()
            .text(tr!("إظهار خانة البحث العائمة", "Show the floating search box"))
            .check(settings.floating_mode != crate::settings::FloatingMode::Off)
            .parent(&app.tray_menu)
            .build(&mut app.tray_floating)?;
        nwg::MenuItem::builder().text(tr!("النافذة الكاملة", "Full window")).parent(&app.tray_menu).build(&mut app.tray_show)?;
        nwg::MenuItem::builder().text(tr!("خيارات…", "Options…")).parent(&app.tray_menu).build(&mut app.tray_settings)?;
        nwg::MenuItem::builder().text(tr!("حول InstaFind", "About InstaFind")).parent(&app.tray_menu).build(&mut app.tray_about)?;
        app.build_report_tray_item()?;
        app.build_rating_tray_item()?;
        nwg::MenuItem::builder().text(tr!("خروج", "Exit")).parent(&app.tray_menu).build(&mut app.tray_exit)?;

        phase("tray");
        nwg::Menu::builder().popup(true).parent(&app.window).build(&mut app.options_menu)?;
        // The new interface, first so it is seen; not offered without
        // WebView2 (nor in mingw builds, which can't host it).
        if super::webui::available() {
            nwg::MenuItem::builder().text(tr!("جرّب الشكل الجديد", "Try the new look")).parent(&app.options_menu).build(&mut app.menu_new_ui)?;
            nwg::MenuSeparator::builder().parent(&app.options_menu).build(&mut app.menu_new_ui_sep)?;
        }
        // In groups: look and display, indexing and search, shortcuts and
        // start-up, tools, help and support; then Hide and Exit, last.
        // Some builders below also add their item to the result menu.
        nwg::MenuItem::builder()
            .text(tr!("عرض الأرقام بالعربية (٠١٢٣)", "Show Arabic-Indic digits (٠١٢٣)"))
            .check(settings.arabic_digits)
            .parent(&app.options_menu)
            .build(&mut app.menu_digits)?;
        app.build_thumbs()?;
        super::lang_menu::build(&app.options_menu, &mut app.lang_menu, &mut app.menu_lang_ar, &mut app.menu_lang_en)?;
        nwg::MenuSeparator::builder().parent(&app.options_menu).build(&mut app.options_seps[0])?;

        nwg::MenuItem::builder()
            .text(tr!("تحديث الفهرس تلقائياً عند تغيّر الملفات", "Update the index automatically when files change"))
            .check(settings.auto_index)
            .parent(&app.options_menu)
            .build(&mut app.menu_auto_index)?;
        nwg::MenuItem::builder()
            .text(tr!("فهرسة محتوى المستندات في الخلفية (بحث أسرع داخل الملفات)", "Index document contents in the background (faster content search)"))
            .check(settings.content_index)
            .parent(&app.options_menu)
            .build(&mut app.menu_content_index)?;
        nwg::MenuItem::builder()
            .text(tr!("قراءة محتوى الملفات الموجودة على السحابة فقط (يُنزّلها من الإنترنت)", "Read contents of cloud-only files (downloads them)"))
            .check(settings.read_cloud_files)
            .parent(&app.options_menu)
            .build(&mut app.menu_read_cloud)?;
        nwg::MenuItem::builder()
            .text(tr!("إخفاء ملفات النظام", "Hide system files"))
            .check(settings.hide_system)
            .parent(&app.options_menu)
            .build(&mut app.menu_hide_system)?;
        nwg::MenuSeparator::builder().parent(&app.options_menu).build(&mut app.options_seps[1])?;

        nwg::MenuItem::builder()
            .text(&trf!("إظهار البرنامج من أي مكان\t{HOTKEY_TEXT}", "Show the program from anywhere\t{HOTKEY_TEXT}"))
            .parent(&app.options_menu)
            .build(&mut app.menu_hotkey)?;
        app.build_quick_menus(&settings)?;
        app.build_quick_switch_menus()?;
        nwg::MenuItem::builder()
            .text(tr!("التشغيل مع ويندوز (في الخلفية)", "Start with Windows (in the background)"))
            .check(autostart::is_enabled())
            .parent(&app.options_menu)
            .build(&mut app.menu_autostart)?;
        nwg::MenuSeparator::builder().parent(&app.options_menu).build(&mut app.options_seps[2])?;

        app.build_dupes()?;
        app.build_replace_window()?;
        app.build_disk_usage()?;
        app.build_web_ui()?;
        nwg::MenuSeparator::builder().parent(&app.options_menu).build(&mut app.options_seps[3])?;

        nwg::MenuItem::builder()
            .text(&if crate::store::is_packaged() {
                trf!("التحديثات من Microsoft Store (الإصدار {})", "Updates from Microsoft Store (version {})", env!("CARGO_PKG_VERSION"))
            } else {
                trf!("التحقق من التحديثات (الإصدار {})", "Check for updates (version {})", env!("CARGO_PKG_VERSION"))
            })
            .parent(&app.options_menu)
            .build(&mut app.menu_update)?;
        super::feedback::build(&app.options_menu, &mut app.feedback_menu, [&mut app.menu_feedback_bug, &mut app.menu_feedback_idea, &mut app.menu_feedback_talk])?;
        super::feedback::build_support(&app.options_menu, &mut app.support_menu, [&mut app.menu_support_mail, &mut app.menu_support_page])?;
        app.build_report_item()?;
        app.build_rating()?;
        nwg::MenuSeparator::builder().parent(&app.options_menu).build(&mut app.options_seps[4])?;

        nwg::MenuItem::builder().text(tr!("إخفاء بجانب الساعة\tEsc", "Hide next to the clock\tEsc")).parent(&app.options_menu).build(&mut app.menu_hide)?;
        nwg::MenuItem::builder().text(tr!("خروج من البرنامج", "Exit the program")).parent(&app.options_menu).build(&mut app.menu_exit)?;
        // Last in the result menu, as in Explorer.
        nwg::MenuSeparator::builder().parent(&app.menu).build(&mut app.menu_properties_sep)?;
        nwg::MenuItem::builder().text(tr!("خصائص", "Properties")).parent(&app.menu).build(&mut app.menu_properties)?;
        app.build_suggest_bar()?;
        crate::cloud::set_read_cloud(settings.read_cloud_files);
        app.state.get_mut().settings = settings.clone();
        app.state.get_mut().history = History::load();
        let ui = AppUi { inner: Rc::new(app), handlers: Default::default(), raw_handlers: Default::default() };

        // Regular events.
        let weak = Rc::downgrade(&ui.inner);
        let handler = nwg::full_bind_event_handler(&ui.window.handle, crate::crash_log::guard_events(move |evt, data, handle| {
            use nwg::Event as E;
            let Some(app) = weak.upgrade() else { return };
            match evt {
                // Only the main window's own size matters: child controls
                // report OnResize too, and re-laying out on each of those
                // nests layouts inside layouts.
                E::OnResize | E::OnWindowMaximize if handle == app.window => app.layout(),
                // The X button keeps the program running next to the clock
                // (the floating box and the shortcut keep working); only
                // "Exit" really closes it.
                E::OnWindowClose if handle == app.window => {
                    let hides = app.close_hides();
                    {
                        let s = &app.state.borrow().settings;
                        super::startup::log(&format!(
                            "window close: {} (tray icon {}, floating {:?})",
                            if hides { "hide to tray" } else { "exit" },
                            s.tray_icon,
                            s.floating_mode
                        ));
                    }
                    if hides {
                        if let nwg::EventData::OnWindowClose(d) = &data {
                            d.close(false);
                        }
                        app.hide_to_tray();
                    } else {
                        app.on_close();
                    }
                }
                E::OnTextInput if handle == app.query => {
                    app.note_typing();
                    app.on_query_changed();
                }
                // The dialog-style message loop turns Enter into IDOK and Esc
                // into IDCANCEL, which nwg reports as OnKeyEnter / OnKeyEsc.
                E::OnKeyEnter => app.on_enter(),
                E::OnKeyEsc => app.on_escape(),
                E::OnKeyPress => app.on_key(handle, data.on_key()),
                E::OnTimerTick if handle == app.timer => app.on_tick(),
                E::OnNotice if handle == app.name_notice => app.on_names_found(),
                E::OnNotice if handle == app.update_notice => app.on_update_event(),
                E::OnNotice if handle == app.watch_notice => app.on_watch_notice(),
                E::OnNotice if handle == app.matches_notice => app.on_matches_read(),
                E::OnComboxBoxSelection if handle == app.kind_combo => app.on_kind_changed(),
                E::OnButtonClick => {
                    if app.on_suggestion_click(handle) || app.on_rating_click(handle) {
                    } else if handle == app.search_btn {
                        app.on_search();
                    } else if handle == app.history_btn {
                        app.show_history_menu();
                    } else if handle == app.content_chk {
                        app.on_query_changed();
                    } else if handle == app.scope_btn {
                        app.choose_folder();
                    } else if handle == app.all_btn {
                        app.set_scope(String::new());
                    } else if handle == app.reindex_btn {
                        app.start_scan();
                    } else if handle == app.open_btn {
                        app.open_selected();
                    } else if handle == app.folder_btn {
                        app.open_folder();
                    } else if handle == app.copy_btn {
                        app.copy_path();
                    } else if handle == app.export_btn {
                        app.export_csv();
                    } else if handle == app.stop_btn {
                        app.on_stop();
                    } else if handle == app.options_btn {
                        app.show_menu(&app.options_menu);
                    } else {
                        app.on_app_button(&handle);
                    }
                }
                E::OnListViewItemActivated if handle == app.list => app.open_selected(),
                E::OnListViewItemChanged if handle == app.list => app.show_selected_path(),
                E::OnListViewRightClick if handle == app.list => app.show_menu(&app.menu),
                E::OnListViewColumnClick if handle == app.list => app.on_column_click(data.on_list_view_item_index().1),
                E::OnMousePress(nwg::MousePressEvent::MousePressLeftUp) if handle == app.tray => app.on_tray_click(),
                E::OnContextMenu if handle == app.tray => app.show_tray_menu(),
                E::OnMenuItemSelected => {
                    if app.on_history_item(handle) {
                    } else if handle == app.menu_open {
                        app.open_selected();
                    } else if handle == app.menu_folder {
                        app.open_folder();
                    } else if handle == app.menu_copy {
                        app.copy_path();
                    } else if handle == app.menu_copy_all {
                        app.copy_all_paths();
                    } else if handle == app.menu_copy_file {
                        app.copy_file();
                    } else if handle == app.menu_rename {
                        app.start_rename();
                    } else if handle == app.menu_matches {
                        app.show_matches();
                    } else if handle == app.menu_open_with {
                        app.shell_verb_selected("openas");
                    } else if handle == app.menu_properties {
                        app.shell_verb_selected("properties");
                    } else if handle == app.menu_delete {
                        app.delete_selected();
                    } else if handle == app.menu_hotkey {
                        if app.state.borrow().settings.hotkey_opens == crate::settings::HotkeyOpens::Quick {
                            app.set_status(&trf!("اضغط {HOTKEY_TEXT} من أي برنامج لفتح نافذة البحث الصغيرة.", "Press {HOTKEY_TEXT} in any program to open the quick search."));
                        } else {
                            app.set_status(&trf!("اضغط {HOTKEY_TEXT} من أي برنامج لإظهار بحث سريع أو إخفائه.", "Press {HOTKEY_TEXT} in any program to show or hide InstaFind."));
                        }
                    } else if handle == app.menu_autostart {
                        let on = !app.menu_autostart.checked();
                        if autostart::set_enabled(on) {
                            app.menu_autostart.set_checked(on);
                            app.set_status(if on {
                                tr!("سيعمل البرنامج في الخلفية عند تشغيل ويندوز، وتُظهره بـ Ctrl+Alt+F.", "The program will run in the background when Windows starts; show it with Ctrl+Alt+F.")
                            } else {
                                tr!("لن يعمل البرنامج تلقائياً مع ويندوز.", "The program will not start with Windows.")
                            });
                        } else {
                            app.set_status(&autostart::change_failed_message());
                        }
                    } else if app.on_lang_item(handle) || app.on_feedback_item(handle) || app.on_rating_item(handle) {
                    } else if handle == app.menu_update {
                        // The Store version is told where its updates come from.
                        app.check_for_update(true);
                    } else if handle == app.menu_digits {
                        app.toggle_digits();
                    } else if handle == app.menu_content_index {
                        app.toggle_content_index();
                    } else if handle == app.menu_read_cloud {
                        app.toggle_read_cloud();
                    } else if handle == app.menu_auto_index {
                        app.toggle_auto_index();
                    } else if handle == app.menu_hide_system {
                        app.toggle_hide_system();
                    } else if handle == app.menu_export {
                        app.export_csv();
                    } else if handle == app.menu_hide {
                        app.hide_to_tray();
                    } else if handle == app.menu_new_ui {
                        app.webui_menu_item();
                    } else if handle == app.tray_show {
                        app.show_window();
                    } else if handle == app.tray_exit || handle == app.menu_exit {
                        app.quit();
                    }
                }
                _ => {}
            }
        }));
        ui.handlers.borrow_mut().push(handler);

        // The "all matches" window only hides when closed, and its text fills it.
        let weak = Rc::downgrade(&ui.inner);
        let handler = nwg::full_bind_event_handler(&ui.matches_window.handle, crate::crash_log::guard_events(move |evt, data, handle| {
            let Some(app) = weak.upgrade() else { return };
            if handle != app.matches_window {
                return;
            }
            match evt {
                nwg::Event::OnWindowClose => {
                    if let nwg::EventData::OnWindowClose(d) = &data {
                        d.close(false);
                    }
                    app.matches_window.set_visible(false);
                }
                nwg::Event::OnResize => {
                    let (w, h) = app.matches_window.size();
                    app.matches_text.set_size(w.saturating_sub(16), h.saturating_sub(16));
                }
                _ => {}
            }
        }));
        ui.handlers.borrow_mut().push(handler);

        // Virtual list: answer LVN_GETDISPINFO with the text and icon of each cell.
        let weak = Rc::downgrade(&ui.inner);
        let list_hwnd = ui.list.handle.hwnd().unwrap_or(std::ptr::null_mut());
        let raw = nwg::bind_raw_event_handler(&ui.window.handle, 0x1_0001, crate::crash_log::guard_raw(move |_hwnd: HWND, msg: UINT, w: WPARAM, l: LPARAM| {
            if msg == winapi::um::winuser::WM_HOTKEY && w as i32 == HOTKEY_ID {
                weak.upgrade()?.on_hotkey();
                return Some(0);
            }
            // Windows is shutting down or the user is signing out: agree,
            // then save and leave when it is certain (`w` is true).
            if msg == winapi::um::winuser::WM_QUERYENDSESSION {
                return Some(1);
            }
            if msg == winapi::um::winuser::WM_ENDSESSION {
                if w != 0 {
                    weak.upgrade()?.session_end();
                }
                return Some(0);
            }
            if msg != WM_NOTIFY || l == 0 {
                return None;
            }
            let hdr = unsafe { &*(l as *const NMHDR) };
            if hdr.hwndFrom != list_hwnd {
                return None;
            }
            let app = weak.upgrade()?;
            use winapi::um::commctrl::{LVN_BEGINDRAG, LVN_BEGINLABELEDITW, LVN_ENDLABELEDITW, NM_CUSTOMDRAW, NMLISTVIEW};
            match hdr.code {
                LVN_GETDISPINFOW => {}
                LVN_BEGINLABELEDITW => {
                    let info = unsafe { &*(l as *const NMLVDISPINFOW) };
                    return Some(app.begin_rename(info.item.iItem as usize) as isize);
                }
                LVN_ENDLABELEDITW => {
                    let info = unsafe { &*(l as *const NMLVDISPINFOW) };
                    let text = (!info.item.pszText.is_null()).then(|| unsafe { wide_to_string(info.item.pszText) });
                    return Some(app.finish_rename(text) as isize);
                }
                LVN_BEGINDRAG => {
                    let nm = unsafe { &*(l as *const NMLISTVIEW) };
                    app.drag_row(nm.iItem as usize);
                    return Some(0);
                }
                NM_CUSTOMDRAW => return app.custom_draw(l),
                winapi::um::commctrl::LVN_GETEMPTYMARKUP => {
                    // The sentence an empty list shows (the first index, a
                    // document search still reading).
                    use winapi::um::commctrl::{EMF_CENTERED, L_MAX_URL_LENGTH, NMLVEMPTYMARKUP};
                    let Some(text) = app.empty_list_text() else { return Some(0) };
                    let m = unsafe { &mut *(l as *mut NMLVEMPTYMARKUP) };
                    let wide = crate::indexing_status::empty_markup(&text, L_MAX_URL_LENGTH);
                    m.szMarkup[..wide.len()].copy_from_slice(&wide);
                    m.dwFlags = EMF_CENTERED;
                    return Some(1);
                }
                _ => return None,
            }
            let info = unsafe { &mut *(l as *mut NMLVDISPINFOW) };
            let item = &mut info.item;
            if item.mask & LVIF_TEXT != 0 && !item.pszText.is_null() && item.cchTextMax > 0 {
                let text = app.cell_text(item.iItem as usize, item.iSubItem as usize);
                let max = item.cchTextMax as usize - 1;
                let buf = unsafe { std::slice::from_raw_parts_mut(item.pszText, max + 1) };
                let mut n = 0;
                for u in text.encode_utf16().take(max) {
                    buf[n] = u;
                    n += 1;
                }
                buf[n] = 0;
            }
            if item.mask & LVIF_IMAGE != 0 && item.iSubItem == 0 {
                item.iImage = app.row_icon(item.iItem as usize);
            }
            Some(0)
        }))?;
        ui.raw_handlers.borrow_mut().push(raw);
        let muted = ui.scope_label.handle.hwnd().into_iter().collect();
        ui.raw_handlers.borrow_mut().push(style::bind_window_style(&ui.window, muted)?);
        let (handler, raw) = ui.inner.setup_quick_switch()?;
        ui.handlers.borrow_mut().push(handler);
        ui.raw_handlers.borrow_mut().push(raw);
        ui.handlers.borrow_mut().push(ui.inner.setup_dupes());
        ui.handlers.borrow_mut().push(ui.inner.setup_replace_window());
        ui.handlers.borrow_mut().push(ui.inner.setup_disk_usage());
        ui.handlers.borrow_mut().push(ui.inner.setup_report());
        ui.handlers.borrow_mut().push(ui.inner.setup_web());
        let (handler, raw) = ui.inner.setup_thumbs()?;
        ui.handlers.borrow_mut().push(handler);
        ui.raw_handlers.borrow_mut().push(raw);
        let (handler, raw) = ui.inner.setup_quick()?;
        ui.handlers.borrow_mut().push(handler);
        ui.raw_handlers.borrow_mut().push(raw);
        if let Some((handler, raw)) = ui.inner.setup_webui() {
            ui.handlers.borrow_mut().push(handler);
            ui.raw_handlers.borrow_mut().push(raw);
        }

        ui.register_hotkey();
        ui.show_sort_arrow();
        phase("events");
        ui.layout();
        // Repaint once in full: controls created at (0, 0) before the first
        // layout can leave traces in the margin above the search box.
        if let Some(h) = ui.window.handle.hwnd() {
            use winapi::um::winuser::{RDW_ALLCHILDREN, RDW_ERASE, RDW_INVALIDATE, RedrawWindow};
            unsafe { RedrawWindow(h, std::ptr::null(), std::ptr::null_mut(), RDW_ERASE | RDW_INVALIDATE | RDW_ALLCHILDREN) };
        }
        phase("layout");
        ui.start();
        Ok(ui)
    }
}
