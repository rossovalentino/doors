//! oleaut32.dll exports: BSTR, VARIANT and its conversions (the shared
//! engine in `variant`), the Var*From* family, Var arithmetic, dates,
//! SAFEARRAY (`guest/safearray.rs`) and error info (`guest/errors.rs`).
use crate::decimal::Decimal;
use crate::variant::{self, Value};
use crate::{date, hr, vt};
use alloc::{vec, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};
mod dispatch;
mod errors;
mod marshal;
mod record;
mod register;
mod safearray;
mod typeinfo;

struct Lock<T> {
    held: AtomicBool,
    value: core::cell::UnsafeCell<T>,
}
unsafe impl<T: Send> Sync for Lock<T> {}
impl<T> Lock<T> {
    const fn new(value: T) -> Self {
        Self {
            held: AtomicBool::new(false),
            value: core::cell::UnsafeCell::new(value),
        }
    }
    fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        while self.held.swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }
        // SAFETY: `held` gives this closure the only access.
        let r = f(unsafe { &mut *self.value.get() });
        self.held.store(false, Ordering::Release);
        r
    }
}

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorUserCopy(destination: u64, source: u64, bytes: usize) -> u32;
    fn DoorGetCurrentThreadId() -> u32;
    fn DoorGetProcessHeap() -> u64;
    fn RtlAllocateHeap(heap: usize, flags: u32, bytes: usize) -> usize;
    fn RtlFreeHeap(heap: usize, flags: u32, pointer: usize) -> u8;
}

// ---- application memory ------------------------------------------------------------
const COPY_CHUNK: usize = door_windows_abi::APPLICATION_COPY_BYTES;
fn read(pointer: u64, out: &mut [u8]) -> Result<(), u32> {
    if pointer == 0 {
        return Err(hr::E_POINTER);
    }
    for (i, chunk) in out.chunks_mut(COPY_CHUNK).enumerate() {
        if unsafe {
            DoorUserCopy(
                chunk.as_mut_ptr() as u64,
                pointer + (i * COPY_CHUNK) as u64,
                chunk.len(),
            )
        } != 0
        {
            return Err(hr::E_POINTER);
        }
    }
    Ok(())
}
fn write(pointer: u64, data: &[u8]) -> Result<(), u32> {
    if pointer == 0 {
        return Err(hr::E_POINTER);
    }
    for (i, chunk) in data.chunks(COPY_CHUNK).enumerate() {
        if unsafe {
            DoorUserCopy(
                pointer + (i * COPY_CHUNK) as u64,
                chunk.as_ptr() as u64,
                chunk.len(),
            )
        } != 0
        {
            return Err(hr::E_POINTER);
        }
    }
    Ok(())
}
fn read_n<const N: usize>(pointer: u64) -> Result<[u8; N], u32> {
    let mut b = [0u8; N];
    read(pointer, &mut b)?;
    Ok(b)
}
fn read_u16(p: u64) -> Result<u16, u32> {
    Ok(u16::from_le_bytes(read_n(p)?))
}
fn read_u32(p: u64) -> Result<u32, u32> {
    Ok(u32::from_le_bytes(read_n(p)?))
}
fn read_u64(p: u64) -> Result<u64, u32> {
    Ok(u64::from_le_bytes(read_n(p)?))
}
fn write_u16(p: u64, v: u16) -> Result<(), u32> {
    write(p, &v.to_le_bytes())
}
fn write_u32(p: u64, v: u32) -> Result<(), u32> {
    write(p, &v.to_le_bytes())
}
fn write_u64(p: u64, v: u64) -> Result<(), u32> {
    write(p, &v.to_le_bytes())
}
/// The program's pointer width (P17: 4 for a 32-bit program).
pub(crate) const POINTER: usize = door_windows_abi::layout::POINTER;
/// A pointer-sized program value (BSTRs, interfaces, SAFEARRAYs, out
/// parameters), sign-extended on x86.
fn read_pointer(p: u64) -> Result<u64, u32> {
    let mut b = [0u8; 8];
    read(p, &mut b[..POINTER])?;
    Ok(door_windows_abi::layout::get(
        &b,
        &[door_windows_abi::layout::Field::Pointer],
        0,
    ))
}
fn write_pointer(p: u64, v: u64) -> Result<(), u32> {
    write(p, &v.to_le_bytes()[..POINTER])
}
/// A NUL-terminated UTF-16 string.
fn read_wide(pointer: u64) -> Result<Vec<u16>, u32> {
    if pointer == 0 {
        return Err(hr::E_POINTER);
    }
    let mut out = Vec::new();
    loop {
        let unit = read_u16(pointer + 2 * out.len() as u64)?;
        if unit == 0 {
            return Ok(out);
        }
        out.push(unit);
        if out.len() > 1 << 24 {
            return Err(hr::E_INVALIDARG);
        }
    }
}
fn result(r: Result<(), u32>) -> u32 {
    r.err().unwrap_or(hr::S_OK)
}
fn heap() -> u64 {
    unsafe { DoorGetProcessHeap() }
}
fn thread() -> u32 {
    unsafe { DoorGetCurrentThreadId() }
}

// ---- COM objects --------------------------------------------------------------------
/// SAFETY: `object` is a live interface pointer in this process.
/// Method `index` of a COM object: its first field is the vtable, a table
/// of pointer-sized entries (P17: 4 bytes in a 32-bit process).
unsafe fn method(object: u64, index: usize) -> u64 {
    unsafe { *((*(object as *const usize) + index * POINTER) as *const usize) as u64 }
}
fn add_ref(object: u64) -> u32 {
    if object == 0 {
        return 0;
    }
    // SAFETY: callers pass interface pointers they hold.
    let f: extern "system" fn(usize) -> u32 =
        unsafe { core::mem::transmute(method(object, 1) as usize) };
    f(door_windows_abi::narrow(object))
}
fn release(object: u64) -> u32 {
    if object == 0 {
        return 0;
    }
    // SAFETY: as add_ref.
    let f: extern "system" fn(usize) -> u32 =
        unsafe { core::mem::transmute(method(object, 2) as usize) };
    f(door_windows_abi::narrow(object))
}

// ---- BSTR ---------------------------------------------------------------------------
/// A BSTR: a 4-byte byte length before the text, two NUL bytes after it,
/// in a block of the process heap. Extra references (SysAddRefString)
/// postpone the free.
static REFERENCES: Lock<Vec<(u64, u32)>> = Lock::new(Vec::new());
fn bstr_alloc(bytes: usize, data: Option<&[u8]>) -> u64 {
    let Some(total) = bytes.checked_add(4 + 2).filter(|t| *t < u32::MAX as usize) else {
        return 0;
    };
    // 8 zero bytes of tail keep a byte-length string's last unit whole.
    let block = unsafe { RtlAllocateHeap(door_windows_abi::narrow(heap()), 0x8, total + 2) };
    if block == 0 {
        return 0;
    }
    let ok = write_u32(door_windows_abi::widen(block), bytes as u32).is_ok()
        && data.is_none_or(|d| {
            write(door_windows_abi::widen(block + 4), &d[..bytes.min(d.len())]).is_ok()
        });
    if !ok {
        unsafe { RtlFreeHeap(door_windows_abi::narrow(heap()), 0, block) };
        return 0;
    }
    door_windows_abi::widen(block + 4)
}
/// A BSTR's text (NULL is the empty string).
fn bstr_units(bstr: u64) -> Result<Vec<u16>, u32> {
    if bstr == 0 {
        return Ok(Vec::new());
    }
    let bytes = read_u32(bstr - 4)? as usize;
    let mut b = vec![0u8; bytes / 2 * 2];
    read(bstr, &mut b)?;
    Ok(b.chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect())
}
fn bstr_from(units: &[u16]) -> u64 {
    let bytes: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
    bstr_alloc(bytes.len(), Some(&bytes))
}
fn bstr_free(bstr: u64) {
    if bstr == 0 {
        return;
    }
    let held = REFERENCES.with(|r| {
        if let Some(i) = r.iter().position(|(p, _)| *p == bstr) {
            r[i].1 -= 1;
            if r[i].1 == 0 {
                r.remove(i);
            }
            true
        } else {
            false
        }
    });
    if !held {
        unsafe {
            RtlFreeHeap(
                door_windows_abi::narrow(heap()),
                0,
                door_windows_abi::narrow(bstr - 4),
            )
        };
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SysAllocString(text: usize) -> usize {
    let text = door_windows_abi::widen(text);
    door_windows_abi::narrow((move || -> u64 {
        if text == 0 {
            return 0;
        }
        read_wide(text).map_or(0, |u| bstr_from(&u))
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn SysAllocStringLen(text: usize, length: u32) -> usize {
    let text = door_windows_abi::widen(text);
    door_windows_abi::narrow((move || -> u64 {
        let bytes = length as usize * 2;
        if text == 0 {
            return bstr_alloc(bytes, None);
        }
        let mut b = vec![0u8; bytes];
        if read(text, &mut b).is_err() {
            return 0;
        }
        bstr_alloc(bytes, Some(&b))
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn SysAllocStringByteLen(data: usize, length: u32) -> usize {
    let data = door_windows_abi::widen(data);
    door_windows_abi::narrow((move || -> u64 {
        let bytes = length as usize;
        if data == 0 {
            return bstr_alloc(bytes, None);
        }
        let mut b = vec![0u8; bytes];
        if read(data, &mut b).is_err() {
            return 0;
        }
        bstr_alloc(bytes, Some(&b))
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn SysReAllocString(target: usize, text: usize) -> i32 {
    let target = door_windows_abi::widen(target);
    let text = door_windows_abi::widen(text);
    {
        SysReAllocStringLen(
            door_windows_abi::narrow(target),
            door_windows_abi::narrow(text),
            if text == 0 {
                0
            } else {
                read_wide(text).map_or(0, |u| u.len() as u32)
            },
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SysReAllocStringLen(target: usize, text: usize, length: u32) -> i32 {
    let target = door_windows_abi::widen(target);
    let text = door_windows_abi::widen(text);
    {
        if target == 0 {
            return 0;
        }
        let fresh = SysAllocStringLen(door_windows_abi::narrow(text), length);
        if fresh == 0 {
            return 0;
        }
        if let Ok(old) = read_pointer(target) {
            // The new text may come from the old string: read it first.
            if write_pointer(target, door_windows_abi::widen(fresh)).is_err() {
                bstr_free(door_windows_abi::widen(fresh));
                return 0;
            }
            bstr_free(old);
        }
        1
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SysFreeString(bstr: usize) {
    let bstr = door_windows_abi::widen(bstr);
    {
        bstr_free(bstr);
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SysStringLen(bstr: usize) -> u32 {
    let bstr = door_windows_abi::widen(bstr);
    {
        if bstr == 0 {
            return 0;
        }
        read_u32(bstr - 4).map_or(0, |b| b / 2)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SysStringByteLen(bstr: usize) -> u32 {
    let bstr = door_windows_abi::widen(bstr);
    {
        if bstr == 0 {
            return 0;
        }
        read_u32(bstr - 4).unwrap_or(0)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SysAddRefString(bstr: usize) -> u32 {
    let bstr = door_windows_abi::widen(bstr);
    {
        if bstr == 0 {
            return hr::E_INVALIDARG;
        }
        REFERENCES.with(|r| match r.iter_mut().find(|(p, _)| *p == bstr) {
            Some(e) => e.1 += 1,
            None => r.push((bstr, 1)),
        });
        hr::S_OK
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SysReleaseString(bstr: usize) {
    let bstr = door_windows_abi::widen(bstr);
    {
        bstr_free(bstr);
    }
}

// ---- VARIANT ---------------------------------------------------------------------------
/// sizeof(VARIANT): vt, three reserved words, then a union of two
/// pointers (BRECORD) or eight bytes: 24 bytes on x64, 16 on x86.
pub(crate) const VARIANT_BYTES: u64 = 8 + 2 * POINTER as u64;
/// The value a VARIANT at `p` holds (a BYREF one: what it points at).
/// Arrays are not values (SAFEARRAY functions handle them).
fn read_value(p: u64) -> Result<Value, u32> {
    let kind = read_u16(p)?;
    if kind & vt::ARRAY != 0 {
        return Err(hr::DISP_E_TYPEMISMATCH);
    }
    let (base, at) = if kind & vt::BYREF != 0 {
        let target = read_pointer(p + 8)?;
        if kind & vt::TYPEMASK == vt::VARIANT {
            return read_value(target);
        }
        (kind & vt::TYPEMASK, target)
    } else {
        (kind, p + 8)
    };
    if kind & vt::VECTOR != 0 {
        return Err(hr::DISP_E_BADVARTYPE);
    }
    Ok(match base {
        vt::EMPTY => Value::Empty,
        vt::NULL => Value::Null,
        vt::I1 => Value::I1(read_n::<1>(at)?[0] as i8),
        vt::UI1 => Value::UI1(read_n::<1>(at)?[0]),
        vt::I2 => Value::I2(read_u16(at)? as i16),
        vt::UI2 => Value::UI2(read_u16(at)?),
        vt::BOOL => Value::Bool(read_u16(at)? as i16),
        vt::I4 => Value::I4(read_u32(at)? as i32),
        vt::INT => Value::Int(read_u32(at)? as i32),
        vt::UI4 => Value::UI4(read_u32(at)?),
        vt::UINT => Value::UInt(read_u32(at)?),
        vt::ERROR => Value::Error(read_u32(at)?),
        vt::I8 => Value::I8(read_u64(at)? as i64),
        vt::UI8 => Value::UI8(read_u64(at)?),
        vt::R4 => Value::R4(f32::from_bits(read_u32(at)?)),
        vt::R8 => Value::R8(f64::from_bits(read_u64(at)?)),
        vt::DATE => Value::Date(f64::from_bits(read_u64(at)?)),
        vt::CY => Value::Cy(read_u64(at)? as i64),
        vt::BSTR => Value::Bstr(bstr_units(read_pointer(at)?)?),
        vt::UNKNOWN => Value::Unknown(read_pointer(at)?),
        vt::DISPATCH => Value::Dispatch(read_pointer(at)?),
        vt::DECIMAL => {
            // By value the DECIMAL fills the VARIANT (its wReserved is vt).
            let start = if kind & vt::BYREF != 0 { at } else { p };
            let mut b: [u8; 16] = read_n(start)?;
            b[0] = 0;
            b[1] = 0;
            Value::Decimal(Decimal::decode(&b)?)
        }
        _ => return Err(hr::DISP_E_BADVARTYPE),
    })
}
/// Store `v` in the VARIANT at `p` (a new BSTR for text; interface
/// pointers are stored as given).
fn write_value(p: u64, v: &Value) -> Result<(), u32> {
    let mut b = [0u8; VARIANT_BYTES as usize];
    b[..2].copy_from_slice(&v.vt().to_le_bytes());
    let mut put = |bytes: &[u8]| b[8..8 + bytes.len()].copy_from_slice(bytes);
    match v {
        Value::Empty | Value::Null => (),
        Value::I1(x) => put(&x.to_le_bytes()),
        Value::UI1(x) => put(&x.to_le_bytes()),
        Value::I2(x) | Value::Bool(x) => put(&x.to_le_bytes()),
        Value::UI2(x) => put(&x.to_le_bytes()),
        Value::I4(x) | Value::Int(x) => put(&x.to_le_bytes()),
        Value::UI4(x) | Value::UInt(x) | Value::Error(x) => put(&x.to_le_bytes()),
        Value::I8(x) | Value::Cy(x) => put(&x.to_le_bytes()),
        Value::UI8(x) | Value::Unknown(x) | Value::Dispatch(x) => put(&x.to_le_bytes()),
        Value::R4(x) => put(&x.to_le_bytes()),
        Value::R8(x) | Value::Date(x) => put(&x.to_le_bytes()),
        Value::Bstr(units) => {
            let s = bstr_from(units);
            if s == 0 {
                return Err(hr::E_OUTOFMEMORY);
            }
            put(&s.to_le_bytes());
        }
        Value::Decimal(d) => {
            b[..16].copy_from_slice(&d.encode());
            b[..2].copy_from_slice(&vt::DECIMAL.to_le_bytes());
        }
    }
    write(
        p,
        &b[..if matches!(v, Value::Decimal(_)) {
            16
        } else {
            VARIANT_BYTES as usize
        }],
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn VariantInit(p: usize) {
    let p = door_windows_abi::widen(p);
    {
        let _ = write_u16(p, vt::EMPTY);
    }
}
/// Free what a VARIANT owns (not a BYREF one's referent).
fn clear(p: u64) -> Result<(), u32> {
    let kind = read_u16(p)?;
    // A VARIANT never holds a vector or the reserved bit; its base type must
    // be one VARIANTs can hold (Windows leaves an invalid one untouched).
    let base = kind & vt::TYPEMASK;
    let valid_base = matches!(base, 0..=14 | 16..=25 | vt::RECORD | vt::INT_PTR | vt::UINT_PTR);
    if kind & (vt::VECTOR | 0x8000) != 0 || !valid_base {
        return Err(hr::DISP_E_BADVARTYPE);
    }
    if kind & vt::BYREF == 0 {
        if kind & vt::ARRAY != 0 {
            let sa = read_pointer(p + 8)?;
            if sa != 0 {
                let r = safearray::SafeArrayDestroy(door_windows_abi::narrow(sa));
                if r != hr::S_OK {
                    return Err(r);
                }
            }
        } else {
            match kind {
                vt::BSTR => bstr_free(read_pointer(p + 8)?),
                vt::UNKNOWN | vt::DISPATCH => {
                    release(read_pointer(p + 8)?);
                }
                vt::RECORD => {
                    // BRECORD: pvRecord, pRecInfo; IRecordInfo::RecordClear (14).
                    let (record, info) =
                        (read_pointer(p + 8)?, read_pointer(p + 8 + POINTER as u64)?);
                    if info != 0 {
                        if record != 0 {
                            // SAFETY: a record VARIANT holds a live IRecordInfo.
                            let f: extern "system" fn(usize, usize) -> u32 =
                                unsafe { core::mem::transmute(method(info, 4) as usize) };
                            f(
                                door_windows_abi::narrow(info),
                                door_windows_abi::narrow(record),
                            );
                        }
                        release(info);
                    }
                }
                vt::EMPTY
                | vt::NULL
                | vt::I1
                | vt::UI1
                | vt::I2
                | vt::UI2
                | vt::BOOL
                | vt::I4
                | vt::INT
                | vt::UI4
                | vt::UINT
                | vt::ERROR
                | vt::I8
                | vt::UI8
                | vt::R4
                | vt::R8
                | vt::DATE
                | vt::CY
                | vt::DECIMAL
                | vt::INT_PTR
                | vt::UINT_PTR
                | vt::VOID
                | vt::HRESULT => (),
                _ => return Err(hr::DISP_E_BADVARTYPE),
            }
        }
    }
    write_u16(p, vt::EMPTY)
}
#[unsafe(no_mangle)]
pub extern "system" fn VariantClear(p: usize) -> u32 {
    let p = door_windows_abi::widen(p);
    result(clear(p))
}
/// VariantCopy's deep copy of `src` into `dst` (cleared first).
fn copy(dst: u64, src: u64, indirect: bool) -> Result<(), u32> {
    let kind = read_u16(src)?;
    let mut raw = [0u8; 24];
    read(src, &mut raw[..VARIANT_BYTES as usize])?;
    let raw = &mut raw[..VARIANT_BYTES as usize];
    if dst != src {
        clear(dst)?;
    }
    if kind & vt::BYREF != 0 && indirect {
        // VariantCopyInd: the referent by value.
        if kind & vt::ARRAY != 0 {
            let sa = read_pointer(read_pointer(src + 8)?)?;
            let copy = safearray::copy_array(sa)?;
            raw[..2].copy_from_slice(&(kind & !vt::BYREF).to_le_bytes());
            raw[8..8 + POINTER].copy_from_slice(&copy.to_le_bytes()[..POINTER]);
            return write(dst, raw);
        }
        let v = read_value(src)?;
        return write_value(dst, &v);
    }
    if kind & vt::BYREF == 0 {
        if kind & vt::ARRAY != 0 {
            let sa = read_pointer(src + 8)?;
            let copy = if sa == 0 {
                0
            } else {
                safearray::copy_array(sa)?
            };
            raw[8..8 + POINTER].copy_from_slice(&copy.to_le_bytes()[..POINTER]);
        } else {
            match kind {
                vt::BSTR => {
                    let s = read_pointer(src + 8)?;
                    let copy = if s == 0 {
                        0
                    } else {
                        bstr_from(&bstr_units(s)?)
                    };
                    raw[8..8 + POINTER].copy_from_slice(&copy.to_le_bytes()[..POINTER]);
                }
                vt::UNKNOWN | vt::DISPATCH => {
                    add_ref(read_pointer(src + 8)?);
                }
                vt::RECORD => return Err(hr::E_NOTIMPL),
                _ => (),
            }
        }
    }
    write(dst, raw)
}
#[unsafe(no_mangle)]
pub extern "system" fn VariantCopy(dst: usize, src: usize) -> u32 {
    let dst = door_windows_abi::widen(dst);
    let src = door_windows_abi::widen(src);
    {
        if dst == src {
            return hr::S_OK;
        }
        result(copy(dst, src, false))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn VariantCopyInd(dst: usize, src: usize) -> u32 {
    let dst = door_windows_abi::widen(dst);
    let src = door_windows_abi::widen(src);
    result(copy(dst, src, true))
}
/// DISPID_VALUE of an IDispatch (its default property), for conversions
/// of a VT_DISPATCH: IDispatch::Invoke (method 6) with DISPATCH_PROPERTYGET.
fn default_property(dispatch: u64) -> Result<Value, u32> {
    if dispatch == 0 {
        return Err(hr::DISP_E_TYPEMISMATCH);
    }
    let mut result = [0u8; 24];
    let params = [0u8; 24]; // DISPPARAMS: no arguments.
    let iid_null = [0u8; 16];
    // SAFETY: a VT_DISPATCH holds a live IDispatch.
    let f: extern "system" fn(usize, i32, usize, u32, u16, usize, usize, usize, usize) -> u32 =
        unsafe { core::mem::transmute(method(dispatch, 6) as usize) };
    let r = f(
        door_windows_abi::narrow(dispatch),
        0,
        door_windows_abi::narrow(iid_null.as_ptr() as u64),
        0x409,
        2,
        door_windows_abi::narrow(params.as_ptr() as u64),
        door_windows_abi::narrow(result.as_mut_ptr() as u64),
        0,
        0,
    );
    if r != hr::S_OK {
        return Err(r);
    }
    let p = result.as_mut_ptr() as u64;
    let v = read_value(p);
    let _ = clear(p);
    v
}
fn change(dst: u64, src: u64, flags: u16, target: u16) -> Result<(), u32> {
    if target & (vt::BYREF | vt::VECTOR) != 0 {
        return Err(hr::DISP_E_BADVARTYPE);
    }
    // A VARIANT cannot become VT_VARIANT (Windows: a type mismatch).
    if target & vt::TYPEMASK == vt::VARIANT {
        return Err(hr::DISP_E_TYPEMISMATCH);
    }
    let kind = read_u16(src)?;
    if target & vt::ARRAY != 0 || kind & vt::ARRAY != 0 {
        // Arrays convert only to themselves.
        if kind & !vt::BYREF != target {
            return Err(hr::DISP_E_TYPEMISMATCH);
        }
        return copy(dst, src, true);
    }
    let mut v = read_value(src)?;
    if let Value::Dispatch(d) = v
        && !matches!(target, vt::DISPATCH | vt::UNKNOWN | vt::EMPTY | vt::NULL)
    {
        if flags & variant::flags::NOVALUEPROP != 0 {
            return Err(hr::DISP_E_TYPEMISMATCH);
        }
        v = default_property(d)?;
    }
    let out = variant::change_type(&v, target, flags)?;
    if let Value::Unknown(p) | Value::Dispatch(p) = out {
        add_ref(p);
    }
    clear(dst)?;
    write_value(dst, &out)
}
#[unsafe(no_mangle)]
pub extern "system" fn VariantChangeType(dst: usize, src: usize, flags: u16, target: u16) -> u32 {
    let dst = door_windows_abi::widen(dst);
    let src = door_windows_abi::widen(src);
    result(change(dst, src, flags, target))
}
/// The locale only matters for text; Doors formats and reads en-US.
#[unsafe(no_mangle)]
pub extern "system" fn VariantChangeTypeEx(
    dst: usize,
    src: usize,
    _lcid: u32,
    flags: u16,
    target: u16,
) -> u32 {
    let dst = door_windows_abi::widen(dst);
    let src = door_windows_abi::widen(src);
    result(change(dst, src, flags, target))
}

// ---- Var<T>From<S> -------------------------------------------------------------------
/// Write a converted scalar of type `target` at `out`.
fn store(out: u64, v: &Value) -> Result<(), u32> {
    match v {
        Value::I1(x) => write(out, &x.to_le_bytes()),
        Value::UI1(x) => write(out, &x.to_le_bytes()),
        Value::I2(x) | Value::Bool(x) => write(out, &x.to_le_bytes()),
        Value::UI2(x) => write(out, &x.to_le_bytes()),
        Value::I4(x) | Value::Int(x) => write(out, &x.to_le_bytes()),
        Value::UI4(x) | Value::UInt(x) | Value::Error(x) => write(out, &x.to_le_bytes()),
        Value::I8(x) | Value::Cy(x) => write(out, &x.to_le_bytes()),
        Value::UI8(x) => write(out, &x.to_le_bytes()),
        Value::R4(x) => write(out, &x.to_le_bytes()),
        Value::R8(x) | Value::Date(x) => write(out, &x.to_le_bytes()),
        Value::Decimal(d) => write(out, &d.encode()),
        Value::Bstr(units) => {
            let s = bstr_from(units);
            if s == 0 {
                return Err(hr::E_OUTOFMEMORY);
            }
            write_pointer(out, s)
        }
        _ => Err(hr::DISP_E_TYPEMISMATCH),
    }
}
fn convert(v: Result<Value, u32>, target: u16, flags: u16, out: u64) -> u32 {
    if out == 0 {
        return hr::E_INVALIDARG;
    }
    result(
        v.and_then(|v| variant::change_type(&v, target, flags))
            .and_then(|r| store(out, &r)),
    )
}
fn text_value(text: u64) -> Result<Value, u32> {
    read_wide(text).map(Value::Bstr)
}
fn dec_value(p: u64) -> Result<Value, u32> {
    Decimal::decode(&read_n(p)?).map(Value::Decimal)
}
fn disp_value(p: u64) -> Result<Value, u32> {
    default_property(p)
}
/// Conversions from a scalar argument: `Var<T>From<S>(in, out)`.
macro_rules! from_scalar {
    ($ty:ty, $make:expr; $($name:ident => $target:ident),* $(,)?) => {$(
        #[unsafe(no_mangle)]
        pub extern "system" fn $name(value: $ty, out: usize) -> u32 {
            let out = door_windows_abi::widen(out);
            {
            convert(Ok(($make)(value)), vt::$target, 0, out)
        }
        }
    )*};
}
/// Conversions to BSTR from a scalar: `VarBstrFrom<S>(in, lcid, flags, out)`.
macro_rules! to_text {
    ($($name:ident: $ty:ty => $make:expr),* $(,)?) => {$(
        #[unsafe(no_mangle)]
        pub extern "system" fn $name(value: $ty, _lcid: u32, flags: u32, out: usize) -> u32 {
            let out = door_windows_abi::widen(out);
            {
            convert(Ok(($make)(value)), vt::BSTR, flags as u16, out)
        }
        }
    )*};
}
/// Conversions from text: `Var<T>FromStr(text, lcid, flags, out)`.
macro_rules! from_text {
    ($($name:ident => $target:ident),* $(,)?) => {$(
        #[unsafe(no_mangle)]
        pub extern "system" fn $name(text: usize, _lcid: u32, flags: u32, out: usize) -> u32 {
            let text = door_windows_abi::widen(text);
            let out = door_windows_abi::widen(out);
            {
            convert(text_value(text), vt::$target, flags as u16, out)
        }
        }
    )*};
}
/// Conversions from a DECIMAL*: `Var<T>FromDec(in, out)`.
macro_rules! from_decimal {
    ($($name:ident => $target:ident),* $(,)?) => {$(
        #[unsafe(no_mangle)]
        pub extern "system" fn $name(value: usize, out: usize) -> u32 {
            let value = door_windows_abi::widen(value);
            let out = door_windows_abi::widen(out);
            {
            convert(dec_value(value), vt::$target, 0, out)
        }
        }
    )*};
}
/// Conversions from an IDispatch's default property:
/// `Var<T>FromDisp(dispatch, lcid, out)`.
macro_rules! from_dispatch {
    ($($name:ident => $target:ident),* $(,)?) => {$(
        #[unsafe(no_mangle)]
        pub extern "system" fn $name(value: usize, _lcid: u32, out: usize) -> u32 {
            let value = door_windows_abi::widen(value);
            let out = door_windows_abi::widen(out);
            {
            convert(disp_value(value), vt::$target, 0, out)
        }
        }
    )*};
}
from_scalar!(u8, Value::UI1;
    VarI1FromUI1 => I1, VarI2FromUI1 => I2, VarI4FromUI1 => I4, VarI8FromUI1 => I8,
    VarUI2FromUI1 => UI2, VarUI4FromUI1 => UI4, VarUI8FromUI1 => UI8, VarR4FromUI1 => R4,
    VarR8FromUI1 => R8, VarCyFromUI1 => CY, VarDateFromUI1 => DATE, VarBoolFromUI1 => BOOL,
    VarDecFromUI1 => DECIMAL);
from_scalar!(i16, Value::I2;
    VarI1FromI2 => I1, VarUI1FromI2 => UI1, VarI4FromI2 => I4, VarI8FromI2 => I8,
    VarUI2FromI2 => UI2, VarUI4FromI2 => UI4, VarUI8FromI2 => UI8, VarR4FromI2 => R4,
    VarR8FromI2 => R8, VarCyFromI2 => CY, VarDateFromI2 => DATE, VarBoolFromI2 => BOOL,
    VarDecFromI2 => DECIMAL);
from_scalar!(i32, Value::I4;
    VarI1FromI4 => I1, VarUI1FromI4 => UI1, VarI2FromI4 => I2, VarI8FromI4 => I8,
    VarUI2FromI4 => UI2, VarUI4FromI4 => UI4, VarUI8FromI4 => UI8, VarR4FromI4 => R4,
    VarR8FromI4 => R8, VarCyFromI4 => CY, VarDateFromI4 => DATE, VarBoolFromI4 => BOOL,
    VarDecFromI4 => DECIMAL);
from_scalar!(i64, Value::I8;
    VarI1FromI8 => I1, VarUI1FromI8 => UI1, VarI2FromI8 => I2, VarI4FromI8 => I4,
    VarUI2FromI8 => UI2, VarUI4FromI8 => UI4, VarUI8FromI8 => UI8, VarR4FromI8 => R4,
    VarR8FromI8 => R8, VarCyFromI8 => CY, VarDateFromI8 => DATE, VarBoolFromI8 => BOOL,
    VarDecFromI8 => DECIMAL);
from_scalar!(f32, Value::R4;
    VarI1FromR4 => I1, VarUI1FromR4 => UI1, VarI2FromR4 => I2, VarI4FromR4 => I4,
    VarI8FromR4 => I8, VarUI2FromR4 => UI2, VarUI4FromR4 => UI4, VarUI8FromR4 => UI8,
    VarR8FromR4 => R8, VarCyFromR4 => CY, VarDateFromR4 => DATE, VarBoolFromR4 => BOOL,
    VarDecFromR4 => DECIMAL);
from_scalar!(f64, Value::R8;
    VarI1FromR8 => I1, VarUI1FromR8 => UI1, VarI2FromR8 => I2, VarI4FromR8 => I4,
    VarI8FromR8 => I8, VarUI2FromR8 => UI2, VarUI4FromR8 => UI4, VarUI8FromR8 => UI8,
    VarR4FromR8 => R4, VarCyFromR8 => CY, VarDateFromR8 => DATE, VarBoolFromR8 => BOOL,
    VarDecFromR8 => DECIMAL);
from_scalar!(i64, Value::Cy;
    VarI1FromCy => I1, VarUI1FromCy => UI1, VarI2FromCy => I2, VarI4FromCy => I4,
    VarI8FromCy => I8, VarUI2FromCy => UI2, VarUI4FromCy => UI4, VarUI8FromCy => UI8,
    VarR4FromCy => R4, VarR8FromCy => R8, VarDateFromCy => DATE, VarBoolFromCy => BOOL,
    VarDecFromCy => DECIMAL);
from_scalar!(f64, Value::Date;
    VarI1FromDate => I1, VarUI1FromDate => UI1, VarI2FromDate => I2, VarI4FromDate => I4,
    VarI8FromDate => I8, VarUI2FromDate => UI2, VarUI4FromDate => UI4, VarUI8FromDate => UI8,
    VarR4FromDate => R4, VarR8FromDate => R8, VarCyFromDate => CY, VarBoolFromDate => BOOL,
    VarDecFromDate => DECIMAL);
from_scalar!(i16, Value::Bool;
    VarI1FromBool => I1, VarUI1FromBool => UI1, VarI2FromBool => I2, VarI4FromBool => I4,
    VarI8FromBool => I8, VarUI2FromBool => UI2, VarUI4FromBool => UI4, VarUI8FromBool => UI8,
    VarR4FromBool => R4, VarR8FromBool => R8, VarCyFromBool => CY, VarDateFromBool => DATE,
    VarDecFromBool => DECIMAL);
from_scalar!(i8, Value::I1;
    VarUI1FromI1 => UI1, VarI2FromI1 => I2, VarI4FromI1 => I4, VarI8FromI1 => I8,
    VarUI2FromI1 => UI2, VarUI4FromI1 => UI4, VarUI8FromI1 => UI8, VarR4FromI1 => R4,
    VarR8FromI1 => R8, VarCyFromI1 => CY, VarDateFromI1 => DATE, VarBoolFromI1 => BOOL,
    VarDecFromI1 => DECIMAL);
from_scalar!(u16, Value::UI2;
    VarI1FromUI2 => I1, VarUI1FromUI2 => UI1, VarI2FromUI2 => I2, VarI4FromUI2 => I4,
    VarI8FromUI2 => I8, VarUI4FromUI2 => UI4, VarUI8FromUI2 => UI8, VarR4FromUI2 => R4,
    VarR8FromUI2 => R8, VarCyFromUI2 => CY, VarDateFromUI2 => DATE, VarBoolFromUI2 => BOOL,
    VarDecFromUI2 => DECIMAL);
from_scalar!(u32, Value::UI4;
    VarI1FromUI4 => I1, VarUI1FromUI4 => UI1, VarI2FromUI4 => I2, VarI4FromUI4 => I4,
    VarI8FromUI4 => I8, VarUI2FromUI4 => UI2, VarUI8FromUI4 => UI8, VarR4FromUI4 => R4,
    VarR8FromUI4 => R8, VarCyFromUI4 => CY, VarDateFromUI4 => DATE, VarBoolFromUI4 => BOOL,
    VarDecFromUI4 => DECIMAL);
from_scalar!(u64, Value::UI8;
    VarI1FromUI8 => I1, VarUI1FromUI8 => UI1, VarI2FromUI8 => I2, VarI4FromUI8 => I4,
    VarI8FromUI8 => I8, VarUI2FromUI8 => UI2, VarUI4FromUI8 => UI4, VarR4FromUI8 => R4,
    VarR8FromUI8 => R8, VarCyFromUI8 => CY, VarDateFromUI8 => DATE, VarBoolFromUI8 => BOOL,
    VarDecFromUI8 => DECIMAL);
to_text!(
    VarBstrFromUI1: u8 => Value::UI1, VarBstrFromI2: i16 => Value::I2,
    VarBstrFromI4: i32 => Value::I4, VarBstrFromI8: i64 => Value::I8,
    VarBstrFromR4: f32 => Value::R4, VarBstrFromR8: f64 => Value::R8,
    VarBstrFromCy: i64 => Value::Cy, VarBstrFromDate: f64 => Value::Date,
    VarBstrFromBool: i16 => Value::Bool, VarBstrFromI1: i8 => Value::I1,
    VarBstrFromUI2: u16 => Value::UI2, VarBstrFromUI4: u32 => Value::UI4,
    VarBstrFromUI8: u64 => Value::UI8,
);
from_text!(
    VarUI1FromStr => UI1, VarI2FromStr => I2, VarI4FromStr => I4, VarI8FromStr => I8,
    VarR4FromStr => R4, VarR8FromStr => R8, VarCyFromStr => CY, VarDateFromStr => DATE,
    VarBoolFromStr => BOOL, VarI1FromStr => I1, VarUI2FromStr => UI2, VarUI4FromStr => UI4,
    VarUI8FromStr => UI8, VarDecFromStr => DECIMAL,
);
from_decimal!(
    VarUI1FromDec => UI1, VarI2FromDec => I2, VarI4FromDec => I4, VarI8FromDec => I8,
    VarR4FromDec => R4, VarR8FromDec => R8, VarCyFromDec => CY, VarDateFromDec => DATE,
    VarBoolFromDec => BOOL, VarI1FromDec => I1, VarUI2FromDec => UI2, VarUI4FromDec => UI4,
    VarUI8FromDec => UI8,
);
from_dispatch!(
    VarUI1FromDisp => UI1, VarI2FromDisp => I2, VarI4FromDisp => I4, VarI8FromDisp => I8,
    VarR4FromDisp => R4, VarR8FromDisp => R8, VarCyFromDisp => CY, VarDateFromDisp => DATE,
    VarBoolFromDisp => BOOL, VarI1FromDisp => I1, VarUI2FromDisp => UI2,
    VarUI4FromDisp => UI4, VarUI8FromDisp => UI8, VarDecFromDisp => DECIMAL,
);
#[unsafe(no_mangle)]
pub extern "system" fn VarBstrFromDec(value: usize, _lcid: u32, flags: u32, out: usize) -> u32 {
    let value = door_windows_abi::widen(value);
    let out = door_windows_abi::widen(out);
    convert(dec_value(value), vt::BSTR, flags as u16, out)
}
#[unsafe(no_mangle)]
pub extern "system" fn VarBstrFromDisp(value: usize, _lcid: u32, flags: u32, out: usize) -> u32 {
    let value = door_windows_abi::widen(value);
    let out = door_windows_abi::widen(out);
    convert(disp_value(value), vt::BSTR, flags as u16, out)
}

// ---- arithmetic ----------------------------------------------------------------------
fn binary(
    left: u64,
    right: u64,
    out: u64,
    f: impl FnOnce(&Value, &Value) -> Result<Value, u32>,
) -> u32 {
    let r = (|| {
        let (a, b) = (read_value(left)?, read_value(right)?);
        let v = f(&a, &b)?;
        write_value(out, &v)
    })();
    result(r)
}
#[unsafe(no_mangle)]
pub extern "system" fn VarAdd(left: usize, right: usize, out: usize) -> u32 {
    let left = door_windows_abi::widen(left);
    let right = door_windows_abi::widen(right);
    let out = door_windows_abi::widen(out);
    {
        // Two strings concatenate.
        if let (Ok(vt::BSTR), Ok(vt::BSTR)) = (read_u16(left), read_u16(right)) {
            return binary(left, right, out, variant::concatenate);
        }
        binary(left, right, out, |a, b| variant::arith(a, b, '+'))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn VarSub(left: usize, right: usize, out: usize) -> u32 {
    let left = door_windows_abi::widen(left);
    let right = door_windows_abi::widen(right);
    let out = door_windows_abi::widen(out);
    binary(left, right, out, |a, b| variant::arith(a, b, '-'))
}
#[unsafe(no_mangle)]
pub extern "system" fn VarMul(left: usize, right: usize, out: usize) -> u32 {
    let left = door_windows_abi::widen(left);
    let right = door_windows_abi::widen(right);
    let out = door_windows_abi::widen(out);
    binary(left, right, out, |a, b| variant::arith(a, b, '*'))
}
#[unsafe(no_mangle)]
pub extern "system" fn VarDiv(left: usize, right: usize, out: usize) -> u32 {
    let left = door_windows_abi::widen(left);
    let right = door_windows_abi::widen(right);
    let out = door_windows_abi::widen(out);
    binary(left, right, out, variant::divide)
}
#[unsafe(no_mangle)]
pub extern "system" fn VarIdiv(left: usize, right: usize, out: usize) -> u32 {
    let left = door_windows_abi::widen(left);
    let right = door_windows_abi::widen(right);
    let out = door_windows_abi::widen(out);
    {
        binary(left, right, out, |a, b| {
            variant::integer_divide(a, b, false)
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn VarMod(left: usize, right: usize, out: usize) -> u32 {
    let left = door_windows_abi::widen(left);
    let right = door_windows_abi::widen(right);
    let out = door_windows_abi::widen(out);
    binary(left, right, out, |a, b| variant::integer_divide(a, b, true))
}
#[unsafe(no_mangle)]
pub extern "system" fn VarCat(left: usize, right: usize, out: usize) -> u32 {
    let left = door_windows_abi::widen(left);
    let right = door_windows_abi::widen(right);
    let out = door_windows_abi::widen(out);
    binary(left, right, out, variant::concatenate)
}
#[unsafe(no_mangle)]
pub extern "system" fn VarCmp(left: usize, right: usize, _lcid: u32, _flags: u32) -> u32 {
    let left = door_windows_abi::widen(left);
    let right = door_windows_abi::widen(right);
    {
        match read_value(left)
            .and_then(|a| read_value(right).and_then(|b| variant::compare(&a, &b)))
        {
            Ok(c) => c,
            Err(e) => e,
        }
    }
}
#[link(name = "kernel32", kind = "dylib")]
unsafe extern "system" {
    fn CompareStringW(lcid: u32, flags: u32, a: usize, a_len: i32, b: usize, b_len: i32) -> i32;
}
/// VarBstrCmp: VARCMP_LT 0, VARCMP_EQ 1, VARCMP_GT 2. A NULL or empty
/// BSTR sorts first; LCID 0 compares the bytes, any other locale
/// compares as CompareStringW does with `flags`.
#[unsafe(no_mangle)]
pub extern "system" fn VarBstrCmp(left: usize, right: usize, lcid: u32, flags: u32) -> u32 {
    let (Ok(a), Ok(b)) = (
        bstr_units(door_windows_abi::widen(left)),
        bstr_units(door_windows_abi::widen(right)),
    ) else {
        return hr::E_POINTER;
    };
    let empty = |s: &[u16]| s.first().is_none_or(|u| *u == 0);
    match (empty(&a), empty(&b)) {
        (true, true) => return 1,
        (true, false) => return 0,
        (false, true) => return 2,
        _ => {}
    }
    if lcid == 0 {
        let bytes = |s: &[u16]| -> Vec<u8> { s.iter().flat_map(|u| u.to_le_bytes()).collect() };
        return match bytes(&a).cmp(&bytes(&b)) {
            core::cmp::Ordering::Less => 0,
            core::cmp::Ordering::Equal => 1,
            core::cmp::Ordering::Greater => 2,
        };
    }
    let r = unsafe {
        CompareStringW(lcid, flags, a.as_ptr() as usize, a.len() as i32, b.as_ptr() as usize, b.len() as i32)
    };
    if r == 0 { hr::E_INVALIDARG } else { (r - 1) as u32 }
}
fn unary(input: u64, out: u64, f: impl FnOnce(&Value) -> Result<Value, u32>) -> u32 {
    result(
        read_value(input)
            .and_then(|v| f(&v))
            .and_then(|v| write_value(out, &v)),
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn VarNeg(input: usize, out: usize) -> u32 {
    let input = door_windows_abi::widen(input);
    let out = door_windows_abi::widen(out);
    unary(input, out, |v| variant::arith(&Value::I2(0), v, '-'))
}
#[unsafe(no_mangle)]
pub extern "system" fn VarAbs(input: usize, out: usize) -> u32 {
    let input = door_windows_abi::widen(input);
    let out = door_windows_abi::widen(out);
    {
        unary(input, out, |v| match variant::compare(v, &Value::I2(0))? {
            0 => variant::arith(&Value::I2(0), v, '-'),
            _ => Ok(v.clone()),
        })
    }
}

// ---- dates ---------------------------------------------------------------------------
/// SYSTEMTIME: year, month, day of week, day, hour, minute, second, ms.
fn read_systemtime(p: u64) -> Result<[u16; 8], u32> {
    let b: [u8; 16] = read_n(p)?;
    Ok(core::array::from_fn(|i| {
        u16::from_le_bytes([b[2 * i], b[2 * i + 1]])
    }))
}
fn write_systemtime(p: u64, t: [u16; 8]) -> Result<(), u32> {
    let mut b = [0u8; 16];
    for (i, v) in t.iter().enumerate() {
        b[2 * i..2 * i + 2].copy_from_slice(&v.to_le_bytes());
    }
    write(p, &b)
}
/// A date from parts, Windows' way: out-of-range months and days are
/// refused, the milliseconds ignored.
fn date_of(y: u16, m: u16, d: u16, h: u16, min: u16, s: u16) -> Option<f64> {
    let year = i64::from(y);
    if !(100..=9999).contains(&year) || !(1..=12).contains(&m) || h > 23 || min > 59 || s > 59 {
        return None;
    }
    if d == 0 || u32::from(d) > date::month_days(year, u32::from(m)) {
        return None;
    }
    Some(date::from_parts(
        year,
        u32::from(m),
        u32::from(d),
        u32::from(h),
        u32::from(min),
        u32::from(s),
    ))
}
#[unsafe(no_mangle)]
pub extern "system" fn SystemTimeToVariantTime(time: usize, out: usize) -> i32 {
    let time = door_windows_abi::widen(time);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            let t = read_systemtime(time)?;
            // Windows takes any day up to 31 and lets it run on into the next
            // month (February 30 is March 2).
            date_of(t[0], t[1], 1, t[4], t[5], t[6]).ok_or(hr::E_INVALIDARG)?;
            if !(1..=31).contains(&t[3]) {
                return Err(hr::E_INVALIDARG);
            }
            let day =
                (date::days(i64::from(t[0]), u32::from(t[1]), 1) + i64::from(t[3]) - 1) as f64;
            let time = f64::from(u32::from(t[4]) * 3600 + u32::from(t[5]) * 60 + u32::from(t[6]))
                / 86_400.0;
            let v = if day < 0.0 { day - time } else { day + time };
            write(out, &v.to_le_bytes())
        })();
        i32::from(r.is_ok())
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn VariantTimeToSystemTime(value: f64, out: usize) -> i32 {
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            let p = date::split(value)?;
            write_systemtime(
                out,
                [
                    p.year as u16,
                    p.month as u16,
                    p.day_of_week as u16,
                    p.day as u16,
                    p.hour as u16,
                    p.minute as u16,
                    p.second as u16,
                    0,
                ],
            )
        })();
        i32::from(r.is_ok())
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DosDateTimeToVariantTime(dos_date: u16, dos_time: u16, out: usize) -> i32 {
    let out = door_windows_abi::widen(out);
    {
        let y = 1980 + (dos_date >> 9);
        let m = (dos_date >> 5) & 15;
        let d = dos_date & 31;
        let h = dos_time >> 11;
        let min = (dos_time >> 5) & 63;
        let s = (dos_time & 31) * 2;
        match date_of(y, m, d, h, min, s) {
            Some(v) => i32::from(write(out, &v.to_le_bytes()).is_ok()),
            None => 0,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn VariantTimeToDosDateTime(
    value: f64,
    dos_date: usize,
    dos_time: usize,
) -> i32 {
    let dos_date = door_windows_abi::widen(dos_date);
    let dos_time = door_windows_abi::widen(dos_time);
    {
        let r = (|| {
            let p = date::split(value)?;
            if !(1980..=2107).contains(&p.year) {
                return Err(hr::E_INVALIDARG);
            }
            let d = (((p.year - 1980) as u16) << 9) | ((p.month as u16) << 5) | p.day as u16;
            let t = ((p.hour as u16) << 11) | ((p.minute as u16) << 5) | (p.second as u16 / 2);
            write_u16(dos_date, d)?;
            write_u16(dos_time, t)
        })();
        i32::from(r.is_ok())
    }
}
/// UDATE: a SYSTEMTIME and the day of the year.
#[unsafe(no_mangle)]
pub extern "system" fn VarUdateFromDate(value: f64, _flags: u32, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            let p = date::split(value)?;
            write_systemtime(
                out,
                [
                    p.year as u16,
                    p.month as u16,
                    p.day_of_week as u16,
                    p.day as u16,
                    p.hour as u16,
                    p.minute as u16,
                    p.second as u16,
                    0,
                ],
            )?;
            let first = date::days(p.year, 1, 1);
            let doy = date::days(p.year, p.month, p.day) - first + 1;
            write_u16(out + 16, doy as u16)
        })();
        result(r)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn VarDateFromUdate(udate: usize, _flags: u32, out: usize) -> u32 {
    let udate = door_windows_abi::widen(udate);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            let t = read_systemtime(udate)?;
            let v = date_of(t[0], t[1], t[3], t[4], t[5], t[6]).ok_or(hr::E_INVALIDARG)?;
            write(out, &v.to_le_bytes())
        })();
        result(r)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn VarDateFromUdateEx(udate: usize, _lcid: u32, flags: u32, out: usize) -> u32 {
    let udate = door_windows_abi::widen(udate);
    let out = door_windows_abi::widen(out);
    {
        VarDateFromUdate(
            door_windows_abi::narrow(udate),
            flags,
            door_windows_abi::narrow(out),
        )
    }
}
