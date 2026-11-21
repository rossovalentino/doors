//! String and character functions (api-ms-win-crt-string): the narrow and
//! wide forms from one generic body each, the secure `_s` forms with the
//! CRT's error rules (EINVAL / ERANGE / STRUNCATE, the destination emptied
//! on failure), and the "C" locale's character classes.
use super::{errno, heap, invalid_parameter, set_errno, thread};
use door_crt::ctype;

/// A character unit: u8 (narrow) or u16 (wide).
pub(crate) trait Unit: Copy + PartialEq + Default + Into<u32> {
    fn from_u32(v: u32) -> Self;
}
impl Unit for u8 {
    fn from_u32(v: u32) -> Self {
        v as u8
    }
}
impl Unit for u16 {
    fn from_u32(v: u32) -> Self {
        v as u16
    }
}
unsafe fn len<T: Unit>(s: *const T) -> usize {
    unsafe { super::length(s) }
}
unsafe fn nlen<T: Unit>(s: *const T, max: usize) -> usize {
    let mut n = 0;
    while n < max && unsafe { *s.add(n) } != T::default() {
        n += 1;
    }
    n
}
fn fold(u: u32) -> u32 {
    if (u32::from(b'A')..=u32::from(b'Z')).contains(&u) {
        u + 32
    } else if (128..0x10000).contains(&u) {
        u32::from(ctype::lower(u as u16))
    } else {
        u
    }
}
unsafe fn cmp<T: Unit>(a: *const T, b: *const T, max: usize, ignore_case: bool) -> i32 {
    for i in 0..max {
        let (x, y) = unsafe { ((*a.add(i)).into(), (*b.add(i)).into()) };
        let (x, y) = if ignore_case {
            (fold(x), fold(y))
        } else {
            (x, y)
        };
        if x != y {
            return if x < y { -1 } else { 1 };
        }
        if x == 0 {
            return 0;
        }
    }
    0
}
unsafe fn dup<T: Unit>(s: *const T) -> *mut T {
    if s.is_null() {
        return core::ptr::null_mut();
    }
    let n = unsafe { len(s) } + 1;
    let p = heap::malloc(n * core::mem::size_of::<T>()) as *mut T;
    if !p.is_null() {
        unsafe { core::ptr::copy_nonoverlapping(s, p, n) };
    }
    p
}
/// The secure copy/append core: `destination[..capacity]` gets
/// `prefix_len` kept units, then up to `count` units of `source`.
unsafe fn secure_copy<T: Unit>(
    destination: *mut T,
    capacity: usize,
    keep: usize,
    source: *const T,
    count: usize,
) -> i32 {
    if destination.is_null() || capacity == 0 {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return errno::EINVAL;
    }
    if source.is_null() {
        unsafe { *destination = T::default() };
        invalid_parameter();
        set_errno(errno::EINVAL);
        return errno::EINVAL;
    }
    let truncate = count == usize::MAX - 1;
    let wanted = unsafe { nlen(source, if truncate { usize::MAX } else { count }) };
    if keep + wanted < capacity {
        unsafe {
            core::ptr::copy(source, destination.add(keep), wanted);
            *destination.add(keep + wanted) = T::default();
        }
        return 0;
    }
    if truncate {
        let fit = capacity - 1 - keep;
        unsafe {
            core::ptr::copy(source, destination.add(keep), fit);
            *destination.add(capacity - 1) = T::default();
        }
        return errno::STRUNCATE;
    }
    unsafe { *destination = T::default() };
    invalid_parameter();
    set_errno(errno::ERANGE);
    errno::ERANGE
}
/// `_TRUNCATE` as the count argument.
const TRUNCATE: usize = usize::MAX;

macro_rules! both {
    ($narrow:ident, $wide:ident, |$t:ident| $body:expr, ($($arg:ident: $ty:ty),*) -> $ret:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $narrow($($arg: $ty),*) -> $ret {
            type $t = u8;
            unsafe { $body }
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $wide($($arg: $ty),*) -> $ret {
            type $t = u16;
            unsafe { $body }
        }
    };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strlen(s: *const u8) -> usize {
    unsafe { len(s) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcslen(s: *const u16) -> usize {
    unsafe { len(s) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strnlen(s: *const u8, max: usize) -> usize {
    if s.is_null() {
        0
    } else {
        unsafe { nlen(s, max) }
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsnlen(s: *const u16, max: usize) -> usize {
    if s.is_null() {
        0
    } else {
        unsafe { nlen(s, max) }
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcmp(a: *const u8, b: *const u8) -> i32 {
    unsafe { cmp(a, b, usize::MAX, false) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcscmp(a: *const u16, b: *const u16) -> i32 {
    unsafe { cmp(a, b, usize::MAX, false) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strncmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    unsafe { cmp(a, b, n, false) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsncmp(a: *const u16, b: *const u16, n: usize) -> i32 {
    unsafe { cmp(a, b, n, false) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _stricmp(a: *const u8, b: *const u8) -> i32 {
    unsafe { cmp(a, b, usize::MAX, true) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wcsicmp(a: *const u16, b: *const u16) -> i32 {
    unsafe { cmp(a, b, usize::MAX, true) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _strnicmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    unsafe { cmp(a, b, n, true) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wcsnicmp(a: *const u16, b: *const u16, n: usize) -> i32 {
    unsafe { cmp(a, b, n, true) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _stricmp_l(a: *const u8, b: *const u8, _l: usize) -> i32 {
    unsafe { cmp(a, b, usize::MAX, true) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wcsicmp_l(a: *const u16, b: *const u16, _l: usize) -> i32 {
    unsafe { cmp(a, b, usize::MAX, true) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _strnicmp_l(a: *const u8, b: *const u8, n: usize, _l: usize) -> i32 {
    unsafe { cmp(a, b, n, true) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wcsnicmp_l(a: *const u16, b: *const u16, n: usize, _l: usize) -> i32 {
    unsafe { cmp(a, b, n, true) }
}
/// strcoll and friends: the "C" locale collates by code units.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcoll(a: *const u8, b: *const u8) -> i32 {
    unsafe { cmp(a, b, usize::MAX, false) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcscoll(a: *const u16, b: *const u16) -> i32 {
    unsafe { cmp(a, b, usize::MAX, false) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _stricoll(a: *const u8, b: *const u8) -> i32 {
    unsafe { cmp(a, b, usize::MAX, true) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wcsicoll(a: *const u16, b: *const u16) -> i32 {
    unsafe { cmp(a, b, usize::MAX, true) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _strdup(s: *const u8) -> *mut u8 {
    unsafe { dup(s) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wcsdup(s: *const u16) -> *mut u16 {
    unsafe { dup(s) }
}
both!(strcpy, wcscpy, |T| {
    let n = len(source as *const T) + 1;
    core::ptr::copy(source as *const T, destination as *mut T, n);
    destination
}, (destination: usize, source: usize) -> usize);
both!(strcat, wcscat, |T| {
    let d = destination as *mut T;
    let end = len(d);
    let n = len(source as *const T) + 1;
    core::ptr::copy(source as *const T, d.add(end), n);
    destination
}, (destination: usize, source: usize) -> usize);
both!(strncpy, wcsncpy, |T| {
    let s = source as *const T;
    let d = destination as *mut T;
    let n = nlen(s, count);
    core::ptr::copy(s, d, n);
    for i in n..count {
        *d.add(i) = T::default();
    }
    destination
}, (destination: usize, source: usize, count: usize) -> usize);
both!(strncat, wcsncat, |T| {
    let d = destination as *mut T;
    let end = len(d);
    let n = nlen(source as *const T, count);
    core::ptr::copy(source as *const T, d.add(end), n);
    *d.add(end + n) = T::default();
    destination
}, (destination: usize, source: usize, count: usize) -> usize);
both!(strcpy_s, wcscpy_s, |T| {
    secure_copy(destination as *mut T, capacity, 0, source as *const T, usize::MAX)
}, (destination: usize, capacity: usize, source: usize) -> i32);
both!(strncpy_s, wcsncpy_s, |T| {
    let count = if count == TRUNCATE { usize::MAX - 1 } else { count };
    secure_copy(destination as *mut T, capacity, 0, source as *const T, count)
}, (destination: usize, capacity: usize, source: usize, count: usize) -> i32);
both!(strcat_s, wcscat_s, |T| {
    let d = destination as *mut T;
    if d.is_null() || capacity == 0 {
        return secure_copy(d, capacity, 0, source as *const T, usize::MAX);
    }
    let keep = nlen(d, capacity);
    if keep == capacity {
        *d = T::default();
        invalid_parameter();
        set_errno(errno::EINVAL);
        return errno::EINVAL;
    }
    secure_copy(d, capacity, keep, source as *const T, usize::MAX)
}, (destination: usize, capacity: usize, source: usize) -> i32);
both!(strncat_s, wcsncat_s, |T| {
    let d = destination as *mut T;
    if d.is_null() || capacity == 0 {
        return secure_copy(d, capacity, 0, source as *const T, count);
    }
    let keep = nlen(d, capacity);
    if keep == capacity {
        *d = T::default();
        invalid_parameter();
        set_errno(errno::EINVAL);
        return errno::EINVAL;
    }
    let count = if count == TRUNCATE { usize::MAX - 1 } else { count };
    secure_copy(d, capacity, keep, source as *const T, count)
}, (destination: usize, capacity: usize, source: usize, count: usize) -> i32);

/// strtok_s / wcstok_s: the next token of `s` (or of `*context`).
unsafe fn token<T: Unit>(s: *mut T, delimiters: *const T, context: *mut *mut T) -> *mut T {
    let is_delimiter = |u: T| {
        let mut i = 0;
        loop {
            let d = unsafe { *delimiters.add(i) };
            if d == T::default() {
                return false;
            }
            if d == u {
                return true;
            }
            i += 1;
        }
    };
    let mut p = if s.is_null() { unsafe { *context } } else { s };
    if p.is_null() {
        return core::ptr::null_mut();
    }
    while unsafe { *p } != T::default() && is_delimiter(unsafe { *p }) {
        p = unsafe { p.add(1) };
    }
    if unsafe { *p } == T::default() {
        unsafe { *context = p };
        return core::ptr::null_mut();
    }
    let start = p;
    while unsafe { *p } != T::default() && !is_delimiter(unsafe { *p }) {
        p = unsafe { p.add(1) };
    }
    if unsafe { *p } != T::default() {
        unsafe {
            *p = T::default();
            p = p.add(1);
        }
    }
    unsafe { *context = p };
    start
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtok_s(
    s: *mut u8,
    delimiters: *const u8,
    context: *mut *mut u8,
) -> *mut u8 {
    unsafe { token(s, delimiters, context) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcstok_s(
    s: *mut u16,
    delimiters: *const u16,
    context: *mut *mut u16,
) -> *mut u16 {
    unsafe { token(s, delimiters, context) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtok(s: *mut u8, delimiters: *const u8) -> *mut u8 {
    let t = thread();
    unsafe { token(s, delimiters, &mut t.strtok as *mut usize as *mut *mut u8) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcstok(
    s: *mut u16,
    delimiters: *const u16,
    context: *mut *mut u16,
) -> *mut u16 {
    let t = thread();
    let context = if context.is_null() {
        &mut t.wcstok as *mut usize as *mut *mut u16
    } else {
        context
    };
    unsafe { token(s, delimiters, context) }
}
both!(strspn, wcsspn, |T| {
    let s = s as *const T;
    let set = set as *const T;
    let mut n = 0;
    while *s.add(n) != T::default() && member(set, *s.add(n)) {
        n += 1;
    }
    n
}, (s: usize, set: usize) -> usize);
both!(strcspn, wcscspn, |T| {
    let s = s as *const T;
    let set = set as *const T;
    let mut n = 0;
    while *s.add(n) != T::default() && !member(set, *s.add(n)) {
        n += 1;
    }
    n
}, (s: usize, set: usize) -> usize);
both!(strpbrk, wcspbrk, |T| {
    let p = s as *const T;
    let set = set as *const T;
    let mut i = 0;
    loop {
        let u = *p.add(i);
        if u == T::default() {
            return 0;
        }
        if member(set, u) {
            return p.add(i) as usize;
        }
        i += 1;
    }
}, (s: usize, set: usize) -> usize);
unsafe fn member<T: Unit>(set: *const T, u: T) -> bool {
    let mut i = 0;
    loop {
        let d = unsafe { *set.add(i) };
        if d == T::default() {
            return false;
        }
        if d == u {
            return true;
        }
        i += 1;
    }
}
/// The case changes of `_strlwr`, `_wcsupr` and their `_s` forms.
unsafe fn change<T: Unit>(s: *mut T, max: usize, upper: bool) {
    let mut i = 0;
    while i < max && unsafe { *s.add(i) } != T::default() {
        let u: u32 = unsafe { *s.add(i) }.into();
        let v = if u < 0x10000 {
            u32::from(if upper {
                ctype::upper(u as u16)
            } else {
                ctype::lower(u as u16)
            })
        } else {
            u
        };
        // Narrow strings are UTF-8: only ASCII bytes change.
        if core::mem::size_of::<T>() == 2 || u < 128 {
            unsafe { *s.add(i) = T::from_u32(v) };
        }
        i += 1;
    }
}
both!(_strlwr, _wcslwr, |T| { change(s as *mut T, usize::MAX, false); s }, (s: usize) -> usize);
both!(_strupr, _wcsupr, |T| { change(s as *mut T, usize::MAX, true); s }, (s: usize) -> usize);
both!(_strlwr_s, _wcslwr_s, |T| {
    if s == 0 || nlen(s as *const T, capacity) == capacity {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return errno::EINVAL;
    }
    change(s as *mut T, capacity, false);
    0
}, (s: usize, capacity: usize) -> i32);
both!(_strupr_s, _wcsupr_s, |T| {
    if s == 0 || nlen(s as *const T, capacity) == capacity {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return errno::EINVAL;
    }
    change(s as *mut T, capacity, true);
    0
}, (s: usize, capacity: usize) -> i32);
both!(_strrev, _wcsrev, |T| {
    let p = s as *mut T;
    let n = len(p);
    let (mut a, mut b) = (0, n);
    while a + 1 < b {
        b -= 1;
        core::ptr::swap(p.add(a), p.add(b));
        a += 1;
    }
    s
}, (s: usize) -> usize);

// memcpy_s and friends (the plain mem* are vcruntime's).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcpy_s(
    destination: *mut u8,
    capacity: usize,
    source: *const u8,
    count: usize,
) -> i32 {
    unsafe { memmove_s(destination, capacity, source, count) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memmove_s(
    destination: *mut u8,
    capacity: usize,
    source: *const u8,
    count: usize,
) -> i32 {
    if count == 0 {
        return 0;
    }
    if destination.is_null() || source.is_null() {
        if !destination.is_null() {
            unsafe { core::ptr::write_bytes(destination, 0, capacity) };
        }
        invalid_parameter();
        set_errno(errno::EINVAL);
        return errno::EINVAL;
    }
    if capacity < count {
        unsafe { core::ptr::write_bytes(destination, 0, capacity) };
        invalid_parameter();
        set_errno(errno::ERANGE);
        return errno::ERANGE;
    }
    unsafe { core::ptr::copy(source, destination, count) };
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wmemcpy_s(
    destination: *mut u16,
    capacity: usize,
    source: *const u16,
    count: usize,
) -> i32 {
    unsafe {
        memmove_s(
            destination as *mut u8,
            capacity * 2,
            source as *const u8,
            count * 2,
        )
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wmemmove_s(
    destination: *mut u16,
    capacity: usize,
    source: *const u16,
    count: usize,
) -> i32 {
    unsafe {
        memmove_s(
            destination as *mut u8,
            capacity * 2,
            source as *const u8,
            count * 2,
        )
    }
}

// Character classes ("C" locale; the `_l` forms ignore the locale).
fn class(c: i32) -> u16 {
    if (-1..=255).contains(&c) {
        ctype::TABLE[(c + 1) as usize]
    } else {
        0
    }
}
macro_rules! classifier {
    ($($name:ident, $wname:ident, $bits:expr;)*) => {$(
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(c: i32) -> i32 {
            i32::from(class(c) & $bits)
        }
        #[unsafe(no_mangle)]
        pub extern "C" fn $wname(c: u16) -> i32 {
            i32::from(ctype::wide_class(c) & $bits)
        }
    )*};
}
classifier! {
    isalpha, iswalpha, ctype::ALPHA;
    isupper, iswupper, ctype::UPPER;
    islower, iswlower, ctype::LOWER;
    isdigit, iswdigit, ctype::DIGIT;
    isxdigit, iswxdigit, ctype::HEX;
    isspace, iswspace, ctype::SPACE;
    ispunct, iswpunct, ctype::PUNCT;
    isalnum, iswalnum, ctype::ALPHA | ctype::DIGIT;
    isprint, iswprint, ctype::ALPHA | ctype::DIGIT | ctype::PUNCT | ctype::BLANK;
    isgraph, iswgraph, ctype::ALPHA | ctype::DIGIT | ctype::PUNCT;
    iscntrl, iswcntrl, ctype::CONTROL;
    isblank, iswblank, ctype::BLANK;
}
#[unsafe(no_mangle)]
pub extern "C" fn iswascii(c: u16) -> i32 {
    i32::from(c < 128)
}
#[unsafe(no_mangle)]
pub extern "C" fn __isascii(c: i32) -> i32 {
    i32::from((0..128).contains(&c))
}
#[unsafe(no_mangle)]
pub extern "C" fn iswctype(c: u16, mask: u16) -> i32 {
    i32::from(ctype::wide_class(c) & mask)
}
#[unsafe(no_mangle)]
pub extern "C" fn _isctype(c: i32, mask: i32) -> i32 {
    i32::from(class(c) & mask as u16)
}
#[unsafe(no_mangle)]
pub extern "C" fn _isctype_l(c: i32, mask: i32, _l: usize) -> i32 {
    _isctype(c, mask)
}
#[unsafe(no_mangle)]
pub extern "C" fn toupper(c: i32) -> i32 {
    if (i32::from(b'a')..=i32::from(b'z')).contains(&c) {
        c - 32
    } else {
        c
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn tolower(c: i32) -> i32 {
    if (i32::from(b'A')..=i32::from(b'Z')).contains(&c) {
        c + 32
    } else {
        c
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn _toupper(c: i32) -> i32 {
    c - 32
}
#[unsafe(no_mangle)]
pub extern "C" fn _tolower(c: i32) -> i32 {
    c + 32
}
#[unsafe(no_mangle)]
pub extern "C" fn towupper(c: u16) -> u16 {
    ctype::upper(c)
}
#[unsafe(no_mangle)]
pub extern "C" fn towlower(c: u16) -> u16 {
    ctype::lower(c)
}
#[unsafe(no_mangle)]
pub extern "C" fn _towupper_l(c: u16, _l: usize) -> u16 {
    ctype::upper(c)
}
#[unsafe(no_mangle)]
pub extern "C" fn _towlower_l(c: u16, _l: usize) -> u16 {
    ctype::lower(c)
}
#[unsafe(no_mangle)]
pub extern "C" fn _toupper_l(c: i32, _l: usize) -> i32 {
    toupper(c)
}
#[unsafe(no_mangle)]
pub extern "C" fn _tolower_l(c: i32, _l: usize) -> i32 {
    tolower(c)
}
/// `__pctype_func`: the "C" locale's table, indexable from -1 (EOF).
#[unsafe(no_mangle)]
pub extern "C" fn __pctype_func() -> *const u16 {
    unsafe { ctype::TABLE.as_ptr().add(1) }
}
static WIDE_TABLE: [u16; 256] = {
    let mut t = [0u16; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = ctype::class(i as u8);
        i += 1;
    }
    t
};
#[unsafe(no_mangle)]
pub extern "C" fn __pwctype_func() -> *const u16 {
    WIDE_TABLE.as_ptr()
}

/// strerror's messages (the CRT's _sys_errlist).
pub(crate) fn message(code: i32) -> &'static str {
    match code {
        0 => "No error",
        1 => "Operation not permitted",
        2 => "No such file or directory",
        3 => "No such process",
        4 => "Interrupted function call",
        5 => "Input/output error",
        6 => "No such device or address",
        7 => "Arg list too long",
        8 => "Exec format error",
        9 => "Bad file descriptor",
        10 => "No child processes",
        11 => "Resource temporarily unavailable",
        12 => "Not enough space",
        13 => "Permission denied",
        14 => "Bad address",
        16 => "Resource device",
        17 => "File exists",
        18 => "Improper link",
        19 => "No such device",
        20 => "Not a directory",
        21 => "Is a directory",
        22 => "Invalid argument",
        23 => "Too many open files in system",
        24 => "Too many open files",
        25 => "Inappropriate I/O control operation",
        27 => "File too large",
        28 => "No space left on device",
        29 => "Invalid seek",
        30 => "Read-only file system",
        31 => "Too many links",
        32 => "Broken pipe",
        33 => "Domain error",
        34 => "Result too large",
        36 => "Resource deadlock avoided",
        38 => "Filename too long",
        39 => "No locks available",
        40 => "Function not implemented",
        41 => "Directory not empty",
        42 => "Illegal byte sequence",
        _ => "Unknown error",
    }
}
static mut STRERROR: [u8; 64] = [0; 64];
#[unsafe(no_mangle)]
pub extern "C" fn strerror(code: i32) -> *const u8 {
    let m = message(code).as_bytes();
    // SAFETY: as the CRT's, the buffer is shared (strerror is not
    // thread-safe; strerror_s is).
    unsafe {
        let b = &mut *core::ptr::addr_of_mut!(STRERROR);
        b[..m.len()].copy_from_slice(m);
        b[m.len()] = 0;
        b.as_ptr()
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strerror_s(buffer: *mut u8, capacity: usize, code: i32) -> i32 {
    let m = message(code).as_bytes();
    unsafe { secure_copy(buffer, capacity, 0, m.as_ptr(), m.len()) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wcserror_s(buffer: *mut u16, capacity: usize, code: i32) -> i32 {
    if buffer.is_null() || capacity == 0 {
        invalid_parameter();
        return errno::EINVAL;
    }
    let m = message(code).as_bytes();
    let n = m.len().min(capacity - 1);
    for (i, &b) in m[..n].iter().enumerate() {
        unsafe { *buffer.add(i) = u16::from(b) };
    }
    unsafe { *buffer.add(n) = 0 };
    if n < m.len() { errno::STRUNCATE } else { 0 }
}
static mut WCSERROR: [u16; 64] = [0; 64];
#[unsafe(no_mangle)]
pub extern "C" fn _wcserror(code: i32) -> *const u16 {
    unsafe {
        let b = &mut *core::ptr::addr_of_mut!(WCSERROR);
        _wcserror_s(b.as_mut_ptr(), b.len(), code);
        b.as_ptr()
    }
}
