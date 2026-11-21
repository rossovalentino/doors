//! Executables: PE (machine, subsystem, linker time, version resource) and
//! ELF (class, machine, type, linking), including Doors native programs.
use super::Section::{Description, Origin, Program};
use super::*;

fn pe_machine(m: u16) -> &'static str {
    match m {
        0x014C => "x86 (32-bit)",
        0x8664 => "x64",
        0x01C0 => "ARM",
        0x01C4 => "ARM Thumb-2",
        0xAA64 => "ARM64",
        0x0200 => "Itanium",
        0x5064 => "RISC-V (64-bit)",
        0x0EBC => "EFI byte code",
        0 => "Any",
        _ => "",
    }
}
/// How the program presents itself (the PE subsystem), in user terms.
fn interface(s: u16) -> &'static str {
    match s {
        1 => "Native",
        2 => "Desktop app",
        3 => "Command line",
        7 => "POSIX command line",
        9 => "Embedded",
        10 => "EFI application",
        11 => "EFI boot service driver",
        12 => "EFI runtime driver",
        16 => "Boot application",
        _ => "",
    }
}
fn language(id: &str) -> String {
    let name = match id.get(..4).map(str::to_ascii_lowercase).as_deref() {
        Some("0000") => "Language Neutral",
        Some("0409") => "English (United States)",
        Some("0809") => "English (United Kingdom)",
        Some("0407") => "German (Germany)",
        Some("040c") => "French (France)",
        Some("0410") => "Italian (Italy)",
        Some("0c0a") | Some("040a") => "Spanish (Spain)",
        Some("0411") => "Japanese",
        Some("0412") => "Korean",
        Some("0804") => "Chinese (Simplified)",
        Some("0404") => "Chinese (Traditional)",
        Some("0419") => "Russian",
        Some("0416") => "Portuguese (Brazil)",
        Some("0413") => "Dutch (Netherlands)",
        Some("041d") => "Swedish",
        Some("0415") => "Polish",
        _ => "",
    };
    if name.is_empty() {
        format!("Language ID {}", id.get(..4).unwrap_or(id))
    } else {
        String::from(name)
    }
}

pub(super) fn pe(d: &Sparse, r: &mut Report, ext: &str) -> Result<(), Miss> {
    if d.size() < 0x40 {
        r.format("MS-DOS program");
        return Ok(());
    }
    let pe = u64::from(d.le32(0x3C)?);
    if pe < 0x40 || pe + 24 > d.size() || d.get(pe, 4)? != b"PE\0\0" {
        r.format("MS-DOS program");
        return Ok(());
    }
    let coff = d.get(pe, 24)?;
    let machine = le16(coff, 4).unwrap_or(0);
    let sections = u64::from(le16(coff, 6).unwrap_or(0)).min(96);
    let stamp = le32(coff, 8).unwrap_or(0);
    let opt_size = u64::from(le16(coff, 20).unwrap_or(0));
    let flags = le16(coff, 22).unwrap_or(0);
    let opt_at = pe + 24;
    let opt = d.clip(opt_at, opt_size.min(240))?;
    let wide = le16(opt, 0) == Some(0x20B);
    let sub = le16(opt, 68).unwrap_or(0);
    let dirs = if wide { 112 } else { 96 };
    let count = le32(opt, dirs - 4).unwrap_or(0).min(16) as usize;
    let dir = |i: usize| -> (u64, u64) {
        if i >= count {
            return (0, 0);
        }
        (
            u64::from(le32(opt, dirs + 8 * i).unwrap_or(0)),
            u64::from(le32(opt, dirs + 8 * i + 4).unwrap_or(0)),
        )
    };
    let managed = dir(14).0 != 0;
    let kind = if flags & 0x2000 != 0 {
        "Dynamic-link library"
    } else if sub == 1 && ext == "sys" {
        "Driver"
    } else if matches!(sub, 10..=12) {
        "EFI application"
    } else {
        "Application"
    };
    r.format(match (kind, managed) {
        ("Dynamic-link library", true) => ".NET library",
        ("Dynamic-link library", false) => "Application extension (DLL)",
        ("Driver", _) => "System driver",
        ("EFI application", _) => "EFI application",
        (_, true) => ".NET application",
        _ => "Application",
    });
    r.add(Program, "Type", kind);
    r.add(Program, "Architecture", pe_machine(machine));
    r.add(
        Program,
        "Format",
        if wide { "PE32+ (64-bit)" } else { "PE32" },
    );
    r.add(Program, "Interface", interface(sub));
    if managed {
        r.add(Program, ".NET", "Yes (managed code)");
    }
    if let Some(when) = plausible_date(i64::from(stamp)) {
        r.add(Program, "Linker timestamp", when);
    }
    // Section table: RVA to file offset.
    let table = d.get(opt_at + opt_size, sections * 40)?.to_vec();
    let to_file = |rva: u64| -> Option<u64> {
        table.chunks_exact(40).find_map(|s| {
            let va = u64::from(le32(s, 12)?);
            let span = u64::from(le32(s, 8)?).max(u64::from(le32(s, 16)?));
            (va <= rva && rva < va + span).then(|| u64::from(le32(s, 20).unwrap_or(0)) + (rva - va))
        })
    };
    let (rva, _) = dir(2);
    let Some(base) = to_file(rva).filter(|_| rva != 0) else {
        return Ok(());
    };
    // Resource tree: type RT_VERSION (16) -> first name -> first language.
    let mut node = 0u64;
    for level in 0..3 {
        let h = d.get(base + node, 16)?;
        let n =
            (u64::from(le16(h, 12).unwrap_or(0)) + u64::from(le16(h, 14).unwrap_or(0))).min(256);
        let entries = d.get(base + node + 16, n * 8)?;
        let pick = entries
            .chunks_exact(8)
            .find(|e| level > 0 || le32(e, 0) == Some(16));
        let Some(e) = pick else { return Ok(()) };
        let target = le32(e, 4).unwrap_or(0);
        if level < 2 {
            if target & 0x8000_0000 == 0 {
                return Ok(());
            }
            node = u64::from(target & 0x7FFF_FFFF);
        } else {
            let data = d.get(base + u64::from(target & 0x7FFF_FFFF), 16)?;
            let (data_rva, len) = (
                u64::from(le32(data, 0).unwrap_or(0)),
                u64::from(le32(data, 4).unwrap_or(0)),
            );
            let Some(at) = to_file(data_rva) else {
                return Ok(());
            };
            let blob = d.clip(at, len.min(16_384))?;
            version_info(blob, r);
        }
    }
    Ok(())
}

/// A VS_VERSIONINFO-style block: (end, key, value start, value bytes, children start).
fn block(b: &[u8], at: usize) -> Option<(usize, String, usize, usize, usize)> {
    let len = usize::from(le16(b, at)?);
    let value_len = usize::from(le16(b, at + 2)?);
    let text = le16(b, at + 4)? == 1;
    if len < 6 {
        return None;
    }
    let end = (at + len).min(b.len());
    let mut key_end = at + 6;
    while key_end + 1 < end && le16(b, key_end)? != 0 {
        key_end += 2;
    }
    let key = utf16(b.get(at + 6..key_end)?, true);
    let value = (key_end + 2 + 3) & !3;
    let bytes = if text { value_len * 2 } else { value_len };
    let children = (value + bytes + 3) & !3;
    Some((end, key, value, bytes, children))
}
fn children(b: &[u8], start: usize, end: usize) -> Vec<(usize, String, usize, usize, usize)> {
    let mut out = Vec::new();
    let mut at = start;
    while at + 6 <= end && out.len() < 64 {
        let Some(c) = block(b, at) else { break };
        at = (c.0 + 3) & !3;
        if c.0 <= start {
            break;
        }
        out.push(c);
    }
    out
}
fn version_info(b: &[u8], r: &mut Report) {
    let Some((end, key, value, bytes, kids)) = block(b, 0) else {
        return;
    };
    if key != "VS_VERSION_INFO" {
        return;
    }
    if bytes >= 52 && le32(b, value) == Some(0xFEEF_04BD) {
        let v = |at: usize| le32(b, value + at).unwrap_or(0);
        let dotted =
            |ms: u32, ls: u32| format!("{}.{}.{}.{}", ms >> 16, ms & 0xFFFF, ls >> 16, ls & 0xFFFF);
        r.add(Program, "File version", dotted(v(8), v(12)));
        r.add(Program, "Product version", dotted(v(16), v(20)));
    }
    for (end, key, _, _, kids) in children(b, kids, end) {
        if key != "StringFileInfo" {
            continue;
        }
        for (end, table, _, _, kids) in children(b, kids, end) {
            r.add(Program, "Language", language(&table));
            for (_, key, value, bytes, _) in children(b, kids, end) {
                let text = utf16(
                    b.get(value..(value + bytes).min(b.len())).unwrap_or(&[]),
                    true,
                );
                match key.as_str() {
                    "FileDescription" => r.add(Description, "File description", text),
                    "ProductName" => r.add(Program, "Product name", text),
                    "CompanyName" => r.add(Origin, "Company", text),
                    "LegalCopyright" => r.add(Origin, "Copyright", text),
                    "OriginalFilename" => r.add(Program, "Original filename", text),
                    _ => (),
                }
            }
        }
    }
}

fn elf_machine(m: u16) -> &'static str {
    match m {
        2 => "SPARC",
        3 => "x86 (32-bit)",
        8 => "MIPS",
        20 => "PowerPC",
        21 => "PowerPC (64-bit)",
        22 => "IBM S/390",
        40 => "ARM",
        43 => "SPARC V9",
        62 => "x64",
        183 => "ARM64",
        243 => "RISC-V",
        258 => "LoongArch",
        _ => "",
    }
}
pub(super) fn elf(d: &Sparse, r: &mut Report) -> Result<(), Miss> {
    let h = d.clip(0, 64)?;
    let (class, little) = (h.get(4).copied().unwrap_or(0), h.get(5) == Some(&1));
    let wide = class == 2;
    if !matches!(class, 1 | 2) || h.len() < if wide { 64 } else { 52 } {
        r.format("ELF file");
        return Err(Miss::Truncated);
    }
    let u16_at = |at: usize| if little { le16(h, at) } else { be16(h, at) }.unwrap_or(0);
    let u32_at = |b: &[u8], at: usize| if little { le32(b, at) } else { be32(b, at) }.unwrap_or(0);
    let u64_at = |b: &[u8], at: usize| if little { le64(b, at) } else { be64(b, at) }.unwrap_or(0);
    let word = |b: &[u8], at32: usize, at64: usize| {
        if wide {
            u64_at(b, at64)
        } else {
            u64::from(u32_at(b, at32))
        }
    };
    let (kind, machine) = (u16_at(16), u16_at(18));
    let entry = word(h, 24, 24);
    let ph_offset = word(h, 28, 32);
    let ph_size = u64::from(u16_at(if wide { 54 } else { 42 }));
    let ph_count = u64::from(u16_at(if wide { 56 } else { 44 })).min(128);
    let (mut interpreter, mut dynamic, mut first_load) = (None, false, None);
    if (32..=64).contains(&ph_size) && ph_count > 0 {
        let table = d.get(ph_offset, ph_size * ph_count)?.to_vec();
        for p in table.chunks_exact(ph_size as usize) {
            match u32_at(p, 0) {
                1 if first_load.is_none() => first_load = Some(word(p, 8, 16)),
                2 => dynamic = true,
                3 => {
                    let (at, len) = (word(p, 4, 8), word(p, 16, 32));
                    interpreter = Some(utf8(d.clip(at, len.min(256))?));
                }
                _ => (),
            }
        }
    }
    let doors = wide
        && little
        && machine == 62
        && kind == 2
        && interpreter.is_none()
        && !dynamic
        && entry == 0x40_0000
        && first_load == Some(0x40_0000);
    let type_name = match kind {
        1 => "Relocatable object",
        2 => "Executable",
        3 if interpreter.is_some() => "Executable (position-independent)",
        3 => "Shared object",
        4 => "Core dump",
        _ => "",
    };
    r.format(if doors {
        "Doors native application"
    } else {
        match kind {
            1 => "ELF object file",
            3 if interpreter.is_none() => "ELF shared library",
            4 => "ELF core dump",
            _ => "ELF executable",
        }
    });
    if doors {
        r.add(Program, "Platform", "Doors (native program)");
    }
    r.add(Program, "Type", type_name);
    r.add(Program, "Architecture", elf_machine(machine));
    r.add(
        Program,
        "Format",
        format!(
            "ELF {}-bit, {}",
            if wide { 64 } else { 32 },
            if little {
                "little-endian"
            } else {
                "big-endian"
            }
        ),
    );
    r.add(
        Program,
        "OS ABI",
        match h[7] {
            3 => "Linux",
            6 => "Solaris",
            9 => "FreeBSD",
            12 => "OpenBSD",
            _ => "",
        },
    );
    r.add(
        Program,
        "Linking",
        match (&interpreter, dynamic) {
            (Some(i), _) => format!("Dynamic ({i})"),
            (None, true) => String::from("Dynamic"),
            (None, false) if kind == 2 => String::from("Static"),
            _ => String::new(),
        },
    );
    if kind == 2 || kind == 3 {
        r.add(Program, "Entry point", format!("0x{entry:X}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    fn utf16z(s: &str) -> Vec<u8> {
        s.encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect()
    }
    fn pad(v: &mut Vec<u8>) {
        while !v.len().is_multiple_of(4) {
            v.push(0);
        }
    }
    fn vblock(key: &str, value: &[u8], text: bool, kids: &[Vec<u8>]) -> Vec<u8> {
        let mut b = alloc::vec![0, 0];
        b.extend(le(
            if text {
                value.len() as u64 / 2
            } else {
                value.len() as u64
            },
            2,
        ));
        b.extend(le(u64::from(text), 2));
        b.extend(utf16z(key));
        pad(&mut b);
        b.extend(value);
        pad(&mut b);
        for k in kids {
            b.extend(k);
            pad(&mut b);
        }
        let len = b.len() as u16;
        b[..2].copy_from_slice(&len.to_le_bytes());
        b
    }
    fn pe_file() -> Vec<u8> {
        let mut f = alloc::vec![0u8; 0x80];
        f[..2].copy_from_slice(b"MZ");
        f[0x3C] = 0x80;
        f.extend(b"PE\0\0");
        f.extend(le(0x8664, 2));
        f.extend(le(1, 2));
        f.extend(le(1_714_570_507, 4));
        f.extend([0; 8]);
        f.extend(le(240, 2));
        f.extend(le(0x22, 2));
        let mut opt = alloc::vec![0u8; 240];
        opt[..2].copy_from_slice(&0x20Bu16.to_le_bytes());
        opt[68..70].copy_from_slice(&2u16.to_le_bytes());
        opt[108..112].copy_from_slice(&16u32.to_le_bytes());
        // Resource directory (2) at RVA 0x1000.
        opt[112 + 16..112 + 20].copy_from_slice(&0x1000u32.to_le_bytes());
        f.extend(opt);
        let mut section = alloc::vec![0u8; 40];
        section[..5].copy_from_slice(b".rsrc");
        section[8..12].copy_from_slice(&0x1000u32.to_le_bytes());
        section[12..16].copy_from_slice(&0x1000u32.to_le_bytes());
        section[16..20].copy_from_slice(&0x1000u32.to_le_bytes());
        section[20..24].copy_from_slice(&0x400u32.to_le_bytes());
        f.extend(section);
        f.resize(0x400, 0);
        // Resource tree at file 0x400 (RVA 0x1000).
        let dir = |id: u32, target: u32| {
            [
                alloc::vec![0; 12],
                le(0, 2),
                le(1, 2),
                le(u64::from(id), 4),
                le(u64::from(target), 4),
            ]
            .concat()
        };
        let mut rsrc = dir(16, 0x8000_0018);
        rsrc.extend(dir(1, 0x8000_0030));
        rsrc.extend(dir(0x409, 0x48));
        let mut fixed = le(0xFEEF_04BD, 4);
        fixed.extend(le(0x0001_0000, 4));
        fixed.extend(le(0x0002_0003, 4));
        fixed.extend(le(0x0004_0005, 4));
        fixed.extend(le(0x0002_0003, 4));
        fixed.extend(le(0x0004_0005, 4));
        fixed.resize(52, 0);
        let strings = [
            ("CompanyName", "Doors Labs"),
            ("FileDescription", "Sample tool"),
            ("ProductName", "Doors Tools"),
        ]
        .iter()
        .map(|(k, v)| vblock(k, &utf16z(v), true, &[]))
        .collect::<Vec<_>>();
        let table = vblock("040904b0", &[], true, &strings);
        let info = vblock(
            "VS_VERSION_INFO",
            &fixed,
            false,
            &[vblock("StringFileInfo", &[], true, &[table])],
        );
        // Data entry at 0x48: RVA of the blob (0x1000 + 0x58), size.
        rsrc.extend(le(0x1058, 4));
        rsrc.extend(le(info.len() as u64, 4));
        rsrc.extend([0; 8]);
        rsrc.extend(info);
        f.extend(rsrc);
        f
    }
    #[test]
    fn pe_header_and_version_resource() {
        let r = run("tool.exe", &pe_file());
        assert_eq!(r.format, Some("Application"));
        assert_eq!(
            (r.get("Architecture"), r.get("Format"), r.get("Interface")),
            (Some("x64"), Some("PE32+ (64-bit)"), Some("Desktop app"))
        );
        assert_eq!(r.get("Linker timestamp"), Some("2024-05-01 13:35:07 UTC"));
        assert_eq!(
            (r.get("File version"), r.get("Product version")),
            (Some("2.3.4.5"), Some("2.3.4.5"))
        );
        assert_eq!(
            (
                r.get("Company"),
                r.get("File description"),
                r.get("Product name")
            ),
            (Some("Doors Labs"), Some("Sample tool"), Some("Doors Tools"))
        );
        assert_eq!(r.get("Language"), Some("English (United States)"));
        fuzz("tool.exe", &pe_file());
        assert_eq!(run("dos.com", b"MZ\x90\0").format, Some("MS-DOS program"));
    }
    fn elf_file(doors: bool) -> Vec<u8> {
        let mut h = alloc::vec![0u8; 64];
        h[..4].copy_from_slice(b"\x7FELF");
        h[4] = 2;
        h[5] = 1;
        h[6] = 1;
        h[16..18].copy_from_slice(&(if doors { 2u16 } else { 3 }).to_le_bytes());
        h[18..20].copy_from_slice(&62u16.to_le_bytes());
        h[24..32].copy_from_slice(&0x40_0000u64.to_le_bytes());
        h[32..40].copy_from_slice(&64u64.to_le_bytes());
        h[54..56].copy_from_slice(&56u16.to_le_bytes());
        h[56..58].copy_from_slice(&2u16.to_le_bytes());
        let mut load = alloc::vec![0u8; 56];
        load[..4].copy_from_slice(&1u32.to_le_bytes());
        load[16..24].copy_from_slice(&0x40_0000u64.to_le_bytes());
        let mut second = alloc::vec![0u8; 56];
        if !doors {
            second[..4].copy_from_slice(&3u32.to_le_bytes());
            second[8..16].copy_from_slice(&176u64.to_le_bytes());
            second[32..40].copy_from_slice(&28u64.to_le_bytes());
        }
        [h, load, second, b"/lib64/ld-linux-x86-64.so.2\0".to_vec()].concat()
    }
    #[test]
    fn elf_doors_native_and_dynamic() {
        let r = run("door-workspace", &elf_file(true));
        assert_eq!(
            (r.format, r.get("Platform"), r.get("Linking")),
            (
                Some("Doors native application"),
                Some("Doors (native program)"),
                Some("Static")
            )
        );
        assert_eq!(
            (r.get("Architecture"), r.get("Format"), r.get("Entry point")),
            (
                Some("x64"),
                Some("ELF 64-bit, little-endian"),
                Some("0x400000")
            )
        );
        let r = run("ls", &elf_file(false));
        assert_eq!(
            (r.format, r.get("Type")),
            (
                Some("ELF executable"),
                Some("Executable (position-independent)")
            )
        );
        assert_eq!(
            r.get("Linking"),
            Some("Dynamic (/lib64/ld-linux-x86-64.so.2)")
        );
        fuzz("ls", &elf_file(false));
    }
}
