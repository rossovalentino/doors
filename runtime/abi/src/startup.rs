use crate::{
    HandleKind, MAJOR, MINOR, Status, USER_END, USER_LOW, get16, get32, get64, put16, put32, put64,
    rights,
};
pub const STARTUP_BYTES: usize = 256;
pub const STARTUP_MAGIC: [u8; 8] = *b"DOORUSR1";
pub mod role {
    pub const CLIENT: u64 = 1;
    pub const SERVER: u64 = 2;
    pub const MALICIOUS: u64 = 3;
    pub const SURVIVOR: u64 = 4;
    pub const PROBE: u64 = 5;
    pub const INVALID_RETURN: u64 = 6;
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Inherited {
    pub handle: u64,
    pub rights: u64,
    pub kind: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Startup {
    pub pid: u64,
    pub peer_pid: u64,
    pub role: u64,
    pub entry: u64,
    pub tls: u64,
    pub stack_top: u64,
    pub inherited: [Inherited; 8],
}
impl Startup {
    fn validate(&self) -> Result<(), Status> {
        if self.pid == 0
            || self.role == 0
            || !(USER_LOW..USER_END).contains(&self.entry)
            || !(USER_LOW..USER_END).contains(&self.tls)
            || !self.tls.is_multiple_of(16)
            || !(USER_LOW..USER_END).contains(&self.stack_top)
            || !self.stack_top.is_multiple_of(16)
        {
            return Err(Status::InvalidArgument);
        }
        for (i, grant) in self.inherited.iter().enumerate() {
            if grant.handle == 0 {
                if *grant != Inherited::default() {
                    return Err(Status::InvalidArgument);
                }
            } else if HandleKind::from_raw(grant.kind).is_none()
                || grant.rights & !rights::ALL != 0
                || self.inherited[..i]
                    .iter()
                    .any(|old| old.handle == grant.handle)
            {
                return Err(Status::InvalidArgument);
            }
        }
        Ok(())
    }
    pub fn encode(&self, bytes: &mut [u8]) -> Result<(), Status> {
        if bytes.len() != STARTUP_BYTES {
            return Err(Status::InvalidArgument);
        }
        self.validate()?;
        bytes.fill(0);
        bytes[..8].copy_from_slice(&STARTUP_MAGIC);
        put16(bytes, 8, MAJOR);
        put16(bytes, 10, MINOR);
        put32(bytes, 12, STARTUP_BYTES as u32);
        for (i, value) in [
            self.pid,
            self.peer_pid,
            self.role,
            self.entry,
            self.tls,
            self.stack_top,
        ]
        .iter()
        .enumerate()
        {
            put64(bytes, 16 + i * 8, *value);
        }
        for (i, grant) in self.inherited.iter().enumerate() {
            let offset = 64 + i * 24;
            put64(bytes, offset, grant.handle);
            put64(bytes, offset + 8, grant.rights);
            put64(bytes, offset + 16, grant.kind);
        }
        Ok(())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Status> {
        if bytes.len() != STARTUP_BYTES
            || bytes[..8] != STARTUP_MAGIC
            || get32(bytes, 12) != STARTUP_BYTES as u32
        {
            return Err(Status::InvalidArgument);
        }
        if get16(bytes, 8) != MAJOR || get16(bytes, 10) != MINOR {
            return Err(Status::VersionMismatch);
        }
        let mut result = Self {
            pid: get64(bytes, 16),
            peer_pid: get64(bytes, 24),
            role: get64(bytes, 32),
            entry: get64(bytes, 40),
            tls: get64(bytes, 48),
            stack_top: get64(bytes, 56),
            inherited: [Inherited::default(); 8],
        };
        for (i, grant) in result.inherited.iter_mut().enumerate() {
            let offset = 64 + i * 24;
            *grant = Inherited {
                handle: get64(bytes, offset),
                rights: get64(bytes, offset + 8),
                kind: get64(bytes, offset + 16),
            };
        }
        result.validate()?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Startup {
        Startup {
            pid: 1,
            peer_pid: 2,
            role: role::CLIENT,
            entry: 0x400000,
            tls: 0x600000,
            stack_top: 0x800000,
            inherited: [Inherited::default(); 8],
        }
    }
    #[test]
    fn roundtrip_truncation_reserved_slots_and_bad_bounds() {
        let mut bytes = [0; STARTUP_BYTES];
        let mut startup = fixture();
        startup.inherited[0] = Inherited {
            handle: 0x1122334455667788,
            rights: rights::READ,
            kind: 1,
        };
        startup.encode(&mut bytes).unwrap();
        assert_eq!(Startup::decode(&bytes), Ok(startup));
        assert_eq!(&bytes[64..72], &0x1122334455667788u64.to_le_bytes());
        for end in 0..STARTUP_BYTES {
            assert!(Startup::decode(&bytes[..end]).is_err());
        }
        let mut bad = bytes;
        bad[8] = 2;
        assert_eq!(Startup::decode(&bad), Err(Status::VersionMismatch));
        let mut bad = bytes;
        bad[88 + 8] = 1;
        assert!(Startup::decode(&bad).is_err());
        for value in [0, USER_END, 0xffff800000000000] {
            let mut bad = startup;
            bad.entry = value;
            assert!(bad.encode(&mut bytes).is_err());
        }
        let mut bad = startup;
        bad.inherited[1] = bad.inherited[0];
        assert!(bad.encode(&mut bytes).is_err());
        let mut bad = startup;
        bad.inherited[0].rights = 128;
        assert!(bad.encode(&mut bytes).is_err());
    }
}
