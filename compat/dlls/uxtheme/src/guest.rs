//! uxtheme.dll exports: no visual styles, buffered painting accepted,
//! dark-mode preferences from the registry.
use crate::{E_HANDLE, E_INVALIDARG, ELEMENT_NOT_FOUND, S_OK};
use core::sync::atomic::{AtomicU32, Ordering};
use door_windows_abi::widen;

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn RtlSetLastWin32Error(code: u32);
}
#[link(name = "advapi32", kind = "dylib")]
unsafe extern "system" {
    fn RegGetValueW(
        key: usize,
        subkey: usize,
        value: usize,
        flags: u32,
        kind: usize,
        data: usize,
        size: usize,
    ) -> u32;
}
const HKEY_CURRENT_USER: usize = 0x8000_0001u32 as i32 as isize as usize;
/// RRF_RT_REG_DWORD.
const RRF_RT_REG_DWORD: u32 = 0x10;

// ---- theme data ----------------------------------------------------------------------------
/// OpenThemeData(hwnd, classList): with visual styles off no class has
/// theme data.
#[unsafe(no_mangle)]
pub extern "system" fn OpenThemeData(_hwnd: usize, _classes: usize) -> usize {
    unsafe { RtlSetLastWin32Error(ELEMENT_NOT_FOUND) };
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn OpenThemeDataEx(hwnd: usize, classes: usize, _flags: u32) -> usize {
    OpenThemeData(hwnd, classes)
}
#[unsafe(no_mangle)]
pub extern "system" fn OpenThemeDataForDpi(hwnd: usize, classes: usize, _dpi: u32) -> usize {
    OpenThemeData(hwnd, classes)
}
#[unsafe(no_mangle)]
pub extern "system" fn CloseThemeData(theme: usize) -> u32 {
    if theme == 0 { E_HANDLE } else { S_OK }
}
#[unsafe(no_mangle)]
pub extern "system" fn IsThemeActive() -> i32 {
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn IsAppThemed() -> i32 {
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn IsCompositionActive() -> i32 {
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetThemeAppProperties() -> u32 {
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn SetThemeAppProperties(_flags: u32) {}
#[unsafe(no_mangle)]
pub extern "system" fn GetWindowTheme(_hwnd: usize) -> usize {
    0
}
/// SetWindowTheme / EnableThemeDialogTexture / SetWindowThemeAttribute:
/// accepted (nothing is themed).
#[unsafe(no_mangle)]
pub extern "system" fn SetWindowTheme(_hwnd: usize, _app: usize, _ids: usize) -> u32 {
    S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn EnableThemeDialogTexture(_hwnd: usize, _flags: u32) -> u32 {
    S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn SetWindowThemeAttribute(
    _hwnd: usize,
    _kind: u32,
    _attributes: usize,
    _size: u32,
) -> u32 {
    S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn EnableTheming(_enable: i32) -> u32 {
    S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn IsThemeDialogTextureEnabled(_hwnd: usize) -> i32 {
    0
}
/// Calls on a theme handle: none exist, so every handle is invalid.
macro_rules! no_theme {
    ($($name:ident($($arg:ident),*)),* $(,)?) => {$(
        #[unsafe(no_mangle)]
        pub extern "system" fn $name($($arg: usize),*) -> u32 {
            $(let _ = $arg;)*
            E_HANDLE
        }
    )*};
}
no_theme!(
    DrawThemeBackground(theme, dc, part, state, rect, clip),
    DrawThemeBackgroundEx(theme, dc, part, state, rect, options),
    DrawThemeEdge(theme, dc, part, state, rect, edge, flags, content),
    DrawThemeIcon(theme, dc, part, state, rect, list, index),
    DrawThemeText(theme, dc, part, state, text, count, flags, flags2, rect),
    DrawThemeTextEx(theme, dc, part, state, text, count, flags, rect, options),
    GetThemePartSize(theme, dc, part, state, rect, kind, size),
    GetThemeBackgroundContentRect(theme, dc, part, state, bounds, content),
    GetThemeBackgroundExtent(theme, dc, part, state, content, extent),
    GetThemeBackgroundRegion(theme, dc, part, state, rect, region),
    GetThemeColor(theme, part, state, property, color),
    GetThemeInt(theme, part, state, property, value),
    GetThemeMargins(theme, dc, part, state, property, rect, margins),
    GetThemeMetric(theme, dc, part, state, property, value),
    GetThemeFont(theme, dc, part, state, property, font),
    GetThemeBool(theme, part, state, property, value),
    GetThemeEnumValue(theme, part, state, property, value),
    GetThemePosition(theme, part, state, property, point),
    GetThemeRect(theme, part, state, property, rect),
    GetThemeString(theme, part, state, property, buffer, count),
    GetThemeTextExtent(theme, dc, part, state, text, count, flags, bounds, extent),
    GetThemeTextMetrics(theme, dc, part, state, metrics),
    GetThemeTransitionDuration(theme, part, from, to, property, duration),
    DrawThemeParentBackground(hwnd, dc, rect),
    DrawThemeParentBackgroundEx(hwnd, dc, flags, rect),
    GetThemeSysFont(theme, id, font),
    GetThemeSysInt(theme, id, value),
);
#[unsafe(no_mangle)]
pub extern "system" fn IsThemePartDefined(_theme: usize, _part: i32, _state: i32) -> i32 {
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn IsThemeBackgroundPartiallyTransparent(
    _theme: usize,
    _part: i32,
    _state: i32,
) -> i32 {
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn GetThemeSysBool(_theme: usize, _id: i32) -> i32 {
    0
}
/// GetCurrentThemeName: no visual style file is loaded.
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentThemeName(
    name: usize,
    count: i32,
    color: usize,
    color_count: i32,
    size: usize,
    size_count: i32,
) -> u32 {
    for (buffer, units) in [(name, count), (color, color_count), (size, size_count)] {
        let buffer = widen(buffer);
        if buffer != 0 && units > 0 {
            unsafe { (buffer as *mut u16).write_unaligned(0) };
        }
    }
    ELEMENT_NOT_FOUND
}

// ---- buffered painting --------------------------------------------------------------------
/// Buffered painting needs a paint buffer per call; with none offered,
/// callers paint directly (BeginBufferedPaint's documented failure).
#[unsafe(no_mangle)]
pub extern "system" fn BufferedPaintInit() -> u32 {
    S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn BufferedPaintUnInit() -> u32 {
    S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn BeginBufferedPaint(
    _dc: usize,
    _rect: usize,
    _format: u32,
    _params: usize,
    target: usize,
) -> usize {
    let target = widen(target);
    if target != 0 {
        unsafe { (target as *mut usize).write_unaligned(0) };
    }
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn EndBufferedPaint(paint: usize, _update: i32) -> u32 {
    if paint == 0 { E_INVALIDARG } else { S_OK }
}
#[unsafe(no_mangle)]
pub extern "system" fn BufferedPaintSetAlpha(_paint: usize, _rect: usize, _alpha: u8) -> u32 {
    E_INVALIDARG
}
#[unsafe(no_mangle)]
pub extern "system" fn BufferedPaintClear(_paint: usize, _rect: usize) -> u32 {
    E_INVALIDARG
}
#[unsafe(no_mangle)]
pub extern "system" fn BufferedPaintRenderAnimation(_hwnd: usize, _dc: usize) -> i32 {
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn BufferedPaintStopAllAnimations(_hwnd: usize) -> u32 {
    S_OK
}

// ---- dark mode (ordinal exports) ------------------------------------------------------------
/// A Personalize value (1 = light) as the registry holds it.
fn light(value: &str) -> bool {
    let key: [u16; 64] = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    let name: [u16; 64] = wide(value);
    let mut data = 1u32;
    let mut size = 4u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr() as usize,
            name.as_ptr() as usize,
            RRF_RT_REG_DWORD,
            0,
            &mut data as *mut u32 as usize,
            &mut size as *mut u32 as usize,
        )
    };
    status != 0 || data != 0
}
fn wide<const N: usize>(text: &str) -> [u16; N] {
    let mut out = [0u16; N];
    for (slot, unit) in out.iter_mut().zip(text.encode_utf16()) {
        *slot = unit;
    }
    out
}
/// The PreferredAppMode a program set (Default 0, AllowDark 1, ForceDark
/// 2, ForceLight 3).
static APP_MODE: AtomicU32 = AtomicU32::new(0);
/// #132 ShouldAppsUseDarkMode.
#[unsafe(no_mangle)]
pub extern "system" fn ShouldAppsUseDarkMode() -> i32 {
    match APP_MODE.load(Ordering::Relaxed) {
        2 => 1,
        3 => 0,
        _ => i32::from(!light("AppsUseLightTheme")),
    }
}
/// #138 ShouldSystemUseDarkMode.
#[unsafe(no_mangle)]
pub extern "system" fn ShouldSystemUseDarkMode() -> i32 {
    i32::from(!light("SystemUsesLightTheme"))
}
/// #133 AllowDarkModeForWindow(hwnd, allow): recorded as allowed.
#[unsafe(no_mangle)]
pub extern "system" fn AllowDarkModeForWindow(hwnd: usize, _allow: i32) -> i32 {
    i32::from(hwnd != 0)
}
/// #139 IsDarkModeAllowedForWindow.
#[unsafe(no_mangle)]
pub extern "system" fn IsDarkModeAllowedForWindow(hwnd: usize) -> i32 {
    i32::from(hwnd != 0 && ShouldAppsUseDarkMode() != 0)
}
/// #135 SetPreferredAppMode(mode): the previous mode.
#[unsafe(no_mangle)]
pub extern "system" fn SetPreferredAppMode(mode: u32) -> u32 {
    APP_MODE.swap(mode.min(4), Ordering::Relaxed)
}
/// #136 FlushMenuThemes, #104/#137 RefreshImmersiveColorPolicyState:
/// menus and colours are read when drawn.
#[unsafe(no_mangle)]
pub extern "system" fn FlushMenuThemes() {}
#[unsafe(no_mangle)]
pub extern "system" fn RefreshImmersiveColorPolicyState() {}
#[unsafe(no_mangle)]
pub extern "system" fn RefreshImmersiveColorPolicyStateOrdinal() {}
/// #106 GetIsImmersiveColorUsingHighContrast: high contrast is off.
#[unsafe(no_mangle)]
pub extern "system" fn GetIsImmersiveColorUsingHighContrast(_mode: u32) -> i32 {
    0
}
/// HitTestThemeBackground (a POINT by value: eight bytes on either stack).
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn HitTestThemeBackground(
    _theme: usize,
    _dc: usize,
    _part: i32,
    _state: i32,
    _options: u32,
    _rect: usize,
    _region: usize,
    _point: u64,
    _code: usize,
) -> u32 {
    E_HANDLE
}
