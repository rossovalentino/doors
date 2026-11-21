//! ORPC messages between an apartment's proxies and an object exporter,
//! over one link per connection. A message is cut into packet
//! payloads of at most 232 bytes: `[kind u8][last u8][0 u16][call u32]
//! [offset u32][total u32]` then up to `PART_BYTES` of the body; the
//! receiver joins them by call number. The packets of one message are sent
//! in order; those of different calls on one link may interleave.
use crate::Guid;
use crate::objref::StdObjRef;
use crate::value::{Malformed, Reader, Writer};
use alloc::vec::Vec;

/// One part per packet of an Init link (`door_service_protocol::com`).
pub const PAYLOAD_BYTES: usize = 232;
pub const PART_HEADER: usize = 16;
pub const PART_BYTES: usize = PAYLOAD_BYTES - PART_HEADER;
/// The largest message either side accepts.
pub const MAX_MESSAGE: usize = 4 << 20;

pub mod kind {
    /// A method call on an interface (IPID).
    pub const CALL: u8 = 1;
    pub const REPLY: u8 = 2;
    /// IRemUnknown::RemQueryInterface.
    pub const REM_QUERY_INTERFACE: u8 = 3;
    /// IRemUnknown::RemAddRef / RemRelease.
    pub const REM_ADD_REF: u8 = 4;
    pub const REM_RELEASE: u8 = 5;
}

/// The payloads one message becomes.
pub fn split(kind: u8, call: u32, body: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    loop {
        let n = (body.len() - offset).min(PART_BYTES);
        let last = offset + n == body.len();
        let mut p = Writer::new();
        p.u8(kind)
            .u8(u8::from(last))
            .u16(0)
            .u32(call)
            .u32(offset as u32)
            .u32(body.len() as u32)
            .raw(&body[offset..offset + n]);
        out.push(p.bytes);
        offset += n;
        if last {
            return out;
        }
    }
}
/// Messages one Joiner assembles at once.
pub const MAX_PARTIAL: usize = 8;
/// Joins payloads back into messages: `(kind, call, body)` once complete.
#[derive(Default)]
pub struct Joiner {
    partial: Vec<(u32, u8, Vec<u8>)>,
}
impl Joiner {
    pub fn push(&mut self, payload: &[u8]) -> Result<Option<(u8, u32, Vec<u8>)>, Malformed> {
        let mut r = Reader::new(payload);
        let kind = r.u8()?;
        let last = r.u8()? != 0;
        r.u16()?;
        let call = r.u32()?;
        let offset = r.u32()? as usize;
        let total = r.u32()? as usize;
        let data = r.rest();
        if total > MAX_MESSAGE
            || offset + data.len() > total
            || (last && offset + data.len() != total)
        {
            return Err(Malformed);
        }
        let at = self.partial.iter().position(|p| p.0 == call);
        let buffer = match at {
            Some(i) => &mut self.partial[i].2,
            None => {
                // A message grows as its parts arrive (nothing is reserved
                // for what a peer only announces), and a peer has few in
                // progress at once.
                if offset != 0 || self.partial.len() >= MAX_PARTIAL {
                    return Err(Malformed);
                }
                self.partial.push((call, kind, Vec::new()));
                let n = self.partial.len() - 1;
                &mut self.partial[n].2
            }
        };
        if buffer.len() != offset {
            return Err(Malformed);
        }
        buffer.extend_from_slice(data);
        if !last {
            return Ok(None);
        }
        let i = self
            .partial
            .iter()
            .position(|p| p.0 == call)
            .ok_or(Malformed)?;
        let (call, kind, body) = self.partial.remove(i);
        Ok(Some((kind, call, body)))
    }
}

/// A call: the target interface, the vtable slot, the marshaled
/// arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    pub ipid: Guid,
    pub method: u32,
    pub data: Vec<u8>,
}
impl Call {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.raw(&self.ipid).u32(self.method).blob(&self.data);
        w.bytes
    }
    pub fn decode(b: &[u8]) -> Result<Self, Malformed> {
        let mut r = Reader::new(b);
        let c = Self {
            ipid: r.guid()?,
            method: r.u32()?,
            data: r.blob()?.to_vec(),
        };
        if r.done() { Ok(c) } else { Err(Malformed) }
    }
}
/// The error object a failed call left (IErrorInfo's fields).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ErrorInfo {
    pub guid: Guid,
    pub source: Vec<u16>,
    pub description: Vec<u16>,
    pub help_file: Vec<u16>,
    pub help_context: u32,
}
/// A reply: `status` 0 when the call ran (its HRESULT is in `data`, as
/// the stub wrote it), else the RPC failure (RPC_E_*, RPC_S_* as HRESULT).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub status: u32,
    pub data: Vec<u8>,
    pub error: Option<ErrorInfo>,
}
impl Reply {
    pub fn failed(status: u32) -> Self {
        Self {
            status,
            data: Vec::new(),
            error: None,
        }
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(self.status).blob(&self.data);
        match &self.error {
            None => {
                w.u8(0);
            }
            Some(e) => {
                w.u8(1).raw(&e.guid);
                w.bstr(Some(&e.source))
                    .bstr(Some(&e.description))
                    .bstr(Some(&e.help_file))
                    .u32(e.help_context);
            }
        }
        w.bytes
    }
    pub fn decode(b: &[u8]) -> Result<Self, Malformed> {
        let mut r = Reader::new(b);
        let status = r.u32()?;
        let data = r.blob()?.to_vec();
        let error = if r.u8()? == 0 {
            None
        } else {
            Some(ErrorInfo {
                guid: r.guid()?,
                source: r.bstr()?.unwrap_or_default(),
                description: r.bstr()?.unwrap_or_default(),
                help_file: r.bstr()?.unwrap_or_default(),
                help_context: r.u32()?,
            })
        };
        if !r.done() {
            return Err(Malformed);
        }
        Ok(Self {
            status,
            data,
            error,
        })
    }
}
/// IRemUnknown::RemQueryInterface: the object (its IUnknown IPID), the
/// references each new interface carries, the IIDs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryInterface {
    pub ipid: Guid,
    pub refs: u32,
    pub iids: Vec<Guid>,
}
impl QueryInterface {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.raw(&self.ipid).u32(self.refs).u32(self.iids.len() as u32);
        for iid in &self.iids {
            w.raw(iid);
        }
        w.bytes
    }
    pub fn decode(b: &[u8]) -> Result<Self, Malformed> {
        let mut r = Reader::new(b);
        let ipid = r.guid()?;
        let refs = r.u32()?;
        let n = r.u32()? as usize;
        if n > 256 {
            return Err(Malformed);
        }
        let mut iids = Vec::with_capacity(n);
        for _ in 0..n {
            iids.push(r.guid()?);
        }
        if r.done() {
            Ok(Self { ipid, refs, iids })
        } else {
            Err(Malformed)
        }
    }
}
/// RemQueryInterface's results: per IID its HRESULT and, on success, the
/// interface's STDOBJREF.
pub fn encode_qi_results(results: &[(u32, Option<StdObjRef>)]) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(results.len() as u32);
    for (hr, std) in results {
        w.u32(*hr);
        if let Some(s) = std {
            w.u8(1)
                .u32(s.flags)
                .u32(s.refs)
                .u64(s.oxid)
                .u64(s.oid)
                .raw(&s.ipid);
        } else {
            w.u8(0);
        }
    }
    w.bytes
}
pub fn decode_qi_results(b: &[u8]) -> Result<Vec<(u32, Option<StdObjRef>)>, Malformed> {
    let mut r = Reader::new(b);
    let n = r.u32()? as usize;
    if n > 256 {
        return Err(Malformed);
    }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let hr = r.u32()?;
        let std = if r.u8()? == 0 {
            None
        } else {
            Some(StdObjRef {
                flags: r.u32()?,
                refs: r.u32()?,
                oxid: r.u64()?,
                oid: r.u64()?,
                ipid: r.guid()?,
            })
        };
        out.push((hr, std));
    }
    if r.done() { Ok(out) } else { Err(Malformed) }
}
/// RemAddRef / RemRelease: (IPID, references) pairs.
pub fn encode_refs(entries: &[(Guid, u32)]) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(entries.len() as u32);
    for (ipid, refs) in entries {
        w.raw(ipid).u32(*refs);
    }
    w.bytes
}
pub fn decode_refs(b: &[u8]) -> Result<Vec<(Guid, u32)>, Malformed> {
    let mut r = Reader::new(b);
    let n = r.u32()? as usize;
    if n > 256 {
        return Err(Malformed);
    }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push((r.guid()?, r.u32()?));
    }
    if r.done() { Ok(out) } else { Err(Malformed) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn messages_split_into_packets_and_join_back() {
        let body: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        let parts = split(kind::CALL, 9, &body);
        assert_eq!(parts.len(), 1000usize.div_ceil(PART_BYTES));
        assert!(parts.iter().all(|p| p.len() <= PAYLOAD_BYTES));
        let mut j = Joiner::default();
        let other = split(kind::REPLY, 3, b"x");
        for (i, p) in parts.iter().enumerate() {
            let r = j.push(p).unwrap();
            if i == 1 {
                // Another message completes in the middle.
                assert_eq!(
                    j.push(&other[0]).unwrap(),
                    Some((kind::REPLY, 3, b"x".to_vec()))
                );
            }
            if i + 1 < parts.len() {
                assert_eq!(r, None);
            } else {
                assert_eq!(r, Some((kind::CALL, 9, body.clone())));
            }
        }
        // An empty body is one packet.
        let empty = split(kind::REM_RELEASE, 1, &[]);
        assert_eq!(empty.len(), 1);
        assert_eq!(
            j.push(&empty[0]).unwrap(),
            Some((kind::REM_RELEASE, 1, Vec::new()))
        );
        // A continuation without its start, or out of order, is refused.
        assert!(Joiner::default().push(&parts[1]).is_err());
    }
    #[test]
    fn bodies_round_trip() {
        let c = Call {
            ipid: [4; 16],
            method: 7,
            data: alloc::vec![1, 2],
        };
        assert_eq!(Call::decode(&c.encode()), Ok(c));
        let r = Reply {
            status: 0,
            data: alloc::vec![0, 0, 0, 0],
            error: Some(ErrorInfo {
                guid: [1; 16],
                source: alloc::vec![0x44],
                description: alloc::vec![0x45, 0x46],
                help_file: alloc::vec![],
                help_context: 42,
            }),
        };
        assert_eq!(Reply::decode(&r.encode()), Ok(r));
        let q = QueryInterface {
            ipid: [5; 16],
            refs: 5,
            iids: alloc::vec![[6; 16], [7; 16]],
        };
        assert_eq!(QueryInterface::decode(&q.encode()), Ok(q));
        let results = alloc::vec![
            (
                0,
                Some(StdObjRef {
                    flags: 0,
                    refs: 5,
                    oxid: 1,
                    oid: 2,
                    ipid: [3; 16]
                })
            ),
            (0x8000_4002, None),
        ];
        assert_eq!(decode_qi_results(&encode_qi_results(&results)), Ok(results));
        let refs = alloc::vec![([8; 16], 1), ([9; 16], 5)];
        assert_eq!(decode_refs(&encode_refs(&refs)), Ok(refs));
    }
}
