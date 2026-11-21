//! `operation::ENTROPY_STATUS` word and `operation::ENTROPY_READ` limits.
//!
//! The status word is reported to every caller; it describes the kernel pool,
//! never its contents. Bits not listed here are zero.

/// The pool was seeded from a source that passed its health tests and every
/// later health test passed; `ENTROPY_READ` succeeds only while this is set.
pub const READY: u64 = 1 << 0;
/// RDSEED (CPUID.7.0:EBX[18]) contributes seed material.
pub const RDSEED: u64 = 1 << 1;
/// RDRAND (CPUID.1:ECX[30]) contributes seed material.
pub const RDRAND: u64 = 1 << 2;
/// TSC-jitter sampling contributes seed material (the fallback when neither
/// instruction is available, as on the qemu64-v1 QEMU profiles).
pub const JITTER: u64 = 1 << 3;
/// Bits 8..15: saturating count of failed health tests since boot.
pub const FAILURES_SHIFT: u32 = 8;
pub const FAILURES_MASK: u64 = 0xff << FAILURES_SHIFT;
/// Bits 16..31: reseed generation (low 16 bits; 1 after the first seed).
pub const GENERATION_SHIFT: u32 = 16;
pub const GENERATION_MASK: u64 = 0xffff << GENERATION_SHIFT;
/// Every defined bit; anything else set is a malformed word.
pub const DEFINED: u64 = READY | RDSEED | RDRAND | JITTER | FAILURES_MASK | GENERATION_MASK;
/// Largest `ENTROPY_READ` request in bytes.
pub const READ_MAX: usize = 64;

/// Encodes the status word.
pub const fn encode(ready: bool, sources: u64, failures: u32, generation: u32) -> u64 {
    let failures = if failures > 0xff { 0xff } else { failures };
    (if ready { READY } else { 0 })
        | (sources & (RDSEED | RDRAND | JITTER))
        | ((failures as u64) << FAILURES_SHIFT)
        | (((generation & 0xffff) as u64) << GENERATION_SHIFT)
}

/// True when the word uses only defined bits and READY implies a source and
/// a nonzero generation.
pub const fn well_formed(word: u64) -> bool {
    word & !DEFINED == 0
        && (word & READY == 0
            || (word & (RDSEED | RDRAND | JITTER) != 0 && word & GENERATION_MASK != 0))
}

pub const fn failures(word: u64) -> u32 {
    ((word & FAILURES_MASK) >> FAILURES_SHIFT) as u32
}

pub const fn generation(word: u64) -> u32 {
    ((word & GENERATION_MASK) >> GENERATION_SHIFT) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_words_round_trip_and_reject_undefined_bits() {
        let word = encode(true, JITTER | (1 << 40), 300, 0x1_0002);
        assert_eq!(word & (1 << 40), 0);
        assert!(well_formed(word));
        assert_eq!((failures(word), generation(word)), (0xff, 2));
        assert_eq!(word & (READY | JITTER), READY | JITTER);
        assert!(well_formed(0));
        assert!(well_formed(encode(false, 0, 3, 0)));
        assert!(!well_formed(READY));
        assert!(!well_formed(READY | RDRAND));
        assert!(!well_formed(word | 1 << 4));
        assert!(!well_formed(word | 1 << 32));
    }
}
