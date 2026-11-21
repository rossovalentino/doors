//! DPI awareness (P18: Chromium's display code). Doors draws Windows
//! programs at 96 DPI (gdi32's LOGPIXELSX), so every monitor, window and
//! the system report 96 and a 100% scale factor; the awareness a program
//! declares is kept and reported back as Windows does (set once per
//! process). The shcore scaling contract (`api-ms-win-shcore-scaling`:
//! GetDpiForMonitor, GetScaleFactorForMonitor, Set/GetProcessDpiAwareness)
//! is hosted here.
use super::{hwnd, system::MONITOR};
use core::sync::atomic::{AtomicI32, Ordering};
use door_windows_abi::widen;
use door_windows_gui::win32_error as e;
use door_windows_gui_guest as g;

const DPI: u32 = 96;
const S_OK: u32 = 0;
const E_INVALIDARG: u32 = 0x8007_0057;
const E_ACCESSDENIED: u32 = 0x8007_0005;

/// The process's DPI_AWARENESS_CONTEXT (-1 unaware .. -5 GDI-scaled), 0
/// while the program has declared none (unaware).
static PROCESS: AtomicI32 = AtomicI32::new(0);
/// Contexts are the small negative handles Windows uses.
fn context_awareness(context: isize) -> Option<i32> {
    match context {
        -1 | -5 => Some(0),
        -2 => Some(1),
        -3 | -4 => Some(2),
        _ => None,
    }
}
fn process_context() -> isize {
    match PROCESS.load(Ordering::Relaxed) {
        0 => -1,
        c => c as isize,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetDpiForSystem() -> u32 {
    DPI
}
#[unsafe(no_mangle)]
pub extern "system" fn GetDpiForWindow(window: usize) -> u32 {
    if hwnd(widen(window)).is_none() {
        g::set_last_error(e::INVALID_WINDOW_HANDLE);
        return 0;
    }
    DPI
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSystemDpiForProcess(_process: usize) -> u32 {
    DPI
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSystemMetricsForDpi(index: i32, dpi: u32) -> i32 {
    let value = super::misc::GetSystemMetrics(index);
    if dpi == 0 || dpi == DPI {
        value
    } else {
        (i64::from(value) * i64::from(dpi) / i64::from(DPI)) as i32
    }
}
/// GetDpiForMonitor (shcore): MDT_EFFECTIVE_DPI 0, MDT_ANGULAR_DPI 1,
/// MDT_RAW_DPI 2.
#[unsafe(no_mangle)]
pub extern "system" fn GetDpiForMonitor(monitor: usize, kind: u32, x: usize, y: usize) -> u32 {
    if widen(monitor) != MONITOR || kind > 2 || x == 0 || y == 0 {
        return E_INVALIDARG;
    }
    g::write_u32(widen(x), DPI);
    g::write_u32(widen(y), DPI);
    S_OK
}
/// GetScaleFactorForMonitor (shcore): SCALE_100_PERCENT.
#[unsafe(no_mangle)]
pub extern "system" fn GetScaleFactorForMonitor(monitor: usize, scale: usize) -> u32 {
    if widen(monitor) != MONITOR || scale == 0 {
        return E_INVALIDARG;
    }
    g::write_u32(widen(scale), 100);
    S_OK
}
/// SetProcessDpiAwareness (shcore): PROCESS_DPI_UNAWARE 0,
/// PROCESS_SYSTEM_DPI_AWARE 1, PROCESS_PER_MONITOR_DPI_AWARE 2; once.
#[unsafe(no_mangle)]
pub extern "system" fn SetProcessDpiAwareness(value: u32) -> u32 {
    let context = match value {
        0 => -1,
        1 => -2,
        2 => -3,
        _ => return E_INVALIDARG,
    };
    match PROCESS.compare_exchange(0, context, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => S_OK,
        Err(_) => E_ACCESSDENIED,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetProcessDpiAwareness(_process: usize, out: usize) -> u32 {
    if out == 0 {
        return E_INVALIDARG;
    }
    let awareness = context_awareness(process_context()).unwrap_or(0);
    g::write_u32(widen(out), awareness as u32);
    S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn SetProcessDpiAwarenessContext(context: isize) -> i32 {
    if context_awareness(context).is_none() {
        g::set_last_error(e::INVALID_PARAMETER);
        return 0;
    }
    match PROCESS.compare_exchange(0, context as i32, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => 1,
        Err(_) => {
            g::set_last_error(e::ACCESS_DENIED);
            0
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetDpiAwarenessContextForProcess(_process: usize) -> isize {
    process_context()
}
/// A thread's context follows the process's (per-thread contexts are not
/// kept); setting one returns the previous.
#[unsafe(no_mangle)]
pub extern "system" fn GetThreadDpiAwarenessContext() -> isize {
    process_context()
}
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadDpiAwarenessContext(context: isize) -> isize {
    if context_awareness(context).is_none() {
        g::set_last_error(e::INVALID_PARAMETER);
        return 0;
    }
    process_context()
}
#[unsafe(no_mangle)]
pub extern "system" fn GetWindowDpiAwarenessContext(window: usize) -> isize {
    if hwnd(widen(window)).is_none() {
        g::set_last_error(e::INVALID_WINDOW_HANDLE);
        return 0;
    }
    process_context()
}
#[unsafe(no_mangle)]
pub extern "system" fn GetAwarenessFromDpiAwarenessContext(context: isize) -> i32 {
    // DPI_AWARENESS_INVALID -1.
    context_awareness(context).unwrap_or(-1)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetDpiFromDpiAwarenessContext(context: isize) -> u32 {
    if context_awareness(context).is_some() {
        DPI
    } else {
        0
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn AreDpiAwarenessContextsEqual(a: isize, b: isize) -> i32 {
    i32::from(context_awareness(a).is_some() && a == b)
}
#[unsafe(no_mangle)]
pub extern "system" fn IsValidDpiAwarenessContext(context: isize) -> i32 {
    i32::from(context_awareness(context).is_some())
}
#[unsafe(no_mangle)]
pub extern "system" fn IsProcessDPIAware() -> i32 {
    i32::from(context_awareness(process_context()).unwrap_or(0) != 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn EnableNonClientDpiScaling(window: usize) -> i32 {
    i32::from(hwnd(widen(window)).is_some())
}
#[unsafe(no_mangle)]
pub extern "system" fn LogicalToPhysicalPointForPerMonitorDPI(window: usize, _point: usize) -> i32 {
    i32::from(hwnd(widen(window)).is_some())
}
#[unsafe(no_mangle)]
pub extern "system" fn PhysicalToLogicalPointForPerMonitorDPI(window: usize, _point: usize) -> i32 {
    i32::from(hwnd(widen(window)).is_some())
}
/// SetProcessDPIAware: system-aware, unless the process declared already.
pub(super) fn declare_system_aware() {
    let _ = PROCESS.compare_exchange(0, -2, Ordering::AcqRel, Ordering::Acquire);
}
