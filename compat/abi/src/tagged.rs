//! Handle values Doors' DLLs make themselves (files, pipes, directory and
//! stream searches): a kind tag, a table slot and that slot's generation,
//! so a stale handle never names a reused slot. A 64-bit build keeps the
//! 16-bit tag in bits 48-63 and the whole 32-bit generation. A 32-bit
//! process (P17) passes a HANDLE in 4 bytes, sign-extended on the way in,
//! so its values stay below 2 GB: a 7-bit tag in bits 24-30 (above every
//! kernel handle, which count up from 0x1_0000 by 4), the generation's low
//! 16 bits in bits 8-23, the slot in bits 0-7.

/// A handle kind: its 64-bit and 32-bit tags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tag {
    pub wide: u16,
    /// Below 0x80, so 32-bit values stay positive.
    pub narrow: u8,
}

#[cfg(target_pointer_width = "64")]
impl Tag {
    pub const fn encode(self, slot: usize, generation: u32) -> u64 {
        ((self.wide as u64) << 48) | ((generation as u64) << 8) | (slot as u64 + 1)
    }
    pub const fn is(self, handle: u64) -> bool {
        handle >> 48 == self.wide as u64
    }
    /// The slot and generation; None for another kind or a malformed value.
    pub fn decode(self, handle: u64) -> Option<(usize, u32)> {
        if handle >> 40 != (self.wide as u64) << 8 {
            return None;
        }
        let slot = (handle & 255).checked_sub(1)? as usize;
        let generation = (handle >> 8) as u32;
        (generation != 0).then_some((slot, generation))
    }
}
#[cfg(not(target_pointer_width = "64"))]
impl Tag {
    pub const fn encode(self, slot: usize, generation: u32) -> u64 {
        ((self.narrow as u64) << 24) | (((generation & 0xFFFF) as u64) << 8) | (slot as u64 + 1)
    }
    pub const fn is(self, handle: u64) -> bool {
        handle >> 24 == self.narrow as u64
    }
    /// The slot and the generation's low 16 bits (compare with `same`).
    pub fn decode(self, handle: u64) -> Option<(usize, u32)> {
        if !self.is(handle) {
            return None;
        }
        let slot = (handle & 255).checked_sub(1)? as usize;
        Some((slot, ((handle >> 8) & 0xFFFF) as u32))
    }
}

/// Whether a slot's current `generation` is the one a decoded handle names
/// (a 32-bit handle keeps its low 16 bits).
pub const fn same(generation: u32, decoded: u32) -> bool {
    if cfg!(target_pointer_width = "64") {
        generation == decoded
    } else {
        generation & 0xFFFF == decoded
    }
}

/// The kinds the DLLs make.
pub const FILE: Tag = Tag {
    wide: 0x4649,
    narrow: 0x46,
};
pub const PIPE: Tag = Tag {
    wide: 0x5049,
    narrow: 0x50,
};
pub const FIND: Tag = Tag {
    wide: 0x4644,
    narrow: 0x44,
};
pub const STREAM_SEARCH: Tag = Tag {
    wide: 0x5354,
    narrow: 0x53,
};
/// Named pipe instances (CreateNamedPipe, P18).
pub const NAMED_PIPE: Tag = Tag {
    wide: 0x4E50,
    narrow: 0x4E,
};
/// Directories opened with FILE_FLAG_BACKUP_SEMANTICS (P18).
pub const DIRECTORY: Tag = Tag {
    wide: 0x4452,
    narrow: 0x52,
};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kinds_never_alias_and_round_trip() {
        let kinds = [FILE, PIPE, FIND, STREAM_SEARCH, NAMED_PIPE, DIRECTORY];
        for (i, a) in kinds.iter().enumerate() {
            for slot in [0usize, 7, 15] {
                for generation in [1u32, 2, 0xFFFF, 0x1_0001, u32::MAX] {
                    let h = a.encode(slot, generation);
                    let (s, g) = a.decode(h).unwrap();
                    assert_eq!(s, slot);
                    assert!(same(generation, g));
                    for (j, b) in kinds.iter().enumerate() {
                        assert_eq!(b.is(h), i == j);
                    }
                }
            }
        }
        // Kernel handles (0x1_0000 upwards by 4) and pseudo-handles are none
        // of these.
        for h in [0x1_0000u64, 0x1_0004, 0xFFFF_FFFF_FFFF_FFFF, 0] {
            assert!(kinds.iter().all(|k| k.decode(h).is_none()));
        }
    }
    #[test]
    fn the_32_bit_layout_stays_below_2_gb() {
        // The narrow encoding as a 32-bit build computes it.
        for k in [FILE, PIPE, FIND, STREAM_SEARCH, NAMED_PIPE, DIRECTORY] {
            let h = (u64::from(k.narrow) << 24) | (0xFFFF << 8) | 255;
            assert!(h < 0x8000_0000);
            assert!(h > 0x1_0000 * 0x100);
        }
    }
}
