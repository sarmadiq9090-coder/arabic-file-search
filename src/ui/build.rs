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
use super::{autostart, shell};
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
    }
}

impl NativeUi<AppUi> for App {
    fn build_ui(mut app: App) -> Result<AppUi, nwg::NwgError> {
        let mut settings = Settings::load();
        // A size saved on a bigger screen must still fit this one.
        let (max_w, max_h) = work_area();
        settings.width = settings.width.min(max_w);
        settings.height = settings.height.min(max_h);
        ARABIC_DIGITS.store(settings.arabic_digits, Ordering::Relaxed);

        let family = ui_font_family();
        phase("settings");
        let scale = nwg::scale_factor();
        nwg::Font::builder().family(family).size_absolute((15.0 * scale).round() as u32).build(&mut app.font)?;
        nwg::Font::builder()
            .family(family)
            .size_absolute((19.0 * scale).round() as u32)
            .build(&mut app.query_font)?;
        let f = Some(&app.font);

        nwg::Window::builder()
            .title(title())
            .size((settings.width as i32, settings.height as i32))
            .center(true)
            .ex_flags(crate::i18n::rtl_only(WS_EX_LAYOUTRTL))
            .flags({
                // Started with Windows: wait in the tray until called.
                let visible = if std::env::args().any(|a| a == autostart::STARTUP_ARG) {
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
        }
        phase("window");

        nwg::TextInput::builder()
            .parent(w)
            .font(Some(&app.query_font))
            .placeholder_text(Some(tr!("اكتب اسم الملف أو كلمة للبحث…   أمثلة:  تقرير ‎.pdf   ·   عقد !قديم   ·   صورة|فيديو", "Type a file name or a word…   e.g.  report .pdf   ·   contract !old   ·   photo|video")))
            .focus(true)
            .build(&mut app.query)?;
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
        nwg::Label::builder().parent(w).font(f).text(tr!("النطاق:", "Scope:")).build(&mut app.scope_label)?;
        nwg::TextInput::builder()
            .parent(w)
            .font(f)
            .readonly(true)
            .text(tr!("كل الأقراص", "All drives"))
            .build(&mut app.scope_input)?;
        nwg::Button::builder().parent(w).font(f).text(tr!("اختيار مجلد…", "Choose folder…")).build(&mut app.scope_btn)?;
        nwg::Button::builder().parent(w).font(f).text(tr!("كل الأقراص", "All drives")).build(&mut app.all_btn)?;
        nwg::Button::builder().parent(w).font(f).text(tr!("تحديث الفهرس", "Update index")).build(&mut app.reindex_btn)?;

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
        for (i, (title, width)) in COLUMNS.iter().enumerate() {
            let width = settings.columns.get(i).copied().filter(|w| (20..3000).contains(w)).unwrap_or(*width);
            app.list.insert_column(nwg::InsertListViewColumn {
                index: Some(i as i32),
                fmt: None,
                width: Some((width as f64 * scale).round() as i32),
                text: Some(tr!(*title, super::COLUMNS_EN[i]).to_string()),
            });
        }
        app.list.set_headers_enabled(true);

        nwg::Button::builder().parent(w).font(f).text(tr!("فتح الملف", "Open file")).build(&mut app.open_btn)?;
        nwg::Button::builder().parent(w).font(f).text(tr!("فتح المجلد", "Open folder")).build(&mut app.folder_btn)?;
        nwg::Button::builder().parent(w).font(f).text(tr!("نسخ المسار", "Copy path")).build(&mut app.copy_btn)?;
        nwg::Button::builder().parent(w).font(f).text(tr!("تصدير النتائج", "Export results")).build(&mut app.export_btn)?;
        nwg::Button::builder().parent(w).font(f).text(tr!("إيقاف البحث", "Stop search")).enabled(false).build(&mut app.stop_btn)?;
        nwg::Button::builder().parent(w).font(f).text(tr!("خيارات…", "Options…")).build(&mut app.options_btn)?;
        nwg::StatusBar::builder().parent(w).font(f).text("").build(&mut app.status)?;

        nwg::AnimationTimer::builder()
            .parent(w)
            .interval(Duration::from_millis(200))
            .active(true)
            .build(&mut app.timer)?;
        nwg::Notice::builder().parent(w).build(&mut app.name_notice)?;
        nwg::Notice::builder().parent(w).build(&mut app.update_notice)?;
        nwg::Notice::builder().parent(w).build(&mut app.watch_notice)?;
        phase("controls");
        nwg::Menu::builder().popup(true).parent(w).build(&mut app.menu)?;
        nwg::MenuItem::builder().text(tr!("فتح الملف\tEnter", "Open file\tEnter")).parent(&app.menu).build(&mut app.menu_open)?;
        nwg::MenuItem::builder().text(tr!("فتح المجلد الذي يحتويه\tCtrl+Enter", "Open containing folder\tCtrl+Enter")).parent(&app.menu).build(&mut app.menu_folder)?;
        nwg::MenuItem::builder().text(tr!("نسخ الملف\tCtrl+C", "Copy file\tCtrl+C")).parent(&app.menu).build(&mut app.menu_copy_file)?;
        nwg::MenuItem::builder().text(tr!("نسخ المسار\tCtrl+Shift+C", "Copy path\tCtrl+Shift+C")).parent(&app.menu).build(&mut app.menu_copy)?;
        nwg::MenuItem::builder().text(tr!("نسخ مسارات كل النتائج", "Copy paths of all results")).parent(&app.menu).build(&mut app.menu_copy_all)?;
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

        nwg::Menu::builder().popup(true).parent(w).build(&mut app.options_menu)?;
        nwg::MenuItem::builder()
            .text(tr!("عرض الأرقام بالعربية (٠١٢٣)", "Show Arabic-Indic digits (٠١٢٣)"))
            .check(settings.arabic_digits)
            .parent(&app.options_menu)
            .build(&mut app.menu_digits)?;
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
        nwg::MenuItem::builder().text(tr!("تصدير النتائج إلى Excel (CSV)", "Export results to Excel (CSV)")).parent(&app.options_menu).build(&mut app.menu_export)?;
        nwg::MenuItem::builder().text(tr!("إخفاء إلى شريط المهام\tEsc", "Hide to the taskbar tray\tEsc")).parent(&app.options_menu).build(&mut app.menu_hide)?;
        nwg::MenuItem::builder()
            .text(&trf!("إظهار البرنامج من أي مكان\t{HOTKEY_TEXT}", "Show the program from anywhere\t{HOTKEY_TEXT}"))
            .parent(&app.options_menu)
            .build(&mut app.menu_hotkey)?;
        nwg::MenuItem::builder()
            .text(&trf!("التحقق من التحديثات (الإصدار {})", "Check for updates (version {})", env!("CARGO_PKG_VERSION")))
            .parent(&app.options_menu)
            .build(&mut app.menu_update)?;
        nwg::MenuItem::builder()
            .text(tr!("التشغيل مع ويندوز (في الخلفية)", "Start with Windows (in the background)"))
            .check(autostart::is_enabled())
            .parent(&app.options_menu)
            .build(&mut app.menu_autostart)?;
        super::lang_menu::build(&app.options_menu, &mut app.lang_menu, &mut app.menu_lang_ar, &mut app.menu_lang_en)?;

        // Tray icon, so a hidden window comes back instantly.
        app.tray_icon.handle = crate::icon::load_icon(true) as _;
        if app.tray_icon.handle.is_null() {
            nwg::Icon::builder().source_system(Some(nwg::OemIcon::Information)).build(&mut app.tray_icon)?;
        }
        nwg::TrayNotification::builder()
            .parent(w)
            .icon(Some(&app.tray_icon))
            .tip(Some(&format!("{} ({HOTKEY_TEXT})", title())))
            .build(&mut app.tray)?;
        nwg::Menu::builder().popup(true).parent(w).build(&mut app.tray_menu)?;
        nwg::MenuItem::builder().text(tr!("إظهار بحث سريع", "Show InstaFind")).parent(&app.tray_menu).build(&mut app.tray_show)?;
        nwg::MenuItem::builder().text(tr!("خروج", "Exit")).parent(&app.tray_menu).build(&mut app.tray_exit)?;

        phase("tray");
        app.build_quick_switch_menus()?;
        app.state.get_mut().settings = settings.clone();
        app.state.get_mut().history = History::load();
        let ui = AppUi { inner: Rc::new(app), handlers: Default::default(), raw_handlers: Default::default() };

        // Regular events.
        let weak = Rc::downgrade(&ui.inner);
        let handler = nwg::full_bind_event_handler(&ui.window.handle, move |evt, data, handle| {
            use nwg::Event as E;
            let Some(app) = weak.upgrade() else { return };
            match evt {
                // Only the main window's own size matters: child controls
                // report OnResize too, and re-laying out on each of those
                // nests layouts inside layouts.
                E::OnResize | E::OnWindowMaximize if handle == app.window => app.layout(),
                E::OnWindowClose if handle == app.window => app.on_close(),
                E::OnTextInput if handle == app.query => app.on_query_changed(),
                // The dialog-style message loop turns Enter into IDOK and Esc
                // into IDCANCEL, which nwg reports as OnKeyEnter / OnKeyEsc.
                E::OnKeyEnter => app.on_enter(),
                E::OnKeyEsc => app.on_escape(),
                E::OnKeyPress => app.on_key(handle, data.on_key()),
                E::OnTimerTick if handle == app.timer => app.on_tick(),
                E::OnNotice if handle == app.name_notice => app.on_names_found(),
                E::OnNotice if handle == app.update_notice => app.on_update_event(),
                E::OnNotice if handle == app.watch_notice => app.on_watch_notice(),
                E::OnComboxBoxSelection if handle == app.kind_combo => app.on_kind_changed(),
                E::OnButtonClick => {
                    if handle == app.search_btn {
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
                    }
                }
                E::OnListViewItemActivated if handle == app.list => app.open_selected(),
                E::OnListViewItemChanged if handle == app.list => app.show_selected_path(),
                E::OnListViewRightClick if handle == app.list => app.show_menu(&app.menu),
                E::OnListViewColumnClick if handle == app.list => app.on_column_click(data.on_list_view_item_index().1),
                E::OnMousePress(nwg::MousePressEvent::MousePressLeftUp) if handle == app.tray => app.show_window(),
                E::OnContextMenu if handle == app.tray => app.show_menu(&app.tray_menu),
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
                    } else if handle == app.menu_delete {
                        app.delete_selected();
                    } else if handle == app.menu_hotkey {
                        app.set_status(&trf!("اضغط {HOTKEY_TEXT} من أي برنامج لإظهار بحث سريع أو إخفائه.", "Press {HOTKEY_TEXT} in any program to show or hide InstaFind."));
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
                            app.set_status(tr!("تعذر تغيير إعداد التشغيل مع ويندوز.", "Could not change the start-with-Windows setting."));
                        }
                    } else if app.on_lang_item(handle) {
                    } else if handle == app.menu_update {
                        app.check_for_update(true);
                    } else if handle == app.menu_digits {
                        app.toggle_digits();
                    } else if handle == app.menu_content_index {
                        app.toggle_content_index();
                    } else if handle == app.menu_auto_index {
                        app.toggle_auto_index();
                    } else if handle == app.menu_export {
                        app.export_csv();
                    } else if handle == app.menu_hide {
                        app.hide_to_tray();
                    } else if handle == app.tray_show {
                        app.show_window();
                    } else if handle == app.tray_exit {
                        app.window.close();
                    }
                }
                _ => {}
            }
        });
        ui.handlers.borrow_mut().push(handler);

        // The "all matches" window only hides when closed, and its text fills it.
        let weak = Rc::downgrade(&ui.inner);
        let handler = nwg::full_bind_event_handler(&ui.matches_window.handle, move |evt, data, handle| {
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
        });
        ui.handlers.borrow_mut().push(handler);

        // Virtual list: answer LVN_GETDISPINFO with the text and icon of each cell.
        let weak = Rc::downgrade(&ui.inner);
        let list_hwnd = ui.list.handle.hwnd().unwrap_or(std::ptr::null_mut());
        let raw = nwg::bind_raw_event_handler(&ui.window.handle, 0x1_0001, move |_hwnd: HWND, msg: UINT, w: WPARAM, l: LPARAM| {
            if msg == winapi::um::winuser::WM_HOTKEY && w as i32 == HOTKEY_ID {
                weak.upgrade()?.on_hotkey();
                return Some(0);
            }
            if msg != WM_NOTIFY {
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
        })?;
        ui.raw_handlers.borrow_mut().push(raw);
        let (handler, raw) = ui.inner.setup_quick_switch()?;
        ui.handlers.borrow_mut().push(handler);
        ui.raw_handlers.borrow_mut().push(raw);

        ui.register_hotkey();
        ui.show_sort_arrow();
        phase("events");
        ui.layout();
        phase("layout");
        ui.start();
        Ok(ui)
    }
}
