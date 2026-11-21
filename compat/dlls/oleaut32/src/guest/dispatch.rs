//! IDispatch over type information: ITypeInfo::Invoke (arguments from
//! DISPPARAMS coerced to each parameter's type, defaults and missing
//! optional VARIANTs, [retval] into the result, a failed HRESULT into
//! EXCEPINFO from the thread's error info), DispCallFunc's x64 call,
//! DispInvoke, DispGetIDsOfNames, DispGetParam and CreateStdDispatch.
//! Behaviour measured on Windows 11 26H2 (tests/reference/oleautprobe.txt).
use super::typeinfo::{self, IID_IDISPATCH, IID_IUNKNOWN};
/// The program's pointer width, for offsets (P17).
const P: u64 = POINTER as u64;
use super::*;
use crate::typelib::{Linker, Side, TypeDesc, paramflag, tkind};
use alloc::boxed::Box;
use core::sync::atomic::AtomicU32;

/// One argument of a call through `call_raw` (P17): a value in one stack
/// word or register, an 8-byte value (two stack words on x86), or a
/// 16-byte VARIANT/DECIMAL passed by value (a pointer to a copy on x64,
/// its four words on x86's stack).
#[derive(Clone, Copy)]
pub(super) enum Arg {
    Word(u64),
    Wide(u64),
    Struct(u64),
}
impl Arg {
    /// The argument a VARIANT-typed value of `vt` (not BYREF) is passed
    /// as: `value` is the scalar, or the copy's address for a VARIANT or
    /// DECIMAL.
    pub(super) fn of(kind: u16, value: u64) -> Arg {
        match kind {
            vt::VARIANT | vt::DECIMAL => Arg::Struct(value),
            vt::R8 | vt::DATE | vt::CY | vt::I8 | vt::UI8 => Arg::Wide(value),
            _ => Arg::Word(value),
        }
    }
}

// Calls `f` with the x64 convention: the first four arguments in
// RCX/RDX/R8/R9 (and XMM0-3, so floating-point ones arrive too), the rest
// on the stack above the 32-byte home area. Returns RAX; XMM0 goes to the
// fourth argument's location.
#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    ".globl door_oleaut_call",
    "door_oleaut_call:",
    "push rbp",
    "mov rbp, rsp",
    "push rsi",
    "push rdi",
    "push rbx",
    "push r12",
    "mov r10, rcx",
    "mov rsi, rdx",
    "mov rbx, r8",
    "mov r12, r9",
    "mov rax, rbx",
    "cmp rax, 4",
    "jae 2f",
    "mov rax, 4",
    "2:",
    "shl rax, 3",
    "add rax, 15",
    "and rax, -16",
    "sub rsp, rax",
    "mov rcx, 4",
    "3:",
    "cmp rcx, rbx",
    "jae 4f",
    "mov rax, [rsi + rcx*8]",
    "mov [rsp + rcx*8], rax",
    "inc rcx",
    "jmp 3b",
    "4:",
    "mov rcx, [rsi]",
    "mov rdx, [rsi + 8]",
    "mov r8, [rsi + 16]",
    "mov r9, [rsi + 24]",
    "movq xmm0, rcx",
    "movq xmm1, rdx",
    "movq xmm2, r8",
    "movq xmm3, r9",
    "call r10",
    "movq [r12], xmm0",
    "lea rsp, [rbp - 32]",
    "pop r12",
    "pop rbx",
    "pop rdi",
    "pop rsi",
    "pop rbp",
    "ret",
);
#[cfg(target_arch = "x86_64")]
unsafe extern "win64" {
    fn door_oleaut_call(f: u64, args: *const u64, count: u64, xmm0: *mut u64) -> u64;
}
/// RAX and XMM0 after calling `f` with `args` (`float`: the bytes of a
/// floating-point result, 4 or 8, else 0; x64 has it in XMM0 either way).
#[cfg(target_arch = "x86_64")]
pub(super) fn call_raw(f: u64, args: &[Arg], _float: u8) -> (u64, u64) {
    let mut a: Vec<u64> = args
        .iter()
        .map(|a| match *a {
            Arg::Word(v) | Arg::Wide(v) | Arg::Struct(v) => v,
        })
        .collect();
    while a.len() < 4 {
        a.push(0);
    }
    let mut xmm = 0u64;
    let rax = unsafe { door_oleaut_call(f, a.as_ptr(), a.len() as u64, &mut xmm) };
    (rax, xmm)
}
/// x86 (P17): pushes `count` words from `words` (the last first), calls
/// `f` (stdcall or cdecl: the stack is restored from EBP), stores EDX:EAX
/// at `out` and, when `float` is not null, ST0 there as a double.
#[cfg(target_arch = "x86")]
#[unsafe(naked)]
unsafe extern "C" fn door_oleaut_call32(
    f: u32,
    words: *const u32,
    count: u32,
    out: *mut u64,
    float: *mut f64,
) {
    core::arch::naked_asm!(
        "push ebp",
        "mov ebp, esp",
        "push esi",
        "push edi",
        "push ebx",
        "mov esi, [ebp + 12]",
        "mov ecx, [ebp + 16]",
        "lea esi, [esi + ecx*4]",
        "2:",
        "test ecx, ecx",
        "jz 3f",
        "sub esi, 4",
        "push dword ptr [esi]",
        "dec ecx",
        "jmp 2b",
        "3:",
        "call dword ptr [ebp + 8]",
        "lea esp, [ebp - 12]",
        "mov ecx, [ebp + 20]",
        "mov [ecx], eax",
        "mov [ecx + 4], edx",
        "mov ecx, [ebp + 24]",
        "test ecx, ecx",
        "jz 4f",
        "fstp qword ptr [ecx]",
        "4:",
        "pop ebx",
        "pop edi",
        "pop esi",
        "pop ebp",
        "ret",
    )
}
/// EDX:EAX and the floating-point result after calling `f` with `args`
/// as a 32-bit program's stack holds them (P17): `float` 4 or 8 reads ST0
/// (an R4's bits come back as the low 32 bits, as XMM0 holds them on x64).
#[cfg(target_arch = "x86")]
pub(super) fn call_raw(f: u64, args: &[Arg], float: u8) -> (u64, u64) {
    let mut words: Vec<u32> = Vec::with_capacity(args.len() * 2);
    for a in args {
        match *a {
            Arg::Word(v) => words.push(v as u32),
            Arg::Wide(v) => {
                words.push(v as u32);
                words.push((v >> 32) as u32);
            }
            Arg::Struct(p) => {
                let b: [u8; 16] = read_n(p).unwrap_or([0; 16]);
                for c in b.chunks_exact(4) {
                    words.push(u32::from_le_bytes(c.try_into().unwrap()));
                }
            }
        }
    }
    let mut out = 0u64;
    let mut st0 = 0f64;
    let fp = if float != 0 {
        &mut st0 as *mut f64
    } else {
        core::ptr::null_mut()
    };
    // SAFETY: `f` is the function the type information or the caller names;
    // the words are its arguments.
    unsafe { door_oleaut_call32(f as u32, words.as_ptr(), words.len() as u32, &mut out, fp) };
    let bits = match float {
        4 => u64::from((st0 as f32).to_bits()),
        8 => st0.to_bits(),
        _ => 0,
    };
    (out, bits)
}
const DISPID_PROPERTYPUT: i32 = -3;
const DISPATCH_PUTS: u16 = 4 | 8;

/// What a call keeps alive until it returns: converted VARIANTs (cleared
/// after), by-value copies, interfaces queried for parameters.
#[derive(Default)]
// Boxed so each VARIANT keeps its address while the list grows.
#[allow(clippy::vec_box)]
struct Temps {
    variants: Vec<Box<[u8; 24]>>,
    copies: Vec<Box<[u8; 24]>>,
    interfaces: Vec<u64>,
}
impl Temps {
    fn variant(&mut self) -> u64 {
        self.variants.push(Box::new([0; 24]));
        self.variants.last().map_or(0, |b| b.as_ptr() as u64)
    }
    fn copy(&mut self, from: u64) -> Result<u64, u32> {
        let mut b = [0u8; 24];
        read(from, &mut b[..VARIANT_BYTES as usize])?;
        self.copies.push(Box::new(b));
        Ok(self.copies.last().map_or(0, |b| b.as_ptr() as u64))
    }
}
impl Drop for Temps {
    fn drop(&mut self) {
        for v in &self.variants {
            let _ = clear(v.as_ptr() as u64);
        }
        for &i in &self.interfaces {
            release(i);
        }
    }
}

/// A parameter type with aliases and enums resolved: what Invoke passes.
enum Shape {
    /// A VARIANT, or a scalar of this VARTYPE, by value.
    Value(u16),
    /// A pointer to a VARIANT or to a scalar of this VARTYPE.
    Pointer(u16),
    /// A SAFEARRAY of this element type (by value or through a pointer).
    Array(u16, bool),
    /// An interface pointer (its IID; dispatch-based or not).
    Interface([u8; 16]),
    /// A pointer to an interface pointer.
    InterfaceOut,
    Other,
}
fn user(home: usize, r: crate::typelib::Ref) -> Option<(&'static crate::typelib::Type, usize)> {
    let (lib, index) = typeinfo::Guest.resolve(home, r)?;
    Some((typeinfo::library(lib)?.types.get(index as usize)?, lib))
}
/// The VARTYPE a type stands for (enums are VT_I4, aliases their target).
fn base(home: usize, t: &TypeDesc, depth: u32) -> Option<u16> {
    match t {
        TypeDesc::Base(v) => Some(*v),
        TypeDesc::User(r) if depth < 16 => {
            let (ty, lib) = user(home, *r)?;
            match ty.kind {
                tkind::ENUM => Some(vt::I4),
                tkind::ALIAS => base(lib, ty.alias.as_ref()?, depth + 1),
                tkind::INTERFACE | tkind::COCLASS => Some(vt::UNKNOWN),
                tkind::DISPATCH => Some(vt::DISPATCH),
                _ => None,
            }
        }
        _ => None,
    }
}
fn shape(home: usize, t: &TypeDesc) -> Shape {
    let interface = |r: &crate::typelib::Ref| -> Option<([u8; 16], bool)> {
        let (ty, lib) = user(home, *r)?;
        match ty.kind {
            tkind::INTERFACE | tkind::DISPATCH => Some((ty.guid, ty.kind == tkind::DISPATCH)),
            tkind::ALIAS => match ty.alias.as_ref()? {
                TypeDesc::User(inner) => {
                    let (t2, _) = user(lib, *inner)?;
                    matches!(t2.kind, tkind::INTERFACE | tkind::DISPATCH)
                        .then_some((t2.guid, t2.kind == tkind::DISPATCH))
                }
                _ => None,
            },
            _ => None,
        }
    };
    match t {
        TypeDesc::SafeArray(e) => base(home, e, 0).map_or(Shape::Other, |v| Shape::Array(v, false)),
        TypeDesc::Ptr(inner) => match &**inner {
            TypeDesc::SafeArray(e) => {
                base(home, e, 0).map_or(Shape::Other, |v| Shape::Array(v, true))
            }
            TypeDesc::User(r) => match interface(r) {
                Some((iid, _)) => Shape::Interface(iid),
                None => base(home, inner, 0).map_or(Shape::Other, Shape::Pointer),
            },
            TypeDesc::Ptr(p) => match &**p {
                TypeDesc::User(r) => interface(r).map_or(Shape::Other, |_| Shape::InterfaceOut),
                _ => Shape::Other,
            },
            other => base(home, other, 0).map_or(Shape::Other, Shape::Pointer),
        },
        other => base(home, other, 0).map_or(Shape::Other, |v| match v {
            vt::UNKNOWN | vt::DISPATCH if matches!(other, TypeDesc::User(_)) => Shape::Other,
            v => Shape::Value(v),
        }),
    }
}
/// The argument slot for the VARIANT at `src` passed as a parameter of
/// type `t`.
fn argument(home: usize, t: &TypeDesc, src: u64, temps: &mut Temps) -> Result<u64, u32> {
    let kind = read_u16(src)?;
    let byref = kind & vt::BYREF != 0;
    match shape(home, t) {
        Shape::Value(vt::VARIANT) => temps.copy(src),
        Shape::Pointer(vt::VARIANT) => {
            if kind == vt::BYREF | vt::VARIANT {
                read_pointer(src + 8)
            } else {
                Ok(src)
            }
        }
        Shape::Value(v) => {
            let tmp = temps.variant();
            change(tmp, src, 0, v).map_err(|_| hr::DISP_E_TYPEMISMATCH)?;
            match v {
                vt::DECIMAL => Ok(tmp),
                vt::R4 => Ok(read_u32(tmp + 8)? as u64),
                _ => read_u64(tmp + 8),
            }
        }
        Shape::Pointer(v) => {
            if kind == vt::BYREF | v {
                read_pointer(src + 8)
            } else {
                Err(hr::DISP_E_TYPEMISMATCH)
            }
        }
        Shape::Array(e, pointer) => {
            if kind & !vt::BYREF != vt::ARRAY | e || (pointer && !byref) {
                return Err(hr::DISP_E_TYPEMISMATCH);
            }
            let at = read_pointer(src + 8)?;
            if pointer || !byref {
                Ok(at)
            } else {
                read_pointer(at)
            }
        }
        Shape::Interface(iid) => {
            let base = kind & !vt::BYREF;
            if base != vt::UNKNOWN && base != vt::DISPATCH {
                return Err(hr::DISP_E_TYPEMISMATCH);
            }
            let object = if byref {
                read_pointer(read_pointer(src + 8)?)?
            } else {
                read_pointer(src + 8)?
            };
            if object == 0 {
                return Ok(0);
            }
            let mut p = 0u64;
            // SAFETY: a VT_UNKNOWN/VT_DISPATCH holds a live interface.
            let query: extern "system" fn(usize, usize, usize) -> u32 =
                unsafe { core::mem::transmute(method(object, 0) as usize) };
            if query(
                door_windows_abi::narrow(object),
                door_windows_abi::narrow(iid.as_ptr() as u64),
                door_windows_abi::narrow(&mut p as *mut u64 as u64),
            ) != hr::S_OK
            {
                return Err(hr::DISP_E_TYPEMISMATCH);
            }
            temps.interfaces.push(p);
            Ok(p)
        }
        Shape::InterfaceOut => {
            if byref && matches!(kind & !vt::BYREF, vt::UNKNOWN | vt::DISPATCH) {
                read_pointer(src + 8)
            } else {
                Err(hr::DISP_E_TYPEMISMATCH)
            }
        }
        Shape::Other => Err(hr::DISP_E_TYPEMISMATCH),
    }
}
/// The VARIANT a [retval] cell of type `t` (the pointee) becomes.
fn retval_variant(home: usize, t: &TypeDesc, cell: &[u8; 24]) -> [u8; 24] {
    let mut out = [0u8; 24];
    let kind = match t {
        TypeDesc::SafeArray(e) => base(home, e, 0).map(|v| vt::ARRAY | v),
        other => base(home, other, 0),
    };
    match kind {
        Some(vt::VARIANT) => return *cell,
        Some(v) => {
            out[..2].copy_from_slice(&v.to_le_bytes());
            if v == vt::DECIMAL {
                out[2..16].copy_from_slice(&cell[2..16]);
            } else {
                out[8..16].copy_from_slice(&cell[..8]);
            }
        }
        None => {}
    }
    out
}

/// ITypeInfo::Invoke.
#[allow(clippy::too_many_arguments)]
pub(super) extern "system" fn type_info_invoke(
    this: usize,
    instance: usize,
    memid: usize,
    flags: usize,
    params: usize,
    result_out: usize,
    excep: usize,
    argerr: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let instance = door_windows_abi::widen(instance);
    let memid = door_windows_abi::widen(memid);
    let flags = door_windows_abi::widen(flags);
    let params = door_windows_abi::widen(params);
    let result_out = door_windows_abi::widen(result_out);
    let excep = door_windows_abi::widen(excep);
    let argerr = door_windows_abi::widen(argerr);
    {
        let Some(ti) = typeinfo::ours(this) else {
            return hr::E_INVALIDARG;
        };
        if instance == 0 {
            return hr::E_INVALIDARG;
        }
        let (memid, flags) = (memid as u32 as i32, flags as u16);
        // A dual's dispatch side calls through its vtable side.
        let mut vtable = 0;
        let view = if ti.view.dual {
            vtable = match typeinfo::type_info(ti.lib, ti.index, Side::Vtable) {
                Ok(p) => p,
                Err(e) => return e,
            };
            release(vtable);
            match typeinfo::ours(vtable) {
                Some(v) => &v.view,
                None => return hr::E_UNEXPECTED,
            }
        } else {
            &ti.view
        };
        if view.kind == tkind::DISPATCH {
            // A dispinterface: the object's own IDispatch::Invoke.
            let iid_null = [0u8; 16];
            // SAFETY: the caller passes an object implementing the dispinterface.
            let f: extern "system" fn(
                usize,
                i32,
                usize,
                u32,
                u16,
                usize,
                usize,
                usize,
                usize,
            ) -> u32 = unsafe { core::mem::transmute(method(instance, 6) as usize) };
            return f(
                door_windows_abi::narrow(instance),
                memid,
                door_windows_abi::narrow(iid_null.as_ptr() as u64),
                0x409,
                flags,
                door_windows_abi::narrow(params),
                door_windows_abi::narrow(result_out),
                door_windows_abi::narrow(excep),
                door_windows_abi::narrow(argerr),
            );
        }
        let Some((home, f)) = view
            .funcs
            .iter()
            .find(|(_, f)| f.memid == memid && f.invoke as u16 & flags != 0)
        else {
            // A member of the interface this one derives from: its typeinfo's.
            let side = if ti.view.dual { vtable } else { this };
            if view.kind == tkind::INTERFACE
                && let Some(base) = typeinfo::base_info(side)
            {
                let r = type_info_invoke(
                    door_windows_abi::narrow(base),
                    door_windows_abi::narrow(instance),
                    door_windows_abi::narrow(memid as u32 as u64),
                    door_windows_abi::narrow(u64::from(flags)),
                    door_windows_abi::narrow(params),
                    door_windows_abi::narrow(result_out),
                    door_windows_abi::narrow(excep),
                    door_windows_abi::narrow(argerr),
                );
                release(base);
                return r;
            }
            return hr::DISP_E_MEMBERNOTFOUND;
        };
        let home = *home;
        match invoke_func(home, f, instance, flags, params, result_out, excep) {
            Ok(()) => hr::S_OK,
            Err((e, arg)) => {
                if let Some(a) = arg
                    && argerr != 0
                {
                    let _ = write_u32(argerr, a);
                }
                e
            }
        }
    }
}
type Failure = (u32, Option<u32>);
fn invoke_func(
    home: usize,
    f: &crate::typelib::Func,
    instance: u64,
    flags: u16,
    params: u64,
    result_out: u64,
    excep: u64,
) -> Result<(), Failure> {
    let plain = |e: u32| (e, None);
    let (rgvarg, named, cargs, cnamed) = if params == 0 {
        (0, 0, 0, 0)
    } else {
        (
            read_pointer(params).map_err(plain)?,
            read_pointer(params + P).map_err(plain)?,
            read_u32(params + 2 * P).map_err(plain)?,
            read_u32(params + 2 * P + 4).map_err(plain)?,
        )
    };
    if cnamed > cargs {
        return Err(plain(hr::E_INVALIDARG));
    }
    let visible: Vec<usize> = f
        .params
        .iter()
        .enumerate()
        .filter(|(_, p)| p.flags & (paramflag::RETVAL | paramflag::LCID) == 0)
        .map(|(i, _)| i)
        .collect();
    let mut source: Vec<Option<u32>> = vec![None; f.params.len()];
    let names: Vec<i32> = (0..cnamed as u64)
        .map(|k| read_u32(named + 4 * k).map(|d| d as i32))
        .collect::<Result<_, _>>()
        .map_err(plain)?;
    let mut assignable = visible.clone();
    if flags & DISPATCH_PUTS != 0 {
        let k = names
            .iter()
            .position(|&d| d == DISPID_PROPERTYPUT)
            .ok_or(plain(hr::DISP_E_PARAMNOTFOUND))?;
        let last = assignable.pop().ok_or(plain(hr::DISP_E_BADPARAMCOUNT))?;
        source[last] = Some(k as u32);
    }
    let positional = cargs - cnamed;
    if positional as usize > assignable.len() {
        return Err(plain(hr::DISP_E_BADPARAMCOUNT));
    }
    for i in 0..positional {
        source[assignable[i as usize]] = Some(cargs - 1 - i);
    }
    for (k, &d) in names.iter().enumerate() {
        if d == DISPID_PROPERTYPUT {
            continue;
        }
        match usize::try_from(d).ok().filter(|i| visible.contains(i)) {
            Some(i) => source[i] = Some(k as u32),
            None => return Err((hr::DISP_E_PARAMNOTFOUND, Some(k as u32))),
        }
    }
    for &i in &visible {
        if source[i].is_none() && f.params[i].flags & (paramflag::OPT | paramflag::HASDEFAULT) == 0
        {
            return Err(plain(hr::DISP_E_BADPARAMCOUNT));
        }
    }
    let mut temps = Temps::default();
    let mut slots = vec![Arg::Word(instance)];
    let mut retval: Option<(Box<[u8; 24]>, &TypeDesc)> = None;
    for (i, p) in f.params.iter().enumerate() {
        // How the parameter's value is passed (x86: its stack words).
        let pass = |value: u64| match shape(home, &p.ty) {
            Shape::Value(v) => Arg::of(v, value),
            _ => Arg::Word(value),
        };
        if p.flags & paramflag::RETVAL != 0 {
            let cell = Box::new([0u8; 24]);
            slots.push(Arg::Word(cell.as_ptr() as u64));
            let pointee = match &p.ty {
                TypeDesc::Ptr(inner) => &**inner,
                other => other,
            };
            retval = Some((cell, pointee));
        } else if p.flags & paramflag::LCID != 0 {
            slots.push(Arg::Word(0x409));
        } else if let Some(idx) = source[i] {
            let src = rgvarg + VARIANT_BYTES * idx as u64;
            slots.push(pass(
                argument(home, &p.ty, src, &mut temps).map_err(|e| (e, Some(idx)))?,
            ));
        } else {
            // Missing: the default value, or an optional VARIANT's
            // VT_ERROR/DISP_E_PARAMNOTFOUND.
            let tmp = temps.variant();
            match &p.default {
                Some(v) if p.flags & paramflag::HASDEFAULT != 0 => {
                    write_value(tmp, v).map_err(plain)?
                }
                _ => write_value(tmp, &Value::Error(hr::DISP_E_PARAMNOTFOUND)).map_err(plain)?,
            }
            slots.push(pass(argument(home, &p.ty, tmp, &mut temps).map_err(plain)?));
        }
    }
    // The type information counts 8-byte slots; a 32-bit vtable's are 4.
    let function = read_pointer(instance)
        .and_then(|v| read_pointer(v + f.vft as u16 as u64))
        .map_err(plain)?;
    let ret_vt = base(home, &f.ret, 0).unwrap_or(vt::VOID);
    let float = match ret_vt {
        vt::R4 => 4,
        vt::R8 | vt::DATE => 8,
        _ => 0,
    };
    let (rax, xmm0) = call_raw(function, &slots, float);
    let mut value: Option<[u8; 24]> = None;
    if ret_vt == vt::HRESULT {
        let code = rax as u32;
        if code & 0x8000_0000 != 0 {
            fill_exception(excep, code);
            return Err(plain(hr::DISP_E_EXCEPTION));
        }
        if let Some((cell, t)) = &retval {
            value = Some(retval_variant(home, t, cell));
        }
    } else if ret_vt != vt::VOID {
        let mut v = [0u8; 24];
        v[..2].copy_from_slice(&ret_vt.to_le_bytes());
        let bits = if matches!(ret_vt, vt::R4 | vt::R8 | vt::DATE) {
            xmm0
        } else {
            rax
        };
        v[8..16].copy_from_slice(&bits.to_le_bytes());
        value = Some(v);
    }
    if let Some(mut v) = value {
        if result_out != 0 {
            write(result_out, &v).map_err(plain)?;
        } else {
            let _ = clear(v.as_mut_ptr() as u64);
        }
    }
    Ok(())
}
/// EXCEPINFO for a failed HRESULT, from the thread's error info.
fn fill_exception(excep: u64, code: u32) {
    if excep == 0 {
        return;
    }
    use door_windows_abi::layout::{self, Field::*};
    const EXCEPINFO: [layout::Field; 9] = [
        U16, U16, Pointer, Pointer, Pointer, U32, Pointer, Pointer, U32,
    ];
    let field = |index| excep + layout::offset(&EXCEPINFO, index) as u64;
    let _ = write(excep, &[0u8; layout::size(&EXCEPINFO)]);
    let mut e = 0u64;
    if super::errors::GetErrorInfo(0, door_windows_abi::narrow(&mut e as *mut u64 as u64))
        == hr::S_OK
        && e != 0
    {
        // IErrorInfo::GetSource, GetDescription, GetHelpFile.
        for (slot, at) in [(4usize, field(2)), (5, field(3)), (6, field(4))] {
            let mut s = 0u64;
            // SAFETY: GetErrorInfo gave an IErrorInfo.
            let get: extern "system" fn(usize, usize) -> u32 =
                unsafe { core::mem::transmute(method(e, slot) as usize) };
            if get(
                door_windows_abi::narrow(e),
                door_windows_abi::narrow(&mut s as *mut u64 as u64),
            ) == hr::S_OK
            {
                let _ = write_pointer(at, s);
            }
        }
        let mut context = 0u32;
        // SAFETY: as above.
        let get: extern "system" fn(usize, usize) -> u32 =
            unsafe { core::mem::transmute(method(e, 7) as usize) };
        if get(
            door_windows_abi::narrow(e),
            door_windows_abi::narrow(&mut context as *mut u32 as u64),
        ) == hr::S_OK
        {
            let _ = write_u32(field(5), context);
        }
        release(e);
    }
    let _ = write_u32(field(8), code);
}

#[unsafe(no_mangle)]
pub extern "system" fn DispInvoke(
    this: usize,
    info: usize,
    memid: i32,
    flags: u16,
    params: usize,
    result_out: usize,
    excep: usize,
    argerr: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let info = door_windows_abi::widen(info);
    let params = door_windows_abi::widen(params);
    let result_out = door_windows_abi::widen(result_out);
    let excep = door_windows_abi::widen(excep);
    let argerr = door_windows_abi::widen(argerr);
    {
        if info == 0 {
            return hr::E_INVALIDARG;
        }
        // SAFETY: `info` is an ITypeInfo; Invoke is method 11.
        let f: extern "system" fn(usize, usize, i32, u16, usize, usize, usize, usize) -> u32 =
            unsafe { core::mem::transmute(method(info, 11) as usize) };
        f(
            door_windows_abi::narrow(info),
            door_windows_abi::narrow(this),
            memid,
            flags,
            door_windows_abi::narrow(params),
            door_windows_abi::narrow(result_out),
            door_windows_abi::narrow(excep),
            door_windows_abi::narrow(argerr),
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DispGetIDsOfNames(info: usize, names: usize, count: u32, ids: usize) -> u32 {
    let info = door_windows_abi::widen(info);
    let names = door_windows_abi::widen(names);
    let ids = door_windows_abi::widen(ids);
    {
        if info == 0 {
            return hr::E_INVALIDARG;
        }
        // SAFETY: `info` is an ITypeInfo; GetIDsOfNames is method 10.
        let f: extern "system" fn(usize, usize, u32, usize) -> u32 =
            unsafe { core::mem::transmute(method(info, 10) as usize) };
        f(
            door_windows_abi::narrow(info),
            door_windows_abi::narrow(names),
            count,
            door_windows_abi::narrow(ids),
        )
    }
}
/// DispGetParam: argument `position` (positional, else named by that
/// DISPID) coerced to `target`.
#[unsafe(no_mangle)]
pub extern "system" fn DispGetParam(
    params: usize,
    position: u32,
    target: u16,
    out: usize,
    argerr: usize,
) -> u32 {
    let params = door_windows_abi::widen(params);
    let out = door_windows_abi::widen(out);
    let argerr = door_windows_abi::widen(argerr);
    {
        let r = (|| -> Result<(), (u32, Option<u32>)> {
            let p = |e| (e, None);
            let rgvarg = read_pointer(params).map_err(p)?;
            let named = read_pointer(params + P).map_err(p)?;
            let cargs = read_u32(params + 2 * P).map_err(p)?;
            let cnamed = read_u32(params + 2 * P + 4).map_err(p)?;
            let positional = cargs.saturating_sub(cnamed);
            let index = if position < positional {
                cargs - 1 - position
            } else {
                (0..cnamed)
                    .find(|&k| read_u32(named + 4 * k as u64).is_ok_and(|d| d == position))
                    .ok_or((hr::DISP_E_PARAMNOTFOUND, None))?
            };
            change(out, rgvarg + VARIANT_BYTES * index as u64, 0, target)
                .map_err(|e| (e, Some(index)))
        })();
        match r {
            Ok(()) => hr::S_OK,
            Err((e, index)) => {
                if let Some(i) = index
                    && matches!(e, hr::DISP_E_TYPEMISMATCH | hr::DISP_E_OVERFLOW)
                    && argerr != 0
                {
                    let _ = write_u32(argerr, i);
                }
                e
            }
        }
    }
}
/// DispCallFunc: `instance`'s vtable slot `offset` (or the function at
/// `offset` without an instance) with the arguments' VARIANTs by type.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn DispCallFunc(
    instance: usize,
    offset: usize,
    _cc: u32,
    ret: u16,
    count: u32,
    types: usize,
    args: usize,
    result_out: usize,
) -> u32 {
    let instance = door_windows_abi::widen(instance);
    let offset = door_windows_abi::widen(offset);
    let types = door_windows_abi::widen(types);
    let args = door_windows_abi::widen(args);
    let result_out = door_windows_abi::widen(result_out);
    {
        let r = (|| -> Result<(), u32> {
            let mut temps = Temps::default();
            let mut slots = Vec::new();
            let function = if instance != 0 {
                slots.push(Arg::Word(instance));
                read_pointer(read_pointer(instance)? + offset)?
            } else {
                offset
            };
            if ret == vt::VARIANT {
                // Returned through a hidden pointer after `this`.
                slots.push(Arg::Word(result_out));
            }
            for k in 0..count as u64 {
                let kind = read_u16(types + 2 * k)?;
                let v = read_pointer(args + P * k)?;
                slots.push(if kind & vt::BYREF != 0 {
                    Arg::Word(read_pointer(v + 8)?)
                } else {
                    Arg::of(
                        kind,
                        match kind {
                            vt::VARIANT => temps.copy(v)?,
                            vt::DECIMAL => temps.copy(v)?,
                            vt::R4 => read_u32(v + 8)? as u64,
                            _ => read_u64(v + 8)?,
                        },
                    )
                });
            }
            let float = match ret {
                vt::R4 => 4,
                vt::R8 | vt::DATE => 8,
                _ => 0,
            };
            let (rax, xmm0) = call_raw(function, &slots, float);
            if result_out == 0 || ret == vt::VARIANT {
                return Ok(());
            }
            let mut out = [0u8; 24];
            out[..2]
                .copy_from_slice(&(if ret == vt::VOID { vt::EMPTY } else { ret }).to_le_bytes());
            let bits = match ret {
                vt::R4 | vt::R8 | vt::DATE => xmm0,
                vt::EMPTY | vt::VOID => 0,
                _ => rax,
            };
            out[8..16].copy_from_slice(&bits.to_le_bytes());
            if ret == vt::R4 {
                out[12..16].fill(0);
            }
            write(result_out, &out)
        })();
        result(r)
    }
}

// ---- CreateStdDispatch --------------------------------------------------------------------
#[repr(C)]
struct StdVtbl {
    query: extern "system" fn(usize, usize, usize) -> u32,
    add_ref: extern "system" fn(usize) -> u32,
    release: extern "system" fn(usize) -> u32,
    get_type_info_count: extern "system" fn(usize, usize) -> u32,
    get_type_info: extern "system" fn(usize, usize, usize, usize) -> u32,
    get_ids_of_names: extern "system" fn(usize, usize, usize, usize, usize, usize) -> u32,
    invoke:
        extern "system" fn(usize, usize, usize, usize, usize, usize, usize, usize, usize) -> u32,
}
static STD_VTBL: StdVtbl = StdVtbl {
    query: std_query,
    add_ref: std_add_ref,
    release: std_release,
    get_type_info_count: std_count,
    get_type_info: std_type_info,
    get_ids_of_names: std_ids,
    invoke: std_invoke,
};
#[repr(C)]
struct StdDispatch {
    vtbl: &'static StdVtbl,
    references: AtomicU32,
    this: u64,
    info: u64,
}
fn std(this: u64) -> &'static StdDispatch {
    // SAFETY: STD_VTBL's methods are only called on StdDispatch objects.
    unsafe { &*(this as *const StdDispatch) }
}
extern "system" fn std_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        let Ok(g) = read_n::<16>(iid) else {
            return hr::E_POINTER;
        };
        if g == IID_IUNKNOWN || g == IID_IDISPATCH {
            std_add_ref(door_windows_abi::narrow(this));
            return result(write_pointer(out, this));
        }
        let _ = write_pointer(out, 0);
        hr::E_NOINTERFACE
    }
}
extern "system" fn std_add_ref(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    std(this).references.fetch_add(1, Ordering::AcqRel) + 1
}
extern "system" fn std_release(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        let left = std(this).references.fetch_sub(1, Ordering::AcqRel) - 1;
        if left == 0 {
            release(std(this).info);
            // SAFETY: the last reference; the object came from Box::into_raw.
            drop(unsafe { Box::from_raw(this as *mut StdDispatch) });
        }
        left
    }
}
extern "system" fn std_count(_this: usize, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    result(write_u32(out, 1))
}
extern "system" fn std_type_info(this: usize, index: usize, _lcid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let index = door_windows_abi::widen(index);
    let out = door_windows_abi::widen(out);
    {
        if index as u32 != 0 {
            let _ = write_pointer(out, 0);
            return hr::DISP_E_BADINDEX;
        }
        let info = std(this).info;
        add_ref(info);
        result(write_pointer(out, info))
    }
}
extern "system" fn std_ids(
    this: usize,
    iid: usize,
    names: usize,
    count: usize,
    _lcid: usize,
    ids: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let names = door_windows_abi::widen(names);
    let count = door_windows_abi::widen(count);
    let ids = door_windows_abi::widen(ids);
    {
        if read_n::<16>(iid).is_ok_and(|g| g != [0; 16]) {
            return hr::DISP_E_UNKNOWNINTERFACE;
        }
        DispGetIDsOfNames(
            door_windows_abi::narrow(std(this).info),
            door_windows_abi::narrow(names),
            count as u32,
            door_windows_abi::narrow(ids),
        )
    }
}
#[allow(clippy::too_many_arguments)]
extern "system" fn std_invoke(
    this: usize,
    memid: usize,
    iid: usize,
    _lcid: usize,
    flags: usize,
    params: usize,
    result_out: usize,
    excep: usize,
    argerr: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let memid = door_windows_abi::widen(memid);
    let iid = door_windows_abi::widen(iid);
    let flags = door_windows_abi::widen(flags);
    let params = door_windows_abi::widen(params);
    let result_out = door_windows_abi::widen(result_out);
    let excep = door_windows_abi::widen(excep);
    let argerr = door_windows_abi::widen(argerr);
    {
        if read_n::<16>(iid).is_ok_and(|g| g != [0; 16]) {
            return hr::DISP_E_UNKNOWNINTERFACE;
        }
        let s = std(this);
        DispInvoke(
            door_windows_abi::narrow(s.this),
            door_windows_abi::narrow(s.info),
            memid as u32 as i32,
            flags as u16,
            door_windows_abi::narrow(params),
            door_windows_abi::narrow(result_out),
            door_windows_abi::narrow(excep),
            door_windows_abi::narrow(argerr),
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateStdDispatch(
    _outer: usize,
    this: usize,
    info: usize,
    out: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let info = door_windows_abi::widen(info);
    let out = door_windows_abi::widen(out);
    {
        if this == 0 || info == 0 || out == 0 {
            return hr::E_INVALIDARG;
        }
        add_ref(info);
        let object = Box::into_raw(Box::new(StdDispatch {
            vtbl: &STD_VTBL,
            references: AtomicU32::new(1),
            this,
            info,
        })) as u64;
        result(write_pointer(out, object))
    }
}
