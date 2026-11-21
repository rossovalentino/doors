//! Anonymous pipe handles (CreatePipe): local adapter values, like file
//! handles, naming one end in ntdll's pipe table. They confer no authority:
//! the pipe is the section each end maps.
//! The values are `crate::tagged` PIPE handles (below 2 GB in a 32-bit
//! process); compare a decoded generation with `crate::tagged::same`.
pub const SLOTS: usize = 16;
use crate::tagged::PIPE;
pub fn is_pipe(handle: u64) -> bool {
    PIPE.is(handle)
}
pub fn encode(slot: usize, generation: u32) -> Option<u64> {
    (slot < SLOTS && generation != 0).then_some(PIPE.encode(slot, generation))
}
pub fn decode(handle: u64) -> Option<(usize, u32)> {
    PIPE.decode(handle).filter(|&(index, _)| index < SLOTS)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pipe_handles_never_alias_file_handles_or_stale_slots() {
        for slot in 0..SLOTS {
            let handle = encode(slot, 7).unwrap();
            assert!(is_pipe(handle));
            assert!(!crate::file::is_file(handle));
            assert_eq!(decode(handle), Some((slot, 7)));
            assert_ne!(encode(slot, 1), encode(slot, 2));
        }
        assert_eq!(encode(SLOTS, 1), None);
        assert_eq!(encode(0, 0), None);
        for value in [
            0,
            u64::MAX,
            u64::from(PIPE.wide) << 48,
            crate::file::encode(0, 1).unwrap(),
        ] {
            assert_eq!(decode(value), None);
        }
    }
}
