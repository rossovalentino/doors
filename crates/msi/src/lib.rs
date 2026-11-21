#![no_std]
//! Windows Installer databases (.msi): the compound file (`cfb`), the
//! encoded stream names, the shared string pool, the column-major tables
//! with their `_Tables`/`_Columns` catalog, embedded streams (`Binary`,
//! cabinets) and the summary information (`summary`). `install` turns a
//! database into the work an installation does, `engine` runs a session's
//! sequences and actions over the machine (`execute::Target`) with the
//! rollback script of `journal`, `link` writes the shell links
//! CreateShortcuts makes, `write` builds databases and `fixtures` the
//! test packages built with it.
extern crate alloc;
pub mod cfb;
pub mod engine;
pub mod execute;
pub mod fixtures;
pub mod install;
pub mod journal;
pub mod link;
pub mod summary;
pub mod version;
pub mod write;

use alloc::{string::String, vec::Vec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not an installer database, or a structure is inconsistent.
    Malformed,
    NotFound,
}
impl From<cfb::Error> for Error {
    fn from(_: cfb::Error) -> Self {
        Error::Malformed
    }
}

/// Column type bits as `_Columns` stores them.
pub mod column {
    pub const SIZE: u16 = 0x00ff;
    pub const VALID: u16 = 0x0100;
    pub const LOCALIZABLE: u16 = 0x0200;
    pub const STRING: u16 = 0x0800;
    pub const NULLABLE: u16 = 0x1000;
    pub const KEY: u16 = 0x2000;
    pub const TEMPORARY: u16 = 0x4000;
}

/// One encoded stream-name character from `0..64` (MS-MSI's base64-like
/// alphabet: digits, letters, `.`, `_`).
fn from_mime(x: u16) -> u16 {
    match x {
        0..=9 => u16::from(b'0') + x,
        10..=35 => u16::from(b'A') + x - 10,
        36..=61 => u16::from(b'a') + x - 36,
        62 => u16::from(b'.'),
        _ => u16::from(b'_'),
    }
}
fn to_mime(c: u16) -> Option<u16> {
    match c {
        0x30..=0x39 => Some(c - 0x30),
        0x41..=0x5a => Some(c - 0x41 + 10),
        0x61..=0x7a => Some(c - 0x61 + 36),
        0x2e => Some(62),
        0x5f => Some(63),
        _ => None,
    }
}
/// The prefix of a table's stream name.
pub const TABLE_PREFIX: u16 = 0x4840;
/// A stream name as the compound file stores it: pairs of name characters
/// packed into one code unit (0x3800 + a + b*64), lone ones as 0x4800 + a,
/// a table's name after `TABLE_PREFIX`.
pub fn encode_name(name: &str, table: bool) -> Vec<u16> {
    let mut out = Vec::new();
    if table {
        out.push(TABLE_PREFIX);
    }
    let units: Vec<u16> = name.encode_utf16().collect();
    let mut i = 0;
    while i < units.len() {
        let c = units[i];
        match to_mime(c) {
            Some(a) => match units.get(i + 1).and_then(|n| to_mime(*n)) {
                Some(b) => {
                    out.push(0x3800 + a + (b << 6));
                    i += 2;
                }
                None => {
                    out.push(0x4800 + a);
                    i += 1;
                }
            },
            None => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}
/// The inverse of `encode_name`: (name, whether it names a table).
pub fn decode_name(units: &[u16]) -> (String, bool) {
    let (table, rest) = match units.split_first() {
        Some((&TABLE_PREFIX, rest)) => (true, rest),
        _ => (false, units),
    };
    let mut out = Vec::with_capacity(rest.len() * 2);
    for &c in rest {
        match c {
            0x3800..=0x47ff => {
                let v = c - 0x3800;
                out.push(from_mime(v & 0x3f));
                out.push(from_mime((v >> 6) & 0x3f));
            }
            0x4800..=0x483f => out.push(from_mime(c - 0x4800)),
            _ => out.push(c),
        }
    }
    (String::from_utf16_lossy(&out), table)
}

/// Bytes in the string pool's code page.
fn decode_text(bytes: &[u8], codepage: u32) -> String {
    match codepage {
        65001 => String::from_utf8_lossy(bytes).into_owned(),
        // Windows-1252 differs from Latin-1 in 0x80..0x9f.
        _ => bytes
            .iter()
            .map(|&b| match b {
                0x80 => '\u{20ac}',
                0x82 => '\u{201a}',
                0x83 => '\u{0192}',
                0x84 => '\u{201e}',
                0x85 => '\u{2026}',
                0x86 => '\u{2020}',
                0x87 => '\u{2021}',
                0x88 => '\u{02c6}',
                0x89 => '\u{2030}',
                0x8a => '\u{0160}',
                0x8b => '\u{2039}',
                0x8c => '\u{0152}',
                0x8e => '\u{017d}',
                0x91 => '\u{2018}',
                0x92 => '\u{2019}',
                0x93 => '\u{201c}',
                0x94 => '\u{201d}',
                0x95 => '\u{2022}',
                0x96 => '\u{2013}',
                0x97 => '\u{2014}',
                0x98 => '\u{02dc}',
                0x99 => '\u{2122}',
                0x9a => '\u{0161}',
                0x9b => '\u{203a}',
                0x9c => '\u{0153}',
                0x9e => '\u{017e}',
                0x9f => '\u{0178}',
                _ => char::from(b),
            })
            .collect(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub kind: u16,
}
impl Column {
    pub fn is_key(&self) -> bool {
        self.kind & column::KEY != 0
    }
    pub fn is_binary(&self) -> bool {
        self.kind & !(column::NULLABLE | column::KEY) == column::STRING | column::VALID
    }
    pub fn is_string(&self) -> bool {
        self.kind & column::STRING != 0 && !self.is_binary()
    }
    pub fn nullable(&self) -> bool {
        self.kind & column::NULLABLE != 0
    }
    /// Stored bytes of one value.
    fn width(&self, string_bytes: usize) -> usize {
        if self.is_binary() {
            2
        } else if self.kind & column::STRING != 0 {
            string_bytes
        } else if self.kind & column::SIZE <= 2 {
            2
        } else {
            4
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Null,
    Int(i32),
    Str(String),
    /// A binary column: the name of the stream holding its data
    /// (`Table.Key1.Key2`).
    Stream(String),
}
impl Value {
    pub fn str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn int(&self) -> Option<i32> {
        match self {
            Value::Int(v) => Some(*v),
            _ => None,
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub name: String,
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Value>>,
}
impl Table {
    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == name)
    }
    /// Row `row`'s value in column `name` (Null when the column is absent).
    pub fn get(&self, row: usize, name: &str) -> &Value {
        const NULL: Value = Value::Null;
        self.column(name)
            .and_then(|c| self.rows.get(row).and_then(|r| r.get(c)))
            .unwrap_or(&NULL)
    }
    /// The row whose first (key) column is `key`.
    pub fn find(&self, key: &str) -> Option<usize> {
        self.rows
            .iter()
            .position(|r| r.first().and_then(Value::str) == Some(key))
    }
}

pub struct Database<'a> {
    file: cfb::File<'a>,
    /// String id 1.. at index 1.. (id 0 is the null string).
    strings: Vec<String>,
    string_bytes: usize,
    pub codepage: u32,
    /// The catalog: every table's name and columns.
    pub tables: Vec<(String, Vec<Column>)>,
}

fn u16s(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

impl<'a> Database<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<Self, Error> {
        let file = cfb::File::parse(bytes)?;
        let read = |name: &str| -> Result<Vec<u8>, Error> {
            match file.stream(&encode_name(name, true)) {
                Some(e) => Ok(file.read(e)?),
                None => Ok(Vec::new()),
            }
        };
        let pool = u16s(&read("_StringPool")?);
        let data = read("_StringData")?;
        if pool.len() < 2 {
            return Err(Error::Malformed);
        }
        let long_refs = pool[1] & 0x8000 != 0;
        let codepage = u32::from(pool[0]) | u32::from(pool[1] & 0x7fff) << 16;
        let mut strings = alloc::vec![String::new()];
        let mut offset = 0usize;
        let mut i = 1;
        let entries = pool.len() / 2;
        while i < entries {
            let (length, refs) = (pool[2 * i], pool[2 * i + 1]);
            if length == 0 && refs == 0 {
                strings.push(String::new());
                i += 1;
                continue;
            }
            // Over 64 KiB: a zero length, the high word in the reference
            // count, the low word in the next entry.
            let length = if length == 0 {
                let low = *pool.get(2 * i + 2).ok_or(Error::Malformed)?;
                i += 2;
                (usize::from(refs) << 16) | usize::from(low)
            } else {
                i += 1;
                usize::from(length)
            };
            let text = data.get(offset..offset + length).ok_or(Error::Malformed)?;
            strings.push(decode_text(text, codepage));
            offset += length;
        }
        let mut db = Self {
            file,
            strings,
            string_bytes: if long_refs { 3 } else { 2 },
            codepage,
            tables: Vec::new(),
        };
        // The catalog's own schema is fixed.
        let s = column::STRING | column::VALID;
        let tables = db.read_table(
            "_Tables",
            &[Column {
                name: String::from("Name"),
                kind: s | column::KEY | 64,
            }],
        )?;
        let columns = db.read_table(
            "_Columns",
            &[
                Column {
                    name: String::from("Table"),
                    kind: s | column::KEY | 64,
                },
                Column {
                    name: String::from("Number"),
                    kind: column::VALID | column::KEY | 2,
                },
                Column {
                    name: String::from("Name"),
                    kind: s | 64,
                },
                Column {
                    name: String::from("Type"),
                    kind: column::VALID | 2,
                },
            ],
        )?;
        for row in &tables.rows {
            let Some(name) = row[0].str() else {
                return Err(Error::Malformed);
            };
            let mut list: Vec<(i32, Column)> = columns
                .rows
                .iter()
                .filter(|c| c[0].str() == Some(name))
                .map(|c| {
                    Ok((
                        c[1].int().ok_or(Error::Malformed)?,
                        Column {
                            name: String::from(c[2].str().ok_or(Error::Malformed)?),
                            kind: c[3].int().ok_or(Error::Malformed)? as u16,
                        },
                    ))
                })
                .collect::<Result<_, Error>>()?;
            list.sort_by_key(|c| c.0);
            db.tables
                .push((String::from(name), list.into_iter().map(|c| c.1).collect()));
        }
        Ok(db)
    }
    pub fn string(&self, id: u32) -> Option<&str> {
        self.strings.get(id as usize).map(String::as_str)
    }
    /// The table `name` with every row; NotFound when the catalog has none.
    pub fn table(&self, name: &str) -> Result<Table, Error> {
        let columns = self
            .tables
            .iter()
            .find(|t| t.0 == name)
            .map(|t| t.1.clone())
            .ok_or(Error::NotFound)?;
        self.read_table(name, &columns)
    }
    fn read_table(&self, name: &str, columns: &[Column]) -> Result<Table, Error> {
        let data = match self.file.stream(&encode_name(name, true)) {
            Some(e) => self.file.read(e)?,
            None => Vec::new(),
        };
        let widths: Vec<usize> = columns.iter().map(|c| c.width(self.string_bytes)).collect();
        let row_bytes: usize = widths.iter().sum();
        if row_bytes == 0 || data.len() % row_bytes != 0 {
            return Err(Error::Malformed);
        }
        let count = data.len() / row_bytes;
        let mut rows = alloc::vec![Vec::with_capacity(columns.len()); count];
        // Column-major: every row's first column, then the second, ...
        let mut at = 0;
        for (column, &width) in columns.iter().zip(&widths) {
            for row in rows.iter_mut() {
                let raw = &data[at..at + width];
                at += width;
                let mut value = 0u32;
                for (i, b) in raw.iter().enumerate() {
                    value |= u32::from(*b) << (8 * i);
                }
                row.push(if value == 0 {
                    Value::Null
                } else if column.is_binary() {
                    Value::Stream(String::new())
                } else if column.kind & column::STRING != 0 {
                    Value::Str(String::from(self.string(value).ok_or(Error::Malformed)?))
                } else if width == 2 {
                    Value::Int(i32::from((value ^ 0x8000) as u16 as i16))
                } else {
                    Value::Int((value ^ 0x8000_0000) as i32)
                });
            }
        }
        // A binary value's stream is named after the table and the row's
        // keys.
        for row in rows.iter_mut() {
            let mut stream = String::from(name);
            for (c, v) in columns.iter().zip(row.iter()) {
                if c.is_key() {
                    stream.push('.');
                    match v {
                        Value::Str(s) => stream.push_str(s),
                        Value::Int(i) => stream.push_str(&alloc::format!("{i}")),
                        _ => (),
                    }
                }
            }
            for (c, v) in columns.iter().zip(row.iter_mut()) {
                if c.is_binary() && matches!(v, Value::Stream(_)) {
                    *v = Value::Stream(stream.clone());
                }
            }
        }
        Ok(Table {
            name: String::from(name),
            columns: columns.to_vec(),
            rows,
        })
    }
    /// An embedded stream (`Binary.Name`, a cabinet), by its plain name.
    pub fn stream(&self, name: &str) -> Result<Vec<u8>, Error> {
        let entry = self
            .file
            .stream(&encode_name(name, false))
            .ok_or(Error::NotFound)?;
        Ok(self.file.read(entry)?)
    }
    /// Every stream's plain name, tables included (`_Streams`).
    pub fn streams(&self) -> Vec<(String, bool)> {
        self.file
            .children(0)
            .into_iter()
            .map(|i| &self.file.entries[i])
            .filter(|e| e.kind == 2)
            .map(|e| decode_name(&e.name))
            .collect()
    }
    /// The summary information stream.
    pub fn summary(&self) -> Result<summary::Summary, Error> {
        let name: Vec<u16> = "\u{5}SummaryInformation".encode_utf16().collect();
        let entry = self.file.stream(&name).ok_or(Error::NotFound)?;
        summary::Summary::parse(&self.file.read(entry)?)
    }
    /// A Property table value.
    pub fn property(&self, name: &str) -> Option<String> {
        let table = self.table("Property").ok()?;
        let row = table.find(name)?;
        table.get(row, "Value").str().map(String::from)
    }
}

#[cfg(test)]
mod execute_tests;
#[cfg(test)]
mod tests;
