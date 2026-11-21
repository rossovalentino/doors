//! SAFEARRAY (oaidl.h), laid out as Windows lays it: cDims u16, fFeatures
//! u16, cbElements u32, cLocks u32, pvData (@16 on x64, @12 on x86), then
//! the bounds (cElements u32, lLbound i32) with the first dimension last.
//! 16 bytes before the descriptor hold its IID (FADF_HAVEIID),
//! IRecordInfo* (in the last pointer-sized slot, FADF_RECORD) or VARTYPE
//! (at -4, FADF_HAVEVARTYPE). rgIndices[0] is the first dimension and
//! varies fastest. Measured on Windows 11.
use super::*;

/// pvData's offset and the descriptor's fixed part (P17).
const DATA_AT: u64 = if POINTER == 8 { 16 } else { 12 };
const HEAD: u64 = DATA_AT + POINTER as u64;
/// FADF_RECORD's IRecordInfo*: the pointer-sized slot before the
/// descriptor.
const RECORD_AT: u64 = POINTER as u64;

pub(super) mod feature {
    pub const AUTO: u16 = 0x1;
    pub const STATIC: u16 = 0x2;
    pub const EMBEDDED: u16 = 0x4;
    pub const RECORD: u16 = 0x20;
    pub const HAVEIID: u16 = 0x40;
    pub const HAVEVARTYPE: u16 = 0x80;
    pub const BSTR: u16 = 0x100;
    pub const UNKNOWN: u16 = 0x200;
    pub const DISPATCH: u16 = 0x400;
    pub const VARIANT: u16 = 0x800;
    pub const CREATEVECTOR: u16 = 0x2000;
}
const IID_IUNKNOWN: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
const IID_IDISPATCH: [u8; 16] = [0, 0x04, 0x02, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];

/// The element size of an array of `kind` (0: not an array type).
fn element_size(kind: u16) -> u32 {
    match kind {
        vt::I1 | vt::UI1 => 1,
        vt::I2 | vt::UI2 | vt::BOOL => 2,
        vt::I4 | vt::UI4 | vt::INT | vt::UINT | vt::R4 | vt::ERROR => 4,
        vt::I8 | vt::UI8 | vt::R8 | vt::DATE | vt::CY => 8,
        vt::BSTR | vt::UNKNOWN | vt::DISPATCH | vt::INT_PTR | vt::UINT_PTR => POINTER as u32,
        vt::VARIANT => VARIANT_BYTES as u32,
        vt::DECIMAL => 16,
        _ => 0,
    }
}
fn kind_feature(kind: u16) -> u16 {
    match kind {
        vt::BSTR => feature::BSTR,
        vt::UNKNOWN => feature::UNKNOWN,
        vt::DISPATCH => feature::DISPATCH,
        vt::VARIANT => feature::VARIANT,
        _ => 0,
    }
}
struct Header {
    dims: u16,
    features: u16,
    element: u32,
    locks: u32,
    data: u64,
    /// (cElements, lLbound) in storage order (the last dimension first).
    bounds: Vec<(u32, i32)>,
}
fn header(sa: u64) -> Result<Header, u32> {
    if sa == 0 {
        return Err(hr::E_INVALIDARG);
    }
    let mut head = [0u8; 24];
    read(sa, &mut head[..HEAD as usize])?;
    let dims = u16::from_le_bytes([head[0], head[1]]);
    let mut bounds = Vec::with_capacity(usize::from(dims));
    for i in 0..u64::from(dims) {
        let b: [u8; 8] = read_n(sa + HEAD + 8 * i)?;
        bounds.push((
            u32::from_le_bytes(b[..4].try_into().unwrap()),
            i32::from_le_bytes(b[4..].try_into().unwrap()),
        ));
    }
    Ok(Header {
        dims,
        features: u16::from_le_bytes([head[2], head[3]]),
        element: u32::from_le_bytes(head[4..8].try_into().unwrap()),
        locks: u32::from_le_bytes(head[8..12].try_into().unwrap()),
        data: door_windows_abi::layout::get(
            &head[DATA_AT as usize..],
            &[door_windows_abi::layout::Field::Pointer],
            0,
        ),
        bounds,
    })
}
impl Header {
    fn cells(&self) -> u64 {
        self.bounds.iter().map(|(n, _)| u64::from(*n)).product()
    }
    fn bytes(&self) -> u64 {
        self.cells() * u64::from(self.element)
    }
}
fn descriptor_bytes(dims: u32) -> usize {
    HEAD as usize + 8 * dims as usize
}
/// A new descriptor (zeroed) after its 16-byte prefix; its address.
fn new_descriptor(dims: u32, extra: usize) -> u64 {
    let block = unsafe {
        RtlAllocateHeap(
            door_windows_abi::narrow(heap()),
            0x8,
            16 + descriptor_bytes(dims) + extra,
        )
    };
    if block == 0 {
        0
    } else {
        door_windows_abi::widen(block + 16)
    }
}
fn set_u16(sa: u64, at: u64, v: u16) -> Result<(), u32> {
    write_u16(sa + at, v)
}
fn vartype(sa: u64, h: &Header) -> Result<u16, u32> {
    if h.features & feature::HAVEVARTYPE != 0 {
        return Ok(read_u32(sa - 4)? as u16);
    }
    Ok(if h.features & feature::BSTR != 0 {
        vt::BSTR
    } else if h.features & feature::UNKNOWN != 0 {
        vt::UNKNOWN
    } else if h.features & feature::DISPATCH != 0 {
        vt::DISPATCH
    } else if h.features & feature::VARIANT != 0 {
        vt::VARIANT
    } else if h.features & feature::RECORD != 0 {
        vt::RECORD
    } else {
        vt::EMPTY
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayAllocDescriptor(dims: u32, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        if dims == 0 || dims > 0xFFFF || out == 0 {
            return hr::E_INVALIDARG;
        }
        let sa = new_descriptor(dims, 0);
        if sa == 0 {
            return hr::E_OUTOFMEMORY;
        }
        result(set_u16(sa, 0, dims as u16).and_then(|()| write_pointer(out, sa)))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayAllocDescriptorEx(kind: u16, dims: u32, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        let r = SafeArrayAllocDescriptor(dims, door_windows_abi::narrow(out));
        if r != hr::S_OK {
            return r;
        }
        let sa = read_pointer(out).unwrap_or(0);
        // Interfaces keep their IID; records an IRecordInfo; the rest a VARTYPE.
        let r = match kind {
            vt::UNKNOWN | vt::DISPATCH => set_u16(sa, 2, feature::HAVEIID).and_then(|()| {
                write(
                    sa - 16,
                    if kind == vt::UNKNOWN {
                        &IID_IUNKNOWN
                    } else {
                        &IID_IDISPATCH
                    },
                )
            }),
            vt::RECORD => set_u16(sa, 2, feature::RECORD),
            _ => set_u16(sa, 2, feature::HAVEVARTYPE)
                .and_then(|()| write_u32(sa - 4, u32::from(kind))),
        };
        result(r)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayAllocData(sa: usize) -> u32 {
    let sa = door_windows_abi::widen(sa);
    {
        let r = (|| {
            let h = header(sa)?;
            let block = unsafe {
                RtlAllocateHeap(
                    door_windows_abi::narrow(heap()),
                    0x8,
                    h.bytes().max(1) as usize,
                )
            };
            if block == 0 {
                return Err(hr::E_OUTOFMEMORY);
            }
            write_pointer(sa + DATA_AT, door_windows_abi::widen(block))
        })();
        result(r)
    }
}
fn create(kind: u16, dims: u32, bounds: u64, vector: Option<(i32, u32)>) -> u64 {
    let size = element_size(kind);
    if size == 0 || dims == 0 {
        return 0;
    }
    let r = (|| -> Result<u64, u32> {
        // The caller's bounds are first dimension first; storage is last first.
        let mut list: Vec<(u32, i32)> = match vector {
            Some((lower, n)) => vec![(n, lower)],
            None => {
                let mut l = Vec::new();
                for i in 0..u64::from(dims) {
                    let b: [u8; 8] = read_n(bounds + 8 * i)?;
                    l.push((
                        u32::from_le_bytes(b[..4].try_into().unwrap()),
                        i32::from_le_bytes(b[4..].try_into().unwrap()),
                    ));
                }
                l
            }
        };
        list.reverse();
        let cells: u64 = list.iter().map(|(n, _)| u64::from(*n)).product();
        let data_bytes = (cells * u64::from(size)) as usize;
        // A vector's data follows its descriptor (FADF_CREATEVECTOR).
        let sa = new_descriptor(dims, if vector.is_some() { data_bytes } else { 0 });
        if sa == 0 {
            return Err(hr::E_OUTOFMEMORY);
        }
        let mut features = kind_feature(kind);
        match kind {
            vt::UNKNOWN | vt::DISPATCH => {
                features |= feature::HAVEIID;
                write(
                    sa - 16,
                    if kind == vt::UNKNOWN {
                        &IID_IUNKNOWN
                    } else {
                        &IID_IDISPATCH
                    },
                )?;
            }
            _ => {
                features |= feature::HAVEVARTYPE;
                write_u32(sa - 4, u32::from(kind))?;
            }
        }
        let data = if vector.is_some() {
            features |= feature::CREATEVECTOR;
            sa + descriptor_bytes(dims) as u64
        } else {
            let block = unsafe {
                RtlAllocateHeap(door_windows_abi::narrow(heap()), 0x8, data_bytes.max(1))
            };
            if block == 0 {
                return Err(hr::E_OUTOFMEMORY);
            }
            door_windows_abi::widen(block)
        };
        let mut head = [0u8; 24];
        head[..2].copy_from_slice(&(dims as u16).to_le_bytes());
        head[2..4].copy_from_slice(&features.to_le_bytes());
        head[4..8].copy_from_slice(&size.to_le_bytes());
        head[DATA_AT as usize..HEAD as usize].copy_from_slice(&data.to_le_bytes()[..POINTER]);
        write(sa, &head[..HEAD as usize])?;
        for (i, (n, lower)) in list.iter().enumerate() {
            let mut b = [0u8; 8];
            b[..4].copy_from_slice(&n.to_le_bytes());
            b[4..].copy_from_slice(&lower.to_le_bytes());
            write(sa + HEAD + 8 * i as u64, &b)?;
        }
        Ok(sa)
    })();
    r.unwrap_or(0)
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayCreate(kind: u16, dims: u32, bounds: usize) -> usize {
    let bounds = door_windows_abi::widen(bounds);
    door_windows_abi::narrow((move || -> u64 {
        if bounds == 0 {
            return 0;
        }
        create(kind, dims, bounds, None)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayCreateEx(
    kind: u16,
    dims: u32,
    bounds: usize,
    _extra: usize,
) -> usize {
    let bounds = door_windows_abi::widen(bounds);
    door_windows_abi::narrow((move || -> u64 {
        door_windows_abi::widen(SafeArrayCreate(
            kind,
            dims,
            door_windows_abi::narrow(bounds),
        ))
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayCreateVector(kind: u16, lower: i32, count: u32) -> usize {
    door_windows_abi::narrow((move || -> u64 {
        create(kind, 1, 0, Some((lower, count)))
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayCreateVectorEx(
    kind: u16,
    lower: i32,
    count: u32,
    _extra: usize,
) -> usize {
    door_windows_abi::narrow((move || -> u64 {
        door_windows_abi::widen(SafeArrayCreateVector(kind, lower, count))
    })())
}
/// Release what the cells own (BSTRs, interfaces, VARIANTs).
fn clear_cells(sa: u64, h: &Header, from: u64) -> Result<(), u32> {
    if h.data == 0 {
        return Ok(());
    }
    let size = u64::from(h.element);
    for cell in from..h.cells() {
        let p = h.data + cell * size;
        if h.features & feature::BSTR != 0 {
            bstr_free(read_pointer(p)?);
            write_pointer(p, 0)?;
        } else if h.features & (feature::UNKNOWN | feature::DISPATCH) != 0 {
            release(read_pointer(p)?);
            write_pointer(p, 0)?;
        } else if h.features & feature::VARIANT != 0 {
            clear(p)?;
        }
    }
    let _ = sa;
    Ok(())
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayDestroyData(sa: usize) -> u32 {
    let sa = door_windows_abi::widen(sa);
    {
        let r = (|| {
            let h = header(sa)?;
            if h.locks != 0 {
                return Err(hr::DISP_E_ARRAYISLOCKED);
            }
            clear_cells(sa, &h, 0)?;
            if h.features
                & (feature::STATIC | feature::EMBEDDED | feature::AUTO | feature::CREATEVECTOR)
                == 0
                && h.data != 0
            {
                unsafe {
                    RtlFreeHeap(
                        door_windows_abi::narrow(heap()),
                        0,
                        door_windows_abi::narrow(h.data),
                    )
                };
                write_pointer(sa + DATA_AT, 0)?;
            } else if h.features & feature::CREATEVECTOR != 0 {
                // The vector's own block stays (its data is zeroed).
                write(h.data, &vec![0u8; h.bytes() as usize])?;
            }
            Ok(())
        })();
        result(r)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayDestroyDescriptor(sa: usize) -> u32 {
    let sa = door_windows_abi::widen(sa);
    {
        let r = (|| {
            let h = header(sa)?;
            if h.locks != 0 {
                return Err(hr::DISP_E_ARRAYISLOCKED);
            }
            if h.features & feature::RECORD != 0 {
                release(read_pointer(sa - RECORD_AT)?);
            }
            unsafe {
                RtlFreeHeap(
                    door_windows_abi::narrow(heap()),
                    0,
                    door_windows_abi::narrow(sa - 16),
                )
            };
            Ok(())
        })();
        result(r)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayDestroy(sa: usize) -> u32 {
    let sa = door_windows_abi::widen(sa);
    {
        if sa == 0 {
            return hr::S_OK;
        }
        match header(sa) {
            Ok(h) if h.locks != 0 => return hr::DISP_E_ARRAYISLOCKED,
            Ok(_) => (),
            Err(e) => return e,
        }
        let r = SafeArrayDestroyData(door_windows_abi::narrow(sa));
        if r != hr::S_OK {
            return r;
        }
        SafeArrayDestroyDescriptor(door_windows_abi::narrow(sa))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayLock(sa: usize) -> u32 {
    let sa = door_windows_abi::widen(sa);
    {
        let r = (|| {
            let h = header(sa)?;
            if h.locks >= 0xFFFF {
                return Err(hr::E_UNEXPECTED);
            }
            write_u32(sa + 8, h.locks + 1)
        })();
        result(r)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayUnlock(sa: usize) -> u32 {
    let sa = door_windows_abi::widen(sa);
    {
        let r = (|| {
            let h = header(sa)?;
            if h.locks == 0 {
                return Err(hr::E_UNEXPECTED);
            }
            write_u32(sa + 8, h.locks - 1)
        })();
        result(r)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayAccessData(sa: usize, out: usize) -> u32 {
    let sa = door_windows_abi::widen(sa);
    let out = door_windows_abi::widen(out);
    {
        let r = SafeArrayLock(door_windows_abi::narrow(sa));
        if r != hr::S_OK {
            return r;
        }
        result(header(sa).and_then(|h| write_pointer(out, h.data)))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayUnaccessData(sa: usize) -> u32 {
    let sa = door_windows_abi::widen(sa);
    SafeArrayUnlock(door_windows_abi::narrow(sa))
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayGetDim(sa: usize) -> u32 {
    let sa = door_windows_abi::widen(sa);
    header(sa).map_or(0, |h| u32::from(h.dims))
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayGetElemsize(sa: usize) -> u32 {
    let sa = door_windows_abi::widen(sa);
    header(sa).map_or(0, |h| h.element)
}
fn bound(sa: u64, dim: u32, out: u64, upper: bool) -> u32 {
    let r = (|| {
        let h = header(sa)?;
        if dim == 0 || dim > u32::from(h.dims) {
            return Err(hr::DISP_E_BADINDEX);
        }
        let (n, lower) = h.bounds[(u32::from(h.dims) - dim) as usize];
        let v = if upper {
            lower.wrapping_add(n as i32).wrapping_sub(1)
        } else {
            lower
        };
        write(out, &v.to_le_bytes())
    })();
    result(r)
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayGetLBound(sa: usize, dim: u32, out: usize) -> u32 {
    let sa = door_windows_abi::widen(sa);
    let out = door_windows_abi::widen(out);
    bound(sa, dim, out, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayGetUBound(sa: usize, dim: u32, out: usize) -> u32 {
    let sa = door_windows_abi::widen(sa);
    let out = door_windows_abi::widen(out);
    bound(sa, dim, out, true)
}
/// The address of the cell `indices` names (rgIndices[0]: the first
/// dimension, fastest varying).
fn cell(sa: u64, h: &Header, indices: u64) -> Result<u64, u32> {
    let mut offset: u64 = 0;
    let mut stride: u64 = 1;
    for i in 0..usize::from(h.dims) {
        let index = read_u32(indices + 4 * i as u64)? as i32;
        let (n, lower) = h.bounds[usize::from(h.dims) - 1 - i];
        let rel = i64::from(index) - i64::from(lower);
        if rel < 0 || rel >= i64::from(n) {
            return Err(hr::DISP_E_BADINDEX);
        }
        offset += rel as u64 * stride;
        stride *= u64::from(n);
    }
    let _ = sa;
    Ok(h.data + offset * u64::from(h.element))
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayPtrOfIndex(sa: usize, indices: usize, out: usize) -> u32 {
    let sa = door_windows_abi::widen(sa);
    let indices = door_windows_abi::widen(indices);
    let out = door_windows_abi::widen(out);
    {
        result(
            header(sa)
                .and_then(|h| cell(sa, &h, indices))
                .and_then(|p| write_pointer(out, p)),
        )
    }
}
/// Copy one element of an array of `features`: a new BSTR, an AddRef, a
/// VariantCopy, or the bytes.
fn copy_element(h: &Header, from: u64, to: u64) -> Result<(), u32> {
    if h.features & feature::BSTR != 0 {
        let s = read_pointer(from)?;
        let copy = if s == 0 {
            0
        } else {
            bstr_from(&bstr_units(s)?)
        };
        let old = read_pointer(to)?;
        write_pointer(to, copy)?;
        bstr_free(old);
    } else if h.features & (feature::UNKNOWN | feature::DISPATCH) != 0 {
        let p = read_pointer(from)?;
        add_ref(p);
        let old = read_pointer(to)?;
        write_pointer(to, p)?;
        release(old);
    } else if h.features & feature::VARIANT != 0 {
        super::copy(to, from, false)?;
    } else {
        let mut b = vec![0u8; h.element as usize];
        read(from, &mut b)?;
        write(to, &b)?;
    }
    Ok(())
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayPutElement(sa: usize, indices: usize, data: usize) -> u32 {
    let sa = door_windows_abi::widen(sa);
    let indices = door_windows_abi::widen(indices);
    let data = door_windows_abi::widen(data);
    {
        let r = (|| {
            let h = header(sa)?;
            let p = cell(sa, &h, indices)?;
            if h.features & (feature::BSTR | feature::UNKNOWN | feature::DISPATCH) != 0 {
                // `data` is the BSTR / interface pointer itself.
                let value = data.to_le_bytes();
                copy_element(&h, value.as_ptr() as u64, p)
            } else {
                copy_element(&h, data, p)
            }
        })();
        result(r)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayGetElement(sa: usize, indices: usize, out: usize) -> u32 {
    let sa = door_windows_abi::widen(sa);
    let indices = door_windows_abi::widen(indices);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            let h = header(sa)?;
            let p = cell(sa, &h, indices)?;
            if h.features & feature::BSTR != 0 {
                let s = read_pointer(p)?;
                let copy = if s == 0 {
                    0
                } else {
                    bstr_from(&bstr_units(s)?)
                };
                return write_pointer(out, copy);
            }
            if h.features & feature::VARIANT != 0 {
                VariantInit(door_windows_abi::narrow(out));
            } else if h.features & (feature::UNKNOWN | feature::DISPATCH) != 0 {
                write_pointer(out, 0)?;
            }
            copy_element(&h, p, out)
        })();
        result(r)
    }
}
/// SafeArrayCopy's deep copy; its descriptor keeps the type but not
/// FADF_CREATEVECTOR (Windows: an I4 vector's copy has 0x80).
pub(super) fn copy_array(sa: u64) -> Result<u64, u32> {
    let h = header(sa)?;
    let kind = vartype(sa, &h)?;
    let mut bounds: Vec<u8> = Vec::new();
    for (n, lower) in h.bounds.iter().rev() {
        bounds.extend_from_slice(&n.to_le_bytes());
        bounds.extend_from_slice(&lower.to_le_bytes());
    }
    let copy = create(kind, u32::from(h.dims), bounds.as_ptr() as u64, None);
    if copy == 0 {
        return Err(hr::E_OUTOFMEMORY);
    }
    let target = header(copy)?;
    for i in 0..h.cells() {
        let size = u64::from(h.element);
        copy_element(&target, h.data + i * size, target.data + i * size)?;
    }
    Ok(copy)
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayCopy(sa: usize, out: usize) -> u32 {
    let sa = door_windows_abi::widen(sa);
    let out = door_windows_abi::widen(out);
    {
        if sa == 0 {
            return result(write_pointer(out, 0));
        }
        result(copy_array(sa).and_then(|c| write_pointer(out, c)))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayCopyData(source: usize, target: usize) -> u32 {
    let source = door_windows_abi::widen(source);
    let target = door_windows_abi::widen(target);
    {
        let r = (|| {
            let (a, b) = (header(source)?, header(target)?);
            if a.dims != b.dims || a.element != b.element || a.bounds != b.bounds {
                return Err(hr::E_INVALIDARG);
            }
            for i in 0..a.cells() {
                let size = u64::from(a.element);
                copy_element(&b, a.data + i * size, b.data + i * size)?;
            }
            Ok(())
        })();
        result(r)
    }
}
/// SafeArrayRedim: only the last dimension (stored first) changes; new
/// cells are zero, removed ones released.
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayRedim(sa: usize, bound: usize) -> u32 {
    let sa = door_windows_abi::widen(sa);
    let bound = door_windows_abi::widen(bound);
    {
        let r = (|| {
            let h = header(sa)?;
            if h.locks != 0 {
                return Err(hr::DISP_E_ARRAYISLOCKED);
            }
            let b: [u8; 8] = read_n(bound)?;
            let n = u32::from_le_bytes(b[..4].try_into().unwrap());
            let lower = i32::from_le_bytes(b[4..].try_into().unwrap());
            let others: u64 = h.bounds[1..].iter().map(|(n, _)| u64::from(*n)).product();
            let new_cells = others * u64::from(n);
            let size = u64::from(h.element);
            if new_cells < h.cells() {
                clear_cells(sa, &h, new_cells)?;
            }
            let mut data = vec![0u8; (h.cells().min(new_cells) * size) as usize];
            read(h.data, &mut data)?;
            data.resize((new_cells * size) as usize, 0);
            let block = unsafe {
                RtlAllocateHeap(door_windows_abi::narrow(heap()), 0x8, data.len().max(1))
            };
            if block == 0 {
                return Err(hr::E_OUTOFMEMORY);
            }
            write(door_windows_abi::widen(block), &data)?;
            if h.features & feature::CREATEVECTOR == 0 {
                unsafe {
                    RtlFreeHeap(
                        door_windows_abi::narrow(heap()),
                        0,
                        door_windows_abi::narrow(h.data),
                    )
                };
            }
            write_pointer(sa + DATA_AT, door_windows_abi::widen(block))?;
            // The vector's data no longer follows it.
            set_u16(sa, 2, h.features & !feature::CREATEVECTOR)?;
            let mut nb = [0u8; 8];
            nb[..4].copy_from_slice(&n.to_le_bytes());
            nb[4..].copy_from_slice(&lower.to_le_bytes());
            write(sa + HEAD, &nb)
        })();
        result(r)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayGetVartype(sa: usize, out: usize) -> u32 {
    let sa = door_windows_abi::widen(sa);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            let h = header(sa)?;
            if h.features
                & (feature::HAVEVARTYPE
                    | feature::BSTR
                    | feature::UNKNOWN
                    | feature::DISPATCH
                    | feature::VARIANT
                    | feature::RECORD)
                == 0
            {
                return Err(hr::E_INVALIDARG);
            }
            write_u16(out, vartype(sa, &h)?)
        })();
        result(r)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayGetIID(sa: usize, out: usize) -> u32 {
    let sa = door_windows_abi::widen(sa);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            let h = header(sa)?;
            if h.features & feature::HAVEIID == 0 {
                return Err(hr::E_INVALIDARG);
            }
            let iid: [u8; 16] = read_n(sa - 16)?;
            write(out, &iid)
        })();
        result(r)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArraySetIID(sa: usize, iid: usize) -> u32 {
    let sa = door_windows_abi::widen(sa);
    let iid = door_windows_abi::widen(iid);
    {
        let r = (|| {
            let h = header(sa)?;
            if h.features & feature::HAVEIID == 0 {
                return Err(hr::E_INVALIDARG);
            }
            let g: [u8; 16] = read_n(iid)?;
            write(sa - 16, &g)
        })();
        result(r)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArrayGetRecordInfo(sa: usize, out: usize) -> u32 {
    let sa = door_windows_abi::widen(sa);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            let h = header(sa)?;
            if h.features & feature::RECORD == 0 {
                return Err(hr::E_INVALIDARG);
            }
            let info = read_pointer(sa - RECORD_AT)?;
            add_ref(info);
            write_pointer(out, info)
        })();
        result(r)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SafeArraySetRecordInfo(sa: usize, info: usize) -> u32 {
    let sa = door_windows_abi::widen(sa);
    let info = door_windows_abi::widen(info);
    {
        let r = (|| {
            let h = header(sa)?;
            if h.features & feature::RECORD == 0 {
                return Err(hr::E_INVALIDARG);
            }
            add_ref(info);
            release(read_pointer(sa - RECORD_AT)?);
            write_pointer(sa - RECORD_AT, info)
        })();
        result(r)
    }
}
