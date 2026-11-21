//! OLE data transfer and the rest of what Chromium imports (P18):
//! drag and drop between this process's windows (RegisterDragDrop,
//! RevokeDragDrop and DoDragDrop's modal loop over the drop source and the
//! registered drop targets; other processes' windows are not targets),
//! STGMEDIUM release, OleDuplicateData for HGLOBAL formats, ILockBytes on
//! HGLOBAL (compound files over it are not supported), agile references
//! (Doors' apartments share the process, so the reference holds the
//! object), and CoSetProxyBlanket (in-process objects have no proxy:
//! E_NOINTERFACE unless the object implements IClientSecurity).
use super::*;

#[link(name = "user32", kind = "dylib")]
unsafe extern "system" {
    fn GetCursorPos(point: usize) -> i32;
    fn GetAsyncKeyState(key: i32) -> i16;
    fn WindowFromPoint(point: u64) -> usize;
    fn GetAncestor(hwnd: usize, flags: u32) -> usize;
    fn IsWindow(hwnd: usize) -> i32;
    fn GetCapture() -> usize;
    fn SetCapture(hwnd: usize) -> usize;
    fn ReleaseCapture() -> i32;
    fn PeekMessageW(msg: usize, hwnd: usize, first: u32, last: u32, remove: u32) -> i32;
    fn TranslateMessage(msg: usize) -> i32;
    fn DispatchMessageW(msg: usize) -> isize;
    fn PostQuitMessage(code: i32);
    fn MsgWaitForMultipleObjects(
        count: u32,
        handles: usize,
        all: i32,
        millis: u32,
        mask: u32,
    ) -> u32;
    fn SetCursor(cursor: usize) -> usize;
    fn LoadCursorW(instance: usize, name: usize) -> usize;
}
#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorDeleteFileW(path: u64) -> i32;
}

const DRAGDROP_E_NOTREGISTERED: u32 = 0x8004_0100;
const DRAGDROP_E_ALREADYREGISTERED: u32 = 0x8004_0101;
const DRAGDROP_E_INVALIDHWND: u32 = 0x8004_0102;
const DRAGDROP_S_DROP: u32 = 0x0004_0100;
const DRAGDROP_S_CANCEL: u32 = 0x0004_0101;
const DRAGDROP_S_USEDEFAULTCURSORS: u32 = 0x0004_0102;
const STG_E_UNIMPLEMENTEDFUNCTION: u32 = 0x8003_00FE;

fn call(object: u64, index: usize, args: &[u64]) -> u32 {
    // SAFETY: a COM interface the caller holds; slot `index` takes the
    // object and `N` pointer-width arguments (wider ones are passed as
    // u64, as POINTL by value is).
    unsafe {
        let f = method(object, index) as usize;
        let o = door_windows_abi::narrow(object);
        let arg = |i: usize| door_windows_abi::narrow(args.get(i).copied().unwrap_or(0));
        match args.len() {
            0 => core::mem::transmute::<usize, extern "system" fn(usize) -> u32>(f)(o),
            1 => {
                core::mem::transmute::<usize, extern "system" fn(usize, usize) -> u32>(f)(o, arg(0))
            }
            2 => core::mem::transmute::<usize, extern "system" fn(usize, usize, usize) -> u32>(f)(
                o,
                arg(0),
                arg(1),
            ),
            _ => hr::E_UNEXPECTED,
        }
    }
}

// ---- drop targets ---------------------------------------------------------------------
static TARGETS: Lock<Vec<(u64, u64)>> = Lock::new(Vec::new());

#[unsafe(no_mangle)]
pub extern "system" fn RegisterDragDrop(window: usize, target: usize) -> u32 {
    let (w, t) = (
        door_windows_abi::widen(window),
        door_windows_abi::widen(target),
    );
    if t == 0 {
        return hr::E_INVALIDARG;
    }
    if unsafe { IsWindow(window) } == 0 {
        return DRAGDROP_E_INVALIDHWND;
    }
    let added = TARGETS.with(|list| {
        if list.iter().any(|(x, _)| *x == w) {
            false
        } else {
            list.push((w, t));
            true
        }
    });
    if !added {
        return DRAGDROP_E_ALREADYREGISTERED;
    }
    add_ref(t);
    hr::S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn RevokeDragDrop(window: usize) -> u32 {
    let w = door_windows_abi::widen(window);
    let removed = TARGETS.with(|list| {
        list.iter()
            .position(|(x, _)| *x == w)
            .map(|i| list.remove(i).1)
    });
    match removed {
        Some(t) => {
            release(t);
            hr::S_OK
        }
        None if unsafe { IsWindow(window) } == 0 => DRAGDROP_E_INVALIDHWND,
        None => DRAGDROP_E_NOTREGISTERED,
    }
}
/// The registered target of the window at `point` or its nearest
/// registered ancestor (a new reference).
fn target_at(point: u64) -> Option<(u64, u64)> {
    let mut w = unsafe { WindowFromPoint(point) };
    for _ in 0..64 {
        if w == 0 {
            return None;
        }
        let hit = TARGETS.with(|list| {
            list.iter()
                .find(|(x, _)| *x == door_windows_abi::widen(w))
                .copied()
        });
        if let Some((window, target)) = hit {
            add_ref(target);
            return Some((window, target));
        }
        // GA_PARENT.
        w = unsafe { GetAncestor(w, 1) };
    }
    None
}
/// MK_ flags from the keyboard and buttons now.
fn key_state() -> u32 {
    let down = |vk| unsafe { GetAsyncKeyState(vk) } < 0;
    const KEYS: [(i32, u32); 6] = [
        (0x01, 0x01),
        (0x02, 0x02),
        (0x04, 0x10),
        (0x10, 0x04),
        (0x11, 0x08),
        (0x12, 0x20),
    ];
    KEYS.iter()
        .filter(|(vk, _)| down(*vk))
        .fold(0, |a, (_, mk)| a | mk)
}
fn cursor() -> u64 {
    let mut p = [0i32; 2];
    unsafe { GetCursorPos(p.as_mut_ptr() as usize) };
    u64::from(p[0] as u32) | (u64::from(p[1] as u32) << 32)
}
/// IDropTarget::DragEnter (3) / DragOver (4) / Drop (6): the effect the
/// target chose of `allowed`.
fn enter_over_drop(
    target: u64,
    slot: usize,
    data: u64,
    keys: u32,
    point: u64,
    allowed: u32,
) -> u32 {
    let mut effect = allowed;
    let at = &mut effect as *mut u32 as usize;
    // SAFETY: IDropTarget's methods: (this, [data,] keys, POINTL, *effect).
    let r = unsafe {
        let f = method(target, slot) as usize;
        let o = door_windows_abi::narrow(target);
        if slot == 4 {
            core::mem::transmute::<usize, extern "system" fn(usize, u32, u64, usize) -> u32>(f)(
                o, keys, point, at,
            )
        } else {
            core::mem::transmute::<usize, extern "system" fn(usize, usize, u32, u64, usize) -> u32>(
                f,
            )(o, door_windows_abi::narrow(data), keys, point, at)
        }
    };
    if r & 0x8000_0000 != 0 {
        0
    } else {
        effect & allowed
    }
}
/// DoDragDrop: a modal loop until the source drops or cancels. Mouse
/// input goes to the drag (the source window keeps the capture); other
/// messages are dispatched.
#[unsafe(no_mangle)]
pub extern "system" fn DoDragDrop(
    data: usize,
    source: usize,
    allowed: u32,
    effect_out: usize,
) -> u32 {
    let (data, source) = (
        door_windows_abi::widen(data),
        door_windows_abi::widen(source),
    );
    let effect_out = door_windows_abi::widen(effect_out);
    if data == 0 || source == 0 || effect_out == 0 {
        return hr::E_INVALIDARG;
    }
    let captured = unsafe { GetCapture() };
    let holder = if captured != 0 {
        captured
    } else {
        unsafe { WindowFromPoint(cursor()) }
    };
    if holder != 0 && captured == 0 {
        unsafe { SetCapture(holder) };
    }
    let mut over: Option<(u64, u64)> = None;
    let mut effect = 0u32;
    let mut escape = false;
    let mut last = (u64::MAX, u32::MAX);
    let mut quit = None;
    let mut msg = [0u8; 64];
    let outcome = loop {
        while unsafe { PeekMessageW(msg.as_mut_ptr() as usize, 0, 0, 0, 1) } != 0 {
            let message =
                u32::from_le_bytes(msg[POINTER..POINTER + 4].try_into().unwrap_or([0; 4]));
            let wparam = read_pointer(msg.as_ptr() as u64 + 2 * POINTER as u64).unwrap_or(0);
            match message {
                // WM_QUIT ends the drag; it is posted again afterwards.
                0x0012 => {
                    quit = Some(wparam as i32);
                    escape = true;
                }
                // Mouse input belongs to the drag.
                0x0200..=0x020E => {}
                // WM_KEYDOWN of Escape cancels; other keys change the state.
                0x0100 | 0x0104 => escape |= wparam == 0x1B,
                0x0101 | 0x0105 => {}
                _ => unsafe {
                    TranslateMessage(msg.as_ptr() as usize);
                    DispatchMessageW(msg.as_ptr() as usize);
                },
            }
        }
        let keys = key_state();
        // IDropSource::QueryContinueDrag (3).
        let r = call(source, 3, &[u64::from(escape), u64::from(keys)]);
        if r == DRAGDROP_S_DROP || r == DRAGDROP_S_CANCEL || r & 0x8000_0000 != 0 {
            break (r == DRAGDROP_S_DROP, keys);
        }
        let point = cursor();
        let now = target_at(point);
        match (&over, now) {
            (Some((w, t)), Some((nw, nt))) if *w == nw => {
                release(nt);
                if (point, keys) != last {
                    effect = enter_over_drop(*t, 4, data, keys, point, allowed);
                }
            }
            (_, now) => {
                if let Some((_, t)) = over.take() {
                    // IDropTarget::DragLeave (5).
                    call(t, 5, &[]);
                    release(t);
                }
                effect = match now {
                    Some((nw, nt)) => {
                        over = Some((nw, nt));
                        enter_over_drop(nt, 3, data, keys, point, allowed)
                    }
                    None => 0,
                };
            }
        }
        last = (point, keys);
        // IDropSource::GiveFeedback (4); the default cursors otherwise.
        if call(source, 4, &[u64::from(effect)]) == DRAGDROP_S_USEDEFAULTCURSORS {
            // IDC_NO, or the arrow (Doors has no copy/move drag cursors).
            let id: usize = if effect == 0 { 32648 } else { 32512 };
            unsafe { SetCursor(LoadCursorW(0, id)) };
        }
        unsafe { MsgWaitForMultipleObjects(0, 0, 0, 15, 0x04FF) };
    };
    let (dropped, keys) = outcome;
    let mut result = DRAGDROP_S_CANCEL;
    let mut final_effect = 0;
    if let Some((_, t)) = over.take() {
        if dropped && effect != 0 {
            final_effect = enter_over_drop(t, 6, data, keys, cursor(), effect);
            result = DRAGDROP_S_DROP;
        } else {
            call(t, 5, &[]);
        }
        release(t);
    }
    if holder != 0 && captured == 0 {
        unsafe { ReleaseCapture() };
    }
    if let Some(code) = quit {
        unsafe { PostQuitMessage(code) };
    }
    let _ = write_u32(effect_out, final_effect);
    result
}

// ---- STGMEDIUM and HGLOBAL data -----------------------------------------------------------
/// ReleaseStgMedium over STGMEDIUM (tymed, the union, pUnkForRelease).
#[unsafe(no_mangle)]
pub extern "system" fn ReleaseStgMedium(medium: usize) {
    let medium = door_windows_abi::widen(medium);
    let Ok(tymed) = read_u32(medium) else {
        return;
    };
    let value = read_pointer(medium + POINTER as u64).unwrap_or(0);
    let owner = read_pointer(medium + 2 * POINTER as u64).unwrap_or(0);
    match tymed {
        // TYMED_HGLOBAL: a process-heap block (GlobalAlloc's).
        1 if owner == 0 && value != 0 => unsafe {
            RtlFreeHeap(
                door_windows_abi::narrow(heap()),
                0,
                door_windows_abi::narrow(value),
            );
        },
        // TYMED_FILE: the file goes too when nobody else owns it.
        2 if value != 0 => {
            if owner == 0 {
                unsafe { DoorDeleteFileW(value) };
            }
            CoTaskMemFree(door_windows_abi::narrow(value));
        }
        // TYMED_ISTREAM, TYMED_ISTORAGE.
        4 | 8 if value != 0 => {
            release(value);
        }
        _ => {}
    }
    if owner != 0 {
        release(owner);
    }
    let _ = write(medium, &[0u8; 3 * 8][..3 * POINTER]);
}
fn duplicate_block(handle: u64) -> u64 {
    let size = unsafe {
        RtlSizeHeap(
            door_windows_abi::narrow(heap()),
            0,
            door_windows_abi::narrow(handle),
        )
    };
    if size == usize::MAX {
        return 0;
    }
    let block = unsafe { RtlAllocateHeap(door_windows_abi::narrow(heap()), 0, size.max(1)) };
    if block != 0 {
        // SAFETY: both blocks hold `size` bytes.
        unsafe { core::ptr::copy_nonoverlapping(handle as *const u8, block as *mut u8, size) };
    }
    door_windows_abi::widen(block)
}
/// OleDuplicateData: HGLOBAL formats; GDI ones (CF_BITMAP, CF_PALETTE,
/// CF_METAFILEPICT, CF_ENHMETAFILE) are not duplicated here.
#[unsafe(no_mangle)]
pub extern "system" fn OleDuplicateData(source: usize, format: u16, _flags: u32) -> usize {
    let source = door_windows_abi::widen(source);
    if source == 0 || matches!(format, 2 | 3 | 9 | 14) {
        return 0;
    }
    door_windows_abi::narrow(duplicate_block(source))
}

// ---- ILockBytes on HGLOBAL ----------------------------------------------------------------
struct Bytes {
    refs: AtomicU32,
    handle: Lock<(u64, u64)>,
    delete: bool,
}
#[repr(C)]
struct BytesObject {
    vtbl: *const (),
    bytes: Bytes,
}
#[repr(transparent)]
struct Vtbl<const N: usize>([*const (); N]);
// SAFETY: immutable function addresses.
unsafe impl<const N: usize> Sync for Vtbl<N> {}
static BYTES_VTBL: Vtbl<10> = Vtbl([
    b_query as *const (),
    b_add_ref as *const (),
    b_release as *const (),
    b_read as *const (),
    b_write as *const (),
    b_flush as *const (),
    b_set_size as *const (),
    b_lock as *const (),
    b_lock as *const (),
    b_stat as *const (),
]);
const IID_ILOCKBYTES: Guid = guid(0x0000_000A, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
fn bytes<'a>(this: usize) -> &'a Bytes {
    // SAFETY: only our objects reach this table.
    unsafe { &(*(this as *const BytesObject)).bytes }
}
extern "system" fn b_query(this: usize, iid: usize, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    let Ok(g) = read_guid(door_windows_abi::widen(iid)) else {
        return hr::E_POINTER;
    };
    if g == IID_IUNKNOWN || g == IID_ILOCKBYTES {
        b_add_ref(this);
        result(write_pointer(out, door_windows_abi::widen(this)))
    } else {
        let _ = write_pointer(out, 0);
        hr::E_NOINTERFACE
    }
}
extern "system" fn b_add_ref(this: usize) -> u32 {
    bytes(this).refs.fetch_add(1, Ordering::AcqRel) + 1
}
extern "system" fn b_release(this: usize) -> u32 {
    let left = bytes(this).refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if left == 0 {
        // SAFETY: the last reference to a Box::into_raw object.
        let o = unsafe { Box::from_raw(this as *mut BytesObject) };
        if o.bytes.delete {
            let (h, _) = o.bytes.handle.with(|h| *h);
            unsafe {
                RtlFreeHeap(
                    door_windows_abi::narrow(heap()),
                    0,
                    door_windows_abi::narrow(h),
                )
            };
        }
    }
    left
}
extern "system" fn b_read(this: usize, offset: u64, out: usize, count: u32, done: usize) -> u32 {
    let (h, size) = bytes(this).handle.with(|h| *h);
    let n = size.saturating_sub(offset).min(u64::from(count));
    if n > 0 {
        // SAFETY: [offset, offset + n) is inside the block.
        let source = unsafe { core::slice::from_raw_parts((h + offset) as *const u8, n as usize) };
        if write(door_windows_abi::widen(out), source).is_err() {
            return hr::STG_E_INVALIDPOINTER;
        }
    }
    let done = door_windows_abi::widen(done);
    if done != 0 {
        let _ = write_u32(done, n as u32);
    }
    hr::S_OK
}
fn grow(b: &Bytes, size: u64) -> Result<(), u32> {
    b.handle.with(|(h, logical)| {
        let capacity = unsafe {
            RtlSizeHeap(
                door_windows_abi::narrow(heap()),
                0,
                door_windows_abi::narrow(*h),
            )
        } as u64;
        if size > capacity {
            let n = unsafe {
                RtlReAllocateHeap(
                    door_windows_abi::narrow(heap()),
                    0,
                    door_windows_abi::narrow(*h),
                    size as usize,
                )
            };
            if n == 0 {
                return Err(hr::STG_E_MEDIUMFULL);
            }
            *h = door_windows_abi::widen(n);
        }
        *logical = size;
        Ok(())
    })
}
extern "system" fn b_write(this: usize, offset: u64, data: usize, count: u32, done: usize) -> u32 {
    let b = bytes(this);
    let end = offset.saturating_add(u64::from(count));
    let (_, size) = b.handle.with(|h| *h);
    if end > size
        && let Err(e) = grow(b, end)
    {
        return e;
    }
    let (h, _) = b.handle.with(|h| *h);
    // SAFETY: [offset, end) is inside the block.
    let target =
        unsafe { core::slice::from_raw_parts_mut((h + offset) as *mut u8, count as usize) };
    if read(door_windows_abi::widen(data), target).is_err() {
        return hr::STG_E_INVALIDPOINTER;
    }
    let done = door_windows_abi::widen(done);
    if done != 0 {
        let _ = write_u32(done, count);
    }
    hr::S_OK
}
extern "system" fn b_flush(_this: usize) -> u32 {
    hr::S_OK
}
extern "system" fn b_set_size(this: usize, size: u64) -> u32 {
    result(grow(bytes(this), size))
}
extern "system" fn b_lock(_this: usize, _offset: u64, _count: u64, _kind: u32) -> u32 {
    // STG_E_INVALIDFUNCTION: HGLOBAL byte arrays support no locking.
    hr::STG_E_INVALIDFUNCTION
}
/// STATSTG: STGTY_LOCKBYTES and the size; no name.
extern "system" fn b_stat(this: usize, out: usize, _flags: u32) -> u32 {
    let (_, size) = bytes(this).handle.with(|h| *h);
    let mut s = [0u8; 80];
    let (kind_at, size_at, total) = if POINTER == 8 {
        (8, 16, 80)
    } else {
        (4, 8, 72)
    };
    s[kind_at..kind_at + 4].copy_from_slice(&3u32.to_le_bytes());
    s[size_at..size_at + 8].copy_from_slice(&size.to_le_bytes());
    result(write(door_windows_abi::widen(out), &s[..total]))
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateILockBytesOnHGlobal(handle: usize, delete: i32, out: usize) -> u32 {
    let (handle, out) = (
        door_windows_abi::widen(handle),
        door_windows_abi::widen(out),
    );
    if out == 0 {
        return hr::E_INVALIDARG;
    }
    let (h, size) = if handle == 0 {
        let block = unsafe { RtlAllocateHeap(door_windows_abi::narrow(heap()), 0, 1) };
        if block == 0 {
            return hr::E_OUTOFMEMORY;
        }
        (door_windows_abi::widen(block), 0)
    } else {
        let size = unsafe {
            RtlSizeHeap(
                door_windows_abi::narrow(heap()),
                0,
                door_windows_abi::narrow(handle),
            )
        };
        if size == usize::MAX {
            return hr::E_INVALIDARG;
        }
        (handle, size as u64)
    };
    let object = Box::into_raw(Box::new(BytesObject {
        vtbl: &BYTES_VTBL as *const _ as *const (),
        bytes: Bytes {
            refs: AtomicU32::new(1),
            handle: Lock::new((h, size)),
            delete: delete != 0,
        },
    })) as u64;
    result(write_pointer(out, object))
}
#[unsafe(no_mangle)]
pub extern "system" fn GetHGlobalFromILockBytes(object: usize, out: usize) -> u32 {
    let (o, out) = (
        door_windows_abi::widen(object),
        door_windows_abi::widen(out),
    );
    if o == 0 || out == 0 {
        return hr::E_INVALIDARG;
    }
    // SAFETY: reads the object's first field (its table).
    if unsafe { *(o as *const *const ()) } != &BYTES_VTBL as *const _ as *const () {
        return hr::E_INVALIDARG;
    }
    let (h, _) = bytes(object).handle.with(|h| *h);
    result(write_pointer(out, h))
}
/// Compound files (structured storage) are not implemented.
#[unsafe(no_mangle)]
pub extern "system" fn StgCreateDocfileOnILockBytes(
    _bytes: usize,
    _mode: u32,
    _reserved: u32,
    out: usize,
) -> u32 {
    let _ = write_pointer(door_windows_abi::widen(out), 0);
    STG_E_UNIMPLEMENTEDFUNCTION
}

// ---- agile references and proxy security --------------------------------------------------
const IID_IAGILEREFERENCE: Guid = guid(
    0xC03F_6A43,
    0x65A4,
    0x9818,
    [0x98, 0x7E, 0xE0, 0xB8, 0x10, 0xD2, 0xA6, 0xF2],
);
const IID_ICLIENTSECURITY: Guid = guid(0x0000_013D, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
#[repr(C)]
struct Agile {
    vtbl: *const (),
    refs: AtomicU32,
    object: u64,
}
static AGILE_VTBL: Vtbl<4> = Vtbl([
    a_query as *const (),
    a_add_ref as *const (),
    a_release as *const (),
    a_resolve as *const (),
]);
fn agile<'a>(this: usize) -> &'a Agile {
    // SAFETY: only our objects reach this table.
    unsafe { &*(this as *const Agile) }
}
extern "system" fn a_query(this: usize, iid: usize, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    let Ok(g) = read_guid(door_windows_abi::widen(iid)) else {
        return hr::E_POINTER;
    };
    if g == IID_IUNKNOWN || g == IID_IAGILEREFERENCE {
        a_add_ref(this);
        result(write_pointer(out, door_windows_abi::widen(this)))
    } else {
        let _ = write_pointer(out, 0);
        hr::E_NOINTERFACE
    }
}
extern "system" fn a_add_ref(this: usize) -> u32 {
    agile(this).refs.fetch_add(1, Ordering::AcqRel) + 1
}
extern "system" fn a_release(this: usize) -> u32 {
    let left = agile(this).refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if left == 0 {
        // SAFETY: the last reference to a Box::into_raw object.
        let a = unsafe { Box::from_raw(this as *mut Agile) };
        release(a.object);
    }
    left
}
extern "system" fn a_resolve(this: usize, iid: usize, out: usize) -> u32 {
    let Ok(g) = read_guid(door_windows_abi::widen(iid)) else {
        return hr::E_POINTER;
    };
    query_interface(agile(this).object, &g, door_windows_abi::widen(out))
}
#[unsafe(no_mangle)]
pub extern "system" fn RoGetAgileReference(
    _options: u32,
    iid: usize,
    object: usize,
    out: usize,
) -> u32 {
    let (object, out) = (
        door_windows_abi::widen(object),
        door_windows_abi::widen(out),
    );
    if object == 0 || out == 0 {
        return hr::E_INVALIDARG;
    }
    let _ = write_pointer(out, 0);
    let Ok(g) = read_guid(door_windows_abi::widen(iid)) else {
        return hr::E_POINTER;
    };
    // The object must answer the interface; the reference keeps that one.
    let mut held = 0u64;
    let r = query_interface(object, &g, &mut held as *mut u64 as u64);
    if r != hr::S_OK || held == 0 {
        return if r == hr::S_OK { hr::E_NOINTERFACE } else { r };
    }
    let a = Box::into_raw(Box::new(Agile {
        vtbl: &AGILE_VTBL as *const _ as *const (),
        refs: AtomicU32::new(1),
        object: held,
    })) as u64;
    result(write_pointer(out, a))
}
#[unsafe(no_mangle)]
pub extern "system" fn CoSetProxyBlanket(
    proxy: usize,
    authn: u32,
    authz: u32,
    principal: usize,
    level: u32,
    impersonation: u32,
    identity: usize,
    capabilities: u32,
) -> u32 {
    let proxy = door_windows_abi::widen(proxy);
    if proxy == 0 {
        return hr::E_INVALIDARG;
    }
    let mut security = 0u64;
    if query_interface(
        proxy,
        &IID_ICLIENTSECURITY,
        &mut security as *mut u64 as u64,
    ) != hr::S_OK
        || security == 0
    {
        return hr::E_NOINTERFACE;
    }
    // IClientSecurity::SetBlanket (4).
    // SAFETY: the object's IClientSecurity table.
    let r = unsafe {
        core::mem::transmute::<
            usize,
            extern "system" fn(usize, usize, u32, u32, usize, u32, u32, usize, u32) -> u32,
        >(method(security, 4) as usize)(
            door_windows_abi::narrow(security),
            door_windows_abi::narrow(proxy),
            authn,
            authz,
            principal,
            level,
            impersonation,
            identity,
            capabilities,
        )
    };
    release(security);
    r
}
