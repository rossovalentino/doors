//! The type-library views against Windows 11 26H2: oleautprobe.exe's
//! "tlb"/"std" lines for doorsprobe.tlb (MIDL-built from
//! tests/fixtures/windows-com/doorsprobe.idl) and its dump of System32's
//! stdole2.tlb, rendered here the way the probe prints them.
use super::*;
use alloc::{format, string::ToString, vec};

const DOORSPROBE: &[u8] = include_bytes!("../../tests/reference/doorsprobe.tlb");
const PROBE: &str = include_str!("../../tests/reference/oleautprobe.txt");
const STDOLE: &str = include_str!("../../tests/reference/stdole2.txt");

struct Host {
    libs: Vec<Library>,
}
impl Linker for Host {
    fn library(&self, id: usize) -> Option<&Library> {
        self.libs.get(id)
    }
    fn resolve(&self, id: usize, r: Ref) -> Option<(usize, u32)> {
        match r {
            Ref::Local(i) => Some((id, i)),
            Ref::Import(j) => {
                let lib = self.libs.get(id)?;
                let import = lib.imports.get(j as usize)?;
                let file = lib.files.get(import.file)?;
                let target = self.libs.iter().position(|l| l.guid == file.guid)?;
                let index = match import.guid {
                    Some(g) => self.libs[target].by_guid(&g)?,
                    None => import.index,
                };
                Some((target, index))
            }
        }
    }
}
fn host() -> Host {
    Host {
        libs: vec![parse(DOORSPROBE).unwrap(), stdole::library()],
    }
}

fn hexn(v: u64) -> String {
    format!("0x{v:x}")
}
fn ghex(g: &Guid) -> String {
    g.iter().map(|b| format!("{b:02x}")).collect()
}
fn bytes(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}
fn quoted(s: &Option<String>) -> String {
    match s {
        Some(t) => format!("\"{t}\""),
        None => "(null)".into(),
    }
}
fn show(v: &Value) -> String {
    let payload = match v {
        Value::Bstr(u) => format!("\"{}\"", String::from_utf16_lossy(u)),
        Value::I2(x) => x.to_string(),
        Value::I4(x) | Value::Int(x) => x.to_string(),
        Value::UI1(x) => x.to_string(),
        Value::Bool(x) => x.to_string(),
        Value::Error(x) => hexn(*x as u64),
        Value::R8(x) | Value::Date(x) => bytes(&x.to_le_bytes()),
        Value::Cy(x) => bytes(&x.to_le_bytes()),
        Value::R4(x) => bytes(&x.to_le_bytes()),
        Value::Empty | Value::Null => String::new(),
        _ => "?".into(),
    };
    format!("vt={} {payload}", v.vt())
}
fn tdesc(h: &Host, home: usize, t: &TypeDesc) -> String {
    match t {
        TypeDesc::Base(v) => v.to_string(),
        TypeDesc::Ptr(i) => format!("26({})", tdesc(h, home, i)),
        TypeDesc::SafeArray(i) => format!("27({})", tdesc(h, home, i)),
        TypeDesc::CArray(..) => "28[]".into(),
        TypeDesc::User(r) => match h.resolve(home, *r) {
            Some((l, i)) => format!("29({})", h.libs[l].types[i as usize].name),
            None => "29(?)".into(),
        },
    }
}
fn docs(v: &View, help_file: &Option<String>, memid: i32) -> String {
    let (name, doc, ctx) = v.documentation(memid).unwrap();
    format!(
        " doc 0x0 \"{name}\" {} ctx={ctx} file={}",
        quoted(&doc),
        quoted(help_file)
    )
}
fn names(v: &View, memid: i32) -> String {
    let mut s = String::from(" names 0x0");
    for n in v.names(memid).unwrap() {
        s.push(' ');
        s.push_str(&n);
    }
    s
}
fn typeinfo(h: &Host, v: &View, label: &str, out: &mut Vec<String>) {
    let help_file = &h.libs[v.lib].help_file;
    let mut s = format!(
        "{label} guid={} kind={} funcs={} vars={} impl={} vft={} size={} align={} flags={} ver={}.{} lcid=0 ctor=-1",
        ghex(&v.guid),
        v.kind,
        v.funcs.len(),
        v.vars.len(),
        v.impls.len(),
        v.vft,
        v.size,
        v.align,
        hexn(v.flags as u64),
        v.major,
        v.minor
    );
    if let Some(a) = &v.alias {
        s += &format!(" alias={}", tdesc(h, v.lib, a));
    }
    s += &docs(v, help_file, NIL);
    out.push(s);
    for (i, (home, imp)) in v.impls.iter().enumerate() {
        let (l, t) = h.resolve(*home, imp.target).unwrap();
        out.push(format!(
            "{label} impl {i} {} flags={}",
            h.libs[l].types[t as usize].name, imp.flags
        ));
    }
    for (i, (home, f)) in v.funcs.iter().enumerate() {
        let mut s = format!(
            "{label} func {i} id={} kind={} inv={} cc={} params={} opt={} vft={} flags={} ret={}",
            f.memid,
            f.kind,
            f.invoke,
            f.callconv,
            f.params.len(),
            f.optional,
            f.vft,
            hexn(f.flags as u64),
            tdesc(h, *home, &f.ret)
        );
        s += &names(v, f.memid);
        for p in &f.params {
            s += &format!(" [{} f={}", tdesc(h, *home, &p.ty), hexn(p.flags as u64));
            if p.flags & paramflag::HASDEFAULT != 0 {
                s += &format!(" def={}", show(p.default.as_ref().unwrap()));
            }
            s.push(']');
        }
        s += &docs(v, help_file, f.memid);
        if f.kind == funckind::STATIC {
            let entry = match &f.entry {
                Some(Entry::Name(n)) => format!("\"{n}\" 0"),
                Some(Entry::Ordinal(o)) => format!("(null) {o}"),
                None => "(null) 0".into(),
            };
            s += &format!(" entry 0x0 {} {entry}", quoted(&v.dll));
        }
        out.push(s);
    }
    for (i, var) in v.vars.iter().enumerate() {
        let mut s = format!(
            "{label} var {i} id={} kind={} flags={} type={}",
            var.memid,
            var.kind,
            hexn(var.flags as u64),
            tdesc(h, v.lib, &var.ty)
        );
        match &var.value {
            Some(value) => s += &format!(" value={}", show(value)),
            None => s += &format!(" offset={}", var.offset),
        }
        s += &names(v, var.memid);
        out.push(s);
    }
}
fn lib_line(lib: &Library) -> String {
    format!(
        "guid={} lcid={} sys={} ver={}.{} flags={}",
        ghex(&lib.guid),
        lib.lcid,
        lib.syskind,
        lib.major,
        lib.minor,
        hexn((lib.flags | HAS_DISK_IMAGE) as u64)
    )
}
fn compare(expected: Vec<&str>, got: Vec<String>) {
    for (i, (e, g)) in expected.iter().zip(got.iter()).enumerate() {
        assert_eq!(e, g, "line {i}");
    }
    assert_eq!(expected.len(), got.len(), "line count");
}

#[test]
fn doorsprobe_matches_windows() {
    let h = host();
    let lib = &h.libs[0];
    let mut out = vec![String::from("tlb load 0x0")];
    out.push(format!(
        "tlb lib {} \"{}\" {} count={}",
        lib_line(lib),
        lib.name,
        quoted(&lib.doc),
        lib.types.len()
    ));
    for i in 0..lib.types.len() as u32 {
        let v = view(&h, 0, i, Side::Natural).unwrap();
        typeinfo(&h, &v, &format!("tlb t{i}"), &mut out);
        if i == 2 {
            let vt = view(&h, 0, i, Side::Vtable).unwrap();
            typeinfo(&h, &vt, "tlb t2v", &mut out);
        }
    }
    let counter = lib.by_guid(&h.libs[0].types[3].guid);
    let mut missing = h.libs[0].types[3].guid;
    missing[0] = 0xff;
    out.push(format!(
        "tlb ofguid {} missing {} isname 0x0 1 {}",
        if counter.is_some() { "0x0" } else { "?" },
        if lib.by_guid(&missing).is_none() {
            hexn(hr::TYPE_E_ELEMENTNOTFOUND as u64)
        } else {
            "?".into()
        },
        lib.is_name("idoorsprobe").unwrap()
    ));
    // stdole2 as IDoorsProbe's vtable side reaches it.
    let vtable = view(&h, 0, 2, Side::Vtable).unwrap();
    let (dl, di) = h.resolve(0, vtable.impls[0].1.target).unwrap();
    let d = view(&h, dl, di, Side::Natural).unwrap();
    typeinfo(&h, &d, "std d", &mut out);
    let (ul, ui) = h.resolve(dl, d.impls[0].1.target).unwrap();
    let u = view(&h, ul, ui, Side::Natural).unwrap();
    typeinfo(&h, &u, "std u", &mut out);
    let std = &h.libs[dl];
    out.push(format!(
        "std lib {} count={} index={di}",
        lib_line(std),
        std.types.len()
    ));
    for (k, t) in std.types.iter().enumerate() {
        out.push(format!("std t{k} kind={} {}", t.kind, t.name));
    }
    let expected: Vec<&str> = PROBE
        .lines()
        .filter(|l| {
            l.starts_with("tlb ") || (l.starts_with("std ") && !l.starts_with("std create"))
        })
        .collect();
    compare(expected, out);
}

#[test]
fn stdole2_matches_windows() {
    let h = host();
    let lib = &h.libs[1];
    let mut out = vec![String::from("load 0x0")];
    out.push(format!(
        "lib {} \"{}\" {} ctx=0 file=(null) count={}",
        lib_line(lib),
        lib.name,
        quoted(&lib.doc),
        lib.types.len()
    ));
    for i in 0..lib.types.len() as u32 {
        let v = view(&h, 1, i, Side::Natural).unwrap();
        typeinfo(&h, &v, &format!("t{i}"), &mut out);
    }
    compare(STDOLE.lines().collect(), out);
}

#[test]
fn hreftypes_round_trip() {
    for r in [Ref::Local(0), Ref::Local(7), Ref::Import(0), Ref::Import(3)] {
        assert_eq!(Ref::from_href(r.href()), Some(r));
    }
    assert_eq!(Ref::from_href(2), None);
}

#[test]
fn rejects_other_files() {
    assert_eq!(parse(b"SLTG...."), Err(hr::TYPE_E_CANTLOADLIBRARY));
    assert_eq!(parse(&DOORSPROBE[..200]), Err(hr::TYPE_E_CANTLOADLIBRARY));
    assert!(resource(DOORSPROBE, 1).is_none());
}
