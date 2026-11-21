//! Public read-only boot inventory. No physical addresses or mutation authority.
use crate::Status;
pub const BYTES: usize = 16;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HardwareInfo {
    pub power_profile: u8,
    pub processors: u16,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Illustration {
    Monitor,
    Laptop,
    Tablet,
}
impl HardwareInfo {
    pub const fn illustration(self) -> Illustration {
        match self.power_profile {
            2 => Illustration::Laptop,
            8 => Illustration::Tablet,
            _ => Illustration::Monitor,
        }
    }
    pub fn encode(self) -> Result<[u8; BYTES], Status> {
        if self.power_profile > 8 || !(1..=64).contains(&self.processors) {
            return Err(Status::InvalidArgument);
        }
        let mut b = [0; BYTES];
        b[0] = 1;
        b[1] = self.power_profile;
        b[2..4].copy_from_slice(&self.processors.to_le_bytes());
        Ok(b)
    }
    pub fn decode(b: &[u8]) -> Result<Self, Status> {
        if b.len() != BYTES || b[0] != 1 || b[4..].iter().any(|v| *v != 0) {
            return Err(Status::InvalidArgument);
        }
        let out = Self {
            power_profile: b[1],
            processors: u16::from_le_bytes([b[2], b[3]]),
        };
        out.encode()?;
        Ok(out)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn firmware_profiles_select_art_without_guessing_unknown_chassis() {
        for profile in 0..=8 {
            let h = HardwareInfo {
                power_profile: profile,
                processors: 1,
            };
            assert_eq!(HardwareInfo::decode(&h.encode().unwrap()), Ok(h));
            assert_eq!(
                h.illustration(),
                match profile {
                    2 => Illustration::Laptop,
                    8 => Illustration::Tablet,
                    _ => Illustration::Monitor,
                }
            );
        }
        assert!(
            HardwareInfo {
                power_profile: 9,
                processors: 1
            }
            .encode()
            .is_err()
        );
        assert!(
            HardwareInfo {
                power_profile: 0,
                processors: 0
            }
            .encode()
            .is_err()
        );
        let mut b = HardwareInfo {
            power_profile: 8,
            processors: 4,
        }
        .encode()
        .unwrap();
        for length in 0..16 {
            assert!(HardwareInfo::decode(&b[..length]).is_err());
        }
        b[15] = 1;
        assert!(HardwareInfo::decode(&b).is_err());
    }
}
