#![no_std]
//! Original uxtheme.dll. Doors draws its own Fluent controls, so programs
//! see Windows with visual styles off: `IsThemeActive` and `IsAppThemed`
//! are FALSE and `OpenThemeData` finds no theme, so a program draws its
//! classic controls (as Chromium's native theme and the common controls
//! do). Buffered painting and window themes are accepted. The undocumented
//! dark-mode ordinals (104, 106, 132-139) report the user's choice in
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize`
//! (AppsUseLightTheme, SystemUsesLightTheme), as Windows does.
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

pub const S_OK: u32 = 0;
pub const E_HANDLE: u32 = 0x8007_0006;
pub const E_INVALIDARG: u32 = 0x8007_0057;
/// ERROR_NOT_FOUND as an HRESULT: no theme data for a class.
pub const ELEMENT_NOT_FOUND: u32 = 0x8007_0490;
/// The dark-mode exports Windows has only by ordinal.
pub const ORDINALS: [(&str, u16); 10] = [
    ("DrawThemeBackgroundEx", 47),
    ("RefreshImmersiveColorPolicyState", 104),
    ("GetIsImmersiveColorUsingHighContrast", 106),
    ("ShouldAppsUseDarkMode", 132),
    ("AllowDarkModeForWindow", 133),
    ("SetPreferredAppMode", 135),
    ("FlushMenuThemes", 136),
    ("RefreshImmersiveColorPolicyStateOrdinal", 137),
    ("ShouldSystemUseDarkMode", 138),
    ("IsDarkModeAllowedForWindow", 139),
];
