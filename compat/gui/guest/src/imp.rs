use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

extern crate alloc;
use alloc::vec::Vec;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn RtlGetLastWin32Error() -> u32;
    fn RtlSetLastWin32Error(value: u32);
    fn NtTerminateProcess(process: usize, code: u32) -> u32;
    #[allow(clippy::too_many_arguments)]
    fn NtWriteFile(
        handle: usize,
        event: usize,
        routine: usize,
        context: usize,
        status: usize,
        buffer: usize,
        bytes: u32,
        offset: usize,
        key: usize,
    ) -> u32;
    fn DoorUserCopy(destination: u64, source: u64, bytes: usize) -> u32;
    fn DoorGetCurrentThreadId() -> u32;
    fn DoorGetTickCount64() -> u64;
    fn DoorCreateEventW(attributes: u64, manual: i32, initial: i32, name: u64) -> u64;
    fn DoorSetEvent(handle: u64) -> i32;
    fn DoorCloseHandle(handle: u64) -> i32;
    fn DoorWaitForMultipleObjectsEx(
        count: u32,
        handles: u64,
        all: i32,
        millis: u32,
        alert: i32,
    ) -> u32;
    fn DoorSleep(millis: u32);
    fn DoorGetInheritedHandle(index: u32) -> u64;
}

/// Largest single checked copy (the WINDOWS_GUI manifest limit).
pub const COPY_CHUNK: usize = 64 * 1024;
pub const WAIT_TIMEOUT: u32 = 258;
pub const WAIT_FAILED: u32 = 0xffff_ffff;

pub fn last_error() -> u32 {
    // SAFETY: reads the calling thread's private last-error slot.
    unsafe { RtlGetLastWin32Error() }
}
pub fn set_last_error(value: u32) {
    // SAFETY: writes the calling thread's private last-error slot.
    unsafe { RtlSetLastWin32Error(value) }
}
pub fn thread_id() -> u32 {
    // SAFETY: plain query.
    unsafe { DoorGetCurrentThreadId() }
}
pub fn now() -> u64 {
    // SAFETY: plain query.
    unsafe { DoorGetTickCount64() }
}
pub fn sleep(millis: u32) {
    // SAFETY: plain wait.
    unsafe { DoorSleep(millis) }
}
/// An inherited native handle the trusted startup recorded (REGISTRY,
/// APP_ASSETS); 0 when the process was not granted it.
pub fn inherited(index: usize) -> u64 {
    // SAFETY: plain query.
    unsafe { DoorGetInheritedHandle(index as u32) }
}
/// Auto-reset event used to wake one thread's message wait.
pub fn create_event() -> u64 {
    // SAFETY: no attributes or name pointers.
    unsafe { DoorCreateEventW(0, 0, 0, 0) }
}
pub fn set_event(handle: u64) {
    if handle != 0 {
        // SAFETY: a handle this DLL created.
        unsafe {
            DoorSetEvent(handle);
        }
    }
}
/// Close a handle this DLL created (a retired thread's wake event).
pub fn close_handle(handle: u64) {
    if handle != 0 {
        // SAFETY: a handle this DLL created and no longer uses.
        unsafe {
            DoorCloseHandle(handle);
        }
    }
}
/// Wait for any of `handles` (DLL-owned array) up to `millis`.
pub fn wait_any(handles: &[u64], millis: u32) -> u32 {
    if handles.is_empty() {
        sleep(millis);
        return WAIT_TIMEOUT;
    }
    // ntdll reads a HANDLE array as programs pass it, a pointer-sized
    // entry each (P17): 4 bytes in the x86 DLLs.
    let mut native = [0usize; 64];
    let count = handles.len().min(native.len());
    for (slot, &handle) in native.iter_mut().zip(&handles[..count]) {
        *slot = handle as usize;
    }
    // SAFETY: the array is ours; ntdll captures it through checked copy.
    unsafe { DoorWaitForMultipleObjectsEx(count as u32, native.as_ptr() as u64, 0, millis, 0) }
}

/// Copy `out.len()` bytes from application address `source`.
pub fn read(source: u64, out: &mut [u8]) -> bool {
    let mut done = 0;
    while done < out.len() {
        let n = (out.len() - done).min(COPY_CHUNK);
        let Some(at) = source.checked_add(done as u64) else {
            return false;
        };
        // SAFETY: the destination is our live slice; the kernel validates
        // the complete application range before copying.
        if unsafe { DoorUserCopy(out[done..].as_mut_ptr() as u64, at, n) } != 0 {
            return false;
        }
        done += n;
    }
    true
}
/// Copy `data` to application address `destination`.
pub fn write(destination: u64, data: &[u8]) -> bool {
    let mut done = 0;
    while done < data.len() {
        let n = (data.len() - done).min(COPY_CHUNK);
        let Some(at) = destination.checked_add(done as u64) else {
            return false;
        };
        // SAFETY: as in `read`, with our slice as the source.
        if unsafe { DoorUserCopy(at, data[done..].as_ptr() as u64, n) } != 0 {
            return false;
        }
        done += n;
    }
    true
}
pub fn read_u32(source: u64) -> Option<u32> {
    let mut b = [0u8; 4];
    read(source, &mut b).then(|| u32::from_le_bytes(b))
}
/// The width of the caller's pointers (8 in the x64 build, 4 in the x86
/// one, P17).
pub const POINTER: usize = core::mem::size_of::<usize>();
/// A pointer, handle or pointer-sized integer the application stored at
/// `source`, widened as a pointer argument is (sign-extended on x86).
pub fn read_pointer(source: u64) -> Option<u64> {
    let mut b = [0u8; 8];
    read(source, &mut b[..POINTER]).then(|| {
        if POINTER == 4 {
            i64::from(i32::from_le_bytes([b[0], b[1], b[2], b[3]])) as u64
        } else {
            u64::from_le_bytes(b)
        }
    })
}
/// Store a pointer-sized value at `destination` (its low bytes on x86).
pub fn write_pointer(destination: u64, value: u64) -> bool {
    write(destination, &value.to_le_bytes()[..POINTER])
}
pub fn read_u64(source: u64) -> Option<u64> {
    let mut b = [0u8; 8];
    read(source, &mut b).then(|| u64::from_le_bytes(b))
}
pub fn read_i32(source: u64) -> Option<i32> {
    read_u32(source).map(|v| v as i32)
}
pub fn write_u32(destination: u64, value: u32) -> bool {
    write(destination, &value.to_le_bytes())
}
pub fn write_u64(destination: u64, value: u64) -> bool {
    write(destination, &value.to_le_bytes())
}
pub fn read_bytes(source: u64, n: usize) -> Option<Vec<u8>> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).ok()?;
    v.resize(n, 0);
    read(source, &mut v).then_some(v)
}
/// A NUL-terminated UTF-16 string of at most `max` units (without the NUL).
pub fn read_wide(source: u64, max: usize) -> Option<Vec<u16>> {
    let mut out = Vec::new();
    let mut at = source;
    let mut chunk = [0u8; 128];
    // Units are assembled byte-wise: an unaligned string carries one byte
    // across each chunk boundary.
    let mut decoder = crate::WideDecoder::default();
    loop {
        // Never read past the page holding the terminator: chunks end on a
        // 128-byte boundary so a short string near an unmapped page works.
        let take = 128 - (at % 128) as usize;
        if !read(at, &mut chunk[..take]) {
            return None;
        }
        let mut done = None;
        decoder.feed(&chunk[..take], |u| {
            if u == 0 {
                done = Some(true);
            } else if out.len() >= max {
                done = Some(false);
            } else {
                out.push(u);
            }
            done.is_none()
        });
        match done {
            Some(true) => return Some(out),
            Some(false) => return None,
            None => (),
        }
        at = at.checked_add(take as u64)?;
    }
}
/// Exactly `n` UTF-16 units.
pub fn read_wide_n(source: u64, n: usize) -> Option<Vec<u16>> {
    let bytes = read_bytes(source, n.checked_mul(2)?)?;
    Some(
        bytes
            .chunks_exact(2)
            .map(|p| u16::from_le_bytes([p[0], p[1]]))
            .collect(),
    )
}
/// Store `text` plus a terminator into a buffer of `capacity` units,
/// truncating; returns the units stored (without the NUL). Capacity 0 stores
/// nothing.
pub fn write_wide(destination: u64, capacity: usize, text: &[u16]) -> Option<usize> {
    if capacity == 0 {
        return Some(0);
    }
    let n = text.len().min(capacity - 1);
    let mut bytes = Vec::new();
    bytes.try_reserve_exact((n + 1) * 2).ok()?;
    for u in &text[..n] {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    bytes.extend_from_slice(&[0, 0]);
    write(destination, &bytes).then_some(n)
}
/// Encode a value array of `i32`s (RECT, POINT, ...).
pub fn write_i32s(destination: u64, values: &[i32]) -> bool {
    let mut b = [0u8; 64];
    let n = values.len().min(16);
    for (i, v) in values[..n].iter().enumerate() {
        b[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    write(destination, &b[..n * 4])
}
pub fn read_i32s<const N: usize>(source: u64) -> Option<[i32; N]> {
    let mut b = [0u8; 64];
    if N > 16 || !read(source, &mut b[..N * 4]) {
        return None;
    }
    Some(core::array::from_fn(|i| {
        i32::from_le_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]])
    }))
}

/// A short spin lock. Holders never call application code or wait for
/// other threads while holding it.
pub struct Lock<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}
// SAFETY: the atomic flag serializes every access to the value.
unsafe impl<T: Send> Sync for Lock<T> {}
impl<T> Lock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }
    pub fn lock(&self) -> Guard<'_, T> {
        let mut spins = 0u32;
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            spins += 1;
            if spins > 256 {
                sleep(0);
            } else {
                core::hint::spin_loop();
            }
        }
        Guard { lock: self }
    }
    pub fn try_lock(&self) -> Option<Guard<'_, T>> {
        self.locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| Guard { lock: self })
    }
}
pub struct Guard<'a, T> {
    lock: &'a Lock<T>,
}
impl<T> Deref for Guard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: the guard owns the lock.
        unsafe { &*self.lock.value.get() }
    }
}
impl<T> DerefMut for Guard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: the guard owns the lock exclusively.
        unsafe { &mut *self.lock.value.get() }
    }
}
impl<T> Drop for Guard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
    }
}

/// "<file>:<line>: GUI DLL panic: <message>" on the standard error of a
/// console process (ignored without one), so the cause of a fast fail is
/// kept (the message names an allocation's size or an index).
fn report(info: &core::panic::PanicInfo<'_>) {
    struct Text {
        bytes: [u8; 240],
        n: usize,
    }
    impl core::fmt::Write for Text {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            for b in s.bytes() {
                if self.n < self.bytes.len() - 2 {
                    self.bytes[self.n] = b;
                    self.n += 1;
                }
            }
            Ok(())
        }
    }
    use core::fmt::Write;
    let mut text = Text {
        bytes: [0; 240],
        n: 0,
    };
    if let Some(location) = info.location() {
        let _ = write!(text, "{}:{}: ", location.file(), location.line());
    }
    let _ = write!(text, "GUI DLL panic: {}", info.message());
    let mut n = text.n;
    text.bytes[n] = b'\r';
    text.bytes[n + 1] = b'\n';
    n += 2;
    let text = text.bytes;
    let mut status = [0u64; 2];
    // SAFETY: both buffers are this frame's; a missing console fails the call.
    unsafe {
        NtWriteFile(
            ERROR_HANDLE as usize,
            0,
            0,
            0,
            status.as_mut_ptr() as usize,
            text.as_ptr() as usize,
            n as u32,
            0,
            0,
        );
    }
}
/// The console's standard error handle (door_windows_abi::ERROR_HANDLE).
const ERROR_HANDLE: u64 = 0x108;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    report(info);
    // STATUS_STACK_BUFFER_OVERRUN-style fast fail: no unwinding, no callbacks.
    unsafe {
        NtTerminateProcess(usize::MAX, 0xc000_0409);
    }
    loop {
        core::hint::spin_loop();
    }
}

/// Linkage marker required by Windows compiler-builtins, not an FP operation.
#[unsafe(no_mangle)]
static _fltused: i32 = 0;

/// x86 (P17): `fmod`, the routine the MSVC x86 target leaves the f64 `%`
/// operator to (a C-runtime function this DLL has none of). Exact: with
/// |x| = mx * 2^ex and |y| = my * 2^ey (integer significands), |x| >= |y|
/// implies ex >= ey, and the remainder is ((mx * 2^(ex - ey)) mod my) * 2^ey,
/// a multiple of 2^ey below |y| and so representable; it keeps x's sign.
#[cfg(target_arch = "x86")]
#[unsafe(no_mangle)]
extern "C" fn fmod(x: f64, y: f64) -> f64 {
    if x.is_nan() || y.is_nan() || x.is_infinite() || y == 0.0 {
        return f64::NAN;
    }
    if y.is_infinite() || x.abs() < y.abs() {
        return x;
    }
    fn split(v: f64) -> (u64, i32) {
        let bits = v.to_bits() & !(1u64 << 63);
        let exponent = (bits >> 52) as i32;
        let fraction = bits & ((1u64 << 52) - 1);
        if exponent == 0 {
            (fraction, -1074)
        } else {
            (fraction | (1u64 << 52), exponent - 1075)
        }
    }
    let (mx, ex) = split(x);
    let (my, ey) = split(y);
    let mut r = mx % my;
    for _ in 0..(ex - ey) {
        r = (r << 1) % my;
    }
    let scale = if ey >= -1022 {
        f64::from_bits(((ey + 1023) as u64) << 52)
    } else {
        f64::from_bits(1u64 << (ey + 1074))
    };
    let magnitude = r as f64 * scale;
    if x.is_sign_negative() {
        -magnitude
    } else {
        magnitude
    }
}

/// x86 (P17): the precompiled `core`'s landing pads name MSVC's C++ frame
/// handler. No foreign or Rust unwind is ever dispatched here (panics abort),
/// so reaching it ends the process.
#[cfg(target_arch = "x86")]
#[unsafe(no_mangle)]
extern "C" fn __CxxFrameHandler3() -> ! {
    unsafe {
        NtTerminateProcess(usize::MAX, 0xc000_0409);
    }
    loop {
        core::hint::spin_loop();
    }
}

// Freestanding memory primitives (string instructions; the bitmaps these
// DLLs move make the byte loops of a volatile fallback too slow).
#[unsafe(no_mangle)]
unsafe extern "C" fn memcpy(destination: *mut u8, source: *const u8, bytes: usize) -> *mut u8 {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!("rep movsb", inout("rcx") bytes => _, inout("rdi") destination => _,
            inout("rsi") source => _, options(nostack, preserves_flags));
    }
    // x86 (P17): LLVM reserves ESI, so the source is swapped in and out.
    #[cfg(target_arch = "x86")]
    unsafe {
        core::arch::asm!("xchg {s}, esi", "rep movsb", "xchg {s}, esi", s = inout(reg) source => _,
            inout("ecx") bytes => _, inout("edi") destination => _, options(nostack, preserves_flags));
    }
    destination
}
#[unsafe(no_mangle)]
unsafe extern "C" fn memset(destination: *mut u8, value: i32, bytes: usize) -> *mut u8 {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!("rep stosb", inout("rcx") bytes => _, inout("rdi") destination => _,
            in("al") value as u8, options(nostack, preserves_flags));
    }
    #[cfg(target_arch = "x86")]
    unsafe {
        core::arch::asm!("rep stosb", inout("ecx") bytes => _, inout("edi") destination => _,
            in("al") value as u8, options(nostack, preserves_flags));
    }
    destination
}
#[unsafe(no_mangle)]
unsafe extern "C" fn memmove(destination: *mut u8, source: *const u8, bytes: usize) -> *mut u8 {
    if (destination as usize).wrapping_sub(source as usize) >= bytes {
        unsafe { memcpy(destination, source, bytes) }
    } else {
        // Overlap with the destination above the source: copy backwards.
        #[cfg(target_arch = "x86_64")]
        unsafe {
            core::arch::asm!("std", "rep movsb", "cld",
                inout("rcx") bytes => _, inout("rdi") destination.add(bytes - 1) => _,
                inout("rsi") source.add(bytes - 1) => _, options(nostack));
        }
        #[cfg(target_arch = "x86")]
        unsafe {
            core::arch::asm!("xchg {s}, esi", "std", "rep movsb", "cld", "xchg {s}, esi",
                s = inout(reg) source.add(bytes - 1) => _, inout("ecx") bytes => _,
                inout("edi") destination.add(bytes - 1) => _, options(nostack));
        }
        destination
    }
}
#[unsafe(no_mangle)]
unsafe extern "C" fn memcmp(left: *const u8, right: *const u8, bytes: usize) -> i32 {
    for index in 0..bytes {
        let (a, b) = unsafe { (*left.add(index), *right.add(index)) };
        if a != b {
            return i32::from(a) - i32::from(b);
        }
    }
    0
}
#[unsafe(no_mangle)]
unsafe extern "C" fn bcmp(left: *const u8, right: *const u8, bytes: usize) -> i32 {
    unsafe { memcmp(left, right, bytes) }
}
#[unsafe(no_mangle)]
unsafe extern "C" fn strlen(text: *const u8) -> usize {
    let mut n = 0;
    while unsafe { *text.add(n) } != 0 {
        n += 1;
    }
    n
}

/// `fmodf` for the float remainder operator in the text stack (the musl
/// algorithm on the bit representation; exact, no CRT).
#[unsafe(no_mangle)]
pub extern "C" fn fmodf(x: f32, y: f32) -> f32 {
    let (mut ux, mut uy) = (x.to_bits(), y.to_bits());
    let (mut ex, mut ey) = ((ux >> 23 & 0xff) as i32, (uy >> 23 & 0xff) as i32);
    let sign = ux & 0x8000_0000;
    if uy << 1 == 0 || y.is_nan() || ex == 0xff {
        // NaN for a zero divisor, a NaN operand or an infinite dividend.
        return f32::NAN;
    }
    if ux << 1 <= uy << 1 {
        return if ux << 1 == uy << 1 { 0.0 * x } else { x };
    }
    if ex == 0 {
        let mut i = ux << 9;
        while i >> 31 == 0 {
            ex -= 1;
            i <<= 1;
        }
        ux <<= (1 - ex) as u32;
    } else {
        ux &= u32::MAX >> 9;
        ux |= 1 << 23;
    }
    if ey == 0 {
        let mut i = uy << 9;
        while i >> 31 == 0 {
            ey -= 1;
            i <<= 1;
        }
        uy <<= (1 - ey) as u32;
    } else {
        uy &= u32::MAX >> 9;
        uy |= 1 << 23;
    }
    while ex > ey {
        let i = ux.wrapping_sub(uy);
        if i >> 31 == 0 {
            if i == 0 {
                return 0.0 * x;
            }
            ux = i;
        }
        ux <<= 1;
        ex -= 1;
    }
    let i = ux.wrapping_sub(uy);
    if i >> 31 == 0 {
        if i == 0 {
            return 0.0 * x;
        }
        ux = i;
    }
    while ux >> 23 == 0 {
        ux <<= 1;
        ex -= 1;
    }
    if ex > 0 {
        ux -= 1 << 23;
        ux |= (ex as u32) << 23;
    } else {
        ux >>= (1 - ex) as u32;
    }
    f32::from_bits(ux | sign)
}
