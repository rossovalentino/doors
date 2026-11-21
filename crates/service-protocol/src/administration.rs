//! Read-only inventory from the authenticated volume owner; descriptions confer no authority.
use crate::Code;
/// Allocation usage of the mounted home volume, read from the filesystem's
/// own free-block accounting (no estimate). Free never exceeds total.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VolumeUsage {
    pub total_bytes: u64,
    pub free_bytes: u64,
}
impl VolumeUsage {
    pub const BYTES: usize = 16;
    pub fn encode(self) -> [u8; 16] {
        let mut out = [0; 16];
        out[..8].copy_from_slice(&self.total_bytes.to_le_bytes());
        out[8..].copy_from_slice(&self.free_bytes.to_le_bytes());
        out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Code> {
        if bytes.len() != Self::BYTES {
            return Err(Code::InvalidRequest);
        }
        let number = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
        let usage = Self {
            total_bytes: number(0),
            free_bytes: number(8),
        };
        if usage.free_bytes > usage.total_bytes {
            return Err(Code::InvalidRequest);
        }
        Ok(usage)
    }
    pub fn used_bytes(self) -> u64 {
        self.total_bytes - self.free_bytes
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiskInfo {
    pub sectors: u64,
    pub volume_first: u64,
    pub volume_sectors: u64,
    pub journal_first: u64,
    pub journal_sectors: u64,
    pub writable: bool,
    pub flush: bool,
}
impl DiskInfo {
    pub const BYTES: usize = 48;
    pub fn encode(self) -> [u8; 48] {
        let mut out = [0; 48];
        for (chunk, number) in out[..40].chunks_exact_mut(8).zip([
            self.sectors,
            self.volume_first,
            self.volume_sectors,
            self.journal_first,
            self.journal_sectors,
        ]) {
            chunk.copy_from_slice(&number.to_le_bytes());
        }
        out[40] = u8::from(self.writable);
        out[41] = u8::from(self.flush);
        out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Code> {
        if bytes.len() != 48 || bytes[40] > 1 || bytes[41] > 1 || bytes[42..] != [0; 6] {
            return Err(Code::InvalidRequest);
        }
        let number = |at| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
        let info = Self {
            sectors: number(0),
            volume_first: number(8),
            volume_sectors: number(16),
            journal_first: number(24),
            journal_sectors: number(32),
            writable: bytes[40] != 0,
            flush: bytes[41] != 0,
        };
        let end = |first: u64, count: u64| {
            first
                .checked_add(count)
                .filter(|end| count > 0 && first >= 34 && *end < info.sectors)
        };
        let a = end(info.volume_first, info.volume_sectors).ok_or(Code::InvalidRequest)?;
        let b = end(info.journal_first, info.journal_sectors).ok_or(Code::InvalidRequest)?;
        if info.sectors.checked_mul(512).is_none()
            || !(a <= info.journal_first || b <= info.volume_first)
        {
            return Err(Code::InvalidRequest);
        }
        Ok(info)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inventory_rejects_reserved_bits_overflow_overlap_and_incomplete_ranges() {
        let info = DiskInfo {
            sectors: 262144,
            volume_first: 4096,
            volume_sectors: 200000,
            journal_first: 204096,
            journal_sectors: 10000,
            writable: true,
            flush: true,
        };
        assert_eq!(DiskInfo::decode(&info.encode()), Ok(info));
        for length in 0..48 {
            assert!(DiskInfo::decode(&info.encode()[..length]).is_err());
        }
        for bad in [
            DiskInfo {
                journal_first: 4096,
                ..info
            },
            DiskInfo {
                volume_sectors: u64::MAX,
                ..info
            },
            DiskInfo {
                volume_sectors: 0,
                ..info
            },
            DiskInfo {
                sectors: u64::MAX,
                ..info
            },
        ] {
            assert!(DiskInfo::decode(&bad.encode()).is_err());
        }
        for index in 40..48 {
            let mut bytes = info.encode();
            bytes[index] = 2;
            assert!(DiskInfo::decode(&bytes).is_err());
        }
    }
}
