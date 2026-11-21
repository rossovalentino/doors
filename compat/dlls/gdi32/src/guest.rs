//! PE exports of gdi32. One process-wide GDI model behind a short lock; no
//! callback into application code exists in GDI, so the lock never spans
//! one. Application pointers are captured with checked copies; the only
//! memory GDI touches directly is memory this DLL allocated (DIB-section
//! bits it hands to the application by design of `CreateDIBSection`) or that
//! user32 registered (window back buffers).
//!
//! The `DoorGdi*` exports are user32's private interface but, like every
//! export, callable by name; they therefore trust no argument. A back buffer
//! is accepted only when ntdll's heap records show it is exactly one live
//! block of the registered size overlapping nothing GDI already draws into;
//! it is re-verified before every draw, surface DCs exist only over such
//! buffers, and releases name only registered buffers.
extern crate alloc;
use alloc::boxed::Box;
use alloc::vec::Vec;
use door_graphics_text::{FontProvider, FontRole, FontSet};
use door_windows_abi::layout::{self, Field, Field::*};
use door_windows_gui::dib::DibInfo;
use door_windows_gui::gdi::{
    Bitmap, Brush, External, Gdi, LogFont, Object, Pixels, TextBackend, pen,
};
use door_windows_gui::handles::Handle;
use door_windows_gui::msg::{bk, hs, mm, obj, ps, rop};
use door_windows_gui::region::{Op, Region};
use door_windows_gui::surface::{MAX_EDGE, Surface, ddb_stride};
use door_windows_gui::text::DoorText;
use door_windows_gui::{ColorRef, Error, Point, Rect, Size, win32_error as e};
use door_windows_gui_guest as g;

mod system;

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

const CLR_INVALID: u32 = 0xffff_ffff;
/// BITMAP: bmType, bmWidth, bmHeight, bmWidthBytes, bmPlanes, bmBitsPixel,
/// bmBits; LOGBRUSH: lbStyle, lbColor, lbHatch (ULONG_PTR). Their sizes and
/// the pointer's offset follow the build (P17).
const BITMAP: [Field; 7] = [U32, U32, U32, U32, U16, U16, Pointer];
const LOGBRUSH: [Field; 3] = [U32, U32, Pointer];
const ERROR_REGION: i32 = 0;

/// Memory GDI draws into directly: DIB-section bits this DLL allocated
/// (`owned`) or a back buffer user32 registered.
struct Memory {
    address: u64,
    bytes: usize,
    owned: bool,
}
impl Memory {
    fn overlaps(&self, address: u64, end: u64) -> bool {
        address < self.address.saturating_add(self.bytes as u64) && self.address < end
    }
}

struct State {
    gdi: Gdi,
    memory: Vec<Memory>,
    text: Option<DoorText>,
    fonts_failed: bool,
    screen: (i32, i32),
}

struct Ext<'a>(&'a [Memory]);
impl External for Ext<'_> {
    fn pixels(&mut self, key: u64) -> Option<&mut [u32]> {
        let m = self.0.iter().find(|m| m.address == key)?;
        // A registered buffer must still be the live heap block it was when
        // registered: a block freed behind GDI's back is never drawn into.
        if !m.owned && !live_heap_block(m.address, m.bytes) {
            return None;
        }
        // SAFETY: `key` names memory this DLL allocated (DIB bits) or a
        // registered back buffer that is still the live heap block of exactly
        // `bytes`. Application writes may race with GDI exactly as on Windows
        // (the documented GdiFlush contract); no Rust reference outlives the
        // call.
        Some(unsafe { core::slice::from_raw_parts_mut(m.address as *mut u32, m.bytes / 4) })
    }
}

static STATE: g::Lock<Option<State>> = g::Lock::new(None);

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn RtlSizeHeap(heap: usize, flags: u32, pointer: usize) -> usize;
    fn DoorGetProcessHeap() -> u64;
}

/// Is `[address, address + bytes)` exactly one live block of the process
/// heap? ntdll answers from its own records and reads nothing outside its
/// segments, so any value may be asked about.
fn live_heap_block(address: u64, bytes: usize) -> bool {
    // SAFETY: plain queries; ntdll validates the handle and the pointer.
    let size = unsafe {
        RtlSizeHeap(
            door_windows_abi::narrow(DoorGetProcessHeap()),
            0,
            door_windows_abi::narrow(address),
        )
    };
    size != usize::MAX && size == bytes
}

fn win32(err: Error) -> u32 {
    err.win32()
}
/// Run `f` on the process GDI state; failures set the last error.
fn with<R>(fail: R, f: impl FnOnce(&mut State) -> Result<R, u32>) -> R {
    let mut guard = STATE.lock();
    if guard.is_none() {
        match Gdi::new() {
            Ok(gdi) => {
                *guard = Some(State {
                    gdi,
                    memory: Vec::new(),
                    text: None,
                    fonts_failed: false,
                    screen: (1280, 720),
                })
            }
            Err(err) => {
                drop(guard);
                g::set_last_error(win32(err));
                return fail;
            }
        }
    }
    match f(guard.as_mut().unwrap()) {
        Ok(v) => v,
        Err(code) => {
            drop(guard);
            g::set_last_error(code);
            fail
        }
    }
}
fn dc(s: &State, value: u64) -> Result<Handle, u32> {
    s.gdi.resolve_dc(value).ok_or(e::INVALID_HANDLE)
}
fn object(s: &State, value: u64) -> Result<Handle, u32> {
    s.gdi.resolve(value).ok_or(e::INVALID_HANDLE)
}
fn create(s: &mut State, o: Object) -> Result<u64, u32> {
    s.gdi.create(o).map(Handle::value).map_err(win32)
}

// ---- Door Text font set ------------------------------------------------------

/// Read-only view of the application asset pack: the top of the native
/// section window, as native desktop applications map it.
const ASSETS_ADDRESS: u64 = 0x7e00_0000;
const FONT_KEYS: [(&str, FontRole); 5] = [
    ("fonts/segoe-ui/SegUIVar.ttf", FontRole::UiVariable),
    ("fonts/segoe-ui/segoeui.ttf", FontRole::Ui),
    ("fonts/segoe-ui/seguisb.ttf", FontRole::Ui),
    ("fonts/windows-unicode/seguisym.ttf", FontRole::Symbol),
    ("fonts/cascadia-mono/CascadiaMono.ttf", FontRole::Monospace),
];
static ASSETS: g::Lock<Option<&'static [u8]>> = g::Lock::new(None);

fn asset_pack() -> Option<&'static [u8]> {
    let mut a = ASSETS.lock();
    if let Some(v) = *a {
        return Some(v);
    }
    let handle = g::inherited(door_abi::platform::inherited::APP_ASSETS);
    if handle == 0 {
        return None;
    }
    // SAFETY: the address range is reserved for this one read-only view,
    // which is never unmapped; no reference to it exists before the map.
    let bytes =
        unsafe { door_sys::section_map(handle, ASSETS_ADDRESS, door_abi::rights::READ) }.ok()?;
    if bytes == 0 || bytes > door_initfs::app_assets::MAX_BYTES as u64 {
        return None;
    }
    let view = unsafe { core::slice::from_raw_parts(ASSETS_ADDRESS as *const u8, bytes as usize) };
    door_initfs::app_assets::Pack::parse(view)?;
    *a = Some(view);
    Some(view)
}

/// On-demand read-only Windows fonts (CJK and others); refused by the
/// kernel unless the manifest is granted resource views.
struct ResourceFonts;
/// The next free address of the resource window, and each font resource's
/// view once mapped (GDI text and DirectWrite share them).
static RESOURCE_NEXT: g::Lock<u64> = g::Lock::new(door_abi::platform::resource::VIEW_START);
static RESOURCE_VIEWS: g::Lock<[Option<&'static [u8]>; 7]> = g::Lock::new([None; 7]);
/// The view of font resource `id` (1..=7), mapped on first use.
fn resource_font(id: u64) -> Option<&'static [u8]> {
    use door_abi::platform::resource as r;
    let slot = usize::try_from(id)
        .ok()?
        .checked_sub(1)
        .filter(|s| *s < 7)?;
    let mut views = RESOURCE_VIEWS.lock();
    if let Some(view) = views[slot] {
        return Some(view);
    }
    let handle = door_sys::resource_open(id).ok()?;
    let mut next = RESOURCE_NEXT.lock();
    let address = *next;
    // SAFETY: the resource window is reserved for these views and
    // `address` lies past every earlier one; nothing references it yet.
    let mapped = unsafe { door_sys::section_map(handle, address, door_abi::rights::READ) };
    let _ = door_sys::close(handle);
    let bytes = mapped.ok()?;
    let end = address + bytes.div_ceil(0x10_0000) * 0x10_0000;
    if end > r::VIEW_END {
        return None;
    }
    *next = end;
    let view = unsafe { core::slice::from_raw_parts(address as *const u8, bytes as usize) };
    views[slot] = Some(view);
    Some(view)
}
impl FontProvider for ResourceFonts {
    fn load(&self, role: FontRole) -> Vec<&'static [u8]> {
        use door_abi::platform::resource as r;
        let id = match role {
            FontRole::Emoji => r::EMOJI,
            FontRole::Historic => r::HISTORIC,
            FontRole::ChineseSimplified => r::CHINESE_SIMPLIFIED,
            FontRole::ChineseTraditional => r::CHINESE_TRADITIONAL,
            FontRole::Japanese => r::JAPANESE,
            FontRole::Korean => r::KOREAN,
            FontRole::Indic => r::INDIC,
            _ => return Vec::new(),
        };
        resource_font(id).into_iter().collect()
    }
}
/// Door-private, for dwrite.dll: the system font files, as records of
/// (address, bytes, file name: pointer to a NUL-terminated ASCII name),
/// each three pointer-sized values. Returns how many exist and writes at
/// most `capacity`. The views stay mapped for the process lifetime.
#[unsafe(no_mangle)]
pub extern "system" fn DoorSystemFontFiles(out: usize, capacity: u32) -> u32 {
    use door_abi::platform::resource as r;
    const NAMES: [&str; 7] = [
        "seguiemj.ttf\0",
        "seguihis.ttf\0",
        "msyh.ttc\0",
        "msjh.ttc\0",
        "YuGothM.ttc\0",
        "malgun.ttf\0",
        "Nirmala.ttc\0",
    ];
    let out = door_windows_abi::widen(out);
    let mut files: Vec<(&'static [u8], &'static str)> = Vec::new();
    if let Some(pack) = asset_pack().and_then(door_initfs::app_assets::Pack::parse) {
        for key in FONT_KEYS.iter().map(|(k, _)| *k) {
            if let Some(bytes) = pack.get(door_initfs::app_assets::key_of(key))
                && !bytes.is_empty()
            {
                files.push((bytes, key.rsplit('/').next().unwrap_or(key)));
            }
        }
    }
    for (i, name) in NAMES.iter().enumerate() {
        if let Some(view) = resource_font(r::EMOJI + i as u64) {
            files.push((view, name));
        }
    }
    let width = core::mem::size_of::<usize>();
    for (i, (bytes, name)) in files.iter().take(capacity as usize).enumerate() {
        let record = out + (3 * width * i) as u64;
        // A file name without its NUL is copied with one (names in the
        // asset table are not NUL-terminated).
        let name_pointer = if name.ends_with('\0') {
            name.as_ptr() as u64
        } else {
            let mut z = Vec::with_capacity(name.len() + 1);
            z.extend_from_slice(name.as_bytes());
            z.push(0);
            alloc::boxed::Box::leak(z.into_boxed_slice()).as_ptr() as u64
        };
        for (j, v) in [bytes.as_ptr() as u64, bytes.len() as u64, name_pointer]
            .into_iter()
            .enumerate()
        {
            // SAFETY: the caller supplies `capacity` records.
            unsafe { ((record + (j * width) as u64) as *mut usize).write_unaligned(v as usize) };
        }
    }
    files.len() as u32
}

fn text(s: &mut State) -> Result<&DoorText, u32> {
    if s.text.is_none() {
        if s.fonts_failed {
            return Err(e::NOT_SUPPORTED);
        }
        let pack = asset_pack().and_then(door_initfs::app_assets::Pack::parse);
        let files: Vec<(&'static [u8], FontRole)> = match pack {
            Some(p) => FONT_KEYS
                .iter()
                .filter_map(|(k, role)| {
                    p.get(door_initfs::app_assets::key_of(k))
                        .map(|b| (b, *role))
                })
                .filter(|(b, _)| !b.is_empty())
                .collect(),
            None => Vec::new(),
        };
        match FontSet::from_files(&files) {
            Ok(set) if !files.is_empty() => {
                s.text = Some(DoorText::new(set.with_provider(Box::new(ResourceFonts))));
            }
            _ => {
                s.fonts_failed = true;
                return Err(e::NOT_SUPPORTED);
            }
        }
    }
    Ok(s.text.as_ref().unwrap())
}

// ---- device contexts ------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn CreateCompatibleDC(hdc: usize) -> usize {
    let hdc = door_windows_abi::widen(hdc);
    door_windows_abi::narrow((move || -> u64 {
        with(0, |s| {
            if hdc != 0 {
                dc(s, hdc)?;
            }
            s.gdi.create_memory_dc().map(Handle::value).map_err(win32)
        })
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn DeleteDC(hdc: usize) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    {
        with(0, |s| {
            let h = dc(s, hdc)?;
            s.gdi.delete_dc(h).map_err(win32)?;
            Ok(1)
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SaveDC(hdc: usize) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    {
        with(0, |s| {
            let h = dc(s, hdc)?;
            s.gdi.save(h).map_err(win32)
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn RestoreDC(hdc: usize, level: i32) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    {
        with(0, |s| {
            let h = dc(s, hdc)?;
            s.gdi.restore(h, level).map_err(win32)?;
            Ok(1)
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GdiFlush() -> i32 {
    // Every GDI operation completes before it returns: nothing is batched.
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetDeviceCaps(hdc: usize, index: i32) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    {
        use door_windows_gui::msg::caps as c;
        with(0, |s| {
            dc(s, hdc)?;
            let (w, h) = s.screen;
            // Millimetres at 96 DPI, in i64: the screen size is not trusted.
            let mm = |v: i32| (i64::from(v) * 254 / 960).clamp(0, i64::from(i32::MAX)) as i32;
            Ok(match index {
                c::DRIVERVERSION => 0x4000,
                c::TECHNOLOGY => 1,
                c::HORZSIZE => mm(w),
                c::VERTSIZE => mm(h),
                c::HORZRES => w,
                c::VERTRES => h,
                c::BITSPIXEL => 32,
                c::PLANES => 1,
                c::NUMCOLORS => -1,
                88 | 90 => 96,
                38 => 0x7e99,
                _ => 0,
            })
        })
    }
}

// ---- objects ------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn GetStockObject(index: i32) -> usize {
    door_windows_abi::narrow((move || -> u64 {
        with(0, |s| {
            s.gdi
                .stock(index)
                .map(Handle::value)
                .ok_or(e::INVALID_PARAMETER)
        })
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn CreatePen(style: i32, width: i32, color: u32) -> usize {
    door_windows_abi::narrow((move || -> u64 {
        with(0, |s| {
            let style = style as u32;
            if style & ps::STYLE_MASK > ps::INSIDEFRAME {
                return Err(e::INVALID_PARAMETER);
            }
            create(
                s,
                Object::Pen(pen(style, width.max(0), color & 0x00ff_ffff)),
            )
        })
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateSolidBrush(color: u32) -> usize {
    door_windows_abi::narrow((move || -> u64 {
        with(0, |s| {
            create(s, Object::Brush(Brush::Solid(color & 0x03ff_ffff)))
        })
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateHatchBrush(style: i32, color: u32) -> usize {
    door_windows_abi::narrow((move || -> u64 {
        with(0, |s| {
            if !(0..=hs::DIAGCROSS as i32).contains(&style) {
                return Err(e::INVALID_PARAMETER);
            }
            create(
                s,
                Object::Brush(Brush::Hatch(style as u32, color & 0x00ff_ffff)),
            )
        })
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn CreatePatternBrush(bitmap: usize) -> usize {
    let bitmap = door_windows_abi::widen(bitmap);
    door_windows_abi::narrow((move || -> u64 {
        with(0, |s| {
            let h = object(s, bitmap)?;
            let mono = s.gdi.is_mono_bitmap(h);
            let State { gdi, memory, .. } = s;
            let (width, height, px) = gdi.bitmap_pixels(h, &mut Ext(memory)).map_err(win32)?;
            create(
                s,
                Object::Brush(Brush::Pattern {
                    width,
                    height,
                    px,
                    mono,
                }),
            )
        })
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateFontIndirectW(logfont: usize) -> usize {
    let logfont = door_windows_abi::widen(logfont);
    door_windows_abi::narrow((move || -> u64 {
        with(0, |s| {
            let mut b = [0u8; 92];
            if !g::read(logfont, &mut b) {
                return Err(e::INVALID_PARAMETER);
            }
            create(s, Object::Font(LogFont::from_bytes(&b).map_err(win32)?))
        })
    })())
}
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn CreateFontW(
    height: i32,
    width: i32,
    escapement: i32,
    orientation: i32,
    weight: i32,
    italic: u32,
    underline: u32,
    strike_out: u32,
    charset: u32,
    _out_precision: u32,
    _clip_precision: u32,
    _quality: u32,
    pitch_family: u32,
    face: usize,
) -> usize {
    let face = door_windows_abi::widen(face);
    door_windows_abi::narrow((move || -> u64 {
        let mut name = [0u16; 32];
        if face != 0 {
            match g::read_wide(face, 31) {
                Some(text) => name[..text.len()].copy_from_slice(&text),
                None => {
                    g::set_last_error(e::INVALID_PARAMETER);
                    return 0;
                }
            }
        }
        with(0, |s| {
            create(
                s,
                Object::Font(LogFont {
                    height,
                    width,
                    escapement,
                    orientation,
                    weight: weight.clamp(0, 1000),
                    italic: italic != 0,
                    underline: underline != 0,
                    strike_out: strike_out != 0,
                    charset: charset as u8,
                    pitch_family: pitch_family as u8,
                    face: name,
                }),
            )
        })
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateCompatibleBitmap(hdc: usize, width: i32, height: i32) -> usize {
    let hdc = door_windows_abi::widen(hdc);
    door_windows_abi::narrow((move || -> u64 {
        with(0, |s| {
            let h = dc(s, hdc)?;
            // Compatible with the DC's bitmap: a memory DC still holding its
            // stock 1x1 bitmap (or any mono bitmap) yields a monochrome bitmap.
            let mono = s.gdi.is_mono_dc(h);
            if width == 0 || height == 0 {
                return Ok(s.gdi.default_bitmap().value());
            }
            if width < 0 || height < 0 {
                return Err(e::INVALID_PARAMETER);
            }
            let surface = if mono {
                Surface::new_mono(width as u32, height as u32)
            } else {
                Surface::new(width as u32, height as u32)
            }
            .map_err(win32)?;
            create_ddb(s, surface)
        })
    })())
}
fn create_ddb(s: &mut State, surface: Surface) -> Result<u64, u32> {
    let bits_pixel = if surface.is_mono() { 1 } else { 32 };
    create(
        s,
        Object::Bitmap(Bitmap {
            pixels: Pixels::Owned(surface),
            bits_pixel,
            dib_address: 0,
        }),
    )
}
/// CreateBitmap/CreateBitmapIndirect: device-dependent bitmaps of one plane
/// at 1 bpp (monochrome) or 32 bpp (the display depth), with optional
/// WORD-aligned initial rows. A zero extent yields the stock 1x1 monochrome
/// bitmap; negative extents count by magnitude.
fn create_bitmap(width: i32, height: i32, planes: u32, bits_pixel: u32, bits: u64) -> u64 {
    if width == 0 || height == 0 {
        return with(0, |s| Ok(s.gdi.default_bitmap().value()));
    }
    let (w, h) = (width.unsigned_abs(), height.unsigned_abs());
    let failure = if planes != 1 {
        Some(e::INVALID_PARAMETER)
    } else if bits_pixel != 1 && bits_pixel != 32 {
        Some(e::NOT_SUPPORTED)
    } else if w > MAX_EDGE || h > MAX_EDGE {
        Some(e::INVALID_PARAMETER)
    } else {
        None
    };
    if let Some(code) = failure {
        g::set_last_error(code);
        return 0;
    }
    let bits_pixel = bits_pixel as u16;
    // Initial rows are captured before the GDI lock is taken.
    let rows = if bits == 0 {
        None
    } else {
        match Surface::checked_count(w, h)
            .ok()
            .and_then(|_| g::read_bytes(bits, ddb_stride(w, bits_pixel) * h as usize))
        {
            Some(b) => Some(b),
            None => {
                g::set_last_error(e::INVALID_PARAMETER);
                return 0;
            }
        }
    };
    let surface = match Surface::from_ddb(w, h, bits_pixel, rows.as_deref()) {
        Ok(v) => v,
        Err(err) => {
            g::set_last_error(win32(err));
            return 0;
        }
    };
    with(0, |s| create_ddb(s, surface))
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateBitmap(
    width: i32,
    height: i32,
    planes: u32,
    bits_pixel: u32,
    bits: usize,
) -> usize {
    let bits = door_windows_abi::widen(bits);
    door_windows_abi::narrow((move || -> u64 {
        create_bitmap(width, height, planes, bits_pixel, bits)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateBitmapIndirect(bitmap: usize) -> usize {
    let bitmap = door_windows_abi::widen(bitmap);
    door_windows_abi::narrow((move || -> u64 {
        let mut b = [0u8; layout::size(&BITMAP)];
        if !g::read(bitmap, &mut b) {
            g::set_last_error(e::INVALID_PARAMETER);
            return 0;
        }
        let i = |at: usize| i32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
        if i(0) != 0 {
            g::set_last_error(e::INVALID_PARAMETER);
            return 0;
        }
        let planes = layout::get(&b, &BITMAP, 4);
        let bits_pixel = layout::get(&b, &BITMAP, 5);
        let bits = layout::get(&b, &BITMAP, 6);
        create_bitmap(i(4), i(8), planes as u32, bits_pixel as u32, bits)
    })())
}
/// CreateDIBSection: 32-bpp sections whose bits this DLL allocates and the
/// application may address directly.
#[unsafe(no_mangle)]
pub extern "system" fn CreateDIBSection(
    hdc: usize,
    info: usize,
    usage: u32,
    bits: usize,
    section: usize,
    offset: u32,
) -> usize {
    let hdc = door_windows_abi::widen(hdc);
    let info = door_windows_abi::widen(info);
    let bits = door_windows_abi::widen(bits);
    let section = door_windows_abi::widen(section);
    door_windows_abi::narrow((move || -> u64 {
        let mut head = [0u8; 40];
        if !g::read(info, &mut head) {
            g::set_last_error(e::INVALID_PARAMETER);
            return 0;
        }
        let parsed = DibInfo::needed(&head)
            .and_then(|n| g::read_bytes(info, n).ok_or(Error::InvalidParameter))
            .and_then(|b| DibInfo::parse(&b, usage));
        let dib = match parsed {
            Ok(d) => d,
            Err(err) => {
                g::set_last_error(win32(err));
                return 0;
            }
        };
        if section != 0 || offset != 0 {
            g::set_last_error(e::NOT_SUPPORTED);
            return 0;
        }
        if dib.bit_count != 32 || dib.masks != (0x00ff_0000, 0x0000_ff00, 0x0000_00ff) {
            // Only the native 32-bpp layout is addressable as one pixel store.
            g::set_last_error(e::NOT_SUPPORTED);
            return 0;
        }
        let bytes = dib.image_bytes();
        let layout = match core::alloc::Layout::from_size_align(bytes.max(4), 16) {
            Ok(l) => l,
            Err(_) => {
                g::set_last_error(e::NOT_ENOUGH_MEMORY);
                return 0;
            }
        };
        // SAFETY: nonzero size; released by DeleteObject with the same layout.
        let address = unsafe { alloc::alloc::alloc_zeroed(layout) } as u64;
        if address == 0 {
            g::set_last_error(e::NOT_ENOUGH_MEMORY);
            return 0;
        }
        let handle = with(0, |s| {
            if hdc != 0 {
                dc(s, hdc)?;
            }
            s.memory.try_reserve(1).map_err(|_| e::NOT_ENOUGH_MEMORY)?;
            let h = create(
                s,
                Object::Bitmap(Bitmap {
                    pixels: Pixels::External {
                        key: address,
                        width: dib.width,
                        height: dib.height,
                        stride: dib.width as usize,
                        flip: !dib.top_down,
                    },
                    bits_pixel: 32,
                    dib_address: address,
                }),
            )?;
            s.memory.push(Memory {
                address,
                bytes,
                owned: true,
            });
            Ok(h)
        });
        if handle == 0 {
            unsafe { alloc::alloc::dealloc(address as *mut u8, layout) };
            return 0;
        }
        if bits != 0 && !g::write_pointer(bits, address) {
            DeleteObject(door_windows_abi::narrow(handle));
            g::set_last_error(e::INVALID_PARAMETER);
            return 0;
        }
        handle
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn DeleteObject(handle: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    {
        let mut release = None;
        let ok = with(0, |s| {
            let h = object(s, handle)?;
            if let Some(Object::Bitmap(Bitmap {
                pixels: Pixels::External { key, .. },
                ..
            })) = s.gdi.delete(h).map_err(win32)?
                && let Some(i) = s.memory.iter().position(|m| m.address == key && m.owned)
            {
                release = Some(s.memory.remove(i));
            }
            Ok(1)
        });
        if let Some(m) = release
            && let Ok(layout) = core::alloc::Layout::from_size_align(m.bytes.max(4), 16)
        {
            // SAFETY: allocated by CreateDIBSection with this layout and now
            // unreachable through any GDI object.
            unsafe { alloc::alloc::dealloc(m.address as *mut u8, layout) };
        }
        ok
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SelectObject(hdc: usize, handle: usize) -> usize {
    let hdc = door_windows_abi::widen(hdc);
    let handle = door_windows_abi::widen(handle);
    door_windows_abi::narrow((move || -> u64 {
        with(0, |s| {
            let d = dc(s, hdc)?;
            let h = object(s, handle)?;
            if let Some(Object::Region(r)) = s.gdi.object(h) {
                let r = r.clone();
                return s
                    .gdi
                    .select_clip(d, Some(&r), Op::Copy)
                    .map(|kind| kind as u64)
                    .map_err(win32);
            }
            s.gdi.select(d, h).map(Handle::value).map_err(win32)
        })
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentObject(hdc: usize, kind: u32) -> usize {
    let hdc = door_windows_abi::widen(hdc);
    door_windows_abi::narrow((move || -> u64 {
        with(0, |s| {
            let st = s.gdi.state(dc(s, hdc)?).map_err(win32)?;
            Ok(match kind {
                obj::PEN => st.pen.value(),
                obj::BRUSH => st.brush.value(),
                obj::FONT => st.font.value(),
                obj::BITMAP => st.bitmap.map_or(0, Handle::value),
                _ => return Err(e::INVALID_PARAMETER),
            })
        })
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn GetObjectType(handle: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    {
        with(0, |s| {
            if let Some(h) = s.gdi.resolve_dc(handle) {
                let memory = s
                    .gdi
                    .dc(h)
                    .is_some_and(|d| d.target == door_windows_gui::gdi::Target::Memory);
                return Ok(if memory { obj::MEMDC } else { obj::DC });
            }
            let h = object(s, handle)?;
            Ok(s.gdi.object(h).map_or(0, Object::type_code))
        })
    }
}
/// GetObjectW for bitmaps (BITMAP), pens (LOGPEN), brushes (LOGBRUSH) and
/// fonts (LOGFONTW).
#[unsafe(no_mangle)]
pub extern "system" fn GetObjectW(handle: usize, size: i32, out: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let out = door_windows_abi::widen(out);
    {
        let data: Option<Vec<u8>> = with(None, |s| {
            let h = object(s, handle)?;
            let mut b = Vec::new();
            match s.gdi.object(h) {
                Some(Object::Bitmap(bm)) => {
                    let (w, hh) = bm.size();
                    // bmWidthBytes: WORD-aligned rows (1 bpp: (w + 15) / 16 * 2).
                    let stride = ddb_stride(w, bm.bits_pixel) as u32;
                    b.resize(layout::size(&BITMAP), 0);
                    for (index, value) in [
                        0,
                        u64::from(w),
                        u64::from(hh),
                        u64::from(stride),
                        1,
                        u64::from(bm.bits_pixel),
                        bm.dib_address,
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        layout::set(&mut b, &BITMAP, index, value);
                    }
                }
                Some(Object::Pen(p)) => {
                    for v in [p.style, p.width as u32, 0, p.color] {
                        b.extend_from_slice(&v.to_le_bytes());
                    }
                }
                Some(Object::Brush(br)) => {
                    let (style, color, hatch) = match br {
                        Brush::Solid(c) => (0u32, *c, 0u64),
                        Brush::Null => (1, 0, 0),
                        Brush::Hatch(h, c) => (2, *c, u64::from(*h)),
                        Brush::Pattern { .. } => (3, 0, 0),
                        Brush::DcBrush => (0, 0xff_ffff, 0),
                    };
                    b.resize(layout::size(&LOGBRUSH), 0);
                    for (index, value) in [u64::from(style), u64::from(color), hatch]
                        .into_iter()
                        .enumerate()
                    {
                        layout::set(&mut b, &LOGBRUSH, index, value);
                    }
                }
                Some(Object::Font(f)) => b.extend_from_slice(&f.to_bytes()),
                _ => return Err(e::INVALID_PARAMETER),
            }
            Ok(Some(b))
        });
        let Some(data) = data else { return 0 };
        if out == 0 {
            return data.len() as i32;
        }
        let n = (size.max(0) as usize).min(data.len());
        if !g::write(out, &data[..n]) {
            g::set_last_error(e::INVALID_PARAMETER);
            return 0;
        }
        n as i32
    }
}

// ---- attributes -------------------------------------------------------------------------

fn set_state(
    hdc: u64,
    fail: u32,
    f: impl FnOnce(&mut door_windows_gui::gdi::DcState) -> Result<u32, u32>,
) -> u32 {
    with(fail, |s| {
        let h = dc(s, hdc)?;
        f(s.gdi.state_mut(h).map_err(win32)?)
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn SetBkColor(hdc: usize, color: u32) -> u32 {
    let hdc = door_windows_abi::widen(hdc);
    {
        set_state(hdc, CLR_INVALID, |st| {
            Ok(core::mem::replace(&mut st.bk_color, color & 0x03ff_ffff))
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetBkColor(hdc: usize) -> u32 {
    let hdc = door_windows_abi::widen(hdc);
    set_state(hdc, CLR_INVALID, |st| Ok(st.bk_color))
}
#[unsafe(no_mangle)]
pub extern "system" fn SetTextColor(hdc: usize, color: u32) -> u32 {
    let hdc = door_windows_abi::widen(hdc);
    {
        set_state(hdc, CLR_INVALID, |st| {
            Ok(core::mem::replace(&mut st.text_color, color & 0x03ff_ffff))
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetTextColor(hdc: usize) -> u32 {
    let hdc = door_windows_abi::widen(hdc);
    set_state(hdc, CLR_INVALID, |st| Ok(st.text_color))
}
#[unsafe(no_mangle)]
pub extern "system" fn SetBkMode(hdc: usize, mode: i32) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    {
        set_state(hdc, 0, |st| {
            if mode != bk::TRANSPARENT && mode != bk::OPAQUE {
                return Err(e::INVALID_PARAMETER);
            }
            Ok(core::mem::replace(&mut st.bk_mode, mode) as u32)
        }) as i32
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetBkMode(hdc: usize) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    set_state(hdc, 0, |st| Ok(st.bk_mode as u32)) as i32
}
#[unsafe(no_mangle)]
pub extern "system" fn SetROP2(hdc: usize, mode: i32) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    {
        set_state(hdc, 0, |st| {
            if !(1..=16).contains(&mode) {
                return Err(e::INVALID_PARAMETER);
            }
            Ok(core::mem::replace(&mut st.rop2, mode as u32))
        }) as i32
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetROP2(hdc: usize) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    set_state(hdc, 0, |st| Ok(st.rop2)) as i32
}
#[unsafe(no_mangle)]
pub extern "system" fn SetTextAlign(hdc: usize, align: u32) -> u32 {
    let hdc = door_windows_abi::widen(hdc);
    {
        set_state(hdc, 0xffff_ffff, |st| {
            Ok(core::mem::replace(&mut st.text_align, align))
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SetStretchBltMode(hdc: usize, mode: i32) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    {
        set_state(hdc, 0, |st| {
            if !(1..=4).contains(&mode) {
                return Err(e::INVALID_PARAMETER);
            }
            Ok(core::mem::replace(&mut st.stretch_mode, mode) as u32)
        }) as i32
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SetDCBrushColor(hdc: usize, color: u32) -> u32 {
    let hdc = door_windows_abi::widen(hdc);
    {
        set_state(hdc, CLR_INVALID, |st| {
            Ok(core::mem::replace(&mut st.dc_brush, color & 0x00ff_ffff))
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SetDCPenColor(hdc: usize, color: u32) -> u32 {
    let hdc = door_windows_abi::widen(hdc);
    {
        set_state(hdc, CLR_INVALID, |st| {
            Ok(core::mem::replace(&mut st.dc_pen, color & 0x00ff_ffff))
        })
    }
}
fn point_out(out: u64, p: Point) -> bool {
    out == 0 || g::write_i32s(out, &[p.x, p.y])
}
fn size_out(out: u64, s: Size) -> bool {
    out == 0 || g::write_i32s(out, &[s.cx, s.cy])
}
fn with_map(
    hdc: u64,
    out: u64,
    f: impl FnOnce(&mut door_windows_gui::units::Mapping) -> Result<Point, u32>,
) -> i32 {
    let old = with(None, |s| {
        let h = dc(s, hdc)?;
        f(&mut s.gdi.state_mut(h).map_err(win32)?.map).map(Some)
    });
    match old {
        Some(p) if point_out(out, p) => 1,
        Some(_) => {
            g::set_last_error(e::INVALID_PARAMETER);
            0
        }
        None => 0,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SetViewportOrgEx(hdc: usize, x: i32, y: i32, old: usize) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let old = door_windows_abi::widen(old);
    {
        with_map(hdc, old, |m| {
            Ok(core::mem::replace(&mut m.viewport_org, Point::new(x, y)))
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn OffsetViewportOrgEx(hdc: usize, x: i32, y: i32, old: usize) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let old = door_windows_abi::widen(old);
    {
        with_map(hdc, old, |m| {
            let p = m.viewport_org;
            m.viewport_org = p.offset(x, y);
            Ok(p)
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetViewportOrgEx(hdc: usize, out: usize) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let out = door_windows_abi::widen(out);
    with_map(hdc, out, |m| Ok(m.viewport_org))
}
#[unsafe(no_mangle)]
pub extern "system" fn SetWindowOrgEx(hdc: usize, x: i32, y: i32, old: usize) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let old = door_windows_abi::widen(old);
    {
        with_map(hdc, old, |m| {
            Ok(core::mem::replace(&mut m.window_org, Point::new(x, y)))
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetWindowOrgEx(hdc: usize, out: usize) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let out = door_windows_abi::widen(out);
    with_map(hdc, out, |m| Ok(m.window_org))
}
#[unsafe(no_mangle)]
pub extern "system" fn SetBrushOrgEx(hdc: usize, x: i32, y: i32, old: usize) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let old = door_windows_abi::widen(old);
    {
        let p = with(None, |s| {
            let h = dc(s, hdc)?;
            let st = s.gdi.state_mut(h).map_err(win32)?;
            Ok(Some(core::mem::replace(
                &mut st.brush_org,
                Point::new(x, y),
            )))
        });
        match p {
            Some(p) if point_out(old, p) => 1,
            _ => 0,
        }
    }
}
fn ext(hdc: u64, x: i32, y: i32, old: u64, viewport: bool) -> i32 {
    let r = with(None, |s| {
        let h = dc(s, hdc)?;
        let m = &mut s.gdi.state_mut(h).map_err(win32)?.map;
        let prev = if viewport {
            m.viewport_ext
        } else {
            m.window_ext
        };
        let ok = if viewport {
            m.set_viewport_ext(Size::new(x, y))
        } else {
            m.set_window_ext(Size::new(x, y))
        };
        if !ok {
            return Err(e::INVALID_PARAMETER);
        }
        Ok(Some(prev))
    });
    match r {
        Some(p) if size_out(old, p) => 1,
        _ => 0,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SetWindowExtEx(hdc: usize, x: i32, y: i32, old: usize) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let old = door_windows_abi::widen(old);
    ext(hdc, x, y, old, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn SetViewportExtEx(hdc: usize, x: i32, y: i32, old: usize) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let old = door_windows_abi::widen(old);
    ext(hdc, x, y, old, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn SetMapMode(hdc: usize, mode: i32) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    {
        set_state(hdc, 0, |st| {
            st.map
                .set_mode(mode)
                .map(|o| o as u32)
                .ok_or(e::INVALID_PARAMETER)
        }) as i32
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetMapMode(hdc: usize) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    set_state(hdc, 0, |st| Ok(st.map.mode as u32)) as i32
}
fn convert(hdc: u64, points: u64, count: i32, to_device: bool) -> i32 {
    if !(0..=4096).contains(&count) {
        g::set_last_error(e::INVALID_PARAMETER);
        return 0;
    }
    let Some(bytes) = g::read_bytes(points, count as usize * 8) else {
        g::set_last_error(e::INVALID_PARAMETER);
        return 0;
    };
    let map = with(None, |s| {
        Ok(Some(s.gdi.state(dc(s, hdc)?).map_err(win32)?.map))
    });
    let Some(map) = map else { return 0 };
    let mut out = Vec::with_capacity(bytes.len());
    for p in bytes.chunks_exact(8) {
        let pt = Point::new(
            i32::from_le_bytes([p[0], p[1], p[2], p[3]]),
            i32::from_le_bytes([p[4], p[5], p[6], p[7]]),
        );
        let q = if to_device {
            map.lp_to_dp(pt)
        } else {
            map.dp_to_lp(pt)
        };
        out.extend_from_slice(&q.x.to_le_bytes());
        out.extend_from_slice(&q.y.to_le_bytes());
    }
    i32::from(g::write(points, &out))
}
#[unsafe(no_mangle)]
pub extern "system" fn LPtoDP(hdc: usize, points: usize, count: i32) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let points = door_windows_abi::widen(points);
    convert(hdc, points, count, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn DPtoLP(hdc: usize, points: usize, count: i32) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let points = door_windows_abi::widen(points);
    convert(hdc, points, count, false)
}

// ---- drawing ------------------------------------------------------------------------------

fn draw(hdc: u64, f: impl FnOnce(&mut Gdi, Handle, &mut Ext) -> Result<(), Error>) -> i32 {
    with(0, |s| {
        let h = dc(s, hdc)?;
        let State { gdi, memory, .. } = s;
        f(gdi, h, &mut Ext(memory)).map_err(win32)?;
        Ok(1)
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn MoveToEx(hdc: usize, x: i32, y: i32, old: usize) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let old = door_windows_abi::widen(old);
    {
        let p = with(None, |s| {
            let h = dc(s, hdc)?;
            s.gdi.move_to(h, Point::new(x, y)).map(Some).map_err(win32)
        });
        match p {
            Some(p) if point_out(old, p) => 1,
            _ => 0,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn LineTo(hdc: usize, x: i32, y: i32) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    draw(hdc, |g, h, x_| g.line_to(h, Point::new(x, y), x_))
}
#[unsafe(no_mangle)]
pub extern "system" fn Rectangle(hdc: usize, left: i32, top: i32, right: i32, bottom: i32) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    {
        draw(hdc, |g, h, x| {
            g.rectangle(h, Rect::new(left, top, right, bottom), x)
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn PatBlt(hdc: usize, x: i32, y: i32, w: i32, h: i32, code: u32) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    {
        draw(hdc, |g, d, ext| {
            g.pat_blt(d, Rect::from_size(x, y, w, h).normalized(), code, ext)
        })
    }
}
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn BitBlt(
    dst: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    src: usize,
    sx: i32,
    sy: i32,
    code: u32,
) -> i32 {
    let dst = door_windows_abi::widen(dst);
    let src = door_windows_abi::widen(src);
    {
        with(0, |s| {
            let d = dc(s, dst)?;
            let source = if src == 0 {
                None
            } else {
                Some((dc(s, src)?, Point::new(sx, sy)))
            };
            let State { gdi, memory, .. } = s;
            gdi.bit_blt(
                d,
                Rect::from_size(x, y, w, h),
                source,
                code,
                &mut Ext(memory),
            )
            .map_err(win32)?;
            Ok(1)
        })
    }
}
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn StretchBlt(
    dst: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    src: usize,
    sx: i32,
    sy: i32,
    sw: i32,
    sh: i32,
    code: u32,
) -> i32 {
    let dst = door_windows_abi::widen(dst);
    let src = door_windows_abi::widen(src);
    {
        with(0, |s| {
            let d = dc(s, dst)?;
            let source = dc(s, src)?;
            let State { gdi, memory, .. } = s;
            gdi.stretch_blt(
                d,
                (x, y, w, h),
                source,
                (sx, sy, sw, sh),
                code,
                &mut Ext(memory),
            )
            .map_err(win32)?;
            Ok(1)
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetPixel(hdc: usize, x: i32, y: i32) -> u32 {
    let hdc = door_windows_abi::widen(hdc);
    {
        let mut pixel = CLR_INVALID;
        draw(hdc, |g, h, ext| {
            // Outside the clip region or surface GetPixel reports CLR_INVALID
            // without failing the call.
            pixel = g.get_pixel(h, Point::new(x, y), ext).unwrap_or(CLR_INVALID);
            Ok(())
        });
        pixel
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SetPixel(hdc: usize, x: i32, y: i32, color: u32) -> u32 {
    let hdc = door_windows_abi::widen(hdc);
    {
        let mut out = CLR_INVALID;
        draw(hdc, |g, h, ext| {
            out = g
                .set_pixel(h, Point::new(x, y), color, ext)
                .unwrap_or(CLR_INVALID);
            Ok(())
        });
        out
    }
}
/// FillRect's GDI half: user32 resolves (COLOR_* + 1) brushes first.
#[unsafe(no_mangle)]
pub extern "system" fn DoorGdiFillRect(hdc: u64, rect: u64, brush: u64) -> i32 {
    let Some([l, t, r, b]) = g::read_i32s::<4>(rect) else {
        g::set_last_error(e::INVALID_PARAMETER);
        return 0;
    };
    with(0, |s| {
        let h = dc(s, hdc)?;
        let br = object(s, brush)?;
        let State { gdi, memory, .. } = s;
        gdi.fill_rect(h, Rect::new(l, t, r, b), br, &mut Ext(memory))
            .map_err(win32)?;
        Ok(1)
    })
}

// ---- text -----------------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn TextOutW(hdc: usize, x: i32, y: i32, text: usize, count: i32) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let text = door_windows_abi::widen(text);
    ExtTextOutW(
        door_windows_abi::narrow(hdc),
        x,
        y,
        0,
        0,
        door_windows_abi::narrow(text),
        count.max(0) as u32,
        0,
    )
}
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn ExtTextOutW(
    hdc: usize,
    x: i32,
    y: i32,
    options: u32,
    rect: usize,
    string: usize,
    count: u32,
    dx: usize,
) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let rect = door_windows_abi::widen(rect);
    let string = door_windows_abi::widen(string);
    let dx = door_windows_abi::widen(dx);
    {
        use door_windows_gui::msg::eto;
        if count > 8192 || options & (eto::GLYPH_INDEX | eto::PDY) != 0 {
            g::set_last_error(if count > 8192 {
                e::INVALID_PARAMETER
            } else {
                e::NOT_SUPPORTED
            });
            return 0;
        }
        let units = if count == 0 {
            Some(Vec::new())
        } else {
            g::read_wide_n(string, count as usize)
        };
        let Some(units) = units else {
            g::set_last_error(e::INVALID_PARAMETER);
            return 0;
        };
        // Explicit advances (lpDx) place every character separately.
        let advances: Option<Vec<i32>> = if dx != 0 && count != 0 {
            match g::read_bytes(dx, count as usize * 4) {
                Some(b) => Some(
                    b.chunks_exact(4)
                        .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                        .collect(),
                ),
                None => {
                    g::set_last_error(e::INVALID_PARAMETER);
                    return 0;
                }
            }
        } else {
            None
        };
        let r = if rect != 0 && options & (eto::OPAQUE | eto::CLIPPED) != 0 {
            match g::read_i32s::<4>(rect) {
                Some([l, t, rr, b]) => Some(Rect::new(l, t, rr, b)),
                None => {
                    g::set_last_error(e::INVALID_PARAMETER);
                    return 0;
                }
            }
        } else {
            None
        };
        with(0, |s| {
            let h = dc(s, hdc)?;
            let opaque = r.filter(|_| options & eto::OPAQUE != 0);
            let clipped = r.filter(|_| options & eto::CLIPPED != 0);
            if units.is_empty() && opaque.is_none() {
                return Ok(1);
            }
            text(s)?;
            let State {
                gdi, memory, text, ..
            } = s;
            let backend: &dyn TextBackend = text.as_ref().unwrap();
            match advances {
                None => gdi
                    .text_out(
                        h,
                        Point::new(x, y),
                        &units,
                        opaque,
                        clipped,
                        backend,
                        &mut Ext(memory),
                    )
                    .map_err(win32)?,
                Some(adv) => {
                    if let Some(o) = opaque {
                        gdi.text_out(
                            h,
                            Point::new(x, y),
                            &[],
                            Some(o),
                            clipped,
                            backend,
                            &mut Ext(memory),
                        )
                        .map_err(win32)?;
                    }
                    let mut at = x;
                    for (u, a) in units.iter().zip(adv) {
                        gdi.text_out(
                            h,
                            Point::new(at, y),
                            &[*u],
                            None,
                            clipped,
                            backend,
                            &mut Ext(memory),
                        )
                        .map_err(win32)?;
                        at = at.saturating_add(a);
                    }
                }
            }
            Ok(1)
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetTextExtentPoint32W(
    hdc: usize,
    text_ptr: usize,
    count: i32,
    size: usize,
) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let text_ptr = door_windows_abi::widen(text_ptr);
    let size = door_windows_abi::widen(size);
    {
        if !(0..=8192).contains(&count) {
            g::set_last_error(e::INVALID_PARAMETER);
            return 0;
        }
        let units = if count == 0 {
            Some(Vec::new())
        } else {
            g::read_wide_n(text_ptr, count as usize)
        };
        let Some(units) = units else {
            g::set_last_error(e::INVALID_PARAMETER);
            return 0;
        };
        let extent = with(None, |s| {
            let h = dc(s, hdc)?;
            text(s)?;
            let backend: &dyn TextBackend = s.text.as_ref().unwrap();
            s.gdi
                .text_extent(h, &units, backend)
                .map(Some)
                .map_err(win32)
        });
        match extent {
            Some(sz) if g::write_i32s(size, &[sz.cx, sz.cy]) => 1,
            Some(_) => {
                g::set_last_error(e::INVALID_PARAMETER);
                0
            }
            None => 0,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetTextExtentPointW(
    hdc: usize,
    text_ptr: usize,
    count: i32,
    size: usize,
) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let text_ptr = door_windows_abi::widen(text_ptr);
    let size = door_windows_abi::widen(size);
    GetTextExtentPoint32W(
        door_windows_abi::narrow(hdc),
        door_windows_abi::narrow(text_ptr),
        count,
        door_windows_abi::narrow(size),
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn GetTextMetricsW(hdc: usize, out: usize) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let out = door_windows_abi::widen(out);
    {
        let m = with(None, |s| {
            let h = dc(s, hdc)?;
            text(s)?;
            let backend: &dyn TextBackend = s.text.as_ref().unwrap();
            s.gdi.text_metrics(h, backend).map(Some).map_err(win32)
        });
        match m {
            Some(m) if g::write(out, &m.to_bytes()) => 1,
            Some(_) => {
                g::set_last_error(e::INVALID_PARAMETER);
                0
            }
            None => 0,
        }
    }
}
/// Per-character advances of a string in the DC's font (user32 controls).
#[unsafe(no_mangle)]
pub extern "system" fn DoorGdiTextAdvances(hdc: u64, text_ptr: u64, count: u32, out: u64) -> i32 {
    if count > 8192 {
        return 0;
    }
    let Some(units) = (if count == 0 {
        Some(Vec::new())
    } else {
        g::read_wide_n(text_ptr, count as usize)
    }) else {
        return 0;
    };
    let adv = with(None, |s| {
        let h = dc(s, hdc)?;
        let font = s.gdi.font(h).map_err(win32)?;
        let backend: &dyn TextBackend = text(s)?;
        Ok(Some(backend.advances(&font, &units)))
    });
    let Some(adv) = adv else { return 0 };
    let bytes: Vec<u8> = adv.iter().flat_map(|v| v.to_le_bytes()).collect();
    i32::from(g::write(out, &bytes))
}

// ---- regions and clipping -----------------------------------------------------------------

fn region(s: &State, value: u64) -> Result<(Handle, Region), u32> {
    let h = object(s, value)?;
    match s.gdi.object(h) {
        Some(Object::Region(r)) => Ok((h, r.clone())),
        _ => Err(e::INVALID_HANDLE),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateRectRgn(left: i32, top: i32, right: i32, bottom: i32) -> usize {
    door_windows_abi::narrow((move || -> u64 {
        with(0, |s| {
            create(
                s,
                Object::Region(Region::from_rect(Rect::new(left, top, right, bottom))),
            )
        })
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateRectRgnIndirect(rect: usize) -> usize {
    let rect = door_windows_abi::widen(rect);
    door_windows_abi::narrow((move || -> u64 {
        match g::read_i32s::<4>(rect) {
            Some([l, t, r, b]) => door_windows_abi::widen(CreateRectRgn(l, t, r, b)),
            None => {
                g::set_last_error(e::INVALID_PARAMETER);
                0
            }
        }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn SetRectRgn(rgn: usize, left: i32, top: i32, right: i32, bottom: i32) -> i32 {
    let rgn = door_windows_abi::widen(rgn);
    {
        with(0, |s| {
            let (h, _) = region(s, rgn)?;
            *s.gdi.object_mut(h).unwrap() =
                Object::Region(Region::from_rect(Rect::new(left, top, right, bottom)));
            Ok(1)
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CombineRgn(dst: usize, a: usize, b: usize, mode: i32) -> i32 {
    let dst = door_windows_abi::widen(dst);
    let a = door_windows_abi::widen(a);
    let b = door_windows_abi::widen(b);
    {
        with(ERROR_REGION, |s| {
            let op = Op::from_win32(mode).ok_or(e::INVALID_PARAMETER)?;
            let (d, _) = region(s, dst)?;
            let (_, ra) = region(s, a)?;
            let result = if op == Op::Copy {
                ra
            } else {
                let (_, rb) = region(s, b)?;
                ra.combine(&rb, op).map_err(win32)?
            };
            let kind = result.kind();
            *s.gdi.object_mut(d).unwrap() = Object::Region(result);
            Ok(kind)
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn OffsetRgn(rgn: usize, x: i32, y: i32) -> i32 {
    let rgn = door_windows_abi::widen(rgn);
    {
        with(ERROR_REGION, |s| {
            let (h, mut r) = region(s, rgn)?;
            r.offset(x, y);
            let kind = r.kind();
            *s.gdi.object_mut(h).unwrap() = Object::Region(r);
            Ok(kind)
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn PtInRegion(rgn: usize, x: i32, y: i32) -> i32 {
    let rgn = door_windows_abi::widen(rgn);
    {
        with(0, |s| {
            Ok(i32::from(region(s, rgn)?.1.contains(Point::new(x, y))))
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn RectInRegion(rgn: usize, rect: usize) -> i32 {
    let rgn = door_windows_abi::widen(rgn);
    let rect = door_windows_abi::widen(rect);
    {
        let Some([l, t, r, b]) = g::read_i32s::<4>(rect) else {
            g::set_last_error(e::INVALID_PARAMETER);
            return 0;
        };
        with(0, |s| {
            Ok(i32::from(
                region(s, rgn)?.1.intersects(Rect::new(l, t, r, b)),
            ))
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn EqualRgn(a: usize, b: usize) -> i32 {
    let a = door_windows_abi::widen(a);
    let b = door_windows_abi::widen(b);
    {
        with(ERROR_REGION, |s| {
            Ok(i32::from(region(s, a)?.1 == region(s, b)?.1))
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetRgnBox(rgn: usize, out: usize) -> i32 {
    let rgn = door_windows_abi::widen(rgn);
    let out = door_windows_abi::widen(out);
    {
        let r = with(None, |s| {
            let (_, r) = region(s, rgn)?;
            Ok(Some((r.kind(), r.bounds())))
        });
        match r {
            Some((kind, b)) if g::write_i32s(out, &[b.left, b.top, b.right, b.bottom]) => kind,
            _ => ERROR_REGION,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SelectClipRgn(hdc: usize, rgn: usize) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let rgn = door_windows_abi::widen(rgn);
    ExtSelectClipRgn(
        door_windows_abi::narrow(hdc),
        door_windows_abi::narrow(rgn),
        5,
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn ExtSelectClipRgn(hdc: usize, rgn: usize, mode: i32) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let rgn = door_windows_abi::widen(rgn);
    {
        with(ERROR_REGION, |s| {
            let h = dc(s, hdc)?;
            let op = Op::from_win32(mode).ok_or(e::INVALID_PARAMETER)?;
            let r = if rgn == 0 {
                None
            } else {
                Some(region(s, rgn)?.1)
            };
            s.gdi.select_clip(h, r.as_ref(), op).map_err(win32)
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn IntersectClipRect(
    hdc: usize,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    {
        with(ERROR_REGION, |s| {
            let h = dc(s, hdc)?;
            s.gdi
                .clip_rect(h, Rect::new(left, top, right, bottom), false)
                .map_err(win32)
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ExcludeClipRect(
    hdc: usize,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    {
        with(ERROR_REGION, |s| {
            let h = dc(s, hdc)?;
            s.gdi
                .clip_rect(h, Rect::new(left, top, right, bottom), true)
                .map_err(win32)
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetClipBox(hdc: usize, out: usize) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let out = door_windows_abi::widen(out);
    {
        let r = with(None, |s| {
            let h = dc(s, hdc)?;
            let (kind, b) = s.gdi.clip_box(h).map_err(win32)?;
            let map = s.gdi.state(h).map_err(win32)?.map;
            let a = map.dp_to_lp(b.top_left());
            let z = map.dp_to_lp(Point::new(b.right, b.bottom));
            Ok(Some((kind, Rect::new(a.x, a.y, z.x, z.y).normalized())))
        });
        match r {
            Some((kind, b)) if g::write_i32s(out, &[b.left, b.top, b.right, b.bottom]) => kind,
            _ => ERROR_REGION,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn FillRgn(hdc: usize, rgn: usize, brush: usize) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let rgn = door_windows_abi::widen(rgn);
    let brush = door_windows_abi::widen(brush);
    {
        with(0, |s| {
            let h = dc(s, hdc)?;
            let (_, r) = region(s, rgn)?;
            let br = object(s, brush)?;
            let State { gdi, memory, .. } = s;
            gdi.fill_region(h, &r, br, &mut Ext(memory))
                .map_err(win32)?;
            Ok(1)
        })
    }
}

// ---- private interface for user32 -----------------------------------------------------------

/// Register a window back buffer user32 allocated from the process heap
/// (`bytes` of 32-bpp pixels): it must be exactly one live heap block of that
/// size starting at `address`, overlapping no memory GDI already draws into.
#[unsafe(no_mangle)]
pub extern "system" fn DoorGdiRegisterSurface(address: u64, bytes: u64) -> i32 {
    with(0, |s| {
        let end = address.checked_add(bytes).ok_or(e::INVALID_PARAMETER)?;
        if address == 0
            || !address.is_multiple_of(4)
            || bytes == 0
            || bytes > isize::MAX as u64
            || !live_heap_block(address, bytes as usize)
        {
            return Err(e::INVALID_PARAMETER);
        }
        // Re-registering the same buffer replaces it; anything else that
        // overlaps a DIB section or another buffer is refused.
        if s.memory
            .iter()
            .any(|m| m.overlaps(address, end) && (m.owned || m.address != address))
        {
            return Err(e::INVALID_PARAMETER);
        }
        s.memory.try_reserve(1).map_err(|_| e::NOT_ENOUGH_MEMORY)?;
        s.memory.retain(|m| m.address != address || m.owned);
        s.memory.push(Memory {
            address,
            bytes: bytes as usize,
            owned: false,
        });
        Ok(1)
    })
}
/// Release a registered back buffer: DCs still naming it stop drawing.
#[unsafe(no_mangle)]
pub extern "system" fn DoorGdiReleaseSurface(address: u64) -> i32 {
    with(0, |s| {
        let i = s
            .memory
            .iter()
            .position(|m| m.address == address && !m.owned)
            .ok_or(e::INVALID_PARAMETER)?;
        s.gdi.retarget(address, None);
        s.memory.remove(i);
        Ok(1)
    })
}
/// A DC on a registered back buffer: `origin` is the DC's device origin in
/// surface pixels and `rects` (user32 memory) its visible region.
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn DoorGdiCreateSurfaceDC(
    address: u64,
    width: u32,
    height: u32,
    stride: u32,
    origin_x: i32,
    origin_y: i32,
    rects: u64,
    count: u32,
) -> u64 {
    if count > 4096 {
        g::set_last_error(e::INVALID_PARAMETER);
        return 0;
    }
    let Some(bytes) = g::read_bytes(rects, count as usize * 16) else {
        g::set_last_error(e::INVALID_PARAMETER);
        return 0;
    };
    let list: Vec<Rect> = bytes
        .chunks_exact(16)
        .map(|c| Rect::from_bytes(c.try_into().unwrap()))
        .collect();
    with(0, |s| {
        // Only a registered back buffer, never DIB-section memory: a DIB is
        // drawn through a memory DC that keeps it selected (and alive).
        let m = s
            .memory
            .iter()
            .find(|m| m.address == address && !m.owned)
            .ok_or(e::INVALID_HANDLE)?;
        let need = (u64::from(stride) * u64::from(height.saturating_sub(1)))
            .checked_add(u64::from(width))
            .and_then(|v| v.checked_mul(4));
        if need.is_none_or(|n| n > m.bytes as u64) || stride < width {
            return Err(e::INVALID_PARAMETER);
        }
        let visible = Region::from_rects(&list).map_err(win32)?;
        s.gdi
            .create_surface_dc(
                address,
                width,
                height,
                stride as usize,
                Point::new(origin_x, origin_y),
                visible,
            )
            .map(Handle::value)
            .map_err(win32)
    })
}
/// An information DC (GetDC(NULL)): metrics and objects, no drawing.
#[unsafe(no_mangle)]
pub extern "system" fn DoorGdiCreateDisplayDC() -> u64 {
    with(0, |s| {
        s.gdi.create_display_dc().map(Handle::value).map_err(win32)
    })
}
/// GetGuiResources(GR_GDIOBJECTS).
#[unsafe(no_mangle)]
pub extern "system" fn DoorGdiObjectCount() -> u32 {
    with(0, |s| Ok(s.gdi.object_count() as u32))
}
/// The output size behind HORZRES/VERTRES (bounded like any surface).
#[unsafe(no_mangle)]
pub extern "system" fn DoorGdiSetScreen(width: i32, height: i32) -> i32 {
    with(0, |s| {
        let edge = |v: i32| v.clamp(1, MAX_EDGE as i32);
        s.screen = (edge(width), edge(height));
        Ok(1)
    })
}
/// GetSysColorBrush: permanent solid brushes of the system colors.
static SYS_BRUSHES: g::Lock<[u64; 31]> = g::Lock::new([0; 31]);
/// GetSysColorBrush's GDI half.
#[unsafe(no_mangle)]
pub extern "system" fn DoorGdiSysColorBrush(index: i32) -> u64 {
    let Some(color) = door_windows_gui::metrics::sys_color(index) else {
        g::set_last_error(e::INVALID_PARAMETER);
        return 0;
    };
    let mut table = SYS_BRUSHES.lock();
    let Some(slot) = table.get_mut(index as usize) else {
        return 0;
    };
    if *slot == 0 {
        *slot = with(0, |s| {
            s.gdi
                .create_permanent(Object::Brush(Brush::Solid(color)))
                .map(Handle::value)
                .map_err(win32)
        });
    }
    *slot
}
/// The map mode constants are validated by the core; referenced here so a
/// change of their values breaks the build rather than the ABI.
const _: () =
    assert!(mm::TEXT == 1 && rop::SRCCOPY == 0x00cc_0020 && ColorRef::WHITE.0 == 0xff_ffff);

// ---- DIBs to and from devices, alpha blending, icons ---------------------------

/// A `BITMAPINFO` in application memory.
fn dib_info(info: u64, usage: u32) -> Result<DibInfo, u32> {
    let mut head = [0u8; 40];
    if !g::read(info, &mut head) {
        return Err(e::INVALID_PARAMETER);
    }
    DibInfo::needed(&head)
        .and_then(|n| g::read_bytes(info, n).ok_or(Error::InvalidParameter))
        .and_then(|b| DibInfo::parse(&b, usage))
        .map_err(win32)
}
/// Rows `first..first + count` (stored order: from the bottom of a
/// bottom-up DIB) of the bits at `bits`, decoded into a top-down block
/// placed where those rows sit in the whole image.
fn dib_rows(
    dib: &DibInfo,
    bits: u64,
    first: u32,
    count: u32,
) -> Result<door_windows_gui::raster::Block, u32> {
    let first = first.min(dib.height);
    let count = count.min(dib.height - first);
    let stride = dib.stride();
    let data = g::read_bytes(bits, stride * count as usize).ok_or(e::INVALID_PARAMETER)?;
    let w = dib.width as usize;
    let mut px = Vec::new();
    px.try_reserve_exact(w * count as usize)
        .map_err(|_| e::NOT_ENOUGH_MEMORY)?;
    px.resize(w * count as usize, 0);
    for i in 0..count as usize {
        // Stored row i is image row first + i, counted from the top of a
        // top-down DIB or from the bottom of a bottom-up one.
        let to = if dib.top_down {
            i
        } else {
            count as usize - 1 - i
        };
        dib.decode_row(&data[i * stride..][..stride], &mut px[to * w..][..w]);
    }
    let top = if dib.top_down {
        first
    } else {
        dib.height - first - count
    };
    Ok(door_windows_gui::raster::Block {
        origin: Point::new(0, top as i32),
        width: dib.width,
        height: count,
        px,
    })
}
/// The top-down row of DIB source y `y` for `rows` rows (a bottom-up DIB
/// counts source coordinates from its bottom edge).
fn dib_top(dib: &DibInfo, y: i32, rows: i32) -> i32 {
    if dib.top_down {
        y
    } else {
        dib.height as i32 - y.saturating_add(rows)
    }
}
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn SetDIBitsToDevice(
    hdc: usize,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    sx: i32,
    sy: i32,
    start: u32,
    lines: u32,
    bits: usize,
    info: usize,
    usage: u32,
) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let bits = door_windows_abi::widen(bits);
    let info = door_windows_abi::widen(info);
    {
        let dib = match dib_info(info, usage) {
            Ok(d) => d,
            Err(code) => {
                g::set_last_error(code);
                return 0;
            }
        };
        let block = match dib_rows(&dib, bits, start, lines) {
            Ok(b) => b,
            Err(code) => {
                g::set_last_error(code);
                return 0;
            }
        };
        let (w, h) = (w.min(i32::MAX as u32) as i32, h.min(i32::MAX as u32) as i32);
        let top = dib_top(&dib, sy, h);
        let copied = block.height as i32;
        let ok = draw(hdc, |g, d, ext| {
            g.stretch_pixels(
                d,
                (x, y, w, h),
                &block,
                (
                    Point::new(sx, top),
                    Point::new(sx.saturating_add(w), top.saturating_add(h)),
                ),
                rop::SRCCOPY,
                ext,
            )
        });
        if ok == 0 { 0 } else { copied }
    }
}
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn StretchDIBits(
    hdc: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    sx: i32,
    sy: i32,
    sw: i32,
    sh: i32,
    bits: usize,
    info: usize,
    usage: u32,
    code: u32,
) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let bits = door_windows_abi::widen(bits);
    let info = door_windows_abi::widen(info);
    {
        let dib = match dib_info(info, usage) {
            Ok(d) => d,
            Err(err) => {
                g::set_last_error(err);
                return 0;
            }
        };
        let block = match dib_rows(&dib, bits, 0, dib.height) {
            Ok(b) => b,
            Err(err) => {
                g::set_last_error(err);
                return 0;
            }
        };
        let top = dib_top(&dib, sy, sh);
        let ok = draw(hdc, |g, d, ext| {
            g.stretch_pixels(
                d,
                (x, y, w, h),
                &block,
                (
                    Point::new(sx, top),
                    Point::new(sx.saturating_add(sw), top.saturating_add(sh)),
                ),
                code,
                ext,
            )
        });
        if ok == 0 { 0 } else { sh.saturating_abs() }
    }
}
/// GetDIBits: with `bits` NULL and a zero bit count, the bitmap's format
/// into the header; otherwise rows `start..start + lines` in the header's
/// format (16, 24 or 32 bits, or 1 for a monochrome bitmap).
#[unsafe(no_mangle)]
pub extern "system" fn GetDIBits(
    hdc: usize,
    bitmap: usize,
    start: u32,
    lines: u32,
    bits: usize,
    info: usize,
    usage: u32,
) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let bitmap = door_windows_abi::widen(bitmap);
    let bits = door_windows_abi::widen(bits);
    let info = door_windows_abi::widen(info);
    {
        with(0, |s| {
            if hdc != 0 {
                dc(s, hdc)?;
            }
            let h = object(s, bitmap)?;
            let mono = s.gdi.is_mono_bitmap(h);
            let State { gdi, memory, .. } = s;
            let (width, height, px) = gdi.bitmap_pixels(h, &mut Ext(memory)).map_err(win32)?;
            let mut head = [0u8; 40];
            if !g::read(info, &mut head) || u32::from_le_bytes(head[..4].try_into().unwrap()) < 40 {
                return Err(e::INVALID_PARAMETER);
            }
            let count = u16::from_le_bytes([head[14], head[15]]);
            if bits == 0 && count == 0 {
                let bpp: u16 = if mono { 1 } else { 32 };
                head[4..8].copy_from_slice(&width.to_le_bytes());
                head[8..12].copy_from_slice(&height.to_le_bytes());
                head[12..14].copy_from_slice(&1u16.to_le_bytes());
                head[14..16].copy_from_slice(&bpp.to_le_bytes());
                head[16..20].copy_from_slice(&0u32.to_le_bytes());
                let image = (width as usize * usize::from(bpp)).div_ceil(32) * 4 * height as usize;
                head[20..24].copy_from_slice(&(image as u32).to_le_bytes());
                return if g::write(info, &head) {
                    Ok(height as i32)
                } else {
                    Err(e::INVALID_PARAMETER)
                };
            }
            if count == 1 {
                // A two-colour table: black and white.
                let table = [0u8, 0, 0, 0, 255, 255, 255, 0];
                let at = u64::from(u32::from_le_bytes(head[..4].try_into().unwrap()));
                if !g::write(info + at, &table) {
                    return Err(e::INVALID_PARAMETER);
                }
            }
            let dib = dib_info(info, usage)?;
            if !matches!(dib.bit_count, 1 | 16 | 24 | 32) || dib.width != width {
                return Err(e::INVALID_PARAMETER);
            }
            if bits == 0 {
                return Ok(height as i32);
            }
            let first = start.min(height);
            let lines = lines.min(height - first).min(dib.height);
            let stride = dib.stride();
            let mut row = alloc::vec![0u8; stride];
            let w = width as usize;
            for i in 0..lines {
                // Stored row `first + i` of a bottom-up DIB is image row
                // height - 1 - (first + i).
                let image_row = if dib.top_down {
                    first + i
                } else {
                    height - 1 - (first + i)
                } as usize;
                dib.encode_row(&px[image_row * w..][..w], &mut row);
                if !g::write(bits + u64::from(i) * stride as u64, &row) {
                    return Err(e::INVALID_PARAMETER);
                }
            }
            Ok(lines as i32)
        })
    }
}
/// SetDIBits: the rows into a bitmap that no DC has selected.
#[unsafe(no_mangle)]
pub extern "system" fn SetDIBits(
    hdc: usize,
    bitmap: usize,
    start: u32,
    lines: u32,
    bits: usize,
    info: usize,
    usage: u32,
) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let bitmap = door_windows_abi::widen(bitmap);
    let bits = door_windows_abi::widen(bits);
    let info = door_windows_abi::widen(info);
    {
        let dib = match dib_info(info, usage) {
            Ok(d) => d,
            Err(err) => {
                g::set_last_error(err);
                return 0;
            }
        };
        let block = match dib_rows(&dib, bits, start, lines) {
            Ok(b) => b,
            Err(err) => {
                g::set_last_error(err);
                return 0;
            }
        };
        with(0, |s| {
            if hdc != 0 {
                dc(s, hdc)?;
            }
            let h = object(s, bitmap)?;
            let temp = s.gdi.create_memory_dc().map_err(win32)?;
            let (w, hh) = (dib.width as i32, dib.height as i32);
            let State { gdi, memory, .. } = &mut *s;
            let result = gdi.select(temp, h).map_err(win32).and_then(|_| {
                gdi.stretch_pixels(
                    temp,
                    (0, 0, w, hh),
                    &block,
                    (Point::new(0, 0), Point::new(w, hh)),
                    rop::SRCCOPY,
                    &mut Ext(memory),
                )
                .map_err(win32)
            });
            let _ = gdi.delete_dc(temp);
            result.map(|()| block.height as i32)
        })
    }
}
/// AlphaBlend (msimg32 forwards here): `blend` is BLENDFUNCTION packed
/// little-endian (BlendOp AC_SRC_OVER, flags 0, SourceConstantAlpha,
/// AlphaFormat AC_SRC_ALPHA or 0).
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn GdiAlphaBlend(
    dst: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    src: usize,
    sx: i32,
    sy: i32,
    sw: i32,
    sh: i32,
    blend: u32,
) -> i32 {
    let dst = door_windows_abi::widen(dst);
    let src = door_windows_abi::widen(src);
    {
        let [op, flags, constant, format] = blend.to_le_bytes();
        if op != 0 || flags != 0 || format & !1 != 0 {
            g::set_last_error(e::INVALID_PARAMETER);
            return 0;
        }
        with(0, |s| {
            let d = dc(s, dst)?;
            let source = dc(s, src)?;
            let State { gdi, memory, .. } = s;
            gdi.alpha_blend(
                d,
                (x, y, w, h),
                source,
                (sx, sy, sw, sh),
                constant,
                format == 1,
                &mut Ext(memory),
            )
            .map_err(win32)?;
            Ok(1)
        })
    }
}
/// The most pixels a private icon draw or bitmap read moves.
const MAX_ICON_PIXELS: usize = 1024 * 1024;
/// user32's DrawIconEx: premultiplied `0xAARRGGBB` pixels (`width` x
/// `height`, top-down, in the caller's memory) stretched onto the logical
/// rectangle and composited; `opaque` 1 ignores their alpha.
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn DoorGdiDrawPixels(
    hdc: u64,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    pixels: u64,
    width: u32,
    height: u32,
    opaque: u32,
) -> i32 {
    let n = width as usize * height as usize;
    if n == 0 || n > MAX_ICON_PIXELS {
        g::set_last_error(e::INVALID_PARAMETER);
        return 0;
    }
    let Some(bytes) = g::read_bytes(pixels, n * 4) else {
        g::set_last_error(e::INVALID_PARAMETER);
        return 0;
    };
    let px: Vec<u32> = bytes
        .chunks_exact(4)
        .map(|p| u32::from_le_bytes([p[0], p[1], p[2], p[3]]))
        .collect();
    draw(hdc, |g, d, ext| {
        g.draw_pixels(d, (x, y, w, h), px, (width, height), opaque == 0, ext)
    })
}
/// user32's CreateIconIndirect and GetIconInfo: a bitmap's pixels
/// (`0xAARRGGBB`; a monochrome bitmap's are black or white) into `out`
/// (capacity `pixels` u32s), its size into `size` (two u32s). 1 = colour,
/// 2 = monochrome, 0 = failure.
#[unsafe(no_mangle)]
pub extern "system" fn DoorGdiBitmapPixels(bitmap: u64, out: u64, pixels: u32, size: u64) -> i32 {
    with(0, |s| {
        let h = object(s, bitmap)?;
        let mono = s.gdi.is_mono_bitmap(h);
        let State { gdi, memory, .. } = s;
        let (w, hh, px) = gdi.bitmap_argb(h, &mut Ext(memory)).map_err(win32)?;
        let mut dims = [0u8; 8];
        dims[..4].copy_from_slice(&w.to_le_bytes());
        dims[4..].copy_from_slice(&hh.to_le_bytes());
        if !g::write(size, &dims) {
            return Err(e::INVALID_PARAMETER);
        }
        if out != 0 {
            if px.len() > pixels as usize || px.len() > MAX_ICON_PIXELS {
                return Err(e::INSUFFICIENT_BUFFER);
            }
            let bytes: Vec<u8> = px.iter().flat_map(|p| p.to_le_bytes()).collect();
            if !g::write(out, &bytes) {
                return Err(e::INVALID_PARAMETER);
            }
        }
        Ok(if mono { 2 } else { 1 })
    })
}
/// An entry of the application asset pack (icons, sounds): its address in
/// the read-only view, its length into `length`; 0 without it.
#[unsafe(no_mangle)]
pub extern "system" fn DoorGdiAsset(key: u64, key_len: u32, length: u64) -> u64 {
    let Some(name) = g::read_bytes(key, key_len.min(256) as usize) else {
        return 0;
    };
    let Some(view) = asset_pack() else {
        return 0;
    };
    let Some(pack) = door_initfs::app_assets::Pack::parse(view) else {
        return 0;
    };
    let Some(data) = core::str::from_utf8(&name).ok().and_then(|n| pack.get(n)) else {
        return 0;
    };
    if !g::write(length, &(data.len() as u64).to_le_bytes()) {
        return 0;
    }
    data.as_ptr() as u64
}
