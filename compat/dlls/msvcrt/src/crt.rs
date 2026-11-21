//! The rest of the C runtime a DLL built against msvcrt.dll imports: the
//! module's onexit table (`__dllonexit`, `_onexit`), static initializers
//! (`_initterm`), the runtime's locks, `strcmp`, wide `_vsnwprintf`, the
//! fatal-error entries (`_purecall`, `_amsg_exit`), and (x64) the C++
//! pieces an MSVC DLL links from msvcrt: `operator delete`,
//! `std::exception`, `type_info`'s destructor, `_CxxThrowException` and
//! the `__CxxFrameHandler3` personality. `__C_specific_handler` is ntdll's
//! (a forwarder, `tools/xtask/src/windows/dlls.rs`).
//!
//! The frame handler finds no catch block (it searches on and runs no
//! unwind actions): a C++ exception leaving one of these DLLs reaches the
//! caller's handlers as the SEH exception 0xE06D7363, as an exception that
//! no frame catches does on Windows.
use core::sync::atomic::{AtomicBool, Ordering};

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn RtlExitUserProcess(code: u32) -> !;
    #[cfg(target_arch = "x86_64")]
    fn RtlRaiseException(record: *const u8);
    #[cfg(target_arch = "x86_64")]
    fn RtlPcToFileHeader(pc: usize, base: *mut usize) -> usize;
}

/// `_initterm(begin, end)`: each non-null initializer, in order.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _initterm(begin: *const usize, end: *const usize) {
    let mut at = begin;
    while at < end {
        let function = unsafe { at.read() };
        if function != 0 {
            // SAFETY: the module's initializer table.
            let f: extern "C" fn() = unsafe { core::mem::transmute(function) };
            f();
        }
        at = unsafe { at.add(1) };
    }
}

const EXIT_FUNCTIONS: usize = 64;
struct ExitTable {
    held: AtomicBool,
    functions: core::cell::UnsafeCell<([usize; EXIT_FUNCTIONS], usize)>,
}
// SAFETY: `held` gives one thread the table at a time.
unsafe impl Sync for ExitTable {}
static EXIT: ExitTable = ExitTable {
    held: AtomicBool::new(false),
    functions: core::cell::UnsafeCell::new(([0; EXIT_FUNCTIONS], 0)),
};
/// `_onexit(function)`: kept for the process's exit (the function, or
/// NULL when the table is full).
#[unsafe(no_mangle)]
pub extern "C" fn _onexit(function: usize) -> usize {
    while EXIT.held.swap(true, Ordering::Acquire) {
        core::hint::spin_loop();
    }
    // SAFETY: `held` gives this thread the table.
    let (table, count) = unsafe { &mut *EXIT.functions.get() };
    let stored = if *count < EXIT_FUNCTIONS {
        table[*count] = function;
        *count += 1;
        function
    } else {
        0
    };
    EXIT.held.store(false, Ordering::Release);
    stored
}
/// `__dllonexit(function, begin, end)`: append to a DLL's own table (a
/// heap array the DLL keeps between `*begin` and `*end`), growing it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __dllonexit(
    function: usize,
    begin: *mut *mut usize,
    end: *mut *mut usize,
) -> usize {
    if begin.is_null() || end.is_null() {
        return 0;
    }
    let (first, last) = unsafe { (*begin, *end) };
    let count = if first.is_null() {
        0
    } else {
        (last as usize - first as usize) / core::mem::size_of::<usize>()
    };
    let grown =
        unsafe { super::guest::realloc(first.cast(), (count + 1) * core::mem::size_of::<usize>()) }
            as *mut usize;
    if grown.is_null() {
        return 0;
    }
    unsafe {
        grown.add(count).write(function);
        *begin = grown;
        *end = grown.add(count + 1);
    }
    function
}

/// The runtime's numbered locks (`_lock`/`_unlock`).
static LOCKS: [AtomicBool; 64] = [const { AtomicBool::new(false) }; 64];
#[unsafe(no_mangle)]
pub extern "C" fn _lock(number: i32) {
    let Some(lock) = LOCKS.get(number as usize) else {
        return;
    };
    while lock.swap(true, Ordering::Acquire) {
        core::hint::spin_loop();
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn _unlock(number: i32) {
    if let Some(lock) = LOCKS.get(number as usize) {
        lock.store(false, Ordering::Release);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcmp(left: *const u8, right: *const u8) -> i32 {
    let mut index = 0;
    loop {
        let (a, b) = unsafe { (left.add(index).read(), right.add(index).read()) };
        if a != b || a == 0 {
            return i32::from(a) - i32::from(b);
        }
        index += 1;
    }
}
/// `_callnewh(bytes)`: no new handler is installed.
#[unsafe(no_mangle)]
pub extern "C" fn _callnewh(_bytes: usize) -> i32 {
    0
}
/// `_purecall`: a pure virtual function was called (R6025): the process
/// ends, as msvcrt ends it.
#[unsafe(no_mangle)]
pub extern "C" fn _purecall() -> i32 {
    unsafe { RtlExitUserProcess(255) }
}
/// `_amsg_exit(error)`: a fatal runtime error ends the process.
#[unsafe(no_mangle)]
pub extern "C" fn _amsg_exit(_error: i32) -> ! {
    unsafe { RtlExitUserProcess(255) }
}
/// `_XcptFilter(code, pointers)`: no signal handlers are installed, so the
/// exception searches on (EXCEPTION_CONTINUE_SEARCH).
#[unsafe(no_mangle)]
pub extern "C" fn _XcptFilter(_code: u32, _pointers: usize) -> i32 {
    0
}

// ---- wide formatting ------------------------------------------------------------------

/// The `va_list` of the x64 calling convention: 8-byte slots.
#[cfg(target_arch = "x86_64")]
struct Arguments(*const u64);
#[cfg(target_arch = "x86_64")]
impl Arguments {
    fn next(&mut self) -> u64 {
        let value = unsafe { self.0.read() };
        self.0 = unsafe { self.0.add(1) };
        value
    }
}
#[cfg(target_arch = "x86_64")]
struct Out {
    buffer: *mut u16,
    capacity: usize,
    len: usize,
}
#[cfg(target_arch = "x86_64")]
impl Out {
    fn push(&mut self, unit: u16) {
        if self.len < self.capacity {
            unsafe { self.buffer.add(self.len).write(unit) };
        }
        self.len += 1;
    }
    fn pad(&mut self, count: usize, unit: u16) {
        for _ in 0..count {
            self.push(unit);
        }
    }
}
/// `_vsnwprintf(buffer, count, format, args)`: the conversions msvcrt
/// formats (`%s %ls %S %hs %c %C %d %i %u %x %X %o %p %%`, with flags
/// `-0`, width, precision and the `l`/`ll`/`I64`/`h`/`z` sizes). Returns
/// the units written, or -1 when they do not fit (the buffer then holds
/// the first `count`, unterminated, as msvcrt's).
#[cfg(target_arch = "x86_64")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _vsnwprintf(
    buffer: *mut u16,
    count: usize,
    format: *const u16,
    args: *const u64,
) -> i32 {
    let mut out = Out {
        buffer,
        capacity: count,
        len: 0,
    };
    let mut args = Arguments(args);
    let mut at = 0;
    let unit = |i: usize| unsafe { format.add(i).read() };
    loop {
        let c = unit(at);
        at += 1;
        if c == 0 {
            break;
        }
        if c != u16::from(b'%') {
            out.push(c);
            continue;
        }
        let (mut left, mut zero) = (false, false);
        loop {
            match unit(at) {
                0x2d => left = true,
                0x30 => zero = true,
                0x2b | 0x20 | 0x23 => {}
                _ => break,
            }
            at += 1;
        }
        let mut width = 0usize;
        if unit(at) == u16::from(b'*') {
            width = args.next() as i32 as usize;
            at += 1;
        }
        while (0x30..=0x39).contains(&unit(at)) {
            width = width * 10 + usize::from(unit(at) - 0x30);
            at += 1;
        }
        let mut precision = None;
        if unit(at) == u16::from(b'.') {
            at += 1;
            let mut p = 0usize;
            if unit(at) == u16::from(b'*') {
                p = args.next() as i32 as usize;
                at += 1;
            }
            while (0x30..=0x39).contains(&unit(at)) {
                p = p * 10 + usize::from(unit(at) - 0x30);
                at += 1;
            }
            precision = Some(p);
        }
        // Sizes: h (narrow), l/w (wide), ll/I64/z/I (64-bit).
        let (mut narrow, mut wide, mut long) = (false, false, false);
        loop {
            match unit(at) {
                0x68 => narrow = true,
                0x6c | 0x77 => {
                    if wide || unit(at + 1) == 0x6c {
                        long = true;
                    }
                    wide = true;
                }
                0x7a | 0x6a | 0x74 => long = true,
                0x49 => {
                    if unit(at + 1) == 0x36 && unit(at + 2) == 0x34 {
                        at += 2;
                    }
                    long = true;
                }
                _ => break,
            }
            at += 1;
        }
        let conversion = unit(at);
        at += 1;
        let mut digits = [0u16; 32];
        let mut wide_text: *const u16 = core::ptr::null();
        let mut narrow_text: *const u8 = core::ptr::null();
        let mut negative = false;
        match conversion as u8 {
            b'%' => {
                out.push(u16::from(b'%'));
                continue;
            }
            b'c' | b'C' => {
                let ch = args.next() as u16;
                let ch = if conversion == u16::from(b'C') || narrow {
                    ch & 0xff
                } else {
                    ch
                };
                if !left && width > 1 {
                    out.pad(width - 1, 0x20);
                }
                out.push(ch);
                if left && width > 1 {
                    out.pad(width - 1, 0x20);
                }
                continue;
            }
            b's' | b'S' => {
                let pointer = args.next();
                // %s is wide in the wide functions; %S and %hs narrow.
                let is_narrow = narrow || (conversion == u16::from(b'S') && !wide);
                if pointer == 0 {
                    narrow_text = c"(null)".as_ptr().cast();
                } else if is_narrow {
                    narrow_text = pointer as *const u8;
                } else {
                    wide_text = pointer as *const u16;
                }
            }
            b'd' | b'i' | b'u' | b'x' | b'X' | b'o' | b'p' => {
                let raw = args.next();
                let value = match conversion as u8 {
                    b'p' => raw,
                    b'd' | b'i' => {
                        let signed = if long {
                            raw as i64
                        } else if narrow {
                            i64::from(raw as i16)
                        } else {
                            i64::from(raw as i32)
                        };
                        negative = signed < 0;
                        signed.unsigned_abs()
                    }
                    _ if long => raw,
                    _ if narrow => u64::from(raw as u16),
                    _ => u64::from(raw as u32),
                };
                let (radix, upper) = match conversion as u8 {
                    b'x' => (16, false),
                    b'X' | b'p' => (16, true),
                    b'o' => (8, false),
                    _ => (10, false),
                };
                let mut n = digits.len();
                let mut v = value;
                let minimum = if conversion == u16::from(b'p') {
                    16
                } else {
                    precision.unwrap_or(1)
                };
                while v != 0 || digits.len() - n < minimum {
                    let d = (v % radix) as u8;
                    n -= 1;
                    digits[n] = u16::from(match d {
                        0..=9 => b'0' + d,
                        _ if upper => b'A' + d - 10,
                        _ => b'a' + d - 10,
                    });
                    v /= radix;
                    if n == 0 {
                        break;
                    }
                }
                let text = &digits[n..];
                let length = text.len() + usize::from(negative);
                if !left && width > length {
                    if zero {
                        if negative {
                            out.push(u16::from(b'-'));
                        }
                        out.pad(width - length, 0x30);
                    } else {
                        out.pad(width - length, 0x20);
                        if negative {
                            out.push(u16::from(b'-'));
                        }
                    }
                } else if negative {
                    out.push(u16::from(b'-'));
                }
                for &u in text {
                    out.push(u);
                }
                if left && width > length {
                    out.pad(width - length, 0x20);
                }
                continue;
            }
            _ => {
                // An unknown conversion is written as it is.
                out.push(u16::from(b'%'));
                out.push(conversion);
                continue;
            }
        }
        // A string: its length (bounded by the precision), then padding.
        let limit = precision.unwrap_or(usize::MAX);
        let mut length = 0;
        let read = |i: usize| -> u16 {
            if !wide_text.is_null() {
                unsafe { wide_text.add(i).read() }
            } else {
                u16::from(unsafe { narrow_text.add(i).read() })
            }
        };
        while length < limit && read(length) != 0 {
            length += 1;
        }
        if !left && width > length {
            out.pad(width - length, 0x20);
        }
        for i in 0..length {
            out.push(read(i));
        }
        if left && width > length {
            out.pad(width - length, 0x20);
        }
    }
    if out.len < count {
        unsafe { buffer.add(out.len).write(0) };
        out.len as i32
    } else if out.len == count {
        out.len as i32
    } else {
        -1
    }
}

// ---- C++ (x64): operator delete, std::exception, type_info, throw ----------------------

/// `operator delete(void*)`.
#[cfg(target_arch = "x86_64")]
#[unsafe(export_name = "??3@YAXPEAX@Z")]
pub unsafe extern "C" fn operator_delete(pointer: *mut u8) {
    unsafe { super::guest::free(pointer) }
}
/// msvcrt's `std::exception`: the vtable, the message and whether the
/// object owns a copy of it.
#[cfg(target_arch = "x86_64")]
#[repr(C)]
pub struct Exception {
    vtable: *const usize,
    what: *const u8,
    owns: i32,
}
/// `std::exception`'s vtable: the scalar deleting destructor, `what`.
#[cfg(target_arch = "x86_64")]
#[repr(C)]
struct VTable {
    destroy: unsafe extern "C" fn(*mut Exception, u32) -> *mut Exception,
    what: unsafe extern "C" fn(*const Exception) -> *const u8,
}
#[cfg(target_arch = "x86_64")]
static EXCEPTION_VTABLE: VTable = VTable {
    destroy: exception_deleting_destructor,
    what: exception_what,
};
#[cfg(target_arch = "x86_64")]
fn vtable() -> *const usize {
    &EXCEPTION_VTABLE as *const VTable as *const usize
}
#[cfg(target_arch = "x86_64")]
fn copy_message(message: *const u8) -> (*const u8, i32) {
    if message.is_null() {
        return (core::ptr::null(), 0);
    }
    let length = unsafe { super::guest::strlen(message) } + 1;
    let copy = super::guest::malloc(length);
    if copy.is_null() {
        return (core::ptr::null(), 0);
    }
    unsafe { super::guest::memcpy(copy, message, length) };
    (copy, 1)
}
/// `exception::exception(char const * const &)`: owns a copy.
#[cfg(target_arch = "x86_64")]
#[unsafe(export_name = "??0exception@@QEAA@AEBQEBD@Z")]
pub unsafe extern "C" fn exception_from_message(
    this: *mut Exception,
    message: *const *const u8,
) -> *mut Exception {
    let (what, owns) = copy_message(unsafe { *message });
    unsafe {
        this.write(Exception {
            vtable: vtable(),
            what,
            owns,
        })
    };
    this
}
/// `exception::exception(char const * const &, int)`: refers to the
/// (static) message.
#[cfg(target_arch = "x86_64")]
#[unsafe(export_name = "??0exception@@QEAA@AEBQEBDH@Z")]
pub unsafe extern "C" fn exception_from_static(
    this: *mut Exception,
    message: *const *const u8,
    _unused: i32,
) -> *mut Exception {
    unsafe {
        this.write(Exception {
            vtable: vtable(),
            what: *message,
            owns: 0,
        })
    };
    this
}
/// `exception::exception(exception const &)`.
#[cfg(target_arch = "x86_64")]
#[unsafe(export_name = "??0exception@@QEAA@AEBV0@@Z")]
pub unsafe extern "C" fn exception_copy(
    this: *mut Exception,
    other: *const Exception,
) -> *mut Exception {
    let other = unsafe { &*other };
    let (what, owns) = if other.owns != 0 {
        copy_message(other.what)
    } else {
        (other.what, 0)
    };
    unsafe {
        this.write(Exception {
            vtable: vtable(),
            what,
            owns,
        })
    };
    this
}
/// `exception::~exception()`.
#[cfg(target_arch = "x86_64")]
#[unsafe(export_name = "??1exception@@UEAA@XZ")]
pub unsafe extern "C" fn exception_destructor(this: *mut Exception) {
    let e = unsafe { &mut *this };
    e.vtable = vtable();
    if e.owns != 0 && !e.what.is_null() {
        unsafe { super::guest::free(e.what as *mut u8) };
    }
    e.what = core::ptr::null();
    e.owns = 0;
}
/// `exception::what() const`: the message, or "Unknown exception".
#[cfg(target_arch = "x86_64")]
#[unsafe(export_name = "?what@exception@@UEBAPEBDXZ")]
pub unsafe extern "C" fn exception_what(this: *const Exception) -> *const u8 {
    let what = unsafe { (*this).what };
    if what.is_null() {
        c"Unknown exception".as_ptr().cast()
    } else {
        what
    }
}
/// The vtable's scalar deleting destructor: destroy, and free when bit 0.
#[cfg(target_arch = "x86_64")]
unsafe extern "C" fn exception_deleting_destructor(
    this: *mut Exception,
    flags: u32,
) -> *mut Exception {
    unsafe { exception_destructor(this) };
    if flags & 1 != 0 {
        unsafe { super::guest::free(this.cast()) };
    }
    this
}
/// `type_info::~type_info()`: nothing is cached for a type's name.
#[cfg(target_arch = "x86_64")]
#[unsafe(export_name = "??1type_info@@UEAA@XZ")]
pub extern "C" fn type_info_destructor(_this: *mut u8) {}

/// `_CxxThrowException(object, throwInfo)`: raise the C++ exception
/// (0xE06D7363, noncontinuable) with the object, its ThrowInfo and the
/// throwing module's base, as MSVC's runtime raises it.
#[cfg(target_arch = "x86_64")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn _CxxThrowException(object: usize, throw_info: usize) -> ! {
    let mut base = 0usize;
    unsafe { RtlPcToFileHeader(throw_info, &mut base) };
    let mut record = [0u8; 152];
    record[..4].copy_from_slice(&0xE06D_7363u32.to_le_bytes());
    record[4..8].copy_from_slice(&1u32.to_le_bytes());
    record[16..24].copy_from_slice(&(_CxxThrowException as *const () as u64).to_le_bytes());
    record[24..28].copy_from_slice(&4u32.to_le_bytes());
    for (i, value) in [
        0x1993_0520u64,
        object as u64,
        throw_info as u64,
        base as u64,
    ]
    .into_iter()
    .enumerate()
    {
        record[32 + 8 * i..40 + 8 * i].copy_from_slice(&value.to_le_bytes());
    }
    unsafe { RtlRaiseException(record.as_ptr()) };
    unsafe { RtlExitUserProcess(0xE06D_7363) }
}
/// `__CxxFrameHandler3`: no catch block is matched and no unwind action
/// runs (see the module notes): ExceptionContinueSearch.
#[cfg(target_arch = "x86_64")]
#[unsafe(no_mangle)]
pub extern "system" fn __CxxFrameHandler3(
    _record: usize,
    _frame: usize,
    _context: usize,
    _dispatcher: usize,
) -> u32 {
    1
}
