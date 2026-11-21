//! Host-side import and relocation audit, independent of the compatibility mapper.
use door_image_format::{Machine, PeImage};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, path::Path};

fn u32_at(bytes: &[u8], at: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(
        bytes
            .get(at..at + 4)
            .ok_or("PE u32 extent")?
            .try_into()
            .unwrap(),
    ))
}
fn add(base: u32, offset: usize) -> Result<u32, String> {
    base.checked_add(u32::try_from(offset).map_err(|_| "PE offset")?)
        .ok_or("PE RVA overflow".into())
}
pub fn inspect(path: &Path) -> Result<Value, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    inspect_bytes(&bytes)
}
pub fn inspect_bytes(bytes: &[u8]) -> Result<Value, String> {
    // x64 (PE32+) or, for the 32-bit DLLs and programs (P17), x86 (PE32):
    // thunks and relocation targets are pointer-sized, ordinals flagged by
    // the pointer's top bit, relocations DIR64 or HIGHLOW.
    let image = PeImage::parse_any(bytes).map_err(|e| format!("PE metadata: {e:?}"))?;
    let width = image.machine.pointer_bytes();
    let (top, relocation_kind) = match image.machine {
        Machine::X64 => (63, 10),
        Machine::X86 => (31, 3),
    };
    let mut imports = Vec::new();
    let (rva, size) = image
        .directory(1)
        .map_err(|e| format!("PE imports: {e:?}"))?;
    let mut ended = size == 0;
    for index in 0..(size as usize / 20).min(32) {
        let row = image
            .rva_bytes(add(rva, index * 20)?, 20)
            .map_err(|e| format!("PE descriptor: {e:?}"))?;
        if row.iter().all(|b| *b == 0) {
            ended = true;
            break;
        }
        let lookup = u32_at(row, 0)?;
        let iat = u32_at(row, 16)?;
        let module = image
            .string(u32_at(row, 12)?)
            .map_err(|e| format!("PE module: {e:?}"))?;
        let mut symbols = Vec::new();
        let mut terminated = false;
        for symbol in 0..256 {
            let word = image
                .rva_bytes(
                    add(if lookup == 0 { iat } else { lookup }, symbol * width)?,
                    width,
                )
                .map_err(|e| format!("PE thunk: {e:?}"))?;
            let mut thunk_bytes = [0u8; 8];
            thunk_bytes[..width].copy_from_slice(word);
            let thunk = u64::from_le_bytes(thunk_bytes);
            if thunk == 0 {
                terminated = true;
                break;
            }
            if thunk >> top != 0 {
                if thunk & ((1u64 << top) - 1) & !0xffff != 0 {
                    return Err("PE ordinal reserved bits".into());
                }
                symbols.push(json!({"ordinal":thunk & 0xffff,"iat_rva":add(iat,symbol*width)?}));
            } else {
                let name = u32::try_from(thunk).map_err(|_| "PE import RVA width")?;
                let name = image
                    .string(add(name, 2)?)
                    .map_err(|e| format!("PE symbol: {e:?}"))?;
                symbols.push(json!({"name":name,"iat_rva":add(iat,symbol*width)?}));
            }
        }
        if !terminated {
            return Err("PE unterminated import thunks".into());
        }
        imports.push(json!({"module":module,"symbols":symbols}));
    }
    if !ended {
        return Err("PE unterminated import descriptors".into());
    }
    let (rva, size) = image
        .directory(5)
        .map_err(|e| format!("PE relocations: {e:?}"))?;
    let mut offset = 0usize;
    let mut targets = BTreeSet::new();
    while offset < size as usize {
        let header = image
            .rva_bytes(add(rva, offset)?, 8)
            .map_err(|e| format!("PE relocation header: {e:?}"))?;
        let page = u32_at(header, 0)?;
        let length = u32_at(header, 4)? as usize;
        if length < 8
            || !length.is_multiple_of(2)
            || offset
                .checked_add(length)
                .is_none_or(|end| end > size as usize)
            || page % 4096 != 0
        {
            return Err("PE relocation block extent".into());
        }
        for at in (8..length).step_by(2) {
            let entry = image
                .rva_bytes(add(rva, offset + at)?, 2)
                .map_err(|e| format!("PE relocation entry: {e:?}"))?;
            let entry = u16::from_le_bytes(entry.try_into().unwrap());
            if entry == 0 {
                continue;
            }
            if entry >> 12 != relocation_kind {
                return Err("PE relocation of another width".into());
            }
            let target = add(page, usize::from(entry & 4095))?;
            image
                .rva_bytes(target, width)
                .map_err(|e| format!("PE relocation target: {e:?}"))?;
            if !targets.insert(target) || targets.len() > 65536 {
                return Err("PE duplicate/excess relocation targets".into());
            }
        }
        offset += length;
    }
    let sections = (0..image.section_count()).map(|index| {
        let s=image.section(index).map_err(|e|format!("PE section: {e:?}"))?;
            Ok(json!({"rva":s.virtual_address,"virtual_bytes":s.virtual_bytes,"raw_offset":s.raw_offset,"raw_bytes":s.raw_bytes,"characteristics":s.flags}))
    }).collect::<Result<Vec<_>,String>>()?;
    Ok(
        json!({"machine":if width == 8 { "amd64" } else { "x86" },"imports":imports,"dir64_targets":targets,"preferred_base":image.image_base,"entry_rva":image.entry_rva,"sections":sections}),
    )
}

pub fn fixture(metadata: &Value) -> Result<(), String> {
    let imports = metadata["imports"].as_array().ok_or("fixture imports")?;
    if imports.len() != 1
        || !imports[0]["module"]
            .as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case("kernel32.dll"))
    {
        return Err("fixture must import only ordinary kernel32.dll".into());
    }
    let mut names = imports[0]["symbols"]
        .as_array()
        .ok_or("fixture symbols")?
        .iter()
        .map(|s| {
            s["name"]
                .as_str()
                .ok_or("fixture ordinal imports unsupported")
        })
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    if names
        != [
            "ExitProcess",
            "GetLastError",
            "GetStdHandle",
            "SetLastError",
            "WriteFile",
        ]
        || metadata["dir64_targets"]
            .as_array()
            .is_none_or(Vec::is_empty)
        || metadata["preferred_base"] != 0x140000000u64
    {
        return Err("fixture ordinary imports/required relocation differ".into());
    }
    Ok(())
}

/// Check the actual compiler outputs against the guest planner before booting.
/// Independent metadata above remains the evidence inventory and audit surface.
pub fn bundle(root: &Path, artifacts: &[Value]) -> Result<Value, String> {
    let names = ["windows-basic", "ntdll", "kernel32"];
    let bases = [0x142000000u64, 0x180000000, 0x182000000];
    let mut files = Vec::new();
    for name in names {
        let matching: Vec<_> = artifacts.iter().filter(|a| a["name"] == name).collect();
        if matching.len() != 1 {
            return Err("PE bundle artifact inventory".into());
        }
        files.push(
            fs::read(crate::paths::existing(
                root,
                matching[0]["path"]
                    .as_str()
                    .ok_or("PE bundle artifact path")?,
            )?)
            .map_err(|e| e.to_string())?,
        );
    }
    let names = ["windows-basic.exe", "ntdll.dll", "kernel32.dll"];
    let sources: Vec<_> = files
        .iter()
        .enumerate()
        .map(|(i, bytes)| door_pe_loader::ModuleSource {
            name: names[i],
            bytes,
            base: door_pe_loader::ImageBase(bases[i]),
        })
        .collect();
    let plan = door_pe_loader::BundlePlan::new(&sources)
        .map_err(|e| format!("actual Windows bundle rejected: {e:?}"))?;
    let mut runs = Vec::new();
    for index in 0..3 {
        plan.module(index).ok_or("PE bundle module")?.visit_runs(|run|{runs.push(json!({"module":run.module,"rva":run.rva.0,"address":run.address.0,"bytes":run.bytes,"permissions":format!("{:?}",run.permissions)}));Ok::<_,String>(())}).map_err(|e|format!("PE map walk: {e:?}"))?;
    }
    let mut fixups = Vec::new();
    plan.visit_fixups(|fixup|{fixups.push(json!({"module":fixup.module,"rva":fixup.rva.0,"address":fixup.address.0,"value":fixup.value,"kind":format!("{:?}",fixup.kind)}));Ok::<_,String>(())}).map_err(|e|format!("PE fixup walk: {e:?}"))?;
    let bootstrap = plan
        .resolve_executable_export(1, door_pe_loader::Symbol::Name("DoorProcessBootstrap"))
        .map_err(|e| format!("PE bootstrap export: {e:?}"))?;
    Ok(
        json!({"module_names":names,"bases":bases,"entry":plan.entry().0,"bootstrap":bootstrap.0,"dependency_order":plan.dependency_order(),"runs":runs,"fixups":fixups}),
    )
}
