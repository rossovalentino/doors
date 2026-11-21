//! Protected native execution. Metadata authority and VM ownership have separate
//! locks: metadata never waits for VM, and VM contention admits interrupt/IPI
//! service. Every user buffer is validated while its address space is pinned.
mod fixtures;
mod platform;
mod vm;
mod windows;
use crate::{arch, execution as run, startup::Foundations};
use core::{
    arch::asm,
    cell::UnsafeCell,
    fmt::{self, Write},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use door_abi::{Inherited, Startup, Status, operation as op, rights};
use door_boot_debug::Serial;
use door_kernel::{
    executive::{Executive, Lease, MAX_PROCESSES, ProcessId},
    mm::UserAccess,
    process_image::ImagePlan,
    task::{ObjectId, State, TaskId, WakeReason},
};
use vm::Vm;

const COUNT: usize = 6;
const MAX_VIEWS: usize = 32;
#[derive(Clone, Copy)]
struct Record {
    pid: ProcessId,
    task: TaskId,
    role: u64,
    exited: ObjectId,
}
struct View {
    address: u64,
    bytes: u64,
    lease: Option<Lease>,
    busy: bool,
}
struct Metadata {
    executive: Executive,
    records: [Option<Record>; door_kernel::task::MAX_TASKS],
    processes: [Option<platform::Process>; MAX_PROCESSES],
    threads: [Option<platform::Thread>; door_kernel::task::MAX_TASKS],
    views: [[Option<View>; MAX_VIEWS]; MAX_PROCESSES],
    ipc_sends: u64,
    ipc_receives: u64,
    section_maps: u64,
}
impl Metadata {
    const fn new() -> Self {
        Self {
            executive: Executive::new(),
            records: [None; door_kernel::task::MAX_TASKS],
            processes: [const { None }; MAX_PROCESSES],
            threads: [const { None }; door_kernel::task::MAX_TASKS],
            views: [const { [const { None }; MAX_VIEWS] }; MAX_PROCESSES],
            ipc_sends: 0,
            ipc_receives: 0,
            section_maps: 0,
        }
    }
}
struct Storage<T>(UnsafeCell<Option<T>>);
// Access only inside corresponding interrupt-safe lock below.
unsafe impl<T> Sync for Storage<T> {}
// The enlarged executive lives in its final static owner from boot. Building
// and moving an equivalent local value would exceed the guarded boot stack.
static META: Storage<Metadata> = Storage(UnsafeCell::new(Some(Metadata::new())));
struct VmStorage(UnsafeCell<core::mem::MaybeUninit<Vm>>);
// The VM guard is its exclusive mutable owner after one-time boot publication.
unsafe impl Sync for VmStorage {}
static VM: VmStorage = VmStorage(UnsafeCell::new(core::mem::MaybeUninit::uninit()));
static VM_READY: AtomicBool = AtomicBool::new(false);
fn initialize_vm(
    frames: door_kernel::mm::FrameAllocator<'static>,
    tables: door_kernel::mm::PageTables<'static>,
) {
    assert!(
        !VM_READY.load(Ordering::Acquire),
        "process runtime initialized once"
    );
    // SAFETY: only the serialized boot owner calls this, before installing hooks
    // or publishing a process. No reference exists until every field is valid.
    unsafe {
        Vm::initialize_at((*VM.0.get()).as_mut_ptr(), frames, tables);
    }
    VM_READY.store(true, Ordering::Release);
}
static META_LOCK: AtomicBool = AtomicBool::new(false);
static VM_LOCK: AtomicBool = AtomicBool::new(false);
static SERIAL_LOCK: AtomicBool = AtomicBool::new(false);
static SECTION_KEY: AtomicU64 = AtomicU64::new(1);
/// A fresh section key (old sections and Windows sections share them).
fn next_section_key() -> Result<u64, Status> {
    SECTION_KEY
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |x| x.checked_add(1))
        .map_err(|_| Status::ResourceExhausted)
}

fn irq_save() -> u64 {
    let flags;
    unsafe {
        asm!("pushfq","pop {}","cli",out(reg)flags);
    }
    flags
}
fn irq_restore(flags: u64) {
    if flags & 0x200 != 0 {
        arch::enable_interrupts();
    }
}
fn lock_short(lock: &AtomicBool) {
    while lock
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}
fn with_meta<R>(f: impl FnOnce(&mut Metadata) -> R) -> R {
    let flags = irq_save();
    lock_short(&META_LOCK);
    let result = f(unsafe {
        (*META.0.get())
            .as_mut()
            .expect("process metadata initialized")
    });
    META_LOCK.store(false, Ordering::Release);
    irq_restore(flags);
    result
}
fn with_vm<R>(f: impl FnOnce(&mut Vm) -> R) -> R {
    let flags = irq_save();
    while VM_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        // A VM owner may await this CPU's TLB acknowledgement. No metadata or
        // scheduler lock is held here; a preempted waiter retains no VM borrow.
        arch::enable_interrupts();
        core::hint::spin_loop();
        arch::disable_interrupts();
    }
    assert!(VM_READY.load(Ordering::Acquire), "VM initialized");
    // SAFETY: initialization publication is acquired and this guard is exclusive.
    let result = f(unsafe { (*VM.0.get()).assume_init_mut() });
    VM_LOCK.store(false, Ordering::Release);
    irq_restore(flags);
    result
}

/// Serialize kernel DMA allocation and mapping with user address-space changes.
/// The closure runs with IRQs disabled; it must not wait for device completion.
pub(crate) fn with_kernel_memory<R>(
    f: impl FnOnce(
        &mut door_kernel::mm::FrameAllocator<'static>,
        &mut door_kernel::mm::PageTables<'static>,
    ) -> R,
) -> R {
    with_vm(|vm| {
        let (frames, tables) = vm.kernel_memory();
        f(frames, tables)
    })
}
fn log(args: fmt::Arguments<'_>) {
    let flags = irq_save();
    lock_short(&SERIAL_LOCK);
    unsafe { Serial::new() }.write_fmt(args).unwrap();
    SERIAL_LOCK.store(false, Ordering::Release);
    irq_restore(flags);
}
fn current() -> Record {
    let task = run::current();
    with_meta(|m| {
        m.records
            .iter()
            .flatten()
            .find(|r| r.task == task)
            .copied()
            .expect("native task owner")
    })
}
fn read(vm: &mut Vm, r: Record, address: u64, out: &mut [u8]) -> Result<(), Status> {
    vm.validate(r.pid.slot(), address, out.len(), UserAccess::Read)?;
    unsafe { arch::copy_from_user(vm.cr3(r.pid.slot())?, address, out) }
        .map_err(|_| Status::BadAddress)?;
    Ok(())
}
fn write(vm: &mut Vm, r: Record, address: u64, bytes: &[u8]) -> Result<(), Status> {
    vm.validate_write(r.pid.slot(), address, bytes.len())?;
    unsafe { arch::copy_to_user(vm.cr3(r.pid.slot())?, address, bytes) }
        .map_err(|_| Status::BadAddress)?;
    Ok(())
}
fn validate(vm: &mut Vm, r: Record, address: u64, bytes: usize) -> Result<(), Status> {
    vm.validate_write(r.pid.slot(), address, bytes)
}
fn result(context: *mut arch::Context, value: Result<u64, Status>) {
    let frame = unsafe { &mut *((*context).frame as *mut arch::TrapFrame) };
    match value {
        Ok(v) => {
            frame.rax = 0;
            frame.rdx = v;
        }
        Err(e) => {
            frame.rax = e as u64;
            frame.rdx = 0;
        }
    }
}
fn cleanup_sections() {
    loop {
        let key = with_meta(|m| m.executive.take_retired_section());
        let Some(key) = key else {
            break;
        };
        with_vm(|v| unsafe { v.destroy_section(key) }).expect("unreferenced section retirement");
    }
    // Windows sections: freed now, or with their last view.
    while let Some(key) = with_meta(|m| m.executive.take_retired_windows_section()) {
        with_vm(|v| v.win_section_retire(key)).expect("retired Windows section");
    }
}

unsafe extern "C" fn syscall(context: *mut arch::Context, _cpu: usize) -> *mut arch::Context {
    let r = current();
    if r.role >= 100 {
        platform::reap();
        if let Some(code) = run::stop_requested(r.task) {
            return unsafe { run::exit_user(context, code) };
        }
    }
    let frame = unsafe { &*((*context).frame as *const arch::TrapFrame) };
    let selector = door_abi::decode_selector(frame.rax);
    let args = [
        frame.rdi, frame.rsi, frame.rdx, frame.r10, frame.r8, frame.r9,
    ];
    if selector == Ok(op::EXIT) {
        return unsafe { terminate(context, r, args[0] as i64) };
    }
    if selector == Ok(op::CONTEXT_RESUME) {
        if let Err(error) = unsafe { windows::exception::resume(context, r, args) } {
            result(context, Err(error));
        }
        return context;
    }
    if selector == Ok(op::THREAD_EXIT) && r.role >= 100 {
        if args[1..].iter().any(|argument| *argument != 0) {
            result(context, Err(Status::InvalidArgument));
            return context;
        }
        return unsafe { run::exit_user(context, args[0] as i64) };
    }
    if selector == Ok(op::YIELD) {
        result(context, Ok(0));
        return unsafe { arch::schedule_from_user(context, 1) };
    }
    let value = selector.and_then(|op| dispatch(r, op, args));
    result(context, value);
    cleanup_sections();
    if r.role >= 100 {
        platform::cleanup_waits();
        if let Some(code) = run::stop_requested(r.task) {
            return unsafe { run::exit_user(context, code) };
        }
    }
    context
}
unsafe extern "C" fn fault(
    context: *mut arch::Context,
    _cpu: usize,
    vector: u64,
    error: u64,
    address: u64,
) -> *mut arch::Context {
    let r = current();
    let mut error = error;
    // Paged Windows memory (P18 M0b) first: an EXECUTE_READWRITE page
    // changing sides retries the instruction; a guard page's first touch
    // raises STATUS_GUARD_PAGE_VIOLATION.
    if vector == 14 && matches!(r.role, 107 | 110 | 115) {
        let access = if error & 16 != 0 {
            UserAccess::Execute
        } else if error & 2 != 0 {
            UserAccess::Write
        } else {
            UserAccess::Read
        };
        match with_vm(|v| v.windows_fault(r.pid.slot(), address, access)) {
            door_kernel::mm::Fault::Toggled => return context,
            door_kernel::mm::Fault::Guard => {
                // The thread's own stack growing through its guard page, as
                // Windows' kernel grows it (P18 M0h); any other guard page
                // is the program's: STATUS_GUARD_PAGE_VIOLATION.
                let frame = unsafe { &*((*context).frame as *const arch::TrapFrame) };
                let teb = unsafe { (*context).user_gs_base };
                let wide = frame.cs != u64::from(arch::USER_CODE32_SELECTOR);
                match with_vm(|v| v.windows_stack_guard(r.pid.slot(), address, frame.rsp)) {
                    vm::StackGrowth::Grown(limit) => {
                        if wide && teb != 0 {
                            // NT_TIB.StackLimit: the lowest ordinary page.
                            let _ = with_vm(|v| write(v, r, teb + 16, &limit.to_le_bytes()));
                        }
                        return context;
                    }
                    vm::StackGrowth::Overflow => error |= windows::exception::STACK_OVERFLOW_FAULT,
                    vm::StackGrowth::NotStack => error |= windows::exception::GUARD_FAULT,
                }
            }
            // `windows_fault` resolves copies itself.
            door_kernel::mm::Fault::Violation
            | door_kernel::mm::Fault::CopyOnWrite
            | door_kernel::mm::Fault::Demand => {}
        }
    }
    if unsafe { windows::exception::deliver(context, r, vector, error, address) }.is_ok() {
        return context;
    }
    if r.role >= 100 {
        log(format_args!(
            "DOOR_P08 USER_FAULT pid={} manifest={} vector={vector} error={error:#x} address={address:#x}\n",
            r.pid.raw(),
            r.role - 100
        ));
        platform::stop_process(r.pid, -(vector as i64));
        return unsafe { run::exit_user(context, -(vector as i64)) };
    }
    log(format_args!(
        "DOOR_P05 USER_FAULT pid={} role={} vector={vector} error={error:#x} address={address:#x}\n",
        r.pid.raw(),
        r.role
    ));
    unsafe { terminate(context, r, -(vector as i64)) }
}
unsafe fn terminate(context: *mut arch::Context, r: Record, code: i64) -> *mut arch::Context {
    if r.role >= 100 {
        platform::stop_process(r.pid, code);
        return unsafe { run::exit_user(context, code) };
    }
    with_meta(|m| m.executive.exit(r.pid, code)).expect("process exit once");
    run::with(|s| s.signal_event(r.exited)).expect("process exit signal");
    log(format_args!(
        "DOOR_P05 PROCESS role={} exit={code} pid={}\n",
        r.role,
        r.pid.raw()
    ));
    unsafe { run::exit_user(context, code) }
}

fn dispatch(r: Record, operation: u32, a: [u64; 6]) -> Result<u64, Status> {
    if r.role >= 100 {
        with_meta(|m| m.executive.authority(r.pid))?;
    }
    match operation {
        op::HARDWARE_INFO => {
            if a[1] != door_abi::hardware::BYTES as u64 || a[2..].iter().any(|v| *v != 0) {
                return Err(Status::InvalidArgument);
            }
            let bytes = crate::startup::hardware_info()?.encode()?;
            with_vm(|v| write(v, r, a[0], &bytes))?;
            Ok(bytes.len() as u64)
        }
        op::QUERY => {
            if a[1] != 64 {
                return Err(Status::InvalidArgument);
            }
            let mut b = [0; 64];
            door_abi::AbiInfo {
                groups: door_abi::AbiInfo::GROUPS,
                tick_hz: 100,
            }
            .encode(&mut b)?;
            with_vm(|v| write(v, r, a[0], &b))?;
            Ok(64)
        }
        op::WRITE => {
            let size = usize::try_from(a[1]).map_err(|_| Status::InvalidArgument)?;
            if size > door_abi::MAX_DIAGNOSTIC {
                return Err(Status::InvalidArgument);
            }
            let mut b = [0u8; door_abi::MAX_DIAGNOSTIC];
            with_vm(|v| read(v, r, a[0], &mut b[..size]))?;
            let s = core::str::from_utf8(&b[..size]).map_err(|_| Status::InvalidArgument)?;
            if s.contains('\r') {
                return Err(Status::InvalidArgument);
            }
            with_meta(|m| m.executive.charge_diagnostic(r.pid, size))?;
            for line in s.split_terminator('\n') {
                log(format_args!("DOOR_USER pid={} {line}\n", r.pid.raw()));
            }
            Ok(size as u64)
        }
        op::TIME => Ok(run::now()),
        op::PRECISE_TIME => Ok(arch::precise_100ns()),
        op::MEMORY_STATUS => {
            if a[1..].iter().any(|v| *v != 0) {
                return Err(Status::InvalidArgument);
            }
            with_vm(|v| {
                let (managed, allocated, private) = v.monitor_memory();
                let mut out = [0u8; 32];
                out[..8].copy_from_slice(&(managed * 4096).to_le_bytes());
                out[8..16]
                    .copy_from_slice(&(managed.saturating_sub(allocated) * 4096).to_le_bytes());
                out[16..24].copy_from_slice(&private[r.pid.slot()].to_le_bytes());
                out[24..32].copy_from_slice(&door_kernel::mm::PAGE_POOL_BYTES.to_le_bytes());
                write(v, r, a[0], &out).map(|()| 0)
            })
        }
        op::WALL_TIME => {
            if a.iter().any(|v| *v != 0) {
                return Err(Status::InvalidArgument);
            }
            // The tick is read before the device guard: no scheduler lock
            // is taken while it is held.
            crate::platform_device::wall_time(run::now())
        }
        op::CLOSE => {
            with_meta(|m| m.executive.close(r.pid, a[0]))?;
            Ok(0)
        }
        op::DUPLICATE => with_meta(|m| m.executive.duplicate(r.pid, a[0], a[1])),
        op::INSPECT => {
            let info = with_meta(|m| m.executive.inspect(r.pid, a[0]))?;
            let mut b = [0; 32];
            door_abi::HandleInfo {
                kind: info.kind as u64,
                rights: info.rights,
                object_id: info.object,
                generation: info.generation,
            }
            .encode(&mut b)?;
            with_vm(|v| write(v, r, a[1], &b))?;
            Ok(0)
        }
        op::SECTION_CREATE => {
            let key = SECTION_KEY
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |x| x.checked_add(1))
                .map_err(|_| Status::ResourceExhausted)?;
            with_vm(|v| unsafe { v.create_section(key, a[0]) })?;
            let handle = with_meta(|m| m.executive.create_section(r.pid, key, a[0], a[1]));
            if handle.is_err() {
                with_vm(|v| unsafe { v.destroy_section(key) }).expect("section create rollback");
            }
            handle
        }
        op::SECTION_MAP => map_section(r, a),
        op::SECTION_UNMAP => {
            let index = with_meta(|m| {
                let index = m.views[r.pid.slot()]
                    .iter()
                    .position(|v| {
                        v.as_ref()
                            .is_some_and(|v| v.address == a[0] && v.bytes == a[1])
                    })
                    .ok_or(Status::NotFound)?;
                let view = m.views[r.pid.slot()][index].as_mut().unwrap();
                if view.busy {
                    return Err(Status::WouldBlock);
                }
                view.busy = true;
                Ok(index)
            })?;
            if let Err(error) = with_vm(|v| unsafe { v.unmap(r.pid.slot(), a[0], a[1]) }) {
                with_meta(|m| m.views[r.pid.slot()][index].as_mut().unwrap().busy = false);
                return Err(error);
            }
            with_meta(|m| {
                let view = m.views[r.pid.slot()][index].take().unwrap();
                m.executive
                    .release_lease(view.lease.expect("owned view lease"));
            });
            Ok(0)
        }
        op::PROCESS_WAIT if r.role >= 100 => platform::wait_process(r, a),
        op::PROCESS_WAIT => wait_process(r, a),
        op::CHANNEL_CREATE => with_vm(|v| {
            validate(v, r, a[0], 16)?;
            let pair = with_meta(|m| m.executive.channel_pair(r.pid, r.pid))?;
            let mut b = [0; 16];
            b[..8].copy_from_slice(&pair[0].to_le_bytes());
            b[8..].copy_from_slice(&pair[1].to_le_bytes());
            if let Err(error) = write(v, r, a[0], &b) {
                with_meta(|m| {
                    m.executive.close(r.pid, pair[0]).unwrap();
                    m.executive.close(r.pid, pair[1]).unwrap();
                });
                return Err(error);
            }
            Ok(0)
        }),
        op::CHANNEL_PEER => with_meta(|m| m.executive.channel_peer(r.pid, a[0])),
        op::CHANNEL_SEND => send(r, a, false),
        op::CHANNEL_SEND_DEADLINE => send(r, a, true),
        op::CHANNEL_CANCEL => with_meta(|m| m.executive.cancel(r.pid, a[0], a[1], arch::ticks(0))),
        op::CHANNEL_RECEIVE => receive(r, a),
        op::PROBE => {
            if r.role == 5 {
                Ok(door_abi::probe_checksum(a))
            } else {
                Err(Status::AccessDenied)
            }
        }
        _ => platform::dispatch(r, operation, a),
    }
}
fn map_section(r: Record, a: [u64; 6]) -> Result<u64, Status> {
    let (index, key, bytes) = with_meta(|m| {
        let index = m.views[r.pid.slot()]
            .iter()
            .position(Option::is_none)
            .ok_or(Status::ResourceExhausted)?;
        let section = m.executive.acquire_section(r.pid, a[0], a[2])?;
        m.views[r.pid.slot()][index] = Some(View {
            address: a[1],
            bytes: section.bytes,
            lease: Some(section.lease),
            busy: true,
        });
        Ok::<_, Status>((index, section.key, section.bytes))
    })?;
    if let Err(error) = with_vm(|v| unsafe { v.map_section(r.pid.slot(), key, a[1], a[2]) }) {
        with_meta(|m| {
            let view = m.views[r.pid.slot()][index].take().unwrap();
            m.executive.release_lease(view.lease.unwrap());
        });
        return Err(error);
    }
    with_meta(|m| {
        m.views[r.pid.slot()][index].as_mut().unwrap().busy = false;
        m.section_maps += 1;
    });
    Ok(bytes)
}
fn wait_process(r: Record, a: [u64; 6]) -> Result<u64, Status> {
    with_vm(|v| validate(v, r, a[1], 8))?;
    loop {
        let (target, code) = with_meta(|m| m.executive.wait_process(r.pid, a[0]))?;
        if let Some(code) = code {
            with_vm(|v| write(v, r, a[1], &code.to_le_bytes()))?;
            return Ok(0);
        }
        if target == r.pid.raw() {
            return Err(Status::InvalidArgument);
        }
        if a[2] <= run::now() {
            return Err(Status::Timeout);
        }
        let event = with_meta(|m| {
            m.records
                .iter()
                .flatten()
                .find(|r| r.pid.raw() == target)
                .map(|r| r.exited)
        })
        .ok_or(Status::NotFound)?;
        match run::wait(&[event], Some(a[2])).reason {
            WakeReason::Signaled => {}
            WakeReason::Timeout => return Err(Status::Timeout),
            WakeReason::Cancelled => return Err(Status::Canceled),
            WakeReason::Closed => return Err(Status::BadHandle),
        }
    }
}
fn send(r: Record, a: [u64; 6], timed: bool) -> Result<u64, Status> {
    if a[2] > door_abi::MAX_PACKET as u64
        || a[4] > door_abi::MAX_TRANSFERS as u64
        || (!timed && a[5] != 0)
    {
        return Err(Status::InvalidArgument);
    }
    let count = a[4] as usize;
    let mut packet = [0; door_abi::MAX_PACKET];
    let mut bytes = [0; door_abi::MAX_TRANSFERS * 32];
    with_vm(|v| {
        read(v, r, a[1], &mut packet[..a[2] as usize])?;
        if count > 0 {
            read(v, r, a[3], &mut bytes[..count * 32])?;
        }
        Ok::<(), Status>(())
    })?;
    let mut transfers = [door_abi::Transfer::default(); door_abi::MAX_TRANSFERS];
    for (i, t) in transfers.iter_mut().take(count).enumerate() {
        *t = door_abi::Transfer::decode(&bytes[i * 32..i * 32 + 32])?;
    }
    with_meta(|m| {
        m.executive.send_at(
            r.pid,
            a[0],
            &packet[..a[2] as usize],
            &transfers[..count],
            arch::ticks(0),
            timed.then_some(a[5]),
        )?;
        m.ipc_sends += 1;
        Ok::<(), Status>(())
    })?;
    Ok(0)
}
fn receive(r: Record, a: [u64; 6]) -> Result<u64, Status> {
    if a[2] > door_abi::MAX_PACKET as u64 || a[4] > door_abi::MAX_TRANSFERS as u64 {
        return Err(Status::InvalidArgument);
    }
    with_vm(|v| {
        // Hold authority from peek through validation and dequeue. A peer may
        // cancel the head concurrently, so separate lock acquisitions here would
        // validate one packet's sizes and then consume a different packet.
        let message = with_meta(|m| {
            let now = arch::ticks(0);
            let info = m.executive.receive_info_at(r.pid, a[0], now)?;
            if a[2] < (info.packet_bytes as u64) || a[4] < (info.transfers as u64) {
                return Err(Status::InvalidArgument);
            }
            let ranges = [
                (a[1], info.packet_bytes),
                (a[3], info.transfers * 32),
                (a[5], 32),
            ];
            for (i, (start, size)) in ranges.iter().copied().enumerate() {
                if size == 0 {
                    continue;
                }
                validate(v, r, start, size)?;
                for (other, len) in ranges[..i].iter().copied() {
                    if len > 0 && start < other + len as u64 && other < start + size as u64 {
                        return Err(Status::InvalidArgument);
                    }
                }
            }
            // The process VM guard pins all destinations through the atomic
            // dequeue/install and copy, including against other native threads.
            let msg = m
                .executive
                .receive_at(r.pid, a[0], a[2] as usize, a[4] as usize, now)?;
            m.ipc_receives += 1;
            Ok::<_, Status>(msg)
        })?;
        write(v, r, a[1], &message.packet[..message.length])?;
        let mut transfers = [0; door_abi::MAX_TRANSFERS * 32];
        let count = message.metadata.handle_count as usize;
        for (i, t) in message.transfers.iter().take(count).enumerate() {
            t.encode(&mut transfers[i * 32..i * 32 + 32])?;
        }
        if count > 0 {
            write(v, r, a[3], &transfers[..count * 32])?;
        }
        let mut metadata = [0; 32];
        message.metadata.encode(&mut metadata)?;
        write(v, r, a[5], &metadata)?;
        Ok(0)
    })
}

pub use fixtures::verify;
pub use platform::run_services;

pub(crate) fn current_kernel_stack() -> Option<(u64, u64)> {
    let task = run::current_optional()?;
    let (record, thread) = with_meta(|m| {
        Some((
            m.records[task.slot()]?,
            m.threads[task.slot()].as_ref().and_then(|t| t.memory),
        ))
    })?;
    with_vm(|v| v.kernel_stack(record.pid.slot(), thread))
}

/// A large heap teardown can retire tens of thousands of pages with IRQs off.
/// Preserve raw i8042 make/break bytes during that interval. This uses the same
/// allocation-free BSP producer as the timer, without acquiring another guard
/// or enabling interrupts while a mapping transaction is incomplete. One drain
/// per aligned MiB bounds port traffic; the consumer still owns all decoding.
fn capture_vm_input_progress(address: u64) {
    if address.is_multiple_of(1024 * 1024) && arch::current_cpu() == 0 {
        crate::platform_device::capture_display_input(arch::ticks(0));
    }
}

/// Initialize an ordinary single-CPU session without running qualification tasks.
pub fn initialize_session(foundations: Foundations<'static>) {
    let (frames, mut tables) = foundations.into_memory();
    fn retire(address: u64) {
        arch::shootdown(address).expect("translation retirement");
        capture_vm_input_progress(address);
    }
    unsafe {
        tables.install_retirement(retire);
        initialize_vm(frames, tables);
        arch::install_user_hooks(syscall, fault);
    }
    run::initialize(1);
}
pub use platform::session_result;
