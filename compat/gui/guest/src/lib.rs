#![no_std]
//! PE-only support for Door's original GUI compatibility DLLs. Application
//! pointers are integers here: every read or write of application memory
//! goes through ntdll's `DoorUserCopy`, the kernel's checked-copy boundary
//! (64 KiB per call for the WINDOWS_GUI manifest), before Rust forms a value.
//! Host builds compile none of this except the portable helpers below.
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod imp;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
pub use imp::*;

/// Incremental little-endian UTF-16 decoding of byte chunks of any length:
/// a chunk ending in the middle of a unit carries its last byte into the
/// next one, so strings at odd addresses decode exactly.
#[derive(Default)]
pub struct WideDecoder {
    pending: Option<u8>,
}
impl WideDecoder {
    /// Decode `bytes`, passing each complete unit to `unit` until it returns
    /// false; returns false when `unit` stopped the decoding.
    pub fn feed(&mut self, bytes: &[u8], mut unit: impl FnMut(u16) -> bool) -> bool {
        for &b in bytes {
            match self.pending.take() {
                None => self.pending = Some(b),
                Some(low) => {
                    if !unit(u16::from_le_bytes([low, b])) {
                        return false;
                    }
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::WideDecoder;
    #[test]
    fn odd_chunks_keep_units_whole() {
        // "AB" + NUL at an odd address: the first chunk is 3 bytes.
        let bytes = [0x41, 0x00, 0x42, 0x00, 0x00, 0x00];
        for split in 0..=bytes.len() {
            let mut d = WideDecoder::default();
            let mut out = [0u16; 3];
            let mut n = 0;
            let mut sink = |u: u16| {
                out[n] = u;
                n += 1;
                u != 0
            };
            if d.feed(&bytes[..split], &mut sink) {
                d.feed(&bytes[split..], &mut sink);
            }
            assert_eq!(&out[..n], &[0x41, 0x42, 0], "split at {split}");
        }
    }
}
