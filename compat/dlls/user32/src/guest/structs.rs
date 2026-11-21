//! The Windows structures user32 reads and writes that hold pointers,
//! handles or pointer-sized integers, described by field so the x64 and
//! the x86 build (P17) each lay them out as their callers do
//! (`door_windows_abi::layout`). Field indices name the members (every
//! member is listed, used or not, as the layout's documentation).
#![allow(dead_code)]
use door_windows_abi::layout::{self, Field, Field::*};

/// WNDCLASSEXW: cbSize, style, lpfnWndProc, cbClsExtra, cbWndExtra,
/// hInstance, hIcon, hCursor, hbrBackground, lpszMenuName,
/// lpszClassName, hIconSm. WNDCLASSW is the same without cbSize and
/// hIconSm.
pub(crate) const WNDCLASSEXW: [Field; 12] = [
    U32, U32, Pointer, U32, U32, Pointer, Pointer, Pointer, Pointer, Pointer, Pointer, Pointer,
];
pub(crate) const WNDCLASSW: [Field; 10] = [
    U32, Pointer, U32, U32, Pointer, Pointer, Pointer, Pointer, Pointer, Pointer,
];
pub(crate) mod class {
    pub const SIZE: usize = 0;
    pub const STYLE: usize = 1;
    pub const PROCEDURE: usize = 2;
    pub const CLASS_EXTRA: usize = 3;
    pub const WINDOW_EXTRA: usize = 4;
    pub const INSTANCE: usize = 5;
    pub const ICON: usize = 6;
    pub const CURSOR: usize = 7;
    pub const BACKGROUND: usize = 8;
    pub const MENU_NAME: usize = 9;
    pub const CLASS_NAME: usize = 10;
    pub const SMALL_ICON: usize = 11;
}
/// CREATESTRUCTW: lpCreateParams, hInstance, hMenu, hwndParent, cy, cx,
/// y, x, style, lpszName, lpszClass, dwExStyle.
pub(crate) const CREATESTRUCTW: [Field; 12] = [
    Pointer, Pointer, Pointer, Pointer, U32, U32, U32, U32, U32, Pointer, Pointer, U32,
];
pub(crate) mod create {
    pub const PARAMS: usize = 0;
    pub const INSTANCE: usize = 1;
    pub const MENU: usize = 2;
    pub const PARENT: usize = 3;
    pub const CY: usize = 4;
    pub const CX: usize = 5;
    pub const Y: usize = 6;
    pub const X: usize = 7;
    pub const STYLE: usize = 8;
    pub const NAME: usize = 9;
    pub const CLASS: usize = 10;
    pub const EX_STYLE: usize = 11;
}
/// MSG: hwnd, message, wParam, lParam, time, pt.x, pt.y.
pub(crate) const MSG: [Field; 7] = [Pointer, U32, Pointer, Pointer, U32, U32, U32];
pub(crate) mod msg {
    pub const HWND: usize = 0;
    pub const MESSAGE: usize = 1;
    pub const WPARAM: usize = 2;
    pub const LPARAM: usize = 3;
    pub const TIME: usize = 4;
    pub const X: usize = 5;
    pub const Y: usize = 6;
}
/// WINDOWPOS: hwnd, hwndInsertAfter, x, y, cx, cy, flags.
pub(crate) const WINDOWPOS: [Field; 7] = [Pointer, Pointer, U32, U32, U32, U32, U32];
pub(crate) mod windowpos {
    pub const HWND: usize = 0;
    pub const X: usize = 2;
    pub const Y: usize = 3;
    pub const CX: usize = 4;
    pub const CY: usize = 5;
    pub const FLAGS: usize = 6;
}
/// PAINTSTRUCT: hdc, fErase, rcPaint, fRestore, fIncUpdate,
/// rgbReserved[32].
pub(crate) const PAINTSTRUCT: [Field; 9] = [Pointer, U32, U32, U32, U32, U32, U32, U32, Bytes(32)];
pub(crate) mod paint {
    pub const HDC: usize = 0;
    pub const ERASE: usize = 1;
    pub const LEFT: usize = 2;
}
/// NCCALCSIZE_PARAMS: rgrc[3] (RECTs), lppos.
pub(crate) const NCCALCSIZE_PARAMS: [Field; 2] = [Bytes(48), Pointer];
/// COPYDATASTRUCT: dwData, cbData, lpData.
pub(crate) const COPYDATASTRUCT: [Field; 3] = [Pointer, U32, Pointer];
/// ICONINFO: fIcon, xHotspot, yHotspot, hbmMask, hbmColor.
pub(crate) const ICONINFO: [Field; 5] = [U32, U32, U32, Pointer, Pointer];

pub(crate) const WNDCLASSEXW_BYTES: usize = layout::size(&WNDCLASSEXW);
pub(crate) const WNDCLASSW_BYTES: usize = layout::size(&WNDCLASSW);
pub(crate) const CREATESTRUCTW_BYTES: usize = layout::size(&CREATESTRUCTW);
pub(crate) const MSG_BYTES: usize = layout::size(&MSG);
pub(crate) const WINDOWPOS_BYTES: usize = layout::size(&WINDOWPOS);
pub(crate) const PAINTSTRUCT_BYTES: usize = layout::size(&PAINTSTRUCT);
pub(crate) const NCCALCSIZE_PARAMS_BYTES: usize = layout::size(&NCCALCSIZE_PARAMS);
pub(crate) const COPYDATASTRUCT_BYTES: usize = layout::size(&COPYDATASTRUCT);
pub(crate) const ICONINFO_BYTES: usize = layout::size(&ICONINFO);

/// A field of `bytes` laid out as `fields`, as a 64-bit value.
pub(crate) fn get(bytes: &[u8], fields: &[Field], index: usize) -> u64 {
    layout::get(bytes, fields, index)
}
/// A 32-bit signed field (coordinates, sizes).
pub(crate) fn get_i32(bytes: &[u8], fields: &[Field], index: usize) -> i32 {
    layout::get(bytes, fields, index) as u32 as i32
}
pub(crate) fn set(bytes: &mut [u8], fields: &[Field], index: usize, value: u64) {
    layout::set(bytes, fields, index, value);
}
pub(crate) fn set_i32(bytes: &mut [u8], fields: &[Field], index: usize, value: i32) {
    layout::set(bytes, fields, index, u64::from(value as u32));
}
/// The width of an extra-bytes slot GetWindowLongPtr/GetClassLongPtr (and,
/// in the x86 build, GetWindowLong) reads: a pointer.
pub(crate) const LONG_PTR: usize = layout::POINTER;
