//! dwmapi.dll exports.
use crate::{COLORIZATION, E_HANDLE, E_INVALIDARG, REFRESH_HZ, S_OK};
use door_windows_abi::widen;

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

#[link(name = "user32", kind = "dylib")]
unsafe extern "system" {
    fn GetWindowRect(hwnd: usize, rect: usize) -> i32;
    fn IsWindow(hwnd: usize) -> i32;
}
#[link(name = "kernel32", kind = "dylib")]
unsafe extern "system" {
    fn QueryPerformanceCounter(counter: usize) -> i32;
    fn QueryPerformanceFrequency(frequency: usize) -> i32;
    fn Sleep(milliseconds: u32);
}

fn put<T: Copy>(at: usize, value: T) {
    let at = widen(at);
    if at != 0 {
        unsafe { (at as *mut T).write_unaligned(value) };
    }
}
fn counter() -> (u64, u64) {
    let (mut frequency, mut now) = (0u64, 0u64);
    unsafe {
        QueryPerformanceFrequency(&mut frequency as *mut u64 as usize);
        QueryPerformanceCounter(&mut now as *mut u64 as usize);
    }
    (frequency, now)
}

#[unsafe(no_mangle)]
pub extern "system" fn DwmIsCompositionEnabled(enabled: usize) -> u32 {
    if enabled == 0 {
        return E_INVALIDARG;
    }
    put(enabled, 1i32);
    S_OK
}
/// DwmEnableComposition: composition cannot be turned off.
#[unsafe(no_mangle)]
pub extern "system" fn DwmEnableComposition(_action: u32) -> u32 {
    S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn DwmEnableMMCSS(_enable: i32) -> u32 {
    S_OK
}
/// DwmFlush: returns at the next composition frame.
#[unsafe(no_mangle)]
pub extern "system" fn DwmFlush() -> u32 {
    let (frequency, now) = counter();
    let period = (frequency / REFRESH_HZ).max(1);
    let left = period - now % period;
    let milliseconds = (left * 1000 / frequency.max(1)) as u32;
    unsafe { Sleep(milliseconds.max(1)) };
    S_OK
}
/// DwmGetCompositionTimingInfo(hwnd, DWM_TIMING_INFO*): cbSize must be
/// the structure's.
#[unsafe(no_mangle)]
pub extern "system" fn DwmGetCompositionTimingInfo(_hwnd: usize, info: usize) -> u32 {
    let info = widen(info);
    if info == 0 {
        return E_INVALIDARG;
    }
    let size = unsafe { (info as *const u32).read_unaligned() } as usize;
    if !(64..=4096).contains(&size) {
        return E_INVALIDARG;
    }
    let out = unsafe { core::slice::from_raw_parts_mut(info as *mut u8, size) };
    let (frequency, now) = counter();
    if crate::timing(frequency, now, out) {
        S_OK
    } else {
        E_INVALIDARG
    }
}
/// DwmGetWindowAttribute: DWMWA_NCRENDERING_ENABLED (1), the extended
/// frame bounds (9: the window rectangle; Doors draws no invisible resize
/// borders outside it), DWMWA_CLOAKED (14: never), the caption button
/// bounds (5) and DWMWA_VISIBLE_FRAME_BORDER_THICKNESS (37: 1 pixel).
#[unsafe(no_mangle)]
pub extern "system" fn DwmGetWindowAttribute(
    hwnd: usize,
    attribute: u32,
    out: usize,
    size: u32,
) -> u32 {
    if out == 0 {
        return E_INVALIDARG;
    }
    if unsafe { IsWindow(hwnd) } == 0 {
        return E_HANDLE;
    }
    match (attribute, size) {
        (1, 4..) => put(out, 1i32),
        (14, 4..) => put(out, 0u32),
        (37, 4..) => put(out, 1u32),
        (5 | 9, 16..) => {
            if unsafe { GetWindowRect(hwnd, out) } == 0 {
                return E_HANDLE;
            }
        }
        _ => return E_INVALIDARG,
    }
    S_OK
}
/// DwmSetWindowAttribute: dark title bars, corner preferences, backdrops,
/// colours and transitions are accepted (Doors draws window frames itself).
#[unsafe(no_mangle)]
pub extern "system" fn DwmSetWindowAttribute(
    hwnd: usize,
    _attribute: u32,
    value: usize,
    _size: u32,
) -> u32 {
    if value == 0 {
        return E_INVALIDARG;
    }
    if unsafe { IsWindow(hwnd) } == 0 {
        return E_HANDLE;
    }
    S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn DwmExtendFrameIntoClientArea(hwnd: usize, margins: usize) -> u32 {
    if margins == 0 {
        return E_INVALIDARG;
    }
    if unsafe { IsWindow(hwnd) } == 0 {
        return E_HANDLE;
    }
    S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn DwmEnableBlurBehindWindow(hwnd: usize, blur: usize) -> u32 {
    if blur == 0 {
        return E_INVALIDARG;
    }
    if unsafe { IsWindow(hwnd) } == 0 {
        return E_HANDLE;
    }
    S_OK
}
/// DwmDefWindowProc: no caption-button hit testing of its own.
#[unsafe(no_mangle)]
pub extern "system" fn DwmDefWindowProc(
    _hwnd: usize,
    _message: u32,
    _wparam: usize,
    _lparam: usize,
    result: usize,
) -> i32 {
    put(result, 0usize);
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn DwmGetColorizationColor(color: usize, opaque: usize) -> u32 {
    if color == 0 || opaque == 0 {
        return E_INVALIDARG;
    }
    put(color, COLORIZATION);
    put(opaque, 0i32);
    S_OK
}
/// Taskbar thumbnails and live previews come from Doors' compositor.
#[unsafe(no_mangle)]
pub extern "system" fn DwmSetIconicThumbnail(_hwnd: usize, _bitmap: usize, _flags: u32) -> u32 {
    S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn DwmSetIconicLivePreviewBitmap(
    _hwnd: usize,
    _bitmap: usize,
    _client: usize,
    _flags: u32,
) -> u32 {
    S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn DwmInvalidateIconicBitmaps(_hwnd: usize) -> u32 {
    S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn DwmRegisterThumbnail(
    _destination: usize,
    _source: usize,
    thumbnail: usize,
) -> u32 {
    put(thumbnail, 0usize);
    E_INVALIDARG
}
#[unsafe(no_mangle)]
pub extern "system" fn DwmUnregisterThumbnail(_thumbnail: usize) -> u32 {
    E_HANDLE
}
#[unsafe(no_mangle)]
pub extern "system" fn DwmUpdateThumbnailProperties(_thumbnail: usize, _properties: usize) -> u32 {
    E_HANDLE
}
#[unsafe(no_mangle)]
pub extern "system" fn DwmQueryThumbnailSourceSize(_thumbnail: usize, _size: usize) -> u32 {
    E_HANDLE
}
