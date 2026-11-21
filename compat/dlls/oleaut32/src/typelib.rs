//! Type libraries. `parse` reads the MSFT format MIDL writes (a .tlb file,
//! or a PE image's TYPELIB resource: `resource`); `stdole::library` is
//! stdole2 (library stdole 2.0, what every importlib("stdole2.tlb")
//! names). A `Library` keeps what the file stores; `view` is what Windows'
//! x64 oleaut32 shows through ITypeInfo: a dual interface's dispatch side
//! (IUnknown's and IDispatch's methods first, each [retval] folded into the
//! return type) or its vtable side, and a SYS_WIN32 library widened to
//! 64-bit pointers (vtable offsets doubled, records laid out again).
//! Checked against Windows 11 26H2 (`tests/reference/oleautprobe.txt`,
//! `tests/reference/stdole2.txt`).
use crate::{hr, variant::Value, vt};
use alloc::{boxed::Box, string::String, vec::Vec};

pub mod stdole;

pub type Guid = [u8; 16];
/// MEMBERID_NIL: the type itself.
pub const NIL: i32 = -1;
pub const SYS_WIN32: u32 = 1;
pub const SYS_WIN64: u32 = 3;
/// This process's pointer width, and the SYSKIND whose libraries need no
/// new layout here (the x86 DLL shows SYS_WIN32 libraries as stored).
pub const POINTER_BYTES: u32 = core::mem::size_of::<usize>() as u32;
pub const NATIVE_SYSKIND: u32 = if POINTER_BYTES == 8 {
    SYS_WIN64
} else {
    SYS_WIN32
};
/// A vtable offset a `syskind` library stores (4-byte slots in SYS_WIN32,
/// 8-byte in SYS_WIN64), in this process's slots.
pub fn vft_bytes(vft: i16, syskind: u32) -> i16 {
    let stored = if syskind == SYS_WIN64 { 8 } else { 4 };
    (i32::from(vft) * POINTER_BYTES as i32 / stored) as i16
}
/// LIBFLAG_FHASDISKIMAGE: set on every library loaded from a file.
pub const HAS_DISK_IMAGE: u16 = 8;

/// TYPEKIND.
pub mod tkind {
    pub const ENUM: u32 = 0;
    pub const RECORD: u32 = 1;
    pub const MODULE: u32 = 2;
    pub const INTERFACE: u32 = 3;
    pub const DISPATCH: u32 = 4;
    pub const COCLASS: u32 = 5;
    pub const ALIAS: u32 = 6;
    pub const UNION: u32 = 7;
}
/// TYPEFLAGS.
pub mod typeflag {
    pub const CANCREATE: u16 = 0x2;
    pub const HIDDEN: u16 = 0x10;
    pub const DUAL: u16 = 0x40;
    pub const NONEXTENSIBLE: u16 = 0x80;
    pub const OLEAUTOMATION: u16 = 0x100;
    pub const RESTRICTED: u16 = 0x200;
    pub const DISPATCHABLE: u16 = 0x1000;
}
/// PARAMFLAGS.
pub mod paramflag {
    pub const IN: u16 = 0x1;
    pub const OUT: u16 = 0x2;
    pub const LCID: u16 = 0x4;
    pub const RETVAL: u16 = 0x8;
    pub const OPT: u16 = 0x10;
    pub const HASDEFAULT: u16 = 0x20;
}
/// FUNCKIND.
pub mod funckind {
    pub const PUREVIRTUAL: u32 = 1;
    pub const STATIC: u32 = 3;
    pub const DISPATCH: u32 = 4;
}
/// INVOKEKIND.
pub mod invkind {
    pub const FUNC: u32 = 1;
    pub const GET: u32 = 2;
    pub const PUT: u32 = 4;
    pub const PUTREF: u32 = 8;
}
/// VARKIND.
pub mod varkind {
    pub const PERINSTANCE: u32 = 0;
    pub const CONST: u32 = 2;
    pub const DISPATCH: u32 = 3;
}
pub const CC_STDCALL: u32 = 4;

/// A TYPEDESC.
#[derive(Clone, Debug, PartialEq)]
pub enum TypeDesc {
    Base(u16),
    Ptr(Box<TypeDesc>),
    SafeArray(Box<TypeDesc>),
    /// VT_CARRAY: the element and each dimension's (count, lower bound).
    CArray(Box<TypeDesc>, Vec<(u32, i32)>),
    User(Ref),
}
impl TypeDesc {
    pub fn vt(&self) -> u16 {
        match self {
            Self::Base(v) => *v,
            Self::Ptr(_) => vt::PTR,
            Self::SafeArray(_) => vt::SAFEARRAY,
            Self::CArray(..) => vt::CARRAY,
            Self::User(_) => vt::USERDEFINED,
        }
    }
}
/// A type a library refers to: one of its own (by index) or one of its
/// imports (by index into `Library::imports`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ref {
    Local(u32),
    Import(u32),
}
impl Ref {
    /// The HREFTYPE an MSFT file uses: a type's offset in the typeinfo
    /// table, or an impinfo's offset with bit 0 set.
    pub fn href(self) -> u32 {
        match self {
            Self::Local(i) => i * 0x64,
            Self::Import(i) => (i * 12) | 1,
        }
    }
    pub fn from_href(h: u32) -> Option<Self> {
        match h & 3 {
            0 if h.is_multiple_of(0x64) => Some(Self::Local(h / 0x64)),
            1 if (h & !3).is_multiple_of(12) => Some(Self::Import((h & !3) / 12)),
            _ => None,
        }
    }
}
/// A custom-data item (ITypeLib2::GetCustData and friends).
#[derive(Clone, Debug, PartialEq)]
pub struct Custom {
    pub guid: Guid,
    pub value: Value,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Name(String),
    Ordinal(u16),
}
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub ty: TypeDesc,
    pub flags: u16,
    pub default: Option<Value>,
    pub name: Option<String>,
    pub custom: Vec<Custom>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Func {
    pub memid: i32,
    pub kind: u32,
    pub invoke: u32,
    pub callconv: u32,
    pub ret: TypeDesc,
    pub params: Vec<Param>,
    pub optional: i16,
    /// The vtable offset as the file stores it (4-byte slots in SYS_WIN32).
    pub vft: i16,
    pub flags: u16,
    pub name: String,
    pub doc: Option<String>,
    pub help_context: u32,
    pub help_string_context: u32,
    pub entry: Option<Entry>,
    pub custom: Vec<Custom>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Var {
    pub memid: i32,
    pub kind: u32,
    pub flags: u16,
    pub ty: TypeDesc,
    /// VAR_CONST's value.
    pub value: Option<Value>,
    /// VAR_PERINSTANCE's offset.
    pub offset: u32,
    pub name: String,
    pub doc: Option<String>,
    pub help_context: u32,
    pub help_string_context: u32,
    pub custom: Vec<Custom>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Impl {
    pub target: Ref,
    pub flags: i32,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Type {
    pub kind: u32,
    pub guid: Guid,
    pub flags: u16,
    pub name: String,
    pub doc: Option<String>,
    pub help_context: u32,
    pub help_string_context: u32,
    pub major: u16,
    pub minor: u16,
    pub size: u32,
    pub align: u16,
    /// cbSizeVft as stored.
    pub vft: u16,
    pub impls: Vec<Impl>,
    pub alias: Option<TypeDesc>,
    /// A module's DLL.
    pub dll: Option<String>,
    pub funcs: Vec<Func>,
    pub vars: Vec<Var>,
    pub custom: Vec<Custom>,
}
/// An imported type: which import file, and the type there (by GUID, or
/// by index when the impinfo says so).
#[derive(Clone, Debug, PartialEq)]
pub struct Import {
    pub file: usize,
    pub kind: u32,
    pub guid: Option<Guid>,
    pub index: u32,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ImportFile {
    pub guid: Guid,
    pub lcid: u32,
    pub major: u16,
    pub minor: u16,
    pub name: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Library {
    pub guid: Guid,
    pub lcid: u32,
    pub syskind: u32,
    pub major: u16,
    pub minor: u16,
    pub flags: u16,
    pub name: String,
    pub doc: Option<String>,
    pub help_context: u32,
    pub help_string_context: u32,
    pub help_file: Option<String>,
    pub types: Vec<Type>,
    pub imports: Vec<Import>,
    pub files: Vec<ImportFile>,
    pub custom: Vec<Custom>,
}
impl Library {
    /// ITypeLib::IsName: the stored spelling of a type, member or
    /// parameter name.
    pub fn is_name(&self, name: &str) -> Option<&str> {
        if let Some(t) = self.types.iter().find(|t| same_name(&t.name, name)) {
            return Some(&t.name);
        }
        for t in &self.types {
            for f in &t.funcs {
                if same_name(&f.name, name) {
                    return Some(&f.name);
                }
                if let Some(p) = f
                    .params
                    .iter()
                    .filter_map(|p| p.name.as_deref())
                    .find(|p| same_name(p, name))
                {
                    return Some(p);
                }
            }
            if let Some(v) = t.vars.iter().find(|v| same_name(&v.name, name)) {
                return Some(&v.name);
            }
        }
        None
    }
    pub fn by_guid(&self, guid: &Guid) -> Option<u32> {
        self.types
            .iter()
            .position(|t| t.guid == *guid && *guid != [0; 16])
            .map(|i| i as u32)
    }
}

/// Case-insensitive name comparison (Windows compares with the library's
/// LCID; names are ASCII in practice).
pub fn same_name(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .all(|(x, y)| x.eq_ignore_ascii_case(&y))
}

// ---- MSFT ------------------------------------------------------------------
const S_TYPEINFO: usize = 0;
const S_IMPINFO: usize = 1;
const S_IMPFILES: usize = 2;
const S_REFTAB: usize = 3;
const S_GUID: usize = 5;
const S_NAME: usize = 7;
const S_STRING: usize = 8;
const S_TYPEDESC: usize = 9;
const S_ARRAYDESC: usize = 10;
const S_CUSTDATA: usize = 11;
const S_CDGUIDS: usize = 12;
const BAD: u32 = hr::TYPE_E_CANTLOADLIBRARY;

struct File<'a> {
    data: &'a [u8],
    segments: [(usize, usize); 15],
}
fn ansi(b: &[u8]) -> String {
    b.iter().map(|&c| char::from(c)).collect()
}
impl File<'_> {
    fn bytes(&self, at: usize, len: usize) -> Result<&[u8], u32> {
        self.data
            .get(at..at.checked_add(len).ok_or(BAD)?)
            .ok_or(BAD)
    }
    fn u16(&self, at: usize) -> Result<u16, u32> {
        let b = self.bytes(at, 2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn i16(&self, at: usize) -> Result<i16, u32> {
        Ok(self.u16(at)? as i16)
    }
    fn i32(&self, at: usize) -> Result<i32, u32> {
        let b = self.bytes(at, 4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&self, at: usize) -> Result<u64, u32> {
        let b = self.bytes(at, 8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_le_bytes(a))
    }
    /// The file offset of `off` within segment `s`.
    fn seg(&self, s: usize, off: i32) -> Result<usize, u32> {
        let (base, len) = self.segments[s];
        let off = usize::try_from(off).map_err(|_| BAD)?;
        if off >= len {
            return Err(BAD);
        }
        Ok(base + off)
    }
    fn name(&self, off: i32) -> Result<String, u32> {
        if off < 0 {
            return Ok(String::new());
        }
        let at = self.seg(S_NAME, off)?;
        let len = (self.i32(at + 8)? & 0xff) as usize;
        Ok(ansi(self.bytes(at + 12, len)?))
    }
    fn string(&self, off: i32) -> Result<Option<String>, u32> {
        if off < 0 {
            return Ok(None);
        }
        let at = self.seg(S_STRING, off)?;
        let len = self.u16(at)? as usize;
        Ok(Some(ansi(self.bytes(at + 2, len)?)))
    }
    fn guid(&self, off: i32) -> Result<Guid, u32> {
        if off < 0 {
            return Ok([0; 16]);
        }
        let at = self.seg(S_GUID, off)?;
        let mut g = [0u8; 16];
        g.copy_from_slice(self.bytes(at, 16)?);
        Ok(g)
    }
    fn typedesc(&self, index: usize, depth: u32) -> Result<TypeDesc, u32> {
        if depth > 32 {
            return Err(BAD);
        }
        let at = self.seg(S_TYPEDESC, (index * 8) as i32)?;
        let v = self.u16(at)? & vt::TYPEMASK;
        let w2 = self.i16(at + 4)?;
        let w3 = self.i16(at + 6)?;
        let inner = |d: u32| -> Result<Box<TypeDesc>, u32> {
            Ok(Box::new(if w3 < 0 {
                TypeDesc::Base((w2 as u16) & vt::TYPEMASK)
            } else {
                self.typedesc(w2 as u16 as usize / 8, d + 1)?
            }))
        };
        Ok(match v {
            vt::PTR => TypeDesc::Ptr(inner(depth)?),
            vt::SAFEARRAY => TypeDesc::SafeArray(inner(depth)?),
            vt::CARRAY => {
                let a = self.seg(S_ARRAYDESC, w2 as u16 as i32)?;
                let elem = self.datatype_at(self.i32(a)?, depth + 1)?;
                let dims = self.u16(a + 4)? as usize;
                let mut bounds = Vec::new();
                for d in 0..dims.min(64) {
                    let b = a + 8 + d * 8;
                    bounds.push((self.i32(b)? as u32, self.i32(b + 4)?));
                }
                TypeDesc::CArray(Box::new(elem), bounds)
            }
            vt::USERDEFINED => {
                let h = (w2 as u16 as u32) | ((w3 as u16 as u32) << 16);
                TypeDesc::User(Ref::from_href(h).ok_or(BAD)?)
            }
            _ => TypeDesc::Base(v),
        })
    }
    fn datatype_at(&self, v: i32, depth: u32) -> Result<TypeDesc, u32> {
        if v < 0 {
            Ok(TypeDesc::Base((v as u32 & vt::TYPEMASK as u32) as u16))
        } else {
            self.typedesc(v as usize / 8, depth)
        }
    }
    fn datatype(&self, v: i32) -> Result<TypeDesc, u32> {
        self.datatype_at(v, 0)
    }
    /// A constant or default value: packed into the word itself (negative),
    /// or a VARTYPE and its data in the custom-data segment.
    fn value(&self, v: i32) -> Result<Value, u32> {
        if v < 0 {
            let kind = ((v as u32 & 0x7c00_0000) >> 26) as u16;
            return Ok(scalar(kind, (v as u32 & 0x03ff_ffff) as u64));
        }
        let at = self.seg(S_CUSTDATA, v)?;
        let kind = self.u16(at)?;
        Ok(match kind {
            vt::BSTR => {
                let len = self.i32(at + 2)?;
                if len < 0 {
                    Value::Bstr(Vec::new())
                } else {
                    let text = self.bytes(at + 6, len as usize)?;
                    Value::Bstr(text.iter().map(|&c| c as u16).collect())
                }
            }
            vt::R8 | vt::CY | vt::DATE | vt::I8 | vt::UI8 => scalar(kind, self.u64(at + 2)?),
            _ => scalar(kind, self.i32(at + 2)? as u32 as u64),
        })
    }
    fn custom(&self, mut off: i32) -> Result<Vec<Custom>, u32> {
        let mut out = Vec::new();
        while off >= 0 && out.len() < 1024 {
            let at = self.seg(S_CDGUIDS, off)?;
            out.push(Custom {
                guid: self.guid(self.i32(at)?)?,
                value: self.value(self.i32(at + 4)?)?,
            });
            off = self.i32(at + 8)?;
        }
        Ok(out)
    }
}
/// A Value of VARTYPE `kind` from its little-endian bits.
fn scalar(kind: u16, bits: u64) -> Value {
    match kind {
        vt::NULL => Value::Null,
        vt::I1 => Value::I1(bits as i8),
        vt::UI1 => Value::UI1(bits as u8),
        vt::I2 => Value::I2(bits as i16),
        vt::UI2 => Value::UI2(bits as u16),
        vt::I4 | vt::HRESULT => Value::I4(bits as i32),
        vt::UI4 => Value::UI4(bits as u32),
        vt::INT => Value::Int(bits as i32),
        vt::UINT => Value::UInt(bits as u32),
        vt::I8 => Value::I8(bits as i64),
        vt::UI8 => Value::UI8(bits),
        vt::R4 => Value::R4(f32::from_bits(bits as u32)),
        vt::R8 => Value::R8(f64::from_bits(bits)),
        vt::CY => Value::Cy(bits as i64),
        vt::DATE => Value::Date(f64::from_bits(bits)),
        vt::BOOL => Value::Bool(bits as i16),
        vt::ERROR => Value::Error(bits as u32),
        _ => Value::Empty,
    }
}

/// Reads an MSFT type library.
pub fn parse(data: &[u8]) -> Result<Library, u32> {
    if data.get(..4) != Some(b"MSFT") {
        return Err(BAD);
    }
    let mut f = File {
        data,
        segments: [(0, 0); 15],
    };
    let varflags = f.i32(20)? as u32;
    let count = usize::try_from(f.i32(32)?).map_err(|_| BAD)?;
    let mut pos = 84 + if varflags & 0x100 != 0 { 4 } else { 0 } + 4 * count;
    let mut segments = [(0usize, 0usize); 15];
    for s in segments.iter_mut() {
        let offset = f.i32(pos)?;
        let length = f.i32(pos + 4)?;
        *s = if offset < 0 || length <= 0 {
            (0, 0)
        } else {
            (offset as usize, length as usize)
        };
        pos += 16;
    }
    f.segments = segments;
    let version = f.i32(24)? as u32;
    let mut lib = Library {
        guid: f.guid(f.i32(8)?)?,
        lcid: f.i32(16)? as u32,
        syskind: varflags & 0xf,
        major: version as u16,
        minor: (version >> 16) as u16,
        flags: f.i32(28)? as u16,
        name: f.name(f.i32(56)?)?,
        doc: f.string(f.i32(36)?)?,
        help_context: f.i32(44)? as u32,
        help_string_context: f.i32(40)? as u32,
        help_file: f.string(f.i32(60)?)?,
        types: Vec::new(),
        imports: Vec::new(),
        files: Vec::new(),
        custom: f.custom(f.i32(64)?)?,
    };
    // Imports.
    let mut file_offsets: Vec<i32> = Vec::new();
    for j in 0..usize::try_from(f.i32(80)?).map_err(|_| BAD)?.min(4096) {
        let at = f.seg(S_IMPINFO, (j * 12) as i32)?;
        let flags = f.i32(at)? as u32;
        let file_off = f.i32(at + 4)?;
        let target = f.i32(at + 8)?;
        let file = match file_offsets.iter().position(|&o| o == file_off) {
            Some(i) => i,
            None => {
                let a = f.seg(S_IMPFILES, file_off)?;
                let version = f.i32(a + 8)? as u32;
                let len = (f.u16(a + 12)? >> 2) as usize;
                lib.files.push(ImportFile {
                    guid: f.guid(f.i32(a)?)?,
                    lcid: f.i32(a + 4)? as u32,
                    major: version as u16,
                    minor: (version >> 16) as u16,
                    name: ansi(f.bytes(a + 14, len)?),
                });
                file_offsets.push(file_off);
                file_offsets.len() - 1
            }
        };
        let by_guid = flags & 0x1_0000 != 0;
        lib.imports.push(Import {
            file,
            kind: flags >> 24,
            guid: if by_guid { Some(f.guid(target)?) } else { None },
            index: if by_guid { 0 } else { target as u32 },
        });
    }
    for i in 0..count {
        lib.types.push(parse_type(&f, i)?);
    }
    Ok(lib)
}

fn parse_type(f: &File, index: usize) -> Result<Type, u32> {
    let at = f.seg(S_TYPEINFO, (index * 0x64) as i32)?;
    let raw = f.i32(at)? as u32;
    let kind = raw & 0xf;
    let elements = f.i32(at + 0x18)? as u32;
    let (nfuncs, nvars) = ((elements & 0xffff) as usize, (elements >> 16) as usize);
    let version = f.i32(at + 0x38)? as u32;
    let nimpls = f.i16(at + 0x4c)?.max(0) as usize;
    let datatype1 = f.i32(at + 0x54)?;
    let mut t = Type {
        kind,
        guid: f.guid(f.i32(at + 0x2c)?)?,
        flags: f.i32(at + 0x30)? as u16,
        name: f.name(f.i32(at + 0x34)?)?,
        doc: f.string(f.i32(at + 0x3c)?)?,
        help_context: f.i32(at + 0x44)? as u32,
        help_string_context: f.i32(at + 0x40)? as u32,
        major: version as u16,
        minor: (version >> 16) as u16,
        size: f.i32(at + 0x50)? as u32,
        align: ((raw >> 11) & 0x1f) as u16,
        vft: f.u16(at + 0x4e)?,
        impls: Vec::new(),
        alias: None,
        dll: None,
        funcs: Vec::new(),
        vars: Vec::new(),
        custom: f.custom(f.i32(at + 0x48)?)?,
    };
    match kind {
        tkind::COCLASS => {
            let mut off = datatype1;
            while t.impls.len() < nimpls && off >= 0 {
                let r = f.seg(S_REFTAB, off)?;
                t.impls.push(Impl {
                    target: Ref::from_href(f.i32(r)? as u32).ok_or(BAD)?,
                    flags: f.i32(r + 4)?,
                });
                off = f.i32(r + 12)?;
            }
        }
        tkind::INTERFACE | tkind::DISPATCH if nimpls > 0 => t.impls.push(Impl {
            target: Ref::from_href(datatype1 as u32).ok_or(BAD)?,
            flags: 0,
        }),
        tkind::ALIAS => t.alias = Some(f.datatype(datatype1)?),
        tkind::MODULE => t.dll = f.string(datatype1)?,
        _ => {}
    }
    let n = nfuncs + nvars;
    let memoffset = f.i32(at + 4)?;
    if n == 0 || memoffset < 0 || memoffset as usize >= f.data.len() {
        return Ok(t);
    }
    let base = memoffset as usize;
    let records = base + 4;
    let tables = records + usize::try_from(f.i32(base)?).map_err(|_| BAD)?;
    for i in 0..n {
        let memid = f.i32(tables + 4 * i)?;
        let name = f.name(f.i32(tables + 4 * (n + i))?)?;
        let rec = records + usize::try_from(f.i32(tables + 4 * (2 * n + i))?).map_err(|_| BAD)?;
        if i < nfuncs {
            t.funcs.push(parse_func(f, rec, memid, name)?);
        } else {
            t.vars.push(parse_var(f, rec, memid, name)?);
        }
    }
    Ok(t)
}

fn parse_func(f: &File, rec: usize, memid: i32, name: String) -> Result<Func, u32> {
    let len = (f.i32(rec)? as u32 & 0xffff) as usize;
    let fkccic = f.i32(rec + 16)? as u32;
    let nargs = f.i16(rec + 20)?.max(0) as usize;
    let defaults = fkccic & 0x1000 != 0;
    let params_at = (rec + len).checked_sub(nargs * 12).ok_or(BAD)?;
    let defaults_at = params_at
        .checked_sub(if defaults { nargs * 4 } else { 0 })
        .ok_or(BAD)?;
    let nattrs = defaults_at.saturating_sub(rec + 24) / 4;
    let attr = |k: usize| -> Result<Option<i32>, u32> {
        if k < nattrs {
            Ok(Some(f.i32(rec + 24 + 4 * k)?))
        } else {
            Ok(None)
        }
    };
    let entry = match attr(2)? {
        Some(e) if fkccic & 0x2000 != 0 => Some(Entry::Ordinal(e as u16)),
        Some(e) if e >= 0 => f.string(e)?.map(Entry::Name),
        _ => None,
    };
    let custom = match attr(6)? {
        Some(c) if fkccic & 0x80 != 0 => f.custom(c)?,
        _ => Vec::new(),
    };
    let mut params = Vec::new();
    for j in 0..nargs {
        let p = params_at + 12 * j;
        let pname = f.i32(p + 4)?;
        let default = if defaults {
            match f.i32(defaults_at + 4 * j)? {
                -1 => None,
                d => Some(f.value(d)?),
            }
        } else {
            None
        };
        let pcustom = match attr(7 + j)? {
            Some(c) if fkccic & 0x80 != 0 => f.custom(c)?,
            _ => Vec::new(),
        };
        params.push(Param {
            ty: f.datatype(f.i32(p)?)?,
            flags: f.i32(p + 8)? as u16,
            default,
            name: if pname < 0 {
                None
            } else {
                Some(f.name(pname)?)
            },
            custom: pcustom,
        });
    }
    Ok(Func {
        memid,
        kind: fkccic & 7,
        invoke: (fkccic >> 3) & 0xf,
        callconv: (fkccic >> 8) & 0xf,
        ret: f.datatype(f.i32(rec + 4)?)?,
        params,
        optional: f.i16(rec + 22)?,
        vft: f.i16(rec + 12)?,
        flags: f.i32(rec + 8)? as u16,
        name,
        doc: match attr(1)? {
            Some(d) => f.string(d)?,
            None => None,
        },
        help_context: attr(0)?.unwrap_or(0) as u32,
        help_string_context: attr(5)?.unwrap_or(0) as u32,
        entry,
        custom,
    })
}

fn parse_var(f: &File, rec: usize, memid: i32, name: String) -> Result<Var, u32> {
    let len = f.u16(rec)? as usize;
    let kind = f.u16(rec + 12)? as u32;
    let offs = f.i32(rec + 16)?;
    let opt = |k: usize| -> Result<Option<i32>, u32> {
        if len > 20 + 4 * k {
            Ok(Some(f.i32(rec + 20 + 4 * k)?))
        } else {
            Ok(None)
        }
    };
    Ok(Var {
        memid,
        kind,
        flags: f.i32(rec + 8)? as u16,
        ty: f.datatype(f.i32(rec + 4)?)?,
        value: if kind == varkind::CONST {
            Some(f.value(offs)?)
        } else {
            None
        },
        offset: if kind == varkind::CONST {
            0
        } else {
            offs as u32
        },
        name,
        doc: match opt(1)? {
            Some(d) => f.string(d)?,
            None => None,
        },
        help_context: opt(0)?.unwrap_or(0) as u32,
        help_string_context: opt(4)?.unwrap_or(0) as u32,
        custom: match opt(3)? {
            Some(c) => f.custom(c)?,
            None => Vec::new(),
        },
    })
}

/// The TYPELIB resource `index` (1 when the path names none) of a PE image
/// as a file holds it.
pub fn resource(image: &[u8], index: u32) -> Option<&[u8]> {
    let u16_at = |at: usize| -> Option<u16> {
        Some(u16::from_le_bytes(image.get(at..at + 2)?.try_into().ok()?))
    };
    let u32_at = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(image.get(at..at + 4)?.try_into().ok()?))
    };
    if image.get(..2) != Some(b"MZ") {
        return None;
    }
    let pe = u32_at(0x3c)? as usize;
    if image.get(pe..pe + 4) != Some(b"PE\0\0") {
        return None;
    }
    let sections = u16_at(pe + 6)? as usize;
    let optional = u16_at(pe + 20)? as usize;
    let opt = pe + 24;
    let directories = match u16_at(opt)? {
        0x20b => opt + 112,
        0x10b => opt + 96,
        _ => return None,
    };
    let rsrc_rva = u32_at(directories + 2 * 8)?;
    let table = opt + optional;
    let file_offset = |rva: u32| -> Option<usize> {
        (0..sections).find_map(|s| {
            let h = table + s * 40;
            let va = u32_at(h + 12)?;
            let size = u32_at(h + 16)?.max(u32_at(h + 8)?);
            let raw = u32_at(h + 20)?;
            (rva >= va && rva < va + size).then(|| (rva - va + raw) as usize)
        })
    };
    let root = file_offset(rsrc_rva)?;
    // One directory level: the entry whose name (TYPELIB) or id matches.
    let find = |dir: usize, name: Option<&str>, id: u32| -> Option<u32> {
        let named = u16_at(dir + 12)? as usize;
        let ids = u16_at(dir + 14)? as usize;
        (0..named + ids).find_map(|k| {
            let e = dir + 16 + k * 8;
            let key = u32_at(e)?;
            let hit = if key & 0x8000_0000 != 0 {
                let s = root + (key & 0x7fff_ffff) as usize;
                let n = u16_at(s)? as usize;
                let wanted = name?;
                n == wanted.len()
                    && wanted.bytes().enumerate().all(|(i, c)| {
                        u16_at(s + 2 + 2 * i)
                            .is_some_and(|u| u < 0x80 && (u as u8).eq_ignore_ascii_case(&c))
                    })
            } else {
                name.is_none() && (key == id || id == u32::MAX)
            };
            if hit { u32_at(e + 4) } else { None }
        })
    };
    let types = find(root, Some("TYPELIB"), 0)?;
    let names = root + (types & 0x7fff_ffff) as usize;
    let langs = root + (find(names, None, index)? & 0x7fff_ffff) as usize;
    let data = root + find(langs, None, u32::MAX)? as usize;
    let rva = u32_at(data)?;
    let size = u32_at(data + 4)? as usize;
    let at = file_offset(rva)?;
    image.get(at..at + size)
}

// ---- views ----------------------------------------------------------------------
/// Where `view` finds libraries and the types imports name.
pub trait Linker {
    fn library(&self, id: usize) -> Option<&Library>;
    /// The library and type index `r` (a reference from library `id`)
    /// names, loading an imported library if it must.
    fn resolve(&self, id: usize, r: Ref) -> Option<(usize, u32)>;
}
/// A dual interface's two typeinfos: the dispatch side (what
/// ITypeLib::GetTypeInfo gives) and the vtable side
/// (GetRefTypeOfImplType(-1)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Natural,
    Vtable,
}
/// What one ITypeInfo shows. Every function, variable and implemented
/// interface carries the library its type references are relative to.
#[derive(Clone, Debug)]
pub struct View {
    pub lib: usize,
    pub index: u32,
    pub side: Side,
    pub kind: u32,
    pub guid: Guid,
    pub flags: u16,
    pub size: u32,
    pub align: u16,
    pub vft: u16,
    pub major: u16,
    pub minor: u16,
    pub alias: Option<TypeDesc>,
    pub impls: Vec<(usize, Impl)>,
    pub funcs: Vec<(usize, Func)>,
    pub vars: Vec<Var>,
    pub name: String,
    pub doc: Option<String>,
    pub help_context: u32,
    pub help_string_context: u32,
    pub dll: Option<String>,
    /// The dispatch side of a dual interface (it has a vtable side).
    pub dual: bool,
}

pub fn view(l: &impl Linker, lib: usize, index: u32, side: Side) -> Result<View, u32> {
    let library = l.library(lib).ok_or(hr::TYPE_E_ELEMENTNOTFOUND)?;
    let t = library
        .types
        .get(index as usize)
        .ok_or(hr::TYPE_E_ELEMENTNOTFOUND)?;
    let syskind = library.syskind;
    let dual = t.kind == tkind::DISPATCH && t.flags & typeflag::DUAL != 0;
    if side == Side::Vtable && !dual {
        return Err(hr::TYPE_E_ELEMENTNOTFOUND);
    }
    let mut v = View {
        lib,
        index,
        side,
        kind: t.kind,
        guid: t.guid,
        flags: t.flags,
        size: t.size,
        align: t.align,
        vft: vft_bytes(t.vft as i16, syskind) as u16,
        major: t.major,
        minor: t.minor,
        alias: t.alias.clone(),
        impls: t.impls.iter().map(|i| (lib, i.clone())).collect(),
        funcs: t
            .funcs
            .iter()
            .map(|f| {
                let mut f = f.clone();
                if f.kind != funckind::DISPATCH {
                    f.vft = vft_bytes(f.vft, syskind);
                }
                (lib, f)
            })
            .collect(),
        vars: t.vars.clone(),
        name: t.name.clone(),
        doc: t.doc.clone(),
        help_context: t.help_context,
        help_string_context: t.help_string_context,
        dll: t.dll.clone(),
        dual: dual && side == Side::Natural,
    };
    if dual {
        match side {
            Side::Natural => {
                v.flags &= !typeflag::OLEAUTOMATION;
                v.vft = 7 * POINTER_BYTES as u16;
                let mut funcs = inherited(l, lib, t, 0);
                funcs.extend(t.funcs.iter().map(|f| (lib, f.clone())));
                v.funcs = funcs
                    .into_iter()
                    .map(|(h, f)| (h, to_dispatch(f)))
                    .collect();
            }
            Side::Vtable => v.kind = tkind::INTERFACE,
        }
    }
    if library.syskind != NATIVE_SYSKIND {
        match v.kind {
            tkind::INTERFACE | tkind::DISPATCH | tkind::COCLASS => {
                v.size = POINTER_BYTES;
                v.align = POINTER_BYTES as u16;
            }
            tkind::ALIAS => {
                if let Some(a) = &v.alias {
                    let (s, al) = layout(l, lib, a, 0);
                    v.size = s;
                    v.align = al as u16;
                }
            }
            tkind::RECORD | tkind::UNION => {
                let (s, al) = lay_out_members(l, lib, &mut v.vars, v.kind == tkind::UNION, 0);
                v.size = s;
                v.align = al as u16;
            }
            _ => {}
        }
    }
    Ok(v)
}

/// The methods of `t`'s base interfaces, the root's first, as their own
/// libraries store them.
fn inherited(l: &impl Linker, lib: usize, t: &Type, depth: u32) -> Vec<(usize, Func)> {
    let Some(base) = t.impls.first() else {
        return Vec::new();
    };
    if depth > 16 {
        return Vec::new();
    }
    let Some((blib, bindex)) = l.resolve(lib, base.target) else {
        return Vec::new();
    };
    let Some(bt) = l.library(blib).and_then(|b| b.types.get(bindex as usize)) else {
        return Vec::new();
    };
    let mut out = inherited(l, blib, bt, depth + 1);
    out.extend(bt.funcs.iter().map(|f| (blib, f.clone())));
    out
}

/// IDispatch's IID.
pub const IID_IDISPATCH: Guid = [0, 4, 2, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
/// Every vtable method of interface `index` of `lib`, its bases' first,
/// each with the library its types refer to and its slot offset as an x64
/// caller sees it (a SYS_WIN32 library's doubled); and whether IDispatch is
/// among the bases (its four methods then follow IUnknown's).
pub fn vtable(l: &impl Linker, lib: usize, index: u32) -> Result<(Vec<(usize, Func)>, bool), u32> {
    let mut chain = Vec::new();
    let (mut at, mut i) = (lib, index);
    for _ in 0..16 {
        let library = l.library(at).ok_or(hr::TYPE_E_ELEMENTNOTFOUND)?;
        let t = library
            .types
            .get(i as usize)
            .ok_or(hr::TYPE_E_ELEMENTNOTFOUND)?;
        chain.push((at, i));
        match t.impls.first().and_then(|b| l.resolve(at, b.target)) {
            Some((bl, bi)) => (at, i) = (bl, bi),
            None => break,
        }
    }
    let mut funcs = Vec::new();
    let mut dispatch = false;
    for &(at, i) in chain.iter().rev() {
        let library = l.library(at).ok_or(hr::TYPE_E_ELEMENTNOTFOUND)?;
        let t = &library.types[i as usize];
        dispatch |= t.guid == IID_IDISPATCH;
        let syskind = library.syskind;
        for f in t.funcs.iter().filter(|f| f.kind != funckind::DISPATCH) {
            let mut f = f.clone();
            f.vft = vft_bytes(f.vft, syskind);
            funcs.push((at, f));
        }
    }
    Ok((funcs, dispatch))
}

/// A vtable method as a dual's dispatch side shows it: FUNC_DISPATCH, LCID
/// parameters dropped, an HRESULT return replaced by the [retval]
/// parameter's type (or VT_VOID).
pub fn to_dispatch(mut f: Func) -> Func {
    f.kind = funckind::DISPATCH;
    f.params.retain(|p| p.flags & paramflag::LCID == 0);
    if f.ret == TypeDesc::Base(vt::HRESULT) {
        f.ret = match f.params.last() {
            Some(p) if p.flags & paramflag::RETVAL != 0 => match f.params.pop().map(|p| p.ty) {
                Some(TypeDesc::Ptr(inner)) => *inner,
                Some(other) => other,
                None => TypeDesc::Base(vt::VOID),
            },
            _ => TypeDesc::Base(vt::VOID),
        };
    }
    f
}

/// The size and alignment of a type in this process (MSVC's layout for
/// its pointer width).
pub fn layout(l: &impl Linker, lib: usize, t: &TypeDesc, depth: u32) -> (u32, u32) {
    const P: u32 = POINTER_BYTES;
    match t {
        TypeDesc::Base(v) => match *v {
            vt::I1 | vt::UI1 => (1, 1),
            vt::I2 | vt::UI2 | vt::BOOL => (2, 2),
            vt::I8 | vt::UI8 | vt::R8 | vt::CY | vt::DATE | 64 => (8, 8),
            vt::VARIANT => (8 + 2 * P, 8),
            vt::DECIMAL => (16, 8),
            vt::BSTR
            | vt::UNKNOWN
            | vt::DISPATCH
            | vt::LPSTR
            | vt::LPWSTR
            | vt::INT_PTR
            | vt::UINT_PTR => (P, P),
            _ => (4, 4),
        },
        TypeDesc::Ptr(_) | TypeDesc::SafeArray(_) => (P, P),
        TypeDesc::CArray(e, bounds) => {
            let (s, a) = layout(l, lib, e, depth + 1);
            (s * bounds.iter().map(|b| b.0).product::<u32>(), a)
        }
        TypeDesc::User(r) => match l.resolve(lib, *r) {
            Some((tl, ti)) if depth < 16 => type_layout(l, tl, ti, depth + 1),
            _ => (4, 4),
        },
    }
}
fn type_layout(l: &impl Linker, lib: usize, index: u32, depth: u32) -> (u32, u32) {
    let Some(library) = l.library(lib) else {
        return (4, 4);
    };
    let Some(t) = library.types.get(index as usize) else {
        return (4, 4);
    };
    if library.syskind == NATIVE_SYSKIND {
        return (t.size, t.align.max(1) as u32);
    }
    match t.kind {
        tkind::INTERFACE | tkind::DISPATCH | tkind::COCLASS => (POINTER_BYTES, POINTER_BYTES),
        tkind::ALIAS => match &t.alias {
            Some(a) => layout(l, lib, a, depth),
            None => (4, 4),
        },
        tkind::RECORD | tkind::UNION => {
            let mut vars = t.vars.clone();
            lay_out_members(l, lib, &mut vars, t.kind == tkind::UNION, depth)
        }
        tkind::ENUM => (4, 4),
        _ => (t.size, t.align.max(1) as u32),
    }
}
fn lay_out_members(
    l: &impl Linker,
    lib: usize,
    vars: &mut [Var],
    union: bool,
    depth: u32,
) -> (u32, u32) {
    let (mut end, mut align) = (0u32, 1u32);
    for v in vars.iter_mut().filter(|v| v.kind == varkind::PERINSTANCE) {
        let (s, a) = layout(l, lib, &v.ty, depth + 1);
        let at = if union { 0 } else { end.div_ceil(a) * a };
        v.offset = at;
        end = end.max(at + s);
        align = align.max(a);
    }
    (end.div_ceil(align) * align, align)
}

impl View {
    /// The function with this member ID and invoke kind (any kind for 0).
    pub fn func(&self, memid: i32, invoke: u32) -> Option<&Func> {
        self.funcs
            .iter()
            .map(|(_, f)| f)
            .find(|f| f.memid == memid && (invoke == 0 || f.invoke & invoke != 0))
    }
    /// ITypeInfo::GetNames: the member's name and its parameters' names.
    pub fn names(&self, memid: i32) -> Option<Vec<String>> {
        if let Some(f) = self.func(memid, 0) {
            let mut out = Vec::from([f.name.clone()]);
            out.extend(f.params.iter().filter_map(|p| p.name.clone()));
            return Some(out);
        }
        self.vars
            .iter()
            .find(|v| v.memid == memid)
            .map(|v| Vec::from([v.name.clone()]))
    }
    /// ITypeInfo::GetDocumentation: name, doc string, help context.
    pub fn documentation(&self, memid: i32) -> Option<(String, Option<String>, u32)> {
        if memid == NIL {
            return Some((self.name.clone(), self.doc.clone(), self.help_context));
        }
        if let Some(f) = self.func(memid, 0) {
            return Some((f.name.clone(), f.doc.clone(), f.help_context));
        }
        self.vars
            .iter()
            .find(|v| v.memid == memid)
            .map(|v| (v.name.clone(), v.doc.clone(), v.help_context))
    }
    /// ITypeInfo::GetIDsOfNames: the member's ID, then each named
    /// parameter's index; DISPID_UNKNOWN (-1) where a name is unknown.
    pub fn ids_of_names(&self, names: &[String]) -> (Vec<i32>, bool) {
        let mut ids = alloc::vec![-1; names.len()];
        let Some(first) = names.first() else {
            return (ids, true);
        };
        if let Some(f) = self
            .funcs
            .iter()
            .map(|(_, f)| f)
            .find(|f| same_name(&f.name, first))
        {
            ids[0] = f.memid;
            let mut ok = true;
            for (i, n) in names.iter().enumerate().skip(1) {
                match f
                    .params
                    .iter()
                    .position(|p| p.name.as_deref().is_some_and(|p| same_name(p, n)))
                {
                    Some(k) => ids[i] = k as i32,
                    None => ok = false,
                }
            }
            return (ids, ok);
        }
        if let Some(v) = self.vars.iter().find(|v| same_name(&v.name, first)) {
            ids[0] = v.memid;
            return (ids, names.len() == 1);
        }
        (ids, false)
    }
}

#[cfg(test)]
mod tests;
