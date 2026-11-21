//! Kernel entropy: source health tests, SHA-256 conditioning and a hash DRBG.
//!
//! Everything here is allocation-free and host-testable; the binary's
//! `entropy_source` module supplies raw RDSEED/RDRAND words and TSC-jitter
//! samples and owns the single global pool. Raw samples never leave the
//! kernel: they are health tested, conditioned into a 256-bit seed and only
//! DRBG output (rekeyed after every request) reaches callers.
//!
//! Health tests follow NIST SP 800-90B section 4.4 with alpha = 2^-20:
//! the repetition count test (RCT) and the adaptive proportion test (APT,
//! window 512) on 8-bit jitter samples assessed at H = 1 bit per sample
//! (RCT cutoff 1 + 20/H = 21, APT cutoff 410), and a continuous test that
//! rejects stuck, all-zero, all-one or repeated 64-bit instruction words.
use door_abi::entropy;
use sha2::{Digest, Sha256};

/// Assessed min-entropy per 8-bit jitter sample, in bits (conservative).
pub const JITTER_BITS_PER_SAMPLE: usize = 1;
/// Jitter samples conditioned into one seed: twice the 256 assessed bits.
pub const JITTER_SAMPLES: usize = 512;
/// SP 800-90B RCT cutoff for H = 1, alpha = 2^-20.
pub const RCT_CUTOFF: u32 = 21;
/// SP 800-90B APT window and cutoff for non-binary samples, H = 1.
pub const APT_WINDOW: u32 = 512;
pub const APT_CUTOFF: u32 = 410;
/// Instruction words conditioned into one seed (64 bits each, 4x margin).
pub const WORDS_PER_SEED: usize = 16;
/// Retries per RDRAND/RDSEED word (Intel guidance: 10 for RDRAND).
pub const INSTRUCTION_RETRIES: u32 = 10;
/// DRBG requests between reseeds from the selected source.
pub const RESEED_INTERVAL: u32 = 64;
/// Requests a seeded DRBG may still serve while reseeding keeps failing
/// (SP 800-90A allows far more for Hash_DRBG; Doors keeps a short leash).
pub const MAX_UNRESEEDED: u32 = 64 * RESEED_INTERVAL;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Health {
    Ok,
    Failed,
}

/// SP 800-90B 4.4.1 repetition count test.
#[derive(Clone, Copy, Debug)]
pub struct RepetitionCount {
    last: Option<u8>,
    run: u32,
}
impl RepetitionCount {
    pub const fn new() -> Self {
        Self { last: None, run: 0 }
    }
    pub fn feed(&mut self, sample: u8) -> Health {
        if self.last == Some(sample) {
            self.run += 1;
            if self.run >= RCT_CUTOFF {
                return Health::Failed;
            }
        } else {
            self.last = Some(sample);
            self.run = 1;
        }
        Health::Ok
    }
}
impl Default for RepetitionCount {
    fn default() -> Self {
        Self::new()
    }
}

/// SP 800-90B 4.4.2 adaptive proportion test.
#[derive(Clone, Copy, Debug)]
pub struct AdaptiveProportion {
    first: u8,
    seen: u32,
    matches: u32,
}
impl AdaptiveProportion {
    pub const fn new() -> Self {
        Self {
            first: 0,
            seen: 0,
            matches: 0,
        }
    }
    pub fn feed(&mut self, sample: u8) -> Health {
        if self.seen == 0 {
            self.first = sample;
            self.matches = 1;
        } else if sample == self.first {
            self.matches += 1;
            if self.matches >= APT_CUTOFF {
                return Health::Failed;
            }
        }
        self.seen += 1;
        if self.seen == APT_WINDOW {
            self.seen = 0;
        }
        Health::Ok
    }
}
impl Default for AdaptiveProportion {
    fn default() -> Self {
        Self::new()
    }
}

/// Continuous test for 64-bit instruction words: no stuck values and no
/// identical consecutive outputs.
#[derive(Clone, Copy, Debug, Default)]
pub struct WordTest {
    last: Option<u64>,
}
impl WordTest {
    pub const fn new() -> Self {
        Self { last: None }
    }
    pub fn feed(&mut self, word: u64) -> Health {
        let repeated = self.last == Some(word);
        self.last = Some(word);
        if word == 0 || word == u64::MAX || repeated {
            Health::Failed
        } else {
            Health::Ok
        }
    }
}

/// Health-tests a complete batch of jitter samples (startup or reseed).
/// Fewer than `JITTER_SAMPLES` samples, or fewer than 16 distinct values,
/// also fail: a source that cannot vary is never credited.
pub fn jitter_batch(samples: &[u8]) -> Health {
    if samples.len() < JITTER_SAMPLES {
        return Health::Failed;
    }
    let mut rct = RepetitionCount::new();
    let mut apt = AdaptiveProportion::new();
    let mut seen = [false; 256];
    for &sample in samples {
        if rct.feed(sample) == Health::Failed || apt.feed(sample) == Health::Failed {
            return Health::Failed;
        }
        seen[sample as usize] = true;
    }
    if seen.iter().filter(|s| **s).count() < 16 {
        return Health::Failed;
    }
    Health::Ok
}

/// SHA-256 conditioning (SP 800-90B 3.1.5.1.1 vetted function) of one batch
/// of raw material, domain-separated by `label`.
pub struct Conditioner(Sha256);
impl Conditioner {
    pub fn new(label: &[u8]) -> Self {
        let mut hash = Sha256::new();
        hash.update(b"door-entropy-v1 condition ");
        hash.update(label);
        Self(hash)
    }
    pub fn word(&mut self, word: u64) {
        self.0.update(word.to_le_bytes());
    }
    pub fn bytes(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }
    pub fn finish(self) -> [u8; 32] {
        self.0.finalize().into()
    }
}

/// Hash DRBG over SHA-256. Output block i is SHA-256(key || counter || i);
/// after every request the key is replaced by a one-way function of the
/// old key, so a later compromise cannot recover earlier output.
pub struct Drbg {
    key: [u8; 32],
    counter: u64,
    generation: u32,
    since_reseed: u32,
}
impl Drbg {
    pub fn since_reseed(&self) -> u32 {
        self.since_reseed
    }
    pub fn instantiate(seed: &[u8; 32]) -> Self {
        let mut hash = Sha256::new();
        hash.update(b"door-entropy-v1 instantiate");
        hash.update(seed);
        Self {
            key: hash.finalize().into(),
            counter: 0,
            generation: 1,
            since_reseed: 0,
        }
    }
    pub fn reseed(&mut self, seed: &[u8; 32]) {
        let mut hash = Sha256::new();
        hash.update(b"door-entropy-v1 reseed");
        hash.update(self.key);
        hash.update(seed);
        self.key = hash.finalize().into();
        self.generation = self.generation.wrapping_add(1).max(1);
        self.since_reseed = 0;
    }
    pub fn generation(&self) -> u32 {
        self.generation
    }
    pub fn reseed_due(&self) -> bool {
        self.since_reseed >= RESEED_INTERVAL
    }
    pub fn generate(&mut self, out: &mut [u8]) {
        for (index, chunk) in out.chunks_mut(32).enumerate() {
            let mut hash = Sha256::new();
            hash.update(b"door-entropy-v1 output");
            hash.update(self.key);
            hash.update(self.counter.to_le_bytes());
            hash.update((index as u64).to_le_bytes());
            let block: [u8; 32] = hash.finalize().into();
            chunk.copy_from_slice(&block[..chunk.len()]);
        }
        let mut hash = Sha256::new();
        hash.update(b"door-entropy-v1 rekey");
        hash.update(self.key);
        hash.update(self.counter.to_le_bytes());
        self.key = hash.finalize().into();
        self.counter = self.counter.wrapping_add(1);
        self.since_reseed = self.since_reseed.saturating_add(1);
    }
}

/// The global pool's observable state (the binary holds one behind a lock).
pub struct Pool {
    drbg: Option<Drbg>,
    /// Ordinary session apps' DRBG, reseeded from `drbg` output: app reads
    /// never run the raw sources themselves (one raw reseed per
    /// RESEED_INTERVAL^2 app requests at most).
    apps: Option<Drbg>,
    sources: u64,
    failures: u32,
    healthy: bool,
}
impl Pool {
    pub const fn new() -> Self {
        Self {
            drbg: None,
            apps: None,
            sources: 0,
            failures: 0,
            healthy: false,
        }
    }
    pub fn status(&self) -> u64 {
        let ready = self.healthy && self.drbg.is_some();
        entropy::encode(
            ready,
            self.sources,
            self.failures,
            self.drbg.as_ref().map_or(0, Drbg::generation),
        )
    }
    pub fn ready(&self) -> bool {
        self.status() & entropy::READY != 0
    }
    /// Records a failed health test. Before seeding READY stays clear; a
    /// seeded DRBG keeps serving (its state is still secret and forward-
    /// secure) until MAX_UNRESEEDED requests without a successful reseed,
    /// so one bad batch cannot deny Init and the session service.
    pub fn failed(&mut self) {
        self.failures = self.failures.saturating_add(1);
        if self
            .drbg
            .as_ref()
            .is_none_or(|d| d.since_reseed() >= MAX_UNRESEEDED)
        {
            self.healthy = false;
        }
    }
    /// Mixes one health-tested, conditioned seed from `source`.
    pub fn seed(&mut self, source: u64, seed: &[u8; 32]) {
        match self.drbg.as_mut() {
            Some(drbg) => drbg.reseed(seed),
            None => self.drbg = Some(Drbg::instantiate(seed)),
        }
        self.sources |= source & (entropy::RDSEED | entropy::RDRAND | entropy::JITTER);
        self.healthy = true;
    }
    pub fn reseed_due(&self) -> bool {
        self.drbg.as_ref().is_some_and(Drbg::reseed_due)
    }
    /// DRBG output, or None while not READY (nothing is written).
    pub fn read(&mut self, out: &mut [u8]) -> Option<usize> {
        if !self.ready()
            || self
                .drbg
                .as_ref()
                .is_some_and(|d| d.since_reseed() >= MAX_UNRESEEDED)
        {
            return None;
        }
        self.drbg.as_mut()?.generate(out);
        Some(out.len())
    }
    /// Whether the apps' DRBG needs a seed from the main one.
    pub fn app_reseed_due(&self) -> bool {
        self.apps.as_ref().is_none_or(Drbg::reseed_due)
    }
    /// Output for an ordinary session app, from the apps' DRBG.
    pub fn read_app(&mut self, out: &mut [u8]) -> Option<usize> {
        if self.app_reseed_due() {
            let mut seed = [0u8; 32];
            self.read(&mut seed)?;
            match self.apps.as_mut() {
                Some(apps) => apps.reseed(&seed),
                None => self.apps = Some(Drbg::instantiate(&seed)),
            }
            seed.fill(0);
        }
        self.apps.as_mut()?.generate(out);
        Some(out.len())
    }
}
impl Default for Pool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_failed_reseed_keeps_a_seeded_pool_serving_for_a_bounded_time() {
        let mut p = Pool::new();
        p.failed();
        assert!(!p.ready());
        p.seed(entropy::JITTER, &[7; 32]);
        p.failed();
        assert!(p.ready());
        let mut out = [0u8; 16];
        for _ in 0..MAX_UNRESEEDED {
            assert!(p.read(&mut out).is_some());
        }
        assert!(p.read(&mut out).is_none());
        p.failed();
        assert!(!p.ready());
        p.seed(entropy::JITTER, &[9; 32]);
        assert!(p.read(&mut out).is_some());
    }
    #[test]
    fn app_reads_draw_on_the_main_drbg_once_per_interval() {
        let mut p = Pool::new();
        p.seed(entropy::JITTER, &[3; 32]);
        let mut out = [0u8; 64];
        for _ in 0..RESEED_INTERVAL {
            assert!(p.read_app(&mut out).is_some());
        }
        // One main request seeded the apps' DRBG for the whole interval.
        assert_eq!(p.drbg.as_ref().unwrap().since_reseed(), 1);
        assert!(p.read_app(&mut out).is_some());
        assert_eq!(p.drbg.as_ref().unwrap().since_reseed(), 2);
    }

    fn varied(n: usize) -> [u8; 1024] {
        let mut out = [0; 1024];
        let mut x = 0x9e37_79b9_u32;
        for value in out.iter_mut().take(n) {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            *value = x as u8;
        }
        out
    }

    #[test]
    fn repetition_count_trips_exactly_at_the_cutoff() {
        let mut rct = RepetitionCount::new();
        for _ in 0..RCT_CUTOFF - 1 {
            assert_eq!(rct.feed(7), Health::Ok);
        }
        assert_eq!(rct.feed(7), Health::Failed);
        let mut rct = RepetitionCount::new();
        for i in 0..1000u32 {
            assert_eq!(rct.feed((i / 20) as u8), Health::Ok);
        }
    }

    #[test]
    fn adaptive_proportion_trips_inside_a_window_and_resets_between_windows() {
        let mut apt = AdaptiveProportion::new();
        let mut failed = false;
        for i in 0..APT_WINDOW {
            // First sample repeated in 90% of the window.
            let sample = if i % 10 == 9 { (i % 251) as u8 + 1 } else { 0 };
            failed |= apt.feed(sample) == Health::Failed;
        }
        assert!(failed);
        let mut apt = AdaptiveProportion::new();
        for i in 0..4 * APT_WINDOW {
            let sample = if i % 2 == 0 { 3 } else { (i % 200) as u8 + 4 };
            assert_eq!(apt.feed(sample), Health::Ok, "{i}");
        }
    }

    #[test]
    fn word_test_rejects_stuck_extreme_and_repeated_words() {
        let mut test = WordTest::new();
        assert_eq!(test.feed(0x1234), Health::Ok);
        assert_eq!(test.feed(0x1234), Health::Failed);
        assert_eq!(test.feed(0x5678), Health::Ok);
        assert_eq!(WordTest::new().feed(0), Health::Failed);
        assert_eq!(WordTest::new().feed(u64::MAX), Health::Failed);
    }

    #[test]
    fn jitter_batches_need_size_variety_and_passing_tests() {
        assert_eq!(
            jitter_batch(&varied(JITTER_SAMPLES)[..JITTER_SAMPLES]),
            Health::Ok
        );
        assert_eq!(
            jitter_batch(&varied(JITTER_SAMPLES)[..JITTER_SAMPLES - 1]),
            Health::Failed
        );
        assert_eq!(jitter_batch(&[9; JITTER_SAMPLES]), Health::Failed);
        let mut few = [0u8; JITTER_SAMPLES];
        for (i, v) in few.iter_mut().enumerate() {
            *v = (i % 8) as u8;
        }
        assert_eq!(jitter_batch(&few), Health::Failed);
        let mut stuck = varied(JITTER_SAMPLES);
        stuck[100..100 + RCT_CUTOFF as usize].fill(0x42);
        assert_eq!(jitter_batch(&stuck[..JITTER_SAMPLES]), Health::Failed);
    }

    #[test]
    fn drbg_is_deterministic_per_seed_rekeys_and_reseeds() {
        let mut a = Drbg::instantiate(&[1; 32]);
        let mut b = Drbg::instantiate(&[1; 32]);
        let mut c = Drbg::instantiate(&[2; 32]);
        let (mut x, mut y, mut z) = ([0; 64], [0; 64], [0; 64]);
        a.generate(&mut x);
        b.generate(&mut y);
        c.generate(&mut z);
        assert_eq!(x, y);
        assert_ne!(x, z);
        assert_ne!(x[..32], x[32..]);
        let mut next = [0; 64];
        a.generate(&mut next);
        assert_ne!(next, x, "rekeyed after every request");
        b.reseed(&[3; 32]);
        b.generate(&mut y);
        assert_ne!(y, next);
        assert_eq!((a.generation(), b.generation()), (1, 2));
        // One request already made above.
        for _ in 1..RESEED_INTERVAL {
            assert!(!c.reseed_due());
            c.generate(&mut z[..1]);
        }
        assert!(c.reseed_due());
        c.reseed(&[4; 32]);
        assert!(!c.reseed_due());
    }

    #[test]
    fn pool_reports_real_status_and_withholds_output_until_healthy() {
        let mut pool = Pool::new();
        let mut out = [0xaa; 16];
        assert_eq!(pool.status(), 0);
        assert_eq!(pool.read(&mut out), None);
        assert_eq!(out, [0xaa; 16]);
        pool.failed();
        assert_eq!(pool.status(), entropy::encode(false, 0, 1, 0));
        let mut conditioner = Conditioner::new(b"jitter");
        conditioner.bytes(&varied(JITTER_SAMPLES)[..JITTER_SAMPLES]);
        pool.seed(entropy::JITTER, &conditioner.finish());
        let status = pool.status();
        assert!(entropy::well_formed(status));
        assert_eq!(
            status & (entropy::READY | entropy::JITTER),
            entropy::READY | entropy::JITTER
        );
        assert_eq!(
            (entropy::failures(status), entropy::generation(status)),
            (1, 1)
        );
        assert_eq!(pool.read(&mut out), Some(16));
        assert_ne!(out, [0xaa; 16]);
        // One failed reseed batch after seeding: the seeded DRBG keeps
        // serving (bounded by MAX_UNRESEEDED) and the failure is counted.
        pool.failed();
        assert!(pool.ready());
        assert_eq!(entropy::failures(pool.status()), 2);
        assert_eq!(pool.read(&mut out), Some(16));
        pool.seed(entropy::JITTER, &[5; 32]);
        assert!(pool.ready());
        assert_eq!(entropy::generation(pool.status()), 2);
    }
}
