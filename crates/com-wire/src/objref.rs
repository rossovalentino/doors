//! OBJREF ([MS-DCOM] 2.2.18): the marshaled form of an interface pointer.
//! A standard one names the object (OXID, OID, IPID) and its exporter's
//! string bindings (DUALSTRINGARRAY); Doors' exporters are reached through
//! one ncalrpc binding whose network address is the endpoint Init brokers.
//! A custom one carries the unmarshaler's CLSID and its data.
use crate::Guid;
use crate::value::{Reader, Writer};
use alloc::{string::String, vec::Vec};

/// "MEOW".
pub const SIGNATURE: u32 = 0x574f_454d;
pub const FLAGS_STANDARD: u32 = 1;
pub const FLAGS_HANDLER: u32 = 2;
pub const FLAGS_CUSTOM: u32 = 4;
/// STDOBJREF flags: the client need not ping (Doors never pings).
pub const SORF_NOPING: u32 = 0x1000;
/// The tower ID of ncalrpc ([MS-RPCE] 2.2.1.3 / C706 Appendix I).
pub const NCALRPC: u16 = 0x10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StdObjRef {
    pub flags: u32,
    /// Public references the OBJREF transfers.
    pub refs: u32,
    pub oxid: u64,
    pub oid: u64,
    pub ipid: Guid,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObjRef {
    Standard {
        iid: Guid,
        std: StdObjRef,
        /// The exporter's ncalrpc endpoint.
        endpoint: String,
    },
    Custom {
        iid: Guid,
        clsid: Guid,
        data: Vec<u8>,
    },
}
impl ObjRef {
    pub fn iid(&self) -> &Guid {
        match self {
            Self::Standard { iid, .. } | Self::Custom { iid, .. } => iid,
        }
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Self::Standard { iid, std, endpoint } => {
                w.u32(SIGNATURE).u32(FLAGS_STANDARD).raw(iid);
                w.u32(std.flags)
                    .u32(std.refs)
                    .u64(std.oxid)
                    .u64(std.oid)
                    .raw(&std.ipid);
                // DUALSTRINGARRAY: one STRINGBINDING (tower, network address,
                // NUL), the list's terminating NUL, then an empty security
                // binding list (its terminating NUL).
                let address: Vec<u16> = endpoint.encode_utf16().collect();
                let strings = 1 + address.len() + 1 + 1;
                w.u16((strings + 2) as u16).u16(strings as u16);
                w.u16(NCALRPC);
                for unit in &address {
                    w.u16(*unit);
                }
                w.u16(0).u16(0);
                w.u16(0).u16(0);
            }
            Self::Custom { iid, clsid, data } => {
                w.u32(SIGNATURE).u32(FLAGS_CUSTOM).raw(iid);
                w.raw(clsid).u32(0).u32(data.len() as u32).raw(data);
            }
        }
        w.bytes
    }
    /// An OBJREF at the start of `bytes`, and how many bytes it took.
    pub fn decode(bytes: &[u8]) -> Option<(Self, usize)> {
        let mut r = Reader::new(bytes);
        if r.u32().ok()? != SIGNATURE {
            return None;
        }
        let flags = r.u32().ok()?;
        let iid = r.guid().ok()?;
        let objref = match flags {
            FLAGS_STANDARD => {
                let std = StdObjRef {
                    flags: r.u32().ok()?,
                    refs: r.u32().ok()?,
                    oxid: r.u64().ok()?,
                    oid: r.u64().ok()?,
                    ipid: r.guid().ok()?,
                };
                let entries = r.u16().ok()? as usize;
                let security = r.u16().ok()? as usize;
                if security > entries {
                    return None;
                }
                let mut units = Vec::with_capacity(entries);
                for _ in 0..entries {
                    units.push(r.u16().ok()?);
                }
                let endpoint = ncalrpc_address(&units[..security])?;
                Self::Standard { iid, std, endpoint }
            }
            FLAGS_CUSTOM => {
                let clsid = r.guid().ok()?;
                let _extension = r.u32().ok()?;
                let size = r.u32().ok()? as usize;
                Self::Custom {
                    iid,
                    clsid,
                    data: r.raw(size).ok()?.to_vec(),
                }
            }
            _ => return None,
        };
        Some((objref, r.position()))
    }
}
/// The network address of the first ncalrpc STRINGBINDING.
fn ncalrpc_address(units: &[u16]) -> Option<String> {
    let mut i = 0;
    while i < units.len() && units[i] != 0 {
        let tower = units[i];
        let start = i + 1;
        let end = start + units[start..].iter().position(|&u| u == 0)?;
        if tower == NCALRPC {
            return String::from_utf16(&units[start..end]).ok();
        }
        i = end + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standard_and_custom_round_trip() {
        let std = ObjRef::Standard {
            iid: [1; 16],
            std: StdObjRef {
                flags: SORF_NOPING,
                refs: 5,
                oxid: 0x1234,
                oid: 7,
                ipid: [9; 16],
            },
            endpoint: "OLE0000002A".into(),
        };
        let bytes = std.encode();
        assert_eq!(&bytes[..4], b"MEOW");
        let mut longer = bytes.clone();
        longer.extend_from_slice(&[0xAA; 3]);
        assert_eq!(ObjRef::decode(&longer), Some((std.clone(), bytes.len())));
        let custom = ObjRef::Custom {
            iid: [2; 16],
            clsid: [3; 16],
            data: alloc::vec![1, 2, 3],
        };
        let b = custom.encode();
        assert_eq!(ObjRef::decode(&b), Some((custom, b.len())));
        assert_eq!(ObjRef::decode(&bytes[..bytes.len() - 1]), None);
        assert_eq!(ObjRef::decode(b"MEOW\x03\0\0\0"), None);
    }
}
