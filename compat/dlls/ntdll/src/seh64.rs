//! x64 structured exception handling (P18 M0c), as ntdll provides it on
//! Windows: function-table lookup over the loaded images (their .pdata) and
//! the dynamic tables JIT compilers register (RtlAddFunctionTable,
//! RtlInstallFunctionTableCallback, growable tables), RtlVirtualUnwind with
//! every unwind form (`door_pe_loader::x64_unwind`), RtlCaptureContext and
//! RtlRestoreContext (with longjmp and unwind consolidation, which runs
//! C++ catch funclets), frame-based dispatch with language handlers after
//! the vectored ones, RtlUnwindEx, RtlRaiseException and the unhandled-
//! exception filter.
//!
//! Faults enter at `DoorExceptionDispatch`, an assembly stub whose unwind
//! data is a machine frame holding the faulting RIP and RSP, so an unwind
//! that crosses the dispatcher continues in the faulting code exactly as
//! through KiUserExceptionDispatcher on Windows.
use super::guest::RtlSetLastWin32Error;
use core::{
    arch::naked_asm,
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use door_pe_loader::x64_unwind::{
    self as engine, Context, Error, Function, Memory, Pointers, UNW_FLAG_EHANDLER,
    UNW_FLAG_NHANDLER, UNW_FLAG_UHANDLER,
};

// ---- CONTEXT, EXCEPTION_RECORD, DISPATCHER_CONTEXT ------------------------

const CONTEXT_BYTES: usize = 1232;
const C_FLAGS: usize = 48;
const C_RAX: usize = 120;
const C_RSP: usize = 152;
const C_RIP: usize = 248;
const C_XMM: usize = 416;
/// CONTEXT_AMD64 | CONTROL | INTEGER | SEGMENTS | FLOATING_POINT.
const CONTEXT_CAPTURED: u32 = 0x0010_000f;

const R_CODE: usize = 0;
const R_FLAGS: usize = 4;
const R_ADDRESS: usize = 16;
const R_COUNT: usize = 24;
const R_INFO: usize = 32;
const RECORD_BYTES: usize = 152;

const NONCONTINUABLE: u32 = 1;
const UNWINDING: u32 = 2;
const EXIT_UNWIND: u32 = 4;
const STACK_INVALID: u32 = 8;
const TARGET_UNWIND: u32 = 0x20;
const COLLIDED_UNWIND: u32 = 0x40;

const STATUS_UNWIND: u32 = 0xc000_0027;
const STATUS_NONCONTINUABLE_EXCEPTION: u32 = 0xc000_0025;
const STATUS_INVALID_DISPOSITION: u32 = 0xc000_0026;
const STATUS_INVALID_UNWIND_TARGET: u32 = 0xc000_0029;
const STATUS_BAD_FUNCTION_TABLE: u32 = 0xc000_00ff;
const STATUS_UNWIND_CONSOLIDATE: u32 = 0x8000_0029;
const STATUS_LONGJUMP: u32 = 0x8000_0026;

const CONTINUE_EXECUTION: u32 = 0;
const CONTINUE_SEARCH: u32 = 1;
const NESTED_EXCEPTION: u32 = 2;
const COLLIDED: u32 = 3;

/// A 16-byte aligned CONTEXT (FXSAVE/FXRSTOR need the alignment).
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct Ctx([u8; CONTEXT_BYTES]);
impl Ctx {
    const fn zero() -> Self {
        Self([0; CONTEXT_BYTES])
    }
    fn ptr(&mut self) -> u64 {
        self.0.as_mut_ptr() as u64
    }
    fn get(&self, at: usize) -> u64 {
        u64::from_le_bytes(self.0[at..at + 8].try_into().expect("context field"))
    }
    fn set(&mut self, at: usize, value: u64) {
        self.0[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn rip(&self) -> u64 {
        self.get(C_RIP)
    }
    fn rsp(&self) -> u64 {
        self.get(C_RSP)
    }
    fn load(address: u64) -> Self {
        let mut c = Self::zero();
        // SAFETY: callers pass a live CONTEXT.
        unsafe {
            core::ptr::copy_nonoverlapping(address as *const u8, c.0.as_mut_ptr(), CONTEXT_BYTES)
        };
        c
    }
    fn store(&self, address: u64) {
        unsafe {
            core::ptr::copy_nonoverlapping(self.0.as_ptr(), address as *mut u8, CONTEXT_BYTES)
        };
    }
    fn engine(&self) -> Context {
        Context {
            registers: core::array::from_fn(|i| self.get(C_RAX + i * 8)),
            rip: self.rip(),
            xmm: core::array::from_fn(|i| {
                self.0[C_XMM + i * 16..C_XMM + i * 16 + 16]
                    .try_into()
                    .expect("context xmm")
            }),
        }
    }
    fn apply(&mut self, c: &Context) {
        for (i, value) in c.registers.iter().enumerate() {
            self.set(C_RAX + i * 8, *value);
        }
        self.set(C_RIP, c.rip);
        for (i, value) in c.xmm.iter().enumerate() {
            self.0[C_XMM + i * 16..C_XMM + i * 16 + 16].copy_from_slice(value);
        }
    }
}
fn read_u32(address: u64) -> u32 {
    unsafe { (address as *const u32).read_unaligned() }
}
fn read_u64(address: u64) -> u64 {
    unsafe { (address as *const u64).read_unaligned() }
}
fn write_u32(address: u64, value: u32) {
    unsafe { (address as *mut u32).write_unaligned(value) }
}
fn write_u64(address: u64, value: u64) {
    unsafe { (address as *mut u64).write_unaligned(value) }
}

/// The thread's stack (StackLimit, StackBase) from the TEB.
fn stack_bounds() -> (u64, u64) {
    let (base, limit): (u64, u64);
    unsafe {
        core::arch::asm!("mov {}, gs:[8]", out(reg) base, options(nostack, readonly, preserves_flags));
        core::arch::asm!("mov {}, gs:[16]", out(reg) limit, options(nostack, readonly, preserves_flags));
    }
    (limit, base)
}

// ---- memory the unwinder reads -----------------------------------------------

/// Reads for the unwinder: the thread's stack and mapped images directly
/// (as Windows' unwinder does); anything else through the kernel's checked
/// copy, so a corrupt frame chain cannot fault the dispatcher.
struct Reader;
impl Memory for Reader {
    fn read(&mut self, address: u64, output: &mut [u8]) -> Result<(), Error> {
        let end = address
            .checked_add(output.len() as u64)
            .ok_or(Error::Bounds)?;
        if address < 0x10000 || end > 0x8000_0000_0000 {
            return Err(Error::Bounds);
        }
        let (limit, base) = stack_bounds();
        let direct = (address >= limit && end <= base) || in_known_code(address, end);
        if direct {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    address as *const u8,
                    output.as_mut_ptr(),
                    output.len(),
                )
            };
            Ok(())
        } else {
            unsafe { door_sys::user_copy(output.as_mut_ptr() as u64, address, output.len()) }
                .map_err(|_| Error::Memory)
        }
    }
}
fn in_known_code(address: u64, end: u64) -> bool {
    images().any(|i| address >= i.base && end <= i.base + i.size)
        || with_dynamic(|tables| {
            tables
                .iter()
                .flatten()
                .any(|t| address >= t.base && end <= t.base + t.span)
        })
}

// ---- images and their .pdata ------------------------------------------------------

#[derive(Clone, Copy)]
struct Image {
    base: u64,
    size: u64,
    pdata: u64,
    count: u32,
}
/// Parsed images by base (append-only; a slot empties when its DLL
/// unloads, `forget`). Unwinding reads through `Reader`, whose code check
/// lists the images, so parsing must neither recurse into it nor repeat
/// for every read.
struct Cached {
    base: AtomicU64,
    size: AtomicU64,
    pdata: AtomicU64,
    count: AtomicU64,
}
static CACHE: [Cached; 64] = [const {
    Cached {
        base: AtomicU64::new(0),
        size: AtomicU64::new(0),
        pdata: AtomicU64::new(0),
        count: AtomicU64::new(0),
    }
}; 64];
fn parse(base: u64) -> Option<Image> {
    if let Some(c) = CACHE
        .iter()
        .find(|c| c.base.load(Ordering::Acquire) == base)
    {
        return Some(Image {
            base,
            size: c.size.load(Ordering::Relaxed),
            pdata: c.pdata.load(Ordering::Relaxed),
            count: c.count.load(Ordering::Relaxed) as u32,
        });
    }
    let image = parse_headers(base)?;
    // A slot whose base is published last: readers see whole records.
    for c in &CACHE {
        if c.base.load(Ordering::Acquire) == 0 && c.size.swap(u64::MAX, Ordering::AcqRel) == 0 {
            c.pdata.store(image.pdata, Ordering::Relaxed);
            c.count.store(u64::from(image.count), Ordering::Relaxed);
            c.size.store(image.size, Ordering::Relaxed);
            c.base.store(base, Ordering::Release);
            break;
        }
    }
    Some(image)
}
/// A DLL at `base` unloaded: its cached headers go (ntdll's loader).
pub(super) fn forget(base: u64) {
    for c in &CACHE {
        if c.base.load(Ordering::Acquire) == base {
            c.base.store(0, Ordering::Release);
            c.size.store(0, Ordering::Release);
        }
    }
}
/// Bytes of the image at `address` through the kernel's checked copy.
fn raw(address: u64, output: &mut [u8]) -> Option<()> {
    unsafe { door_sys::user_copy(output.as_mut_ptr() as u64, address, output.len()) }.ok()
}
fn parse_headers(base: u64) -> Option<Image> {
    let mut dos = [0u8; 64];
    raw(base, &mut dos)?;
    if &dos[..2] != b"MZ" {
        return None;
    }
    let nt = u64::from(u32::from_le_bytes(dos[60..64].try_into().ok()?));
    let mut pe = [0u8; 168];
    raw(base.checked_add(nt)?, &mut pe)?;
    if &pe[..4] != b"PE\0\0" || pe[4..6] != [0x64, 0x86] || pe[24..26] != [0x0b, 2] {
        return None;
    }
    let u32_at = |at: usize| u32::from_le_bytes(pe[at..at + 4].try_into().expect("pe field"));
    let size = u64::from(u32_at(80));
    let (table, bytes) = if u32_at(132) >= 4 {
        (u32_at(160), u32_at(164))
    } else {
        (0, 0)
    };
    if size == 0 || u64::from(table) + u64::from(bytes) > size {
        return None;
    }
    Some(Image {
        base,
        size,
        pdata: base + u64::from(table),
        count: bytes / 12,
    })
}
/// Every mapped image (executable first, then DLLs in load order).
fn images() -> impl Iterator<Item = Image> {
    let fixed = super::modules::inventory()
        .map(|t| &t.images[..t.count])
        .unwrap_or(&[]);
    fixed
        .iter()
        .chain(super::modules::loaded())
        .map(|m| m.base)
        .filter(|base| *base != 0x1_8400_0000 || super::loader::dynamic_present())
        .filter_map(parse)
}
fn function_at(entry: u64) -> Function {
    Function {
        begin: read_u32(entry),
        end: read_u32(entry + 4),
        unwind: read_u32(entry + 8),
    }
}
/// The RUNTIME_FUNCTION of `pc` in a sorted .pdata: binary search.
fn search(base: u64, table: u64, count: u32, pc: u64) -> Option<u64> {
    let rva = u32::try_from(pc.checked_sub(base)?).ok()?;
    let (mut low, mut high) = (0u32, count);
    while low < high {
        let middle = low + (high - low) / 2;
        let entry = table + u64::from(middle) * 12;
        let f = function_at(entry);
        if rva < f.begin {
            high = middle;
        } else if rva >= f.end {
            low = middle + 1;
        } else {
            return Some(entry);
        }
    }
    None
}

// ---- dynamic function tables --------------------------------------------------------

#[derive(Clone, Copy)]
enum Source {
    /// RtlAddFunctionTable / growable: an array of RUNTIME_FUNCTIONs.
    Table {
        entries: u64,
        count: u32,
        sorted: bool,
    },
    /// RtlInstallFunctionTableCallback.
    Callback { function: u64, context: u64 },
}
#[derive(Clone, Copy)]
struct Dynamic {
    /// The handle callers pass to delete it (the table pointer, the
    /// callback's table identifier, or the growable handle).
    id: u64,
    base: u64,
    /// Range covered: [base + low, base + span).
    low: u64,
    span: u64,
    source: Source,
}
const MAX_DYNAMIC: usize = 512;
struct Registry {
    locked: AtomicBool,
    tables: UnsafeCell<[Option<Dynamic>; MAX_DYNAMIC]>,
}
unsafe impl Sync for Registry {}
static DYNAMIC: Registry = Registry {
    locked: AtomicBool::new(false),
    tables: UnsafeCell::new([None; MAX_DYNAMIC]),
};
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(0x1000);
fn with_dynamic<R>(f: impl FnOnce(&mut [Option<Dynamic>; MAX_DYNAMIC]) -> R) -> R {
    while DYNAMIC
        .locked
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
        let _ = door_sys::yield_now();
    }
    let result = f(unsafe { &mut *DYNAMIC.tables.get() });
    DYNAMIC.locked.store(false, Ordering::Release);
    result
}
fn register(table: Dynamic) -> bool {
    with_dynamic(|tables| match tables.iter_mut().find(|t| t.is_none()) {
        Some(slot) => {
            *slot = Some(table);
            true
        }
        None => false,
    })
}
fn unregister(id: u64) -> bool {
    with_dynamic(
        |tables| match tables.iter_mut().find(|t| t.is_some_and(|t| t.id == id)) {
            Some(slot) => {
                *slot = None;
                true
            }
            None => false,
        },
    )
}
/// The dynamic entry covering `pc`: (base, RUNTIME_FUNCTION address).
fn dynamic_lookup(pc: u64) -> Option<(u64, u64)> {
    let candidate = with_dynamic(|tables| {
        tables
            .iter()
            .flatten()
            .find(|t| pc >= t.base + t.low && pc < t.base + t.span)
            .copied()
    })?;
    match candidate.source {
        Source::Table {
            entries,
            count,
            sorted,
        } => {
            if sorted {
                return search(candidate.base, entries, count, pc).map(|e| (candidate.base, e));
            }
            let rva = pc - candidate.base;
            (0..count)
                .map(|i| entries + u64::from(i) * 12)
                .find(|e| {
                    let f = function_at(*e);
                    rva >= u64::from(f.begin) && rva < u64::from(f.end)
                })
                .map(|e| (candidate.base, e))
        }
        Source::Callback { function, context } => {
            // PGET_RUNTIME_FUNCTION_CALLBACK(ControlPc, Context).
            let callback: extern "system" fn(u64, u64) -> u64 =
                unsafe { core::mem::transmute(function as usize) };
            let entry = callback(pc, context);
            (entry != 0).then_some((candidate.base, entry))
        }
    }
}

/// RtlAddFunctionTable: entries relative to `base`, in any order.
#[unsafe(no_mangle)]
pub extern "system" fn RtlAddFunctionTable(table: u64, count: u32, base: u64) -> u8 {
    if table == 0 || count == 0 {
        return 0;
    }
    let (mut low, mut high) = (u64::MAX, 0u64);
    for i in 0..count {
        let f = function_at(table + u64::from(i) * 12);
        low = low.min(u64::from(f.begin));
        high = high.max(u64::from(f.end));
    }
    u8::from(register(Dynamic {
        id: table,
        base,
        low,
        span: high,
        source: Source::Table {
            entries: table,
            count,
            sorted: false,
        },
    }))
}
/// RtlInstallFunctionTableCallback: `identifier` (its low two bits set)
/// covers [base, base + length).
#[unsafe(no_mangle)]
pub extern "system" fn RtlInstallFunctionTableCallback(
    identifier: u64,
    base: u64,
    length: u32,
    callback: u64,
    context: u64,
    _out_of_process_dll: u64,
) -> u8 {
    if identifier & 3 != 3 || callback == 0 || length == 0 {
        return 0;
    }
    u8::from(register(Dynamic {
        id: identifier,
        base,
        low: 0,
        span: u64::from(length),
        source: Source::Callback {
            function: callback,
            context,
        },
    }))
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlDeleteFunctionTable(table: u64) -> u8 {
    u8::from(unregister(table))
}
/// RtlAddGrowableFunctionTable: a sorted table that may grow up to
/// `maximum` entries over [range_base, range_end). The handle goes to
/// `handle`; STATUS_SUCCESS or STATUS_NO_MEMORY.
#[unsafe(no_mangle)]
pub extern "system" fn RtlAddGrowableFunctionTable(
    handle: u64,
    table: u64,
    count: u32,
    maximum: u32,
    range_base: u64,
    range_end: u64,
) -> u32 {
    if handle == 0 || table == 0 || count > maximum || range_end <= range_base {
        return 0xc000_000d;
    }
    let id = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    if !register(Dynamic {
        id,
        base: range_base,
        low: 0,
        span: range_end - range_base,
        source: Source::Table {
            entries: table,
            count,
            sorted: true,
        },
    }) {
        return 0xc000_0017;
    }
    write_u64(handle, id);
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlGrowFunctionTable(handle: u64, count: u32) {
    with_dynamic(|tables| {
        if let Some(Some(Dynamic {
            source: Source::Table { count: current, .. },
            ..
        })) = tables
            .iter_mut()
            .find(|t| t.is_some_and(|t| t.id == handle))
        {
            *current = count;
        }
    });
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlDeleteGrowableFunctionTable(handle: u64) {
    unregister(handle);
}

// ---- lookup -----------------------------------------------------------------------

/// The function entry of `pc`: (image base, RUNTIME_FUNCTION address).
/// Dispatcher stubs first, then images, then dynamic tables.
fn lookup(pc: u64) -> Option<(u64, u64)> {
    if let Some(found) = builtin(pc) {
        return Some(found);
    }
    for image in images() {
        if pc >= image.base && pc < image.base + image.size {
            if let Some(entry) = search(image.base, image.pdata, image.count, pc) {
                return Some((image.base, entry));
            }
            break;
        }
    }
    dynamic_lookup(pc)
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlLookupFunctionEntry(pc: u64, base: u64, _history: u64) -> u64 {
    match lookup(pc) {
        Some((image, entry)) => {
            if base != 0 {
                write_u64(base, image);
            }
            entry
        }
        None => 0,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorLookupFunctionEntry(pc: u64, base: u64, history: u64) -> u64 {
    RtlLookupFunctionEntry(pc, base, history)
}
/// RtlPcToFileHeader: the base of the image holding `pc` into `base`.
#[unsafe(no_mangle)]
pub extern "system" fn RtlPcToFileHeader(pc: u64, base: u64) -> u64 {
    let found = images()
        .find(|i| pc >= i.base && pc < i.base + i.size)
        .map_or(0, |i| i.base);
    write_u64(base, found);
    found
}

// ---- RtlVirtualUnwind ---------------------------------------------------------------

fn bad_table() -> ! {
    // A corrupt function table must not pass for a successful unwind.
    door_sys::exit(i64::from(STATUS_BAD_FUNCTION_TABLE))
}
/// One frame through the engine: Ok((handler, data, establisher)).
fn unwind_frame(
    kind: u32,
    base: u64,
    pc: u64,
    entry: u64,
    context: &mut Ctx,
    pointers: u64,
) -> Result<(u64, u64, u64), Error> {
    let mut state = context.engine();
    let mut recorded = Pointers::default();
    let frame = engine::virtual_unwind(
        &mut Reader,
        kind,
        base,
        pc,
        function_at(entry),
        &mut state,
        &mut recorded,
    )?;
    context.apply(&state);
    if pointers != 0 {
        // KNONVOLATILE_CONTEXT_POINTERS: FloatingContext[16], IntegerContext[16].
        for i in 0..16 {
            if recorded.float[i] != 0 {
                write_u64(pointers + i as u64 * 8, recorded.float[i]);
            }
            if recorded.integer[i] != 0 {
                write_u64(pointers + 128 + i as u64 * 8, recorded.integer[i]);
            }
        }
    }
    let (handler, data) = frame.handler.unwrap_or((0, 0));
    Ok((handler, data, frame.establisher))
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn RtlVirtualUnwind(
    kind: u32,
    base: u64,
    pc: u64,
    entry: u64,
    context: u64,
    handler_data: u64,
    establisher: u64,
    pointers: u64,
) -> u64 {
    let mut ctx = Ctx::load(context);
    match unwind_frame(kind, base, pc, entry, &mut ctx, pointers) {
        Ok((handler, data, frame)) => {
            ctx.store(context);
            if handler_data != 0 {
                write_u64(handler_data, data);
            }
            if establisher != 0 {
                write_u64(establisher, frame);
            }
            handler
        }
        Err(_) => bad_table(),
    }
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn DoorVirtualUnwind(
    kind: u32,
    base: u64,
    pc: u64,
    entry: u64,
    context: u64,
    data: u64,
    frame: u64,
    pointers: u64,
) -> u64 {
    RtlVirtualUnwind(kind, base, pc, entry, context, data, frame, pointers)
}

// ---- capture and restore ------------------------------------------------------------

/// RtlCaptureContext: the caller's registers, RIP (the return address),
/// RSP (after the return), flags, segments and floating-point state.
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub extern "system" fn RtlCaptureContext(context: u64) {
    naked_asm!(
        "pushfq",
        "mov [rcx+0x78], rax",
        "mov [rcx+0x80], rcx",
        "mov [rcx+0x88], rdx",
        "mov [rcx+0x90], rbx",
        "lea rax, [rsp+0x10]",
        "mov [rcx+0x98], rax",
        "mov [rcx+0xa0], rbp",
        "mov [rcx+0xa8], rsi",
        "mov [rcx+0xb0], rdi",
        "mov [rcx+0xb8], r8",
        "mov [rcx+0xc0], r9",
        "mov [rcx+0xc8], r10",
        "mov [rcx+0xd0], r11",
        "mov [rcx+0xd8], r12",
        "mov [rcx+0xe0], r13",
        "mov [rcx+0xe8], r14",
        "mov [rcx+0xf0], r15",
        "mov rax, [rsp+8]",
        "mov [rcx+0xf8], rax",
        "mov word ptr [rcx+0x38], cs",
        "mov word ptr [rcx+0x3a], ds",
        "mov word ptr [rcx+0x3c], es",
        "mov word ptr [rcx+0x3e], fs",
        "mov word ptr [rcx+0x40], gs",
        "mov word ptr [rcx+0x42], ss",
        "pop rax",
        "mov [rcx+0x44], eax",
        "mov dword ptr [rcx+0x30], 0x10000f",
        "fxsave [rcx+0x100]",
        "stmxcsr [rcx+0x34]",
        "mov rax, [rcx+0x78]",
        "ret",
    )
}
/// Continue at `context` in user mode: floating-point state when the
/// CONTEXT carries it, every general register, flags, RSP and RIP.
#[unsafe(naked)]
extern "system" fn restore(context: u64) -> ! {
    naked_asm!(
        "test dword ptr [rcx+0x30], 8",
        "jz 2f",
        "fxrstor [rcx+0x100]",
        "2:",
        "mov rdx, [rcx+0x88]",
        "mov rbx, [rcx+0x90]",
        "mov rbp, [rcx+0xa0]",
        "mov rsi, [rcx+0xa8]",
        "mov rdi, [rcx+0xb0]",
        "mov r8, [rcx+0xb8]",
        "mov r9, [rcx+0xc0]",
        "mov r10, [rcx+0xc8]",
        "mov r11, [rcx+0xd0]",
        "mov r12, [rcx+0xd8]",
        "mov r13, [rcx+0xe0]",
        "mov r14, [rcx+0xe8]",
        "mov r15, [rcx+0xf0]",
        "mov rsp, [rcx+0x98]",
        "mov rax, [rcx+0xf8]",
        "push rax",
        "mov eax, [rcx+0x44]",
        "push rax",
        "mov rax, [rcx+0x78]",
        "mov rcx, [rcx+0x80]",
        "popfq",
        "ret",
    )
}
/// _JUMP_BUFFER (x64) fields RtlRestoreContext takes for STATUS_LONGJUMP.
fn apply_jump_buffer(context: &mut Ctx, buffer: u64) {
    // Frame 0, Rbx 8, Rsp 16, Rbp 24, Rsi 32, Rdi 40, R12..R15 48..72,
    // Rip 80, MxCsr 88, FpCsr 92, Spare 94, Xmm6..Xmm15 96...
    for (offset, register) in [
        (8, 3),
        (16, 4),
        (24, 5),
        (32, 6),
        (40, 7),
        (48, 12),
        (56, 13),
        (64, 14),
        (72, 15),
    ] {
        context.set(C_RAX + register * 8, read_u64(buffer + offset));
    }
    context.set(C_RIP, read_u64(buffer + 80));
    for i in 0..10 {
        let source = buffer + 96 + i * 16;
        let target = C_XMM + (6 + i as usize) * 16;
        for byte in 0..16 {
            context.0[target + byte] = unsafe { ((source + byte as u64) as *const u8).read() };
        }
    }
}
/// Calls an unwind-consolidation callback (a C++ catch funclet's caller)
/// on this stack below a machine frame that describes `context`, so an
/// unwind from inside the callback continues at the target frame; returns
/// the callback's continuation address.
#[unsafe(naked)]
extern "system" fn consolidate(context: u64, callback: u64, record: u64) -> u64 {
    naked_asm!(
        "sub rsp, 0x48",
        "mov rax, [rcx+0xf8]",
        "mov [rsp+0x20], rax",
        "mov rax, [rcx+0x98]",
        "mov [rsp+0x38], rax",
        "mov rcx, r8",
        "call rdx",
        // Not an epilogue: an unwind from the callback must reach the
        // machine frame, not this function's return.
        "nop",
        "add rsp, 0x48",
        "ret",
    )
}
/// `consolidate` with the target context's nonvolatile registers (RBX,
/// RBP, RSI, RDI, R12-R15, XMM6-XMM15) live, as Windows' RcConsolidateFrames
/// calls the callback: the machine frame restores only RIP and RSP, so an
/// exception thrown inside the catch block (a rethrow) unwinds past the
/// target frame with the target's own register values. The caller's
/// nonvolatile registers are kept and restored around the call.
#[unsafe(naked)]
extern "system" fn consolidate_frames(context: u64, callback: u64, record: u64) -> u64 {
    naked_asm!(
        "push rbx",
        "push rbp",
        "push rsi",
        "push rdi",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "sub rsp, 0xc8",
        "movups [rsp+0x20], xmm6",
        "movups [rsp+0x30], xmm7",
        "movups [rsp+0x40], xmm8",
        "movups [rsp+0x50], xmm9",
        "movups [rsp+0x60], xmm10",
        "movups [rsp+0x70], xmm11",
        "movups [rsp+0x80], xmm12",
        "movups [rsp+0x90], xmm13",
        "movups [rsp+0xa0], xmm14",
        "movups [rsp+0xb0], xmm15",
        "mov rbx, [rcx+0x90]",
        "mov rbp, [rcx+0xa0]",
        "mov rsi, [rcx+0xa8]",
        "mov rdi, [rcx+0xb0]",
        "mov r12, [rcx+0xd8]",
        "mov r13, [rcx+0xe0]",
        "mov r14, [rcx+0xe8]",
        "mov r15, [rcx+0xf0]",
        "movups xmm6, [rcx+0x200]",
        "movups xmm7, [rcx+0x210]",
        "movups xmm8, [rcx+0x220]",
        "movups xmm9, [rcx+0x230]",
        "movups xmm10, [rcx+0x240]",
        "movups xmm11, [rcx+0x250]",
        "movups xmm12, [rcx+0x260]",
        "movups xmm13, [rcx+0x270]",
        "movups xmm14, [rcx+0x280]",
        "movups xmm15, [rcx+0x290]",
        "call {consolidate}",
        "movups xmm6, [rsp+0x20]",
        "movups xmm7, [rsp+0x30]",
        "movups xmm8, [rsp+0x40]",
        "movups xmm9, [rsp+0x50]",
        "movups xmm10, [rsp+0x60]",
        "movups xmm11, [rsp+0x70]",
        "movups xmm12, [rsp+0x80]",
        "movups xmm13, [rsp+0x90]",
        "movups xmm14, [rsp+0xa0]",
        "movups xmm15, [rsp+0xb0]",
        "add rsp, 0xc8",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rdi",
        "pop rsi",
        "pop rbp",
        "pop rbx",
        "ret",
        consolidate = sym consolidate,
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlRestoreContext(context: u64, record: u64) -> ! {
    let mut ctx = Ctx::load(context);
    if record != 0 {
        let code = read_u32(record + R_CODE as u64);
        let count = read_u32(record + R_COUNT as u64);
        if code == STATUS_LONGJUMP && count >= 1 {
            apply_jump_buffer(&mut ctx, read_u64(record + R_INFO as u64));
        } else if code == STATUS_UNWIND_CONSOLIDATE && count >= 1 {
            let callback = read_u64(record + R_INFO as u64);
            let continuation = consolidate_frames(ctx.ptr(), callback, record);
            ctx.set(C_RIP, continuation);
        }
    }
    restore(ctx.ptr())
}

// ---- dispatcher stubs with machine frames -------------------------------------------

/// UNWIND_INFO of the stubs: version 1, prolog 4 bytes, two codes at
/// offset 4: ALLOC_SMALL 0x20 (shadow space) then PUSH_MACHFRAME.
#[repr(C, align(4))]
struct StubInfo([u8; 8]);
static STUB_INFO: StubInfo = StubInfo([1, 4, 2, 0, 4, 0x32, 4, 0x0a]);
/// Fault entry: the kernel calls it with RCX = EXCEPTION_RECORD and RDX =
/// CONTEXT on this thread's stack. It records the faulting RIP and RSP as
/// a machine frame and calls the dispatcher.
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub extern "system" fn DoorExceptionDispatch(record: u64, context: u64) -> ! {
    naked_asm!(
        "sub rsp, 0x48",
        "mov rax, [rdx+0xf8]",
        "mov [rsp+0x20], rax",
        "mov rax, [rdx+0x98]",
        "mov [rsp+0x38], rax",
        "call {dispatch}",
        "int3",
        dispatch = sym fault_dispatch,
    )
}
/// The stubs' code ranges and their RUNTIME_FUNCTIONs (relative to the
/// lower of stub and unwind info).
fn builtin(pc: u64) -> Option<(u64, u64)> {
    static ENTRIES: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
    let info = core::ptr::addr_of!(STUB_INFO) as u64;
    // Exact stub lengths: DoorExceptionDispatch 34 bytes (sub, two loads and
    // stores with 32-bit displacements, call rel32, int3), consolidate 39
    // (the same plus mov rcx,r8, call rdx, nop, add, ret).
    for (slot, start, length) in [
        (0, DoorExceptionDispatch as *const () as u64, 34u64),
        (3, consolidate as *const () as u64, 39),
    ] {
        if pc < start || pc >= start + length {
            continue;
        }
        let base = start.min(info) & !0xfff;
        if ENTRIES[slot].load(Ordering::Acquire) == 0 {
            ENTRIES[slot + 1].store(
                (start - base) | ((start - base + length) << 32),
                Ordering::Relaxed,
            );
            ENTRIES[slot + 2].store(info - base, Ordering::Relaxed);
            ENTRIES[slot].store(base, Ordering::Release);
        }
        // The entry lives in the statics: begin/end in one quadword, the
        // unwind RVA in the next (RUNTIME_FUNCTION's 12-byte layout).
        return Some((base, ENTRIES[slot + 1].as_ptr() as u64));
    }
    None
}

// ---- dispatch -------------------------------------------------------------------------

/// DISPATCHER_CONTEXT (x64).
#[repr(C)]
struct Dispatcher {
    control_pc: u64,
    image_base: u64,
    function_entry: u64,
    establisher_frame: u64,
    target_ip: u64,
    context_record: u64,
    language_handler: u64,
    handler_data: u64,
    history_table: u64,
    scope_index: u32,
    fill: u32,
}
fn call_handler(
    handler: u64,
    record: u64,
    frame: u64,
    context: u64,
    dispatcher: &mut Dispatcher,
) -> u32 {
    // PEXCEPTION_ROUTINE(ExceptionRecord, EstablisherFrame, ContextRecord, DispatcherContext).
    let routine: extern "system" fn(u64, u64, u64, u64) -> u32 =
        unsafe { core::mem::transmute(handler as usize) };
    routine(record, frame, context, dispatcher as *mut Dispatcher as u64)
}
/// Unwind `context` one frame for `kind`: (handler, data, establisher,
/// base, entry). A function without unwind data is a leaf.
fn step(kind: u32, context: &mut Ctx) -> Result<(u64, u64, u64, u64, u64), ()> {
    let pc = context.rip();
    match lookup(pc) {
        Some((base, entry)) => {
            let (handler, data, frame) =
                unwind_frame(kind, base, pc, entry, context, 0).map_err(|_| ())?;
            Ok((handler, data, frame, base, entry))
        }
        None => {
            let rsp = context.rsp();
            let mut word = [0u8; 8];
            Reader.read(rsp, &mut word).map_err(|_| ())?;
            context.set(C_RIP, u64::from_le_bytes(word));
            context.set(C_RSP, rsp + 8);
            Ok((0, 0, rsp, 0, 0))
        }
    }
}
/// Frame-based dispatch: Ok(true) when a handler said continue (the
/// CONTEXT at `context` may have been changed by it).
fn dispatch(record: u64, context: u64) -> bool {
    let (limit, stack_base) = stack_bounds();
    let mut frame_context = Ctx::load(context);
    for _ in 0..4096 {
        let mut next = frame_context;
        let pc = frame_context.rip();
        let Ok((handler, data, establisher, base, entry)) = step(UNW_FLAG_EHANDLER, &mut next)
        else {
            break;
        };
        if establisher & 7 != 0 || establisher < limit || establisher > stack_base {
            write_u32(
                record + R_FLAGS as u64,
                read_u32(record + R_FLAGS as u64) | STACK_INVALID,
            );
            break;
        }
        if handler != 0 {
            let mut dispatcher = Dispatcher {
                control_pc: pc,
                image_base: base,
                function_entry: entry,
                establisher_frame: establisher,
                target_ip: 0,
                context_record: frame_context.ptr(),
                language_handler: handler,
                handler_data: data,
                history_table: 0,
                scope_index: 0,
                fill: 0,
            };
            let disposition = call_handler(handler, record, establisher, context, &mut dispatcher);
            let flags = read_u32(record + R_FLAGS as u64);
            write_u32(record + R_FLAGS as u64, flags & NONCONTINUABLE);
            match disposition {
                CONTINUE_EXECUTION => {
                    if flags & NONCONTINUABLE != 0 {
                        raise_status(STATUS_NONCONTINUABLE_EXCEPTION, record);
                    }
                    return true;
                }
                CONTINUE_SEARCH | NESTED_EXCEPTION => {}
                COLLIDED => {
                    // Restart from where the colliding unwind had got to.
                    frame_context = Ctx::load(dispatcher.context_record);
                    continue;
                }
                _ => raise_status(STATUS_INVALID_DISPOSITION, record),
            }
        }
        if next.rip() == 0 || frame_context.rsp() >= stack_base {
            break;
        }
        frame_context = next;
    }
    false
}
/// A new noncontinuable exception chained to `previous`.
fn raise_status(code: u32, previous: u64) -> ! {
    let mut record = [0u8; RECORD_BYTES];
    record[R_CODE..R_CODE + 4].copy_from_slice(&code.to_le_bytes());
    record[R_FLAGS..R_FLAGS + 4].copy_from_slice(&NONCONTINUABLE.to_le_bytes());
    record[8..16].copy_from_slice(&previous.to_le_bytes());
    RtlRaiseException(record.as_mut_ptr() as u64);
    door_sys::exit(i64::from(code))
}

static TOP_LEVEL_FILTER: AtomicU64 = AtomicU64::new(0);
/// kernel32's SetUnhandledExceptionFilter: the previous filter.
#[unsafe(no_mangle)]
pub extern "system" fn DoorSetUnhandledExceptionFilter(filter: u64) -> u64 {
    TOP_LEVEL_FILTER.swap(filter, Ordering::AcqRel)
}
/// UnhandledExceptionFilter: the top-level filter's verdict, else
/// EXCEPTION_EXECUTE_HANDLER (terminate).
#[unsafe(no_mangle)]
pub extern "system" fn DoorUnhandledExceptionFilter(pointers: u64) -> i32 {
    let filter = TOP_LEVEL_FILTER.load(Ordering::Acquire);
    if filter == 0 {
        return 1;
    }
    let f: extern "system" fn(u64) -> i32 = unsafe { core::mem::transmute(filter as usize) };
    f(pointers)
}
/// No handler took the exception: the top-level filter decides, then the
/// process ends with the exception code (no DLL notifications, as Windows).
fn unhandled(record: u64, context: u64) -> ! {
    let mut pointers = [record, context];
    if DoorUnhandledExceptionFilter(pointers.as_mut_ptr() as u64) == -1 {
        restore(context);
    }
    log_backtrace(context);
    door_sys::exit(i64::from(read_u32(record + R_CODE as u64)))
}
/// `DOOR_P18 BACKTRACE <rip> <caller> ...`: the faulting address and up to
/// 15 callers, unwound through the function tables (P18 diagnostics: what a
/// program ran into when it ended on an unhandled exception).
fn log_backtrace(context: u64) {
    let mut ctx = Ctx::load(context);
    let mut line = [0u8; 18 + 16 * 13 + 1];
    line[..18].copy_from_slice(b"DOOR_P18 BACKTRACE");
    let mut n = 18;
    for frame in 0..16 {
        if frame > 0 && step(UNW_FLAG_NHANDLER, &mut ctx).is_err() {
            break;
        }
        let pc = ctx.rip();
        if pc == 0 {
            break;
        }
        line[n] = b' ';
        for i in 0..12 {
            line[n + 1 + i] = b"0123456789abcdef"[((pc >> ((11 - i) * 4)) & 15) as usize];
        }
        n += 13;
    }
    line[n] = b'\n';
    let _ = door_sys::write(&line[..=n]);
}
/// Continue at `context`: a delivered fault through the kernel (NtContinue
/// checks and resumes the pending fault context), anything else in user mode.
fn resume(context: u64, fault: bool) -> ! {
    if fault {
        super::exception::NtContinue(door_windows_abi::narrow(context), 0);
    }
    restore(context)
}
/// Vectored handlers, then frame handlers, then the unhandled path.
fn dispatch_all(record: u64, context: u64, fault: bool) -> ! {
    if super::exception::vectored_continue(record, context) {
        resume(context, fault);
    }
    if dispatch(record, context) {
        resume(context, fault);
    }
    unhandled(record, context)
}
extern "system" fn fault_dispatch(record: u64, context: u64) -> ! {
    dispatch_all(record, context, true)
}

/// RtlRaiseException: the caller's context (this frame unwound once),
/// ExceptionAddress kept when set, then dispatch. A handler that continues
/// returns to the caller.
#[unsafe(no_mangle)]
pub extern "system" fn RtlRaiseException(record: u64) {
    let mut context = Ctx::zero();
    RtlCaptureContext(context.ptr());
    if step(UNW_FLAG_NHANDLER, &mut context).is_err() {
        door_sys::exit(i64::from(read_u32(record + R_CODE as u64)));
    }
    if read_u64(record + R_ADDRESS as u64) == 0 {
        write_u64(record + R_ADDRESS as u64, context.rip());
    }
    log_raise(
        read_u32(record + R_CODE as u64),
        read_u64(record + R_ADDRESS as u64),
        context.rsp(),
    );
    dispatch_all(record, context.ptr(), false)
}
/// The first software exceptions a process raises, on the serial log
/// (`DOOR_EXC raise code=.. address=.. rsp=..`, within the diagnostic
/// budget): a program's exception traffic, and a runaway raise loop.
fn log_raise(code: u32, address: u64, rsp: u64) {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    if COUNT.fetch_add(1, Ordering::Relaxed) >= 32 {
        return;
    }
    let mut line = [0u8; 96];
    let mut n = 0;
    let mut push = |bytes: &[u8]| {
        for &b in bytes {
            if n < line.len() {
                line[n] = b;
                n += 1;
            }
        }
    };
    let hex = |value: u64, digits: usize, out: &mut [u8; 16]| {
        for (i, slot) in out[..digits].iter_mut().enumerate() {
            *slot = b"0123456789abcdef"[((value >> ((digits - 1 - i) * 4)) & 15) as usize];
        }
    };
    let mut digits = [0u8; 16];
    push(b"DOOR_EXC raise code=0x");
    hex(u64::from(code), 8, &mut digits);
    push(&digits[..8]);
    push(b" address=0x");
    hex(address, 16, &mut digits);
    push(&digits);
    push(b" rsp=0x");
    hex(rsp, 16, &mut digits);
    push(&digits);
    push(b"\n");
    let _ = door_sys::write(&line[..n]);
}

// ---- RtlUnwindEx ------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn RtlUnwindEx(
    target_frame: u64,
    target_ip: u64,
    record: u64,
    return_value: u64,
    context: u64,
    _history: u64,
) {
    let mut local = Ctx::zero();
    let context = if context == 0 { local.ptr() } else { context };
    let mut current = Ctx::zero();
    RtlCaptureContext(current.ptr());
    let mut own = [0u8; RECORD_BYTES];
    let record = if record == 0 {
        own[R_CODE..R_CODE + 4].copy_from_slice(&STATUS_UNWIND.to_le_bytes());
        own[R_ADDRESS..R_ADDRESS + 8].copy_from_slice(&current.rip().to_le_bytes());
        own.as_mut_ptr() as u64
    } else {
        record
    };
    let flags = read_u32(record + R_FLAGS as u64)
        | UNWINDING
        | if target_frame == 0 { EXIT_UNWIND } else { 0 };
    write_u32(record + R_FLAGS as u64, flags);
    let (limit, stack_base) = stack_bounds();
    for _ in 0..4096 {
        let mut next = current;
        let pc = current.rip();
        let Ok((handler, data, establisher, base, entry)) = step(UNW_FLAG_UHANDLER, &mut next)
        else {
            raise_status(STATUS_BAD_FUNCTION_TABLE, record);
        };
        if establisher & 7 != 0 || establisher < limit || establisher > stack_base {
            write_u32(
                record + R_FLAGS as u64,
                read_u32(record + R_FLAGS as u64) | STACK_INVALID,
            );
            break;
        }
        if handler != 0 {
            if target_frame != 0 && establisher > target_frame {
                raise_status(STATUS_INVALID_UNWIND_TARGET, record);
            }
            if establisher == target_frame {
                write_u32(
                    record + R_FLAGS as u64,
                    read_u32(record + R_FLAGS as u64) | TARGET_UNWIND,
                );
            }
            current.store(context);
            let mut dispatcher = Dispatcher {
                control_pc: pc,
                image_base: base,
                function_entry: entry,
                establisher_frame: establisher,
                target_ip,
                context_record: context,
                language_handler: handler,
                handler_data: data,
                history_table: 0,
                scope_index: 0,
                fill: 0,
            };
            match call_handler(handler, record, establisher, context, &mut dispatcher) {
                CONTINUE_SEARCH => {}
                COLLIDED => {
                    current = Ctx::load(dispatcher.context_record);
                    write_u32(
                        record + R_FLAGS as u64,
                        read_u32(record + R_FLAGS as u64) | COLLIDED_UNWIND,
                    );
                    continue;
                }
                _ => raise_status(STATUS_INVALID_DISPOSITION, record),
            }
            write_u32(
                record + R_FLAGS as u64,
                read_u32(record + R_FLAGS as u64) & !COLLIDED_UNWIND,
            );
        }
        if establisher == target_frame || (target_frame == 0 && current.rsp() >= stack_base) {
            break;
        }
        current = next;
    }
    current.set(C_RAX, return_value);
    if target_ip != 0 {
        current.set(C_RIP, target_ip);
    }
    current.store(context);
    RtlRestoreContext(context, record)
}
/// RtlUnwind: RtlUnwindEx with a context of its own.
#[unsafe(no_mangle)]
pub extern "system" fn RtlUnwind(
    target_frame: u64,
    target_ip: u64,
    record: u64,
    return_value: u64,
) {
    let mut context = Ctx::zero();
    RtlUnwindEx(
        target_frame,
        target_ip,
        record,
        return_value,
        context.ptr(),
        0,
    )
}

#[allow(dead_code)]
fn unused_flags() -> u32 {
    let _ = RtlSetLastWin32Error;
    CONTEXT_CAPTURED | C_FLAGS as u32 | UNW_FLAG_NHANDLER
}

// ---- thread contexts ------------------------------------------------------------------

/// Copy the parts of CONTEXT `source` that `flags` names into `target`.
fn merge(target: u64, flags: u32, source: &Ctx) {
    let copy = |offset: usize, bytes: usize| unsafe {
        core::ptr::copy_nonoverlapping(
            source.0.as_ptr().add(offset),
            (target + offset as u64) as *mut u8,
            bytes,
        )
    };
    if flags & 1 != 0 {
        copy(56, 2);
        copy(66, 2);
        copy(68, 4);
        copy(C_RSP, 8);
        copy(C_RIP, 8);
    }
    if flags & 2 != 0 {
        copy(C_RAX, 32);
        copy(160, 88);
    }
    if flags & 4 != 0 {
        copy(58, 8);
    }
    if flags & 8 != 0 {
        copy(52, 4);
        copy(256, 512);
    }
}
const CURRENT_THREAD: u64 = u64::MAX - 1;
/// The kernel's copy of a suspended thread's context, retrying while the
/// suspension has not yet taken effect.
fn thread_context(thread: u64, buffer: &mut Ctx, set: bool) -> Result<(), door_abi::Status> {
    let operation = if set {
        door_abi::operation::THREAD_SET_CONTEXT
    } else {
        door_abi::operation::THREAD_GET_CONTEXT
    };
    for _ in 0..10_000 {
        match unsafe { door_sys::raw(operation, [thread, buffer.ptr(), 0, 0, 0, 0]) }.result() {
            Err(door_abi::Status::WouldBlock) => {
                let _ = door_sys::yield_now();
            }
            other => return other.map(|_| ()),
        }
    }
    Err(door_abi::Status::Timeout)
}
fn nt_status(error: door_abi::Status) -> u32 {
    match error {
        door_abi::Status::BadHandle | door_abi::Status::Canceled => 0xc000_0008,
        door_abi::Status::AccessDenied => 0xc000_0022,
        door_abi::Status::Unsupported => 0xc000_00bb,
        door_abi::Status::BadAddress => 0xc000_0005,
        _ => 0xc000_000d,
    }
}
/// NtGetContextThread: the parts the CONTEXT's flags ask for. The current
/// thread gets its own state at this call.
#[unsafe(no_mangle)]
pub extern "system" fn NtGetContextThread(thread: u64, context: u64) -> u32 {
    let flags = read_u32(context + C_FLAGS as u64);
    let mut full = Ctx::zero();
    if thread == CURRENT_THREAD {
        RtlCaptureContext(full.ptr());
        if step(UNW_FLAG_NHANDLER, &mut full).is_err() {
            return 0xc000_000d;
        }
    } else {
        match thread_context(thread, &mut full, false) {
            Ok(()) => {}
            Err(door_abi::Status::InvalidArgument) => {
                // A handle to the calling thread itself.
                RtlCaptureContext(full.ptr());
                let _ = step(UNW_FLAG_NHANDLER, &mut full);
            }
            Err(error) => return nt_status(error),
        }
    }
    merge(context, flags, &full);
    0
}
/// NtSetContextThread on a suspended thread of this process.
#[unsafe(no_mangle)]
pub extern "system" fn NtSetContextThread(thread: u64, context: u64) -> u32 {
    if thread == CURRENT_THREAD {
        return 0xc000_000d;
    }
    let flags = read_u32(context + C_FLAGS as u64);
    let mut full = Ctx::zero();
    if let Err(error) = thread_context(thread, &mut full, false) {
        return nt_status(error);
    }
    merge(full.ptr(), flags, &Ctx::load(context));
    match thread_context(thread, &mut full, true) {
        Ok(()) => 0,
        Err(error) => nt_status(error),
    }
}

/// Where a call failed with ERROR_INVALID_FUNCTION, ERROR_ACCESS_DENIED,
/// ERROR_INVALID_HANDLE, ERROR_NOT_ENOUGH_MEMORY, ERROR_NOT_SUPPORTED,
/// ERROR_INVALID_PARAMETER, ERROR_CALL_NOT_IMPLEMENTED, ERROR_INVALID_NAME
/// or ERROR_COMMITMENT_LIMIT: the first 64 such errors, each with the
/// return addresses of the four frames above RtlSetLastWin32Error
/// (`DOOR_ERR code=.. at=[..]`, within the diagnostic budget). A program that gives up on a Doors gap names it this way.
pub(super) fn log_error_site(code: u32) {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    if !matches!(code, 1 | 5 | 6 | 8 | 50 | 87 | 120 | 123 | 1455)
        || COUNT.fetch_add(1, Ordering::Relaxed) >= 64
    {
        return;
    }
    let mut context = Ctx::zero();
    RtlCaptureContext(context.ptr());
    let mut sites = [0u64; 5];
    for site in &mut sites {
        if step(UNW_FLAG_NHANDLER, &mut context).is_err() {
            break;
        }
        *site = context.rip();
    }
    let mut line = [0u8; 160];
    let mut n = 0;
    let mut push = |bytes: &[u8]| {
        for &b in bytes {
            if n < line.len() {
                line[n] = b;
                n += 1;
            }
        }
    };
    push(b"DOOR_ERR code=");
    let mut digits = [0u8; 10];
    let mut d = digits.len();
    let mut v = code;
    loop {
        d -= 1;
        digits[d] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    push(&digits[d..]);
    push(b" at=");
    // Skip the logger's own frame (RtlSetLastWin32Error's caller first).
    for &site in &sites[1..] {
        let mut hex = [0u8; 13];
        hex[0] = b' ';
        for (i, slot) in hex[1..].iter_mut().enumerate() {
            *slot = b"0123456789abcdef"[((site >> ((11 - i) * 4)) & 15) as usize];
        }
        push(&hex);
    }
    push(b"\n");
    let _ = door_sys::write(&line[..n]);
}

// ---- __C_specific_handler ---------------------------------------------------------------

/// `__C_specific_handler`: the language handler MSVC gives C functions with
/// `__try`/`__except` and `__try`/`__finally` (x64). `HandlerData` is the
/// function's scope table (Count, then {Begin, End, Handler, JumpTarget}
/// RVAs). Dispatching, each `__except` scope around the faulting address
/// runs its filter (Handler 1 is EXCEPTION_EXECUTE_HANDLER): a positive
/// result unwinds to the scope's JumpTarget, a negative one continues
/// execution, zero searches on. Unwinding, each `__finally` scope the
/// unwind leaves runs its termination handler (abnormal termination), up
/// to the target scope; `ScopeIndex` records progress so a nested
/// exception resumes after the handlers already run.
#[unsafe(no_mangle)]
pub extern "system" fn __C_specific_handler(
    record: u64,
    frame: u64,
    context: u64,
    dispatcher: u64,
) -> u32 {
    const CONTINUE_EXECUTION: u32 = 0;
    const CONTINUE_SEARCH: u32 = 1;
    // SAFETY: the dispatcher hands its own DISPATCHER_CONTEXT.
    let dispatch = unsafe { &mut *(dispatcher as *mut Dispatcher) };
    let base = dispatch.image_base;
    let table = dispatch.handler_data;
    if table == 0 {
        return CONTINUE_SEARCH;
    }
    let count = read_u32(table);
    let entry = |i: u32| {
        let at = table + 4 + u64::from(i) * 16;
        (
            base + u64::from(read_u32(at)),
            base + u64::from(read_u32(at + 4)),
            read_u32(at + 8),
            read_u32(at + 12),
        )
    };
    let flags = read_u32(record + R_FLAGS as u64);
    let pc = dispatch.control_pc;
    if flags & (UNWINDING | EXIT_UNWIND) != 0 {
        for i in dispatch.scope_index..count {
            let (begin, end, handler, jump) = entry(i);
            if pc < begin || pc >= end {
                continue;
            }
            if jump != 0 {
                // The unwind's target `__except` scope: stop here.
                if flags & TARGET_UNWIND != 0 && dispatch.target_ip == base + u64::from(jump) {
                    break;
                }
                continue;
            }
            dispatch.scope_index = i + 1;
            // SAFETY: the function's own termination handler.
            let finally: extern "system" fn(u8, u64) =
                unsafe { core::mem::transmute((base + u64::from(handler)) as usize) };
            finally(1, frame);
        }
        return CONTINUE_SEARCH;
    }
    for i in dispatch.scope_index..count {
        let (begin, end, handler, jump) = entry(i);
        if pc < begin || pc >= end || jump == 0 {
            continue;
        }
        if handler != 1 {
            let pointers = [record, context];
            // SAFETY: the function's own exception filter.
            let filter: extern "system" fn(u64, u64) -> i32 =
                unsafe { core::mem::transmute((base + u64::from(handler)) as usize) };
            match filter(pointers.as_ptr() as u64, frame) {
                result if result < 0 => return CONTINUE_EXECUTION,
                0 => continue,
                _ => {}
            }
        }
        let code = u64::from(read_u32(record + R_CODE as u64));
        RtlUnwindEx(
            frame,
            base + u64::from(jump),
            record,
            code,
            dispatch.context_record,
            dispatch.history_table,
        );
    }
    CONTINUE_SEARCH
}

// ---- stack back traces --------------------------------------------------------------

/// RtlCaptureStackBackTrace: the return addresses of the caller's frames
/// (frame 0 is the caller), after `skip`, unwound through the images'
/// function tables; leaf frames pop their return address. Stops at the
/// stack's end or at a frame the tables cannot unwind. `hash` (optional)
/// receives the sum of the captured addresses.
#[unsafe(no_mangle)]
pub extern "system" fn RtlCaptureStackBackTrace(skip: u32, count: u32, out: u64, hash: u64) -> u16 {
    let mut context = Ctx::zero();
    RtlCaptureContext(context.ptr());
    let (limit, base) = stack_bounds();
    let mut captured = 0u32;
    let mut sum = 0u32;
    // Frame -1 is this function; each pass unwinds one frame.
    for frame in 0..(skip as u64 + count as u64 + 1).min(1024) {
        let pc = context.rip();
        let rsp = context.rsp();
        if pc == 0 || rsp < limit || rsp >= base {
            break;
        }
        if frame > skip as u64 && captured < count && out != 0 {
            write_u64(out + captured as u64 * 8, pc);
            sum = sum.wrapping_add(pc as u32);
            captured += 1;
        }
        if captured >= count {
            break;
        }
        let mut image = 0u64;
        let entry = RtlLookupFunctionEntry(pc, &mut image as *mut u64 as u64, 0);
        if entry == 0 {
            // A leaf: its return address is at RSP.
            if rsp + 8 > base {
                break;
            }
            context.set(C_RIP, read_u64(rsp));
            context.set(C_RSP, rsp + 8);
        } else if unwind_frame(0, image, pc, entry, &mut context, 0).is_err() {
            break;
        }
    }
    if hash != 0 {
        write_u32(hash, sum);
    }
    captured as u16
}
