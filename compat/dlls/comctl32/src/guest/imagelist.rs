//! Image lists (ImageList_*): equal-sized images kept as icons, so masks
//! and per-pixel alpha draw through user32's DrawIconEx. Bitmap strips
//! (ImageList_Add / AddMasked) are cut into icons through
//! CreateIconFromResourceEx; the list owns its icons and destroys them.
use alloc::vec::Vec;
use door_windows_gui_guest as g;

#[link(name = "user32", kind = "dylib")]
unsafe extern "system" {
    fn CopyImage(image: usize, kind: u32, cx: i32, cy: i32, flags: u32) -> usize;
    fn CopyIcon(icon: usize) -> usize;
    fn DestroyIcon(icon: usize) -> i32;
    fn DrawIconEx(
        hdc: usize,
        x: i32,
        y: i32,
        icon: usize,
        cx: i32,
        cy: i32,
        step: u32,
        brush: usize,
        flags: u32,
    ) -> i32;
    fn CreateIconFromResourceEx(
        bits: usize,
        size: u32,
        icon: i32,
        version: u32,
        cx: i32,
        cy: i32,
        flags: u32,
    ) -> usize;
}
#[link(name = "gdi32", kind = "dylib")]
unsafe extern "system" {
    fn DoorGdiBitmapPixels(bitmap: u64, out: u64, pixels: u32, size: u64) -> i32;
}

const IMAGE_ICON: u32 = 1;
const DI_NORMAL: u32 = 3;
const DI_MASK: u32 = 1;
const ILD_MASK: u32 = 0x10;
const CLR_NONE: u32 = 0xffff_ffff;
const MAX_LISTS: usize = 1024;
const MAX_IMAGES: usize = 4096;

struct List {
    cx: i32,
    cy: i32,
    icons: Vec<u64>,
    background: u32,
}
static LISTS: g::Lock<Vec<Option<List>>> = g::Lock::new(Vec::new());
const BASE: u64 = 0x4c49_0000;

fn index(handle: u64) -> Option<usize> {
    let i = handle.checked_sub(BASE)? as usize;
    (1..=MAX_LISTS).contains(&i).then_some(i - 1)
}
fn with<R>(handle: u64, f: impl FnOnce(&mut List) -> R) -> Option<R> {
    let mut lists = LISTS.lock();
    lists.get_mut(index(handle)?)?.as_mut().map(f)
}

#[unsafe(no_mangle)]
pub extern "system" fn ImageList_Create(
    cx: i32,
    cy: i32,
    _flags: u32,
    _initial: i32,
    _grow: i32,
) -> usize {
    door_windows_abi::narrow((move || -> u64 {
        if !(1..=256).contains(&cx) || !(1..=256).contains(&cy) {
            return 0;
        }
        let mut lists = LISTS.lock();
        let list = List {
            cx,
            cy,
            icons: Vec::new(),
            background: CLR_NONE,
        };
        let slot = match lists.iter().position(Option::is_none) {
            Some(i) => i,
            None if lists.len() < MAX_LISTS => {
                lists.push(None);
                lists.len() - 1
            }
            None => return 0,
        };
        lists[slot] = Some(list);
        BASE + slot as u64 + 1
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn ImageList_Destroy(handle: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    {
        let taken = {
            let mut lists = LISTS.lock();
            index(handle).and_then(|i| lists.get_mut(i)?.take())
        };
        match taken {
            Some(list) => {
                for icon in list.icons {
                    unsafe { DestroyIcon(door_windows_abi::narrow(icon)) };
                }
                1
            }
            None => 0,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ImageList_GetImageCount(handle: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    with(handle, |l| l.icons.len() as i32).unwrap_or(0)
}
#[unsafe(no_mangle)]
pub extern "system" fn ImageList_SetImageCount(handle: usize, count: u32) -> i32 {
    let handle = door_windows_abi::widen(handle);
    {
        let removed = with(handle, |l| {
            let count = (count as usize).min(MAX_IMAGES);
            if count <= l.icons.len() {
                l.icons.split_off(count)
            } else {
                l.icons.resize(count, 0);
                Vec::new()
            }
        });
        match removed {
            Some(icons) => {
                for icon in icons.into_iter().filter(|i| *i != 0) {
                    unsafe { DestroyIcon(door_windows_abi::narrow(icon)) };
                }
                1
            }
            None => 0,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ImageList_GetIconSize(handle: usize, cx: usize, cy: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let cx = door_windows_abi::widen(cx);
    let cy = door_windows_abi::widen(cy);
    {
        let Some((w, h)) = with(handle, |l| (l.cx, l.cy)) else {
            return 0;
        };
        i32::from(
            (cx == 0 || g::write(cx, &w.to_le_bytes()))
                && (cy == 0 || g::write(cy, &h.to_le_bytes())),
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ImageList_SetIconSize(handle: usize, cx: i32, cy: i32) -> i32 {
    let handle = door_windows_abi::widen(handle);
    {
        if !(1..=256).contains(&cx) || !(1..=256).contains(&cy) {
            return 0;
        }
        match with(handle, |l| {
            l.cx = cx;
            l.cy = cy;
            core::mem::take(&mut l.icons)
        }) {
            Some(icons) => {
                for icon in icons.into_iter().filter(|i| *i != 0) {
                    unsafe { DestroyIcon(door_windows_abi::narrow(icon)) };
                }
                1
            }
            None => 0,
        }
    }
}
/// ImageList_ReplaceIcon (ImageList_AddIcon is `index` -1): a copy of the
/// icon at the list's size; its index, or -1.
#[unsafe(no_mangle)]
pub extern "system" fn ImageList_ReplaceIcon(handle: usize, index: i32, icon: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let icon = door_windows_abi::widen(icon);
    {
        let Some((cx, cy)) = with(handle, |l| (l.cx, l.cy)) else {
            return -1;
        };
        let copy = unsafe { CopyImage(door_windows_abi::narrow(icon), IMAGE_ICON, cx, cy, 0) };
        if copy == 0 {
            return -1;
        }
        put(handle, index, door_windows_abi::widen(copy))
    }
}
/// Store an icon the list now owns at `index` (-1 appends).
fn put(handle: u64, index: i32, icon: u64) -> i32 {
    let outcome = with(handle, |l| {
        if index < 0 {
            if l.icons.len() >= MAX_IMAGES {
                return (-1, Some(icon));
            }
            l.icons.push(icon);
            ((l.icons.len() - 1) as i32, None)
        } else if (index as usize) < l.icons.len() {
            let old = core::mem::replace(&mut l.icons[index as usize], icon);
            (index, (old != 0).then_some(old))
        } else {
            (-1, Some(icon))
        }
    });
    match outcome {
        Some((at, drop)) => {
            if let Some(d) = drop {
                unsafe { DestroyIcon(door_windows_abi::narrow(d)) };
            }
            at
        }
        None => {
            unsafe { DestroyIcon(door_windows_abi::narrow(icon)) };
            -1
        }
    }
}
/// A bitmap's `0xAARRGGBB` pixels and size.
fn pixels(bitmap: u64) -> Option<(u32, u32, Vec<u32>)> {
    let mut size = [0u32; 2];
    if unsafe { DoorGdiBitmapPixels(bitmap, 0, 0, size.as_mut_ptr() as u64) } == 0 {
        return None;
    }
    let n = size[0] as usize * size[1] as usize;
    if n == 0 || n > 1 << 20 {
        return None;
    }
    let mut px = alloc::vec![0u32; n];
    (unsafe {
        DoorGdiBitmapPixels(
            bitmap,
            px.as_mut_ptr() as u64,
            n as u32,
            size.as_mut_ptr() as u64,
        )
    } != 0)
        .then_some((size[0], size[1], px))
}
/// One `cx` x `cy` tile of a strip as an RT_ICON frame (a 32-bit DIB with
/// its AND mask), made into an icon.
fn tile_icon(
    strip: &(u32, u32, Vec<u32>),
    left: u32,
    cx: u32,
    cy: u32,
    hidden: impl Fn(u32, u32, u32) -> bool,
) -> u64 {
    let (w, h, px) = strip;
    let mask_stride = cx.div_ceil(32) as usize * 4;
    let mut frame = Vec::new();
    for v in [40u32, cx, 2 * cy] {
        frame.extend_from_slice(&v.to_le_bytes());
    }
    frame.extend_from_slice(&1u16.to_le_bytes());
    frame.extend_from_slice(&32u16.to_le_bytes());
    frame.extend_from_slice(&[0; 24]);
    let alpha = px.iter().any(|p| p >> 24 != 0);
    let mut mask = alloc::vec![0u8; mask_stride * cy as usize];
    // Rows bottom-up.
    for row in 0..cy {
        let y = cy - 1 - row;
        for x in 0..cx {
            let (sx, sy) = (left + x, y);
            let p = if sx < *w && sy < *h {
                px[(sy * w + sx) as usize]
            } else {
                0
            };
            let gone = hidden(sx, sy, p);
            let a = if alpha {
                p >> 24
            } else if gone {
                0
            } else {
                255
            };
            frame.extend_from_slice(&(p & 0x00ff_ffff | a << 24).to_le_bytes());
            if gone || (alpha && a == 0) {
                mask[row as usize * mask_stride + x as usize / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    frame.extend_from_slice(&mask);
    unsafe {
        door_windows_abi::widen(CreateIconFromResourceEx(
            door_windows_abi::narrow(frame.as_ptr() as u64),
            frame.len() as u32,
            1,
            0x0003_0000,
            cx as i32,
            cy as i32,
            0,
        ))
    }
}
fn add_strip(handle: u64, bitmap: u64, hidden: impl Fn(u32, u32, u32) -> bool) -> i32 {
    let Some((cx, cy)) = with(handle, |l| (l.cx as u32, l.cy as u32)) else {
        return -1;
    };
    let Some(strip) = pixels(bitmap) else {
        return -1;
    };
    let count = (strip.0 / cx).max(1);
    let mut first = -1;
    for i in 0..count {
        let icon = tile_icon(&strip, i * cx, cx, cy, &hidden);
        if icon == 0 {
            return -1;
        }
        let at = put(handle, -1, icon);
        if at < 0 {
            return -1;
        }
        if first < 0 {
            first = at;
        }
    }
    first
}
/// ImageList_Add: a strip of images, with a monochrome mask strip (white =
/// transparent) or none.
#[unsafe(no_mangle)]
pub extern "system" fn ImageList_Add(handle: usize, image: usize, mask: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let image = door_windows_abi::widen(image);
    let mask = door_windows_abi::widen(mask);
    {
        let mask_px = if mask == 0 { None } else { pixels(mask) };
        add_strip(handle, image, |x, y, _| {
            mask_px.as_ref().is_some_and(|(w, h, px)| {
                x < *w && y < *h && px[(y * w + x) as usize] & 0x00ff_ffff == 0x00ff_ffff
            })
        })
    }
}
/// ImageList_AddMasked: pixels of `key` (a COLORREF) are transparent.
#[unsafe(no_mangle)]
pub extern "system" fn ImageList_AddMasked(handle: usize, image: usize, key: u32) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let image = door_windows_abi::widen(image);
    {
        // COLORREF 0x00BBGGRR against pixels 0xAARRGGBB.
        let rgb = (key & 0xff) << 16 | (key & 0xff00) | (key >> 16 & 0xff);
        add_strip(handle, image, |_, _, p| {
            key != CLR_NONE && p & 0x00ff_ffff == rgb
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ImageList_Remove(handle: usize, index: i32) -> i32 {
    let handle = door_windows_abi::widen(handle);
    {
        let removed = with(handle, |l| {
            if index < 0 {
                Some(core::mem::take(&mut l.icons))
            } else if (index as usize) < l.icons.len() {
                Some(alloc::vec![l.icons.remove(index as usize)])
            } else {
                None
            }
        });
        match removed.flatten() {
            Some(icons) => {
                for icon in icons.into_iter().filter(|i| *i != 0) {
                    unsafe { DestroyIcon(door_windows_abi::narrow(icon)) };
                }
                1
            }
            None => 0,
        }
    }
}
fn icon_at(handle: u64, index: i32) -> Option<(u64, i32, i32)> {
    with(handle, |l| {
        let icon = *l.icons.get(usize::try_from(index).ok()?)?;
        (icon != 0).then_some((icon, l.cx, l.cy))
    })
    .flatten()
}
#[unsafe(no_mangle)]
pub extern "system" fn ImageList_GetIcon(handle: usize, index: i32, _flags: u32) -> usize {
    let handle = door_windows_abi::widen(handle);
    door_windows_abi::narrow((move || -> u64 {
        match icon_at(handle, index) {
            Some((icon, _, _)) => unsafe {
                door_windows_abi::widen(CopyIcon(door_windows_abi::narrow(icon)))
            },
            None => 0,
        }
    })())
}
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn ImageList_DrawEx(
    handle: usize,
    index: i32,
    hdc: usize,
    x: i32,
    y: i32,
    dx: i32,
    dy: i32,
    _background: u32,
    _foreground: u32,
    style: u32,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let hdc = door_windows_abi::widen(hdc);
    {
        let Some((icon, cx, cy)) = icon_at(handle, index) else {
            return 0;
        };
        let (w, h) = (if dx > 0 { dx } else { cx }, if dy > 0 { dy } else { cy });
        let flags = if style & ILD_MASK != 0 {
            DI_MASK
        } else {
            DI_NORMAL
        };
        unsafe {
            DrawIconEx(
                door_windows_abi::narrow(hdc),
                x,
                y,
                door_windows_abi::narrow(icon),
                w,
                h,
                0,
                0,
                flags,
            )
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ImageList_Draw(
    handle: usize,
    index: i32,
    hdc: usize,
    x: i32,
    y: i32,
    style: u32,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let hdc = door_windows_abi::widen(hdc);
    {
        ImageList_DrawEx(
            door_windows_abi::narrow(handle),
            index,
            door_windows_abi::narrow(hdc),
            x,
            y,
            0,
            0,
            CLR_NONE,
            CLR_NONE,
            style,
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ImageList_SetBkColor(handle: usize, color: u32) -> u32 {
    let handle = door_windows_abi::widen(handle);
    with(handle, |l| core::mem::replace(&mut l.background, color)).unwrap_or(CLR_NONE)
}
#[unsafe(no_mangle)]
pub extern "system" fn ImageList_GetBkColor(handle: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    with(handle, |l| l.background).unwrap_or(CLR_NONE)
}
#[unsafe(no_mangle)]
pub extern "system" fn ImageList_Duplicate(handle: usize) -> usize {
    let handle = door_windows_abi::widen(handle);
    door_windows_abi::narrow((move || -> u64 {
        let Some((cx, cy, icons, background)) =
            with(handle, |l| (l.cx, l.cy, l.icons.clone(), l.background))
        else {
            return 0;
        };
        let copy = ImageList_Create(cx, cy, 0x21, icons.len() as i32, 4);
        if copy == 0 {
            return 0;
        }
        ImageList_SetBkColor(copy, background);
        for icon in icons {
            let duplicate = if icon == 0 {
                0
            } else {
                unsafe { CopyIcon(door_windows_abi::narrow(icon)) }
            };
            if put(
                door_windows_abi::widen(copy),
                -1,
                door_windows_abi::widen(duplicate),
            ) < 0
            {
                ImageList_Destroy(copy);
                return 0;
            }
        }
        door_windows_abi::widen(copy)
    })())
}
