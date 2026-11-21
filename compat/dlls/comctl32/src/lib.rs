#![no_std]
//! Original comctl32 subset: InitCommonControlsEx, the status bar
//! (`msctls_statusbar32`) with parts, text and NM_CLICK notifications,
//! implemented as an ordinary window class over user32 and gdi32, and image
//! lists (ImageList_*) kept as icons. Exported PE entry points exist only in
//! guest builds.
//!
//! The same sources build the x64 and the x86 DLL (P17): Windows-facing
//! exports take pointer-width parameters, widened at entry; a pointer or
//! handle result is narrowed from the body's 64-bit value (an immediately
//! called closure, so the body's own `return`s keep their meaning).
#![allow(clippy::redundant_closure_call)]
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
extern crate alloc;

/// `ICC_BAR_CLASSES`: the family of the status bar, the one common control
/// this subset implements.
pub const ICC_BAR_CLASSES: u32 = 0x4;
/// `ICC_WIN95_CLASSES`: list view, header, tree view, tab, up-down, progress,
/// hot key, animate, toolbar, status bar, trackbar and tooltips.
pub const ICC_WIN95_CLASSES: u32 = 0xff;
/// ICC_* families `InitCommonControlsEx` can register.
pub const SUPPORTED_CLASSES: u32 = ICC_BAR_CLASSES;

/// Does `InitCommonControlsEx` succeed for this `dwICC`? Windows returns
/// TRUE only when every requested class was registered, so any family
/// outside `SUPPORTED_CLASSES` makes the whole request fail.
pub const fn accepts(classes: u32) -> bool {
    classes & !SUPPORTED_CLASSES == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_fully_supported_masks_succeed() {
        // The F2 editor fixture requests exactly ICC_BAR_CLASSES.
        assert!(accepts(ICC_BAR_CLASSES));
        assert!(accepts(0));
        assert!(!accepts(ICC_WIN95_CLASSES));
        assert!(!accepts(ICC_BAR_CLASSES | 0x1));
        assert!(!accepts(0x8000));
    }
}
