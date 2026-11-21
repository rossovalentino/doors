//! Threads (`_beginthreadex`, `_endthreadex`), the filesystem set
//! (remove, rename, _waccess, _wmkdir, _wgetcwd...) and the utility set
//! (qsort, bsearch, abs, div, rand).
use super::{
    CreateDirectoryW, CreateThread, DeleteFileW, ExitThread, GetCurrentDirectoryW,
    GetFileAttributesW, MoveFileExW, convert, errno, heap, invalid_parameter, map_os_error,
    set_errno, thread,
};

#[repr(C)]
struct Start {
    routine: usize,
    argument: usize,
}
unsafe extern "system" fn trampoline(start: usize) -> u32 {
    let s = unsafe { core::ptr::read(start as *const Start) };
    unsafe { heap::free(start as *mut u8) };
    let routine: extern "system" fn(usize) -> u32 = unsafe { core::mem::transmute(s.routine) };
    let code = routine(s.argument);
    super::stdio::flush_all();
    code
}
/// `_beginthreadex`: CreateThread with the routine wrapped so the CRT's
/// per-thread data is set up on its first use and the code returned.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _beginthreadex(
    security: usize,
    stack: u32,
    routine: usize,
    argument: usize,
    flags: u32,
    id: *mut u32,
) -> usize {
    if routine == 0 {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return 0;
    }
    let block = heap::malloc(core::mem::size_of::<Start>()) as *mut Start;
    if block.is_null() {
        set_errno(errno::ENOMEM);
        return 0;
    }
    unsafe { *block = Start { routine, argument } };
    let handle = unsafe {
        CreateThread(
            security,
            stack as usize,
            trampoline as *const () as usize,
            block as usize,
            flags,
            id as usize,
        )
    };
    if handle == 0 {
        unsafe { heap::free(block as *mut u8) };
        map_os_error(unsafe { super::GetLastError() });
    }
    handle
}
#[unsafe(no_mangle)]
pub extern "C" fn _endthreadex(code: u32) -> ! {
    super::stdio::flush_all();
    unsafe { ExitThread(code) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _beginthread(routine: usize, stack: u32, argument: usize) -> usize {
    let handle = unsafe { _beginthreadex(0, stack, routine, argument, 0, core::ptr::null_mut()) };
    if handle == 0 { usize::MAX } else { handle }
}
#[unsafe(no_mangle)]
pub extern "C" fn _endthread() -> ! {
    _endthreadex(0)
}

// The filesystem set.
fn os_result(ok: bool) -> i32 {
    if ok {
        0
    } else {
        map_os_error(unsafe { super::GetLastError() });
        -1
    }
}
unsafe fn widen_then<R>(s: *const u8, f: impl FnOnce(*const u16) -> R, failed: R) -> R {
    if s.is_null() {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return failed;
    }
    let units = unsafe { convert::mbstowcs(core::ptr::null_mut(), s, 0) };
    if units == usize::MAX {
        return failed;
    }
    let p = heap::malloc((units + 1) * 2) as *mut u16;
    if p.is_null() {
        set_errno(errno::ENOMEM);
        return failed;
    }
    unsafe { convert::mbstowcs(p, s, units + 1) };
    let r = f(p);
    unsafe { heap::free(p as *mut u8) };
    r
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wremove(path: *const u16) -> i32 {
    os_result(unsafe { DeleteFileW(path as usize) } != 0)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wunlink(path: *const u16) -> i32 {
    unsafe { _wremove(path) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn remove(path: *const u8) -> i32 {
    unsafe { widen_then(path, |p| _wremove(p), -1) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _unlink(path: *const u8) -> i32 {
    unsafe { remove(path) }
}
/// rename: never replaces an existing file (as the CRT's).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wrename(from: *const u16, to: *const u16) -> i32 {
    // MOVEFILE_COPY_ALLOWED.
    os_result(unsafe { MoveFileExW(from as usize, to as usize, 2) } != 0)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rename(from: *const u8, to: *const u8) -> i32 {
    unsafe { widen_then(from, |f| widen_then(to, |t| _wrename(f, t), -1), -1) }
}
/// `_waccess_s`: 0 exists, 2 writable, 4 readable, 6 both.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _waccess_s(path: *const u16, mode: i32) -> i32 {
    if path.is_null() || mode & !6 != 0 {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return errno::EINVAL;
    }
    let attributes = unsafe { GetFileAttributesW(path as usize) };
    if attributes == u32::MAX {
        map_os_error(unsafe { super::GetLastError() });
        return super::thread().errno;
    }
    // FILE_ATTRIBUTE_READONLY on a file refuses write access.
    if mode & 2 != 0 && attributes & 1 != 0 && attributes & 0x10 == 0 {
        set_errno(errno::EACCES);
        return errno::EACCES;
    }
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _waccess(path: *const u16, mode: i32) -> i32 {
    if unsafe { _waccess_s(path, mode) } == 0 {
        0
    } else {
        -1
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _access(path: *const u8, mode: i32) -> i32 {
    unsafe { widen_then(path, |p| _waccess(p, mode), -1) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _access_s(path: *const u8, mode: i32) -> i32 {
    unsafe { widen_then(path, |p| _waccess_s(p, mode), errno::EINVAL) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wmkdir(path: *const u16) -> i32 {
    os_result(unsafe { CreateDirectoryW(path as usize, 0) } != 0)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _mkdir(path: *const u8) -> i32 {
    unsafe { widen_then(path, |p| _wmkdir(p), -1) }
}
/// `_wgetcwd`: into `buffer` (`count` units), or a new block when null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wgetcwd(buffer: *mut u16, count: i32) -> *mut u16 {
    let needed = unsafe { GetCurrentDirectoryW(0, 0) } as usize;
    if needed == 0 {
        map_os_error(unsafe { super::GetLastError() });
        return core::ptr::null_mut();
    }
    let (out, capacity) = if buffer.is_null() {
        let size = needed.max(count.max(0) as usize);
        (heap::malloc(size * 2) as *mut u16, size)
    } else {
        (buffer, count.max(0) as usize)
    };
    if out.is_null() {
        set_errno(errno::ENOMEM);
        return core::ptr::null_mut();
    }
    if capacity < needed {
        set_errno(errno::ERANGE);
        return core::ptr::null_mut();
    }
    unsafe { GetCurrentDirectoryW(capacity as u32, out as usize) };
    out
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _getcwd(buffer: *mut u8, count: i32) -> *mut u8 {
    let wide = unsafe { _wgetcwd(core::ptr::null_mut(), 0) };
    if wide.is_null() {
        return core::ptr::null_mut();
    }
    let needed = unsafe { convert::wcstombs(core::ptr::null_mut(), wide, usize::MAX) } + 1;
    let (out, capacity) = if buffer.is_null() {
        let size = needed.max(count.max(0) as usize);
        (heap::malloc(size), size)
    } else {
        (buffer, count.max(0) as usize)
    };
    let result = if out.is_null() {
        set_errno(errno::ENOMEM);
        core::ptr::null_mut()
    } else if capacity < needed {
        set_errno(errno::ERANGE);
        core::ptr::null_mut()
    } else {
        unsafe { convert::wcstombs(out, wide, capacity) };
        out
    };
    unsafe { heap::free(wide as *mut u8) };
    result
}

// The utility set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qsort(
    base: *mut u8,
    count: usize,
    size: usize,
    compare: extern "C" fn(*const u8, *const u8) -> i32,
) {
    unsafe { door_crt::sort::qsort(base, count, size, &mut |a, b| compare(a, b)) };
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qsort_s(
    base: *mut u8,
    count: usize,
    size: usize,
    compare: extern "C" fn(usize, *const u8, *const u8) -> i32,
    context: usize,
) {
    unsafe { door_crt::sort::qsort(base, count, size, &mut |a, b| compare(context, a, b)) };
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bsearch(
    key: *const u8,
    base: *const u8,
    count: usize,
    size: usize,
    compare: extern "C" fn(*const u8, *const u8) -> i32,
) -> *const u8 {
    unsafe { door_crt::sort::bsearch(key, base, count, size, &mut |a, b| compare(a, b)) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bsearch_s(
    key: *const u8,
    base: *const u8,
    count: usize,
    size: usize,
    compare: extern "C" fn(usize, *const u8, *const u8) -> i32,
    context: usize,
) -> *const u8 {
    unsafe { door_crt::sort::bsearch(key, base, count, size, &mut |a, b| compare(context, a, b)) }
}
#[unsafe(no_mangle)]
pub extern "C" fn abs(v: i32) -> i32 {
    v.wrapping_abs()
}
#[unsafe(no_mangle)]
pub extern "C" fn labs(v: i32) -> i32 {
    v.wrapping_abs()
}
#[unsafe(no_mangle)]
pub extern "C" fn llabs(v: i64) -> i64 {
    v.wrapping_abs()
}
#[unsafe(no_mangle)]
pub extern "C" fn _abs64(v: i64) -> i64 {
    v.wrapping_abs()
}
#[repr(C)]
pub struct Div32 {
    quot: i32,
    rem: i32,
}
#[repr(C)]
pub struct Div64 {
    quot: i64,
    rem: i64,
}
#[unsafe(no_mangle)]
pub extern "C" fn div(a: i32, b: i32) -> Div32 {
    Div32 {
        quot: a.wrapping_div(b),
        rem: a.wrapping_rem(b),
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn ldiv(a: i32, b: i32) -> Div32 {
    div(a, b)
}
#[unsafe(no_mangle)]
pub extern "C" fn lldiv(a: i64, b: i64) -> Div64 {
    Div64 {
        quot: a.wrapping_div(b),
        rem: a.wrapping_rem(b),
    }
}
/// rand: the CRT's linear congruential generator, per thread.
#[unsafe(no_mangle)]
pub extern "C" fn rand() -> i32 {
    let t = thread();
    t.rand = t.rand.wrapping_mul(214_013).wrapping_add(2_531_011);
    ((t.rand >> 16) & 0x7FFF) as i32
}
#[unsafe(no_mangle)]
pub extern "C" fn srand(seed: u32) {
    thread().rand = seed;
}
#[unsafe(no_mangle)]
pub extern "C" fn _rotl(v: u32, n: i32) -> u32 {
    v.rotate_left(n as u32 & 31)
}
#[unsafe(no_mangle)]
pub extern "C" fn _rotr(v: u32, n: i32) -> u32 {
    v.rotate_right(n as u32 & 31)
}
#[unsafe(no_mangle)]
pub extern "C" fn _rotl64(v: u64, n: i32) -> u64 {
    v.rotate_left(n as u32 & 63)
}
#[unsafe(no_mangle)]
pub extern "C" fn _rotr64(v: u64, n: i32) -> u64 {
    v.rotate_right(n as u32 & 63)
}
#[unsafe(no_mangle)]
pub extern "C" fn _byteswap_ushort(v: u16) -> u16 {
    v.swap_bytes()
}
#[unsafe(no_mangle)]
pub extern "C" fn _byteswap_ulong(v: u32) -> u32 {
    v.swap_bytes()
}
#[unsafe(no_mangle)]
pub extern "C" fn _byteswap_uint64(v: u64) -> u64 {
    v.swap_bytes()
}
