//! The locale set (api-ms-win-crt-locale): the "C" locale only, with the
//! UTF-8 code page (65001) for multibyte text, as a program that calls
//! setlocale(LC_ALL, ".UTF-8") or runs on a UTF-8 system sees it. A locale
//! name a program asks for is accepted when it is "C", "" (the user
//! default, here "C" with UTF-8) or a ".UTF-8" form, as M0e's
//! globalization work extends.
use super::{errno, heap, invalid_parameter, set_errno, thread};
use core::sync::atomic::{AtomicI32, Ordering};

static C_NAME: [u8; 2] = *b"C\0";
static C_WIDE: [u16; 2] = [b'C' as u16, 0];
static UTF8_NAME: [u8; 6] = *b"C.UTF8";
static mut CURRENT: [u8; 32] = {
    let mut n = [0u8; 32];
    n[0] = b'C';
    n
};

/// Whether a locale name is one Doors provides.
unsafe fn supported<T: Copy + Into<u32>>(name: *const T) -> bool {
    let n = unsafe { super::length_of_units(name) };
    let mut text = [0u8; 64];
    if n > text.len() {
        return false;
    }
    for (i, b) in text.iter_mut().enumerate().take(n) {
        let u: u32 = unsafe { *name.add(i) }.into();
        *b = (u as u8).to_ascii_lowercase();
    }
    let t = &text[..n];
    t.is_empty()
        || t == b"c"
        || t == b"posix"
        || t.ends_with(b".utf-8")
        || t.ends_with(b".utf8")
        || t == b"en-us"
        || t == b"en_us"
        || t.starts_with(b"english")
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn setlocale(category: i32, name: *const u8) -> *const u8 {
    if !(0..=5).contains(&category) {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return core::ptr::null();
    }
    if name.is_null() {
        return core::ptr::addr_of!(CURRENT) as *const u8;
    }
    if !unsafe { supported(name) } {
        return core::ptr::null();
    }
    let _ = UTF8_NAME;
    core::ptr::addr_of!(CURRENT) as *const u8
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wsetlocale(category: i32, name: *const u16) -> *const u16 {
    if !(0..=5).contains(&category) {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return core::ptr::null();
    }
    if !name.is_null() && !unsafe { supported(name) } {
        return core::ptr::null();
    }
    C_WIDE.as_ptr()
}
/// `_locale_t`: a pointer to two pointers (the locale and multibyte
/// information), both to the shared "C" data here.
#[repr(C)]
pub struct LocaleData {
    locinfo: usize,
    mbcinfo: usize,
}
static C_LOCALE: LocaleData = LocaleData {
    locinfo: 0,
    mbcinfo: 0,
};
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _create_locale(category: i32, name: *const u8) -> *const LocaleData {
    if !(0..=5).contains(&category) || name.is_null() || !unsafe { supported(name) } {
        return core::ptr::null();
    }
    let p = heap::malloc(core::mem::size_of::<LocaleData>()) as *mut LocaleData;
    if !p.is_null() {
        unsafe {
            *p = LocaleData {
                locinfo: 0,
                mbcinfo: 0,
            }
        };
    }
    p
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wcreate_locale(category: i32, name: *const u16) -> *const LocaleData {
    if !(0..=5).contains(&category) || name.is_null() || !unsafe { supported(name) } {
        return core::ptr::null();
    }
    unsafe { _create_locale(category, C_NAME.as_ptr()) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _free_locale(locale: *mut LocaleData) {
    if !locale.is_null() && !core::ptr::eq(locale, &C_LOCALE) {
        unsafe { heap::free(locale as *mut u8) };
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn _get_current_locale() -> *const LocaleData {
    unsafe { _create_locale(0, C_NAME.as_ptr()) }
}
static PER_THREAD: AtomicI32 = AtomicI32::new(2);
/// `_configthreadlocale`: 1 per-thread, 2 global, 0 or -1 query/reset.
#[unsafe(no_mangle)]
pub extern "C" fn _configthreadlocale(kind: i32) -> i32 {
    let t = thread();
    let old = if t.own_locale != 0 { 1 } else { 2 };
    match kind {
        0 => {}
        1 => t.own_locale = 1,
        2 => t.own_locale = 0,
        -1 => PER_THREAD.store(2, Ordering::Relaxed),
        _ => {
            invalid_parameter();
            set_errno(errno::EINVAL);
            return -1;
        }
    }
    old
}
#[unsafe(no_mangle)]
pub extern "C" fn ___lc_codepage_func() -> u32 {
    65001
}
#[unsafe(no_mangle)]
pub extern "C" fn ___lc_collate_cp_func() -> u32 {
    65001
}
#[unsafe(no_mangle)]
pub extern "C" fn ___lc_locale_name_func() -> *const usize {
    // LC_ALL..LC_TIME names: null (the "C" locale), as the CRT returns.
    static NAMES: [usize; 6] = [0; 6];
    NAMES.as_ptr()
}
#[unsafe(no_mangle)]
pub extern "C" fn _lock_locales() {}
#[unsafe(no_mangle)]
pub extern "C" fn _unlock_locales() {}
#[unsafe(no_mangle)]
pub extern "C" fn _getmbcp() -> i32 {
    65001
}
#[unsafe(no_mangle)]
pub extern "C" fn _setmbcp(code_page: i32) -> i32 {
    // _MB_CP_SBCS 0, _MB_CP_OEM -2, _MB_CP_ANSI -3, _MB_CP_LOCALE -4, UTF-8.
    if matches!(code_page, 0 | -2 | -3 | -4 | 65001) {
        0
    } else {
        set_errno(errno::EINVAL);
        -1
    }
}
/// `localeconv`: the "C" locale's numeric and monetary conventions.
#[repr(C)]
pub struct Lconv {
    decimal_point: *const u8,
    thousands_sep: *const u8,
    grouping: *const u8,
    int_curr_symbol: *const u8,
    currency_symbol: *const u8,
    mon_decimal_point: *const u8,
    mon_thousands_sep: *const u8,
    mon_grouping: *const u8,
    positive_sign: *const u8,
    negative_sign: *const u8,
    int_frac_digits: u8,
    frac_digits: u8,
    p_cs_precedes: u8,
    p_sep_by_space: u8,
    n_cs_precedes: u8,
    n_sep_by_space: u8,
    p_sign_posn: u8,
    n_sign_posn: u8,
    w_decimal_point: *const u16,
    w_thousands_sep: *const u16,
    w_int_curr_symbol: *const u16,
    w_currency_symbol: *const u16,
    w_mon_decimal_point: *const u16,
    w_mon_thousands_sep: *const u16,
    w_positive_sign: *const u16,
    w_negative_sign: *const u16,
}
unsafe impl Sync for Lconv {}
static POINT: [u8; 2] = *b".\0";
static EMPTY: [u8; 1] = [0];
static WPOINT: [u16; 2] = [b'.' as u16, 0];
static WEMPTY: [u16; 1] = [0];
static LCONV: Lconv = Lconv {
    decimal_point: POINT.as_ptr(),
    thousands_sep: EMPTY.as_ptr(),
    grouping: EMPTY.as_ptr(),
    int_curr_symbol: EMPTY.as_ptr(),
    currency_symbol: EMPTY.as_ptr(),
    mon_decimal_point: EMPTY.as_ptr(),
    mon_thousands_sep: EMPTY.as_ptr(),
    mon_grouping: EMPTY.as_ptr(),
    positive_sign: EMPTY.as_ptr(),
    negative_sign: EMPTY.as_ptr(),
    int_frac_digits: 127,
    frac_digits: 127,
    p_cs_precedes: 127,
    p_sep_by_space: 127,
    n_cs_precedes: 127,
    n_sep_by_space: 127,
    p_sign_posn: 127,
    n_sign_posn: 127,
    w_decimal_point: WPOINT.as_ptr(),
    w_thousands_sep: WEMPTY.as_ptr(),
    w_int_curr_symbol: WEMPTY.as_ptr(),
    w_currency_symbol: WEMPTY.as_ptr(),
    w_mon_decimal_point: WEMPTY.as_ptr(),
    w_mon_thousands_sep: WEMPTY.as_ptr(),
    w_positive_sign: WEMPTY.as_ptr(),
    w_negative_sign: WEMPTY.as_ptr(),
};
#[unsafe(no_mangle)]
pub extern "C" fn localeconv() -> *const Lconv {
    &LCONV
}
