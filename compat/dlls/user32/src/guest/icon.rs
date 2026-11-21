//! Icons and cursors made from images: LoadIcon/LoadImage (resources,
//! `.ico`/`.cur`/`.bmp` files, the system icons), CreateIconFromResource(Ex),
//! LookupIconIdFromDirectory(Ex), CreateIconIndirect/CreateIcon,
//! GetIconInfo, CopyIcon/CopyImage, DestroyIcon/DestroyCursor,
//! DrawIcon(Ex), PrivateExtractIcons and the window icons of
//! WM_SETICON/WM_GETICON. Pixels are kept premultiplied
//! (`door_windows_gui::icon`) and drawn by gdi32 with per-pixel alpha.
//!
//! The icon table has its own lock, taken after USER when both are held and
//! never across a call into application code.
use alloc::vec::Vec;
use door_windows_gui::handles::{Table, kind};
use door_windows_gui::icon::{Image, System};
use door_windows_gui::win32_error as e;
use door_windows_gui_guest as g;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorCreateFileW(
        path: u64,
        access: u32,
        share: u32,
        security: u64,
        disposition: u32,
        flags: u32,
        template: u64,
    ) -> u64;
    fn DoorReadFile(handle: u64, buffer: u64, bytes: u32, read: u64, overlapped: u64) -> i32;
    fn DoorGetFileSizeEx(handle: u64, size: u64) -> i32;
    fn DoorCloseFile(handle: u64) -> i32;
}
#[link(name = "gdi32", kind = "dylib")]
unsafe extern "system" {
    fn DoorGdiDrawPixels(
        hdc: u64,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        pixels: u64,
        width: u32,
        height: u32,
        opaque: u32,
    ) -> i32;
    fn DoorGdiBitmapPixels(bitmap: u64, out: u64, pixels: u32, size: u64) -> i32;
    fn DoorGdiAsset(key: u64, key_len: u32, length: u64) -> u64;
    fn CreateDIBSection(
        hdc: usize,
        info: usize,
        usage: u32,
        bits: usize,
        section: usize,
        offset: u32,
    ) -> usize;
    fn CreateBitmap(width: i32, height: i32, planes: u32, bits_pixel: u32, bits: usize) -> usize;
    fn SetDIBits(
        hdc: usize,
        bitmap: usize,
        start: u32,
        lines: u32,
        bits: usize,
        info: usize,
        usage: u32,
    ) -> i32;
    fn DeleteObject(object: usize) -> i32;
}

const IMAGE_BITMAP: u32 = 0;
const IMAGE_ICON: u32 = 1;
const IMAGE_CURSOR: u32 = 2;
const LR_LOADFROMFILE: u32 = 0x10;
const LR_DEFAULTSIZE: u32 = 0x40;
const LR_SHARED: u32 = 0x8000;
const DI_MASK: u32 = 1;
const DI_IMAGE: u32 = 2;
const DI_DEFAULTSIZE: u32 = 8;
const RT_CURSOR: u64 = 1;
const RT_BITMAP: u64 = 2;
const RT_ICON: u64 = 3;
const RT_GROUP_CURSOR: u64 = 12;
const RT_GROUP_ICON: u64 = 14;
/// SM_CXICON / SM_CXSMICON / SM_CXCURSOR.
const ICON_SIZE: u32 = 32;
const SMALL_ICON: u32 = 16;
/// Files larger than this are not read for icons.
const MAX_FILE: u64 = 64 * 1024 * 1024;

pub(crate) struct Icon {
    pub image: Image,
    pub cursor: bool,
    pub hotspot: (u32, u32),
    /// LoadIcon / LR_SHARED: kept for the process, never destroyed.
    pub shared: bool,
}
/// A shared icon's identity: module, resource (id, or a hash of its name),
/// size.
type SharedKey = (u64, u64, u32, u32);
struct Icons {
    table: Table<Icon>,
    shared: Vec<(SharedKey, u64)>,
    /// WM_SETICON: (window, big, small).
    windows: Vec<(u64, u64, u64)>,
}
static ICONS: g::Lock<Option<Icons>> = g::Lock::new(None);

fn with<R>(f: impl FnOnce(&mut Icons) -> R) -> R {
    let mut guard = ICONS.lock();
    let icons = guard.get_or_insert_with(|| Icons {
        table: Table::new(kind::ICON, 8192),
        shared: Vec::new(),
        windows: Vec::new(),
    });
    f(icons)
}
fn fail<T: Default>(code: u32) -> T {
    g::set_last_error(code);
    T::default()
}
fn insert(icon: Icon) -> u64 {
    match with(|i| i.table.insert(icon)) {
        Ok(h) => h.value(),
        Err(_) => fail(e::NOT_ENOUGH_MEMORY),
    }
}
/// The pixels of an icon handle, cloned out of the lock.
pub(crate) fn image_of(handle: u64) -> Option<(Image, bool, (u32, u32))> {
    with(|i| {
        let h = i.table.resolve(handle)?;
        i.table
            .get(h)
            .map(|icon| (icon.image.clone(), icon.cursor, icon.hotspot))
    })
}

// ---- building icons -------------------------------------------------------------

/// The size a caller asks for: 0 means the image's own size, or the
/// system size with LR_DEFAULTSIZE.
fn wanted(cx: i32, cy: i32, flags: u32, own: (u32, u32)) -> (u32, u32) {
    // SM_CXICON and SM_CXCURSOR are both 32.
    let default = ICON_SIZE;
    let pick = |v: i32, own: u32| {
        if v > 0 {
            (v as u32).min(door_windows_gui::icon::MAX_EDGE)
        } else if flags & LR_DEFAULTSIZE != 0 {
            default
        } else {
            own
        }
    };
    (pick(cx, own.0), pick(cy, own.1))
}
/// One icon frame's bytes (a DIB or PNG; a cursor frame starts with its
/// hot spot) to an icon at the requested size.
fn from_frame(bits: &[u8], cursor: bool, cx: i32, cy: i32, flags: u32) -> Option<Icon> {
    let (hotspot, frame) = if cursor {
        let x = u32::from(u16::from_le_bytes([*bits.first()?, *bits.get(1)?]));
        let y = u32::from(u16::from_le_bytes([*bits.get(2)?, *bits.get(3)?]));
        ((x, y), bits.get(4..)?)
    } else {
        ((0, 0), bits)
    };
    let decoded = door_pe_icon::decode(frame)?;
    let image = Image::from_bgra(decoded.width, decoded.height, &decoded.bgra)?;
    let (w, h) = wanted(cx, cy, flags, (image.width, image.height));
    let scaled = image.resample(w, h)?;
    let hotspot = (
        hotspot.0 * w / image.width.max(1),
        hotspot.1 * h / image.height.max(1),
    );
    Some(Icon {
        image: scaled,
        cursor,
        hotspot,
        shared: false,
    })
}
/// The entries of an RT_GROUP_ICON or RT_GROUP_CURSOR directory.
fn group_entries(dir: &[u8], icon: bool) -> Option<Vec<door_pe_icon::Entry>> {
    if icon {
        return door_pe_icon::directory(dir);
    }
    // CURSORDIR entries: wWidth, wHeight (both masks), planes, bits,
    // bytes, id.
    if dir.len() < 6 || u16::from_le_bytes([dir[2], dir[3]]) != 2 {
        return None;
    }
    let count = usize::from(u16::from_le_bytes([dir[4], dir[5]]));
    if count > 256 || dir.len() < 6 + count * 14 {
        return None;
    }
    Some(
        dir[6..6 + count * 14]
            .chunks_exact(14)
            .map(|e| door_pe_icon::Entry {
                width: u32::from(u16::from_le_bytes([e[0], e[1]])),
                height: u32::from(u16::from_le_bytes([e[2], e[3]])) / 2,
                bits: u32::from(u16::from_le_bytes([e[6], e[7]])),
                id: u32::from(u16::from_le_bytes([e[12], e[13]])),
            })
            .collect(),
    )
}
fn choose(
    entries: &[door_pe_icon::Entry],
    cx: i32,
    cy: i32,
    flags: u32,
) -> Option<door_pe_icon::Entry> {
    let (w, h) = if cx <= 0 && cy <= 0 && flags & LR_DEFAULTSIZE == 0 {
        // No size: the first (Windows' "actual size") entry's.
        let first = entries.first()?;
        (first.width, first.height)
    } else {
        wanted(cx, cy, flags | LR_DEFAULTSIZE, (ICON_SIZE, ICON_SIZE))
    };
    door_pe_icon::choose_size(entries, w, h)
}
/// A resource name argument as a pointer ntdll can read: an ordinal as
/// is, an ANSI name converted.
struct Name {
    value: u64,
    _wide: Option<Vec<u16>>,
}
fn name_arg(name: u64, ansi: bool) -> Option<Name> {
    if name >> 16 == 0 || !ansi {
        return Some(Name {
            value: name,
            _wide: None,
        });
    }
    let bytes = read_ansi(name)?;
    let mut wide: Vec<u16> = bytes.iter().map(|&b| u16::from(b)).collect();
    wide.push(0);
    Some(Name {
        value: wide.as_ptr() as u64,
        _wide: Some(wide),
    })
}
fn read_ansi(pointer: u64) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut at = pointer;
    loop {
        let mut b = [0u8; 1];
        if !g::read(at, &mut b) || out.len() > 4096 {
            return None;
        }
        if b[0] == 0 {
            return Some(out);
        }
        out.push(b[0]);
        at += 1;
    }
}
/// A shared-cache key for a resource name.
fn name_key(name: u64) -> u64 {
    if name >> 16 == 0 {
        return name;
    }
    // A string name: FNV-1a over its units, high bit set.
    let units = g::read_wide(name, 256).unwrap_or_default();
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for u in units {
        hash = (hash ^ u64::from(u)).wrapping_mul(0x100_0000_01b3);
    }
    hash | 1 << 63
}
/// An icon or cursor group resource of `module` at the requested size.
fn from_resource(
    module: u64,
    name: u64,
    cursor: bool,
    cx: i32,
    cy: i32,
    flags: u32,
) -> Option<Icon> {
    let (group, single) = if cursor {
        (RT_GROUP_CURSOR, RT_CURSOR)
    } else {
        (RT_GROUP_ICON, RT_ICON)
    };
    let dir = super::menu::resource_bytes(module, group, name)?;
    let entry = choose(&group_entries(&dir, !cursor)?, cx, cy, flags)?;
    let bits = super::menu::resource_bytes(module, single, u64::from(entry.id))?;
    from_frame(&bits, cursor, cx, cy, flags)
}
/// The whole file at the (wide) `path`.
fn read_file(path: u64) -> Option<Vec<u8>> {
    // SAFETY: ntdll captures the path and validates the handle.
    let file = unsafe { DoorCreateFileW(path, 0x8000_0000, 1, 0, 3, 0, 0) };
    if file == 0 || file == u64::MAX {
        return None;
    }
    let mut size = [0u8; 8];
    let ok = unsafe { DoorGetFileSizeEx(file, size.as_mut_ptr() as u64) } != 0;
    let size = u64::from_le_bytes(size);
    let data = (ok && size <= MAX_FILE).then(|| {
        let mut data = Vec::new();
        data.try_reserve_exact(size as usize).ok()?;
        data.resize(size as usize, 0);
        let mut done = 0usize;
        while done < data.len() {
            let n = (data.len() - done).min(1 << 20);
            let mut got = [0u8; 4];
            let read = unsafe {
                DoorReadFile(
                    file,
                    data[done..].as_mut_ptr() as u64,
                    n as u32,
                    got.as_mut_ptr() as u64,
                    0,
                )
            };
            let got = u32::from_le_bytes(got) as usize;
            if read == 0 || got == 0 {
                return None;
            }
            done += got;
        }
        Some(data)
    });
    unsafe { DoorCloseFile(file) };
    data.flatten()
}
/// An `.ico` or `.cur` file's frame for the requested size.
fn from_icon_file(bytes: &[u8], want_cursor: bool, cx: i32, cy: i32, flags: u32) -> Option<Icon> {
    let (cursor, frames) = door_pe_icon::icon_file(bytes)?;
    let entries: Vec<_> = frames.iter().map(|f| f.entry).collect();
    let entry = choose(&entries, cx, cy, flags)?;
    let frame = frames.iter().find(|f| f.entry.id == entry.id)?;
    let data = bytes.get(frame.offset as usize..(frame.offset + frame.size) as usize)?;
    let mut icon = if cursor {
        let mut with_hotspot = Vec::with_capacity(data.len() + 4);
        with_hotspot.extend_from_slice(&frame.hotspot.0.to_le_bytes());
        with_hotspot.extend_from_slice(&frame.hotspot.1.to_le_bytes());
        with_hotspot.extend_from_slice(data);
        from_frame(&with_hotspot, true, cx, cy, flags)?
    } else {
        from_frame(data, false, cx, cy, flags)?
    };
    icon.cursor = cursor || want_cursor;
    Some(icon)
}
/// A Doors system icon (IDI_* / OIC_*) at `size`: the Doors logo from the
/// asset pack for IDI_WINLOGO when it is there.
fn system_icon(which: System, size: u32) -> Option<Icon> {
    let image = match which {
        System::WinLogo => asset_image("branding/door", size)
            .or_else(|| door_windows_gui::icon::system(which, size))?,
        _ => door_windows_gui::icon::system(which, size)?,
    };
    Some(Icon {
        image,
        cursor: false,
        hotspot: (0, 0),
        shared: true,
    })
}
/// Sizes the asset pack stores icons at.
const ASSET_SIZES: [u32; 16] = [
    16, 20, 24, 28, 32, 40, 48, 56, 60, 64, 72, 80, 96, 120, 160, 240,
];
/// The asset `<stem>.<n>.qoi` or `.bgra` closest to (not below) `size`,
/// resampled to it.
pub(crate) fn asset_image(stem: &str, size: u32) -> Option<Image> {
    let mut sizes: Vec<u32> = ASSET_SIZES.to_vec();
    // Prefer the smallest stored size at least as large, then larger
    // ones, then smaller ones (largest first).
    sizes.sort_by_key(|&s| if s >= size { (0, s) } else { (1, u32::MAX - s) });
    for s in sizes {
        for ext in ["qoi", "bgra"] {
            let key = alloc::format!("{stem}.{s}.{ext}");
            let mut length = 0u64;
            // SAFETY: gdi32 reads the key and writes the length.
            let address = unsafe {
                DoorGdiAsset(
                    key.as_ptr() as u64,
                    key.len() as u32,
                    &mut length as *mut u64 as u64,
                )
            };
            if address == 0 || length == 0 || length > 4 << 20 {
                continue;
            }
            // SAFETY: gdi32 maps the pack read-only for the process's life.
            let data =
                unsafe { core::slice::from_raw_parts(address as *const u8, length as usize) };
            let image = if ext == "qoi" {
                door_pe_icon::decode_qoi(data)
                    .and_then(|i| Image::from_bgra(i.width, i.height, &i.bgra))
            } else {
                Image::from_bgra(s, s, data)
            };
            if let Some(image) = image {
                return image.resample(size, size);
            }
        }
    }
    None
}

// ---- exports ----------------------------------------------------------------

/// CreateIconFromResourceEx: an RT_ICON or RT_CURSOR resource's bytes.
#[unsafe(no_mangle)]
pub extern "system" fn CreateIconFromResourceEx(
    bits: usize,
    size: u32,
    icon: i32,
    _version: u32,
    cx: i32,
    cy: i32,
    flags: u32,
) -> usize {
    let bits = door_windows_abi::widen(bits);
    door_windows_abi::narrow((move || -> u64 {
        if bits == 0 || size == 0 || u64::from(size) > door_pe_icon::MAX_FRAME {
            return fail(e::INVALID_PARAMETER);
        }
        let Some(data) = g::read_bytes(bits, size as usize) else {
            return fail(e::INVALID_PARAMETER);
        };
        match from_frame(&data, icon == 0, cx, cy, flags) {
            Some(i) => insert(i),
            None => fail(e::INVALID_DATA),
        }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateIconFromResource(
    bits: usize,
    size: u32,
    icon: i32,
    version: u32,
) -> usize {
    let bits = door_windows_abi::widen(bits);
    door_windows_abi::narrow((move || -> u64 {
        door_windows_abi::widen(CreateIconFromResourceEx(
            door_windows_abi::narrow(bits),
            size,
            icon,
            version,
            0,
            0,
            LR_DEFAULTSIZE,
        ))
    })())
}
/// LookupIconIdFromDirectoryEx: the RT_ICON / RT_CURSOR id that best fits.
#[unsafe(no_mangle)]
pub extern "system" fn LookupIconIdFromDirectoryEx(
    dir: usize,
    icon: i32,
    cx: i32,
    cy: i32,
    flags: u32,
) -> i32 {
    let dir = door_windows_abi::widen(dir);
    {
        let Some(head) = g::read_bytes(dir, 6) else {
            return fail(e::INVALID_PARAMETER);
        };
        let count = usize::from(u16::from_le_bytes([head[4], head[5]])).min(256);
        let Some(all) = g::read_bytes(dir, 6 + count * 14) else {
            return fail(e::INVALID_PARAMETER);
        };
        let flags = if cx == 0 && cy == 0 {
            flags | LR_DEFAULTSIZE
        } else {
            flags
        };
        group_entries(&all, icon != 0)
            .and_then(|entries| choose(&entries, cx, cy, flags))
            .map_or_else(|| fail(e::INVALID_DATA), |e| e.id as i32)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn LookupIconIdFromDirectory(dir: usize, icon: i32) -> i32 {
    let dir = door_windows_abi::widen(dir);
    LookupIconIdFromDirectoryEx(door_windows_abi::narrow(dir), icon, 0, 0, LR_DEFAULTSIZE)
}
fn load_icon(instance: u64, name: u64, ansi: bool) -> u64 {
    if instance == 0 {
        return match System::from_id(name) {
            Some(which) => shared(0, name, ICON_SIZE, ICON_SIZE, || {
                system_icon(which, ICON_SIZE)
            }),
            None => fail(e::RESOURCE_NAME_NOT_FOUND),
        };
    }
    let Some(arg) = name_arg(name, ansi) else {
        return fail(e::INVALID_PARAMETER);
    };
    let key = name_key(arg.value);
    shared(instance, key, ICON_SIZE, ICON_SIZE, || {
        from_resource(
            instance,
            arg.value,
            false,
            ICON_SIZE as i32,
            ICON_SIZE as i32,
            0,
        )
    })
}
/// The process-wide shared copy of an icon, made once.
fn shared(module: u64, name: u64, w: u32, h: u32, make: impl FnOnce() -> Option<Icon>) -> u64 {
    let key = (module, name, w, h);
    if let Some(h) = with(|i| i.shared.iter().find(|(k, _)| *k == key).map(|(_, h)| *h)) {
        return h;
    }
    let Some(mut icon) = make() else {
        return fail(e::RESOURCE_NAME_NOT_FOUND);
    };
    icon.shared = true;
    let handle = insert(icon);
    if handle != 0 {
        with(|i| i.shared.push((key, handle)));
    }
    handle
}
#[unsafe(no_mangle)]
pub extern "system" fn LoadIconW(instance: usize, name: usize) -> usize {
    let instance = door_windows_abi::widen(instance);
    let name = door_windows_abi::widen(name);
    door_windows_abi::narrow((move || -> u64 { load_icon(instance, name, false) })())
}
#[unsafe(no_mangle)]
pub extern "system" fn LoadIconA(instance: usize, name: usize) -> usize {
    let instance = door_windows_abi::widen(instance);
    let name = door_windows_abi::widen(name);
    door_windows_abi::narrow((move || -> u64 { load_icon(instance, name, true) })())
}
/// An application cursor resource (system cursors are LoadCursorW's).
pub(crate) fn load_cursor_resource(instance: u64, name: u64, ansi: bool) -> u64 {
    let Some(arg) = name_arg(name, ansi) else {
        return fail(e::INVALID_PARAMETER);
    };
    let key = name_key(arg.value);
    shared(instance, key, ICON_SIZE, ICON_SIZE, || {
        from_resource(instance, arg.value, true, 0, 0, LR_DEFAULTSIZE)
    })
}
fn load_image(
    instance: u64,
    name: u64,
    kind: u32,
    cx: i32,
    cy: i32,
    flags: u32,
    ansi: bool,
) -> u64 {
    if flags & LR_LOADFROMFILE != 0 {
        let path = if ansi {
            let Some(bytes) = read_ansi(name) else {
                return fail(e::INVALID_PARAMETER);
            };
            let mut wide: Vec<u16> = bytes.iter().map(|&b| u16::from(b)).collect();
            wide.push(0);
            read_file(wide.as_ptr() as u64)
        } else {
            read_file(name)
        };
        let Some(bytes) = path else {
            return fail(e::FILE_NOT_FOUND);
        };
        return match kind {
            IMAGE_BITMAP => bitmap_file(&bytes),
            IMAGE_ICON | IMAGE_CURSOR => {
                match from_icon_file(&bytes, kind == IMAGE_CURSOR, cx, cy, flags) {
                    Some(icon) => insert(icon),
                    None => fail(e::INVALID_DATA),
                }
            }
            _ => fail(e::INVALID_PARAMETER),
        };
    }
    match kind {
        IMAGE_BITMAP => {
            let Some(arg) = name_arg(name, ansi) else {
                return fail(e::INVALID_PARAMETER);
            };
            match super::menu::resource_bytes(instance, RT_BITMAP, arg.value) {
                Some(dib) => bitmap_from_dib(&dib, 0),
                None => fail(e::RESOURCE_NAME_NOT_FOUND),
            }
        }
        IMAGE_ICON | IMAGE_CURSOR => {
            let cursor = kind == IMAGE_CURSOR;
            if instance == 0 {
                if cursor {
                    return door_windows_abi::widen(super::misc::LoadCursorW(
                        0,
                        door_windows_abi::narrow(name),
                    ));
                }
                let Some(which) = System::from_id(name) else {
                    return fail(e::RESOURCE_NAME_NOT_FOUND);
                };
                let (w, h) = wanted(cx, cy, flags | LR_DEFAULTSIZE, (ICON_SIZE, ICON_SIZE));
                return shared(0, name, w, h, || {
                    system_icon(which, w).and_then(|mut i| {
                        i.image = i.image.resample(w, h)?;
                        Some(i)
                    })
                });
            }
            let Some(arg) = name_arg(name, ansi) else {
                return fail(e::INVALID_PARAMETER);
            };
            if flags & LR_SHARED != 0 {
                let (w, h) = wanted(cx, cy, flags, (0, 0));
                let key = name_key(arg.value);
                return shared(instance, key, w, h, || {
                    from_resource(instance, arg.value, cursor, cx, cy, flags)
                });
            }
            match from_resource(instance, arg.value, cursor, cx, cy, flags) {
                Some(icon) => insert(icon),
                None => fail(e::RESOURCE_NAME_NOT_FOUND),
            }
        }
        _ => fail(e::INVALID_PARAMETER),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn LoadImageW(
    instance: usize,
    name: usize,
    kind: u32,
    cx: i32,
    cy: i32,
    flags: u32,
) -> usize {
    let instance = door_windows_abi::widen(instance);
    let name = door_windows_abi::widen(name);
    door_windows_abi::narrow((move || -> u64 {
        load_image(instance, name, kind, cx, cy, flags, false)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn LoadImageA(
    instance: usize,
    name: usize,
    kind: u32,
    cx: i32,
    cy: i32,
    flags: u32,
) -> usize {
    let instance = door_windows_abi::widen(instance);
    let name = door_windows_abi::widen(name);
    door_windows_abi::narrow((move || -> u64 {
        load_image(instance, name, kind, cx, cy, flags, true)
    })())
}
/// A `.bmp` file: BITMAPFILEHEADER (14 bytes) then the DIB.
fn bitmap_file(bytes: &[u8]) -> u64 {
    if bytes.len() < 14 + 40 || &bytes[..2] != b"BM" {
        return fail(e::INVALID_DATA);
    }
    let offset = u32::from_le_bytes(bytes[10..14].try_into().unwrap()) as usize;
    if offset < 14 + 40 || offset > bytes.len() {
        return fail(e::INVALID_DATA);
    }
    bitmap_from_dib(&bytes[14..], offset - 14)
}
/// A packed DIB (header, colour table, then bits at `bits_at` or right
/// after the table when 0) to a 32-bit DIB section.
fn bitmap_from_dib(dib: &[u8], bits_at: usize) -> u64 {
    let Ok(info) = door_windows_gui::dib::DibInfo::needed(dib)
        .and_then(|n| dib.get(..n).ok_or(door_windows_gui::Error::Malformed))
        .and_then(|h| door_windows_gui::dib::DibInfo::parse(h, 0))
    else {
        return fail(e::INVALID_DATA);
    };
    let at = if bits_at == 0 {
        info.header_bytes
    } else {
        bits_at
    };
    let Some(bits) = dib.get(at..at + info.image_bytes()) else {
        return fail(e::INVALID_DATA);
    };
    let header = door_windows_gui::dib::header32(info.width, info.height, false);
    let mut address = 0u64;
    // SAFETY: gdi32 validates every argument and writes the bits pointer.
    let bitmap = unsafe {
        CreateDIBSection(
            0,
            door_windows_abi::narrow(header.as_ptr() as u64),
            0,
            door_windows_abi::narrow(&mut address as *mut u64 as u64),
            0,
            0,
        )
    };
    if bitmap == 0 {
        return 0;
    }
    let lines = unsafe {
        SetDIBits(
            0,
            bitmap,
            0,
            info.height,
            door_windows_abi::narrow(bits.as_ptr() as u64),
            door_windows_abi::narrow(dib.as_ptr() as u64),
            0,
        )
    };
    if lines == 0 {
        unsafe { DeleteObject(bitmap) };
        return 0;
    }
    door_windows_abi::widen(bitmap)
}
/// A bitmap's `0xAARRGGBB` pixels through gdi32 (`mono` for a 1-bpp one).
fn bitmap_pixels(bitmap: u64) -> Option<(u32, u32, Vec<u32>, bool)> {
    let mut size = [0u32; 2];
    // SAFETY: gdi32 validates the handle and the output pointers.
    let kind = unsafe { DoorGdiBitmapPixels(bitmap, 0, 0, size.as_mut_ptr() as u64) };
    if kind == 0 {
        return None;
    }
    let n = size[0] as usize * size[1] as usize;
    if n == 0 || n > 1 << 20 {
        return None;
    }
    let mut px = alloc::vec![0u32; n];
    let again = unsafe {
        DoorGdiBitmapPixels(
            bitmap,
            px.as_mut_ptr() as u64,
            n as u32,
            size.as_mut_ptr() as u64,
        )
    };
    (again != 0).then_some((size[0], size[1], px, kind == 2))
}
/// CreateIconIndirect: ICONINFO {fIcon, xHotspot, yHotspot, hbmMask,
/// hbmColor} (32 bytes).
#[unsafe(no_mangle)]
pub extern "system" fn CreateIconIndirect(info: usize) -> usize {
    let info = door_windows_abi::widen(info);
    door_windows_abi::narrow((move || -> u64 {
        let mut b = [0u8; super::structs::ICONINFO_BYTES];
        if !g::read(info, &mut b) {
            return fail(e::INVALID_PARAMETER);
        }
        let field = |index| super::structs::get(&b, &super::structs::ICONINFO, index);
        let icon = field(0) as u32 != 0;
        let hotspot = (field(1) as u32, field(2) as u32);
        let Some(mask) = bitmap_pixels(field(3)) else {
            return fail(e::INVALID_PARAMETER);
        };
        let color = match field(4) {
            0 => None,
            h => match bitmap_pixels(h) {
                Some(c) => Some(c),
                None => return fail(e::INVALID_PARAMETER),
            },
        };
        let image = door_windows_gui::icon::from_bitmaps(
            color.as_ref().map(|(w, h, px, _)| (*w, *h, px.as_slice())),
            (mask.0, mask.1, &mask.2),
        );
        match image {
            Some(image) => insert(Icon {
                image,
                cursor: !icon,
                hotspot: if icon { image_center(&mask) } else { hotspot },
                shared: false,
            }),
            None => fail(e::INVALID_PARAMETER),
        }
    })())
}
/// An icon's hot spot is its centre.
fn image_center(mask: &(u32, u32, Vec<u32>, bool)) -> (u32, u32) {
    let h = if mask.3 && mask.1.is_multiple_of(2) {
        mask.1 / 2
    } else {
        mask.1
    };
    (mask.0 / 2, h / 2)
}
/// CreateIcon: AND and XOR bits in the caller's memory (1-bpp masks; the
/// XOR image 1, 24 or 32 bits per pixel).
#[unsafe(no_mangle)]
pub extern "system" fn CreateIcon(
    _instance: usize,
    width: i32,
    height: i32,
    planes: u8,
    bits_pixel: u8,
    and_bits: usize,
    xor_bits: usize,
) -> usize {
    let and_bits = door_windows_abi::widen(and_bits);
    let xor_bits = door_windows_abi::widen(xor_bits);
    door_windows_abi::narrow((move || -> u64 {
        let depth = u32::from(planes) * u32::from(bits_pixel);
        if width <= 0 || height <= 0 || width > 256 || height > 256 || !matches!(depth, 1 | 24 | 32)
        {
            return fail(e::INVALID_PARAMETER);
        }
        let (w, h) = (width as usize, height as usize);
        // Bitmap rows are WORD aligned (CreateBitmap's layout).
        let mask_stride = w.div_ceil(16) * 2;
        let xor_stride = (w * depth as usize).div_ceil(16) * 2;
        let (Some(and), Some(xor)) = (
            g::read_bytes(and_bits, mask_stride * h),
            g::read_bytes(xor_bits, xor_stride * h),
        ) else {
            return fail(e::INVALID_PARAMETER);
        };
        let bit = |row: &[u8], x: usize| row[x / 8] & (0x80 >> (x % 8)) != 0;
        let white = 0x00ff_ffffu32;
        let mut mask = Vec::with_capacity(w * h);
        let mut color = Vec::with_capacity(w * h);
        for y in 0..h {
            let arow = &and[y * mask_stride..][..mask_stride];
            let xrow = &xor[y * xor_stride..][..xor_stride];
            for x in 0..w {
                mask.push(if bit(arow, x) { white } else { 0 });
                color.push(match depth {
                    1 => {
                        if bit(xrow, x) {
                            white
                        } else {
                            0
                        }
                    }
                    24 => u32::from_le_bytes([xrow[3 * x], xrow[3 * x + 1], xrow[3 * x + 2], 0]),
                    _ => u32::from_le_bytes(xrow[4 * x..4 * x + 4].try_into().unwrap()),
                });
            }
        }
        match door_windows_gui::icon::from_bitmaps(
            Some((w as u32, h as u32, &color)),
            (w as u32, h as u32, &mask),
        ) {
            Some(image) => insert(Icon {
                image,
                cursor: false,
                hotspot: (w as u32 / 2, h as u32 / 2),
                shared: false,
            }),
            None => fail(e::INVALID_PARAMETER),
        }
    })())
}
/// GetIconInfo: new colour (32-bit DIB section, straight alpha) and mask
/// bitmaps the caller deletes.
#[unsafe(no_mangle)]
pub extern "system" fn GetIconInfo(icon: usize, info: usize) -> i32 {
    let icon = door_windows_abi::widen(icon);
    let info = door_windows_abi::widen(info);
    {
        let Some((image, cursor, hotspot)) = image_of(icon) else {
            return fail(e::INVALID_CURSOR_HANDLE);
        };
        let header = door_windows_gui::dib::header32(image.width, image.height, true);
        let mut address = 0u64;
        // SAFETY: gdi32 validates every argument and writes the bits pointer.
        let color = unsafe {
            CreateDIBSection(
                0,
                door_windows_abi::narrow(header.as_ptr() as u64),
                0,
                door_windows_abi::narrow(&mut address as *mut u64 as u64),
                0,
                0,
            )
        };
        if color == 0 {
            return 0;
        }
        let straight: Vec<u8> = image
            .straight()
            .iter()
            .flat_map(|p| p.to_le_bytes())
            .collect();
        if !g::write(address, &straight) {
            unsafe { DeleteObject(color) };
            return fail(e::INVALID_PARAMETER);
        }
        let and = image.and_mask();
        let mask = unsafe {
            CreateBitmap(
                image.width as i32,
                image.height as i32,
                1,
                1,
                door_windows_abi::narrow(and.as_ptr() as u64),
            )
        };
        if mask == 0 {
            unsafe { DeleteObject(color) };
            return 0;
        }
        let mut b = [0u8; super::structs::ICONINFO_BYTES];
        let spot = if cursor {
            hotspot
        } else {
            (image.width / 2, image.height / 2)
        };
        for (index, value) in [
            u64::from(!cursor),
            u64::from(spot.0),
            u64::from(spot.1),
            door_windows_abi::widen(mask),
            door_windows_abi::widen(color),
        ]
        .into_iter()
        .enumerate()
        {
            super::structs::set(&mut b, &super::structs::ICONINFO, index, value);
        }
        if !g::write(info, &b) {
            unsafe {
                DeleteObject(mask);
                DeleteObject(color);
            }
            return fail(e::INVALID_PARAMETER);
        }
        1
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CopyIcon(icon: usize) -> usize {
    let icon = door_windows_abi::widen(icon);
    door_windows_abi::narrow((move || -> u64 {
        match image_of(icon) {
            Some((image, cursor, hotspot)) => insert(Icon {
                image,
                cursor,
                hotspot,
                shared: false,
            }),
            None => fail(e::INVALID_CURSOR_HANDLE),
        }
    })())
}
/// CopyImage for icons and cursors (resized when asked); bitmaps are not
/// copied here.
#[unsafe(no_mangle)]
pub extern "system" fn CopyImage(image: usize, kind: u32, cx: i32, cy: i32, flags: u32) -> usize {
    let image = door_windows_abi::widen(image);
    door_windows_abi::narrow((move || -> u64 {
        if !matches!(kind, IMAGE_ICON | IMAGE_CURSOR) {
            return fail(e::NOT_SUPPORTED);
        }
        let Some((pixels, cursor, hotspot)) = image_of(image) else {
            return fail(e::INVALID_CURSOR_HANDLE);
        };
        let (w, h) = wanted(cx, cy, flags, (pixels.width, pixels.height));
        match pixels.resample(w, h) {
            Some(resized) => insert(Icon {
                image: resized,
                cursor,
                hotspot,
                shared: false,
            }),
            None => fail(e::INVALID_PARAMETER),
        }
    })())
}
fn destroy(handle: u64) -> i32 {
    // System cursors are shared: destroying one succeeds and keeps it.
    if handle >> 24 == u64::from(kind::CURSOR) {
        return 1;
    }
    let removed = with(|i| {
        let h = i.table.resolve(handle)?;
        match i.table.get(h) {
            Some(icon) if icon.shared => Some(true),
            Some(_) => i.table.remove(h).map(|_| true),
            None => None,
        }
    });
    match removed {
        Some(_) => 1,
        None => fail(e::INVALID_CURSOR_HANDLE),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DestroyIcon(icon: usize) -> i32 {
    let icon = door_windows_abi::widen(icon);
    destroy(icon)
}
#[unsafe(no_mangle)]
pub extern "system" fn DestroyCursor(cursor: usize) -> i32 {
    let cursor = door_windows_abi::widen(cursor);
    destroy(cursor)
}
/// DrawIconEx: the icon at (x, y), `cx` x `cy` (0: its own size, or the
/// system size with DI_DEFAULTSIZE), over `brush` when one is given.
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn DrawIconEx(
    hdc: usize,
    x: i32,
    y: i32,
    icon: usize,
    cx: i32,
    cy: i32,
    _step: u32,
    brush: usize,
    flags: u32,
) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let icon = door_windows_abi::widen(icon);
    let brush = door_windows_abi::widen(brush);
    {
        let Some((image, _, _)) = image_of(icon) else {
            return fail(e::INVALID_CURSOR_HANDLE);
        };
        let size = |v: i32, own: u32| {
            if v > 0 {
                v
            } else if flags & DI_DEFAULTSIZE != 0 {
                ICON_SIZE as i32
            } else {
                own as i32
            }
        };
        let (w, h) = (size(cx, image.width), size(cy, image.height));
        if brush != 0 {
            let rect = [x, y, x.saturating_add(w), y.saturating_add(h)];
            // SAFETY: gdi32 validates the DC, the brush and the rectangle.
            unsafe { super::DoorGdiFillRect(hdc, rect.as_ptr() as u64, brush) };
        }
        let flags = if flags & (DI_MASK | DI_IMAGE) == 0 {
            DI_MASK | DI_IMAGE
        } else {
            flags
        };
        let px: Vec<u32> = match (flags & DI_MASK != 0, flags & DI_IMAGE != 0) {
            // The mask alone: black where the icon is opaque.
            (true, false) => image.px.iter().map(|p| p & 0xff00_0000).collect(),
            _ => image.px,
        };
        let opaque = u32::from(flags & DI_MASK == 0);
        // SAFETY: gdi32 copies the pixels through its checked boundary.
        unsafe {
            DoorGdiDrawPixels(
                hdc,
                x,
                y,
                w,
                h,
                px.as_ptr() as u64,
                image.width,
                image.height,
                opaque,
            )
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DrawIcon(hdc: usize, x: i32, y: i32, icon: usize) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let icon = door_windows_abi::widen(icon);
    {
        DrawIconEx(
            door_windows_abi::narrow(hdc),
            x,
            y,
            door_windows_abi::narrow(icon),
            0,
            0,
            0,
            0,
            DI_MASK | DI_IMAGE | DI_DEFAULTSIZE,
        )
    }
}
/// PrivateExtractIconsW: up to `count` icons (from index `index`; a
/// negative index names a resource id) of a program, DLL or `.ico` file
/// at `cx` x `cy`. With `icons` NULL, the number of icons in the file.
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn PrivateExtractIconsW(
    file: usize,
    index: i32,
    cx: i32,
    cy: i32,
    icons: usize,
    ids: usize,
    count: u32,
    _flags: u32,
) -> u32 {
    let file = door_windows_abi::widen(file);
    let icons = door_windows_abi::widen(icons);
    let ids = door_windows_abi::widen(ids);
    {
        // The shell's own icon library answers from the shared table.
        if g::read_wide(file, 520)
            .map(|units| alloc::string::String::from_utf16_lossy(&units))
            .is_some_and(|p| door_windows_abi::shell_icons::is_shell_library(&p))
        {
            if icons == 0 {
                return door_windows_abi::shell_icons::ICONS.len() as u32;
            }
            let size = if cx > 0 {
                (cx & 0xffff) as u32
            } else {
                ICON_SIZE
            };
            let mut made = 0;
            // A negative index names one icon by its id.
            let wanted = if index < 0 { 1 } else { count.min(64) };
            for n in 0..wanted {
                let index = if index < 0 {
                    index.unsigned_abs()
                } else {
                    index as u32 + n
                };
                let Some(icon) = door_windows_abi::shell_icons::stem(index)
                    .and_then(|stem| shell_image(stem, size))
                else {
                    break;
                };
                let h = insert(Icon {
                    image: icon,
                    cursor: false,
                    hotspot: (0, 0),
                    shared: false,
                });
                if h == 0 || !g::write_pointer(icons + g::POINTER as u64 * u64::from(n), h) {
                    break;
                }
                if ids != 0 {
                    let _ = g::write(ids + 4 * u64::from(n), &index.to_le_bytes());
                }
                made += 1;
            }
            return made;
        }
        let Some(bytes) = read_file(file) else {
            return 0;
        };
        let (cx, cy) = if cx <= 0 {
            (ICON_SIZE as i32, ICON_SIZE as i32)
        } else {
            (cx, cy)
        };
        let cx_low = cx & 0xffff;
        let cy_low = cy & 0xffff;
        if door_pe_icon::icon_file(&bytes).is_some() {
            if icons == 0 {
                return 1;
            }
            if index != 0 || count == 0 {
                return 0;
            }
            let Some(icon) = from_icon_file(&bytes, false, cx_low, cy_low, 0) else {
                return 0;
            };
            let h = insert(icon);
            return u32::from(h != 0 && g::write_pointer(icons, h));
        }
        let source: &[u8] = &bytes;
        let total = door_pe_icon::group_count(source).unwrap_or(0);
        if icons == 0 {
            return total;
        }
        let mut made = 0u32;
        for n in 0..count.min(64) {
            let which = if index < 0 {
                if n > 0 {
                    break;
                }
                door_pe_icon::Which::Id(index.unsigned_abs())
            } else {
                door_pe_icon::Which::Index(index as u32 + n)
            };
            let Ok(Some(entries)) = door_pe_icon::group_at(source, which) else {
                break;
            };
            let Some(entry) = door_pe_icon::choose_size(&entries, cx_low as u32, cy_low as u32)
            else {
                break;
            };
            let Some(frame) = door_pe_icon::frame(source, &entry).ok().flatten() else {
                break;
            };
            let Some(icon) = from_frame(frame, false, cx_low, cy_low, 0) else {
                break;
            };
            let h = insert(icon);
            if h == 0 || !g::write_pointer(icons + g::POINTER as u64 * u64::from(n), h) {
                break;
            }
            if ids != 0 {
                let _ = g::write(ids + 4 * u64::from(n), &entry.id.to_le_bytes());
            }
            made += 1;
        }
        made
    }
}
/// A shell icon table entry's pixels: an asset stem or `system:<id>`.
fn shell_image(stem: &str, size: u32) -> Option<Image> {
    match stem
        .strip_prefix("system:")
        .and_then(|id| id.parse::<u64>().ok())
    {
        Some(id) => System::from_id(id)
            .and_then(|w| system_icon(w, size))
            .map(|i| i.image),
        None => asset_image(stem, size),
    }
}
/// shell32's file-type icons: the asset `stem` (`icons/windows/folder`)
/// at `size`, a new icon the caller destroys.
#[unsafe(no_mangle)]
pub extern "system" fn DoorShellIcon(stem: u64, stem_len: u32, size: u32) -> u64 {
    let Some(name) = g::read_bytes(stem, stem_len.min(128) as usize) else {
        return fail(e::INVALID_PARAMETER);
    };
    let Ok(name) = core::str::from_utf8(&name) else {
        return fail(e::INVALID_PARAMETER);
    };
    match shell_image(name, size.clamp(SMALL_ICON, 256)) {
        Some(image) => insert(Icon {
            image,
            cursor: false,
            hotspot: (0, 0),
            shared: false,
        }),
        None => fail(e::RESOURCE_NAME_NOT_FOUND),
    }
}

// ---- window icons ---------------------------------------------------------

const ICON_SMALL: u64 = 0;
const ICON_BIG: u64 = 1;
const ICON_SMALL2: u64 = 2;
/// WM_SETICON: the previous icon of that kind.
pub(crate) fn set_window_icon(window: u64, which: u64, icon: u64) -> u64 {
    with(|i| {
        let at = match i.windows.iter().position(|w| w.0 == window) {
            Some(at) => at,
            None => {
                i.windows.push((window, 0, 0));
                i.windows.len() - 1
            }
        };
        let slot = &mut i.windows[at];
        match which {
            ICON_BIG => core::mem::replace(&mut slot.1, icon),
            ICON_SMALL => core::mem::replace(&mut slot.2, icon),
            _ => 0,
        }
    })
}
/// WM_GETICON: ICON_SMALL2 falls back to the big icon, as DefWindowProc
/// does.
pub(crate) fn window_icon(window: u64, which: u64) -> u64 {
    with(|i| {
        i.windows
            .iter()
            .find(|w| w.0 == window)
            .map_or(0, |w| match which {
                ICON_BIG => w.1,
                ICON_SMALL => w.2,
                ICON_SMALL2 if w.2 != 0 => w.2,
                ICON_SMALL2 => w.1,
                _ => 0,
            })
    })
}
/// A destroyed window forgets its icons (the icons themselves are the
/// application's).
pub(crate) fn forget_window(window: u64) {
    with(|i| i.windows.retain(|w| w.0 != window));
}
