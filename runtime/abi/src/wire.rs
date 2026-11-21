use crate::{
    HEADER_BYTES, MAJOR, MAX_DIAGNOSTIC, MAX_PACKET, MAX_PAYLOAD, MAX_TRANSFERS, METADATA_BYTES,
    MINOR, QUERY_BYTES, QUEUE_DEPTH, STARTUP_BYTES, Status, TRANSFER_BYTES, get16, get32, get64,
    put16, put32, put64, rights,
};

pub const MAGIC: u32 = 0x524f_4f44;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HandleInfo {
    pub kind: u64,
    pub rights: u64,
    pub object_id: u64,
    pub generation: u64,
}
impl HandleInfo {
    pub fn encode(&self, bytes: &mut [u8]) -> Result<(), Status> {
        if bytes.len() != 32
            || crate::HandleKind::from_raw(self.kind).is_none()
            || self.rights & !rights::ALL != 0
            || self.object_id == 0
            || self.generation == 0
        {
            return Err(Status::InvalidArgument);
        }
        for (i, value) in [self.kind, self.rights, self.object_id, self.generation]
            .iter()
            .enumerate()
        {
            put64(bytes, i * 8, *value);
        }
        Ok(())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Status> {
        if bytes.len() != 32 {
            return Err(Status::InvalidArgument);
        }
        let result = Self {
            kind: get64(bytes, 0),
            rights: get64(bytes, 8),
            object_id: get64(bytes, 16),
            generation: get64(bytes, 24),
        };
        if crate::HandleKind::from_raw(result.kind).is_none()
            || result.rights & !rights::ALL != 0
            || result.object_id == 0
            || result.generation == 0
        {
            return Err(Status::InvalidArgument);
        }
        Ok(result)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Header {
    pub opcode: u32,
    pub flags: u32,
    pub request_id: u64,
    pub payload_bytes: u32,
    pub handle_count: u16,
}
impl Header {
    fn validate(&self) -> Result<(), Status> {
        if self.flags != 0
            || self.request_id == 0
            || self.payload_bytes as usize > MAX_PAYLOAD
            || self.handle_count as usize > MAX_TRANSFERS
        {
            return Err(Status::InvalidArgument);
        }
        Ok(())
    }
    pub fn encode(&self, bytes: &mut [u8]) -> Result<(), Status> {
        if bytes.len() != HEADER_BYTES {
            return Err(Status::InvalidArgument);
        }
        self.validate()?;
        bytes.fill(0);
        put32(bytes, 0, MAGIC);
        put16(bytes, 4, MAJOR);
        put16(bytes, 6, MINOR);
        put32(bytes, 8, self.opcode);
        put32(bytes, 12, self.flags);
        put64(bytes, 16, self.request_id);
        put32(bytes, 24, self.payload_bytes);
        put16(bytes, 28, self.handle_count);
        Ok(())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Status> {
        if bytes.len() != HEADER_BYTES || get32(bytes, 0) != MAGIC || get16(bytes, 30) != 0 {
            return Err(Status::InvalidArgument);
        }
        if get16(bytes, 4) != MAJOR || get16(bytes, 6) != MINOR {
            return Err(Status::VersionMismatch);
        }
        let result = Self {
            opcode: get32(bytes, 8),
            flags: get32(bytes, 12),
            request_id: get64(bytes, 16),
            payload_bytes: get32(bytes, 24),
            handle_count: get16(bytes, 28),
        };
        result.validate()?;
        Ok(result)
    }
    pub fn packet(bytes: &[u8]) -> Result<(Self, &[u8]), Status> {
        let header = Self::decode(bytes.get(..HEADER_BYTES).ok_or(Status::InvalidArgument)?)?;
        if bytes.len() != HEADER_BYTES + header.payload_bytes as usize {
            return Err(Status::InvalidArgument);
        }
        Ok((header, &bytes[HEADER_BYTES..]))
    }
}

/// Send transfer request: copy=0, move=1. Receive returns installed handles in
/// the same layout with mode=0 and effective rights. Reserved bytes are zero.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Transfer {
    pub handle: u64,
    pub rights: u64,
    pub mode: u64,
}
impl Transfer {
    pub fn encode(&self, bytes: &mut [u8]) -> Result<(), Status> {
        if bytes.len() != TRANSFER_BYTES
            || self.handle == 0
            || self.rights & !rights::ALL != 0
            || self.mode > 1
        {
            return Err(Status::InvalidArgument);
        }
        bytes.fill(0);
        put64(bytes, 0, self.handle);
        put64(bytes, 8, self.rights);
        put64(bytes, 16, self.mode);
        Ok(())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Status> {
        if bytes.len() != TRANSFER_BYTES || get64(bytes, 24) != 0 {
            return Err(Status::InvalidArgument);
        }
        let result = Self {
            handle: get64(bytes, 0),
            rights: get64(bytes, 8),
            mode: get64(bytes, 16),
        };
        if result.handle == 0 || result.rights & !rights::ALL != 0 || result.mode > 1 {
            return Err(Status::InvalidArgument);
        }
        Ok(result)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Metadata {
    pub sender_pid: u64,
    pub packet_bytes: u64,
    pub handle_count: u64,
    pub request_id: u64,
}
impl Metadata {
    pub fn encode(&self, bytes: &mut [u8]) -> Result<(), Status> {
        if bytes.len() != METADATA_BYTES {
            return Err(Status::InvalidArgument);
        }
        self.validate()?;
        for (i, value) in [
            self.sender_pid,
            self.packet_bytes,
            self.handle_count,
            self.request_id,
        ]
        .iter()
        .enumerate()
        {
            put64(bytes, i * 8, *value);
        }
        Ok(())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Status> {
        if bytes.len() != METADATA_BYTES {
            return Err(Status::InvalidArgument);
        }
        let result = Self {
            sender_pid: get64(bytes, 0),
            packet_bytes: get64(bytes, 8),
            handle_count: get64(bytes, 16),
            request_id: get64(bytes, 24),
        };
        result.validate()?;
        Ok(result)
    }
    fn validate(&self) -> Result<(), Status> {
        if self.sender_pid == 0
            || self.request_id == 0
            || !(HEADER_BYTES as u64..=MAX_PACKET as u64).contains(&self.packet_bytes)
            || self.handle_count > MAX_TRANSFERS as u64
        {
            return Err(Status::InvalidArgument);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AbiInfo {
    pub groups: u64,
    pub tick_hz: u64,
}
impl AbiInfo {
    pub const GROUPS: u64 =
        (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5) | (1 << 7);
    pub fn encode(&self, bytes: &mut [u8]) -> Result<(), Status> {
        if bytes.len() != QUERY_BYTES || self.tick_hz == 0 || self.groups & !Self::GROUPS != 0 {
            return Err(Status::InvalidArgument);
        }
        bytes.fill(0);
        put16(bytes, 0, MAJOR);
        put16(bytes, 2, MINOR);
        put16(bytes, 4, 64);
        put64(bytes, 8, self.groups);
        for (i, value) in [
            MAX_PAYLOAD,
            MAX_TRANSFERS,
            QUEUE_DEPTH,
            STARTUP_BYTES,
            MAX_DIAGNOSTIC,
        ]
        .iter()
        .enumerate()
        {
            put32(bytes, 16 + i * 4, *value as u32);
        }
        put64(bytes, 40, self.tick_hz);
        Ok(())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Status> {
        if bytes.len() != QUERY_BYTES {
            return Err(Status::InvalidArgument);
        }
        if get16(bytes, 0) != MAJOR || get16(bytes, 2) != MINOR {
            return Err(Status::VersionMismatch);
        }
        if get16(bytes, 4) != 64
            || get16(bytes, 6) != 0
            || get32(bytes, 36) != 0
            || bytes[48..].iter().any(|b| *b != 0)
        {
            return Err(Status::InvalidArgument);
        }
        for (i, value) in [
            MAX_PAYLOAD,
            MAX_TRANSFERS,
            QUEUE_DEPTH,
            STARTUP_BYTES,
            MAX_DIAGNOSTIC,
        ]
        .iter()
        .enumerate()
        {
            if get32(bytes, 16 + i * 4) != *value as u32 {
                return Err(Status::Unsupported);
            }
        }
        let result = Self {
            groups: get64(bytes, 8),
            tick_hz: get64(bytes, 40),
        };
        if result.tick_hz == 0 || result.groups & !Self::GROUPS != 0 {
            return Err(Status::InvalidArgument);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packets_enforce_lengths_versions_flags_counts_and_reserved_bytes() {
        let h = Header {
            opcode: 7,
            flags: 0,
            request_id: u64::MAX,
            payload_bytes: 3,
            handle_count: 4,
        };
        let mut packet = [0u8; 35];
        h.encode(&mut packet[..32]).unwrap();
        packet[32..].copy_from_slice(b"abc");
        assert_eq!(Header::packet(&packet), Ok((h, b"abc".as_slice())));
        for end in 0..packet.len() {
            assert!(Header::packet(&packet[..end]).is_err());
        }
        for (offset, value) in [(0, 0), (4, 2), (6, 1), (12, 1), (28, 5), (30, 1)] {
            let mut bad = packet;
            bad[offset] = value;
            assert!(Header::packet(&bad).is_err());
        }
        let mut bad = packet;
        put32(&mut bad, 24, 257);
        assert!(Header::packet(&bad).is_err());
        let mut bad = packet;
        put64(&mut bad, 16, 0);
        assert!(Header::packet(&bad).is_err());
        let mut large = [0; 36];
        large[..35].copy_from_slice(&packet);
        assert!(Header::packet(&large).is_err());
    }
    #[test]
    fn transfer_metadata_query_are_checked_before_access() {
        let t = Transfer {
            handle: u64::MAX,
            rights: rights::READ | rights::TRANSFER,
            mode: 1,
        };
        let mut b = [0; 32];
        let info = HandleInfo {
            kind: 1,
            rights: 9,
            object_id: 17,
            generation: 2,
        };
        info.encode(&mut b).unwrap();
        assert_eq!(HandleInfo::decode(&b), Ok(info));
        for end in 0..32 {
            assert!(HandleInfo::decode(&b[..end]).is_err());
        }
        let mut bad = b;
        put64(&mut bad, 24, 0);
        assert!(HandleInfo::decode(&bad).is_err());
        t.encode(&mut b).unwrap();
        assert_eq!(Transfer::decode(&b), Ok(t));
        for end in 0..32 {
            assert!(Transfer::decode(&b[..end]).is_err());
            assert!(Metadata::decode(&b[..end]).is_err());
        }
        for (offset, value) in [(16, 2), (24, 1)] {
            let mut bad = b;
            bad[offset] = value;
            assert!(Transfer::decode(&bad).is_err());
        }
        let mut bad = b;
        put64(&mut bad, 8, 128);
        assert!(Transfer::decode(&bad).is_err());
        let meta = Metadata {
            sender_pid: 2,
            packet_bytes: 40,
            handle_count: 1,
            request_id: 9,
        };
        meta.encode(&mut b).unwrap();
        assert_eq!(Metadata::decode(&b), Ok(meta));
        let info = AbiInfo {
            groups: AbiInfo::GROUPS,
            tick_hz: 100,
        };
        let mut q = [0; 64];
        info.encode(&mut q).unwrap();
        assert_eq!(AbiInfo::decode(&q), Ok(info));
        for end in 0..64 {
            assert!(AbiInfo::decode(&q[..end]).is_err());
        }
        for offset in [6, 36, 48, 63] {
            let mut bad = q;
            bad[offset] = 1;
            assert!(AbiInfo::decode(&bad).is_err());
        }
    }
    #[test]
    fn byte_mutations_never_panic() {
        let mut bytes = [0u8; 256];
        for offset in 0..bytes.len() {
            for value in [0, 1, 0x7f, 0xff] {
                bytes[offset] = value;
                let _ = Header::packet(&bytes);
                let _ = Transfer::decode(&bytes[..32]);
                let _ = Metadata::decode(&bytes[..32]);
                let _ = AbiInfo::decode(&bytes[..64]);
                let _ = crate::Startup::decode(&bytes);
            }
            bytes[offset] = 0;
        }
    }
}
