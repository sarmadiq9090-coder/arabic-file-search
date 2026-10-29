//! Highlighting of the matched part of each file name in the results list.
//!
//! The list draws itself as usual (theme selection, icon, shaped Arabic
//! text); we only ask for an item post-paint notification and then draw a
//! thin accent-coloured underline under the matched glyphs. The text itself
//! is never redrawn: drawing part of an Arabic word separately would break
//! the joining of its letters.
//!
//! # Where the glyphs are
//!
//! Positions come from Uniscribe (`ScriptStringAnalyse` +
//! `ScriptStringCPtoX`) run on the cell text with the list's own font, so
//! ligatures, joining, font fallback and bidi reordering of mixed
//! Arabic/Latin names are all accounted for. `ScriptStringCPtoX` reports a
//! visual offset measured from the *visual left edge* of the string, the
//! same whether the paragraph is RTL or LTR.
//!
//! # Mirroring
//!
//! The main window has `WS_EX_LAYOUTRTL`, so the list and the DC it paints
//! with are mirrored: logical x grows from the right edge of the window to
//! the left. Column 0 is always `LVCFMT_LEFT`, i.e. the list draws the label
//! at `label.left + pad` in *logical* units, which on screen is the right end
//! of the label. In a mirrored DC GDI also lays text out with a
//! right-to-left reading order (the LPK turns on `ETO_RTLREADING` for
//! mirrored DCs, as ReactOS's LPK and Wine's win32u both model), so we
//! analyse with `SSA_RTL` in that case.
//!
//! On screen the string therefore occupies `[label.left + pad, label.left +
//! pad + width]` logically, its *visual right* edge being at logical
//! `label.left + pad`. A visual offset `v` from the string's visual left edge
//! is at logical `x = label.left + pad + width - v`. For an unmirrored DC the
//! mapping is simply `x = label.left + pad + v`. A name too long for the
//! column is laid out as the list shows it (the start that fits plus "..."),
//! so the cut does not shift anything. Rectangles are then filled
//! in logical coordinates, which GDI mirrors the same way it mirrored the
//! text.
//!
//! If anything fails (no font, Uniscribe error, odd rectangle) nothing is
//! drawn and the list's own painting is left untouched.

use std::mem::zeroed;
use std::ptr::{null, null_mut};

use winapi::shared::basetsd::LONG_PTR;
use winapi::shared::minwindef::{LPARAM, WPARAM};
use winapi::shared::windef::{HDC, HFONT, HGDIOBJ, HWND, RECT, SIZE};
use winapi::shared::winerror::SUCCEEDED;
use winapi::um::commctrl::{
    CDDS_ITEMPOSTPAINT, CDDS_ITEMPREPAINT, CDDS_PREPAINT, CDRF_DODEFAULT, CDRF_NOTIFYITEMDRAW,
    CDRF_NOTIFYPOSTPAINT, LVIR_LABEL, LVM_GETITEMRECT, NMCUSTOMDRAW,
};
use winapi::um::usp10::{
    SCRIPT_STRING_ANALYSIS, SSA_FALLBACK, SSA_GLYPHS, SSA_RTL, ScriptStringAnalyse, ScriptStringCPtoX,
    ScriptStringFree, ScriptString_pSize,
};
use winapi::um::wingdi::{
    CreateSolidBrush, DeleteObject, GDI_ERROR, GetDeviceCaps, GetLayout, GetTextAlign, GetTextExtentExPointW, GetTextExtentPoint32W,
    GetTextMetricsW, LAYOUT_RTL, LOGPIXELSX, SelectObject, TA_RTLREADING, TEXTMETRICW,
};
use winapi::um::winuser::{
    COLOR_HIGHLIGHT, FillRect, GWL_EXSTYLE, GetSysColor, GetSystemMetrics, GetWindowLongPtrW, SM_CXEDGE,
    SendMessageW, WM_GETFONT, WS_EX_RTLREADING,
};

/// The list asks for at most 260 UTF-16 units of a cell (`cchTextMax`,
/// including the terminating zero), so that is all it ever draws.
const MAX_UNITS: usize = 259;

/// Answers `NM_CUSTOMDRAW` from the results list.
///
/// Call it from the parent's raw `WM_NOTIFY` handler when
/// `hdr.hwndFrom == list && hdr.code == NM_CUSTOMDRAW`, and return its result
/// as the message result (it is always `Some`).
///
/// * `ranges_for_row(row)` — byte ranges `(start, end)` of the match in the
///   column-0 text of `row`; return an empty `Vec` (no allocation) when the
///   row has nothing to highlight. Called once per painted row.
/// * `text_for_row(row)` — the column-0 text of `row`, exactly as answered to
///   `LVN_GETDISPINFOW`. Only called when the row has ranges.
pub fn handle_custom_draw(
    lparam: LPARAM,
    ranges_for_row: impl Fn(usize) -> Vec<(usize, usize)>,
    text_for_row: impl Fn(usize) -> String,
) -> Option<isize> {
    if lparam == 0 {
        return None;
    }
    // Only the leading NMCUSTOMDRAW part is read, so this works whatever size
    // of NMLVCUSTOMDRAW the list sends.
    let cd = unsafe { &*(lparam as *const NMCUSTOMDRAW) };
    match cd.dwDrawStage {
        CDDS_PREPAINT => Some(CDRF_NOTIFYITEMDRAW),
        // Item post-paint arrives once the whole row (selection background,
        // icon, every column's text) is painted: no sub-item notifications
        // are needed, and the default drawing stays exactly as it was.
        CDDS_ITEMPREPAINT => Some(CDRF_NOTIFYPOSTPAINT),
        CDDS_ITEMPOSTPAINT => {
            let row = cd.dwItemSpec;
            let ranges = ranges_for_row(row);
            if !ranges.is_empty() {
                let text = text_for_row(row);
                unsafe { underline(cd.hdr.hwndFrom, cd.hdc, row, &text, &ranges) };
            }
            Some(CDRF_DODEFAULT)
        }
        _ => Some(CDRF_DODEFAULT),
    }
}

/// Restores the DC's font when dropped.
struct FontGuard(HDC, HGDIOBJ);
impl Drop for FontGuard {
    fn drop(&mut self) {
        if !self.1.is_null() {
            unsafe { SelectObject(self.0, self.1) };
        }
    }
}

/// Frees the Uniscribe analysis when dropped.
struct SsaGuard(SCRIPT_STRING_ANALYSIS);
impl SsaGuard {
    /// Width of the laid-out string (checked non-null by `analyse`).
    fn width(&self) -> i32 {
        unsafe { (*ScriptString_pSize(self.0)).cx }
    }
}
impl Drop for SsaGuard {
    fn drop(&mut self) {
        unsafe { ScriptStringFree(&mut self.0) };
    }
}

/// Draws the underline under the matched glyphs of column 0 of `row`.
/// Draws nothing on any failure.
unsafe fn underline(list: HWND, hdc: HDC, row: usize, text: &str, ranges: &[(usize, usize)]) {
    if list.is_null() || hdc.is_null() || text.is_empty() {
        return;
    }

    // Label rectangle of column 0 (after the icon), in the list's client
    // coordinates, which are the DC's logical coordinates.
    let mut label: RECT = unsafe { zeroed() };
    label.left = LVIR_LABEL;
    if unsafe { SendMessageW(list, LVM_GETITEMRECT, row as WPARAM, &mut label as *mut RECT as LPARAM) } == 0 {
        return;
    }
    // The list's font, so measurements match its drawing.
    let font = unsafe { SendMessageW(list, WM_GETFONT, 0, 0) } as HFONT;
    let _font = FontGuard(hdc, if font.is_null() { null_mut() } else { unsafe { SelectObject(hdc, font as HGDIOBJ) } });

    let exstyle = unsafe { GetWindowLongPtrW(list, GWL_EXSTYLE) } as LONG_PTR as u32;
    unsafe { paint_spans(hdc, &label, exstyle & WS_EX_RTLREADING != 0, text, ranges) };
}

/// Underlines the byte `ranges` of `text`, drawn by the list with `DT_LEFT`
/// at `label.left + SM_CXEDGE`, vertically centred, with the font currently
/// selected into `hdc`. `rtl_reading` is the control's own RTL-reading flag;
/// a mirrored DC implies it.
unsafe fn paint_spans(hdc: HDC, label: &RECT, rtl_reading: bool, text: &str, ranges: &[(usize, usize)]) {
    if label.right - label.left <= 0 || label.bottom - label.top <= 0 {
        return;
    }
    // UTF-16 copy of the text, cut where the list cuts it (cchTextMax),
    // with room for the "..." the list puts on a name too long to fit.
    let mut buf = [0u16; MAX_UNITS + 3];
    let mut n16 = 0;
    for (d, u) in buf[..MAX_UNITS].iter_mut().zip(text.encode_utf16()) {
        *d = u;
        n16 += 1;
    }
    if n16 == 0 {
        return;
    }

    let layout = unsafe { GetLayout(hdc) };
    let mirrored = layout != GDI_ERROR && layout & LAYOUT_RTL != 0;
    let align = unsafe { GetTextAlign(hdc) };
    let rtl = mirrored || rtl_reading || (align != GDI_ERROR && align & TA_RTLREADING != 0);
    let flags = SSA_GLYPHS | SSA_FALLBACK | if rtl { SSA_RTL } else { 0 };

    // Same inset the list uses before the label text (SM_CXEDGE, 2 px at
    // 96 dpi).
    let pad = unsafe { GetSystemMetrics(SM_CXEDGE) }.max(1);
    let start = label.left + pad;
    let avail = label.right - pad - start;
    if avail <= 0 {
        return;
    }

    let Some(mut ssa) = (unsafe { analyse(hdc, &buf[..n16], flags) }) else { return };
    // Number of UTF-16 units of the name that are actually shown.
    let mut shown = n16;
    if ssa.width() > avail {
        // Too long: like DrawText's DT_END_ELLIPSIS, the list keeps the
        // longest start of the name that fits with "..." after it. Laying
        // out exactly that string keeps the positions right even when the
        // cut splits a run of the other direction (an English name in the
        // right-to-left list).
        let dots = [b'.' as u16; 3];
        let mut ext: SIZE = unsafe { zeroed() };
        if unsafe { GetTextExtentPoint32W(hdc, dots.as_ptr(), 3, &mut ext) } == 0 {
            return;
        }
        let mut fit: i32 = 0;
        let mut all: SIZE = unsafe { zeroed() };
        let max = (avail - ext.cx).max(0);
        if unsafe { GetTextExtentExPointW(hdc, buf.as_ptr(), n16 as i32, max, &mut fit, null_mut(), &mut all) } == 0 {
            return;
        }
        shown = (fit.max(0) as usize).min(n16);
        if shown > 0 && (0xD800..0xDC00).contains(&buf[shown - 1]) {
            shown -= 1; // never split a surrogate pair
        }
        buf[shown..shown + 3].copy_from_slice(&dots);
        drop(ssa);
        ssa = match unsafe { analyse(hdc, &buf[..shown + 3], flags) } {
            Some(s) => s,
            None => return,
        };
    }
    let width = ssa.width();

    // The text is vertically centred (DT_VCENTER); the line sits at the
    // bottom of its cell, below the baseline and most descenders.
    let mut tm: TEXTMETRICW = unsafe { zeroed() };
    if unsafe { GetTextMetricsW(hdc, &mut tm) } == 0 {
        return;
    }
    let dpi = unsafe { GetDeviceCaps(hdc, LOGPIXELSX) }.max(96);
    let thick = ((2 * dpi + 48) / 96).max(2);
    let text_top = label.top + (label.bottom - label.top - tm.tmHeight) / 2;
    let bottom = (text_top + tm.tmHeight).min(label.bottom);
    let top = (bottom - thick).max(label.top);

    let brush = unsafe { CreateSolidBrush(GetSysColor(COLOR_HIGHLIGHT)) };
    if brush.is_null() {
        return;
    }

    // Walk the text once, mapping byte offsets to UTF-16 offsets, and
    // underline each matched character from its leading to its trailing
    // edge. Bidi may split one logical range into several visual pieces;
    // per-character rectangles handle that, and adjacent ones join up.
    let (mut b, mut u) = (0usize, 0usize);
    for ch in text.chars() {
        let len16 = ch.len_utf16();
        if u + len16 > shown {
            break;
        }
        if ranges.iter().any(|&(s, e)| b >= s && b < e) {
            let (mut x0, mut x1) = (0i32, 0i32);
            let ok = unsafe {
                SUCCEEDED(ScriptStringCPtoX(ssa.0, u as i32, 0, &mut x0))
                    && SUCCEEDED(ScriptStringCPtoX(ssa.0, (u + len16 - 1) as i32, 1, &mut x1))
            };
            if ok {
                let (v0, v1) = (x0.min(x1), x0.max(x1));
                let (l, r) = if mirrored { (start + width - v1, start + width - v0) } else { (start + v0, start + v1) };
                let (l, r) = (l.max(start), r.min(start + avail));
                if r > l {
                    let rc = RECT { left: l, top, right: r, bottom };
                    unsafe { FillRect(hdc, &rc, brush) };
                }
            }
        }
        b += ch.len_utf8();
        u += len16;
    }
    unsafe { DeleteObject(brush as HGDIOBJ) };
}

/// Shapes and lays out `text` with the font selected into `hdc`.
unsafe fn analyse(hdc: HDC, text: &[u16], flags: u32) -> Option<SsaGuard> {
    let mut ssa: SCRIPT_STRING_ANALYSIS = null_mut();
    let hr = unsafe {
        ScriptStringAnalyse(
            hdc,
            text.as_ptr() as *const _,
            text.len() as i32,
            (text.len() * 3 / 2 + 16) as i32,
            -1, // Unicode
            flags,
            0,
            null_mut(),
            null_mut(),
            null(),
            null_mut(),
            null(),
            &mut ssa,
        )
    };
    if !SUCCEEDED(hr) || ssa.is_null() {
        return None;
    }
    let ssa = SsaGuard(ssa);
    if unsafe { ScriptString_pSize(ssa.0) }.is_null() {
        return None;
    }
    Some(ssa)
}
