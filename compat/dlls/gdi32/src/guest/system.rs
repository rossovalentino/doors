//! The rest of gdi32's surface Chromium and Electron import (P18 M7):
//! graphics modes and world transforms (kept per DC; Doors' rasterizer
//! draws untransformed), paths (accepted, not filled or stroked),
//! geometric pens, enhanced metafiles (recorded as empty documents),
//! printing (no printers: StartDoc fails as on a computer without one),
//! OpenGL pixel formats (none), ANSI fonts, font family enumeration over
//! Doors' font set, character widths, clip regions and region data.
use super::{CreateFontIndirectW, CreatePen, dc, object, with};
use super::alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use door_windows_abi::{narrow, widen};
use door_windows_gui::gdi::Object;
use door_windows_gui::win32_error as e;
use door_windows_gui_guest as g;

const SP_ERROR: i32 = -1;
const ERROR_INVALID_PRINTER_NAME: u32 = 1801;
const ERROR_INVALID_PIXEL_FORMAT: u32 = 2000;
const GDI_ERROR: u32 = 0xFFFF_FFFF;
/// GM_COMPATIBLE 1, GM_ADVANCED 2.
const GM_ADVANCED: i32 = 2;

fn fail<T: Default>(code: u32) -> T {
    g::set_last_error(code);
    T::default()
}
fn valid_dc(hdc: usize) -> bool {
    with(false, |s| dc(s, widen(hdc)).map(|_| true))
}

// ---- graphics modes and world transforms ------------------------------------------------
/// Per DC: (DC value, graphics mode, XFORM eM11 eM12 eM21 eM22 eDx eDy).
static TRANSFORMS: g::Lock<Vec<(u64, i32, [f32; 6])>> = g::Lock::new(Vec::new());
const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
fn transform_of(hdc: u64) -> (i32, [f32; 6]) {
    TRANSFORMS
        .lock()
        .iter()
        .find(|(d, _, _)| *d == hdc)
        .map_or((1, IDENTITY), |(_, m, x)| (*m, *x))
}
fn set_transform(hdc: u64, mode: i32, xform: [f32; 6]) {
    let mut t = TRANSFORMS.lock();
    t.retain(|(d, _, _)| *d != hdc);
    if mode != 1 || xform != IDENTITY {
        t.push((hdc, mode, xform));
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SetGraphicsMode(hdc: usize, mode: i32) -> i32 {
    if !valid_dc(hdc) || !(1..=2).contains(&mode) {
        return fail(e::INVALID_PARAMETER);
    }
    let (old, xform) = transform_of(widen(hdc));
    // Leaving GM_ADVANCED requires the identity transform.
    if mode == 1 && xform != IDENTITY {
        return 0;
    }
    set_transform(widen(hdc), mode, xform);
    old
}
#[unsafe(no_mangle)]
pub extern "system" fn GetGraphicsMode(hdc: usize) -> i32 {
    if !valid_dc(hdc) {
        return 0;
    }
    transform_of(widen(hdc)).0
}
fn read_xform(pointer: usize) -> Option<[f32; 6]> {
    let bytes = g::read_bytes(widen(pointer), 24)?;
    let mut x = [0f32; 6];
    for (i, v) in x.iter_mut().enumerate() {
        *v = f32::from_le_bytes(bytes[4 * i..4 * i + 4].try_into().ok()?);
    }
    Some(x)
}
fn multiply(a: [f32; 6], b: [f32; 6]) -> [f32; 6] {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}
#[unsafe(no_mangle)]
pub extern "system" fn SetWorldTransform(hdc: usize, xform: usize) -> i32 {
    let Some(x) = read_xform(xform) else {
        return fail(e::INVALID_PARAMETER);
    };
    if !valid_dc(hdc) || transform_of(widen(hdc)).0 != GM_ADVANCED {
        return fail(e::INVALID_PARAMETER);
    }
    set_transform(widen(hdc), GM_ADVANCED, x);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetWorldTransform(hdc: usize, out: usize) -> i32 {
    if !valid_dc(hdc) {
        return fail(e::INVALID_HANDLE);
    }
    let x = transform_of(widen(hdc)).1;
    let bytes: Vec<u8> = x.iter().flat_map(|v| v.to_le_bytes()).collect();
    i32::from(g::write(widen(out), &bytes))
}
/// ModifyWorldTransform: MWT_IDENTITY (1), MWT_LEFTMULTIPLY (2),
/// MWT_RIGHTMULTIPLY (3).
#[unsafe(no_mangle)]
pub extern "system" fn ModifyWorldTransform(hdc: usize, xform: usize, mode: u32) -> i32 {
    let (graphics, current) = transform_of(widen(hdc));
    if !valid_dc(hdc) || graphics != GM_ADVANCED {
        return fail(e::INVALID_PARAMETER);
    }
    let next = match mode {
        1 => IDENTITY,
        2 | 3 => {
            let Some(x) = read_xform(xform) else {
                return fail(e::INVALID_PARAMETER);
            };
            if mode == 2 {
                multiply(x, current)
            } else {
                multiply(current, x)
            }
        }
        _ => return fail(e::INVALID_PARAMETER),
    };
    set_transform(widen(hdc), GM_ADVANCED, next);
    1
}

// ---- paths, pens and fill modes -------------------------------------------------------------------
/// Path brackets and operations: accepted on a valid DC; Doors does not
/// record path geometry, so filling, stroking and path clipping draw
/// nothing.
macro_rules! path_call {
    ($($name:ident),* $(,)?) => {$(
        #[unsafe(no_mangle)]
        pub extern "system" fn $name(hdc: usize) -> i32 {
            if valid_dc(hdc) { 1 } else { fail(e::INVALID_HANDLE) }
        }
    )*};
}
path_call!(
    BeginPath,
    EndPath,
    AbortPath,
    CloseFigure,
    FillPath,
    StrokePath,
    StrokeAndFillPath,
    WidenPath,
    FlattenPath
);
#[unsafe(no_mangle)]
pub extern "system" fn SelectClipPath(hdc: usize, _mode: i32) -> i32 {
    if valid_dc(hdc) { 1 } else { fail(e::INVALID_HANDLE) }
}
/// PolyBezierTo: the current position moves to the last point.
#[unsafe(no_mangle)]
pub extern "system" fn PolyBezierTo(hdc: usize, points: usize, count: u32) -> i32 {
    if count == 0 || count % 3 != 0 {
        return fail(e::INVALID_PARAMETER);
    }
    let Some([x, y]) = g::read_i32s::<2>(widen(points) + 8 * u64::from(count - 1)) else {
        return fail(e::INVALID_PARAMETER);
    };
    super::MoveToEx(hdc, x, y, 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn SetMiterLimit(hdc: usize, _limit: f32, old: usize) -> i32 {
    if !valid_dc(hdc) {
        return fail(e::INVALID_HANDLE);
    }
    if old != 0 {
        g::write(widen(old), &10.0f32.to_le_bytes());
    }
    1
}
static ARC: AtomicU64 = AtomicU64::new(1);
/// SetArcDirection: AD_COUNTERCLOCKWISE (1) or AD_CLOCKWISE (2).
#[unsafe(no_mangle)]
pub extern "system" fn SetArcDirection(hdc: usize, direction: i32) -> i32 {
    if !valid_dc(hdc) || !(1..=2).contains(&direction) {
        return fail(e::INVALID_PARAMETER);
    }
    ARC.swap(direction as u64, Ordering::Relaxed) as i32
}
/// SetPolyFillMode: ALTERNATE (1) or WINDING (2), the DC's previous one.
#[unsafe(no_mangle)]
pub extern "system" fn SetPolyFillMode(hdc: usize, mode: i32) -> i32 {
    if !(1..=2).contains(&mode) {
        return fail(e::INVALID_PARAMETER);
    }
    with(0, |s| {
        let h = dc(s, widen(hdc))?;
        let state = s.gdi.state_mut(h).map_err(|_| e::INVALID_HANDLE)?;
        Ok(core::mem::replace(&mut state.poly_fill, mode))
    })
}
/// ExtCreatePen: a cosmetic or geometric pen of the brush's colour
/// (LOGBRUSH { lbStyle, lbColor, lbHatch }).
#[unsafe(no_mangle)]
pub extern "system" fn ExtCreatePen(style: u32, width: u32, brush: usize, _count: u32, _styles: usize) -> usize {
    let Some(color) = g::read_u32(widen(brush) + 4) else {
        return fail(e::INVALID_PARAMETER);
    };
    // The line style (PS_SOLID .. PS_INSIDEFRAME) of PS_STYLE_MASK.
    CreatePen((style & 0xF) as i32, width.max(1) as i32, color)
}

// ---- enhanced metafiles -------------------------------------------------------------------------
/// Metafile DCs draw into a memory DC; their metafiles hold no records.
static METAFILE_DCS: g::Lock<Vec<u64>> = g::Lock::new(Vec::new());
static METAFILES: g::Lock<Vec<u64>> = g::Lock::new(Vec::new());
static NEXT_METAFILE: AtomicU64 = AtomicU64::new(0x0046_0000);
fn new_metafile() -> usize {
    let h = NEXT_METAFILE.fetch_add(4, Ordering::Relaxed);
    METAFILES.lock().push(h);
    narrow(h)
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateEnhMetaFileW(reference: usize, _file: usize, _bounds: usize, _description: usize) -> usize {
    let hdc = super::CreateCompatibleDC(reference);
    if hdc != 0 {
        METAFILE_DCS.lock().push(widen(hdc));
    }
    hdc
}
#[unsafe(no_mangle)]
pub extern "system" fn CloseEnhMetaFile(hdc: usize) -> usize {
    let mut dcs = METAFILE_DCS.lock();
    let Some(i) = dcs.iter().position(|d| *d == widen(hdc)) else {
        drop(dcs);
        return fail(e::INVALID_HANDLE);
    };
    dcs.remove(i);
    drop(dcs);
    super::DeleteDC(hdc);
    new_metafile()
}
fn is_metafile(h: usize) -> bool {
    METAFILES.lock().contains(&widen(h))
}
#[unsafe(no_mangle)]
pub extern "system" fn DeleteEnhMetaFile(metafile: usize) -> i32 {
    let mut m = METAFILES.lock();
    let before = m.len();
    m.retain(|x| *x != widen(metafile));
    i32::from(m.len() < before)
}
#[unsafe(no_mangle)]
pub extern "system" fn SetEnhMetaFileBits(_size: u32, data: usize) -> usize {
    if data == 0 {
        return fail(e::INVALID_PARAMETER);
    }
    new_metafile()
}
/// GetEnhMetaFileBits / GetEnhMetaFileHeader: an empty metafile has no
/// bytes to give.
#[unsafe(no_mangle)]
pub extern "system" fn GetEnhMetaFileBits(metafile: usize, _size: u32, _data: usize) -> u32 {
    if is_metafile(metafile) { 0 } else { fail(e::INVALID_HANDLE) }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetEnhMetaFileHeader(metafile: usize, _size: u32, _header: usize) -> u32 {
    if is_metafile(metafile) { 0 } else { fail(e::INVALID_HANDLE) }
}
#[unsafe(no_mangle)]
pub extern "system" fn PlayEnhMetaFile(_hdc: usize, metafile: usize, _rect: usize) -> i32 {
    i32::from(is_metafile(metafile))
}
#[unsafe(no_mangle)]
pub extern "system" fn PlayEnhMetaFileRecord(_hdc: usize, _table: usize, _record: usize, _count: u32) -> i32 {
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn EnumEnhMetaFile(_hdc: usize, metafile: usize, _procedure: usize, _data: usize, _rect: usize) -> i32 {
    i32::from(is_metafile(metafile))
}
#[unsafe(no_mangle)]
pub extern "system" fn GdiComment(hdc: usize, _size: u32, _data: usize) -> i32 {
    i32::from(valid_dc(hdc))
}

// ---- printing --------------------------------------------------------------------------------
/// CreateDCW: "DISPLAY" is the screen; there are no printers.
#[unsafe(no_mangle)]
pub extern "system" fn CreateDCW(driver: usize, device: usize, _port: usize, _mode: usize) -> usize {
    let name = |p: usize| -> Vec<u16> {
        if p == 0 {
            return Vec::new();
        }
        g::read_wide(widen(p), 64)
            .unwrap_or_default()
            .iter()
            .map(|u| if (0x61..=0x7A).contains(u) { u - 32 } else { *u })
            .collect()
    };
    let display: Vec<u16> = "DISPLAY".encode_utf16().collect();
    let monitor: Vec<u16> = "\\\\.\\DISPLAY1".encode_utf16().collect();
    let (d, v) = (name(driver), name(device));
    if d == display || v == monitor || v == display {
        return narrow(with(0, |s| s.gdi.create_display_dc().map(|h| h.value()).map_err(super::win32)));
    }
    fail(ERROR_INVALID_PRINTER_NAME)
}
#[unsafe(no_mangle)]
pub extern "system" fn StartDocW(_hdc: usize, _info: usize) -> i32 {
    g::set_last_error(ERROR_INVALID_PRINTER_NAME);
    SP_ERROR
}
macro_rules! print_call {
    ($($name:ident),* $(,)?) => {$(
        #[unsafe(no_mangle)]
        pub extern "system" fn $name(_hdc: usize) -> i32 {
            g::set_last_error(ERROR_INVALID_PRINTER_NAME);
            SP_ERROR
        }
    )*};
}
print_call!(EndDoc, StartPage, EndPage, AbortDoc);
#[unsafe(no_mangle)]
pub extern "system" fn SetAbortProc(_hdc: usize, _procedure: usize) -> i32 {
    SP_ERROR
}
#[unsafe(no_mangle)]
pub extern "system" fn CancelDC(hdc: usize) -> i32 {
    i32::from(valid_dc(hdc))
}
/// ExtEscape: no driver escapes (QUERYESCSUPPORT finds none).
#[unsafe(no_mangle)]
pub extern "system" fn ExtEscape(_hdc: usize, _escape: i32, _in_size: i32, _input: usize, _out_size: i32, _output: usize) -> i32 {
    0
}

// ---- OpenGL pixel formats -----------------------------------------------------------------------
/// No OpenGL driver: no DC has pixel formats.
#[unsafe(no_mangle)]
pub extern "system" fn ChoosePixelFormat(_hdc: usize, _descriptor: usize) -> i32 {
    fail(ERROR_INVALID_PIXEL_FORMAT)
}
#[unsafe(no_mangle)]
pub extern "system" fn DescribePixelFormat(_hdc: usize, _format: i32, _size: u32, _descriptor: usize) -> i32 {
    fail(ERROR_INVALID_PIXEL_FORMAT)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetPixelFormat(_hdc: usize) -> i32 {
    fail(ERROR_INVALID_PIXEL_FORMAT)
}
#[unsafe(no_mangle)]
pub extern "system" fn SetPixelFormat(_hdc: usize, _format: i32, _descriptor: usize) -> i32 {
    fail(ERROR_INVALID_PIXEL_FORMAT)
}
#[unsafe(no_mangle)]
pub extern "system" fn SwapBuffers(_hdc: usize) -> i32 {
    fail(ERROR_INVALID_PIXEL_FORMAT)
}

// ---- fonts ------------------------------------------------------------------------------------
/// LOGFONTA (60 bytes) as LOGFONTW (92): the same 28-byte header, the
/// face name widened.
fn logfont_wide(a: &[u8]) -> [u8; 92] {
    let mut w = [0u8; 92];
    w[..28].copy_from_slice(&a[..28]);
    for (i, b) in a[28..60].iter().take_while(|b| **b != 0).enumerate() {
        w[28 + 2 * i] = *b;
    }
    w
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateFontIndirectA(logfont: usize) -> usize {
    let Some(a) = g::read_bytes(widen(logfont), 60) else {
        return fail(e::INVALID_PARAMETER);
    };
    let w = logfont_wide(&a);
    CreateFontIndirectW(w.as_ptr() as usize)
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn CreateFontA(
    height: i32,
    width: i32,
    escapement: i32,
    orientation: i32,
    weight: i32,
    italic: u32,
    underline: u32,
    strike_out: u32,
    charset: u32,
    precision: u32,
    clip: u32,
    quality: u32,
    pitch: u32,
    face: usize,
) -> usize {
    let mut a = [0u8; 60];
    for (i, v) in [height, width, escapement, orientation, weight].iter().enumerate() {
        a[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
    }
    for (i, v) in [italic, underline, strike_out, charset, precision, clip, quality, pitch].iter().enumerate() {
        a[20 + i] = *v as u8;
    }
    if face != 0
        && let Some(name) = g::read_bytes(widen(face), 32)
    {
        let n = name.iter().position(|b| *b == 0).unwrap_or(31).min(31);
        a[28..28 + n].copy_from_slice(&name[..n]);
    }
    CreateFontIndirectA(a.as_ptr() as usize)
}
/// The families gdi32's font set serves.
const FAMILIES: [&str; 4] = ["Segoe UI", "Segoe UI Variable", "Segoe UI Symbol", "Cascadia Mono"];
type FontEnumProc = extern "system" fn(usize, usize, u32, isize) -> i32;
/// EnumFontFamiliesEx: each family (or the one `logfont` names) with its
/// metrics, as a TrueType font (FontType 4).
fn enum_families(hdc: usize, logfont: usize, procedure: usize, data: isize, wide: bool) -> i32 {
    if procedure == 0 || !valid_dc(hdc) {
        return 0;
    }
    let face_offset = 28u64;
    let wanted: Vec<u16> = if wide {
        g::read_wide(widen(logfont) + face_offset, 32).unwrap_or_default()
    } else {
        g::read_bytes(widen(logfont) + face_offset, 32)
            .unwrap_or_default()
            .iter()
            .take_while(|b| **b != 0)
            .map(|b| u16::from(*b))
            .collect()
    };
    // SAFETY: the application's FONTENUMPROC, called with no lock held.
    let f: FontEnumProc = unsafe { core::mem::transmute(procedure) };
    let mut last = 1;
    for family in FAMILIES {
        let name: Vec<u16> = family.encode_utf16().collect();
        if !wanted.is_empty() && !wanted.eq_ignore_ascii_case_units(&name) {
            continue;
        }
        // The family at 16 px: its LOGFONTW and real text metrics.
        let mut lf = [0u8; 92];
        lf[..4].copy_from_slice(&(-16i32).to_le_bytes());
        lf[16..20].copy_from_slice(&400i32.to_le_bytes());
        lf[23] = 1;
        lf[26] = 5;
        lf[27] = if family == "Cascadia Mono" { 0x31 } else { 0x22 };
        for (i, u) in name.iter().enumerate() {
            lf[28 + 2 * i..30 + 2 * i].copy_from_slice(&u.to_le_bytes());
        }
        let font = CreateFontIndirectW(lf.as_ptr() as usize);
        let memory = super::CreateCompatibleDC(hdc);
        let old = super::SelectObject(memory, font);
        let mut tm = [0u8; 60];
        super::GetTextMetricsW(memory, tm.as_mut_ptr() as usize);
        super::SelectObject(memory, old);
        super::DeleteDC(memory);
        super::DeleteObject(font);
        // ENUMLOGFONTEXW: LOGFONTW, elfFullName[64], elfStyle[32],
        // elfScript[32]; NEWTEXTMETRICEXW: TEXTMETRICW, ntmFlags,
        // ntmSizeEM, ntmCellHeight, ntmAvgWidth, FONTSIGNATURE.
        let mut elf = [0u8; 348];
        elf[..92].copy_from_slice(&lf);
        for (i, u) in name.iter().enumerate() {
            elf[92 + 2 * i..94 + 2 * i].copy_from_slice(&u.to_le_bytes());
        }
        for (i, u) in "Regular".encode_utf16().enumerate() {
            elf[220 + 2 * i..222 + 2 * i].copy_from_slice(&u.to_le_bytes());
        }
        for (i, u) in "Western".encode_utf16().enumerate() {
            elf[284 + 2 * i..286 + 2 * i].copy_from_slice(&u.to_le_bytes());
        }
        let mut ntm = [0u8; 100];
        ntm[..60].copy_from_slice(&tm);
        ntm[60..64].copy_from_slice(&0x40u32.to_le_bytes());
        ntm[64..68].copy_from_slice(&2048u32.to_le_bytes());
        let (elf_out, ntm_out): (Vec<u8>, Vec<u8>) = if wide {
            (elf.to_vec(), ntm.to_vec())
        } else {
            (ansi_logfont_ex(&elf), ansi_metrics(&ntm))
        };
        last = f(elf_out.as_ptr() as usize, ntm_out.as_ptr() as usize, 4, data);
        if last == 0 {
            break;
        }
    }
    last
}
trait UnitsEq {
    fn eq_ignore_ascii_case_units(&self, other: &[u16]) -> bool;
}
impl UnitsEq for Vec<u16> {
    fn eq_ignore_ascii_case_units(&self, other: &[u16]) -> bool {
        let fold = |u: &u16| if (0x61..=0x7A).contains(u) { u - 32 } else { *u };
        self.len() == other.len() && self.iter().map(fold).eq(other.iter().map(fold))
    }
}
/// ENUMLOGFONTEXA from the wide record: LOGFONTA (60), then 64, 32 and 32
/// chars.
fn ansi_logfont_ex(w: &[u8]) -> Vec<u8> {
    let mut a = [0u8; 188].to_vec();
    a[..28].copy_from_slice(&w[..28]);
    for (src, dst, n) in [(28, 28, 32), (92, 60, 64), (220, 124, 32), (284, 156, 32)] {
        for i in 0..n {
            a[dst + i] = w[src + 2 * i];
        }
    }
    a
}
/// NEWTEXTMETRICEXA: TEXTMETRICA (its four characters are bytes), then
/// the same tail.
fn ansi_metrics(w: &[u8]) -> Vec<u8> {
    let mut a = [0u8; 96].to_vec();
    a[..44].copy_from_slice(&w[..44]);
    for i in 0..4 {
        a[44 + i] = w[44 + 2 * i];
    }
    a[48..56].copy_from_slice(&w[52..60]);
    a[56..96].copy_from_slice(&w[60..100]);
    a
}
#[unsafe(no_mangle)]
pub extern "system" fn EnumFontFamiliesExW(hdc: usize, logfont: usize, procedure: usize, data: isize, _flags: u32) -> i32 {
    enum_families(hdc, logfont, procedure, data, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn EnumFontFamiliesExA(hdc: usize, logfont: usize, procedure: usize, data: isize, _flags: u32) -> i32 {
    enum_families(hdc, logfont, procedure, data, false)
}
/// GetCharWidthW: each character's advance in the DC's font.
#[unsafe(no_mangle)]
pub extern "system" fn GetCharWidthW(hdc: usize, first: u32, last: u32, out: usize) -> i32 {
    if last < first || last - first > 0xFFFF || !valid_dc(hdc) {
        return fail(e::INVALID_PARAMETER);
    }
    let mut widths = Vec::with_capacity((last - first + 1) as usize);
    for c in first..=last {
        let unit = c as u16;
        let mut advance = 0i32;
        super::DoorGdiTextAdvances(
            widen(hdc),
            &unit as *const u16 as u64,
            1,
            &mut advance as *mut i32 as u64,
        );
        widths.push(advance);
    }
    i32::from(g::write_i32s(widen(out), &widths))
}
#[unsafe(no_mangle)]
pub extern "system" fn GetCharWidth32W(hdc: usize, first: u32, last: u32, out: usize) -> i32 {
    GetCharWidthW(hdc, first, last, out)
}
/// GetFontData: font tables are not exposed through GDI (DirectWrite
/// serves them).
#[unsafe(no_mangle)]
pub extern "system" fn GetFontData(_hdc: usize, _table: u32, _offset: u32, _buffer: usize, _size: u32) -> u32 {
    GDI_ERROR
}
#[unsafe(no_mangle)]
pub extern "system" fn GetOutlineTextMetricsW(_hdc: usize, _size: u32, _out: usize) -> u32 {
    0
}
/// GetTextFaceA: the selected font's face name.
#[unsafe(no_mangle)]
pub extern "system" fn GetTextFaceA(hdc: usize, count: i32, out: usize) -> i32 {
    let Some(face) = with(None, |s| {
        let h = dc(s, widen(hdc))?;
        Ok(s.gdi.font(h).ok().map(|f| f.face))
    }) else {
        return 0;
    };
    let n = face.iter().position(|u| *u == 0).unwrap_or(face.len());
    if out == 0 {
        return n as i32 + 1;
    }
    let take = n.min(count.max(1) as usize - 1);
    let mut bytes: Vec<u8> = face[..take].iter().map(|u| if *u < 0x80 { *u as u8 } else { b'?' }).collect();
    bytes.push(0);
    if g::write(widen(out), &bytes) { take as i32 } else { 0 }
}

// ---- regions and colour ---------------------------------------------------------------------------
/// GetClipRgn: the DC's application clip region into `region` (1), or 0
/// without one.
#[unsafe(no_mangle)]
pub extern "system" fn GetClipRgn(hdc: usize, region: usize) -> i32 {
    with(-1, |s| {
        let h = dc(s, widen(hdc))?;
        let r = object(s, widen(region))?;
        let clip = s.gdi.state(h).map_err(|_| e::INVALID_HANDLE)?.clip.clone();
        let Some(clip) = clip else {
            return Ok(0);
        };
        match s.gdi.object_mut(r) {
            Some(Object::Region(target)) => {
                *target = clip;
                Ok(1)
            }
            _ => Err(e::INVALID_HANDLE),
        }
    })
}
/// GetRegionData: RGNDATAHEADER (32 bytes) and the region's rectangles;
/// a null or short buffer gets the size needed.
#[unsafe(no_mangle)]
pub extern "system" fn GetRegionData(region: usize, size: u32, out: usize) -> u32 {
    let Some(rects) = with(None, |s| {
        let r = object(s, widen(region))?;
        match s.gdi.object(r) {
            Some(Object::Region(region)) => Ok(Some((region.rects().to_vec(), region.bounds()))),
            _ => Err(e::INVALID_HANDLE),
        }
    }) else {
        return 0;
    };
    let (list, bounds) = rects;
    let needed = 32 + 16 * list.len() as u32;
    if out == 0 || size < needed {
        return needed;
    }
    let mut b = Vec::with_capacity(needed as usize);
    for v in [32u32, 1, list.len() as u32, 16 * list.len() as u32] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    for v in [bounds.left, bounds.top, bounds.right, bounds.bottom] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    for r in &list {
        for v in [r.left, r.top, r.right, r.bottom] {
            b.extend_from_slice(&v.to_le_bytes());
        }
    }
    if g::write(widen(out), &b) { needed } else { 0 }
}
/// GetICMProfileW: no colour profile is installed (sRGB is assumed).
#[unsafe(no_mangle)]
pub extern "system" fn GetICMProfileW(_hdc: usize, _size: usize, _name: usize) -> i32 {
    0
}
