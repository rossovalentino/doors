#![allow(dead_code)]

use door_pe_loader::{ImageBase, ModuleSource};

pub const OPTIONAL: usize = 0x98;
pub const SECTION_TABLE: usize = 0x188;
pub const EXE_BASE: u64 = 0x1_4000_0000;
pub const DLL_BASE: u64 = 0x1_8000_0000;
pub const IAT: u32 = 0x2400;
pub const DATA: u32 = 0x5000;
pub const RELOC: u32 = 0x6000;

#[derive(Clone)]
pub struct Image {
    pub bytes: Vec<u8>,
    pub base: u64,
}
impl Image {
    pub fn new(dll: bool) -> Self {
        let mut image = Self {
            bytes: vec![0; 0x3a00],
            base: if dll { DLL_BASE } else { EXE_BASE },
        };
        image.bytes[..2].copy_from_slice(b"MZ");
        image.put32(0x3c, 0x80);
        image.bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        image.put16(0x84, 0x8664);
        image.put16(0x86, 4);
        image.put16(0x94, 240);
        image.put16(0x96, if dll { 0x2022 } else { 0x22 });
        image.put16(OPTIONAL, 0x20b);
        image.put32(OPTIONAL + 16, if dll { 0 } else { 0x1000 });
        image.put64(OPTIONAL + 24, image.base);
        image.put32(OPTIONAL + 32, 0x1000);
        image.put32(OPTIONAL + 36, 0x200);
        image.put32(OPTIONAL + 56, 0x7000);
        image.put32(OPTIONAL + 60, 0x400);
        image.put16(OPTIONAL + 68, 3);
        image.put16(OPTIONAL + 70, 0x160);
        image.put32(OPTIONAL + 108, 16);
        for (index, (name, rva, raw, bytes, flags)) in [
            (b".text\0\0\0", 0x1000, 0x400, 0x200, 0x6000_0020),
            (b".rdata\0\0", 0x2000, 0x600, 0x3000, 0x4000_0040),
            (b".data\0\0\0", DATA, 0x3600, 0x200, 0xc000_0040),
            (b".reloc\0\0", RELOC, 0x3800, 0x200, 0x4200_0040),
        ]
        .into_iter()
        .enumerate()
        {
            let section = SECTION_TABLE + index * 40;
            image.bytes[section..section + 8].copy_from_slice(name);
            image.put32(section + 8, bytes);
            image.put32(section + 12, rva);
            image.put32(section + 16, bytes);
            image.put32(section + 20, raw);
            image.put32(section + 36, flags);
        }
        image.bytes[0x400..0x600].fill(0xc3);
        image.rva64(DATA, image.base + 0x1010);
        image.relocations(&[0xa000, 0]);
        image
    }
    pub fn source<'a>(&'a self, name: &'a str, base: u64) -> ModuleSource<'a> {
        ModuleSource {
            name,
            bytes: &self.bytes,
            base: ImageBase(base),
        }
    }
    pub fn directory(&mut self, index: usize, rva: u32, bytes: u32) {
        self.put32(OPTIONAL + 112 + index * 8, rva);
        self.put32(OPTIONAL + 116 + index * 8, bytes);
    }
    pub fn imports(&mut self, module: &str, symbols: &[&str]) {
        self.directory(1, 0x2000, 40);
        self.rva32(0x2000, IAT);
        self.rva32(0x2000 + 12, 0x2100);
        self.rva32(0x2000 + 16, IAT);
        self.string(0x2100, module);
        for (index, symbol) in symbols.iter().enumerate() {
            let name = 0x2200 + index as u32 * 64;
            self.rva16(name, 0);
            self.string(name + 2, symbol);
            self.rva64(IAT + index as u32 * 8, u64::from(name));
        }
        self.rva64(IAT + symbols.len() as u32 * 8, 0);
    }
    pub fn import_ordinal(&mut self, module: &str, ordinal: u16) {
        self.imports(module, &["ignored"]);
        self.rva64(IAT, (1 << 63) | u64::from(ordinal));
    }
    pub fn exports(&mut self, symbols: &[(&str, u32)]) {
        self.directory(0, 0x3000, 0x600);
        self.rva32(0x3000 + 12, 0x33c0);
        self.string(0x33c0, "fixture.dll");
        self.rva32(0x3000 + 16, 1);
        self.rva32(0x3000 + 20, symbols.len() as u32);
        self.rva32(0x3000 + 24, symbols.len() as u32);
        self.rva32(0x3000 + 28, 0x3040);
        self.rva32(0x3000 + 32, 0x3080);
        self.rva32(0x3000 + 36, 0x30c0);
        for (index, (name, target)) in symbols.iter().enumerate() {
            self.rva32(0x3040 + index as u32 * 4, *target);
            let name_rva = 0x3100 + index as u32 * 64;
            self.string(name_rva, name);
            self.rva32(0x3080 + index as u32 * 4, name_rva);
            self.rva16(0x30c0 + index as u32 * 2, index as u16);
        }
    }
    pub fn relocations(&mut self, entries: &[u16]) {
        self.directory(5, RELOC, 8 + entries.len() as u32 * 2);
        self.rva32(RELOC, DATA);
        self.rva32(RELOC + 4, 8 + entries.len() as u32 * 2);
        for (index, entry) in entries.iter().enumerate() {
            self.rva16(RELOC + 8 + index as u32 * 2, *entry);
        }
    }
    pub fn unwind(&mut self, header: [u8; 4], codes: &[u16]) {
        self.directory(3, 0x3800, 12);
        self.rva32(0x3800, 0x1000);
        self.rva32(0x3804, 0x1100);
        self.rva32(0x3808, 0x3900);
        let offset = self.offset(0x3900);
        self.bytes[offset..offset + 4].copy_from_slice(&header);
        for (index, code) in codes.iter().enumerate() {
            self.rva16(0x3904 + index as u32 * 2, *code);
        }
    }
    pub fn offset(&self, rva: u32) -> usize {
        for index in 0..4 {
            let section = SECTION_TABLE + index * 40;
            let virtual_address =
                u32::from_le_bytes(self.bytes[section + 12..section + 16].try_into().unwrap());
            let raw =
                u32::from_le_bytes(self.bytes[section + 20..section + 24].try_into().unwrap());
            let length =
                u32::from_le_bytes(self.bytes[section + 16..section + 20].try_into().unwrap());
            if rva >= virtual_address && rva - virtual_address < length {
                return (raw + rva - virtual_address) as usize;
            }
        }
        panic!("fixture RVA out of range: {rva:x}");
    }
    pub fn string(&mut self, rva: u32, string: &str) {
        let offset = self.offset(rva);
        self.bytes[offset..offset + string.len()].copy_from_slice(string.as_bytes());
        self.bytes[offset + string.len()] = 0;
    }
    pub fn rva16(&mut self, rva: u32, value: u16) {
        self.put16(self.offset(rva), value);
    }
    pub fn rva32(&mut self, rva: u32, value: u32) {
        self.put32(self.offset(rva), value);
    }
    pub fn rva64(&mut self, rva: u32, value: u64) {
        self.put64(self.offset(rva), value);
    }
    pub fn put16(&mut self, offset: usize, value: u16) {
        self.bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
    pub fn put32(&mut self, offset: usize, value: u32) {
        self.bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    pub fn put64(&mut self, offset: usize, value: u64) {
        self.bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
}
