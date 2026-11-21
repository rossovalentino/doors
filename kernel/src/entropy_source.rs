//! Hardware entropy sources feeding the single kernel pool
//! (`door_kernel::entropy`). RDSEED is preferred, then RDRAND, then
//! TSC-jitter sampling (the qemu64-v1 profiles expose neither instruction).
//! Every batch is health tested before it is conditioned and credited; a
//! failed batch withdraws READY until a later one passes. Raw samples are
//! gathered without the pool lock; only conditioned seeds and DRBG output
//! cross it, with local interrupts disabled for those few hash blocks.
use core::{
    arch::asm,
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, AtomicU8, Ordering},
};
use door_abi::{Status, entropy as bits};
use door_kernel::entropy::{
    self as pool, Conditioner, Health, INSTRUCTION_RETRIES, JITTER_SAMPLES, Pool, WORDS_PER_SEED,
    WordTest,
};

struct Global {
    lock: AtomicBool,
    pool: UnsafeCell<Pool>,
}
// SAFETY: `pool` is only reached through `with`, which holds `lock` with local
// interrupts disabled, so no two contexts ever alias the mutable reference.
unsafe impl Sync for Global {}
static GLOBAL: Global = Global {
    lock: AtomicBool::new(false),
    pool: UnsafeCell::new(Pool::new()),
};
/// bit0 RDSEED, bit1 RDRAND, bit7 detected.
static FEATURES: AtomicU8 = AtomicU8::new(0);

fn with<R>(f: impl FnOnce(&mut Pool) -> R) -> R {
    let flags: u64;
    unsafe {
        asm!("pushfq", "pop {}", "cli", out(reg) flags, options(nomem));
    }
    while GLOBAL
        .lock
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    // SAFETY: exclusive by the lock above (see `Global`).
    let result = f(unsafe { &mut *GLOBAL.pool.get() });
    GLOBAL.lock.store(false, Ordering::Release);
    if flags & (1 << 9) != 0 {
        unsafe {
            asm!("sti", options(nomem, nostack));
        }
    }
    result
}

fn features() -> u8 {
    let known = FEATURES.load(Ordering::Acquire);
    if known & 0x80 != 0 {
        return known;
    }
    let mut detected = 0x80;
    if core::arch::x86_64::__cpuid(0).eax >= 7
        && core::arch::x86_64::__cpuid_count(7, 0).ebx & (1 << 18) != 0
    {
        detected |= 1;
    }
    if core::arch::x86_64::__cpuid(1).ecx & (1 << 30) != 0 {
        detected |= 2;
    }
    FEATURES.store(detected, Ordering::Release);
    detected
}

fn rdseed() -> Option<u64> {
    for _ in 0..INSTRUCTION_RETRIES {
        let (value, ok): (u64, u8);
        unsafe {
            asm!("rdseed {v}", "setc {ok}", v = out(reg) value, ok = out(reg_byte) ok,
                options(nomem, nostack));
        }
        if ok != 0 {
            return Some(value);
        }
        core::hint::spin_loop();
    }
    None
}

fn rdrand() -> Option<u64> {
    for _ in 0..INSTRUCTION_RETRIES {
        let (value, ok): (u64, u8);
        unsafe {
            asm!("rdrand {v}", "setc {ok}", v = out(reg) value, ok = out(reg_byte) ok,
                options(nomem, nostack));
        }
        if ok != 0 {
            return Some(value);
        }
    }
    None
}

fn rdtsc() -> u64 {
    let (low, high): (u32, u32);
    unsafe {
        asm!("lfence", "rdtsc", out("eax") low, out("edx") high, options(nomem, nostack));
    }
    u64::from(high) << 32 | u64::from(low)
}

/// `WORDS_PER_SEED` health-tested instruction words, conditioned.
fn instruction_seed(label: &[u8], draw: fn() -> Option<u64>) -> Option<[u8; 32]> {
    let mut test = WordTest::new();
    let mut conditioner = Conditioner::new(label);
    for _ in 0..WORDS_PER_SEED {
        let word = draw()?;
        if test.feed(word) == Health::Failed {
            return None;
        }
        conditioner.word(word);
    }
    Some(conditioner.finish())
}

/// `JITTER_SAMPLES` TSC deltas around a data-dependent workload. The raw
/// low byte of each delta is health tested; the full deltas are conditioned.
fn jitter_seed() -> Option<[u8; 32]> {
    let mut samples = [0u8; JITTER_SAMPLES];
    let mut scratch = [0u64; 64];
    let mut conditioner = Conditioner::new(b"jitter");
    let mut last = rdtsc();
    let mut mix = last;
    for sample in samples.iter_mut() {
        let rounds = 16 + (mix as usize & 15);
        for round in 0..rounds {
            let slot = (mix as usize ^ round) & 63;
            scratch[slot] = scratch[slot].wrapping_add(mix).rotate_left(7);
            mix = mix.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ scratch[(slot * 7) & 63];
        }
        core::hint::black_box(&scratch);
        let now = rdtsc();
        let delta = now.wrapping_sub(last);
        conditioner.word(delta);
        *sample = delta as u8;
        mix ^= delta;
        last = now;
    }
    if pool::jitter_batch(&samples) == Health::Failed {
        return None;
    }
    conditioner.bytes(&samples);
    Some(conditioner.finish())
}

/// One seed from the best available source. Each failed source counts as a
/// health failure; the next source is tried.
fn gather() -> Option<(u64, [u8; 32])> {
    let features = features();
    if features & 1 != 0 {
        match instruction_seed(b"rdseed", rdseed) {
            Some(seed) => return Some((bits::RDSEED, seed)),
            None => with(Pool::failed),
        }
    }
    if features & 2 != 0 {
        match instruction_seed(b"rdrand", rdrand) {
            Some(seed) => return Some((bits::RDRAND, seed)),
            None => with(Pool::failed),
        }
    }
    match jitter_seed() {
        Some(seed) => Some((bits::JITTER, seed)),
        None => {
            with(Pool::failed);
            None
        }
    }
}

fn reseed() -> bool {
    match gather() {
        Some((source, seed)) => {
            with(|p| p.seed(source, &seed));
            true
        }
        None => false,
    }
}

/// Seeds the pool at boot (three attempts); READY stays clear if all fail.
pub fn initialize() {
    for _ in 0..3 {
        if reseed() {
            return;
        }
    }
}

/// `operation::ENTROPY_STATUS` for any caller.
pub fn status() -> u64 {
    with(|p| p.status())
}

/// `operation::ENTROPY_READ` after the caller's capability check. Reseeds
/// first when due or after a failed batch withdrew READY.
pub fn read(out: &mut [u8]) -> Result<usize, Status> {
    if with(|p| p.reseed_due() || !p.ready()) {
        reseed();
    }
    with(|p| p.read(out)).ok_or(Status::Unsupported)
}

/// ENTROPY_READ for an ordinary session app: served by the apps' DRBG,
/// which draws one seed from the main DRBG per RESEED_INTERVAL requests;
/// only that draw may trigger a raw-source reseed.
pub fn read_app(out: &mut [u8]) -> Result<usize, Status> {
    if with(|p| p.app_reseed_due() && (p.reseed_due() || !p.ready())) {
        reseed();
    }
    with(|p| p.read_app(out)).ok_or(Status::Unsupported)
}
