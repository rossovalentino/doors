//! The time set (api-ms-win-crt-time) over GetSystemTimeAsFileTime: time,
//! gmtime, localtime (the zone's bias from `_timezone`, UTC by default),
//! mktime, strftime and wcsftime in the "C" locale, clock.
use super::{GetSystemTimeAsFileTime, errno, invalid_parameter, set_errno, thread};
use core::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use door_crt::time::{self, Tm};

/// 100 ns intervals from 1601-01-01 to 1970-01-01.
const EPOCH_DIFFERENCE: u64 = 116_444_736_000_000_000;
fn now_100ns() -> u64 {
    let mut ft = 0u64;
    unsafe { GetSystemTimeAsFileTime(&mut ft as *mut u64 as usize) };
    ft
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _time64(out: *mut i64) -> i64 {
    let t = (now_100ns().saturating_sub(EPOCH_DIFFERENCE) / 10_000_000) as i64;
    if !out.is_null() {
        unsafe { *out = t };
    }
    t
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _time32(out: *mut i32) -> i32 {
    let t = unsafe { _time64(core::ptr::null_mut()) } as i32;
    if !out.is_null() {
        unsafe { *out = t };
    }
    t
}
/// The zone's bias in seconds (UTC - local), `_timezone`; daylight time
/// is not applied.
static TIMEZONE: AtomicI32 = AtomicI32::new(0);
static DAYLIGHT: AtomicI32 = AtomicI32::new(0);
static DSTBIAS: AtomicI32 = AtomicI32::new(-3600);
#[unsafe(no_mangle)]
pub extern "C" fn __timezone() -> *mut i32 {
    TIMEZONE.as_ptr()
}
#[unsafe(no_mangle)]
pub extern "C" fn __daylight() -> *mut i32 {
    DAYLIGHT.as_ptr()
}
#[unsafe(no_mangle)]
pub extern "C" fn __dstbias() -> *mut i32 {
    DSTBIAS.as_ptr()
}
#[unsafe(no_mangle)]
pub extern "C" fn _get_timezone(out: *mut i32) -> i32 {
    if out.is_null() {
        return errno::EINVAL;
    }
    unsafe { *out = TIMEZONE.load(Ordering::Relaxed) };
    0
}
#[unsafe(no_mangle)]
pub extern "C" fn _get_daylight(out: *mut i32) -> i32 {
    if out.is_null() {
        return errno::EINVAL;
    }
    unsafe { *out = DAYLIGHT.load(Ordering::Relaxed) };
    0
}
#[unsafe(no_mangle)]
pub extern "C" fn _get_dstbias(out: *mut i32) -> i32 {
    if out.is_null() {
        return errno::EINVAL;
    }
    unsafe { *out = DSTBIAS.load(Ordering::Relaxed) };
    0
}
static mut TZNAMES: [[u8; 64]; 2] = {
    let mut n = [[0u8; 64]; 2];
    n[0][0] = b'U';
    n[0][1] = b'T';
    n[0][2] = b'C';
    n[1][0] = b'U';
    n[1][1] = b'T';
    n[1][2] = b'C';
    n
};
static mut TZNAME_POINTERS: [usize; 2] = [0; 2];
#[unsafe(no_mangle)]
pub extern "C" fn __tzname() -> *mut usize {
    unsafe {
        let names = &*core::ptr::addr_of!(TZNAMES);
        let pointers = &mut *core::ptr::addr_of_mut!(TZNAME_POINTERS);
        pointers[0] = names[0].as_ptr() as usize;
        pointers[1] = names[1].as_ptr() as usize;
        pointers.as_mut_ptr()
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn _tzset() {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _gmtime64_s(out: *mut Tm, t: *const i64) -> i32 {
    if out.is_null() || t.is_null() {
        invalid_parameter();
        return errno::EINVAL;
    }
    match time::gmtime(unsafe { *t }) {
        Some(tm) => {
            unsafe { *out = tm };
            0
        }
        None => {
            unsafe {
                *out = Tm {
                    mday: -1,
                    mon: -1,
                    year: -1,
                    ..Tm::default()
                }
            };
            invalid_parameter();
            errno::EINVAL
        }
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _gmtime64(t: *const i64) -> *mut Tm {
    let slot = &mut thread().tm;
    if unsafe { _gmtime64_s(slot, t) } == 0 {
        slot
    } else {
        core::ptr::null_mut()
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _gmtime32_s(out: *mut Tm, t: *const i32) -> i32 {
    if t.is_null() {
        return errno::EINVAL;
    }
    let wide = i64::from(unsafe { *t });
    unsafe { _gmtime64_s(out, &wide) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _localtime64_s(out: *mut Tm, t: *const i64) -> i32 {
    if t.is_null() {
        return errno::EINVAL;
    }
    let local = unsafe { *t } - i64::from(TIMEZONE.load(Ordering::Relaxed));
    unsafe { _gmtime64_s(out, &local) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _localtime64(t: *const i64) -> *mut Tm {
    let slot = &mut thread().tm;
    if unsafe { _localtime64_s(slot, t) } == 0 {
        slot
    } else {
        core::ptr::null_mut()
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _localtime32_s(out: *mut Tm, t: *const i32) -> i32 {
    if t.is_null() {
        return errno::EINVAL;
    }
    let wide = i64::from(unsafe { *t });
    unsafe { _localtime64_s(out, &wide) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _mkgmtime64(tm: *mut Tm) -> i64 {
    if tm.is_null() {
        return -1;
    }
    match time::timegm(unsafe { &*tm }) {
        Some(t) => {
            if let Some(normal) = time::gmtime(t) {
                unsafe { *tm = normal };
            }
            t
        }
        None => -1,
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _mktime64(tm: *mut Tm) -> i64 {
    if tm.is_null() {
        set_errno(errno::EINVAL);
        return -1;
    }
    let bias = i64::from(TIMEZONE.load(Ordering::Relaxed));
    match time::timegm(unsafe { &*tm }) {
        Some(local) => {
            let t = local + bias;
            if let Some(normal) = time::gmtime(local) {
                unsafe { *tm = normal };
            }
            t
        }
        None => -1,
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _mktime32(tm: *mut Tm) -> i32 {
    let t = unsafe { _mktime64(tm) };
    if t > i64::from(i32::MAX) {
        -1
    } else {
        t as i32
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _difftime64(end: i64, start: i64) -> f64 {
    (end - start) as f64
}
unsafe fn strftime_into<T: Copy + PartialEq + Default>(
    buffer: *mut T,
    capacity: usize,
    format: *const T,
    tm: *const Tm,
    unit: fn(u16) -> T,
    read: fn(T) -> u16,
) -> usize {
    if buffer.is_null() || format.is_null() || tm.is_null() || capacity == 0 {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return 0;
    }
    let n = unsafe { super::length(format) };
    let mut units = [0u16; 512];
    let n = n.min(units.len());
    for (i, u) in units.iter_mut().enumerate().take(n) {
        *u = read(unsafe { *format.add(i) });
    }
    let mut written = 0usize;
    let result = time::strftime(&units[..n], unsafe { &*tm }, &mut |u| {
        if written + 1 >= capacity {
            return false;
        }
        unsafe { *buffer.add(written) = unit(u) };
        written += 1;
        true
    });
    match result {
        Some(count) => {
            unsafe { *buffer.add(count) = unit(0) };
            count
        }
        None => {
            unsafe { *buffer = unit(0) };
            set_errno(errno::ERANGE);
            0
        }
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strftime(
    buffer: *mut u8,
    capacity: usize,
    format: *const u8,
    tm: *const Tm,
) -> usize {
    unsafe { strftime_into(buffer, capacity, format, tm, |u| u as u8, u16::from) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsftime(
    buffer: *mut u16,
    capacity: usize,
    format: *const u16,
    tm: *const Tm,
) -> usize {
    unsafe { strftime_into(buffer, capacity, format, tm, |u| u, |u| u) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _strftime_l(
    buffer: *mut u8,
    capacity: usize,
    format: *const u8,
    tm: *const Tm,
    _l: usize,
) -> usize {
    unsafe { strftime(buffer, capacity, format, tm) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wcsftime_l(
    buffer: *mut u16,
    capacity: usize,
    format: *const u16,
    tm: *const Tm,
    _l: usize,
) -> usize {
    unsafe { wcsftime(buffer, capacity, format, tm) }
}
static START: AtomicU64 = AtomicU64::new(0);
/// `clock`: milliseconds since the process's CRT started (CLOCKS_PER_SEC
/// is 1000).
#[unsafe(no_mangle)]
pub extern "C" fn clock() -> i32 {
    let now = now_100ns();
    let start = match START.compare_exchange(0, now, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => now,
        Err(start) => start,
    };
    ((now - start) / 10_000) as i32
}
/// `_ftime64_s`: seconds, milliseconds, the zone's minutes west.
#[repr(C)]
pub struct TimeB {
    time: i64,
    millitm: u16,
    timezone: i16,
    dstflag: i16,
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _ftime64_s(out: *mut TimeB) -> i32 {
    if out.is_null() {
        return errno::EINVAL;
    }
    let units = now_100ns().saturating_sub(EPOCH_DIFFERENCE);
    unsafe {
        *out = TimeB {
            time: (units / 10_000_000) as i64,
            millitm: ((units / 10_000) % 1000) as u16,
            timezone: (TIMEZONE.load(Ordering::Relaxed) / 60) as i16,
            dstflag: 0,
        };
    }
    0
}
/// `timespec_get(TIME_UTC)`.
#[repr(C)]
pub struct TimeSpec {
    seconds: i64,
    nanoseconds: i32,
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _timespec64_get(out: *mut TimeSpec, base: i32) -> i32 {
    if out.is_null() || base != 1 {
        return 0;
    }
    let units = now_100ns().saturating_sub(EPOCH_DIFFERENCE);
    unsafe {
        *out = TimeSpec {
            seconds: (units / 10_000_000) as i64,
            nanoseconds: ((units % 10_000_000) * 100) as i32,
        };
    }
    base
}
