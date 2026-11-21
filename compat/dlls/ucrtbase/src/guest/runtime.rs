//! The runtime set (api-ms-win-crt-runtime): what vcruntime's start-up
//! code calls (`_initterm`, onexit tables, `_set_app_type`, `_seh_filter_*`),
//! exit and abort, the invalid-parameter handler, terminate, and the
//! floating-point control word.
use super::{
    ExitProcess, IsDebuggerPresent, RaiseFailFastException, errno, heap, set_errno, stdio,
};
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};

type Function = Option<extern "C" fn()>;
type IntFunction = Option<extern "C" fn() -> i32>;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _initterm(begin: *const Function, end: *const Function) {
    let mut p = begin;
    while p < end {
        if let Some(f) = unsafe { *p } {
            f();
        }
        p = unsafe { p.add(1) };
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _initterm_e(begin: *const IntFunction, end: *const IntFunction) -> i32 {
    let mut p = begin;
    while p < end {
        if let Some(f) = unsafe { *p } {
            let r = f();
            if r != 0 {
                return r;
            }
        }
        p = unsafe { p.add(1) };
    }
    0
}

/// `_onexit_table_t`: first, last, end pointers of a heap array of
/// functions; registered last-in, run first-out.
#[repr(C)]
pub struct OnExitTable {
    first: *mut usize,
    last: *mut usize,
    end: *mut usize,
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _initialize_onexit_table(table: *mut OnExitTable) -> i32 {
    if table.is_null() {
        return -1;
    }
    let t = unsafe { &mut *table };
    if t.first.is_null() {
        t.last = core::ptr::null_mut();
        t.end = core::ptr::null_mut();
    }
    0
}
static TABLE_LOCK: AtomicBool = AtomicBool::new(false);
fn lock() -> impl Drop {
    struct G;
    impl Drop for G {
        fn drop(&mut self) {
            TABLE_LOCK.store(false, Ordering::Release);
        }
    }
    while TABLE_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    G
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _register_onexit_function(
    table: *mut OnExitTable,
    function: usize,
) -> i32 {
    if table.is_null() {
        return -1;
    }
    let _g = lock();
    let t = unsafe { &mut *table };
    let word = core::mem::size_of::<usize>();
    if t.first.is_null() || t.last == t.end {
        let used = if t.first.is_null() {
            0
        } else {
            (t.last as usize - t.first as usize) / word
        };
        let capacity = (used * 2).max(32);
        let grown = unsafe { heap::realloc(t.first as *mut u8, capacity * word) } as *mut usize;
        if grown.is_null() {
            return -1;
        }
        t.first = grown;
        t.last = unsafe { grown.add(used) };
        t.end = unsafe { grown.add(capacity) };
    }
    unsafe {
        *t.last = function;
        t.last = t.last.add(1);
    }
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _execute_onexit_table(table: *mut OnExitTable) -> i32 {
    if table.is_null() {
        return -1;
    }
    let (first, mut last) = {
        let _g = lock();
        let t = unsafe { &mut *table };
        let taken = (t.first, t.last);
        t.first = core::ptr::null_mut();
        t.last = core::ptr::null_mut();
        t.end = core::ptr::null_mut();
        taken
    };
    if first.is_null() {
        return 0;
    }
    while last > first {
        last = unsafe { last.sub(1) };
        let f = unsafe { *last };
        if f != 0 {
            let f: extern "C" fn() = unsafe { core::mem::transmute(f) };
            f();
        }
    }
    unsafe { heap::free(first as *mut u8) };
    0
}
/// The process-wide atexit table (`_crt_atexit`, `atexit`, `_onexit`).
static mut ATEXIT: OnExitTable = OnExitTable {
    first: core::ptr::null_mut(),
    last: core::ptr::null_mut(),
    end: core::ptr::null_mut(),
};
static mut AT_QUICK_EXIT: OnExitTable = OnExitTable {
    first: core::ptr::null_mut(),
    last: core::ptr::null_mut(),
    end: core::ptr::null_mut(),
};
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _crt_atexit(function: usize) -> i32 {
    unsafe { _register_onexit_function(core::ptr::addr_of_mut!(ATEXIT), function) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atexit(function: usize) -> i32 {
    unsafe { _crt_atexit(function) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _onexit(function: usize) -> usize {
    if unsafe { _crt_atexit(function) } == 0 {
        function
    } else {
        0
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _crt_at_quick_exit(function: usize) -> i32 {
    unsafe { _register_onexit_function(core::ptr::addr_of_mut!(AT_QUICK_EXIT), function) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn at_quick_exit(function: usize) -> i32 {
    unsafe { _crt_at_quick_exit(function) }
}
static THREAD_EXIT_CALLBACK: AtomicUsize = AtomicUsize::new(0);
/// The callback the executable's TLS destructors run through
/// (`__dyn_tls_dtor`); kept for completeness, called at exit.
#[unsafe(no_mangle)]
pub extern "C" fn _register_thread_local_exe_atexit_callback(callback: usize) {
    THREAD_EXIT_CALLBACK.store(callback, Ordering::Release);
}

static EXITING: AtomicBool = AtomicBool::new(false);
fn run_exit_handlers() {
    if EXITING.swap(true, Ordering::AcqRel) {
        return;
    }
    let callback = THREAD_EXIT_CALLBACK.load(Ordering::Acquire);
    if callback != 0 {
        // PIMAGE_TLS_CALLBACK(DllHandle, DLL_PROCESS_DETACH, reserved).
        let f: extern "system" fn(usize, u32, usize) = unsafe { core::mem::transmute(callback) };
        f(0, 0, 0);
    }
    unsafe { _execute_onexit_table(core::ptr::addr_of_mut!(ATEXIT)) };
    stdio::flush_all();
}
#[unsafe(no_mangle)]
pub extern "C" fn exit(code: i32) -> ! {
    run_exit_handlers();
    unsafe { ExitProcess(code as u32) }
}
#[unsafe(no_mangle)]
pub extern "C" fn quick_exit(code: i32) -> ! {
    unsafe { _execute_onexit_table(core::ptr::addr_of_mut!(AT_QUICK_EXIT)) };
    unsafe { ExitProcess(code as u32) }
}
#[unsafe(no_mangle)]
pub extern "C" fn _exit(code: i32) -> ! {
    unsafe { ExitProcess(code as u32) }
}
#[unsafe(no_mangle)]
pub extern "C" fn _Exit(code: i32) -> ! {
    unsafe { ExitProcess(code as u32) }
}
/// `_cexit`: the exit handlers without ending the process.
#[unsafe(no_mangle)]
pub extern "C" fn _cexit() {
    run_exit_handlers();
}
#[unsafe(no_mangle)]
pub extern "C" fn _c_exit() {}

static ABORT_BEHAVIOR: AtomicU32 = AtomicU32::new(1);
#[unsafe(no_mangle)]
pub extern "C" fn abort() -> ! {
    // SIGABRT's default: the message is not shown in a guest console;
    // the process ends with exit code 3, as the CRT's abort does.
    if unsafe { IsDebuggerPresent() } != 0 && ABORT_BEHAVIOR.load(Ordering::Relaxed) & 1 != 0 {
        unsafe { RaiseFailFastException(0, 0, 0) }
    }
    unsafe { ExitProcess(3) }
}
#[unsafe(no_mangle)]
pub extern "C" fn _set_abort_behavior(flags: u32, mask: u32) -> u32 {
    let old = ABORT_BEHAVIOR.load(Ordering::Relaxed);
    ABORT_BEHAVIOR.store((old & !mask) | (flags & mask), Ordering::Relaxed);
    old
}
static TERMINATE: AtomicUsize = AtomicUsize::new(0);
#[unsafe(no_mangle)]
pub extern "C" fn terminate() -> ! {
    let handler = TERMINATE.load(Ordering::Acquire);
    if handler != 0 {
        let f: extern "C" fn() = unsafe { core::mem::transmute(handler) };
        f();
    }
    abort()
}
#[unsafe(no_mangle)]
pub extern "C" fn set_terminate(handler: usize) -> usize {
    TERMINATE.swap(handler, Ordering::AcqRel)
}
#[unsafe(no_mangle)]
pub extern "C" fn _get_terminate() -> usize {
    TERMINATE.load(Ordering::Acquire)
}

static INVALID_PARAMETER: AtomicUsize = AtomicUsize::new(0);
static THREAD_INVALID_PARAMETER: AtomicUsize = AtomicUsize::new(0);
/// What the CRT does on an invalid parameter: the program's handler (it
/// returns: the function fails with EINVAL), or fail fast.
pub(crate) fn invalid_parameter_handler() {
    let handler = THREAD_INVALID_PARAMETER.load(Ordering::Acquire);
    let handler = if handler != 0 {
        handler
    } else {
        INVALID_PARAMETER.load(Ordering::Acquire)
    };
    if handler != 0 {
        let f: extern "C" fn(usize, usize, usize, u32, usize) =
            unsafe { core::mem::transmute(handler) };
        f(0, 0, 0, 0, 0);
    }
    // The default handler (release CRT) silently returns EINVAL unless the
    // program asked for Watson reports; the functions fail as documented.
}
#[unsafe(no_mangle)]
pub extern "C" fn _set_invalid_parameter_handler(handler: usize) -> usize {
    INVALID_PARAMETER.swap(handler, Ordering::AcqRel)
}
#[unsafe(no_mangle)]
pub extern "C" fn _get_invalid_parameter_handler() -> usize {
    INVALID_PARAMETER.load(Ordering::Acquire)
}
#[unsafe(no_mangle)]
pub extern "C" fn _set_thread_local_invalid_parameter_handler(handler: usize) -> usize {
    THREAD_INVALID_PARAMETER.swap(handler, Ordering::AcqRel)
}
#[unsafe(no_mangle)]
pub extern "C" fn _get_thread_local_invalid_parameter_handler() -> usize {
    THREAD_INVALID_PARAMETER.load(Ordering::Acquire)
}
#[unsafe(no_mangle)]
pub extern "C" fn _invalid_parameter_noinfo() {
    invalid_parameter_handler();
}
#[unsafe(no_mangle)]
pub extern "C" fn _invalid_parameter_noinfo_noreturn() -> ! {
    invalid_parameter_handler();
    _invoke_watson(0, 0, 0, 0, 0)
}
#[unsafe(no_mangle)]
pub extern "C" fn _invalid_parameter(_e: usize, _f: usize, _file: usize, _line: u32, _r: usize) {
    invalid_parameter_handler();
}
/// `_invoke_watson`: fail fast (STATUS_INVALID_CRUNTIME_PARAMETER).
#[unsafe(no_mangle)]
pub extern "C" fn _invoke_watson(_e: usize, _f: usize, _file: usize, _line: u32, _r: usize) -> ! {
    unsafe { ExitProcess(0xC000_0417) }
}

static APP_TYPE: AtomicI32 = AtomicI32::new(0);
#[unsafe(no_mangle)]
pub extern "C" fn _set_app_type(kind: i32) {
    APP_TYPE.store(kind, Ordering::Relaxed);
}
#[unsafe(no_mangle)]
pub extern "C" fn _query_app_type() -> i32 {
    APP_TYPE.load(Ordering::Relaxed)
}
#[unsafe(no_mangle)]
pub extern "C" fn __setusermatherr(_handler: usize) {}
#[unsafe(no_mangle)]
pub extern "C" fn _set_error_mode(mode: i32) -> i32 {
    static MODE: AtomicI32 = AtomicI32::new(0);
    if mode == 3 {
        MODE.load(Ordering::Relaxed)
    } else {
        MODE.swap(mode, Ordering::Relaxed)
    }
}
static FMODE: AtomicI32 = AtomicI32::new(0x4000); // _O_TEXT
static COMMODE: AtomicI32 = AtomicI32::new(0);
#[unsafe(no_mangle)]
pub extern "C" fn _set_fmode(mode: i32) -> i32 {
    if mode != 0x4000 && mode != 0x8000 && mode != 0x10000 && mode != 0x20000 && mode != 0x40000 {
        set_errno(errno::EINVAL);
        return errno::EINVAL;
    }
    FMODE.store(mode, Ordering::Relaxed);
    0
}
#[unsafe(no_mangle)]
pub extern "C" fn _get_fmode(out: *mut i32) -> i32 {
    if out.is_null() {
        return errno::EINVAL;
    }
    unsafe { *out = FMODE.load(Ordering::Relaxed) };
    0
}
#[unsafe(no_mangle)]
pub extern "C" fn __p__fmode() -> *mut i32 {
    FMODE.as_ptr()
}
#[unsafe(no_mangle)]
pub extern "C" fn __p__commode() -> *mut i32 {
    COMMODE.as_ptr()
}
pub(crate) fn default_binary() -> bool {
    FMODE.load(Ordering::Relaxed) == 0x8000
}

/// `_seh_filter_exe` / `_seh_filter_dll`: the CRT's signal mapping for
/// the start-up code's exception filter: continue the search (no signal
/// handlers are installed by default).
#[unsafe(no_mangle)]
pub extern "C" fn _seh_filter_exe(_code: u32, _pointers: usize) -> i32 {
    0 // EXCEPTION_CONTINUE_SEARCH
}
#[unsafe(no_mangle)]
pub extern "C" fn _seh_filter_dll(_code: u32, _pointers: usize) -> i32 {
    0
}
/// `signal`: handlers are recorded; SIGINT/SIGBREAK come from the console
/// control handler, the others from the CRT's own raise.
static SIGNALS: [AtomicUsize; 23] = [const { AtomicUsize::new(0) }; 23];
#[unsafe(no_mangle)]
pub extern "C" fn signal(number: i32, handler: usize) -> usize {
    match usize::try_from(number).ok().filter(|n| *n < SIGNALS.len()) {
        Some(n) => SIGNALS[n].swap(handler, Ordering::AcqRel),
        None => {
            set_errno(errno::EINVAL);
            usize::MAX // SIG_ERR
        }
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn raise(number: i32) -> i32 {
    let Some(n) = usize::try_from(number).ok().filter(|n| *n < SIGNALS.len()) else {
        return -1;
    };
    let handler = SIGNALS[n].load(Ordering::Acquire);
    match handler {
        0 => {
            if number == 22 {
                abort();
            }
            unsafe { ExitProcess(3) }
        }
        1 => 0, // SIG_IGN
        f => {
            SIGNALS[n].store(0, Ordering::Release);
            let f: extern "C" fn(i32) = unsafe { core::mem::transmute(f) };
            f(number);
            0
        }
    }
}

// The floating-point control word (x87 and SSE kept in step).
static CONTROL: AtomicU32 = AtomicU32::new(0x0009_001F);
#[unsafe(no_mangle)]
pub extern "C" fn _controlfp_s(current: *mut u32, new: u32, mask: u32) -> i32 {
    let valid = 0x000B_031F;
    if mask & !valid != 0 {
        if !current.is_null() {
            unsafe { *current = CONTROL.load(Ordering::Relaxed) };
        }
        set_errno(errno::EINVAL);
        return errno::EINVAL;
    }
    let old = CONTROL.load(Ordering::Relaxed);
    let next = (old & !mask) | (new & mask);
    CONTROL.store(next, Ordering::Relaxed);
    apply(next);
    if !current.is_null() {
        unsafe { *current = next };
    }
    0
}
#[unsafe(no_mangle)]
pub extern "C" fn _controlfp(new: u32, mask: u32) -> u32 {
    let mut out = 0;
    _controlfp_s(&mut out, new, mask & 0x000B_031F);
    out
}
#[unsafe(no_mangle)]
pub extern "C" fn _control87(new: u32, mask: u32) -> u32 {
    _controlfp(new, mask)
}
#[unsafe(no_mangle)]
pub extern "C" fn _clearfp() -> u32 {
    let mut csr: u32 = 0;
    unsafe {
        core::arch::asm!("stmxcsr [{}]", in(reg) &mut csr, options(nostack));
        let cleared = csr & !0x3F;
        core::arch::asm!("ldmxcsr [{}]", in(reg) &cleared, options(nostack));
    }
    status_word(csr)
}
#[unsafe(no_mangle)]
pub extern "C" fn _statusfp() -> u32 {
    let mut csr: u32 = 0;
    unsafe { core::arch::asm!("stmxcsr [{}]", in(reg) &mut csr, options(nostack)) };
    status_word(csr)
}
fn status_word(csr: u32) -> u32 {
    // MXCSR flags (IE DE ZE OE UE PE) to _SW_* (INVALID 0x10, DENORMAL
    // 0x80000, ZERODIVIDE 0x8, OVERFLOW 0x4, UNDERFLOW 0x2, INEXACT 0x1).
    let mut w = 0;
    for (bit, value) in [
        (0, 0x10),
        (1, 0x8_0000),
        (2, 0x8),
        (3, 0x4),
        (4, 0x2),
        (5, 0x1),
    ] {
        if csr & (1 << bit) != 0 {
            w |= value;
        }
    }
    w
}
#[unsafe(no_mangle)]
pub extern "C" fn _fpreset() {
    CONTROL.store(0x0009_001F, Ordering::Relaxed);
    apply(0x0009_001F);
}
/// The control word's masks and rounding into MXCSR (the x87 word of a
/// 64-bit process is unused by compiled code).
fn apply(control: u32) {
    let mut csr: u32 = 0;
    unsafe { core::arch::asm!("stmxcsr [{}]", in(reg) &mut csr, options(nostack)) };
    // _EM_INVALID 0x10, _EM_DENORMAL 0x80000, _EM_ZERODIVIDE 0x8,
    // _EM_OVERFLOW 0x4, _EM_UNDERFLOW 0x2, _EM_INEXACT 0x1 -> MXCSR masks.
    let mut masks = 0u32;
    for (value, bit) in [
        (0x10, 7),
        (0x8_0000, 8),
        (0x8, 9),
        (0x4, 10),
        (0x2, 11),
        (0x1, 12),
    ] {
        if control & value != 0 {
            masks |= 1 << bit;
        }
    }
    // Rounding: _RC_NEAR 0, _RC_DOWN 0x100, _RC_UP 0x200, _RC_CHOP 0x300.
    let rounding = match control & 0x300 {
        0x100 => 1,
        0x200 => 2,
        0x300 => 3,
        _ => 0,
    };
    // Denormals are zero (_DN_FLUSH 0x1000000).
    let daz = if control & 0x0100_0000 != 0 {
        1 << 6 | 1 << 15
    } else {
        0
    };
    let next =
        (csr & !(0x3F << 7) & !(3 << 13) & !(1 << 6) & !(1 << 15)) | masks | (rounding << 13) | daz;
    unsafe { core::arch::asm!("ldmxcsr [{}]", in(reg) &next, options(nostack)) };
}
#[unsafe(no_mangle)]
pub extern "C" fn __fpe_flt_rounds() -> i32 {
    match CONTROL.load(Ordering::Relaxed) & 0x300 {
        0x100 => 3,
        0x200 => 2,
        0x300 => 0,
        _ => 1,
    }
}
static mut FPECODE: i32 = 0;
#[unsafe(no_mangle)]
pub extern "C" fn __fpecode() -> *mut i32 {
    core::ptr::addr_of_mut!(FPECODE)
}
static mut PGM: usize = 0;
#[unsafe(no_mangle)]
pub extern "C" fn _get_pgmptr(out: *mut usize) -> i32 {
    unsafe { *out = PGM };
    0
}
#[unsafe(no_mangle)]
pub extern "C" fn _get_wpgmptr(out: *mut usize) -> i32 {
    unsafe { *out = PGM };
    0
}
