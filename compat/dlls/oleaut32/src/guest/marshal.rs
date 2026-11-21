//! The automation marshalers (M6): IPSFactoryBuffer for PSOAInterface
//! {00020424-...} (interfaces a type library describes: oleautomation and
//! dual) and PSDispatch {00020420-...} (IDispatch, dispinterfaces), what
//! ole32's standard marshaling asks for. A proxy's vtable is a table of
//! thunks that land in `proxy_invoke` with the slot number and the
//! caller's arguments; it encodes the [in] values (`door_com_wire::value`)
//! into an RPCOLEMESSAGE, calls IRpcChannelBuffer::SendReceive and decodes
//! the HRESULT and [out] values. The stub decodes, calls the object's
//! method through the x64 call thunk, and encodes the results.
//!
//! The x86 build (P17) does not offer these marshalers yet: their proxy
//! thunks and stubs follow the x64 calling convention, so a 32-bit
//! process's DllGetClassObject answers CLASS_E_CLASSNOTAVAILABLE for them
//! (CoMarshalInterface then fails for type-library interfaces there).
//!
//! Values travel by type: scalars as bytes, BSTRs, VARIANTs (by reference
//! ones dereferenced), SAFEARRAYs (bounds and elements) and interface
//! pointers as OBJREFs (CoMarshalInterface). IDispatch::Invoke carries its
//! DISPPARAMS, result, EXCEPINFO and the by-reference arguments' new
//! values back to the caller's variables.
// The x86 build keeps the proxy side compiled but unused (see above).
#![cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
use super::dispatch::call_raw;
use super::typeinfo::{self, Guest, IID_IDISPATCH, IID_IUNKNOWN};
use super::*;
use crate::typelib::{self, Func, Linker, Side, TypeDesc, paramflag, tkind};
use alloc::boxed::Box;
use core::sync::atomic::{AtomicU32, AtomicU64};
use door_com_wire::value::{Reader, Writer};

#[link(name = "ole32", kind = "dylib")]
unsafe extern "system" {
    fn CoMarshalInterface(
        stream: usize,
        iid: usize,
        unknown: usize,
        context: u32,
        reserved: usize,
        flags: u32,
    ) -> u32;
    fn CoUnmarshalInterface(stream: usize, iid: usize, out: usize) -> u32;
}

const PS_OA_INTERFACE: [u8; 16] = [0x24, 4, 2, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
const PS_DISPATCH: [u8; 16] = [0x20, 4, 2, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
/// {D5F569D0-593B-101A-B569-08002B2DBF7A}
const IID_IPSFACTORYBUFFER: [u8; 16] = [
    0xD0, 0x69, 0xF5, 0xD5, 0x3B, 0x59, 0x1A, 0x10, 0xB5, 0x69, 0x08, 0x00, 0x2B, 0x2D, 0xBF, 0x7A,
];
/// {D5F56A34-593B-101A-B569-08002B2DBF7A}
const IID_IRPCPROXYBUFFER: [u8; 16] = [
    0x34, 0x6A, 0xF5, 0xD5, 0x3B, 0x59, 0x1A, 0x10, 0xB5, 0x69, 0x08, 0x00, 0x2B, 0x2D, 0xBF, 0x7A,
];
/// {D5F56AFC-593B-101A-B569-08002B2DBF7A}
const IID_IRPCSTUBBUFFER: [u8; 16] = [
    0xFC, 0x6A, 0xF5, 0xD5, 0x3B, 0x59, 0x1A, 0x10, 0xB5, 0x69, 0x08, 0x00, 0x2B, 0x2D, 0xBF, 0x7A,
];
const CO_E_OBJNOTCONNECTED: u32 = 0x8004_01FD;
const RPC_E_INVALID_DATA: u32 = 0x8001_000F;
const CLASS_E_CLASSNOTAVAILABLE: u32 = 0x8004_0111;
/// Slots the thunk table covers.
const THUNKS: usize = 1024;

// The proxy thunks: thunk k is `mov eax, k; jmp common` (10 bytes);
// common spills the register arguments into the caller's home area, keeps
// XMM0-3, and calls proxy_invoke(this, k, &arguments, &xmm).
#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    ".globl door_oleaut_thunks",
    "door_oleaut_thunks:",
    ".set door_thunk_index, 0",
    ".rept 1024",
    ".byte 0xB8",
    ".long door_thunk_index",
    ".byte 0xE9",
    ".long door_oleaut_proxy_common - (. + 4)",
    ".set door_thunk_index, door_thunk_index + 1",
    ".endr",
    "door_oleaut_proxy_common:",
    "mov [rsp + 8], rcx",
    "mov [rsp + 16], rdx",
    "mov [rsp + 24], r8",
    "mov [rsp + 32], r9",
    "sub rsp, 72",
    "movq [rsp + 32], xmm0",
    "movq [rsp + 40], xmm1",
    "movq [rsp + 48], xmm2",
    "movq [rsp + 56], xmm3",
    "mov edx, eax",
    "lea r8, [rsp + 80]",
    "lea r9, [rsp + 32]",
    "call {invoke}",
    "add rsp, 72",
    "ret",
    invoke = sym proxy_invoke,
);
#[cfg(target_arch = "x86_64")]
unsafe extern "C" {
    static door_oleaut_thunks: [u8; 0];
}
#[cfg(target_arch = "x86_64")]
fn thunk(slot: usize) -> u64 {
    // SAFETY: only the symbol's address is taken.
    (core::ptr::addr_of!(door_oleaut_thunks) as u64) + 10 * slot as u64
}
/// x86: no proxies are made (DllGetClassObject refuses the marshalers).
#[cfg(not(target_arch = "x86_64"))]
fn thunk(_slot: usize) -> u64 {
    0
}

// ---- the interface's methods ---------------------------------------------------------
/// What each vtable slot of an interface carries.
struct Methods {
    /// Slots 3..=6 are IDispatch's.
    dispatch: bool,
    /// The typed methods: (slot, library, function).
    typed: Vec<(usize, usize, Func)>,
    /// The library and type index of the interface, for GetTypeInfo.
    info: Option<(usize, u32)>,
    slots: usize,
}
impl Methods {
    fn of(iid: &[u8; 16]) -> Result<Self, u32> {
        if *iid == IID_IDISPATCH {
            return Ok(Self {
                dispatch: true,
                typed: Vec::new(),
                info: None,
                slots: 7,
            });
        }
        let (lib, index) = registered_type(iid).ok_or(super::hr::TYPE_E_LIBNOTREGISTERED)?;
        let t = &typeinfo::library(lib)
            .ok_or(super::hr::TYPE_E_LIBNOTREGISTERED)?
            .types[index as usize];
        // A dispinterface (not dual) is marshaled as IDispatch.
        if t.kind == tkind::DISPATCH && t.flags & typelib::typeflag::DUAL == 0 {
            return Ok(Self {
                dispatch: true,
                typed: Vec::new(),
                info: Some((lib, index)),
                slots: 7,
            });
        }
        let (funcs, dispatch) = typelib::vtable(&Guest, lib, index)?;
        let mut typed = Vec::new();
        let mut slots = if dispatch { 7 } else { 3 };
        for (home, f) in funcs {
            let slot = f.vft as u16 as usize / 8;
            if slot < slots.min(if dispatch { 7 } else { 3 }) {
                continue;
            }
            slots = slots.max(slot + 1);
            typed.push((slot, home, f));
        }
        if slots > THUNKS {
            return Err(super::hr::E_INVALIDARG);
        }
        Ok(Self {
            dispatch,
            typed,
            info: Some((lib, index)),
            slots,
        })
    }
    fn typed(&self, slot: usize) -> Option<(usize, &Func)> {
        self.typed.iter().find(|m| m.0 == slot).map(|m| (m.1, &m.2))
    }
}
/// The type library HKCR\Interface\{iid}\TypeLib names, loaded, and the
/// interface's index in it.
fn registered_type(iid: &[u8; 16]) -> Option<(usize, u32)> {
    let base = alloc::format!("Interface\\{}\\TypeLib", super::register::guid_text(iid));
    let (libid, version) = super::register::interface_typelib(&base)?;
    let (major, minor) = version
        .split_once('.')
        .and_then(|(a, b)| {
            Some((
                u16::from_str_radix(a, 16).ok()?,
                u16::from_str_radix(b, 16).ok()?,
            ))
        })
        .unwrap_or((1, 0));
    let path = super::register::registered_path(&libid, major, minor, 0).ok()?;
    let lib = typeinfo::load(&path).ok()?;
    let index = typeinfo::library(lib)?.by_guid(iid)?;
    Some((lib, index))
}

// ---- values -------------------------------------------------------------------------------
/// How a type's value lives in memory and travels.
#[derive(Clone, Copy)]
enum Native {
    Scalar(u16, usize),
    Bstr,
    Variant,
    Array,
    /// An interface pointer (its IID).
    Interface([u8; 16]),
}
fn native(home: usize, t: &TypeDesc, depth: u32) -> Result<Native, u32> {
    Ok(match t {
        TypeDesc::Base(v) => match *v {
            vt::BSTR => Native::Bstr,
            vt::VARIANT => Native::Variant,
            vt::UNKNOWN => Native::Interface(IID_IUNKNOWN),
            vt::DISPATCH => Native::Interface(IID_IDISPATCH),
            vt::I1 | vt::UI1 => Native::Scalar(*v, 1),
            vt::I2 | vt::UI2 | vt::BOOL => Native::Scalar(*v, 2),
            vt::I4 | vt::UI4 | vt::INT | vt::UINT | vt::ERROR | vt::HRESULT | vt::R4 => {
                Native::Scalar(*v, 4)
            }
            vt::I8 | vt::UI8 | vt::R8 | vt::CY | vt::DATE => Native::Scalar(*v, 8),
            vt::DECIMAL => Native::Scalar(*v, 16),
            _ => return Err(hr::DISP_E_BADVARTYPE),
        },
        TypeDesc::SafeArray(_) => Native::Array,
        TypeDesc::Ptr(inner) => match &**inner {
            TypeDesc::User(r) => {
                let (lib, index) = Guest.resolve(home, *r).ok_or(hr::TYPE_E_ELEMENTNOTFOUND)?;
                let ty = &typeinfo::library(lib)
                    .ok_or(hr::TYPE_E_ELEMENTNOTFOUND)?
                    .types[index as usize];
                match ty.kind {
                    tkind::INTERFACE | tkind::DISPATCH => Native::Interface(ty.guid),
                    tkind::ALIAS if depth < 8 => native(
                        lib,
                        &TypeDesc::Ptr(Box::new(ty.alias.clone().ok_or(hr::DISP_E_BADVARTYPE)?)),
                        depth + 1,
                    )?,
                    _ => return Err(hr::DISP_E_BADVARTYPE),
                }
            }
            TypeDesc::Base(vt::VOID) => Native::Interface(IID_IUNKNOWN),
            _ => return Err(hr::DISP_E_BADVARTYPE),
        },
        TypeDesc::User(r) if depth < 8 => {
            let (lib, index) = Guest.resolve(home, *r).ok_or(hr::TYPE_E_ELEMENTNOTFOUND)?;
            let ty = &typeinfo::library(lib)
                .ok_or(hr::TYPE_E_ELEMENTNOTFOUND)?
                .types[index as usize];
            match ty.kind {
                tkind::ENUM => Native::Scalar(vt::I4, 4),
                tkind::ALIAS => native(
                    lib,
                    ty.alias.as_ref().ok_or(hr::DISP_E_BADVARTYPE)?,
                    depth + 1,
                )?,
                _ => return Err(hr::DISP_E_BADVARTYPE),
            }
        }
        _ => return Err(hr::DISP_E_BADVARTYPE),
    })
}
fn native_bytes(n: Native) -> usize {
    match n {
        Native::Scalar(_, size) => size,
        Native::Variant => VARIANT_BYTES as usize,
        _ => POINTER,
    }
}
/// A parameter's shape: a value (in a register or a by-value copy) or a
/// pointer to one ([out], [in, out] or [in] by reference).
fn param_shape(home: usize, t: &TypeDesc) -> Result<(Native, bool), u32> {
    if let Ok(n @ Native::Interface(_)) = native(home, t, 0) {
        return Ok((n, false));
    }
    match t {
        TypeDesc::Ptr(inner) => Ok((native(home, inner, 0)?, true)),
        _ => Ok((native(home, t, 0)?, false)),
    }
}

// A memory IStream for OBJREFs.
#[repr(C)]
struct Stream {
    vtbl: &'static StreamVtbl,
    data: Vec<u8>,
    at: usize,
}
extern "system" fn s_query(this: usize, _iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    result(write_pointer(out, this))
}
extern "system" fn s_count(_this: usize) -> u32 {
    1
}
extern "system" fn s_read(this: usize, out: usize, n: u32, got: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    let got = door_windows_abi::widen(got);
    {
        // SAFETY: only our streams reach these methods.
        let s = unsafe { &mut *(this as *mut Stream) };
        let k = (n as usize).min(s.data.len() - s.at);
        if write(out, &s.data[s.at..s.at + k]).is_err() {
            return hr::E_POINTER;
        }
        s.at += k;
        if got != 0 {
            let _ = write_u32(got, k as u32);
        }
        hr::S_OK
    }
}
extern "system" fn s_write(this: usize, data: usize, n: u32, put: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let data = door_windows_abi::widen(data);
    let put = door_windows_abi::widen(put);
    {
        // SAFETY: as s_read.
        let s = unsafe { &mut *(this as *mut Stream) };
        let mut b = vec![0u8; n as usize];
        if read(data, &mut b).is_err() {
            return hr::E_POINTER;
        }
        s.data.extend_from_slice(&b);
        if put != 0 {
            let _ = write_u32(put, n);
        }
        hr::S_OK
    }
}
extern "system" fn s_other(_this: usize) -> u32 {
    hr::E_NOTIMPL
}
/// IStream: QueryInterface, AddRef, Release, Read, Write, then Seek,
/// SetSize, CopyTo, Commit, Revert, LockRegion, UnlockRegion, Stat and
/// Clone (E_NOTIMPL; OBJREFs only need Read and Write).
#[repr(C)]
struct StreamVtbl {
    query: extern "system" fn(usize, usize, usize) -> u32,
    add_ref: extern "system" fn(usize) -> u32,
    release: extern "system" fn(usize) -> u32,
    read: extern "system" fn(usize, usize, u32, usize) -> u32,
    write: extern "system" fn(usize, usize, u32, usize) -> u32,
    other: [extern "system" fn(usize) -> u32; 9],
}
static STREAM_VTBL: StreamVtbl = StreamVtbl {
    query: s_query,
    add_ref: s_count,
    release: s_count,
    read: s_read,
    write: s_write,
    other: [s_other; 9],
};
fn stream(data: Vec<u8>) -> Stream {
    Stream {
        vtbl: &STREAM_VTBL,
        data,
        at: 0,
    }
}
fn put_interface(w: &mut Writer, pointer: u64, iid: &[u8; 16]) -> Result<(), u32> {
    if pointer == 0 {
        w.u8(0);
        return Ok(());
    }
    let mut s = stream(Vec::new());
    // MSHCTX_LOCAL, MSHLFLAGS_NORMAL.
    let r = unsafe {
        CoMarshalInterface(
            door_windows_abi::narrow(&mut s as *mut Stream as u64),
            door_windows_abi::narrow(iid.as_ptr() as u64),
            door_windows_abi::narrow(pointer),
            1,
            0,
            0,
        )
    };
    if r != hr::S_OK {
        return Err(r);
    }
    w.u8(1).blob(&s.data);
    Ok(())
}
fn get_interface(r: &mut Reader, iid: &[u8; 16]) -> Result<u64, u32> {
    if r.u8().map_err(|_| RPC_E_INVALID_DATA)? == 0 {
        return Ok(0);
    }
    let blob = r.blob().map_err(|_| RPC_E_INVALID_DATA)?;
    let mut s = stream(blob.to_vec());
    let mut out = 0u64;
    let code = unsafe {
        CoUnmarshalInterface(
            door_windows_abi::narrow(&mut s as *mut Stream as u64),
            door_windows_abi::narrow(iid.as_ptr() as u64),
            door_windows_abi::narrow(&mut out as *mut u64 as u64),
        )
    };
    if code != hr::S_OK {
        return Err(code);
    }
    Ok(out)
}
fn bad<T>(_: T) -> u32 {
    RPC_E_INVALID_DATA
}
/// Encodes the value of kind `n` at `p`.
fn put(w: &mut Writer, n: Native, p: u64) -> Result<(), u32> {
    match n {
        Native::Scalar(_, size) => {
            let mut b = [0u8; 16];
            read(p, &mut b[..size])?;
            w.raw(&b[..size]);
        }
        Native::Bstr => {
            let b = read_u64(p)?;
            if b == 0 {
                w.bstr(None);
            } else {
                w.bstr(Some(&bstr_units(b)?));
            }
        }
        Native::Variant => put_variant(w, p, 0)?,
        Native::Array => put_array(w, read_u64(p)?)?,
        Native::Interface(iid) => put_interface(w, read_u64(p)?, &iid)?,
    }
    Ok(())
}
/// Decodes a value of kind `n` into `p` (the receiver owns it).
fn take(r: &mut Reader, n: Native, p: u64) -> Result<(), u32> {
    match n {
        Native::Scalar(_, size) => write(p, r.raw(size).map_err(bad)?),
        Native::Bstr => {
            let b = match r.bstr().map_err(bad)? {
                None => 0,
                Some(u) => bstr_from(&u),
            };
            write_u64(p, b)
        }
        Native::Variant => take_variant(r, p),
        Native::Array => {
            let sa = take_array(r)?;
            write_u64(p, sa)
        }
        Native::Interface(iid) => {
            let i = get_interface(r, &iid)?;
            write_u64(p, i)
        }
    }
}
/// Frees what a value of kind `n` at `p` owns.
fn drop_value(n: Native, p: u64) {
    match n {
        Native::Bstr => {
            if let Ok(b) = read_u64(p) {
                bstr_free(b);
            }
        }
        Native::Variant => {
            let _ = clear(p);
        }
        Native::Array => {
            if let Ok(sa) = read_u64(p)
                && sa != 0
            {
                safearray::SafeArrayDestroy(door_windows_abi::narrow(sa));
            }
        }
        Native::Interface(_) => {
            if let Ok(i) = read_u64(p) {
                release(i);
            }
        }
        Native::Scalar(..) => {}
    }
    let _ = write(p, &[0u8; 8]);
}
fn scalar_size(kind: u16) -> Option<usize> {
    Some(match kind {
        vt::I1 | vt::UI1 => 1,
        vt::I2 | vt::UI2 | vt::BOOL => 2,
        vt::I4 | vt::UI4 | vt::INT | vt::UINT | vt::ERROR | vt::R4 => 4,
        vt::I8 | vt::UI8 | vt::R8 | vt::CY | vt::DATE => 8,
        _ => return None,
    })
}
/// A VARIANT's value (a BYREF one by its referent).
fn put_variant(w: &mut Writer, p: u64, depth: u32) -> Result<(), u32> {
    let kind = read_u16(p)?;
    if kind & vt::BYREF != 0 {
        let target = read_u64(p + 8)?;
        let base = kind & !vt::BYREF;
        if base == vt::VARIANT {
            if depth > 4 {
                return Err(hr::DISP_E_BADVARTYPE);
            }
            return put_variant(w, target, depth + 1);
        }
        // The referent as a by-value VARIANT of its type.
        let mut copy = [0u8; 24];
        copy[..2].copy_from_slice(&base.to_le_bytes());
        if base == vt::DECIMAL {
            read(target, &mut copy[..16])?;
            copy[..2].copy_from_slice(&base.to_le_bytes());
        } else {
            let size = if base & vt::ARRAY != 0 {
                8
            } else {
                scalar_size(base).unwrap_or(8)
            };
            read(target, &mut copy[8..8 + size])?;
        }
        return put_variant(w, copy.as_ptr() as u64, depth + 1);
    }
    w.u16(kind);
    if kind & vt::ARRAY != 0 {
        return put_array(w, read_u64(p + 8)?);
    }
    match kind {
        vt::EMPTY | vt::NULL => {}
        vt::BSTR => put(w, Native::Bstr, p + 8)?,
        vt::UNKNOWN => put_interface(w, read_u64(p + 8)?, &IID_IUNKNOWN)?,
        vt::DISPATCH => put_interface(w, read_u64(p + 8)?, &IID_IDISPATCH)?,
        vt::DECIMAL => {
            let mut b = [0u8; 16];
            read(p, &mut b)?;
            w.raw(&b[2..]);
        }
        k => {
            let size = scalar_size(k).ok_or(hr::DISP_E_BADVARTYPE)?;
            let mut b = [0u8; 8];
            read(p + 8, &mut b[..size])?;
            w.raw(&b[..size]);
        }
    }
    Ok(())
}
fn take_variant(r: &mut Reader, p: u64) -> Result<(), u32> {
    let kind = r.u16().map_err(bad)?;
    let mut v = [0u8; 24];
    v[..2].copy_from_slice(&kind.to_le_bytes());
    if kind & vt::ARRAY != 0 {
        let sa = take_array(r)?;
        v[8..16].copy_from_slice(&sa.to_le_bytes());
        return write(p, &v);
    }
    match kind {
        vt::EMPTY | vt::NULL => {}
        vt::BSTR => {
            let b = match r.bstr().map_err(bad)? {
                None => 0,
                Some(u) => bstr_from(&u),
            };
            v[8..16].copy_from_slice(&b.to_le_bytes());
        }
        vt::UNKNOWN | vt::DISPATCH => {
            let iid = if kind == vt::UNKNOWN {
                IID_IUNKNOWN
            } else {
                IID_IDISPATCH
            };
            v[8..16].copy_from_slice(&get_interface(r, &iid)?.to_le_bytes());
        }
        vt::DECIMAL => v[2..16].copy_from_slice(r.raw(14).map_err(bad)?),
        k => {
            let size = scalar_size(k).ok_or(hr::DISP_E_BADVARTYPE)?;
            v[8..8 + size].copy_from_slice(r.raw(size).map_err(bad)?);
        }
    }
    write(p, &v)
}
/// A SAFEARRAY: dimensions, element VARTYPE, bounds (first dimension
/// first) and its elements in memory order.
fn put_array(w: &mut Writer, sa: u64) -> Result<(), u32> {
    use safearray::*;
    if sa == 0 {
        w.u8(0);
        return Ok(());
    }
    let dims = SafeArrayGetDim(door_windows_abi::narrow(sa));
    let mut kind = 0u16;
    if SafeArrayGetVartype(
        door_windows_abi::narrow(sa),
        door_windows_abi::narrow(&mut kind as *mut u16 as u64),
    ) != hr::S_OK
    {
        return Err(hr::DISP_E_BADVARTYPE);
    }
    w.u8(1).u16(dims as u16).u16(kind);
    let mut total = 1usize;
    for d in 1..=dims {
        let (mut lo, mut hi) = (0i32, 0i32);
        SafeArrayGetLBound(
            door_windows_abi::narrow(sa),
            d,
            door_windows_abi::narrow(&mut lo as *mut i32 as u64),
        );
        SafeArrayGetUBound(
            door_windows_abi::narrow(sa),
            d,
            door_windows_abi::narrow(&mut hi as *mut i32 as u64),
        );
        let count = (hi - lo + 1).max(0) as u32;
        w.u32(count).u32(lo as u32);
        total *= count as usize;
    }
    let size = SafeArrayGetElemsize(door_windows_abi::narrow(sa)) as u64;
    let mut data = 0u64;
    if SafeArrayAccessData(
        door_windows_abi::narrow(sa),
        door_windows_abi::narrow(&mut data as *mut u64 as u64),
    ) != hr::S_OK
    {
        return Err(hr::E_UNEXPECTED);
    }
    let n = element_native(kind);
    let r = (|| {
        for i in 0..total as u64 {
            put(w, n?, data + i * size)?;
        }
        Ok(())
    })();
    SafeArrayUnaccessData(door_windows_abi::narrow(sa));
    r
}
fn element_native(kind: u16) -> Result<Native, u32> {
    Ok(match kind {
        vt::BSTR => Native::Bstr,
        vt::VARIANT => Native::Variant,
        vt::UNKNOWN => Native::Interface(IID_IUNKNOWN),
        vt::DISPATCH => Native::Interface(IID_IDISPATCH),
        vt::DECIMAL => Native::Scalar(vt::DECIMAL, 16),
        k => Native::Scalar(k, scalar_size(k).ok_or(hr::DISP_E_BADVARTYPE)?),
    })
}
fn take_array(r: &mut Reader) -> Result<u64, u32> {
    use safearray::*;
    if r.u8().map_err(bad)? == 0 {
        return Ok(0);
    }
    let dims = r.u16().map_err(bad)? as usize;
    let kind = r.u16().map_err(bad)?;
    if dims == 0 || dims > 32 {
        return Err(RPC_E_INVALID_DATA);
    }
    let mut bounds = Vec::with_capacity(dims * 8);
    let mut total = 1usize;
    for _ in 0..dims {
        let count = r.u32().map_err(bad)?;
        let low = r.u32().map_err(bad)?;
        bounds.extend_from_slice(&count.to_le_bytes());
        bounds.extend_from_slice(&low.to_le_bytes());
        total = total
            .checked_mul(count as usize)
            .ok_or(RPC_E_INVALID_DATA)?;
    }
    if total > 1 << 22 {
        return Err(RPC_E_INVALID_DATA);
    }
    let sa = SafeArrayCreate(
        kind,
        dims as u32,
        door_windows_abi::narrow(bounds.as_ptr() as u64),
    );
    if sa == 0 {
        return Err(hr::E_OUTOFMEMORY);
    }
    let size = SafeArrayGetElemsize(sa) as u64;
    let mut data = 0u64;
    SafeArrayAccessData(sa, door_windows_abi::narrow(&mut data as *mut u64 as u64));
    let n = element_native(kind);
    let filled = (|| {
        for i in 0..total as u64 {
            take(r, n?, data + i * size)?;
        }
        Ok(())
    })();
    SafeArrayUnaccessData(sa);
    if let Err(e) = filled {
        SafeArrayDestroy(sa);
        return Err(e);
    }
    Ok(door_windows_abi::widen(sa))
}

// ---- the factory --------------------------------------------------------------------------
#[repr(C)]
struct FactoryVtbl {
    query: extern "system" fn(usize, usize, usize) -> u32,
    add_ref: extern "system" fn(usize) -> u32,
    release: extern "system" fn(usize) -> u32,
    create_proxy: extern "system" fn(usize, usize, usize, usize, usize) -> u32,
    create_stub: extern "system" fn(usize, usize, usize, usize) -> u32,
}
static FACTORY_VTBL: FactoryVtbl = FactoryVtbl {
    query: factory_query,
    add_ref: s_count,
    release: s_count,
    create_proxy,
    create_stub,
};
static FACTORY: [&FactoryVtbl; 1] = [&FACTORY_VTBL];
extern "system" fn factory_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        match read_n::<16>(iid) {
            Ok(g) if g == IID_IUNKNOWN || g == IID_IPSFACTORYBUFFER => {
                result(write_pointer(out, this))
            }
            _ => {
                let _ = write_pointer(out, 0);
                hr::E_NOINTERFACE
            }
        }
    }
}
/// DllGetClassObject: the automation marshalers' IPSFactoryBuffer.
#[unsafe(no_mangle)]
pub extern "system" fn DllGetClassObject(clsid: usize, iid: usize, out: usize) -> u32 {
    let clsid = door_windows_abi::widen(clsid);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        let _ = write_pointer(out, 0);
        match read_n::<16>(clsid) {
            Ok(c) if (c == PS_OA_INTERFACE || c == PS_DISPATCH) && POINTER == 8 => factory_query(
                door_windows_abi::narrow(FACTORY.as_ptr() as u64),
                door_windows_abi::narrow(iid),
                door_windows_abi::narrow(out),
            ),
            _ => CLASS_E_CLASSNOTAVAILABLE,
        }
    }
}

// ---- proxies --------------------------------------------------------------------------------
#[repr(C)]
struct BufferVtbl {
    query: extern "system" fn(usize, usize, usize) -> u32,
    add_ref: extern "system" fn(usize) -> u32,
    release: extern "system" fn(usize) -> u32,
    connect: extern "system" fn(usize, usize) -> u32,
    disconnect: extern "system" fn(usize) -> u32,
}
static BUFFER_VTBL: BufferVtbl = BufferVtbl {
    query: buffer_query,
    add_ref: buffer_add_ref,
    release: buffer_release,
    connect: buffer_connect,
    disconnect: buffer_disconnect,
};
#[repr(C)]
struct Proxy {
    /// The interface's vtable (`table`).
    vtbl: u64,
    /// IRpcProxyBuffer, at offset 8.
    buffer: &'static BufferVtbl,
    references: AtomicU32,
    outer: u64,
    channel: AtomicU64,
    iid: [u8; 16],
    methods: Methods,
    table: Vec<u64>,
}
fn proxy(this: u64) -> &'static Proxy {
    // SAFETY: proxy thunks and IUnknown methods only run on Proxies.
    unsafe { &*(this as *const Proxy) }
}
extern "system" fn proxy_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        let outer = proxy(this).outer;
        // SAFETY: the outer unknown is a live IUnknown.
        let f: extern "system" fn(usize, usize, usize) -> u32 =
            unsafe { core::mem::transmute(method(outer, 0) as usize) };
        f(
            door_windows_abi::narrow(outer),
            door_windows_abi::narrow(iid),
            door_windows_abi::narrow(out),
        )
    }
}
extern "system" fn proxy_add_ref(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    add_ref(proxy(this).outer)
}
extern "system" fn proxy_release(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    release(proxy(this).outer)
}
extern "system" fn buffer_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        match read_n::<16>(iid) {
            Ok(g) if g == IID_IUNKNOWN || g == IID_IRPCPROXYBUFFER => {
                buffer_add_ref(door_windows_abi::narrow(this));
                result(write_pointer(out, this))
            }
            _ => {
                let _ = write_pointer(out, 0);
                hr::E_NOINTERFACE
            }
        }
    }
}
extern "system" fn buffer_add_ref(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    proxy(this - 8).references.fetch_add(1, Ordering::AcqRel) + 1
}
extern "system" fn buffer_release(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        let p = proxy(this - 8);
        let left = p.references.fetch_sub(1, Ordering::AcqRel) - 1;
        if left == 0 {
            buffer_disconnect(door_windows_abi::narrow(this));
            // SAFETY: the last reference; the proxy came from Box::into_raw.
            drop(unsafe { Box::from_raw((this - 8) as *mut Proxy) });
        }
        left
    }
}
extern "system" fn buffer_connect(this: usize, channel: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let channel = door_windows_abi::widen(channel);
    {
        add_ref(channel);
        let old = proxy(this - 8).channel.swap(channel, Ordering::AcqRel);
        if old != 0 {
            release(old);
        }
        hr::S_OK
    }
}
extern "system" fn buffer_disconnect(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        let old = proxy(this - 8).channel.swap(0, Ordering::AcqRel);
        if old != 0 {
            release(old);
        }
        hr::S_OK
    }
}
/// IPSFactoryBuffer::CreateProxy: the proxy (its IRpcProxyBuffer and the
/// interface, holding a reference on `outer`).
extern "system" fn create_proxy(
    _this: usize,
    outer: usize,
    iid: usize,
    buffer_out: usize,
    out: usize,
) -> u32 {
    let outer = door_windows_abi::widen(outer);
    let iid = door_windows_abi::widen(iid);
    let buffer_out = door_windows_abi::widen(buffer_out);
    let out = door_windows_abi::widen(out);
    {
        let _ = write_pointer(buffer_out, 0);
        let _ = write_pointer(out, 0);
        let Ok(g) = read_n::<16>(iid) else {
            return hr::E_INVALIDARG;
        };
        if outer == 0 {
            return hr::E_INVALIDARG;
        }
        let methods = match Methods::of(&g) {
            Ok(m) => m,
            Err(e) => return e,
        };
        let mut table = Vec::with_capacity(methods.slots);
        table.push(proxy_query as *const () as u64);
        table.push(proxy_add_ref as *const () as u64);
        table.push(proxy_release as *const () as u64);
        for slot in 3..methods.slots {
            table.push(thunk(slot));
        }
        let p = Box::into_raw(Box::new(Proxy {
            vtbl: 0,
            buffer: &BUFFER_VTBL,
            references: AtomicU32::new(1),
            outer,
            channel: AtomicU64::new(0),
            iid: g,
            methods,
            table,
        }));
        // SAFETY: just allocated; the table lives as long as the proxy.
        unsafe { (*p).vtbl = (*p).table.as_ptr() as u64 };
        add_ref(outer);
        let this = p as u64;
        let _ = write_pointer(buffer_out, this + 8);
        result(write_pointer(out, this))
    }
}
/// The message the channel carries: request in, reply out.
fn send_receive(channel: u64, iid: &[u8; 16], slot: usize, request: &[u8]) -> Result<Vec<u8>, u32> {
    let mut msg = [0u8; 80];
    msg[8..12].copy_from_slice(&0x10u32.to_le_bytes());
    msg[24..28].copy_from_slice(&(request.len() as u32).to_le_bytes());
    msg[28..32].copy_from_slice(&(slot as u32).to_le_bytes());
    let m = msg.as_mut_ptr() as u64;
    // SAFETY: IRpcChannelBuffer::GetBuffer (3), SendReceive (4), FreeBuffer (5).
    let get: extern "system" fn(usize, usize, usize) -> u32 =
        unsafe { core::mem::transmute(method(channel, 3) as usize) };
    let send: extern "system" fn(usize, usize, usize) -> u32 =
        unsafe { core::mem::transmute(method(channel, 4) as usize) };
    let free: extern "system" fn(usize, usize) -> u32 =
        unsafe { core::mem::transmute(method(channel, 5) as usize) };
    let r = get(
        door_windows_abi::narrow(channel),
        door_windows_abi::narrow(m),
        door_windows_abi::narrow(iid.as_ptr() as u64),
    );
    if r != hr::S_OK {
        return Err(r);
    }
    write(read_u64(m + 16)?, request)?;
    let mut status = 0u32;
    let r = send(
        door_windows_abi::narrow(channel),
        door_windows_abi::narrow(m),
        door_windows_abi::narrow(&mut status as *mut u32 as u64),
    );
    if r != hr::S_OK {
        free(
            door_windows_abi::narrow(channel),
            door_windows_abi::narrow(m),
        );
        return Err(r);
    }
    let n = read_u32(m + 24)? as usize;
    let mut reply = vec![0u8; n];
    let got = read(read_u64(m + 16)?, &mut reply);
    free(
        door_windows_abi::narrow(channel),
        door_windows_abi::narrow(m),
    );
    got.map(|_| reply)
}
/// Every proxy method: `slot`, the caller's arguments (this first; the
/// first four also as XMM0-3).
#[cfg(target_arch = "x86_64")]
extern "win64" fn proxy_invoke(this: u64, slot: u32, args: *const u64, xmm: *const u64) -> u64 {
    let p = proxy(this);
    let channel = p.channel.load(Ordering::Acquire);
    if channel == 0 {
        return u64::from(CO_E_OBJNOTCONNECTED);
    }
    let slot = slot as usize;
    // SAFETY: the thunk passes the home area (and the caller's stack
    // arguments above it) and the saved XMM registers.
    let arg = |i: usize| unsafe { *args.add(i) };
    let float = |i: usize| {
        if i < 4 {
            unsafe { *xmm.add(i) }
        } else {
            arg(i)
        }
    };
    let r = if p.methods.dispatch && (3..7).contains(&slot) {
        dispatch_proxy(p, channel, slot, &arg)
    } else if let Some((home, f)) = p.methods.typed(slot) {
        typed_proxy(p, channel, slot, home, f, &arg, &float)
    } else {
        Err(hr::E_NOTIMPL)
    };
    u64::from(match r {
        Ok(code) => code,
        Err(e) => e,
    })
}
fn typed_proxy(
    p: &Proxy,
    channel: u64,
    slot: usize,
    home: usize,
    f: &Func,
    arg: &dyn Fn(usize) -> u64,
    float: &dyn Fn(usize) -> u64,
) -> Result<u32, u32> {
    let mut w = Writer::new();
    for (k, param) in f.params.iter().enumerate() {
        if param.flags & paramflag::LCID != 0 {
            continue;
        }
        let i = k + 1;
        let (n, pointer) = param_shape(home, &param.ty)?;
        let input =
            param.flags & paramflag::IN != 0 || param.flags & (paramflag::IN | paramflag::OUT) == 0;
        if !input {
            continue;
        }
        if pointer {
            put(&mut w, n, arg(i))?;
        } else if native_bytes(n) > 8 {
            // A VARIANT or DECIMAL by value: the caller passed a copy's address.
            put(&mut w, n, arg(i))?;
        } else {
            let value = if matches!(n, Native::Scalar(vt::R4 | vt::R8 | vt::DATE, _)) {
                float(i)
            } else {
                arg(i)
            };
            put(&mut w, n, &value as *const u64 as u64)?;
        }
    }
    let reply = send_receive(channel, &p.iid, slot, &w.bytes)?;
    let mut r = Reader::new(&reply);
    let code = r.u32().map_err(bad)?;
    for (k, param) in f.params.iter().enumerate() {
        if param.flags & paramflag::OUT == 0 || param.flags & paramflag::LCID != 0 {
            continue;
        }
        let (n, pointer) = param_shape(home, &param.ty)?;
        let target = arg(k + 1);
        if !pointer || target == 0 {
            continue;
        }
        if param.flags & paramflag::IN != 0 {
            drop_value(n, target);
        }
        take(&mut r, n, target)?;
    }
    Ok(code)
}
/// IDispatch's four methods over the wire.
fn dispatch_proxy(
    p: &Proxy,
    channel: u64,
    slot: usize,
    arg: &dyn Fn(usize) -> u64,
) -> Result<u32, u32> {
    let mut w = Writer::new();
    match slot {
        3 => {
            let reply = send_receive(channel, &p.iid, 3, &[])?;
            let mut r = Reader::new(&reply);
            let code = r.u32().map_err(bad)?;
            write_u32(arg(1), r.u32().map_err(bad)?)?;
            Ok(code)
        }
        4 => {
            // The interface's own type information when it has one here.
            if let Some((lib, index)) = p.methods.info {
                let side = Side::Natural;
                let ti = typeinfo::type_info(lib, index, side)?;
                write_u64(arg(3), ti)?;
                return Ok(hr::S_OK);
            }
            w.u32(arg(1) as u32).u32(arg(2) as u32);
            let reply = send_receive(channel, &p.iid, 4, &w.bytes)?;
            let mut r = Reader::new(&reply);
            let code = r.u32().map_err(bad)?;
            if code != hr::S_OK {
                write_u64(arg(3), 0)?;
                return Ok(code);
            }
            let libid = r.guid().map_err(bad)?;
            let (major, minor) = (r.u16().map_err(bad)?, r.u16().map_err(bad)?);
            let lcid = r.u32().map_err(bad)?;
            let guid = r.guid().map_err(bad)?;
            let path = super::register::registered_path(&libid, major, minor, lcid)?;
            let lib = typeinfo::load(&path)?;
            let index = typeinfo::library(lib)
                .and_then(|l| l.by_guid(&guid))
                .ok_or(hr::TYPE_E_ELEMENTNOTFOUND)?;
            write_u64(arg(3), typeinfo::type_info(lib, index, Side::Natural)?)?;
            Ok(hr::S_OK)
        }
        5 => {
            let (names, count, lcid, ids) = (arg(2), arg(3) as u32, arg(4) as u32, arg(5));
            w.u32(count).u32(lcid);
            for k in 0..count as u64 {
                let n = read_wide(read_u64(names + 8 * k)?)?;
                w.bstr(Some(&n));
            }
            let reply = send_receive(channel, &p.iid, 5, &w.bytes)?;
            let mut r = Reader::new(&reply);
            let code = r.u32().map_err(bad)?;
            for k in 0..count as u64 {
                write_u32(ids + 4 * k, r.u32().map_err(bad)?)?;
            }
            Ok(code)
        }
        _ => invoke_proxy(p, channel, arg),
    }
}
/// IDispatch::Invoke(memid, riid, lcid, flags, params, result, excep, argerr).
fn invoke_proxy(p: &Proxy, channel: u64, arg: &dyn Fn(usize) -> u64) -> Result<u32, u32> {
    let (memid, lcid, flags, params, result_out, excep, argerr) = (
        arg(1) as u32,
        arg(3) as u32,
        arg(4) as u16,
        arg(5),
        arg(6),
        arg(7),
        arg(8),
    );
    let (rgvarg, named, cargs, cnamed) = if params == 0 {
        (0, 0, 0, 0)
    } else {
        (
            read_u64(params)?,
            read_u64(params + 8)?,
            read_u32(params + 16)?,
            read_u32(params + 20)?,
        )
    };
    let mut w = Writer::new();
    w.u32(memid)
        .u32(lcid)
        .u16(flags)
        .u8(u8::from(result_out != 0))
        .u32(cargs)
        .u32(cnamed);
    for k in 0..cnamed as u64 {
        w.u32(read_u32(named + 4 * k)?);
    }
    for k in 0..cargs as u64 {
        let v = rgvarg + 24 * k;
        w.u8(u8::from(read_u16(v)? & vt::BYREF != 0));
        put_variant(&mut w, v, 0)?;
    }
    let reply = send_receive(channel, &p.iid, 6, &w.bytes)?;
    let mut r = Reader::new(&reply);
    let code = r.u32().map_err(bad)?;
    if r.u8().map_err(bad)? != 0 {
        let mut tmp = [0u8; 24];
        take_variant(&mut r, tmp.as_mut_ptr() as u64)?;
        if result_out != 0 {
            write(result_out, &tmp)?;
        } else {
            let _ = clear(tmp.as_mut_ptr() as u64);
        }
    }
    if r.u8().map_err(bad)? != 0 {
        let code16 = r.u16().map_err(bad)?;
        let texts: [Option<Vec<u16>>; 3] = [
            r.bstr().map_err(bad)?,
            r.bstr().map_err(bad)?,
            r.bstr().map_err(bad)?,
        ];
        let context = r.u32().map_err(bad)?;
        let scode = r.u32().map_err(bad)?;
        if excep != 0 {
            write(excep, &[0u8; 64])?;
            write_u16(excep, code16)?;
            for (i, t) in texts.iter().enumerate() {
                if let Some(u) = t {
                    write_u64(excep + 8 + 8 * i as u64, bstr_from(u))?;
                }
            }
            write_u32(excep + 32, context)?;
            write_u32(excep + 56, scode)?;
        }
    }
    let err = r.u32().map_err(bad)?;
    if argerr != 0 {
        write_u32(argerr, err)?;
    }
    // By-reference arguments: their new values into the caller's variables.
    for k in 0..cargs as u64 {
        let v = rgvarg + 24 * k;
        let kind = read_u16(v)?;
        if kind & vt::BYREF == 0 {
            continue;
        }
        let mut fresh = [0u8; 24];
        take_variant(&mut r, fresh.as_mut_ptr() as u64)?;
        let target = read_u64(v + 8)?;
        let base = kind & !vt::BYREF;
        if base == vt::VARIANT {
            let _ = clear(target);
            write(target, &fresh)?;
            continue;
        }
        let mut converted = [0u8; 24];
        let f = fresh.as_mut_ptr() as u64;
        let c = converted.as_mut_ptr() as u64;
        let ok = change(c, f, 0, base).is_ok();
        let _ = clear(f);
        if !ok {
            continue;
        }
        let n = element_native(base & !vt::ARRAY).ok();
        if base & vt::ARRAY != 0 {
            if let Ok(old) = read_u64(target)
                && old != 0
            {
                safearray::SafeArrayDestroy(door_windows_abi::narrow(old));
            }
            write(target, &converted[8..16])?;
        } else if let Some(n) = n {
            if !matches!(n, Native::Scalar(..)) {
                drop_value(n, target);
            }
            if base == vt::DECIMAL {
                write(target, &converted[..16])?;
            } else {
                write(target, &converted[8..8 + native_bytes(n).min(8)])?;
            }
        }
    }
    Ok(code)
}

// ---- stubs ------------------------------------------------------------------------------------
#[repr(C)]
struct StubVtbl {
    query: extern "system" fn(usize, usize, usize) -> u32,
    add_ref: extern "system" fn(usize) -> u32,
    release: extern "system" fn(usize) -> u32,
    connect: extern "system" fn(usize, usize) -> u32,
    disconnect: extern "system" fn(usize) -> u32,
    invoke: extern "system" fn(usize, usize, usize) -> u32,
    is_iid_supported: extern "system" fn(usize, usize) -> usize,
    count_refs: extern "system" fn(usize) -> u32,
    debug_query: extern "system" fn(usize, usize) -> u32,
    debug_release: extern "system" fn(usize, usize),
}
static STUB_VTBL: StubVtbl = StubVtbl {
    query: stub_query,
    add_ref: stub_add_ref,
    release: stub_release,
    connect: stub_connect,
    disconnect: stub_disconnect,
    invoke: stub_invoke,
    is_iid_supported: stub_supports,
    count_refs: stub_count,
    debug_query: stub_debug_query,
    debug_release: stub_debug_release,
};
#[repr(C)]
struct Stub {
    vtbl: &'static StubVtbl,
    references: AtomicU32,
    server: AtomicU64,
    iid: [u8; 16],
    methods: Methods,
}
fn stub(this: u64) -> &'static Stub {
    // SAFETY: STUB_VTBL's methods are only called on Stubs.
    unsafe { &*(this as *const Stub) }
}
extern "system" fn create_stub(_this: usize, iid: usize, server: usize, out: usize) -> u32 {
    let iid = door_windows_abi::widen(iid);
    let server = door_windows_abi::widen(server);
    let out = door_windows_abi::widen(out);
    {
        let _ = write_pointer(out, 0);
        let Ok(g) = read_n::<16>(iid) else {
            return hr::E_INVALIDARG;
        };
        let methods = match Methods::of(&g) {
            Ok(m) => m,
            Err(e) => return e,
        };
        let s = Box::into_raw(Box::new(Stub {
            vtbl: &STUB_VTBL,
            references: AtomicU32::new(1),
            server: AtomicU64::new(0),
            iid: g,
            methods,
        })) as u64;
        if server != 0 {
            stub_connect(
                door_windows_abi::narrow(s),
                door_windows_abi::narrow(server),
            );
        }
        result(write_pointer(out, s))
    }
}
extern "system" fn stub_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        match read_n::<16>(iid) {
            Ok(g) if g == IID_IUNKNOWN || g == IID_IRPCSTUBBUFFER => {
                stub_add_ref(door_windows_abi::narrow(this));
                result(write_pointer(out, this))
            }
            _ => {
                let _ = write_pointer(out, 0);
                hr::E_NOINTERFACE
            }
        }
    }
}
extern "system" fn stub_add_ref(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    stub(this).references.fetch_add(1, Ordering::AcqRel) + 1
}
extern "system" fn stub_release(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        let left = stub(this).references.fetch_sub(1, Ordering::AcqRel) - 1;
        if left == 0 {
            stub_disconnect(door_windows_abi::narrow(this));
            // SAFETY: the last reference; the stub came from Box::into_raw.
            drop(unsafe { Box::from_raw(this as *mut Stub) });
        }
        left
    }
}
/// Connect: the server's interface (`iid`, queried from the object).
extern "system" fn stub_connect(this: usize, server: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let server = door_windows_abi::widen(server);
    {
        let s = stub(this);
        let mut p = 0u64;
        // SAFETY: `server` is the object's IUnknown.
        let query: extern "system" fn(usize, usize, usize) -> u32 =
            unsafe { core::mem::transmute(method(server, 0) as usize) };
        let r = query(
            door_windows_abi::narrow(server),
            door_windows_abi::narrow(s.iid.as_ptr() as u64),
            door_windows_abi::narrow(&mut p as *mut u64 as u64),
        );
        if r != hr::S_OK {
            return r;
        }
        let old = s.server.swap(p, Ordering::AcqRel);
        if old != 0 {
            release(old);
        }
        hr::S_OK
    }
}
extern "system" fn stub_disconnect(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        let old = stub(this).server.swap(0, Ordering::AcqRel);
        if old != 0 {
            release(old);
        }
        hr::S_OK
    }
}
extern "system" fn stub_supports(this: usize, iid: usize) -> usize {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    door_windows_abi::narrow((move || -> u64 {
        match read_n::<16>(iid) {
            Ok(g) if g == stub(this).iid => this,
            _ => 0,
        }
    })())
}
extern "system" fn stub_count(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    u32::from(stub(this).server.load(Ordering::Acquire) != 0)
}
extern "system" fn stub_debug_query(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        let p = stub(this).server.load(Ordering::Acquire);
        add_ref(p);
        result(write_pointer(out, p))
    }
}
extern "system" fn stub_debug_release(_this: usize, pointer: usize) {
    let pointer = door_windows_abi::widen(pointer);
    {
        release(pointer);
    }
}
/// IRpcStubBuffer::Invoke: the request in the message, the reply into a
/// buffer the channel gives.
extern "system" fn stub_invoke(this: usize, msg: usize, channel: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let msg = door_windows_abi::widen(msg);
    let channel = door_windows_abi::widen(channel);
    {
        let s = stub(this);
        let server = s.server.load(Ordering::Acquire);
        if server == 0 {
            return CO_E_OBJNOTCONNECTED;
        }
        let r = (|| -> Result<Vec<u8>, u32> {
            let slot = read_u32(msg + 28)? as usize;
            let n = read_u32(msg + 24)? as usize;
            let mut request = vec![0u8; n];
            read(read_u64(msg + 16)?, &mut request)?;
            if s.methods.dispatch && (3..7).contains(&slot) {
                dispatch_stub(server, slot, &request)
            } else if let Some((home, f)) = s.methods.typed(slot) {
                typed_stub(server, slot, home, f, &request)
            } else {
                Err(hr::E_NOTIMPL)
            }
        })();
        let reply = match r {
            Ok(b) => b,
            Err(e) => return e,
        };
        let out = (|| {
            write_u32(msg + 24, reply.len() as u32)?;
            // SAFETY: IRpcChannelBuffer::GetBuffer is method 3.
            let get: extern "system" fn(usize, usize, usize) -> u32 =
                unsafe { core::mem::transmute(method(channel, 3) as usize) };
            let r = get(
                door_windows_abi::narrow(channel),
                door_windows_abi::narrow(msg),
                door_windows_abi::narrow(s.iid.as_ptr() as u64),
            );
            if r != hr::S_OK {
                return Err(r);
            }
            write(read_u64(msg + 16)?, &reply)
        })();
        result(out)
    }
}
/// Calls slot `slot` of `server` with the decoded arguments; replies the
/// HRESULT and the [out] values.
fn typed_stub(
    server: u64,
    slot: usize,
    home: usize,
    f: &Func,
    request: &[u8],
) -> Result<Vec<u8>, u32> {
    let mut r = Reader::new(request);
    let mut cells: Vec<Box<[u8; 24]>> = Vec::with_capacity(f.params.len());
    let mut shapes = Vec::with_capacity(f.params.len());
    let mut slots = vec![super::dispatch::Arg::Word(server)];
    let decoded = (|| -> Result<(), u32> {
        for param in &f.params {
            let cell = Box::new([0u8; 24]);
            let at = cell.as_ptr() as u64;
            cells.push(cell);
            if param.flags & paramflag::LCID != 0 {
                shapes.push(None);
                slots.push(super::dispatch::Arg::Word(0x409));
                continue;
            }
            let (n, pointer) = param_shape(home, &param.ty)?;
            shapes.push(Some(n));
            let input = param.flags & paramflag::IN != 0
                || param.flags & (paramflag::IN | paramflag::OUT) == 0;
            if input {
                take(&mut r, n, at)?;
            }
            slots.push(if pointer {
                super::dispatch::Arg::Word(at)
            } else if native_bytes(n) > 8 {
                super::dispatch::Arg::Struct(at)
            } else if native_bytes(n) == 8 {
                super::dispatch::Arg::Wide(read_u64(at)?)
            } else {
                super::dispatch::Arg::Word(read_u64(at)?)
            });
        }
        Ok(())
    })();
    let release_all = |cells: &[Box<[u8; 24]>], shapes: &[Option<Native>]| {
        for (cell, n) in cells.iter().zip(shapes) {
            if let Some(n) = n {
                drop_value(*n, cell.as_ptr() as u64);
            }
        }
    };
    if let Err(e) = decoded {
        release_all(&cells, &shapes);
        return Err(e);
    }
    let function = read_pointer(read_pointer(server)? + (POINTER * slot) as u64)?;
    let (rax, _) = call_raw(function, &slots, 0);
    let mut w = Writer::new();
    w.u32(rax as u32);
    for (k, param) in f.params.iter().enumerate() {
        if param.flags & paramflag::OUT == 0 {
            continue;
        }
        if let Some(n) = shapes[k] {
            put(&mut w, n, cells[k].as_ptr() as u64)?;
        }
    }
    release_all(&cells, &shapes);
    Ok(w.bytes)
}
/// IDispatch's methods on the server.
fn dispatch_stub(server: u64, slot: usize, request: &[u8]) -> Result<Vec<u8>, u32> {
    let mut r = Reader::new(request);
    let mut w = Writer::new();
    let iid_null = [0u8; 16];
    match slot {
        3 => {
            let mut n = 0u32;
            // SAFETY: IDispatch::GetTypeInfoCount is method 3.
            let f: extern "system" fn(usize, usize) -> u32 =
                unsafe { core::mem::transmute(method(server, 3) as usize) };
            let code = f(
                door_windows_abi::narrow(server),
                door_windows_abi::narrow(&mut n as *mut u32 as u64),
            );
            w.u32(code).u32(n);
        }
        4 => {
            let (index, lcid) = (r.u32().map_err(bad)?, r.u32().map_err(bad)?);
            let mut ti = 0u64;
            // SAFETY: IDispatch::GetTypeInfo is method 4.
            let f: extern "system" fn(usize, u32, u32, usize) -> u32 =
                unsafe { core::mem::transmute(method(server, 4) as usize) };
            let code = f(
                door_windows_abi::narrow(server),
                index,
                lcid,
                door_windows_abi::narrow(&mut ti as *mut u64 as u64),
            );
            if code != hr::S_OK || ti == 0 {
                w.u32(if code == hr::S_OK { hr::E_FAIL } else { code });
                return Ok(w.bytes);
            }
            let described = describe(ti);
            release(ti);
            match described {
                Some((libid, major, minor, lcid, guid)) => {
                    w.u32(hr::S_OK)
                        .raw(&libid)
                        .u16(major)
                        .u16(minor)
                        .u32(lcid)
                        .raw(&guid);
                }
                None => {
                    w.u32(hr::E_NOTIMPL);
                }
            }
        }
        5 => {
            let count = r.u32().map_err(bad)?;
            let lcid = r.u32().map_err(bad)?;
            if count > 64 {
                return Err(RPC_E_INVALID_DATA);
            }
            let mut names: Vec<Vec<u16>> = Vec::new();
            for _ in 0..count {
                let mut n = r.bstr().map_err(bad)?.unwrap_or_default();
                n.push(0);
                names.push(n);
            }
            let pointers: Vec<u64> = names.iter().map(|n| n.as_ptr() as u64).collect();
            let mut ids = vec![-1i32; count as usize];
            // SAFETY: IDispatch::GetIDsOfNames is method 5.
            let f: extern "system" fn(usize, usize, usize, u32, u32, usize) -> u32 =
                unsafe { core::mem::transmute(method(server, 5) as usize) };
            let code = f(
                door_windows_abi::narrow(server),
                door_windows_abi::narrow(iid_null.as_ptr() as u64),
                door_windows_abi::narrow(pointers.as_ptr() as u64),
                count,
                lcid,
                door_windows_abi::narrow(ids.as_mut_ptr() as u64),
            );
            w.u32(code);
            for id in ids {
                w.u32(id as u32);
            }
        }
        _ => return invoke_stub(server, &mut r),
    }
    Ok(w.bytes)
}
/// An ITypeInfo's library (GUID, version, LCID) and its own GUID.
type Described = ([u8; 16], u16, u16, u32, [u8; 16]);
fn describe(ti: u64) -> Option<Described> {
    let mut tl = 0u64;
    let mut index = 0u32;
    // SAFETY: ITypeInfo::GetContainingTypeLib (18), GetTypeAttr (3),
    // ReleaseTypeAttr (19); ITypeLib::GetLibAttr (7), ReleaseTLibAttr (12).
    let containing: extern "system" fn(usize, usize, usize) -> u32 =
        unsafe { core::mem::transmute(method(ti, 18) as usize) };
    if containing(
        door_windows_abi::narrow(ti),
        door_windows_abi::narrow(&mut tl as *mut u64 as u64),
        door_windows_abi::narrow(&mut index as *mut u32 as u64),
    ) != hr::S_OK
    {
        return None;
    }
    let mut la = 0u64;
    let attr: extern "system" fn(usize, usize) -> u32 =
        unsafe { core::mem::transmute(method(tl, 7) as usize) };
    let ok = attr(
        door_windows_abi::narrow(tl),
        door_windows_abi::narrow(&mut la as *mut u64 as u64),
    ) == hr::S_OK;
    let lib = ok.then(|| {
        (
            read_n::<16>(la).ok(),
            read_u32(la + 16).ok(),
            read_u16(la + 24).ok(),
            read_u16(la + 26).ok(),
        )
    });
    if ok {
        let rel: extern "system" fn(usize, usize) =
            unsafe { core::mem::transmute(method(tl, 12) as usize) };
        rel(door_windows_abi::narrow(tl), door_windows_abi::narrow(la));
    }
    release(tl);
    let mut ta = 0u64;
    let attr: extern "system" fn(usize, usize) -> u32 =
        unsafe { core::mem::transmute(method(ti, 3) as usize) };
    if attr(
        door_windows_abi::narrow(ti),
        door_windows_abi::narrow(&mut ta as *mut u64 as u64),
    ) != hr::S_OK
    {
        return None;
    }
    let guid = read_n::<16>(ta).ok();
    let rel: extern "system" fn(usize, usize) =
        unsafe { core::mem::transmute(method(ti, 19) as usize) };
    rel(door_windows_abi::narrow(ti), door_windows_abi::narrow(ta));
    let (Some(libid), Some(lcid), Some(major), Some(minor)) = lib? else {
        return None;
    };
    Some((libid, major, minor, lcid, guid?))
}
/// IDispatch::Invoke on the server: DISPPARAMS from the request (by
/// reference arguments pointing at cells here), then the result, EXCEPINFO,
/// the argument error and the by-reference values back.
fn invoke_stub(server: u64, r: &mut Reader) -> Result<Vec<u8>, u32> {
    let memid = r.u32().map_err(bad)?;
    let lcid = r.u32().map_err(bad)?;
    let flags = r.u16().map_err(bad)?;
    let want = r.u8().map_err(bad)? != 0;
    let cargs = r.u32().map_err(bad)? as usize;
    let cnamed = r.u32().map_err(bad)? as usize;
    if cargs > 256 || cnamed > cargs {
        return Err(RPC_E_INVALID_DATA);
    }
    let mut named = Vec::with_capacity(cnamed);
    for _ in 0..cnamed {
        named.push(r.u32().map_err(bad)?);
    }
    // rgvarg and, for by-reference arguments, the referents.
    let mut args = vec![[0u8; 24]; cargs];
    let mut cells = vec![[0u8; 24]; cargs];
    let mut byref = vec![false; cargs];
    for k in 0..cargs {
        byref[k] = r.u8().map_err(bad)? != 0;
        take_variant(r, cells[k].as_mut_ptr() as u64)?;
    }
    for k in 0..cargs {
        if byref[k] {
            // A VARIANT referent: rgvarg[k] is VT_BYREF|VT_VARIANT to it.
            let kind = vt::BYREF | vt::VARIANT;
            args[k][..2].copy_from_slice(&kind.to_le_bytes());
            args[k][8..16].copy_from_slice(&(cells[k].as_ptr() as u64).to_le_bytes());
        } else {
            args[k] = cells[k];
        }
    }
    let mut params = [0u8; 24];
    params[..8].copy_from_slice(&(args.as_ptr() as u64).to_le_bytes());
    params[8..16].copy_from_slice(&(named.as_ptr() as u64).to_le_bytes());
    params[16..20].copy_from_slice(&(cargs as u32).to_le_bytes());
    params[20..24].copy_from_slice(&(cnamed as u32).to_le_bytes());
    let mut result_v = [0u8; 24];
    let mut excep = [0u8; 64];
    let mut argerr = 0u32;
    let iid_null = [0u8; 16];
    // SAFETY: IDispatch::Invoke is method 6.
    let f: extern "system" fn(usize, u32, usize, u32, u16, usize, usize, usize, usize) -> u32 =
        unsafe { core::mem::transmute(method(server, 6) as usize) };
    let code = f(
        door_windows_abi::narrow(server),
        memid,
        door_windows_abi::narrow(iid_null.as_ptr() as u64),
        lcid,
        flags,
        door_windows_abi::narrow(params.as_ptr() as u64),
        if want {
            door_windows_abi::narrow(result_v.as_mut_ptr() as u64)
        } else {
            0
        },
        door_windows_abi::narrow(excep.as_mut_ptr() as u64),
        door_windows_abi::narrow(&mut argerr as *mut u32 as u64),
    );
    let mut w = Writer::new();
    w.u32(code);
    if want {
        w.u8(1);
        put_variant(&mut w, result_v.as_ptr() as u64, 0)?;
        let _ = clear(result_v.as_mut_ptr() as u64);
    } else {
        w.u8(0);
    }
    if code == hr::DISP_E_EXCEPTION {
        let text = |at: usize| -> Option<Vec<u16>> {
            let b = u64::from_le_bytes(excep[at..at + 8].try_into().ok()?);
            if b == 0 {
                return None;
            }
            let t = bstr_units(b).ok();
            bstr_free(b);
            t
        };
        w.u8(1).u16(u16::from_le_bytes([excep[0], excep[1]]));
        let (a, b, c) = (text(8), text(16), text(24));
        w.bstr(a.as_deref()).bstr(b.as_deref()).bstr(c.as_deref());
        w.u32(u32::from_le_bytes(
            excep[32..36].try_into().unwrap_or([0; 4]),
        ));
        w.u32(u32::from_le_bytes(
            excep[56..60].try_into().unwrap_or([0; 4]),
        ));
    } else {
        w.u8(0);
    }
    w.u32(argerr);
    for k in 0..cargs {
        if byref[k] {
            put_variant(&mut w, cells[k].as_ptr() as u64, 0)?;
        }
    }
    for c in cells.iter_mut() {
        let _ = clear(c.as_mut_ptr() as u64);
    }
    Ok(w.bytes)
}
