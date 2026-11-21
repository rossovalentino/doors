//! Writing Windows Installer databases: the compound file (version 3,
//! 512-byte sectors, the mini stream for streams under 4096 bytes, the
//! directory as a balanced tree), the string pool (code page 1252), the
//! `_Tables`/`_Columns` catalog, column-major tables with their rows sorted
//! by primary key, OBJECT streams (`Binary.Name`, cabinets) and the summary
//! information property set. What `Database::open` reads, and what Windows
//! Installer's own engine opens.
use crate::{Value, column, encode_name, summary::Property};
use alloc::{string::String, vec, vec::Vec};

/// Column type helpers (`_Columns.Type`), with the bits Windows
/// Installer's engine writes: `s72` 0x0d48, `l0` 0x0f00, `i2` 0x0502, `i4`
/// 0x0104, `v0` 0x0900.
pub mod kind {
    use crate::column::*;
    /// A string column of at most `width` characters (0: unlimited).
    pub const fn string(width: u16) -> u16 {
        0x0d00 | width
    }
    /// A localizable string column.
    pub const fn text(width: u16) -> u16 {
        0x0f00 | width
    }
    pub const I2: u16 = 0x0502;
    pub const I4: u16 = 0x0104;
    /// An OBJECT (stream) column.
    pub const BINARY: u16 = STRING | VALID;
    pub const fn key(k: u16) -> u16 {
        k | KEY
    }
    pub const fn null(k: u16) -> u16 {
        k | NULLABLE
    }
}

struct TableDef {
    name: String,
    columns: Vec<(String, u16)>,
    rows: Vec<Vec<Value>>,
}

/// A database under construction.
#[derive(Default)]
pub struct Builder {
    tables: Vec<TableDef>,
    streams: Vec<(String, Vec<u8>)>,
    summary: Vec<(u32, Property)>,
}

/// Windows-1252 bytes of `text` (characters outside it become `?`).
fn encode_1252(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| match c as u32 {
            0..=0x7f | 0xa0..=0xff => c as u8,
            0x20ac => 0x80,
            0x201a => 0x82,
            0x0192 => 0x83,
            0x201e => 0x84,
            0x2026 => 0x85,
            0x2020 => 0x86,
            0x2021 => 0x87,
            0x02c6 => 0x88,
            0x2030 => 0x89,
            0x0160 => 0x8a,
            0x2039 => 0x8b,
            0x0152 => 0x8c,
            0x017d => 0x8e,
            0x2018 => 0x91,
            0x2019 => 0x92,
            0x201c => 0x93,
            0x201d => 0x94,
            0x2022 => 0x95,
            0x2013 => 0x96,
            0x2014 => 0x97,
            0x02dc => 0x98,
            0x2122 => 0x99,
            0x0161 => 0x9a,
            0x203a => 0x9b,
            0x0153 => 0x9c,
            0x017e => 0x9e,
            0x0178 => 0x9f,
            _ => b'?',
        })
        .collect()
}

impl Builder {
    pub fn new() -> Self {
        Self::default()
    }
    /// Declare a table: `(name, kind)` per column, keys first.
    pub fn table(&mut self, name: &str, columns: &[(&str, u16)]) {
        self.tables.push(TableDef {
            name: String::from(name),
            columns: columns
                .iter()
                .map(|(n, k)| (String::from(*n), *k))
                .collect(),
            rows: Vec::new(),
        });
    }
    /// Add a row to `table` (one value per column; `Value::Stream(name)`
    /// in an OBJECT column refers to a stream added with `stream`).
    pub fn row(&mut self, table: &str, values: Vec<Value>) {
        let t = self
            .tables
            .iter_mut()
            .find(|t| t.name == table)
            .expect("row for an undeclared table");
        assert_eq!(t.columns.len(), values.len(), "{table}: column count");
        t.rows.push(values);
    }
    /// A row of strings and integers: `"text"`, `"#123"` an integer, `""`
    /// null.
    pub fn row_text(&mut self, table: &str, values: &[&str]) {
        let kinds: Vec<u16> = self
            .tables
            .iter()
            .find(|t| t.name == table)
            .expect("row for an undeclared table")
            .columns
            .iter()
            .map(|c| c.1)
            .collect();
        let values = values
            .iter()
            .zip(kinds)
            .map(|(v, k)| {
                if v.is_empty() {
                    Value::Null
                } else if k & column::STRING == 0 {
                    Value::Int(v.trim_start_matches('#').parse().expect("integer column"))
                } else {
                    Value::Str(String::from(*v))
                }
            })
            .collect();
        self.row(table, values);
    }
    /// An embedded stream by plain name (`Binary.Logo`, `product.cab`).
    pub fn stream(&mut self, name: &str, bytes: Vec<u8>) {
        self.streams.push((String::from(name), bytes));
    }
    pub fn summary(&mut self, id: u32, value: Property) {
        self.summary.retain(|p| p.0 != id);
        self.summary.push((id, value));
    }

    /// The finished package.
    pub fn build(&self) -> Vec<u8> {
        // The string pool: every name and string value once, ids in sorted
        // order (so sorting rows by id sorts them by text).
        let mut strings: Vec<String> = Vec::new();
        for t in &self.tables {
            strings.push(t.name.clone());
            for (n, _) in &t.columns {
                strings.push(n.clone());
            }
            for r in &t.rows {
                for v in r {
                    if let Value::Str(s) = v {
                        strings.push(s.clone());
                    }
                }
            }
        }
        strings.sort();
        strings.dedup();
        strings.retain(|s| !s.is_empty());
        let long_refs = strings.len() >= 0xffff;
        let id = |s: &str| -> u32 {
            strings.binary_search_by(|x| x.as_str().cmp(s)).unwrap() as u32 + 1
        };
        // Reference counts.
        let mut refs = vec![0u32; strings.len() + 1];
        let mut count = |s: &str| {
            if !s.is_empty() {
                refs[id(s) as usize] += 1;
            }
        };
        for t in &self.tables {
            count(&t.name);
            for (n, _) in &t.columns {
                count(&t.name);
                count(n);
            }
            for r in &t.rows {
                for v in r {
                    if let Value::Str(s) = v {
                        count(s);
                    }
                }
            }
        }
        let mut pool: Vec<u8> = Vec::new();
        let codepage: u32 = 1252;
        pool.extend_from_slice(&(codepage as u16).to_le_bytes());
        pool.extend_from_slice(
            &(((codepage >> 16) as u16) | if long_refs { 0x8000 } else { 0 }).to_le_bytes(),
        );
        let mut data: Vec<u8> = Vec::new();
        for (i, s) in strings.iter().enumerate() {
            let bytes = encode_1252(s);
            let r = refs[i + 1].min(0xffff) as u16;
            if bytes.len() > 0xffff {
                pool.extend_from_slice(&0u16.to_le_bytes());
                pool.extend_from_slice(&((bytes.len() >> 16) as u16).to_le_bytes());
                pool.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
                pool.extend_from_slice(&r.to_le_bytes());
            } else {
                pool.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
                pool.extend_from_slice(&r.to_le_bytes());
            }
            data.extend_from_slice(&bytes);
        }
        let string_bytes = if long_refs { 3 } else { 2 };

        let mut streams: Vec<(Vec<u16>, Vec<u8>)> = Vec::new();
        streams.push((encode_name("_StringPool", true), pool));
        streams.push((encode_name("_StringData", true), data));
        // The catalog.
        let mut tables_stream = Vec::new();
        let mut names: Vec<&TableDef> = self.tables.iter().collect();
        names.sort_by_key(|t| id(&t.name));
        for t in &names {
            push_value(&mut tables_stream, id(&t.name), string_bytes);
        }
        streams.push((encode_name("_Tables", true), tables_stream));
        let mut catalog: Vec<(u32, i32, u32, i32)> = Vec::new();
        for t in &names {
            for (n, (name, k)) in t.columns.iter().enumerate() {
                catalog.push((id(&t.name), n as i32 + 1, id(name), i32::from(*k)));
            }
        }
        let mut columns_stream = Vec::new();
        for c in &catalog {
            push_value(&mut columns_stream, c.0, string_bytes);
        }
        for c in &catalog {
            push_value(&mut columns_stream, (c.1 as u32) ^ 0x8000, 2);
        }
        for c in &catalog {
            push_value(&mut columns_stream, c.2, string_bytes);
        }
        for c in &catalog {
            push_value(&mut columns_stream, (c.3 as u32) ^ 0x8000, 2);
        }
        streams.push((encode_name("_Columns", true), columns_stream));
        // The tables, column-major, rows sorted by their keys.
        for t in &self.tables {
            let stored = |v: &Value, k: u16| -> u32 {
                match v {
                    Value::Null => 0,
                    Value::Str(s) if s.is_empty() => 0,
                    Value::Str(s) => id(s),
                    // An OBJECT column holds 1 when its stream exists.
                    Value::Stream(_) => 1,
                    Value::Int(i) if k & column::SIZE <= 2 => (*i as u32 ^ 0x8000) & 0xffff,
                    Value::Int(i) => *i as u32 ^ 0x8000_0000,
                }
            };
            let mut rows: Vec<Vec<u32>> = t
                .rows
                .iter()
                .map(|r| {
                    r.iter()
                        .zip(&t.columns)
                        .map(|(v, c)| stored(v, c.1))
                        .collect()
                })
                .collect();
            let keys = t
                .columns
                .iter()
                .take_while(|c| c.1 & column::KEY != 0)
                .count()
                .max(1);
            rows.sort_by(|a, b| a[..keys].cmp(&b[..keys]));
            let mut bytes = Vec::new();
            for (c, (_, k)) in t.columns.iter().enumerate() {
                let width = if k & !(column::NULLABLE | column::KEY) == kind::BINARY {
                    2
                } else if k & column::STRING != 0 {
                    string_bytes
                } else if k & column::SIZE <= 2 {
                    2
                } else {
                    4
                };
                for r in &rows {
                    push_value(&mut bytes, r[c], width);
                }
            }
            streams.push((encode_name(&t.name, true), bytes));
        }
        for (name, bytes) in &self.streams {
            streams.push((encode_name(name, false), bytes.clone()));
        }
        let summary: Vec<u16> = "\u{5}SummaryInformation".encode_utf16().collect();
        streams.push((summary, summary_stream(&self.summary)));
        compound_file(&streams)
    }
}

fn push_value(out: &mut Vec<u8>, value: u32, width: usize) {
    out.extend_from_slice(&value.to_le_bytes()[..width]);
}

/// The summary information property set (MS-OLEPS) with a code page.
fn summary_stream(properties: &[(u32, Property)]) -> Vec<u8> {
    const FMTID: [u8; 16] = [
        0xe0, 0x85, 0x9f, 0xf2, 0xf9, 0x4f, 0x68, 0x10, 0xab, 0x91, 0x08, 0x00, 0x2b, 0x27, 0xb3,
        0xd9,
    ];
    let mut list: Vec<(u32, Property)> = properties.to_vec();
    if !list.iter().any(|p| p.0 == crate::summary::pid::CODEPAGE) {
        list.insert(0, (crate::summary::pid::CODEPAGE, Property::Int(1252)));
    }
    list.sort_by_key(|p| p.0);
    let mut values: Vec<Vec<u8>> = Vec::new();
    for (id, p) in &list {
        let mut v = Vec::new();
        match p {
            Property::Int(i)
                if *id == crate::summary::pid::CODEPAGE || *id == crate::summary::pid::SECURITY =>
            {
                v.extend_from_slice(&2u32.to_le_bytes());
                v.extend_from_slice(&(*i as i16).to_le_bytes());
                v.extend_from_slice(&[0, 0]);
            }
            Property::Int(i) => {
                v.extend_from_slice(&3u32.to_le_bytes());
                v.extend_from_slice(&i.to_le_bytes());
            }
            Property::Text(t) => {
                let mut bytes = encode_1252(t);
                bytes.push(0);
                v.extend_from_slice(&30u32.to_le_bytes());
                v.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                v.extend_from_slice(&bytes);
                while v.len() % 4 != 0 {
                    v.push(0);
                }
            }
            Property::Time(t) => {
                v.extend_from_slice(&64u32.to_le_bytes());
                v.extend_from_slice(&t.to_le_bytes());
            }
        }
        values.push(v);
    }
    let header = 8 + 8 * list.len();
    let size = header + values.iter().map(Vec::len).sum::<usize>();
    let mut out = Vec::new();
    out.extend_from_slice(&0xfffeu16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    // Windows NT 6.2 as the originating system.
    out.extend_from_slice(&0x0002_0602u32.to_le_bytes());
    out.extend_from_slice(&[0; 16]);
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&FMTID);
    out.extend_from_slice(&48u32.to_le_bytes());
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(&(list.len() as u32).to_le_bytes());
    let mut at = header;
    for ((id, _), v) in list.iter().zip(&values) {
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&(at as u32).to_le_bytes());
        at += v.len();
    }
    for v in values {
        out.extend_from_slice(&v);
    }
    out
}

const SECTOR: usize = 512;
const MINI: usize = 64;
const CUTOFF: usize = 4096;
const END: u32 = 0xffff_fffe;
const FREE: u32 = 0xffff_ffff;
const FAT_SECT: u32 = 0xffff_fffd;
const DIFAT_SECT: u32 = 0xffff_fffc;
const NONE: u32 = 0xffff_ffff;
/// The root storage's class: an installer package.
const PACKAGE_CLSID: [u8; 16] = [
    0x84, 0x10, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];

/// The compound file's ordering of names: shorter first, then by
/// upper-cased code unit.
fn cfb_order(a: &[u16], b: &[u16]) -> core::cmp::Ordering {
    let up = |c: u16| {
        if (0x61..=0x7a).contains(&c) {
            c - 0x20
        } else {
            c
        }
    };
    a.len()
        .cmp(&b.len())
        .then_with(|| a.iter().map(|c| up(*c)).cmp(b.iter().map(|c| up(*c))))
}

/// A version 3 compound file holding `streams` in its root storage.
pub fn compound_file(streams: &[(Vec<u16>, Vec<u8>)]) -> Vec<u8> {
    let mut sorted: Vec<&(Vec<u16>, Vec<u8>)> = streams.iter().collect();
    sorted.sort_by(|a, b| cfb_order(&a.0, &b.0));
    // Mini stream: every small stream in 64-byte mini sectors.
    let mut mini_stream: Vec<u8> = Vec::new();
    let mut mini_fat: Vec<u32> = Vec::new();
    let mut starts: Vec<u32> = vec![0; sorted.len()];
    for (i, (_, bytes)) in sorted.iter().enumerate() {
        if bytes.is_empty() {
            starts[i] = END;
        } else if bytes.len() < CUTOFF {
            let first = mini_fat.len() as u32;
            let n = bytes.len().div_ceil(MINI);
            for k in 0..n {
                mini_fat.push(if k + 1 == n {
                    END
                } else {
                    first + k as u32 + 1
                });
            }
            mini_stream.extend_from_slice(bytes);
            mini_stream.resize(mini_fat.len() * MINI, 0);
            starts[i] = first;
        }
    }
    // Regular sectors: big streams, the mini stream, the mini FAT, the
    // directory, then the FAT and DIFAT.
    let mut sectors: Vec<Vec<u8>> = Vec::new();
    let mut fat: Vec<u32> = Vec::new();
    let place = |bytes: &[u8], sectors: &mut Vec<Vec<u8>>, fat: &mut Vec<u32>| -> u32 {
        if bytes.is_empty() {
            return END;
        }
        let first = sectors.len() as u32;
        let n = bytes.len().div_ceil(SECTOR);
        for k in 0..n {
            let mut s = bytes[k * SECTOR..((k + 1) * SECTOR).min(bytes.len())].to_vec();
            s.resize(SECTOR, 0);
            sectors.push(s);
            fat.push(if k + 1 == n {
                END
            } else {
                first + k as u32 + 1
            });
        }
        first
    };
    for (i, (_, bytes)) in sorted.iter().enumerate() {
        if bytes.len() >= CUTOFF {
            starts[i] = place(bytes, &mut sectors, &mut fat);
        }
    }
    let mini_start = place(&mini_stream, &mut sectors, &mut fat);
    let mut mini_fat_bytes = Vec::new();
    for v in &mini_fat {
        mini_fat_bytes.extend_from_slice(&v.to_le_bytes());
    }
    let mini_fat_sectors = mini_fat_bytes.len().div_ceil(SECTOR);
    mini_fat_bytes.resize(mini_fat_sectors * SECTOR, 0xff);
    let mini_fat_start = place(&mini_fat_bytes, &mut sectors, &mut fat);
    // Directory: the root, then the streams as a balanced tree.
    let mut entries: Vec<[u8; 128]> = Vec::new();
    let entry = |name: &[u16], kind: u8, start: u32, size: u64| -> [u8; 128] {
        let mut e = [0u8; 128];
        for (i, c) in name.iter().enumerate().take(31) {
            e[2 * i..2 * i + 2].copy_from_slice(&c.to_le_bytes());
        }
        e[64..66].copy_from_slice(&(((name.len().min(31) + 1) * 2) as u16).to_le_bytes());
        e[66] = kind;
        e[67] = 1; // black
        e[68..72].copy_from_slice(&NONE.to_le_bytes());
        e[72..76].copy_from_slice(&NONE.to_le_bytes());
        e[76..80].copy_from_slice(&NONE.to_le_bytes());
        e[116..120].copy_from_slice(&start.to_le_bytes());
        e[120..128].copy_from_slice(&size.to_le_bytes());
        e
    };
    let root_name: Vec<u16> = "Root Entry".encode_utf16().collect();
    let mut root = entry(
        &root_name,
        5,
        if mini_stream.is_empty() {
            END
        } else {
            mini_start
        },
        mini_stream.len() as u64,
    );
    root[80..96].copy_from_slice(&PACKAGE_CLSID);
    entries.push(root);
    for (i, (name, bytes)) in sorted.iter().enumerate() {
        entries.push(entry(name, 2, starts[i], bytes.len() as u64));
    }
    // Balanced binary tree over the sorted streams (entry ids 1..).
    fn link(entries: &mut [[u8; 128]], lo: usize, hi: usize) -> u32 {
        if lo >= hi {
            return NONE;
        }
        let mid = (lo + hi) / 2;
        let left = link(entries, lo, mid);
        let right = link(entries, mid + 1, hi);
        let e = &mut entries[mid + 1];
        e[68..72].copy_from_slice(&left.to_le_bytes());
        e[72..76].copy_from_slice(&right.to_le_bytes());
        (mid + 1) as u32
    }
    let top = link(&mut entries, 0, sorted.len());
    entries[0][76..80].copy_from_slice(&top.to_le_bytes());
    while !entries.len().is_multiple_of(4) {
        let mut empty = [0u8; 128];
        empty[68..80].fill(0xff);
        entries.push(empty);
    }
    let mut directory = Vec::new();
    for e in &entries {
        directory.extend_from_slice(e);
    }
    let directory_start = place(&directory, &mut sectors, &mut fat);
    // The FAT covers every sector including its own and the DIFAT's.
    let per = SECTOR / 4;
    let mut fat_sectors = 0usize;
    let mut difat_sectors = 0usize;
    loop {
        let total = sectors.len() + fat_sectors + difat_sectors;
        let need_fat = total.div_ceil(per);
        let need_difat = need_fat.saturating_sub(109).div_ceil(per - 1);
        if need_fat == fat_sectors && need_difat == difat_sectors {
            break;
        }
        fat_sectors = need_fat;
        difat_sectors = need_difat;
    }
    let fat_first = sectors.len() as u32;
    fat.resize(fat.len() + fat_sectors, FAT_SECT);
    let difat_first = fat_first + fat_sectors as u32;
    fat.resize(fat.len() + difat_sectors, DIFAT_SECT);
    fat.resize(fat_sectors * per, FREE);
    let mut header = vec![0u8; SECTOR];
    header[..8].copy_from_slice(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]);
    header[24..26].copy_from_slice(&0x3eu16.to_le_bytes());
    header[26..28].copy_from_slice(&3u16.to_le_bytes());
    header[28..30].copy_from_slice(&0xfffeu16.to_le_bytes());
    header[30..32].copy_from_slice(&9u16.to_le_bytes());
    header[32..34].copy_from_slice(&6u16.to_le_bytes());
    header[44..48].copy_from_slice(&(fat_sectors as u32).to_le_bytes());
    header[48..52].copy_from_slice(&directory_start.to_le_bytes());
    header[56..60].copy_from_slice(&(CUTOFF as u32).to_le_bytes());
    header[60..64].copy_from_slice(
        &(if mini_fat_sectors == 0 {
            END
        } else {
            mini_fat_start
        })
        .to_le_bytes(),
    );
    header[64..68].copy_from_slice(&(mini_fat_sectors as u32).to_le_bytes());
    header[68..72]
        .copy_from_slice(&(if difat_sectors == 0 { END } else { difat_first }).to_le_bytes());
    header[72..76].copy_from_slice(&(difat_sectors as u32).to_le_bytes());
    for i in 0..109 {
        let v = if i < fat_sectors {
            fat_first + i as u32
        } else {
            FREE
        };
        header[76 + 4 * i..80 + 4 * i].copy_from_slice(&v.to_le_bytes());
    }
    let mut out = header;
    for s in &sectors {
        out.extend_from_slice(s);
    }
    for chunk in fat.chunks(per) {
        for v in chunk {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    // DIFAT sectors: the FAT sectors past the header's 109.
    let rest: Vec<u32> = (109..fat_sectors).map(|i| fat_first + i as u32).collect();
    for d in 0..difat_sectors {
        let chunk = rest.iter().skip(d * (per - 1)).take(per - 1);
        let mut n = 0;
        for v in chunk {
            out.extend_from_slice(&v.to_le_bytes());
            n += 1;
        }
        for _ in n..per - 1 {
            out.extend_from_slice(&FREE.to_le_bytes());
        }
        let next = if d + 1 == difat_sectors {
            END
        } else {
            difat_first + d as u32 + 1
        };
        out.extend_from_slice(&next.to_le_bytes());
    }
    out
}
