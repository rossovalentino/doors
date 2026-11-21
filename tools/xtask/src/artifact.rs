use crate::evidence::{Run, file_hash, write_json};
use door_image_format::{ElfImage, MAX_IMAGE_BYTES, PeImage};
use serde_json::{Value, json};
use std::{fs, path::Path};

pub const CLASSES: [&str; 6] = [
    "host",
    "firmware",
    "kernel",
    "native-user",
    "windows-dll",
    "windows-fixture",
];
pub fn inspect(path: &Path, class: &str) -> Result<Value, String> {
    if !CLASSES.contains(&class) {
        return Err("unknown artifact class".into());
    }
    let length = fs::metadata(path).map_err(|e| e.to_string())?.len();
    if length > MAX_IMAGE_BYTES as u64 {
        return Err("artifact exceeds 256 MiB".into());
    }
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let details = if matches!(class, "kernel" | "native-user") {
        let image = ElfImage::parse(&bytes).map_err(|e| format!("invalid static ELF64: {e:?}"))?;
        if class == "kernel" && image.entry < 0xffff_ffff_8000_0000 {
            return Err("kernel entry is below the fixed high link base".into());
        }
        if class == "native-user" && image.entry > 0x0000_7fff_ffff_ffff {
            return Err("native user entry is outside the user address half".into());
        }
        if class == "native-user"
            && (image.entry < 0x10000 || length > door_initfs::MAX_NATIVE_FILE as u64)
        {
            return Err("native image exceeds the protected loader profile".into());
        }
        let mut segments = Vec::new();
        let mut native_loads = 0;
        let mut native_pages = 0;
        let mut native_first = None;
        let mut native_end = 0;
        for index in 0..image.segment_count() {
            let segment = image.segment(index).map_err(|e| format!("{e:?}"))?;
            if segment.kind == 1
                && ((class == "kernel" && segment.virtual_address < 0xffff_ffff_8000_0000)
                    || (class == "native-user"
                        && segment.virtual_address + segment.memory_bytes > 0x0000_8000_0000_0000))
            {
                return Err("segment is outside the artifact personality's address half".into());
            }
            if class == "native-user" && segment.kind == 1 {
                let end = segment
                    .virtual_address
                    .checked_add(segment.memory_bytes)
                    .and_then(|end| end.checked_add(4095))
                    .ok_or("native rounded extent overflow")?
                    & !4095;
                native_loads += 1;
                native_pages += end - segment.virtual_address;
                let first = *native_first.get_or_insert(segment.virtual_address);
                if segment.virtual_address < 0x10000
                    || native_loads > 8
                    || native_pages > 16 * 1024 * 1024
                    || end - first > 16 * 1024 * 1024
                    || segment.virtual_address < native_end
                {
                    return Err(
                        "native segments exceed protected mapping bounds or share pages".into(),
                    );
                }
                native_end = end;
            }
            segments.push(json!({"kind":segment.kind,"flags":segment.flags,"file_offset":segment.offset,
                "virtual_address":segment.virtual_address,"file_bytes":segment.file_bytes,"memory_bytes":segment.memory_bytes,
                "zero_fill_bytes":segment.memory_bytes.saturating_sub(segment.file_bytes),"alignment":segment.alignment}));
        }
        json!({"format":"ELF64","machine":"amd64","entry":image.entry,"segments":segments,"imports":[],"exports":[],
            "relocation_model":"fixed_static","symbol_coverage":"entry and program headers; no public ELF export ABI"})
    } else {
        // Windows DLLs and programs may be x86 (PE32, P17); the rest is x64.
        let image = if matches!(class, "windows-dll" | "windows-fixture") {
            PeImage::parse_any(&bytes)
        } else {
            PeImage::parse(&bytes)
        }
        .map_err(|e| format!("invalid PE: {e:?}"))?;
        let (format, machine) = match image.machine {
            door_image_format::Machine::X64 => ("PE32+", "amd64"),
            door_image_format::Machine::X86 => ("PE32", "x86"),
        };
        let imports: Vec<_> = (0..image.import_count().map_err(|e| format!("{e:?}"))?)
            .map(|i| {
                image
                    .import_name(i)
                    .map(str::to_owned)
                    .map_err(|e| format!("{e:?}"))
            })
            .collect::<Result<_, _>>()?;
        let exports: Vec<_> = (0..image.export_count().map_err(|e| format!("{e:?}"))?)
            .map(|i| {
                image
                    .export_name(i)
                    .map(str::to_owned)
                    .map_err(|e| format!("{e:?}"))
            })
            .collect::<Result<_, _>>()?;
        if class == "firmware"
            && (image.subsystem != 10 || image.characteristics & 0x2000 != 0 || !imports.is_empty())
        {
            return Err("firmware must be an EFI application with no host DLL imports".into());
        }
        if class == "windows-dll" && image.characteristics & 0x2000 == 0 {
            return Err("Windows DLL artifact lacks DLL characteristic".into());
        }
        if matches!(class, "host" | "windows-fixture")
            && (!matches!(image.subsystem, 2 | 3) || image.characteristics & 0x2000 != 0)
        {
            return Err("expected a Windows console/GUI executable".into());
        }
        let relocation = image.directory(5).map_err(|e| format!("{e:?}"))?;
        json!({"format":format,"machine":machine,"entry_rva":image.entry_rva,"image_base":image.image_base,
            "subsystem":image.subsystem,"imports":imports,"exports":exports,"relocations":{"rva":relocation.0,"bytes":relocation.1},
            "symbol_coverage":"import DLL names and named exports; ordinal/thunk semantics remain for compatibility loader gates"})
    };
    Ok(
        json!({"schema_version":1,"record_kind":"artifact","class":class,"sha256":file_hash(path)?,"file_bytes":length,
        "abi_personality":match class {"host"=>"host-windows", "firmware"=>"UEFI", "kernel"=>"Door kernel entry", "native-user"=>"Door native user", _=>"Windows user ABI"},
        "inspection":details,"execution_state":"not_run"}),
    )
}
pub fn command(run: &Run, class: &str, path: &str) -> Result<(), String> {
    let path = crate::paths::existing(&run.root, path)?;
    let record = inspect(&path, class)?;
    write_json(&run.directory.join("artifact.json"), &record)?;
    println!(
        "Inspected {} ({class}); {}",
        path.display(),
        record["sha256"]
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_file_limit_accepts_embedded_hd_resources_but_remains_bounded() {
        let workspace = crate::test_support::Workspace::new();
        let path = workspace.root.join("resources.elf");
        let mut bytes = crate::image_fixtures::elf();
        crate::image_fixtures::put64(&mut bytes, 24, 0x400000);
        crate::image_fixtures::put64(&mut bytes, 80, 0x400000);
        bytes.resize(16 * 1024 * 1024, 0);
        fs::write(&path, &bytes).unwrap();
        assert!(inspect(&path, "native-user").is_ok());
        bytes.push(0);
        fs::write(&path, &bytes).unwrap();
        assert!(inspect(&path, "native-user").is_err());
    }
    #[test]
    fn native_inspection_enforces_null_guard_and_committed_span() {
        let workspace = crate::test_support::Workspace::new();
        let path = workspace.root.join("user.elf");
        let mut bytes = crate::image_fixtures::elf();
        crate::image_fixtures::put64(&mut bytes, 24, 0x400000);
        crate::image_fixtures::put64(&mut bytes, 80, 0x400000);
        fs::write(&path, &bytes).unwrap();
        assert!(inspect(&path, "native-user").is_ok());
        let mut bad = bytes.clone();
        crate::image_fixtures::put64(&mut bad, 24, 0);
        crate::image_fixtures::put64(&mut bad, 80, 0);
        fs::write(&path, bad).unwrap();
        assert!(inspect(&path, "native-user").is_err());
        crate::image_fixtures::put64(&mut bytes, 104, 16 * 1024 * 1024 + 1);
        fs::write(&path, bytes).unwrap();
        assert!(inspect(&path, "native-user").is_err());
    }
    #[test]
    fn artifact_personality_cannot_be_inferred_from_a_filename() {
        let workspace = crate::test_support::Workspace::new();
        let path = workspace.root.join("pretend.exe");
        fs::write(&path, crate::image_fixtures::elf()).unwrap();
        assert!(inspect(&path, "kernel").is_ok());
        assert!(inspect(&path, "host").is_err());
        assert!(inspect(&path, "native-user").is_err());
        fs::write(&path, crate::image_fixtures::pe()).unwrap();
        assert!(inspect(&path, "firmware").is_ok());
        assert!(inspect(&path, "host").is_err());
        assert!(inspect(&path, "windows-dll").is_err());
    }
}
