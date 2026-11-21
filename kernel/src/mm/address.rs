use super::{Error, PAGE_BYTES};

/// An x64 physical address. This does not grant access or imply a virtual mapping.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PhysicalAddress(u64);
impl PhysicalAddress {
    pub const fn new(value: u64) -> Result<Self, Error> {
        if value < (1 << 52) {
            Ok(Self(value))
        } else {
            Err(Error::Address)
        }
    }
    pub const fn get(self) -> u64 {
        self.0
    }
    pub fn checked_add(self, bytes: ByteCount) -> Result<Self, Error> {
        Self::new(self.0.checked_add(bytes.get()).ok_or(Error::Overflow)?)
    }
}

/// A canonical address in the selected four-level x64 paging profile.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct VirtualAddress(u64);
impl VirtualAddress {
    pub const fn new(value: u64) -> Result<Self, Error> {
        if value < 0x0000_8000_0000_0000 || value >= 0xffff_8000_0000_0000 {
            Ok(Self(value))
        } else {
            Err(Error::Address)
        }
    }
    pub const fn get(self) -> u64 {
        self.0
    }
    pub fn checked_add(self, bytes: ByteCount) -> Result<Self, Error> {
        let end = self.0.checked_add(bytes.get()).ok_or(Error::Overflow)?;
        let result = Self::new(end)?;
        if self.0 < 0x0000_8000_0000_0000 && end >= 0xffff_8000_0000_0000 {
            return Err(Error::Address);
        }
        Ok(result)
    }
    /// Translate an address between two window origins. Subtraction happens
    /// before addition, so a valid high-half alias never transiently overflows
    /// merely because the source address is in the low half.
    pub fn rebase(self, source_start: Self, target_start: Self) -> Result<Self, Error> {
        let offset = self.0.checked_sub(source_start.0).ok_or(Error::Address)?;
        target_start.checked_add(ByteCount::new(offset))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ByteCount(u64);
impl ByteCount {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    pub const fn get(self) -> u64 {
        self.0
    }
    pub fn round_up(self, alignment: u64) -> Result<Self, Error> {
        if !alignment.is_power_of_two() {
            return Err(Error::Alignment);
        }
        Ok(Self(
            self.0.checked_add(alignment - 1).ok_or(Error::Overflow)? & !(alignment - 1),
        ))
    }
    pub fn pages(self) -> Result<PageCount, Error> {
        if !self.0.is_multiple_of(PAGE_BYTES) {
            return Err(Error::Alignment);
        }
        PageCount::new(self.0 / PAGE_BYTES)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageCount(u64);
impl PageCount {
    pub const fn new(value: u64) -> Result<Self, Error> {
        if value == 0 {
            return Err(Error::Empty);
        }
        if value.checked_mul(PAGE_BYTES).is_none() {
            return Err(Error::Overflow);
        }
        Ok(Self(value))
    }
    pub const fn get(self) -> u64 {
        self.0
    }
    pub const fn bytes(self) -> ByteCount {
        ByteCount(self.0 * PAGE_BYTES)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameNumber(u64);
impl FrameNumber {
    pub fn from_address(address: PhysicalAddress) -> Result<Self, Error> {
        if !address.get().is_multiple_of(PAGE_BYTES) {
            return Err(Error::Alignment);
        }
        Ok(Self(address.get() / PAGE_BYTES))
    }
    pub fn address(self) -> PhysicalAddress {
        PhysicalAddress(self.0 * PAGE_BYTES)
    }
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileOffset(u64);
impl FileOffset {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    pub const fn get(self) -> u64 {
        self.0
    }
    pub fn checked_add(self, bytes: ByteCount) -> Result<Self, Error> {
        Ok(Self(
            self.0.checked_add(bytes.get()).ok_or(Error::Overflow)?,
        ))
    }
}

/// Device bus addresses remain distinct from CPU physical addresses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceAddress(u64);
impl DeviceAddress {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rebase_low_user_windows_to_high_aliases_without_intermediate_overflow() {
        let user = VirtualAddress::new(0x5000_0000).unwrap();
        for slot in 0..8 {
            let alias = VirtualAddress::new(0xffff_ffff_d700_0000 + slot * 0x0400_0000).unwrap();
            for offset in [0, 4096, 0xf_f000, 0xff_f000] {
                let address = user.checked_add(ByteCount::new(offset)).unwrap();
                assert_eq!(
                    address.rebase(user, alias).unwrap().get(),
                    alias.get() + offset
                );
            }
        }
        assert_eq!(
            VirtualAddress::new(0x4fff_f000)
                .unwrap()
                .rebase(user, VirtualAddress::new(0xffff_ffff_f300_0000).unwrap()),
            Err(Error::Address)
        );
        let limit = VirtualAddress::new(0xffff_ffff_ffff_f000).unwrap();
        assert_eq!(
            user.checked_add(ByteCount::new(4096))
                .unwrap()
                .rebase(user, limit),
            Err(Error::Overflow)
        );
    }
    #[test]
    fn arithmetic_and_canonical_boundaries_are_checked() {
        assert_eq!(PageCount::new(0), Err(Error::Empty));
        assert_eq!(PageCount::new(u64::MAX), Err(Error::Overflow));
        assert_eq!(
            ByteCount::new(u64::MAX).round_up(4096),
            Err(Error::Overflow)
        );
        assert_eq!(ByteCount::new(5).round_up(3), Err(Error::Alignment));
        assert_eq!(ByteCount::new(4097).pages(), Err(Error::Alignment));
        assert_eq!(ByteCount::new(4097).round_up(4096).unwrap().get(), 8192);
        assert_eq!(PhysicalAddress::new(1 << 52), Err(Error::Address));
        assert_eq!(VirtualAddress::new(0x8000_0000_0000), Err(Error::Address));
        assert!(VirtualAddress::new(0xffff_8000_0000_0000).is_ok());
        assert_eq!(
            VirtualAddress::new(0x7fff_ffff_f000)
                .unwrap()
                .checked_add(ByteCount::new(4096)),
            Err(Error::Address)
        );
        assert_eq!(
            VirtualAddress::new(0xffff_ffff_ffff_f000)
                .unwrap()
                .checked_add(ByteCount::new(4096)),
            Err(Error::Overflow)
        );
        assert_eq!(
            FrameNumber::from_address(PhysicalAddress::new(1).unwrap()),
            Err(Error::Alignment)
        );
    }
}
