//! IRecordInfo (GetRecordInfoFromTypeInfo, GetRecordInfoFromGuids): a
//! user-defined type's fields read once from its ITypeInfo (any
//! implementation, through the vtable), then RecordInit/Clear/Copy,
//! RecordCreate/CreateCopy/Destroy, field access through VARIANTs, field
//! names and type matching as Windows' oleaut32 does them. BSTRs, VARIANTs,
//! interfaces and SAFEARRAYs in a record are owned by it (freed on clear,
//! duplicated on copy); a nested record recurses; every other field is
//! plain bytes. CoreCLR binds this for COM interop of structures.
use super::*;
use alloc::boxed::Box;
use core::sync::atomic::AtomicU32;

/// {0000002F-0000-0000-C000-000000000046}
const IID_IRECORDINFO: [u8; 16] = [0x2F, 0, 0, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
const IID_IUNKNOWN: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
const TKIND_ENUM: u32 = 0;
const TKIND_RECORD: u32 = 1;
const TKIND_INTERFACE: u32 = 3;
const TKIND_DISPATCH: u32 = 4;
const TKIND_ALIAS: u32 = 6;
const VAR_PERINSTANCE: u32 = 0;
const MEMBERID_NIL: i32 = -1;
const DISP_E_BADVARTYPE: u32 = 0x8002_0008;
const DISP_E_TYPEMISMATCH: u32 = 0x8002_0005;
/// Nested records followed when a type is read (a cycle is malformed).
const MAX_DEPTH: usize = 16;

/// What a field holds and how the record owns it.
#[derive(Clone, Copy)]
enum Kind {
    /// Plain bytes of this size (numbers, pointers, enums, fixed arrays).
    Bytes(u32),
    Bstr,
    Variant,
    /// An IUnknown or IDispatch pointer.
    Interface,
    SafeArray,
    /// A nested record: its IRecordInfo (one reference held).
    Record(u64),
}
struct Field {
    name: Vec<u16>,
    offset: u32,
    /// The VARIANT type a get/put converts through (0: none).
    vt: u16,
    kind: Kind,
}
#[repr(C)]
struct Record {
    vtbl: &'static Vtbl,
    references: AtomicU32,
    type_info: u64,
    guid: [u8; 16],
    name: Vec<u16>,
    size: u32,
    fields: Vec<Field>,
}
type Query = extern "system" fn(usize, usize, usize) -> u32;
#[repr(C)]
struct Vtbl {
    query: Query,
    add_ref: extern "system" fn(usize) -> u32,
    release: extern "system" fn(usize) -> u32,
    record_init: extern "system" fn(usize, usize) -> u32,
    record_clear: extern "system" fn(usize, usize) -> u32,
    record_copy: extern "system" fn(usize, usize, usize) -> u32,
    get_guid: extern "system" fn(usize, usize) -> u32,
    get_name: extern "system" fn(usize, usize) -> u32,
    get_size: extern "system" fn(usize, usize) -> u32,
    get_type_info: extern "system" fn(usize, usize) -> u32,
    get_field: extern "system" fn(usize, usize, usize, usize) -> u32,
    get_field_no_copy: extern "system" fn(usize, usize, usize, usize, usize) -> u32,
    put_field: extern "system" fn(usize, u32, usize, usize, usize) -> u32,
    put_field_no_copy: extern "system" fn(usize, u32, usize, usize, usize) -> u32,
    get_field_names: extern "system" fn(usize, usize, usize) -> u32,
    is_matching_type: extern "system" fn(usize, usize) -> i32,
    record_create: extern "system" fn(usize) -> usize,
    record_create_copy: extern "system" fn(usize, usize, usize) -> u32,
    record_destroy: extern "system" fn(usize, usize) -> u32,
}
static VTBL: Vtbl = Vtbl {
    query,
    add_ref: rec_add_ref,
    release: rec_release,
    record_init,
    record_clear,
    record_copy,
    get_guid,
    get_name,
    get_size,
    get_type_info,
    get_field,
    get_field_no_copy,
    put_field,
    put_field_no_copy,
    get_field_names,
    is_matching_type,
    record_create,
    record_create_copy,
    record_destroy,
};
/// SAFETY: `this` is a Record's interface pointer (ours: the vtable is
/// checked by callers that take a foreign one).
fn object(this: u64) -> &'static Record {
    unsafe { &*(this as *const Record) }
}
fn ours(pointer: u64) -> bool {
    pointer != 0 && read_pointer(pointer).is_ok_and(|v| v == &VTBL as *const Vtbl as u64)
}

// ---- reading the type ---------------------------------------------------------------

/// An ITypeInfo method with the given arity, by vtable index.
fn ti1(object: u64, index: usize, a: u64) -> u32 {
    let f: extern "system" fn(usize, usize) -> u32 =
        unsafe { core::mem::transmute(method(object, index) as usize) };
    f(
        door_windows_abi::narrow(object),
        door_windows_abi::narrow(a),
    )
}
fn ti2(object: u64, index: usize, a: u64, b: u64) -> u32 {
    let f: extern "system" fn(usize, usize, usize) -> u32 =
        unsafe { core::mem::transmute(method(object, index) as usize) };
    f(
        door_windows_abi::narrow(object),
        door_windows_abi::narrow(a),
        door_windows_abi::narrow(b),
    )
}
fn check(hr: u32) -> Result<(), u32> {
    if hr & 0x8000_0000 != 0 {
        Err(hr)
    } else {
        Ok(())
    }
}
/// TYPEATTR: (GUID, cbSizeInstance, typekind, cVars, tdescAlias.vt).
fn type_attr(type_info: u64) -> Result<([u8; 16], u32, u32, u16, u16), u32> {
    let mut attr = 0u64;
    check(ti1(type_info, 3, &mut attr as *mut u64 as u64))?;
    let p = POINTER as u64;
    let read = (|| {
        Ok::<_, u32>((
            read_n::<16>(attr)?,
            read_u32(attr + 32 + p)?,
            read_u32(attr + 36 + p)?,
            read_u16(attr + 42 + p)?,
            // tdescAlias follows the version words (56 + POINTER), its vt
            // after the union.
            read_u16(attr + 56 + 2 * p)?,
        ))
    })();
    // ReleaseTypeAttr.
    ti1(type_info, 19, attr);
    read
}
fn vt_size(vt: u16) -> Option<u32> {
    Some(match vt {
        vt::I1 | vt::UI1 => 1,
        vt::I2 | vt::UI2 | vt::BOOL => 2,
        vt::I4 | vt::UI4 | vt::R4 | vt::INT | vt::UINT | vt::ERROR | vt::HRESULT => 4,
        vt::I8 | vt::UI8 | vt::R8 | vt::CY | vt::DATE => 8,
        vt::DECIMAL => 16,
        vt::PTR | vt::LPSTR | vt::LPWSTR | vt::INT_PTR | vt::UINT_PTR => POINTER as u32,
        _ => return None,
    })
}
/// One field's kind from its TYPEDESC at `tdesc`.
fn field_kind(type_info: u64, tdesc: u64, depth: usize) -> Result<(Kind, u16), u32> {
    let p = POINTER as u64;
    let vt = read_u16(tdesc + p)?;
    Ok(match vt {
        vt::BSTR => (Kind::Bstr, vt),
        vt::VARIANT => (Kind::Variant, vt),
        vt::UNKNOWN | vt::DISPATCH => (Kind::Interface, vt),
        vt::SAFEARRAY => (Kind::SafeArray, vt::ARRAY),
        vt::USERDEFINED => {
            let href = read_u32(tdesc)?;
            let mut referenced = 0u64;
            check(ti2(
                type_info,
                14,
                u64::from(href),
                &mut referenced as *mut u64 as u64,
            ))?;
            let kind = (|| {
                let (_, size, kind, _, alias) = type_attr(referenced)?;
                Ok::<_, u32>(match kind {
                    TKIND_RECORD => (Kind::Record(build(referenced, depth + 1)?), vt::RECORD),
                    TKIND_ENUM => (Kind::Bytes(4), vt::I4),
                    TKIND_ALIAS if vt_size(alias).is_some() => (Kind::Bytes(size), alias),
                    TKIND_ALIAS if alias == vt::BSTR => (Kind::Bstr, vt::BSTR),
                    TKIND_ALIAS if alias == vt::VARIANT => (Kind::Variant, vt::VARIANT),
                    TKIND_INTERFACE | TKIND_DISPATCH => (Kind::Interface, vt::UNKNOWN),
                    _ => (Kind::Bytes(size), 0),
                })
            })();
            release(referenced);
            kind?
        }
        vt::PTR => {
            // A pointer to an interface is an owned interface field.
            let pointee = read_pointer(tdesc)?;
            let interface = pointee != 0
                && read_u16(pointee + p).is_ok_and(|v| v == vt::USERDEFINED)
                && read_u32(pointee).is_ok_and(|href| {
                    let mut referenced = 0u64;
                    if check(ti2(
                        type_info,
                        14,
                        u64::from(href),
                        &mut referenced as *mut u64 as u64,
                    ))
                    .is_err()
                    {
                        return false;
                    }
                    let kind = type_attr(referenced).map(|a| a.2);
                    release(referenced);
                    matches!(kind, Ok(TKIND_INTERFACE | TKIND_DISPATCH))
                });
            if interface {
                (Kind::Interface, vt::UNKNOWN)
            } else {
                (Kind::Bytes(POINTER as u32), 0)
            }
        }
        vt::CARRAY => {
            // ARRAYDESC: the element TYPEDESC, cDims, then the bounds.
            let desc = read_pointer(tdesc)?;
            let element = read_u16(desc + p)?;
            let dims = read_u16(desc + 2 * p)?;
            let mut count = 1u32;
            for d in 0..u64::from(dims) {
                count = count.saturating_mul(read_u32(desc + 2 * p + 4 + d * 8)?);
            }
            let each = vt_size(element).ok_or(DISP_E_BADVARTYPE)?;
            (Kind::Bytes(each.saturating_mul(count)), 0)
        }
        other => (Kind::Bytes(vt_size(other).ok_or(DISP_E_BADVARTYPE)?), other),
    })
}
/// A new IRecordInfo for the record `type_info` describes.
fn build(type_info: u64, depth: usize) -> Result<u64, u32> {
    if depth > MAX_DEPTH {
        return Err(hr::E_INVALIDARG);
    }
    let (guid, size, kind, vars, _) = type_attr(type_info)?;
    if kind != TKIND_RECORD {
        return Err(hr::E_INVALIDARG);
    }
    let mut name = 0u64;
    let _ = ti_documentation(type_info, &mut name);
    let mut record = Box::new(Record {
        vtbl: &VTBL,
        references: AtomicU32::new(1),
        type_info,
        guid,
        name: bstr_units(name).unwrap_or_default(),
        size,
        fields: Vec::new(),
    });
    bstr_free(name);
    let p = POINTER as u64;
    let mut failure = None;
    for index in 0..u64::from(vars) {
        let mut desc = 0u64;
        if let Err(e) = check(ti2(type_info, 6, index, &mut desc as *mut u64 as u64)) {
            failure = Some(e);
            break;
        }
        let field = (|| {
            if read_u32(desc + 7 * p + 4)? != VAR_PERINSTANCE {
                return Ok(None);
            }
            let memid = read_u32(desc)?;
            let offset = read_u32(desc + 2 * p)?;
            let (kind, vt) = field_kind(type_info, desc + 3 * p, depth)?;
            let mut names = 0u64;
            let mut count = 0u32;
            let names_call: extern "system" fn(usize, u32, usize, u32, usize) -> u32 =
                unsafe { core::mem::transmute(method(type_info, 7) as usize) };
            check(names_call(
                door_windows_abi::narrow(type_info),
                memid,
                door_windows_abi::narrow(&mut names as *mut u64 as u64),
                1,
                door_windows_abi::narrow(&mut count as *mut u32 as u64),
            ))?;
            let name = bstr_units(names).unwrap_or_default();
            bstr_free(names);
            Ok::<_, u32>(Some(Field {
                name,
                offset,
                vt,
                kind,
            }))
        })();
        // ReleaseVarDesc.
        ti1(type_info, 21, desc);
        match field {
            Ok(Some(f)) => record.fields.push(f),
            Ok(None) => {}
            Err(e) => {
                failure = Some(e);
                break;
            }
        }
    }
    if let Some(e) = failure {
        for f in &record.fields {
            if let Kind::Record(inner) = f.kind {
                release(inner);
            }
        }
        return Err(e);
    }
    add_ref(type_info);
    Ok(Box::into_raw(record) as u64)
}
/// ITypeInfo::GetDocumentation(MEMBERID_NIL): the type's name.
fn ti_documentation(type_info: u64, name: &mut u64) -> u32 {
    let f: extern "system" fn(usize, i32, usize, usize, usize, usize) -> u32 =
        unsafe { core::mem::transmute(method(type_info, 12) as usize) };
    f(
        door_windows_abi::narrow(type_info),
        MEMBERID_NIL,
        door_windows_abi::narrow(name as *mut u64 as u64),
        0,
        0,
        0,
    )
}

// ---- the record's data ----------------------------------------------------------------

fn clear_data(r: &Record, data: u64) -> Result<(), u32> {
    for f in &r.fields {
        let at = data + u64::from(f.offset);
        match f.kind {
            Kind::Bytes(_) => {}
            Kind::Bstr => {
                bstr_free(read_pointer(at)?);
                write_pointer(at, 0)?;
            }
            Kind::Variant => check(VariantClear(door_windows_abi::narrow(at)))?,
            Kind::Interface => {
                release(read_pointer(at)?);
                write_pointer(at, 0)?;
            }
            Kind::SafeArray => {
                let array = read_pointer(at)?;
                if array != 0 {
                    safearray::SafeArrayDestroy(door_windows_abi::narrow(array));
                }
                write_pointer(at, 0)?;
            }
            Kind::Record(inner) => clear_data(object(inner), at)?,
        }
    }
    Ok(())
}
fn copy_bytes(source: u64, destination: u64, bytes: u32) -> Result<(), u32> {
    let mut buffer = vec![0u8; bytes as usize];
    read(source, &mut buffer)?;
    write(destination, &buffer)
}
/// After a raw copy of `source` into `destination`: each owned field of
/// `destination` made its own (strings duplicated, references taken).
fn own_copy(r: &Record, source: u64, destination: u64) -> Result<(), u32> {
    for f in &r.fields {
        let from = source + u64::from(f.offset);
        let to = destination + u64::from(f.offset);
        match f.kind {
            Kind::Bytes(_) => {}
            Kind::Bstr => {
                let s = read_pointer(from)?;
                let copy = if s == 0 {
                    0
                } else {
                    let bytes = read_u32(s - 4)? as usize;
                    let mut b = vec![0u8; bytes];
                    read(s, &mut b)?;
                    bstr_alloc(bytes, Some(&b))
                };
                write_pointer(to, copy)?;
            }
            Kind::Variant => {
                write_u16(to, vt::EMPTY)?;
                check(VariantCopy(
                    door_windows_abi::narrow(to),
                    door_windows_abi::narrow(from),
                ))?;
            }
            Kind::Interface => {
                add_ref(read_pointer(from)?);
            }
            Kind::SafeArray => {
                let array = read_pointer(from)?;
                write_pointer(to, 0)?;
                if array != 0 {
                    check(safearray::SafeArrayCopy(
                        door_windows_abi::narrow(array),
                        door_windows_abi::narrow(to),
                    ))?;
                }
            }
            Kind::Record(inner) => own_copy(object(inner), from, to)?,
        }
    }
    Ok(())
}
fn copy_record(r: &Record, source: u64, destination: u64) -> Result<(), u32> {
    if source == 0 || destination == 0 {
        return Err(hr::E_INVALIDARG);
    }
    clear_data(r, destination)?;
    copy_bytes(source, destination, r.size)?;
    own_copy(r, source, destination)
}
fn create(r: &Record) -> u64 {
    // HEAP_ZERO_MEMORY: RecordInit's zeroed record.
    unsafe { RtlAllocateHeap(door_windows_abi::narrow(heap()), 8, r.size.max(1) as usize) as u64 }
}
fn field(r: &Record, name: u64) -> Result<&Field, u32> {
    let name = read_wide(name)?;
    let fold = |u: u16| if (65..=90).contains(&u) { u + 32 } else { u };
    r.fields
        .iter()
        .find(|f| {
            f.name.len() == name.len()
                && f.name.iter().zip(&name).all(|(a, b)| fold(*a) == fold(*b))
        })
        .ok_or(hr::DISP_E_UNKNOWNNAME)
}
/// VARIANT data offset (after vt and three reserved words).
const DATA: u64 = 8;

// ---- IRecordInfo ----------------------------------------------------------------------

extern "system" fn query(this: usize, iid: usize, out: usize) -> u32 {
    let (this, iid, out) = (
        door_windows_abi::widen(this),
        door_windows_abi::widen(iid),
        door_windows_abi::widen(out),
    );
    let Ok(g) = read_n::<16>(iid) else {
        return hr::E_POINTER;
    };
    if g != IID_IUNKNOWN && g != IID_IRECORDINFO {
        let _ = write_pointer(out, 0);
        return hr::E_NOINTERFACE;
    }
    rec_add_ref(door_windows_abi::narrow(this));
    result(write_pointer(out, this))
}
extern "system" fn rec_add_ref(this: usize) -> u32 {
    object(door_windows_abi::widen(this))
        .references
        .fetch_add(1, Ordering::AcqRel)
        + 1
}
extern "system" fn rec_release(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let left = object(this).references.fetch_sub(1, Ordering::AcqRel) - 1;
    if left == 0 {
        // SAFETY: the last reference; the object came from Box::into_raw.
        let record = unsafe { Box::from_raw(this as *mut Record) };
        for f in &record.fields {
            if let Kind::Record(inner) = f.kind {
                release(inner);
            }
        }
        release(record.type_info);
    }
    left
}
extern "system" fn record_init(this: usize, data: usize) -> u32 {
    let r = object(door_windows_abi::widen(this));
    let data = door_windows_abi::widen(data);
    if data == 0 {
        return hr::E_INVALIDARG;
    }
    result(write(data, &vec![0u8; r.size as usize]))
}
extern "system" fn record_clear(this: usize, data: usize) -> u32 {
    let data = door_windows_abi::widen(data);
    if data == 0 {
        return hr::E_INVALIDARG;
    }
    result(clear_data(object(door_windows_abi::widen(this)), data))
}
extern "system" fn record_copy(this: usize, source: usize, destination: usize) -> u32 {
    result(copy_record(
        object(door_windows_abi::widen(this)),
        door_windows_abi::widen(source),
        door_windows_abi::widen(destination),
    ))
}
extern "system" fn get_guid(this: usize, out: usize) -> u32 {
    result(write(
        door_windows_abi::widen(out),
        &object(door_windows_abi::widen(this)).guid,
    ))
}
extern "system" fn get_name(this: usize, out: usize) -> u32 {
    let r = object(door_windows_abi::widen(this));
    result(write_pointer(
        door_windows_abi::widen(out),
        bstr_from(&r.name),
    ))
}
extern "system" fn get_size(this: usize, out: usize) -> u32 {
    let r = object(door_windows_abi::widen(this));
    result(write_u32(door_windows_abi::widen(out), r.size))
}
extern "system" fn get_type_info(this: usize, out: usize) -> u32 {
    let r = object(door_windows_abi::widen(this));
    let out = door_windows_abi::widen(out);
    if out == 0 {
        return hr::E_INVALIDARG;
    }
    add_ref(r.type_info);
    result(write_pointer(out, r.type_info))
}
/// A field's value as a new VARIANT at `out` (cleared first).
fn read_field(f: &Field, at: u64, out: u64) -> Result<(), u32> {
    check(VariantClear(door_windows_abi::narrow(out)))?;
    match f.kind {
        Kind::Variant => check(VariantCopy(
            door_windows_abi::narrow(out),
            door_windows_abi::narrow(at),
        )),
        Kind::Bstr => {
            let mut scratch = [0u8; 24];
            scratch[..2].copy_from_slice(&vt::BSTR.to_le_bytes());
            scratch[8..8 + POINTER].copy_from_slice(&read_pointer(at)?.to_le_bytes()[..POINTER]);
            // VariantCopy duplicates the string into `out`.
            check(VariantCopy(
                door_windows_abi::narrow(out),
                door_windows_abi::narrow(scratch.as_ptr() as u64),
            ))
        }
        Kind::Interface => {
            let unknown = read_pointer(at)?;
            add_ref(unknown);
            write_u16(out, f.vt)?;
            write_pointer(out + DATA, unknown)
        }
        Kind::SafeArray => {
            let array = read_pointer(at)?;
            let mut element = 0u16;
            let mut copy = 0u64;
            if array != 0 {
                check(safearray::SafeArrayGetVartype(
                    door_windows_abi::narrow(array),
                    door_windows_abi::narrow(&mut element as *mut u16 as u64),
                ))?;
                check(safearray::SafeArrayCopy(
                    door_windows_abi::narrow(array),
                    door_windows_abi::narrow(&mut copy as *mut u64 as u64),
                ))?;
            }
            write_u16(out, vt::ARRAY | element)?;
            write_pointer(out + DATA, copy)
        }
        Kind::Record(inner) => {
            let r = object(inner);
            let copy = create(r);
            if copy == 0 {
                return Err(hr::E_OUTOFMEMORY);
            }
            copy_record(r, at, copy)?;
            add_ref(inner);
            write_u16(out, vt::RECORD)?;
            write_pointer(out + DATA, copy)?;
            write_pointer(out + DATA + POINTER as u64, inner)
        }
        Kind::Bytes(size) => {
            if f.vt == 0 || vt_size(f.vt).is_none() {
                return Err(DISP_E_BADVARTYPE);
            }
            write_u16(out, f.vt)?;
            copy_bytes(at, out + DATA, size.min(16))
        }
    }
}
extern "system" fn get_field(this: usize, data: usize, name: usize, out: usize) -> u32 {
    let r = object(door_windows_abi::widen(this));
    let (data, name, out) = (
        door_windows_abi::widen(data),
        door_windows_abi::widen(name),
        door_windows_abi::widen(out),
    );
    if data == 0 || out == 0 {
        return hr::E_INVALIDARG;
    }
    result(field(r, name).and_then(|f| read_field(f, data + u64::from(f.offset), out)))
}
extern "system" fn get_field_no_copy(
    this: usize,
    data: usize,
    name: usize,
    out: usize,
    array: usize,
) -> u32 {
    let r = object(door_windows_abi::widen(this));
    let (data, name, out, array) = (
        door_windows_abi::widen(data),
        door_windows_abi::widen(name),
        door_windows_abi::widen(out),
        door_windows_abi::widen(array),
    );
    if data == 0 || out == 0 {
        return hr::E_INVALIDARG;
    }
    result((|| {
        let f = field(r, name)?;
        let at = data + u64::from(f.offset);
        check(VariantClear(door_windows_abi::narrow(out)))?;
        match f.kind {
            // The field itself, no reference taken.
            Kind::Record(inner) => {
                write_u16(out, vt::RECORD)?;
                write_pointer(out + DATA, at)?;
                write_pointer(out + DATA + POINTER as u64, inner)?;
            }
            Kind::Bytes(_) if f.vt == 0 => return Err(DISP_E_BADVARTYPE),
            Kind::SafeArray => {
                let mut element = 0u16;
                let a = read_pointer(at)?;
                if a != 0 {
                    check(safearray::SafeArrayGetVartype(
                        door_windows_abi::narrow(a),
                        door_windows_abi::narrow(&mut element as *mut u16 as u64),
                    ))?;
                }
                write_u16(out, vt::BYREF | vt::ARRAY | element)?;
                write_pointer(out + DATA, at)?;
            }
            _ => {
                write_u16(out, vt::BYREF | f.vt)?;
                write_pointer(out + DATA, at)?;
            }
        }
        if array != 0 {
            write_pointer(array, at)?;
        }
        Ok(())
    })())
}
/// PutField (`copy`) / PutFieldNoCopy: the VARIANT at `value` into the
/// field, converted to its type.
fn put(this: usize, flags: u32, data: usize, name: usize, value: usize, copy: bool) -> u32 {
    let r = object(door_windows_abi::widen(this));
    let (data, name, value) = (
        door_windows_abi::widen(data),
        door_windows_abi::widen(name),
        door_windows_abi::widen(value),
    );
    // INVOKE_PROPERTYPUT or INVOKE_PROPERTYPUTREF.
    if data == 0 || value == 0 || flags & !(4 | 8) != 0 || flags == 0 {
        return hr::E_INVALIDARG;
    }
    result((|| {
        let f = field(r, name)?;
        let at = data + u64::from(f.offset);
        let given = read_u16(value)?;
        match f.kind {
            Kind::Variant => {
                check(VariantClear(door_windows_abi::narrow(at)))?;
                if copy {
                    check(VariantCopy(
                        door_windows_abi::narrow(at),
                        door_windows_abi::narrow(value),
                    ))
                } else {
                    copy_bytes(value, at, (DATA + 2 * POINTER as u64) as u32)
                }
            }
            Kind::Record(inner) => {
                if given & vt::TYPEMASK != vt::RECORD {
                    return Err(DISP_E_TYPEMISMATCH);
                }
                let source = read_pointer(value + DATA)?;
                copy_record(object(inner), source, at)
            }
            Kind::SafeArray => {
                if given & vt::ARRAY == 0 {
                    return Err(DISP_E_TYPEMISMATCH);
                }
                let mut array = read_pointer(value + DATA)?;
                if given & vt::BYREF != 0 {
                    array = read_pointer(array)?;
                }
                let old = read_pointer(at)?;
                if old != 0 {
                    safearray::SafeArrayDestroy(door_windows_abi::narrow(old));
                }
                write_pointer(at, 0)?;
                if !copy {
                    return write_pointer(at, array);
                }
                if array == 0 {
                    return Ok(());
                }
                check(safearray::SafeArrayCopy(
                    door_windows_abi::narrow(array),
                    door_windows_abi::narrow(at),
                ))
            }
            _ => {
                if f.vt == 0 {
                    return Err(DISP_E_BADVARTYPE);
                }
                let target = if matches!(f.kind, Kind::Interface) {
                    if given & vt::TYPEMASK == vt::DISPATCH {
                        vt::DISPATCH
                    } else {
                        vt::UNKNOWN
                    }
                } else {
                    f.vt
                };
                // The value converted to the field's type, owned by `scratch`.
                let mut scratch = [0u8; 24];
                check(VariantChangeType(
                    door_windows_abi::narrow(scratch.as_mut_ptr() as u64),
                    door_windows_abi::narrow(value),
                    0,
                    target,
                ))?;
                let converted = scratch.as_ptr() as u64;
                match f.kind {
                    Kind::Bstr => {
                        bstr_free(read_pointer(at)?);
                        // The converted string moves into the field.
                        write_pointer(at, read_pointer(converted + DATA)?)
                    }
                    Kind::Interface => {
                        release(read_pointer(at)?);
                        write_pointer(at, read_pointer(converted + DATA)?)
                    }
                    Kind::Bytes(size) => {
                        let r = copy_bytes(converted + DATA, at, size.min(16));
                        VariantClear(door_windows_abi::narrow(converted));
                        r
                    }
                    _ => unreachable!(),
                }
            }
        }
    })())
}
extern "system" fn put_field(
    this: usize,
    flags: u32,
    data: usize,
    name: usize,
    value: usize,
) -> u32 {
    put(this, flags, data, name, value, true)
}
extern "system" fn put_field_no_copy(
    this: usize,
    flags: u32,
    data: usize,
    name: usize,
    value: usize,
) -> u32 {
    put(this, flags, data, name, value, false)
}
extern "system" fn get_field_names(this: usize, count: usize, names: usize) -> u32 {
    let r = object(door_windows_abi::widen(this));
    let (count, names) = (
        door_windows_abi::widen(count),
        door_windows_abi::widen(names),
    );
    if count == 0 {
        return hr::E_INVALIDARG;
    }
    result((|| {
        if names != 0 {
            let room = read_u32(count)? as usize;
            for (i, f) in r.fields.iter().take(room).enumerate() {
                write_pointer(names + (i * POINTER) as u64, bstr_from(&f.name))?;
            }
        }
        write_u32(count, r.fields.len() as u32)
    })())
}
extern "system" fn is_matching_type(this: usize, other: usize) -> i32 {
    let r = object(door_windows_abi::widen(this));
    let other = door_windows_abi::widen(other);
    if other == 0 {
        return 0;
    }
    if ours(other) {
        let o = object(other);
        return i32::from(o.guid == r.guid && (r.guid != [0; 16] || o.name == r.name));
    }
    let mut guid = [0u8; 16];
    i32::from(ti1(other, 6, guid.as_mut_ptr() as u64) == hr::S_OK && guid == r.guid)
}
extern "system" fn record_create(this: usize) -> usize {
    door_windows_abi::narrow(create(object(door_windows_abi::widen(this))))
}
extern "system" fn record_create_copy(this: usize, source: usize, out: usize) -> u32 {
    let r = object(door_windows_abi::widen(this));
    let (source, out) = (
        door_windows_abi::widen(source),
        door_windows_abi::widen(out),
    );
    if source == 0 || out == 0 {
        return hr::E_INVALIDARG;
    }
    let copy = create(r);
    if copy == 0 {
        return hr::E_OUTOFMEMORY;
    }
    if let Err(e) = copy_record(r, source, copy) {
        unsafe {
            RtlFreeHeap(
                door_windows_abi::narrow(heap()),
                0,
                door_windows_abi::narrow(copy),
            )
        };
        return e;
    }
    result(write_pointer(out, copy))
}
extern "system" fn record_destroy(this: usize, data: usize) -> u32 {
    let data = door_windows_abi::widen(data);
    if data == 0 {
        return hr::S_OK;
    }
    let cleared = clear_data(object(door_windows_abi::widen(this)), data);
    unsafe {
        RtlFreeHeap(
            door_windows_abi::narrow(heap()),
            0,
            door_windows_abi::narrow(data),
        )
    };
    result(cleared)
}

// ---- exports --------------------------------------------------------------------------

/// GetRecordInfoFromTypeInfo: the IRecordInfo of a TKIND_RECORD type.
#[unsafe(no_mangle)]
pub extern "system" fn GetRecordInfoFromTypeInfo(type_info: usize, out: usize) -> u32 {
    let (type_info, out) = (
        door_windows_abi::widen(type_info),
        door_windows_abi::widen(out),
    );
    if type_info == 0 || out == 0 {
        return hr::E_INVALIDARG;
    }
    match build(type_info, 0) {
        Ok(record) => result(write_pointer(out, record)),
        Err(e) => {
            let _ = write_pointer(out, 0);
            e
        }
    }
}
/// GetRecordInfoFromGuids: the record `info_guid` names in the registered
/// type library `library` (version, locale).
#[unsafe(no_mangle)]
pub extern "system" fn GetRecordInfoFromGuids(
    library: usize,
    major: u32,
    minor: u32,
    lcid: u32,
    info_guid: usize,
    out: usize,
) -> u32 {
    let out64 = door_windows_abi::widen(out);
    if library == 0 || info_guid == 0 || out64 == 0 {
        return hr::E_INVALIDARG;
    }
    let mut lib = 0u64;
    let loaded = typeinfo::LoadRegTypeLib(
        library,
        major as u16,
        minor as u16,
        lcid,
        door_windows_abi::narrow(&mut lib as *mut u64 as u64),
    );
    if loaded != hr::S_OK {
        return loaded;
    }
    // ITypeLib::GetTypeInfoOfGuid.
    let mut info = 0u64;
    let found = ti2(
        lib,
        6,
        door_windows_abi::widen(info_guid),
        &mut info as *mut u64 as u64,
    );
    release(lib);
    if found != hr::S_OK {
        return found;
    }
    let r = GetRecordInfoFromTypeInfo(door_windows_abi::narrow(info), out);
    release(info);
    r
}
