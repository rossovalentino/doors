#![no_std]
//! Portable Win32 GUI semantics for the original Door `user32`, `gdi32`,
//! `comdlg32` and `comctl32`. Everything here is ordinary safe Rust over owned
//! data: application pointers never reach this crate (the DLLs capture them
//! through the checked-copy boundary first), and every collection is bounded.
//! The DLLs keep only the ABI, process/thread binding and service adapters.
extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod dib;
pub mod dlgnav;
pub mod edit;
pub mod gdi;
pub mod geom;
pub mod handles;
pub mod icon;
pub mod keyboard;
pub mod menu;
pub mod metrics;
pub mod msg;
pub mod ofn;
pub mod parameters;
pub mod present;
pub mod queue;
pub mod raster;
pub mod region;
pub mod resource;
pub mod surface;
pub mod text;
pub mod units;
pub mod window;

pub use geom::{ColorRef, Point, Rect, Size};

/// Failure classes shared by the portable core. The DLLs translate them into
/// the documented Win32 last-error values of the calling API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A documented argument constraint was violated (`ERROR_INVALID_PARAMETER`).
    InvalidParameter,
    /// A bounded table, region or allocation quota is exhausted
    /// (`ERROR_NOT_ENOUGH_MEMORY` / `ERROR_NO_SYSTEM_RESOURCES`).
    Quota,
    /// A handle was stale, of the wrong type or never issued (`ERROR_INVALID_HANDLE`).
    InvalidHandle,
    /// Structured input (template, header, resource) is malformed or truncated.
    Malformed,
    /// A defined but intentionally unsupported variant (`ERROR_NOT_SUPPORTED`).
    Unsupported,
}
pub type Result<T> = core::result::Result<T, Error>;

/// Win32 last-error values used by the translators.
pub mod win32_error {
    pub const SUCCESS: u32 = 0;
    pub const FILE_NOT_FOUND: u32 = 2;
    pub const ACCESS_DENIED: u32 = 5;
    pub const INVALID_HANDLE: u32 = 6;
    pub const NOT_ENOUGH_MEMORY: u32 = 8;
    pub const INVALID_DATA: u32 = 13;
    pub const OUTOFMEMORY: u32 = 14;
    pub const NOT_SUPPORTED: u32 = 50;
    pub const INVALID_PARAMETER: u32 = 87;
    pub const INSUFFICIENT_BUFFER: u32 = 122;
    pub const INVALID_FLAGS: u32 = 1004;
    pub const NO_SYSTEM_RESOURCES: u32 = 1450;
    pub const TIMEOUT: u32 = 1460;
    pub const INVALID_WINDOW_HANDLE: u32 = 1400;
    pub const INVALID_MENU_HANDLE: u32 = 1401;
    pub const INVALID_CURSOR_HANDLE: u32 = 1402;
    pub const INVALID_ACCEL_HANDLE: u32 = 1403;
    pub const INVALID_HOOK_HANDLE: u32 = 1404;
    pub const NO_WILDCARD_CHARACTERS: u32 = 1417;
    pub const CLASS_ALREADY_EXISTS: u32 = 1410;
    pub const CLASS_DOES_NOT_EXIST: u32 = 1411;
    pub const CLASS_HAS_WINDOWS: u32 = 1412;
    pub const INVALID_INDEX: u32 = 1413;
    pub const INVALID_ICON_HANDLE: u32 = 1414;
    pub const PRIVATE_DIALOG_INDEX: u32 = 1415;
    pub const LISTBOX_ID_NOT_FOUND: u32 = 1416;
    pub const NOT_CHILD_WINDOW: u32 = 1442;
    pub const INVALID_GW_COMMAND: u32 = 1443;
    pub const INVALID_THREAD_ID: u32 = 1444;
    pub const NON_MDICHILD_WINDOW: u32 = 1445;
    pub const POPUP_ALREADY_ACTIVE: u32 = 1446;
    pub const NO_SCROLLBARS: u32 = 1447;
    pub const INVALID_SCROLLBAR_RANGE: u32 = 1448;
    pub const INVALID_SHOWWIN_COMMAND: u32 = 1449;
    pub const WINDOW_OF_OTHER_THREAD: u32 = 1408;
    pub const CANNOT_FIND_WND_CLASS: u32 = 1407;
    pub const TLW_WITH_WSCHILD: u32 = 1406;
    pub const HOTKEY_ALREADY_REGISTERED: u32 = 1409;
    pub const RESOURCE_DATA_NOT_FOUND: u32 = 1812;
    pub const RESOURCE_TYPE_NOT_FOUND: u32 = 1813;
    pub const RESOURCE_NAME_NOT_FOUND: u32 = 1814;
    pub const CLIPBOARD_NOT_OPEN: u32 = 1418;
    pub const MESSAGE_SYNC_ONLY: u32 = 1159;
    pub const NOT_ENOUGH_QUOTA: u32 = 1816;
    pub const INVALID_PIXEL_FORMAT: u32 = 2000;
}

impl Error {
    pub const fn win32(self) -> u32 {
        match self {
            Self::InvalidParameter => win32_error::INVALID_PARAMETER,
            Self::Quota => win32_error::NOT_ENOUGH_MEMORY,
            Self::InvalidHandle => win32_error::INVALID_HANDLE,
            Self::Malformed => win32_error::INVALID_DATA,
            Self::Unsupported => win32_error::NOT_SUPPORTED,
        }
    }
}

/// Fallible bounded vector growth: the portable core never aborts on a
/// rejected allocation; it reports `Error::Quota` instead.
pub fn try_push<T>(v: &mut alloc::vec::Vec<T>, value: T, limit: usize) -> Result<()> {
    if v.len() >= limit {
        return Err(Error::Quota);
    }
    v.try_reserve(1).map_err(|_| Error::Quota)?;
    v.push(value);
    Ok(())
}

/// Zero-filled fallible allocation.
pub fn try_zeroed<T: Copy + Default>(count: usize) -> Result<alloc::vec::Vec<T>> {
    let mut v = alloc::vec::Vec::new();
    v.try_reserve_exact(count).map_err(|_| Error::Quota)?;
    v.resize(count, T::default());
    Ok(v)
}
