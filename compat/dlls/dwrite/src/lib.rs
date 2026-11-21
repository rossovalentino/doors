#![no_std]
//! Original DirectWrite (dwrite.dll), the subset Skia and Chromium draw
//! text with: IDWriteFactory through IDWriteFactory2, the system font
//! collection (the Windows fonts Doors ships, read through gdi32's shared
//! mappings), custom font files, loaders and collections, font families,
//! fonts and faces (metrics, glyph indices and metrics, OpenType tables,
//! outlines), glyph-run analysis (aliased, grayscale and ClearType-shaped
//! textures) and system font fallback. Text layout and text analysis
//! (IDWriteTextLayout, IDWriteTextAnalyzer) are not provided: Chromium
//! shapes with HarfBuzz. Rendering is software; GPU interop comes with
//! P19.
//!
//! The same sources build the x64 and the x86 DLL.
extern crate alloc;

pub mod fallback;
pub mod font;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
pub mod raster;

pub const S_OK: u32 = 0;
pub const E_NOTIMPL: u32 = 0x8000_4001;
pub const E_NOINTERFACE: u32 = 0x8000_4002;
pub const E_POINTER: u32 = 0x8000_4003;
pub const E_FAIL: u32 = 0x8000_4005;
pub const E_INVALIDARG: u32 = 0x8007_0057;
pub const E_OUTOFMEMORY: u32 = 0x8007_000E;
pub const E_NOT_SUFFICIENT_BUFFER: u32 = 0x8007_007A;
pub const DWRITE_E_FILEFORMAT: u32 = 0x8898_5000;
pub const DWRITE_E_NOFONT: u32 = 0x8898_5002;
pub const DWRITE_E_FILENOTFOUND: u32 = 0x8898_5003;
pub const DWRITE_E_ALREADYREGISTERED: u32 = 0x8898_5006;
pub const DWRITE_E_NOCOLOR: u32 = 0x8898_500C;

/// Font family names Windows programs ask for that Doors does not ship,
/// and the shipped family each means (as GDI's FontSubstitutes do).
pub const SUBSTITUTES: [(&str, &str); 14] = [
    ("Arial", "Segoe UI"),
    ("Helvetica", "Segoe UI"),
    ("Tahoma", "Segoe UI"),
    ("Verdana", "Segoe UI"),
    ("Calibri", "Segoe UI"),
    ("Microsoft Sans Serif", "Segoe UI"),
    ("MS Shell Dlg", "Segoe UI"),
    ("MS Shell Dlg 2", "Segoe UI"),
    ("Times New Roman", "Segoe UI"),
    ("Segoe WPC", "Segoe UI"),
    ("Consolas", "Cascadia Mono"),
    ("Courier New", "Cascadia Mono"),
    ("Lucida Console", "Cascadia Mono"),
    ("Cascadia Code", "Cascadia Mono"),
];

#[cfg(test)]
mod tests;
