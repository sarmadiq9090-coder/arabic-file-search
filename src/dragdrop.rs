//! Copy files to the clipboard (CF_HDROP) and start an OLE drag of files,
//! so results can be pasted or dropped into Explorer, WhatsApp, Word, etc.
//!
//! Everything here must run on the UI (STA) thread.

use std::cell::Cell;
use winapi::ctypes::c_void;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU32, Ordering};

use winapi::Interface;
use winapi::shared::guiddef::{GUID, IsEqualGUID, REFIID};
use winapi::shared::minwindef::{BOOL, DWORD, HGLOBAL, ULONG};
use winapi::shared::windef::{HWND, POINT};
use winapi::shared::winerror::{
    DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, E_NOINTERFACE, E_POINTER,
    HRESULT, S_OK, SUCCEEDED,
};
use winapi::um::combaseapi::{CoTaskMemAlloc, CoTaskMemFree};
use winapi::um::objidl::IDataObject;
use winapi::um::oleidl::{DROPEFFECT_COPY, DROPEFFECT_LINK};
use winapi::um::unknwnbase::IUnknown;
use winapi::um::winbase::{GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalFree, GlobalLock, GlobalUnlock};
use winapi::um::winuser::{
    CF_HDROP, CloseClipboard, EmptyClipboard, MK_LBUTTON, MK_RBUTTON, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};

type Pidl = *mut c_void;

#[link(name = "shell32")]
unsafe extern "system" {
    fn ILCreateFromPathW(path: *const u16) -> Pidl;
    fn SHCreateDataObject(
        pidl_folder: *const c_void,
        cidl: u32,
        apidl: *const *const c_void,
        inner: *mut IDataObject,
        riid: REFIID,
        ppv: *mut *mut c_void,
    ) -> HRESULT;
    fn SHGetDesktopFolder(folder: *mut *mut ShellFolder) -> HRESULT;
    fn SHDoDragDrop(
        hwnd: HWND,
        data: *mut IDataObject,
        source: *mut c_void,
        ok_effects: DWORD,
        effect: *mut DWORD,
    ) -> HRESULT;
}

#[link(name = "ole32")]
unsafe extern "system" {
    fn OleInitialize(reserved: *mut c_void) -> HRESULT;
    fn DoDragDrop(
        data: *mut IDataObject,
        source: *mut c_void,
        ok_effects: DWORD,
        effect: *mut DWORD,
    ) -> HRESULT;
}

/// `DROPFILES` header that precedes the file list in a CF_HDROP global.
#[repr(C)]
struct DropFiles {
    p_files: DWORD,
    pt: POINT,
    f_nc: BOOL,
    f_wide: BOOL,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Normalises a path for the shell: backslashes, no `\\?\` verbatim prefix.
fn shell_path(p: &str) -> String {
    let p = p.replace('/', "\\");
    if let Some(rest) = p.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{rest}")
    } else if let Some(rest) = p.strip_prefix("\\\\?\\") {
        rest.to_string()
    } else {
        p
    }
}

/// Allocates a movable global block holding `bytes`. Returns null on failure.
unsafe fn global_from_bytes(bytes: &[u8]) -> HGLOBAL {
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes.len().max(1));
        if h.is_null() {
            return null_mut();
        }
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            GlobalFree(h);
            return null_mut();
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        GlobalUnlock(h);
        h
    }
}

/// Builds the bytes of a CF_HDROP block: DROPFILES header + wide,
/// NUL-separated, double-NUL-terminated list.
fn hdrop_bytes(paths: &[String]) -> Vec<u8> {
    let header = std::mem::size_of::<DropFiles>();
    let mut list: Vec<u16> = Vec::new();
    for p in paths {
        let p = shell_path(p);
        if p.is_empty() {
            continue;
        }
        list.extend(p.encode_utf16());
        list.push(0);
    }
    list.push(0);
    let mut out = Vec::with_capacity(header + list.len() * 2);
    let df = DropFiles {
        p_files: header as DWORD,
        pt: POINT { x: 0, y: 0 },
        f_nc: 0,
        f_wide: 1,
    };
    // SAFETY: DropFiles is a plain repr(C) struct of integers.
    let raw = unsafe { std::slice::from_raw_parts(&df as *const DropFiles as *const u8, header) };
    out.extend_from_slice(raw);
    for w in list {
        out.extend_from_slice(&w.to_le_bytes());
    }
    out
}

/// Puts `paths` on the clipboard as files (CF_HDROP + "Preferred DropEffect"
/// = copy), so Ctrl+V in Explorer / WhatsApp / Word pastes the files.
pub fn copy_files(hwnd: HWND, paths: &[String]) -> bool {
    if !paths.iter().any(|p| !p.is_empty()) {
        return false;
    }
    unsafe {
        // The clipboard can be held briefly by another process; retry a bit.
        let mut opened = false;
        for _ in 0..10 {
            if OpenClipboard(hwnd) != 0 {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        if !opened {
            return false;
        }
        let mut ok = false;
        if EmptyClipboard() != 0 {
            let h = global_from_bytes(&hdrop_bytes(paths));
            if !h.is_null() {
                if SetClipboardData(CF_HDROP, h).is_null() {
                    GlobalFree(h);
                } else {
                    ok = true;
                }
            }
            if ok {
                let fmt = RegisterClipboardFormatW(wide("Preferred DropEffect").as_ptr());
                if fmt != 0 {
                    let h2 = global_from_bytes(&DROPEFFECT_COPY.to_le_bytes());
                    if !h2.is_null() && SetClipboardData(fmt, h2).is_null() {
                        GlobalFree(h2);
                    }
                }
            }
        }
        CloseClipboard();
        ok
    }
}

thread_local! {
    static OLE_READY: Cell<bool> = const { Cell::new(false) };
}

/// Calls OleInitialize once per thread (needed by DoDragDrop). S_FALSE
/// (already initialised) counts as success. Never uninitialised: lives for
/// the life of the UI thread.
fn ensure_ole() -> bool {
    OLE_READY.with(|r| {
        if !r.get() {
            let hr = unsafe { OleInitialize(null_mut()) };
            if SUCCEEDED(hr) {
                r.set(true);
            }
        }
        r.get()
    })
}

// ---------------------------------------------------------------------------
// Minimal IDropSource
// ---------------------------------------------------------------------------

const IID_IUNKNOWN: GUID = GUID {
    Data1: 0x0000_0000,
    Data2: 0x0000,
    Data3: 0x0000,
    Data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46],
};
const IID_IDROPSOURCE: GUID = GUID {
    Data1: 0x0000_0121,
    Data2: 0x0000,
    Data3: 0x0000,
    Data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46],
};

#[repr(C)]
struct DropSourceVtbl {
    query_interface:
        unsafe extern "system" fn(*mut DropSource, REFIID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut DropSource) -> ULONG,
    release: unsafe extern "system" fn(*mut DropSource) -> ULONG,
    query_continue_drag: unsafe extern "system" fn(*mut DropSource, BOOL, DWORD) -> HRESULT,
    give_feedback: unsafe extern "system" fn(*mut DropSource, DWORD) -> HRESULT,
}

#[repr(C)]
struct DropSource {
    vtbl: *const DropSourceVtbl,
    refs: AtomicU32,
}

static DROP_SOURCE_VTBL: DropSourceVtbl = DropSourceVtbl {
    query_interface: ds_query_interface,
    add_ref: ds_add_ref,
    release: ds_release,
    query_continue_drag: ds_query_continue_drag,
    give_feedback: ds_give_feedback,
};

unsafe extern "system" fn ds_query_interface(
    this: *mut DropSource,
    riid: REFIID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    unsafe {
        if ppv.is_null() || riid.is_null() {
            return E_POINTER;
        }
        if IsEqualGUID(&*riid, &IID_IUNKNOWN) || IsEqualGUID(&*riid, &IID_IDROPSOURCE) {
            ds_add_ref(this);
            *ppv = this as *mut c_void;
            S_OK
        } else {
            *ppv = null_mut();
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn ds_add_ref(this: *mut DropSource) -> ULONG {
    unsafe { (*this).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

unsafe extern "system" fn ds_release(this: *mut DropSource) -> ULONG {
    unsafe {
        let n = (*this).refs.fetch_sub(1, Ordering::AcqRel) - 1;
        if n == 0 {
            drop(Box::from_raw(this));
        }
        n
    }
}

unsafe extern "system" fn ds_query_continue_drag(
    _this: *mut DropSource,
    escape: BOOL,
    keys: DWORD,
) -> HRESULT {
    if escape != 0 {
        DRAGDROP_S_CANCEL
    } else if keys & (MK_LBUTTON | MK_RBUTTON) as DWORD == 0 {
        DRAGDROP_S_DROP
    } else {
        S_OK
    }
}

unsafe extern "system" fn ds_give_feedback(_this: *mut DropSource, _effect: DWORD) -> HRESULT {
    DRAGDROP_S_USEDEFAULTCURSORS
}

/// New drop source with one reference owned by the caller.
fn new_drop_source() -> *mut DropSource {
    Box::into_raw(Box::new(DropSource {
        vtbl: &DROP_SOURCE_VTBL,
        refs: AtomicU32::new(1),
    }))
}

// ---------------------------------------------------------------------------
// PIDL helpers (an ITEMIDLIST is a run of SHITEMID {cb: u16, data}, ended by cb == 0)
// ---------------------------------------------------------------------------

/// Byte offset of the last SHITEMID, and total size without the terminator.
unsafe fn pidl_last(pidl: Pidl) -> (usize, usize) {
    unsafe {
        let base = pidl as *const u8;
        let mut off = 0usize;
        let mut last = 0usize;
        loop {
            let cb = std::ptr::read_unaligned(base.add(off) as *const u16) as usize;
            if cb == 0 {
                return (last, off);
            }
            last = off;
            off += cb;
        }
    }
}

/// Owned PIDLs to free with CoTaskMemFree.
struct Pidls(Vec<Pidl>);

impl Drop for Pidls {
    fn drop(&mut self) {
        for &p in &self.0 {
            if !p.is_null() {
                unsafe { CoTaskMemFree(p) };
            }
        }
    }
}

/// Builds a shell IDataObject (CF_HDROP, CFSTR_SHELLIDLIST, file contents…)
/// for the given files. Files that don't resolve are skipped.
/// Returns an owned IDataObject (Release it) or null.
pub(crate) fn create_data_object(paths: &[String]) -> *mut IDataObject {
    unsafe {
        let mut abs = Pidls(Vec::new());
        for p in paths {
            let p = shell_path(p);
            if p.is_empty() {
                continue;
            }
            let pidl = ILCreateFromPathW(wide(&p).as_ptr());
            if !pidl.is_null() {
                abs.0.push(pidl);
            }
        }
        if abs.0.is_empty() {
            return null_mut();
        }

        // Use the common parent folder when all items share it (the normal
        // case, and what Explorer itself produces); otherwise root the
        // items at the desktop with their absolute PIDLs.
        let parts: Vec<(usize, usize)> = abs.0.iter().map(|&p| pidl_last(p)).collect();
        let first = std::slice::from_raw_parts(abs.0[0] as *const u8, parts[0].0);
        let same_parent = abs.0.iter().zip(&parts).all(|(&p, &(last, _))| {
            std::slice::from_raw_parts(p as *const u8, last) == first
        }) && parts.iter().all(|&(last, end)| end > last);

        let mut owned_parent = Pidls(Vec::new());
        let (parent, children): (*const c_void, Vec<*const c_void>) = if same_parent {
            let n = first.len();
            let buf = CoTaskMemAlloc(n + 2) as *mut u8;
            if buf.is_null() {
                return null_mut();
            }
            std::ptr::copy_nonoverlapping(first.as_ptr(), buf, n);
            *buf.add(n) = 0;
            *buf.add(n + 1) = 0;
            owned_parent.0.push(buf as Pidl);
            let kids = abs
                .0
                .iter()
                .zip(&parts)
                .map(|(&p, &(last, _))| (p as *const u8).add(last) as *const c_void)
                .collect();
            (buf as *const c_void, kids)
        } else {
            (null_mut(), abs.0.iter().map(|&p| p as *const c_void).collect())
        };

        let mut obj: *mut c_void = null_mut();
        let hr = SHCreateDataObject(
            parent,
            children.len() as u32,
            children.as_ptr(),
            null_mut(),
            &IDataObject::uuidof(),
            &mut obj,
        );
        if SUCCEEDED(hr) && !obj.is_null() {
            // The data object copies the PIDLs; ours are freed on drop.
            return obj as *mut IDataObject;
        }
        // Fallback (older shells, Wine): ask the parent folder for the
        // data object, as Explorer does.
        // Only for a common parent: folders don't accept multi-level PIDLs.
        if !same_parent {
            return null_mut();
        }
        ui_object_from_folder(parent, &children)
    }
}

/// Just enough of the IShellFolder vtable to reach GetUIObjectOf.
#[repr(C)]
struct ShellFolderVtbl {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(*mut ShellFolder) -> ULONG,
    parse_display_name: usize,
    enum_objects: usize,
    bind_to_object: unsafe extern "system" fn(
        *mut ShellFolder,
        *const c_void,
        *mut c_void,
        REFIID,
        *mut *mut c_void,
    ) -> HRESULT,
    bind_to_storage: usize,
    compare_ids: usize,
    create_view_object: usize,
    get_attributes_of: usize,
    get_ui_object_of: unsafe extern "system" fn(
        *mut ShellFolder,
        HWND,
        u32,
        *const *const c_void,
        REFIID,
        *mut u32,
        *mut *mut c_void,
    ) -> HRESULT,
}

#[repr(C)]
struct ShellFolder {
    vtbl: *const ShellFolderVtbl,
}

const IID_ISHELLFOLDER: GUID = GUID {
    Data1: 0x0002_14E6,
    Data2: 0x0000,
    Data3: 0x0000,
    Data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46],
};

/// IShellFolder::GetUIObjectOf(IID_IDataObject) on `parent` (null or empty
/// = desktop). Returns an owned IDataObject or null.
unsafe fn ui_object_from_folder(parent: *const c_void, children: &[*const c_void]) -> *mut IDataObject {
    unsafe {
        let mut desktop: *mut ShellFolder = null_mut();
        if !SUCCEEDED(SHGetDesktopFolder(&mut desktop)) || desktop.is_null() {
            return null_mut();
        }
        let is_root = parent.is_null() || std::ptr::read_unaligned(parent as *const u16) == 0;
        let folder = if is_root {
            desktop
        } else {
            let mut f: *mut c_void = null_mut();
            let hr = ((*(*desktop).vtbl).bind_to_object)(
                desktop,
                parent,
                null_mut(),
                &IID_ISHELLFOLDER,
                &mut f,
            );
            ((*(*desktop).vtbl).release)(desktop);
            if !SUCCEEDED(hr) || f.is_null() {
                return null_mut();
            }
            f as *mut ShellFolder
        };
        let mut obj: *mut c_void = null_mut();
        let hr = ((*(*folder).vtbl).get_ui_object_of)(
            folder,
            null_mut(),
            children.len() as u32,
            children.as_ptr(),
            &IDataObject::uuidof(),
            null_mut(),
            &mut obj,
        );
        ((*(*folder).vtbl).release)(folder);
        if SUCCEEDED(hr) { obj as *mut IDataObject } else { null_mut() }
    }
}

/// Drags the files (copy or link) with the shell's drag image. `hwnd` may be
/// null; if it is the list view, the shell asks it for an image of the
/// selected rows (DI_GETDRAGIMAGE). Blocks until the drop or cancel.
/// Returns false if the drag could not be started.
pub fn drag_files_from(hwnd: HWND, paths: &[String]) -> bool {
    if !ensure_ole() {
        return false;
    }
    let data = create_data_object(paths);
    if data.is_null() {
        return false;
    }
    unsafe {
        let src = new_drop_source();
        let mut effect: DWORD = 0;
        let ok_effects = DROPEFFECT_COPY | DROPEFFECT_LINK;
        let mut hr = SHDoDragDrop(hwnd, data, src as *mut c_void, ok_effects, &mut effect);
        if !SUCCEEDED(hr) {
            hr = DoDragDrop(data, src as *mut c_void, ok_effects, &mut effect);
        }
        ds_release(src);
        (*(data as *mut IUnknown)).Release();
        SUCCEEDED(hr)
    }
}

