//! The quick popup's results for screen readers (Narrator, NVDA).
//!
//! The popup draws its rows itself, so Windows knows nothing of them. This
//! module gives them an MSAA object (which UI Automation reads through its
//! MSAA proxy): a list, «النتائج» / "Results", whose children are the rows
//! as list items, named «الاسم، المجلد» with the section's name in front of
//! the first row of a section, with their selected and focused states, their
//! place on screen and a default action (open).
//!
//! The list lives on a child window of the popup (the "host") that covers
//! the list part but has an empty window region: it draws nothing, takes no
//! mouse input and does not clip the popup's own drawing, so nothing changes
//! for the eye. Its `WM_GETOBJECT` answers with the list object.
//!
//! The keyboard focus stays in the search box. When the arrow keys move the
//! selection, the selected row gets `EVENT_OBJECT_FOCUS` and
//! `EVENT_OBJECT_SELECTION` (the way an autocomplete list works), so the
//! screen reader reads it; from then on a new list (typing on) reads its
//! selected row as well, until the caret keys or the box getting the focus
//! again put the reader back in the box.
//!
//! The object only reads a snapshot (names and places) that the popup
//! writes after each change, behind a mutex: it never touches `App`, so a
//! screen reader's calls (which come in on the UI thread while it waits in a
//! message loop) cannot meet a borrowed `RefCell`. Win events are raised
//! after the lock is released: an in-context hook may call straight back.

use std::sync::{Arc, Mutex, MutexGuard};

use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
use winapi::shared::windef::{HWND, RECT};
use winapi::um::winuser::{
    CreateWindowExW, DefWindowProcW, EVENT_OBJECT_FOCUS, EVENT_OBJECT_REORDER, EVENT_OBJECT_SELECTION, GWLP_USERDATA, GetParent, GetWindowLongPtrW,
    GetWindowRect, HTTRANSPARENT, IsWindowVisible, NotifyWinEvent, OBJID_CLIENT, PostMessageW, RegisterClassExW, STATE_SYSTEM_FOCUSABLE, STATE_SYSTEM_FOCUSED,
    STATE_SYSTEM_SELECTABLE, STATE_SYSTEM_SELECTED, SWP_NOACTIVATE, SWP_NOZORDER, SetFocus, SetWindowLongPtrW, SetWindowPos, SetWindowRgn, WM_ERASEBKGND,
    WM_GETOBJECT, WM_NCDESTROY, WM_NCHITTEST, WM_PAINT, WM_SETFOCUS, WNDCLASSEXW, WS_CHILD, WS_VISIBLE,
};
use windows::Win32::Foundation::{DISP_E_MEMBERNOTFOUND, E_FAIL, E_INVALIDARG, E_NOTIMPL, S_FALSE};
use windows::Win32::System::Com::{CoDisconnectObject, DISPATCH_FLAGS, DISPPARAMS, EXCEPINFO, IDispatch, IDispatch_Impl, ITypeInfo};
use windows::Win32::System::Variant::{VARIANT, VT_I4, VT_INT};
use windows::Win32::UI::Accessibility::{
    AccessibleObjectFromWindow, IAccessible, IAccessible_Impl, LresultFromObject, ROLE_SYSTEM_LIST, ROLE_SYSTEM_LISTITEM,
};
use windows::core::{BSTR, GUID, Interface, PCWSTR, Result, implement};

use super::shell::wide;
use super::startup::log;
use super::surface::WM_APP_ACC_OPEN;
use crate::quick::{child_id, navigate_child, row_of_child};

/// CHILDID_SELF: the list itself.
const SELF: i32 = 0;
/// OBJID_WINDOW, for the list's parent.
const OBJID_WINDOW: u32 = 0;
/// The host's id among the popup's children (the search box is 100).
const HOST_ID: usize = 101;
const HOST_CLASS: &str = "InstaFindQuickResults";

/// A result row as a screen reader gets it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Item {
    /// What is read (`crate::quick::spoken_name`).
    pub name: String,
    /// The row's top and height in the list, physical px from its top.
    pub top: i32,
    pub height: i32,
    /// The arrow keys stop on it (`crate::quick::selectable`).
    pub selectable: bool,
}

/// What the list object reads.
#[derive(Default)]
struct State {
    items: Vec<Item>,
    selected: Option<usize>,
    /// The indexing progress line under the results, read as the list's description.
    note: Option<String>,
    /// Changes with the rows, so a default action posted for an older list is dropped.
    generation: u32,
    /// The arrow keys took the screen reader into the list.
    in_list: bool,
    /// False once the host window is gone.
    alive: bool,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

/// The popup's side: the host window and the snapshot its object reads.
pub(super) struct Results {
    host: usize,
    state: Arc<Mutex<State>>,
}

impl Results {
    /// Creates the (empty) list under the popup `parent`. `None` when
    /// Windows refused; the popup works as before without it.
    pub(super) fn create(parent: HWND) -> Option<Results> {
        if !register_class() {
            return None;
        }
        let class = wide(HOST_CLASS);
        let name = wide(list_name());
        let host = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                name.as_ptr(),
                WS_CHILD | WS_VISIBLE,
                0,
                0,
                0,
                0,
                parent,
                HOST_ID as winapi::shared::windef::HMENU,
                winapi::um::libloaderapi::GetModuleHandleW(std::ptr::null()),
                std::ptr::null_mut(),
            )
        };
        if host.is_null() {
            log("accessibility: could not create the quick search's result list");
            return None;
        }
        // Nothing of it is drawn or clicked: the popup under it stays as it is
        // (and is not clipped by it, although it has WS_CLIPCHILDREN).
        let region = unsafe { winapi::um::wingdi::CreateRectRgn(0, 0, 0, 0) };
        if region.is_null() || unsafe { SetWindowRgn(host, region, 0) } == 0 {
            if !region.is_null() {
                unsafe { winapi::um::wingdi::DeleteObject(region.cast()) };
            }
            log("accessibility: the result list has no empty region; left out");
            unsafe { winapi::um::winuser::DestroyWindow(host) };
            return None;
        }
        let state = Arc::new(Mutex::new(State { alive: true, ..State::default() }));
        let object: IAccessible = ResultsList { host: host as usize, state: state.clone() }.into();
        // The window keeps one reference, given back when it is destroyed.
        let owned: Box<Owned> = Box::new((object, state.clone()));
        unsafe { SetWindowLongPtrW(host, GWLP_USERDATA, Box::into_raw(owned) as _) };
        Some(Results { host: host as usize, state })
    }

    fn host(&self) -> HWND {
        self.host as HWND
    }

    /// Covers the list part of the popup (`height` from its top), so the
    /// screen reader's highlight is where the rows are.
    pub(super) fn place(&self, width: i32, height: i32) {
        unsafe { SetWindowPos(self.host(), std::ptr::null_mut(), 0, 0, width.max(0), height.max(0), SWP_NOZORDER | SWP_NOACTIVATE) };
    }

    /// The popup is on screen.
    fn shown(&self) -> bool {
        let parent = unsafe { GetParent(self.host()) };
        !parent.is_null() && unsafe { IsWindowVisible(parent) } != 0
    }

    /// The rows changed. While the reader is in the list, a different
    /// selected row is read out; no row at all puts it back in `edit`.
    pub(super) fn update(&self, items: Vec<Item>, selected: Option<usize>, note: Option<String>, edit: HWND) {
        let selected = selected.filter(|&i| i < items.len());
        let (changed, focus, back) = {
            let mut s = lock(&self.state);
            let name_of = |items: &[Item], i: Option<usize>| i.and_then(|i| items.get(i)).map(|it| it.name.clone());
            let before = (s.selected, name_of(&s.items, s.selected));
            let changed = s.items != items;
            s.items = items;
            s.selected = selected;
            s.note = note;
            if changed {
                s.generation = s.generation.wrapping_add(1);
            }
            let after = (selected, name_of(&s.items, selected));
            let (mut focus, mut back) = (None, false);
            if s.in_list {
                match selected {
                    None => {
                        s.in_list = false;
                        back = true;
                    }
                    Some(i) if before != after => focus = Some(i),
                    _ => {}
                }
            }
            (changed, focus, back)
        };
        if !self.shown() {
            return;
        }
        if changed {
            unsafe { NotifyWinEvent(EVENT_OBJECT_REORDER, self.host(), OBJID_CLIENT, SELF) };
        }
        if let Some(i) = focus {
            self.announce(i);
        }
        if back {
            unsafe { NotifyWinEvent(EVENT_OBJECT_FOCUS, edit, OBJID_CLIENT, SELF) };
        }
    }

    /// The selection moved to `selected`: by the keys (`by_keys`, which
    /// takes the reader into the list and always reads the row), or by the
    /// mouse (read only while the reader is in the list already).
    pub(super) fn select(&self, selected: Option<usize>, by_keys: bool) {
        let read = {
            let mut s = lock(&self.state);
            let selected = selected.filter(|&i| i < s.items.len());
            let moved = s.selected != selected;
            s.selected = selected;
            if by_keys && selected.is_some() {
                s.in_list = true;
            }
            selected.filter(|_| s.in_list && (by_keys || moved))
        };
        if let Some(i) = read
            && self.shown()
        {
            self.announce(i);
        }
    }

    /// The reader goes back to the search box: when the box gets the focus
    /// (Windows tells the reader then), or, with `tell`, for the caret keys.
    pub(super) fn leave_list(&self, edit: HWND, tell: bool) {
        let was = std::mem::replace(&mut lock(&self.state).in_list, false);
        if tell && was && self.shown() {
            unsafe { NotifyWinEvent(EVENT_OBJECT_FOCUS, edit, OBJID_CLIENT, SELF) };
        }
    }

    /// The popup opens again: the reader starts in the box.
    pub(super) fn reset(&self) {
        let mut s = lock(&self.state);
        s.in_list = false;
        s.selected = None;
    }

    /// The rows' generation, to check a posted default action against.
    pub(super) fn generation(&self) -> u32 {
        lock(&self.state).generation
    }

    fn announce(&self, row: usize) {
        let child = child_id(row);
        unsafe {
            NotifyWinEvent(EVENT_OBJECT_FOCUS, self.host(), OBJID_CLIENT, child);
            NotifyWinEvent(EVENT_OBJECT_SELECTION, self.host(), OBJID_CLIENT, child);
        }
    }
}

fn list_name() -> &'static str {
    tr!("النتائج", "Results")
}

fn register_class() -> bool {
    use std::sync::atomic::{AtomicU8, Ordering};
    // 0 = not yet, 1 = registered, 2 = failed.
    static DONE: AtomicU8 = AtomicU8::new(0);
    match DONE.load(Ordering::Relaxed) {
        1 => return true,
        2 => return false,
        _ => {}
    }
    let class = wide(HOST_CLASS);
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as UINT,
        style: 0,
        lpfnWndProc: Some(host_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: unsafe { winapi::um::libloaderapi::GetModuleHandleW(std::ptr::null()) },
        hIcon: std::ptr::null_mut(),
        hCursor: std::ptr::null_mut(),
        hbrBackground: std::ptr::null_mut(),
        lpszMenuName: std::ptr::null(),
        lpszClassName: class.as_ptr(),
        hIconSm: std::ptr::null_mut(),
    };
    let ok = unsafe { RegisterClassExW(&wc) } != 0;
    DONE.store(if ok { 1 } else { 2 }, Ordering::Relaxed);
    ok
}

/// The host's window procedure.
unsafe extern "system" fn host_proc(hwnd: HWND, msg: UINT, w: WPARAM, l: LPARAM) -> LRESULT {
    let answer = crate::crash_log::catch(|| match msg {
        // (lParam is a DWORD: compared as one, as the documentation asks.)
        WM_GETOBJECT if l as u32 == OBJID_CLIENT as u32 => {
            let owned = unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Owned).as_ref() }?;
            Some(unsafe { LresultFromObject(&IAccessible::IID, windows::Win32::Foundation::WPARAM(w), &owned.0) }.0 as LRESULT)
        }
        // Clicks go to the popup under it (the empty region already sees to that).
        WM_NCHITTEST => Some(HTTRANSPARENT),
        WM_ERASEBKGND => Some(1),
        WM_PAINT => {
            unsafe { winapi::um::winuser::ValidateRect(hwnd, std::ptr::null()) };
            Some(0)
        }
        // A reader that moves the focus here hands it on to the search box.
        WM_SETFOCUS => {
            let parent = unsafe { GetParent(hwnd) };
            if !parent.is_null() {
                unsafe { SetFocus(parent) };
            }
            Some(0)
        }
        WM_NCDESTROY => {
            let p = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) } as *mut Owned;
            if !p.is_null() {
                let owned = unsafe { Box::from_raw(p) };
                lock(&owned.1).alive = false;
                // Readers still holding the object are disconnected from it;
                // the window's own reference goes with the box.
                let _ = unsafe { CoDisconnectObject(&owned.0, None) };
            }
            None
        }
        _ => None,
    })
    .flatten();
    match answer {
        Some(r) => r,
        None => unsafe { DefWindowProcW(hwnd, msg, w, l) },
    }
}

/// What the host window keeps (in GWLP_USERDATA): a reference to its list
/// object and the object's snapshot.
type Owned = (IAccessible, Arc<Mutex<State>>);

// ------------------------------------------------------------------ the object

#[implement(IAccessible)]
struct ResultsList {
    host: usize,
    state: Arc<Mutex<State>>,
}

fn int_variant(v: i32) -> VARIANT {
    let mut var = VARIANT::default();
    unsafe {
        let inner = &mut *var.Anonymous.Anonymous;
        inner.vt = VT_I4;
        inner.Anonymous.lVal = v;
    }
    var
}

/// `VT_EMPTY` (all zero).
fn empty_variant() -> VARIANT {
    VARIANT::default()
}

/// The child id a client passed (`VT_I4`, or `VT_INT` from some).
fn child_of(v: &VARIANT) -> Result<i32> {
    let inner = unsafe { &v.Anonymous.Anonymous };
    match inner.vt {
        VT_I4 => Ok(unsafe { inner.Anonymous.lVal }),
        VT_INT => Ok(unsafe { inner.Anonymous.intVal }),
        _ => Err(E_INVALIDARG.into()),
    }
}

/// What a child id names.
enum Who {
    List,
    Row(usize),
}

impl ResultsList {
    fn state(&self) -> Result<MutexGuard<'_, State>> {
        let s = lock(&self.state);
        if s.alive { Ok(s) } else { Err(E_FAIL.into()) }
    }

    fn who(&self, v: &VARIANT) -> Result<(Who, MutexGuard<'_, State>)> {
        let child = child_of(v)?;
        let s = self.state()?;
        if child == SELF {
            return Ok((Who::List, s));
        }
        match row_of_child(child, s.items.len()) {
            Some(i) => Ok((Who::Row(i), s)),
            None => Err(E_INVALIDARG.into()),
        }
    }

    /// That `v` names the list or one of its rows.
    fn check(&self, v: &VARIANT) -> Result<()> {
        self.who(v).map(drop)
    }

    /// The host's place on screen.
    fn screen_rect(&self) -> Result<RECT> {
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        if unsafe { GetWindowRect(self.host as HWND, &mut r) } == 0 {
            return Err(E_FAIL.into());
        }
        Ok(r)
    }
}

fn not_supported<T>() -> Result<T> {
    Err(DISP_E_MEMBERNOTFOUND.into())
}

impl IDispatch_Impl for ResultsList_Impl {
    fn GetTypeInfoCount(&self) -> Result<u32> {
        Ok(0)
    }

    fn GetTypeInfo(&self, _itinfo: u32, _lcid: u32) -> Result<ITypeInfo> {
        Err(E_NOTIMPL.into())
    }

    fn GetIDsOfNames(&self, _riid: *const GUID, _names: *const PCWSTR, _count: u32, _lcid: u32, _ids: *mut i32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn Invoke(
        &self,
        _id: i32,
        _riid: *const GUID,
        _lcid: u32,
        _flags: DISPATCH_FLAGS,
        _params: *const DISPPARAMS,
        _result: *mut VARIANT,
        _excepinfo: *mut EXCEPINFO,
        _argerr: *mut u32,
    ) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
}

impl IAccessible_Impl for ResultsList_Impl {
    fn accParent(&self) -> Result<IDispatch> {
        drop(self.state()?);
        let mut p: *mut std::ffi::c_void = std::ptr::null_mut();
        let host = windows::Win32::Foundation::HWND(self.host as *mut std::ffi::c_void);
        unsafe { AccessibleObjectFromWindow(host, OBJID_WINDOW, &IDispatch::IID, &mut p) }?;
        if p.is_null() {
            return Err(E_FAIL.into());
        }
        Ok(unsafe { IDispatch::from_raw(p) })
    }

    fn accChildCount(&self) -> Result<i32> {
        Ok(i32::try_from(self.state()?.items.len()).unwrap_or(i32::MAX))
    }

    fn get_accChild(&self, child: &VARIANT) -> Result<IDispatch> {
        match self.who(child)? {
            // The rows are simple elements of the list.
            (Who::Row(_), _) => Err(S_FALSE.into()),
            (Who::List, _) => Err(E_INVALIDARG.into()),
        }
    }

    fn get_accName(&self, child: &VARIANT) -> Result<BSTR> {
        Ok(match self.who(child)? {
            (Who::List, _) => BSTR::from(list_name()),
            (Who::Row(i), s) => BSTR::from(s.items[i].name.as_str()),
        })
    }

    fn get_accValue(&self, child: &VARIANT) -> Result<BSTR> {
        self.check(child)?;
        not_supported()
    }

    fn get_accDescription(&self, child: &VARIANT) -> Result<BSTR> {
        match self.who(child)? {
            (Who::List, s) => match s.note.as_deref() {
                Some(note) => Ok(BSTR::from(note)),
                None => not_supported(),
            },
            (Who::Row(_), _) => not_supported(),
        }
    }

    fn get_accRole(&self, child: &VARIANT) -> Result<VARIANT> {
        let role = match self.who(child)?.0 {
            Who::List => ROLE_SYSTEM_LIST,
            Who::Row(_) => ROLE_SYSTEM_LISTITEM,
        };
        Ok(int_variant(role as i32))
    }

    fn get_accState(&self, child: &VARIANT) -> Result<VARIANT> {
        let state = match self.who(child)? {
            (Who::List, _) => 0,
            (Who::Row(i), s) => {
                let mut st = 0;
                if s.items[i].selectable {
                    st |= STATE_SYSTEM_SELECTABLE | STATE_SYSTEM_FOCUSABLE;
                }
                if s.selected == Some(i) {
                    st |= STATE_SYSTEM_SELECTED;
                    // Focused for the reader only once the keys took it into the
                    // list (the keyboard focus itself stays in the search box).
                    if s.in_list {
                        st |= STATE_SYSTEM_FOCUSED;
                    }
                }
                st
            }
        };
        Ok(int_variant(state as i32))
    }

    fn get_accHelp(&self, child: &VARIANT) -> Result<BSTR> {
        self.check(child)?;
        not_supported()
    }

    fn get_accHelpTopic(&self, _file: *mut BSTR, child: &VARIANT) -> Result<i32> {
        self.check(child)?;
        not_supported()
    }

    fn get_accKeyboardShortcut(&self, child: &VARIANT) -> Result<BSTR> {
        self.check(child)?;
        not_supported()
    }

    fn accFocus(&self) -> Result<VARIANT> {
        let s = self.state()?;
        Ok(s.selected.filter(|_| s.in_list).map_or_else(empty_variant, |i| int_variant(child_id(i))))
    }

    fn accSelection(&self) -> Result<VARIANT> {
        let s = self.state()?;
        Ok(s.selected.map_or_else(empty_variant, |i| int_variant(child_id(i))))
    }

    fn get_accDefaultAction(&self, child: &VARIANT) -> Result<BSTR> {
        match self.who(child)?.0 {
            Who::Row(_) => Ok(BSTR::from(tr!("فتح", "Open"))),
            Who::List => not_supported(),
        }
    }

    fn accSelect(&self, _flags: i32, child: &VARIANT) -> Result<()> {
        self.check(child)?;
        not_supported()
    }

    fn accLocation(&self, left: *mut i32, top: *mut i32, width: *mut i32, height: *mut i32, child: &VARIANT) -> Result<()> {
        if left.is_null() || top.is_null() || width.is_null() || height.is_null() {
            return Err(E_INVALIDARG.into());
        }
        let r = self.screen_rect()?;
        let (y, h) = match self.who(child)? {
            (Who::List, _) => (r.top, r.bottom - r.top),
            (Who::Row(i), s) => (r.top + s.items[i].top, s.items[i].height),
        };
        unsafe {
            *left = r.left;
            *top = y;
            *width = r.right - r.left;
            *height = h;
        }
        Ok(())
    }

    fn accNavigate(&self, dir: i32, start: &VARIANT) -> Result<VARIANT> {
        let from = child_of(start)?;
        let len = self.state()?.items.len();
        if from != SELF && row_of_child(from, len).is_none() {
            return Err(E_INVALIDARG.into());
        }
        match navigate_child(from, dir as u32, len) {
            Some(c) => Ok(int_variant(c)),
            None => Err(S_FALSE.into()),
        }
    }

    fn accHitTest(&self, x: i32, y: i32) -> Result<VARIANT> {
        let r = self.screen_rect()?;
        let s = self.state()?;
        if x < r.left || x >= r.right || y < r.top || y >= r.bottom {
            return Ok(empty_variant());
        }
        let dy = y - r.top;
        let hit = s.items.iter().position(|it| dy >= it.top && dy < it.top + it.height);
        Ok(int_variant(hit.map_or(SELF, child_id)))
    }

    fn accDoDefaultAction(&self, child: &VARIANT) -> Result<()> {
        let (Who::Row(i), s) = self.who(child)? else { return not_supported() };
        let generation = s.generation;
        drop(s);
        // Opened by the popup in its own time, never inside the reader's call.
        let parent = unsafe { GetParent(self.host as HWND) };
        if parent.is_null() || unsafe { PostMessageW(parent, WM_APP_ACC_OPEN, i, generation as LPARAM) } == 0 {
            return Err(E_FAIL.into());
        }
        Ok(())
    }

    fn put_accName(&self, _child: &VARIANT, _name: &BSTR) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn put_accValue(&self, _child: &VARIANT, _value: &BSTR) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
}
