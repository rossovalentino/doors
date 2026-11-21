//! The summary information property set (MS-OLEPS) of an installer
//! database: title, author, the platform;language template, the package
//! code, the schema and the source flags.
use crate::Error;
use alloc::{string::String, vec::Vec};

pub mod pid {
    pub const CODEPAGE: u32 = 1;
    pub const TITLE: u32 = 2;
    pub const SUBJECT: u32 = 3;
    pub const AUTHOR: u32 = 4;
    pub const KEYWORDS: u32 = 5;
    pub const COMMENTS: u32 = 6;
    pub const TEMPLATE: u32 = 7;
    pub const LAST_AUTHOR: u32 = 8;
    pub const REVISION: u32 = 9;
    pub const CREATED: u32 = 12;
    pub const SAVED: u32 = 13;
    pub const PAGE_COUNT: u32 = 14;
    pub const WORD_COUNT: u32 = 15;
    pub const APPLICATION: u32 = 18;
    pub const SECURITY: u32 = 19;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Property {
    Int(i32),
    Text(String),
    /// A FILETIME.
    Time(u64),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub properties: Vec<(u32, Property)>,
}
fn u16_at(b: &[u8], at: usize) -> Result<u16, Error> {
    Ok(u16::from_le_bytes(
        b.get(at..at + 2)
            .ok_or(Error::Malformed)?
            .try_into()
            .unwrap(),
    ))
}
fn u32_at(b: &[u8], at: usize) -> Result<u32, Error> {
    Ok(u32::from_le_bytes(
        b.get(at..at + 4)
            .ok_or(Error::Malformed)?
            .try_into()
            .unwrap(),
    ))
}
impl Summary {
    pub fn parse(b: &[u8]) -> Result<Self, Error> {
        if u16_at(b, 0)? != 0xfffe || u32_at(b, 24)? == 0 {
            return Err(Error::Malformed);
        }
        let set = u32_at(b, 44)? as usize;
        let count = u32_at(b, set + 4)? as usize;
        if count > 64 {
            return Err(Error::Malformed);
        }
        let mut out = Summary::default();
        let mut codepage = 1252;
        for i in 0..count {
            let id = u32_at(b, set + 8 + 8 * i)?;
            let at = set + u32_at(b, set + 12 + 8 * i)? as usize;
            let property = match u32_at(b, at)? {
                2 => Property::Int(i32::from(u16_at(b, at + 4)? as i16)),
                3 => Property::Int(u32_at(b, at + 4)? as i32),
                30 => {
                    let n = u32_at(b, at + 4)? as usize;
                    let raw = b.get(at + 8..at + 8 + n).ok_or(Error::Malformed)?;
                    let raw = raw.split(|c| *c == 0).next().unwrap_or(&[]);
                    Property::Text(crate::decode_text(raw, codepage))
                }
                64 => Property::Time(
                    u64::from(u32_at(b, at + 4)?) | u64::from(u32_at(b, at + 8)?) << 32,
                ),
                _ => continue,
            };
            if id == pid::CODEPAGE
                && let Property::Int(cp) = property
            {
                codepage = cp as u16 as u32;
            }
            out.properties.push((id, property));
        }
        Ok(out)
    }
    pub fn text(&self, id: u32) -> Option<&str> {
        self.properties.iter().find_map(|(i, p)| match p {
            Property::Text(t) if *i == id => Some(t.as_str()),
            _ => None,
        })
    }
    pub fn int(&self, id: u32) -> Option<i32> {
        self.properties.iter().find_map(|(i, p)| match p {
            Property::Int(v) if *i == id => Some(*v),
            _ => None,
        })
    }
    /// The package's platform (`x64`, `Intel`, `Arm64`) from the template.
    pub fn platform(&self) -> Option<&str> {
        self.text(pid::TEMPLATE)?.split(';').next()
    }
    /// Word count bit 1: files are in cabinets (else beside the package).
    pub fn compressed(&self) -> bool {
        self.int(pid::WORD_COUNT).is_some_and(|w| w & 2 != 0)
    }
}
