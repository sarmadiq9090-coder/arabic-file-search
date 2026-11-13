//! Accessible names for the program's own native controls that have no
//! label a screen reader could use (Narrator, NVDA): Windows' dynamic
//! annotation (IAccPropServices::SetHwndPropStr with PROPID_ACC_NAME), so
//! the name is given without changing the control's text (an edit box's
//! text is what the user typed).
//!
//! Called on the UI thread, where COM is already initialised.

use winapi::ctypes::c_void;
use winapi::shared::guiddef::GUID;
use winapi::shared::minwindef::DWORD;
use winapi::shared::windef::HWND;
use winapi::shared::winerror::{HRESULT, SUCCEEDED};
use winapi::shared::wtypesbase::CLSCTX_INPROC_SERVER;
use winapi::um::combaseapi::CoCreateInstance;
use winapi::um::unknwnbase::{IUnknown, IUnknownVtbl};
use winapi::um::winnt::LPCWSTR;

use super::shell::wide;
use super::startup::log;

/// CLSID_AccPropServices (oleacc.h).
const CLSID_ACC_PROP_SERVICES: GUID = GUID { Data1: 0xb5f8_350b, Data2: 0x0548, Data3: 0x48b1, Data4: [0xa6, 0xee, 0x88, 0xbd, 0x00, 0xb4, 0xa5, 0xe7] };
/// PROPID_ACC_NAME (oleacc.h).
const PROPID_ACC_NAME: GUID = GUID { Data1: 0x608d_3df8, Data2: 0x8128, Data3: 0x4aa7, Data4: [0xa4, 0x28, 0xf5, 0x5e, 0x49, 0x26, 0x72, 0x91] };
/// OBJID_CLIENT: the control itself, not its frame.
const OBJID_CLIENT: DWORD = 0xFFFF_FFFC;
/// CHILDID_SELF.
const CHILDID_SELF: DWORD = 0;

/// The start of IAccPropServices' table (oleacc.h order), up to the one
/// method used here.
#[repr(C)]
struct AccPropServicesVtbl {
    base: IUnknownVtbl,
    set_prop_value: usize,
    set_prop_server: usize,
    clear_props: usize,
    set_hwnd_prop: usize,
    set_hwnd_prop_str: unsafe extern "system" fn(this: *mut c_void, hwnd: HWND, id_object: DWORD, id_child: DWORD, id_prop: GUID, text: LPCWSTR) -> HRESULT,
}

#[repr(C)]
struct AccPropServices {
    vtbl: *const AccPropServicesVtbl,
}

/// IID_IAccPropServices.
const IID_ACC_PROP_SERVICES: GUID = GUID { Data1: 0x6e26_e776, Data2: 0x04f0, Data3: 0x495d, Data4: [0x80, 0xe4, 0x33, 0x30, 0x35, 0x2e, 0x31, 0x69] };

/// Gives `hwnd` the accessible name `name` (what Narrator and NVDA say for
/// it). False when Windows' accessibility service could not be reached.
pub(super) fn set_name(hwnd: HWND, name: &str) -> bool {
    if hwnd.is_null() {
        return false;
    }
    let mut services: *mut c_void = std::ptr::null_mut();
    let hr = unsafe { CoCreateInstance(&CLSID_ACC_PROP_SERVICES, std::ptr::null_mut(), CLSCTX_INPROC_SERVER, &IID_ACC_PROP_SERVICES, &mut services) };
    if !SUCCEEDED(hr) || services.is_null() {
        log(&format!("accessibility: IAccPropServices not available ({hr:#x})"));
        return false;
    }
    let text = wide(name);
    let ok = unsafe {
        let s = services as *mut AccPropServices;
        let hr = ((*(*s).vtbl).set_hwnd_prop_str)(services, hwnd, OBJID_CLIENT, CHILDID_SELF, PROPID_ACC_NAME, text.as_ptr());
        // (The annotation stays with the window; the service object goes.)
        let unknown = services as *mut IUnknown;
        (*unknown).Release();
        SUCCEEDED(hr)
    };
    if !ok {
        log("accessibility: could not name a control");
    }
    ok
}

// The table must hold IUnknown's three methods, then four, then ours.
const _: () = assert!(std::mem::size_of::<AccPropServicesVtbl>() == 8 * std::mem::size_of::<usize>());
