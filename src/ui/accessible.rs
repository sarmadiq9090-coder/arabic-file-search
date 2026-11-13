//! Accessible names for the program's own native controls that have no
//! label a screen reader could use (Narrator, NVDA): Windows' dynamic
//! annotation (IAccPropServices::SetHwndPropStr / SetHwndProp on the
//! control's OBJID_CLIENT object), so the name is given without changing
//! the control's text (an edit box's text is what the user typed).
//!
//! Each property is set twice: as the MSAA property (PROPID_ACC_*), which
//! MSAA clients (NVDA) and UI Automation's MSAA proxy read, and as the UI
//! Automation property (direct annotation, Windows 8 and later), which UI
//! Automation's own proxies for Win32 controls read (Narrator).
//!
//! Called on the UI thread, where COM is already initialised.

use winapi::shared::windef::HWND;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Variant::{VARIANT, VT_I4};
use windows::Win32::UI::Accessibility::{
    CAccPropServices, ControlType_Property_GUID, HelpText_Property_GUID, IAccPropServices, Name_Property_GUID, PROPID_ACC_DESCRIPTION, PROPID_ACC_NAME,
    PROPID_ACC_ROLE, ROLE_SYSTEM_TEXT, UIA_EditControlTypeId,
};
use windows::core::{GUID, HSTRING, PCWSTR};

use super::startup::log;

/// OBJID_CLIENT: the control itself, not its frame.
const OBJID_CLIENT: u32 = 0xFFFF_FFFC;
/// CHILDID_SELF.
const CHILDID_SELF: u32 = 0;

/// One annotated property.
enum Value<'a> {
    Text(&'a str),
    Int(i32),
}

/// Sets `props` on `hwnd`'s client object. False when Windows'
/// accessibility service could not be reached or refused one of them.
fn annotate(hwnd: HWND, props: &[(GUID, Value)]) -> bool {
    if hwnd.is_null() {
        return false;
    }
    let services: IAccPropServices = match unsafe { CoCreateInstance(&CAccPropServices, None, CLSCTX_INPROC_SERVER) } {
        Ok(s) => s,
        Err(e) => {
            log(&format!("accessibility: IAccPropServices not available ({:#x})", e.code().0));
            return false;
        }
    };
    let h = windows::Win32::Foundation::HWND(hwnd.cast());
    let mut ok = true;
    for (id, value) in props {
        let r = match value {
            Value::Text(t) => {
                let t = HSTRING::from(*t);
                unsafe { services.SetHwndPropStr(h, OBJID_CLIENT, CHILDID_SELF, *id, PCWSTR(t.as_ptr())) }
            }
            Value::Int(n) => {
                let mut v = VARIANT::default();
                unsafe {
                    let inner = &mut *v.Anonymous.Anonymous;
                    inner.vt = VT_I4;
                    inner.Anonymous.lVal = *n;
                }
                unsafe { services.SetHwndProp(h, OBJID_CLIENT, CHILDID_SELF, *id, &v) }
            }
        };
        ok &= r.is_ok();
    }
    // (The annotations stay with the window; the service object goes.)
    if !ok {
        log("accessibility: could not annotate a control");
    }
    ok
}

/// Names a plain EDIT whose text is what the user types: `name` (what it is
/// for), `description` (how it is used), and the role of an edit box
/// (UI Automation's view of such a box has been seen as a nameless "pane"
/// named after the typed text).
pub(super) fn name_edit(hwnd: HWND, name: &str, description: &str) -> bool {
    annotate(
        hwnd,
        &[
            (PROPID_ACC_NAME, Value::Text(name)),
            (PROPID_ACC_DESCRIPTION, Value::Text(description)),
            (PROPID_ACC_ROLE, Value::Int(ROLE_SYSTEM_TEXT as i32)),
            (Name_Property_GUID, Value::Text(name)),
            (HelpText_Property_GUID, Value::Text(description)),
            (ControlType_Property_GUID, Value::Int(UIA_EditControlTypeId.0)),
        ],
    )
}
