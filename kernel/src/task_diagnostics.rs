//! Actual guest qualification of scheduling, wait races and shared translations.
//! These bounded fixtures use the same execution path as future native threads.
use crate::{arch, execution as run, startup::Foundations};
use core::{
    cell::UnsafeCell,
    fmt::Write,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use door_boot_debug::Serial;
use door_boot_protocol::BootInfo;
use door_kernel::{
    mm::{FrameAllocator, MappedRegion, PageCount, PageTables, Permissions, VirtualAddress},
    task::{ObjectId, State, TaskId, WakeReason},
};

const TASKS: usize = 12;
const ROUNDS: u64 = 64;
const TLB_BASE: u64 = 0xffff_ffff_d000_0000;
static READY: AtomicBool = AtomicBool::new(false);
static STOP: AtomicBool = AtomicBool::new(false);
static PARKED: AtomicU64 = AtomicU64::new(0);
static DATA: AtomicU64 = AtomicU64::new(0);
static RACE: AtomicU64 = AtomicU64::new(0);
static CONDITION: AtomicBool = AtomicBool::new(false);
static DONATION: AtomicBool = AtomicBool::new(false);
static CPU_VISITS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static TLS: [AtomicU64; TASKS] = [const { AtomicU64::new(0) }; TASKS];
static READ_EPOCH: AtomicU64 = AtomicU64::new(0);
static READ_ACK: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static READ_VALUE: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
#[derive(Clone, Copy)]
struct Fixtures {
    tasks: [TaskId; TASKS],
    online: u8,
    ping: ObjectId,
    ack: ObjectId,
    manual: ObjectId,
    cancel: ObjectId,
    close: ObjectId,
    never: ObjectId,
    condition: ObjectId,
    condition_mutex: ObjectId,
    condition_ready: ObjectId,
    abandoned_mutex: ObjectId,
    owned: ObjectId,
}
struct FixtureStorage(UnsafeCell<Option<Fixtures>>);
// Written once before READY's release, then immutable for the whole boot.
unsafe impl Sync for FixtureStorage {}
static FIXTURES: FixtureStorage = FixtureStorage(UnsafeCell::new(None));
fn fixtures() -> Fixtures {
    unsafe { (*FIXTURES.0.get()).expect("published fixtures") }
}
fn pattern(index: usize) -> u64 {
    0x0123_4567_89ab_cd00 | index as u64
}
fn checked_wait(object: ObjectId) {
    assert_eq!(
        run::wait(&[object], Some(run::now() + 400)).reason,
        WakeReason::Signaled
    );
}
fn until(mut done: impl FnMut() -> bool) {
    let deadline = run::now() + 800;
    while !done() {
        assert!(run::now() < deadline, "guest worker progress timeout");
        arch::yield_now();
    }
}
extern "C" fn worker(argument: u64) -> ! {
    let index = argument as usize;
    let f = fixtures();
    assert_eq!(run::current(), f.tasks[index]);
    match index {
        0..=3 => {
            for round in 0..4 {
                let id = run::current();
                let counter = run::preemptions(id);
                let target = counter.load(Ordering::Acquire) + 2;
                assert!(
                    unsafe { arch::state_probe(pattern(index), counter, target) },
                    "preempted architectural state"
                );
                CPU_VISITS[index].fetch_or(1 << arch::current_cpu(), Ordering::AcqRel);
                let next = ((round + index + 1) % usize::from(f.online)) as u8;
                run::with(|s| s.set_affinity(id, 1 << next).unwrap());
                arch::yield_now();
                assert_eq!(arch::current_cpu(), usize::from(next), "affinity migration");
                CPU_VISITS[index].fetch_or(1 << next, Ordering::AcqRel);
            }
        }
        4 => {
            for value in 1..=ROUNDS {
                checked_wait(f.ack);
                DATA.store(value, Ordering::Release);
                run::with(|s| s.signal_event(f.ping).unwrap());
            }
            checked_wait(f.ack);
        }
        5 => {
            for value in 1..=ROUNDS {
                checked_wait(f.ping);
                assert_eq!(DATA.load(Ordering::Acquire), value);
                run::with(|s| s.release_semaphore(f.ack, 1).unwrap());
            }
        }
        6 => {
            // A manual event remains signaled, including signal-before-wait.
            checked_wait(f.manual);
            checked_wait(f.manual);
            RACE.store(1, Ordering::Release);
            assert_eq!(run::wait(&[f.cancel], None).reason, WakeReason::Cancelled);
            RACE.store(2, Ordering::Release);
            assert_eq!(run::wait(&[f.close], None).reason, WakeReason::Closed);
            RACE.store(3, Ordering::Release);
            assert_eq!(
                run::wait(&[f.never], Some(run::now() + 3)).reason,
                WakeReason::Timeout
            );
            RACE.store(4, Ordering::Release);
        }
        7 => {
            for stage in 1..=2 {
                until(|| {
                    RACE.load(Ordering::Acquire) == stage
                        && run::with(|s| s.snapshot(f.tasks[6]).unwrap().state == State::Blocked)
                });
                run::with(|s| {
                    if stage == 1 {
                        assert!(s.cancel_wait(f.tasks[6]).unwrap());
                    } else {
                        s.close_object(f.close).unwrap();
                    }
                });
            }
        }
        8 => {
            checked_wait(f.condition_mutex);
            run::with(|s| s.signal_event(f.condition_ready).unwrap());
            while !CONDITION.load(Ordering::Acquire) {
                assert_eq!(
                    run::condition_wait(f.condition, f.condition_mutex, Some(run::now() + 400))
                        .reason,
                    WakeReason::Signaled
                );
                assert_eq!(
                    run::with(|s| s.mutex_owner(f.condition_mutex).unwrap()),
                    Some(f.tasks[8])
                );
            }
            run::with(|s| {
                s.unlock_mutex(arch::current_cpu() as u8, f.condition_mutex)
                    .unwrap()
            });
        }
        9 => {
            checked_wait(f.condition_ready);
            checked_wait(f.condition_mutex);
            CONDITION.store(true, Ordering::Release);
            run::with(|s| {
                s.signal_condition(f.condition, false).unwrap();
                s.unlock_mutex(arch::current_cpu() as u8, f.condition_mutex)
                    .unwrap();
            });
        }
        10 => {
            checked_wait(f.abandoned_mutex);
            run::with(|s| s.signal_event(f.owned).unwrap());
            until(|| run::with(|s| s.snapshot(f.tasks[10]).unwrap().effective_priority == 7));
            DONATION.store(true, Ordering::Release);
            // Exit holds this mutex intentionally; retirement transfers an
            // explicit abandoned result to the high-priority waiter.
            run::exit(17);
        }
        11 => {
            checked_wait(f.owned);
            let outcome = run::wait(&[f.abandoned_mutex], Some(run::now() + 400));
            assert_eq!(outcome.reason, WakeReason::Signaled);
            assert!(outcome.abandoned);
            run::with(|s| {
                s.unlock_mutex(arch::current_cpu() as u8, f.abandoned_mutex)
                    .unwrap()
            });
        }
        _ => unreachable!(),
    }
    run::exit(0)
}

fn observe_mapping(cpu: usize) {
    let epoch = READ_EPOCH.load(Ordering::Acquire);
    if epoch != 0 && READ_ACK[cpu].load(Ordering::Relaxed) != epoch {
        // Publisher keeps the map live until every CPU acknowledges its sole
        // read. No access is repeated until a later epoch publishes a new map.
        let value = unsafe { (TLB_BASE as *const u64).read_volatile() };
        READ_VALUE[cpu].store(value, Ordering::Relaxed);
        READ_ACK[cpu].store(epoch, Ordering::Release);
    }
}
extern "C" fn secondary(cpu: usize) -> ! {
    arch::enable_interrupts();
    while !READY.load(Ordering::Acquire) {
        core::hint::spin_loop();
    }
    arch::start_timer().expect("secondary periodic timer");
    arch::yield_now();
    loop {
        observe_mapping(cpu);
        if STOP.load(Ordering::Acquire) {
            arch::stop_timer();
            PARKED.fetch_or(1 << cpu, Ordering::Release);
            // Retained AP idle/emergency stacks remain live. This CPU can still
            // acknowledge retirement until the enclosing QEMU instance exits.
            loop {
                arch::wait_for_interrupt();
            }
        }
        arch::wait_for_interrupt();
    }
}
fn stack(frames: &mut FrameAllocator<'_>, tables: &mut PageTables<'_>, base: u64) -> MappedRegion {
    let address = VirtualAddress::new(base).unwrap();
    assert!(
        !tables
            .is_mapped(VirtualAddress::new(base - 4096).unwrap())
            .unwrap()
    );
    assert!(
        !tables
            .is_mapped(VirtualAddress::new(base + 65536).unwrap())
            .unwrap()
    );
    let backing = frames
        .allocate(PageCount::new(16).unwrap(), 1)
        .expect("guarded stack backing");
    unsafe { tables.map_frames(backing, address, Permissions::ReadWrite) }.expect("guarded stack")
}
fn retire(address: u64) {
    arch::shootdown(address).expect("synchronous translation retirement");
}

pub fn verify(info: &BootInfo<'_>, foundations: &mut Foundations<'_>, degraded: bool) {
    arch::disable_interrupts();
    let platform = info.validate_platform().unwrap();
    let startup = info.validate_startup().unwrap();
    let (frames, tables) = foundations.memory();
    let mut aps = [arch::ApConfig::default(); 4];
    let mut retained: [Option<MappedRegion>; 15] = core::array::from_fn(|_| None);
    for (cpu, config) in aps
        .iter_mut()
        .enumerate()
        .take(usize::from(platform.processor_count))
        .skip(1)
    {
        for which in 0..5 {
            let slot = (cpu - 1) * 5 + which;
            let mapping = stack(
                frames,
                tables,
                0xffff_ffff_a400_1000 + slot as u64 * 0x12000,
            );
            if which == 0 {
                config.stack_top = mapping.end().get();
            } else {
                config.ist_tops[which - 1] = mapping.end().get();
            }
            retained[slot] = Some(mapping);
        }
    }
    let stacks: [MappedRegion; TASKS] = core::array::from_fn(|index| {
        stack(
            frames,
            tables,
            0xffff_ffff_c000_1000 + index as u64 * 0x12000,
        )
    });
    unsafe {
        arch::prepare_secondary_cpus(&platform, &startup, &aps, secondary)
            .expect("prepare AP startup");
        tables
            .seal_startup_code(info)
            .expect("seal AP executable page");
        tables.install_retirement(retire);
    }
    let started = unsafe { arch::start_secondary_cpus(degraded) }.expect("secondary CPU startup");
    let online = started.online_mask.count_ones() as u8;
    assert_eq!(
        started.requested_mask.count_ones(),
        u32::from(platform.processor_count)
    );
    assert_eq!(
        started.online_mask,
        (1 << online) - 1,
        "only contiguous online CPU slots supported"
    );
    if degraded {
        assert_eq!(started.missing_mask.count_ones(), 1);
    } else {
        assert_eq!(started.missing_mask, 0, "missing secondary CPU");
    }
    // The runtime scheduler, built in place (too large for the boot stack);
    // the fixtures are created under its lock before the timer starts.
    run::initialize(online);
    let tasks = run::with(|scheduler| {
        let tasks = core::array::from_fn(|index| {
            TLS[index].store(pattern(index), Ordering::Release);
            let context = unsafe {
                arch::create_context(
                    stacks[index].end().get(),
                    worker,
                    index as u64,
                    &TLS[index] as *const AtomicU64 as u64,
                )
            }
            .unwrap();
            let priority = match index {
                10 => 1,
                11 => 7,
                _ => 3,
            };
            let id = scheduler
                .create(context as u64, priority, (1 << online) - 1)
                .unwrap();
            scheduler.start(id).unwrap();
            id
        });
        let fixtures = Fixtures {
            tasks,
            online,
            ping: scheduler.create_event(false, false).unwrap(),
            ack: scheduler.create_semaphore(1, 1).unwrap(),
            manual: scheduler.create_event(true, true).unwrap(),
            cancel: scheduler.create_event(false, false).unwrap(),
            close: scheduler.create_event(false, false).unwrap(),
            never: scheduler.create_event(false, false).unwrap(),
            condition: scheduler.create_condition().unwrap(),
            condition_mutex: scheduler.create_mutex().unwrap(),
            condition_ready: scheduler.create_event(false, false).unwrap(),
            abandoned_mutex: scheduler.create_mutex().unwrap(),
            owned: scheduler.create_event(false, false).unwrap(),
        };
        scheduler.validate().expect("initial scheduler invariants");
        unsafe {
            *FIXTURES.0.get() = Some(fixtures);
        }
        tasks
    });
    READY.store(true, Ordering::Release);
    arch::start_timer().expect("BSP periodic timer");
    arch::yield_now();
    // CPU0 resumes this idle context whenever it has no runnable managed task.
    let deadline = arch::ticks(0) + 1200;
    loop {
        let finished = run::with(|s| {
            s.validate().expect("concurrent scheduler invariants");
            tasks
                .iter()
                .all(|id| s.snapshot(*id).unwrap().state == State::Terminated)
        });
        if finished {
            break;
        }
        assert!(arch::ticks(0) < deadline, "scheduler guest timeout");
        arch::wait_for_interrupt();
    }
    assert_eq!(RACE.load(Ordering::Acquire), 4);
    assert!(CONDITION.load(Ordering::Acquire) && DONATION.load(Ordering::Acquire));
    for (index, id) in tasks.iter().enumerate() {
        if index < 4 {
            assert!(run::preemptions(*id).load(Ordering::Acquire) >= 16);
            assert_eq!(
                CPU_VISITS[index].load(Ordering::Acquire),
                u64::from(started.online_mask)
            );
        }
        run::with(|s| {
            assert_eq!(
                s.exit_result(*id).unwrap(),
                Some(if index == 10 { 17 } else { 0 })
            );
            s.reap(*id).unwrap();
            assert!(s.snapshot(*id).is_err(), "stale task identity");
        });
    }
    tlb_stress(frames, tables, online);
    STOP.store(true, Ordering::Release);
    for cpu in 1..usize::from(online) {
        arch::send_reschedule(cpu).expect("AP idle wake");
    }
    while PARKED.load(Ordering::Acquire) != u64::from(started.online_mask & !1) {
        assert!(arch::ticks(0) < deadline, "AP shutdown timeout");
        arch::wait_for_interrupt();
    }
    arch::disable_interrupts();
    arch::stop_timer();
    for mapping in stacks {
        let backing = unsafe { tables.unmap(mapping) }.unwrap().unwrap();
        frames.free(backing).unwrap();
    }
    let stats = run::with(|s| {
        s.validate().unwrap();
        s.stats()
    });
    if online > 1 {
        assert!(stats.migrations > 0);
    }
    let mut serial = unsafe { Serial::new() };
    writeln!(
        serial,
        "DOOR_P04 TASKS PASS threads={TASKS} switches={} migrations={} retirements={}",
        stats.switches, stats.migrations, stats.retirements
    )
    .ok();
    writeln!(serial,"DOOR_P04 WAITS PASS ping_rounds={ROUNDS} completed={} timeouts={} donation=1 abandoned=1 condition=1 cancel=1 close=1",
        stats.completed_waits,stats.timer_wakes).ok();
    writeln!(
        serial,
        "DOOR_P04 SMP PASS discovered={} online={online} degraded={}",
        platform.processor_count,
        u8::from(degraded)
    )
    .ok();
    writeln!(serial, "DOOR_P04 TLB PASS cpus={online} rounds=32").ok();
    // AP stack tokens remain retained because parked CPUs still use those stacks.
}

fn tlb_stress(frames: &mut FrameAllocator<'_>, tables: &mut PageTables<'_>, online: u8) {
    arch::disable_interrupts();
    let mut next = frames.allocate(PageCount::new(1).unwrap(), 1).unwrap();
    let mut spare = frames.allocate(PageCount::new(1).unwrap(), 1).unwrap();
    assert_ne!(next.start(), spare.start());
    for epoch in 1..=32 {
        let mut mapping = unsafe {
            tables.map_frames(
                next,
                VirtualAddress::new(TLB_BASE).unwrap(),
                Permissions::ReadWrite,
            )
        }
        .unwrap();
        let expected = epoch * 0x0101_0101_0101_0101;
        unsafe {
            (TLB_BASE as *mut u64).write_volatile(expected);
        }
        unsafe { tables.protect(&mut mapping, Permissions::ReadOnly) }.unwrap();
        READ_EPOCH.store(epoch, Ordering::Release);
        observe_mapping(0);
        for cpu in 1..usize::from(online) {
            arch::send_reschedule(cpu).unwrap();
        }
        let deadline = arch::ticks(0) + 100;
        while READ_ACK[..usize::from(online)]
            .iter()
            .any(|ack| ack.load(Ordering::Acquire) != epoch)
        {
            assert!(
                arch::ticks(0) < deadline,
                "remote mapping observation timeout"
            );
            arch::wait_for_interrupt();
        }
        arch::disable_interrupts();
        for value in &READ_VALUE[..usize::from(online)] {
            assert_eq!(value.load(Ordering::Acquire), expected);
        }
        READ_EPOCH.store(0, Ordering::Release);
        let retired = unsafe { tables.unmap(mapping) }.unwrap().unwrap();
        next = spare;
        spare = retired;
    }
    frames.free(next).unwrap();
    frames.free(spare).unwrap();
}
