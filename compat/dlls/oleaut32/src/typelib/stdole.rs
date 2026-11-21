//! stdole2: library stdole 2.0 ("OLE Automation"), the types every
//! importlib("stdole2.tlb") brings in (IUnknown, IDispatch, IEnumVARIANT,
//! the OLE_* aliases, IFont/Font/StdFont, IPicture/Picture/StdPicture,
//! StdFunctions). Doors builds it here instead of shipping a file; it is
//! stored as Windows' System32\stdole2.tlb stores it (SYS_WIN32, 4-byte
//! vtable slots), so `view` widens it the same way. Its 42 types match
//! Windows 11 26H2's, member by member (`tests/reference/stdole2.txt`).
use super::*;
use crate::variant::Value;
use alloc::vec;

pub const GUID: Guid = guid(0x0002_0430, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);

const fn guid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> Guid {
    let a = d1.to_le_bytes();
    let b = d2.to_le_bytes();
    let c = d3.to_le_bytes();
    [
        a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d4[0], d4[1], d4[2], d4[3], d4[4], d4[5],
        d4[6], d4[7],
    ]
}
const OLE: [u8; 8] = [0x8B, 0xBB, 0x00, 0xAA, 0x00, 0x30, 0x0C, 0xAB];
const CONTAINER: [u8; 8] = [0xAE, 0x2D, 0x08, 0x00, 0x2B, 0x2E, 0xC7, 0x13];
fn ole(d1: u32) -> Guid {
    guid(d1, 0xBE0F, 0x101A, OLE)
}
fn container(d1: u32) -> Guid {
    guid(d1, 0x9069, 0x101B, CONTAINER)
}

// Type indices.
const T_GUID: u32 = 0;
const T_DISPPARAMS: u32 = 1;
const T_EXCEPINFO: u32 = 2;
const T_IUNKNOWN: u32 = 3;
const T_IDISPATCH: u32 = 4;
const T_IENUMVARIANT: u32 = 5;
const T_OLE_HANDLE: u32 = 19;
const T_XPOS_HIMETRIC: u32 = 11;
const T_YPOS_HIMETRIC: u32 = 12;
const T_XSIZE_HIMETRIC: u32 = 13;
const T_YSIZE_HIMETRIC: u32 = 14;
const T_IFONT: u32 = 30;
const T_FONT: u32 = 31;
const T_IPICTURE: u32 = 34;
const T_PICTURE: u32 = 35;
const T_IPICTUREDISP: u32 = 36;
const T_LOADPICTURE: u32 = 38;
const T_FONTEVENTS: u32 = 40;

fn b(v: u16) -> TypeDesc {
    TypeDesc::Base(v)
}
fn ptr(t: TypeDesc) -> TypeDesc {
    TypeDesc::Ptr(Box::new(t))
}
fn user(i: u32) -> TypeDesc {
    TypeDesc::User(Ref::Local(i))
}
fn p(name: &str, ty: TypeDesc, flags: u16) -> Param {
    Param {
        ty,
        flags,
        default: None,
        name: Some(name.into()),
        custom: Vec::new(),
    }
}
fn with_default(mut param: Param, v: Value) -> Param {
    param.default = Some(v);
    param
}
struct F {
    kind: u32,
    flags: u16,
}
const PURE: F = F {
    kind: funckind::PUREVIRTUAL,
    flags: 0,
};
const RESTRICTED: F = F {
    kind: funckind::PUREVIRTUAL,
    flags: 1,
};
fn func(
    name: &str,
    memid: i32,
    how: F,
    invoke: u32,
    vft: i16,
    ret: TypeDesc,
    params: Vec<Param>,
) -> Func {
    Func {
        memid,
        kind: how.kind,
        invoke,
        callconv: CC_STDCALL,
        ret,
        params,
        optional: 0,
        vft,
        flags: how.flags,
        name: name.into(),
        doc: None,
        help_context: 0,
        help_string_context: 0,
        entry: None,
        custom: Vec::new(),
    }
}
/// A vtable method of an IUnknown-derived interface (memid 0x6001xxxx,
/// slot `slot` counting IUnknown's three).
fn method(name: &str, memid_index: i32, slot: i16, invoke: u32, params: Vec<Param>) -> Func {
    func(
        name,
        0x6001_0000 + memid_index,
        PURE,
        invoke,
        slot * 4,
        b(vt::HRESULT),
        params,
    )
}
fn ty(kind: u32, name: &str, g: Guid, flags: u16) -> Type {
    Type {
        kind,
        guid: g,
        flags,
        name: name.into(),
        doc: None,
        help_context: 0,
        help_string_context: 0,
        major: 0,
        minor: 0,
        size: 4,
        align: 4,
        vft: 0,
        impls: Vec::new(),
        alias: None,
        dll: None,
        funcs: Vec::new(),
        vars: Vec::new(),
        custom: Vec::new(),
    }
}
fn field(name: &str, i: i32, t: TypeDesc) -> Var {
    Var {
        memid: 0x4000_0000 + i,
        kind: varkind::PERINSTANCE,
        flags: 0,
        ty: t,
        value: None,
        offset: 0,
        name: name.into(),
        doc: None,
        help_context: 0,
        help_string_context: 0,
        custom: Vec::new(),
    }
}
fn record(name: &str, flags: u16, fields: &[(&str, TypeDesc)]) -> Type {
    let mut t = ty(tkind::RECORD, name, [0; 16], flags);
    t.vars = fields
        .iter()
        .enumerate()
        .map(|(i, (n, d))| field(n, i as i32, d.clone()))
        .collect();
    t
}
fn constants(name: &str, g: Guid, values: &[(&str, i32)]) -> Type {
    let mut t = ty(tkind::ENUM, name, g, 0);
    t.vars = values
        .iter()
        .enumerate()
        .map(|(i, (n, v))| Var {
            kind: varkind::CONST,
            value: Some(Value::I4(*v)),
            ..field(n, i as i32, b(vt::INT))
        })
        .collect();
    t
}
fn alias(name: &str, g: Guid, flags: u16, to: TypeDesc) -> Type {
    let mut t = ty(tkind::ALIAS, name, g, flags);
    t.alias = Some(to);
    t
}
fn interface(name: &str, g: Guid, flags: u16, base: Option<u32>, funcs: Vec<Func>) -> Type {
    let mut t = ty(tkind::INTERFACE, name, g, flags);
    let inherited: u16 = match base {
        Some(T_IUNKNOWN) => 3,
        Some(_) => 7,
        None => 0,
    };
    t.vft = (inherited + funcs.len() as u16) * 4;
    t.impls = base
        .map(|i| {
            vec![Impl {
                target: Ref::Local(i),
                flags: 0,
            }]
        })
        .unwrap_or_default();
    t.funcs = funcs;
    t
}
fn dispinterface(name: &str, g: Guid, flags: u16, funcs: Vec<Func>, props: Vec<Var>) -> Type {
    let mut t = ty(tkind::DISPATCH, name, g, flags);
    t.vft = 28;
    t.impls = vec![Impl {
        target: Ref::Local(T_IDISPATCH),
        flags: 0,
    }];
    t.funcs = funcs;
    t.vars = props;
    t
}
fn property(name: &str, memid: i32, flags: u16, t: TypeDesc) -> Var {
    Var {
        memid,
        kind: varkind::DISPATCH,
        flags,
        ..field(name, 0, t)
    }
}
fn coclass(name: &str, g: Guid, impls: &[(u32, i32)]) -> Type {
    let mut t = ty(tkind::COCLASS, name, g, typeflag::CANCREATE);
    t.impls = impls
        .iter()
        .map(|&(i, flags)| Impl {
            target: Ref::Local(i),
            flags,
        })
        .collect();
    t
}
/// A property pair of an IUnknown-derived interface: propget at `slot`
/// with an [out, retval] pointer, propput at `slot + 1`.
fn get_put(name: &str, param: &str, memid_index: i32, slot: i16, t: TypeDesc) -> [Func; 2] {
    [
        method(
            name,
            memid_index,
            slot,
            invkind::GET,
            vec![p(param, ptr(t.clone()), paramflag::OUT | paramflag::RETVAL)],
        ),
        method(
            name,
            memid_index,
            slot + 1,
            invkind::PUT,
            vec![p(param, t, paramflag::IN)],
        ),
    ]
}
fn get(name: &str, param: &str, memid_index: i32, slot: i16, t: TypeDesc) -> Func {
    method(
        name,
        memid_index,
        slot,
        invkind::GET,
        vec![p(param, ptr(t), paramflag::OUT | paramflag::RETVAL)],
    )
}

pub fn library() -> Library {
    const IN: u16 = paramflag::IN;
    const OUT: u16 = paramflag::OUT;
    let guid_ptr = || ptr(user(T_GUID));
    let void_pp = || ptr(ptr(b(vt::VOID)));
    let mut types = Vec::with_capacity(42);
    types.push(record(
        "GUID",
        typeflag::HIDDEN,
        &[
            ("Data1", b(vt::UI4)),
            ("Data2", b(vt::UI2)),
            ("Data3", b(vt::UI2)),
            (
                "Data4",
                TypeDesc::CArray(Box::new(b(vt::UI1)), vec![(8, 0)]),
            ),
        ],
    ));
    types.push(record(
        "DISPPARAMS",
        typeflag::RESTRICTED,
        &[
            ("rgvarg", ptr(b(vt::VARIANT))),
            ("rgdispidNamedArgs", ptr(b(vt::I4))),
            ("cArgs", b(vt::UINT)),
            ("cNamedArgs", b(vt::UINT)),
        ],
    ));
    types.push(record(
        "EXCEPINFO",
        typeflag::RESTRICTED,
        &[
            ("wCode", b(vt::UI2)),
            ("wReserved", b(vt::UI2)),
            ("bstrSource", b(vt::BSTR)),
            ("bstrDescription", b(vt::BSTR)),
            ("bstrHelpFile", b(vt::BSTR)),
            ("dwHelpContext", b(vt::UI4)),
            ("pvReserved", ptr(b(vt::VOID))),
            ("pfnDeferredFillIn", ptr(b(vt::VOID))),
            ("scode", b(vt::ERROR)),
        ],
    ));
    types.push(interface(
        "IUnknown",
        guid(0, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]),
        typeflag::HIDDEN,
        None,
        vec![
            func(
                "QueryInterface",
                0x6000_0000,
                RESTRICTED,
                invkind::FUNC,
                0,
                b(vt::HRESULT),
                vec![p("riid", guid_ptr(), IN), p("ppvObj", void_pp(), OUT)],
            ),
            func(
                "AddRef",
                0x6000_0001,
                RESTRICTED,
                invkind::FUNC,
                4,
                b(vt::UI4),
                vec![],
            ),
            func(
                "Release",
                0x6000_0002,
                RESTRICTED,
                invkind::FUNC,
                8,
                b(vt::UI4),
                vec![],
            ),
        ],
    ));
    let restricted = |name: &str, i: i32, params: Vec<Param>| Func {
        flags: 1,
        ..method(name, i, 3 + i as i16, invkind::FUNC, params)
    };
    types.push(interface(
        "IDispatch",
        guid(0x0002_0400, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]),
        typeflag::RESTRICTED,
        Some(T_IUNKNOWN),
        vec![
            restricted(
                "GetTypeInfoCount",
                0,
                vec![p("pctinfo", ptr(b(vt::UINT)), OUT)],
            ),
            restricted(
                "GetTypeInfo",
                1,
                vec![
                    p("itinfo", b(vt::UINT), IN),
                    p("lcid", b(vt::UI4), IN),
                    p("pptinfo", void_pp(), OUT),
                ],
            ),
            restricted(
                "GetIDsOfNames",
                2,
                vec![
                    p("riid", guid_ptr(), IN),
                    p("rgszNames", ptr(ptr(b(vt::I1))), IN),
                    p("cNames", b(vt::UINT), IN),
                    p("lcid", b(vt::UI4), IN),
                    p("rgdispid", ptr(b(vt::I4)), OUT),
                ],
            ),
            restricted(
                "Invoke",
                3,
                vec![
                    p("dispidMember", b(vt::I4), IN),
                    p("riid", guid_ptr(), IN),
                    p("lcid", b(vt::UI4), IN),
                    p("wFlags", b(vt::UI2), IN),
                    p("pdispparams", ptr(user(T_DISPPARAMS)), IN),
                    p("pvarResult", ptr(b(vt::VARIANT)), OUT),
                    p("pexcepinfo", ptr(user(T_EXCEPINFO)), OUT),
                    p("puArgErr", ptr(b(vt::UINT)), OUT),
                ],
            ),
        ],
    ));
    types.push(interface(
        "IEnumVARIANT",
        guid(0x0002_0404, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]),
        typeflag::HIDDEN,
        Some(T_IUNKNOWN),
        vec![
            method(
                "Next",
                0,
                3,
                invkind::FUNC,
                vec![
                    p("celt", b(vt::UI4), IN),
                    p("rgvar", ptr(b(vt::VARIANT)), IN),
                    p("pceltFetched", ptr(b(vt::UI4)), OUT),
                ],
            ),
            method("Skip", 1, 4, invkind::FUNC, vec![p("celt", b(vt::UI4), IN)]),
            method("Reset", 2, 5, invkind::FUNC, vec![]),
            method(
                "Clone",
                3,
                6,
                invkind::FUNC,
                vec![p("ppenum", ptr(ptr(user(T_IENUMVARIANT))), OUT)],
            ),
        ],
    ));
    let h = typeflag::HIDDEN;
    types.push(alias("OLE_COLOR", ole(0x6650_4301), 0, b(vt::UI4)));
    for (i, name) in [
        "OLE_XPOS_PIXELS",
        "OLE_YPOS_PIXELS",
        "OLE_XSIZE_PIXELS",
        "OLE_YSIZE_PIXELS",
        "OLE_XPOS_HIMETRIC",
        "OLE_YPOS_HIMETRIC",
        "OLE_XSIZE_HIMETRIC",
        "OLE_YSIZE_HIMETRIC",
    ]
    .iter()
    .enumerate()
    {
        types.push(alias(name, ole(0x6650_4302 + i as u32), h, b(vt::I4)));
    }
    for (i, name) in [
        "OLE_XPOS_CONTAINER",
        "OLE_YPOS_CONTAINER",
        "OLE_XSIZE_CONTAINER",
        "OLE_YSIZE_CONTAINER",
    ]
    .iter()
    .enumerate()
    {
        types.push(alias(name, container(0xBF03_0640 + i as u32), h, b(vt::R4)));
    }
    types.push(alias("OLE_HANDLE", ole(0x6650_4313), 0, b(vt::INT)));
    types.push(alias("OLE_OPTEXCLUSIVE", ole(0x6650_430B), 0, b(vt::BOOL)));
    types.push(alias(
        "OLE_CANCELBOOL",
        container(0xBF03_0644),
        0,
        b(vt::BOOL),
    ));
    types.push(alias(
        "OLE_ENABLEDEFAULTBOOL",
        container(0xBF03_0645),
        h,
        b(vt::BOOL),
    ));
    types.push(constants(
        "OLE_TRISTATE",
        ole(0x6650_430A),
        &[("Unchecked", 0), ("Checked", 1), ("Gray", 2)],
    ));
    types.push(alias("FONTNAME", ole(0x6650_430D), h, b(vt::BSTR)));
    types.push(alias("FONTSIZE", ole(0x6650_430E), h, b(vt::CY)));
    for (i, name) in [
        "FONTBOLD",
        "FONTITALIC",
        "FONTUNDERSCORE",
        "FONTSTRIKETHROUGH",
    ]
    .iter()
    .enumerate()
    {
        types.push(alias(name, ole(0x6650_430F + i as u32), h, b(vt::BOOL)));
    }
    // IFont.
    let font_ole = [0x8B, 0xBA, 0x00, 0xAA, 0x00, 0x30, 0x0C, 0xAB];
    let mut font = Vec::new();
    for (k, (name, param, t)) in [
        ("Name", "pname", b(vt::BSTR)),
        ("Size", "psize", b(vt::CY)),
        ("Bold", "pbold", b(vt::BOOL)),
        ("Italic", "pitalic", b(vt::BOOL)),
        ("Underline", "punderline", b(vt::BOOL)),
        ("Strikethrough", "pstrikethrough", b(vt::BOOL)),
        ("Weight", "pweight", b(vt::I2)),
        ("Charset", "pcharset", b(vt::I2)),
    ]
    .into_iter()
    .enumerate()
    {
        font.extend(get_put(name, param, 2 * k as i32, 3 + 2 * k as i16, t));
    }
    font.push(get("hFont", "phfont", 16, 19, user(T_OLE_HANDLE)));
    font.push(method(
        "Clone",
        17,
        20,
        invkind::FUNC,
        vec![p("ppfont", ptr(ptr(user(T_IFONT))), OUT)],
    ));
    font.push(method(
        "IsEqual",
        18,
        21,
        invkind::FUNC,
        vec![p("pfontOther", ptr(user(T_IFONT)), IN)],
    ));
    font.push(method(
        "SetRatio",
        19,
        22,
        invkind::FUNC,
        vec![
            p("cyLogical", b(vt::I4), IN),
            p("cyHimetric", b(vt::I4), IN),
        ],
    ));
    font.push(method(
        "AddRefHfont",
        20,
        23,
        invkind::FUNC,
        vec![p("hFont", user(T_OLE_HANDLE), IN)],
    ));
    font.push(method(
        "ReleaseHfont",
        21,
        24,
        invkind::FUNC,
        vec![p("hFont", user(T_OLE_HANDLE), IN)],
    ));
    let mut ifont = interface(
        "IFont",
        guid(0xBEF6_E002, 0xA874, 0x101A, font_ole),
        h,
        Some(T_IUNKNOWN),
        font,
    );
    ifont.doc = Some("Font Object".into());
    types.push(ifont);
    types.push(dispinterface(
        "Font",
        guid(0xBEF6_E003, 0xA874, 0x101A, font_ole),
        typeflag::DISPATCHABLE,
        vec![],
        vec![
            property("Name", 0, 0, b(vt::BSTR)),
            property("Size", 2, 0, b(vt::CY)),
            property("Bold", 3, 0, b(vt::BOOL)),
            property("Italic", 4, 0, b(vt::BOOL)),
            property("Underline", 5, 0, b(vt::BOOL)),
            property("Strikethrough", 6, 0, b(vt::BOOL)),
            property("Weight", 7, 0, b(vt::I2)),
            property("Charset", 8, 0, b(vt::I2)),
        ],
    ));
    types.push(alias("IFontDisp", [0; 16], 0, user(T_FONT)));
    let std_ole = [0x9D, 0xE3, 0x00, 0xAA, 0x00, 0x4B, 0xB8, 0x51];
    types.push(coclass(
        "StdFont",
        guid(0x0BE3_5203, 0x8F91, 0x11CE, std_ole),
        &[(T_FONT, 1), (T_FONTEVENTS, 3), (T_IFONT, 0)],
    ));
    // IPicture.
    let render_params = |flag: u16| {
        vec![
            p("hdc", b(vt::INT), flag),
            p("x", b(vt::I4), flag),
            p("y", b(vt::I4), flag),
            p("cx", b(vt::I4), flag),
            p("cy", b(vt::I4), flag),
            p("xSrc", user(T_XPOS_HIMETRIC), flag),
            p("ySrc", user(T_YPOS_HIMETRIC), flag),
            p("cxSrc", user(T_XSIZE_HIMETRIC), flag),
            p("cySrc", user(T_YSIZE_HIMETRIC), flag),
            p("prcWBounds", ptr(b(vt::VOID)), flag),
        ]
    };
    let mut ipicture = interface(
        "IPicture",
        guid(0x7BF8_0980, 0xBF32, 0x101A, OLE),
        h,
        Some(T_IUNKNOWN),
        vec![
            get("Handle", "phandle", 0, 3, user(T_OLE_HANDLE)),
            get("hPal", "phpal", 1, 4, user(T_OLE_HANDLE)),
            get("Type", "ptype", 2, 5, b(vt::I2)),
            get("Width", "pwidth", 3, 6, user(T_XSIZE_HIMETRIC)),
            get("Height", "pheight", 4, 7, user(T_YSIZE_HIMETRIC)),
            method("Render", 5, 8, invkind::FUNC, render_params(IN)),
            method(
                "hPal",
                1,
                9,
                invkind::PUT,
                vec![p("phpal", user(T_OLE_HANDLE), IN)],
            ),
            get("CurDC", "phdcOut", 7, 10, b(vt::INT)),
            method(
                "SelectPicture",
                8,
                11,
                invkind::FUNC,
                vec![
                    p("hdcIn", b(vt::INT), IN),
                    p("phdcOut", ptr(b(vt::INT)), OUT),
                    p("phbmpOut", ptr(user(T_OLE_HANDLE)), OUT),
                ],
            ),
            get("KeepOriginalFormat", "pfkeep", 9, 12, b(vt::BOOL)),
            method(
                "KeepOriginalFormat",
                9,
                13,
                invkind::PUT,
                vec![p("pfkeep", b(vt::BOOL), IN)],
            ),
            method("PictureChanged", 11, 14, invkind::FUNC, vec![]),
            method(
                "SaveAsFile",
                12,
                15,
                invkind::FUNC,
                vec![
                    p("pstm", ptr(b(vt::VOID)), IN),
                    p("fSaveMemCopy", b(vt::BOOL), IN),
                    p("pcbSize", ptr(b(vt::I4)), OUT),
                ],
            ),
            get("Attributes", "pdwAttr", 13, 16, b(vt::I4)),
            method(
                "SetHdc",
                14,
                17,
                invkind::FUNC,
                vec![p("hdc", user(T_OLE_HANDLE), IN)],
            ),
        ],
    );
    ipicture.doc = Some("Picture Object".into());
    types.push(ipicture);
    types.push(dispinterface(
        "Picture",
        guid(0x7BF8_0981, 0xBF32, 0x101A, OLE),
        typeflag::DISPATCHABLE,
        vec![func(
            "Render",
            6,
            F {
                kind: funckind::DISPATCH,
                flags: 0,
            },
            invkind::FUNC,
            0,
            b(vt::VOID),
            render_params(0),
        )],
        vec![
            property("Handle", 0, 1, user(T_OLE_HANDLE)),
            property("hPal", 2, 0, user(T_OLE_HANDLE)),
            property("Type", 3, 1, b(vt::I2)),
            property("Width", 4, 1, user(T_XSIZE_HIMETRIC)),
            property("Height", 5, 1, user(T_YSIZE_HIMETRIC)),
        ],
    ));
    types.push(alias("IPictureDisp", [0; 16], 0, user(T_PICTURE)));
    types.push(coclass(
        "StdPicture",
        guid(0x0BE3_5204, 0x8F91, 0x11CE, std_ole),
        &[(T_PICTURE, 1), (T_IPICTURE, 0)],
    ));
    let ie4 = [0x98, 0x5E, 0x00, 0xC0, 0x4F, 0xC2, 0x99, 0x93];
    types.push(constants(
        "LoadPictureConstants",
        guid(0xE6C8_FA08, 0xBD9F, 0x11D0, ie4),
        &[
            ("Default", 0),
            ("Monochrome", 1),
            ("VgaColor", 2),
            ("Color", 4),
        ],
    ));
    // StdFunctions.
    let opt = IN | paramflag::OPT;
    let def = IN | paramflag::OPT | paramflag::HASDEFAULT;
    let help = |mut f: Func, doc: &str, entry: &str| {
        f.doc = Some(doc.into());
        f.help_context = 10101;
        f.entry = Some(Entry::Name(entry.into()));
        f
    };
    let stat = || F {
        kind: funckind::STATIC,
        flags: 0,
    };
    let mut module = ty(
        tkind::MODULE,
        "StdFunctions",
        guid(
            0x9120_9AC0,
            0x60F6,
            0x11CF,
            [0x9C, 0x5D, 0x00, 0xAA, 0x00, 0xC1, 0x48, 0x9E],
        ),
        0,
    );
    module.size = 2;
    module.align = 1;
    module.doc = Some("Functions for Standard OLE Objects".into());
    module.help_context = 10101;
    module.dll = Some("oleaut32.dll".into());
    module.funcs = vec![
        help(
            func(
                "LoadPicture",
                0x6000_0000,
                stat(),
                invkind::FUNC,
                0,
                b(vt::HRESULT),
                vec![
                    p("filename", b(vt::VARIANT), opt),
                    with_default(p("widthDesired", b(vt::INT), def), Value::I4(0)),
                    with_default(p("heightDesired", b(vt::INT), def), Value::I4(0)),
                    with_default(p("flags", user(T_LOADPICTURE), def), Value::I4(0)),
                    p(
                        "retval",
                        ptr(ptr(user(T_IPICTUREDISP))),
                        OUT | paramflag::RETVAL,
                    ),
                ],
            ),
            "Loads a picture from a file",
            "OleLoadPictureFileEx",
        ),
        help(
            func(
                "SavePicture",
                0x6000_0001,
                stat(),
                invkind::FUNC,
                0,
                b(vt::HRESULT),
                vec![
                    p("Picture", ptr(user(T_IPICTUREDISP)), IN),
                    p("filename", b(vt::BSTR), IN),
                ],
            ),
            "Saves a picture to a file",
            "OleSavePictureFile",
        ),
    ];
    types.push(module);
    let mut events = dispinterface(
        "FontEvents",
        guid(
            0x4EF6_100A,
            0xAF88,
            0x11D0,
            [0x98, 0x46, 0x00, 0xC0, 0x4F, 0xC2, 0x99, 0x93],
        ),
        typeflag::DISPATCHABLE | h,
        vec![func(
            "FontChanged",
            9,
            F {
                kind: funckind::DISPATCH,
                flags: 0,
            },
            invkind::FUNC,
            0,
            b(vt::VOID),
            vec![p("PropertyName", b(vt::BSTR), IN)],
        )],
        vec![],
    );
    events.doc = Some("Event interface for the Font object".into());
    types.push(events);
    types.push(alias("IFontEventsDisp", [0; 16], 0, user(T_FONTEVENTS)));
    Library {
        guid: GUID,
        lcid: 0,
        syskind: SYS_WIN32,
        major: 2,
        minor: 0,
        flags: 0,
        name: "stdole".into(),
        doc: Some("OLE Automation".into()),
        help_context: 0,
        help_string_context: 0,
        help_file: None,
        types,
        imports: Vec::new(),
        files: Vec::new(),
        custom: Vec::new(),
    }
}
