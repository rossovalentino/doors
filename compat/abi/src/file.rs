//! Local adapter handles distinguish service files from native wait handles.
//! They confer no authority: the storage service checks the authenticated owner.
//! The values are `crate::tagged` FILE handles (below 2 GB in a 32-bit
//! process); compare a decoded generation with `crate::tagged::same`.
pub const SLOTS: usize = 192;
use crate::tagged::FILE;
pub fn is_file(handle: u64) -> bool {
    FILE.is(handle)
}
pub fn encode(slot: usize, generation: u32) -> Option<u64> {
    (slot < SLOTS && generation != 0).then_some(FILE.encode(slot, generation))
}
pub fn decode(handle: u64) -> Option<(usize, u32)> {
    FILE.decode(handle).filter(|&(index, _)| index < SLOTS)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reserved_bits_and_generation_prevent_stale_slot_aliases() {
        for slot in 0..SLOTS {
            let handle = encode(slot, u32::MAX).unwrap();
            assert_eq!(decode(handle), Some((slot, u32::MAX)));
            assert_ne!(encode(slot, 1), encode(slot, 2));
            assert_eq!(decode(handle | (1 << 40)), None);
            assert_eq!(decode(handle ^ (1 << 63)), None);
        }
        assert_eq!(encode(SLOTS, 1), None);
        assert_eq!(encode(0, 0), None);
        let tag = u64::from(FILE.wide) << 48;
        for value in [0, 104, u64::MAX, tag, tag | 1 << 8] {
            assert_eq!(decode(value), None);
        }
    }
}
