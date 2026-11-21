//! Sanitized observational snapshot; never grants process or memory authority.
use super::*;
use door_abi::telemetry::{BYTES, Process as PublicProcess, Snapshot};
const _: () = assert!(MAX_PROCESSES == door_abi::telemetry::MAX_PROCESSES);
pub(super) fn snapshot(r: Record, a: [u64; 6]) -> Result<u64, Status> {
    if a[1] != BYTES as u64 && a[1] != door_abi::telemetry::LEGACY_BYTES as u64 {
        return Err(Status::InvalidArgument);
    }
    let caller = with_meta(|m| m.executive.authority(r.pid))?;
    if caller.manifest != abi::manifest::MONITOR || caller.session == 0 {
        return Err(Status::AccessDenied);
    }
    let identities = with_meta(|m| {
        core::array::from_fn::<_, MAX_PROCESSES, _>(|i| {
            m.processes[i]
                .as_ref()
                .filter(|p| {
                    // Session 0's system processes (Init, the core services,
                    // the window server, and since P15 the network and audio
                    // services) and the caller's own session.
                    !p.starting
                        && (p.identity.session == 0
                            && matches!(
                                p.identity.manifest,
                                1..=4
                                    | abi::manifest::WINDOW
                                    | abi::manifest::NETWORK
                                    | abi::manifest::AUDIO
                            )
                            || p.identity.uid == caller.uid
                                && p.identity.session == caller.session
                                && p.identity.epoch == caller.epoch)
                })
                .map(|p| (p.pid, p.identity, p.mode, p.stopping.is_some()))
        })
    });
    let tasks = with_meta(|m| m.records);
    let (managed, allocated, memory, x86) = with_vm(|v| {
        let (managed, allocated, memory) = v.monitor_memory();
        let x86: [bool; MAX_PROCESSES] = core::array::from_fn(|i| v.windows_x86(i));
        (managed, allocated, memory, x86)
    });
    let mut records = [PublicProcess::default(); MAX_PROCESSES];
    let (ticks, cpus, stats) = run::with(|scheduler| {
        for (i, identity) in identities.iter().enumerate() {
            let Some((pid, id, mode, stopping)) = identity else {
                continue;
            };
            let mut p = PublicProcess {
                pid: pid.raw(),
                instance: id.instance,
                manifest: id.manifest as u32,
                uid: id.uid,
                mode: *mode as u32
                    | if x86[i] {
                        door_abi::telemetry::MODE_X86
                    } else {
                        0
                    },
                state: if *stopping { 5 } else { 3 },
                private_bytes: memory[i],
                ..PublicProcess::default()
            };
            for record in tasks.iter().flatten().filter(|t| t.pid == *pid) {
                if let Ok(task) = scheduler.snapshot(record.task) {
                    p.threads += 1;
                    p.samples = p.samples.saturating_add(task.samples);
                    let state = match task.state {
                        door_kernel::task::State::New => 0,
                        door_kernel::task::State::Ready => 1,
                        door_kernel::task::State::Running => 2,
                        door_kernel::task::State::Blocked => 3,
                        door_kernel::task::State::Suspended => 4,
                        door_kernel::task::State::Exiting => 5,
                        door_kernel::task::State::Terminated => 6,
                    };
                    if state == 2 || p.threads == 1 {
                        p.state = state;
                    }
                }
            }
            records[i] = p;
        }
        (scheduler.now(), scheduler.online_cpus(), scheduler.stats())
    });
    let mut output = Snapshot {
        ticks,
        cpus: cpus as u32,
        total_samples: stats.total_samples,
        busy_samples: stats.busy_samples,
        managed_pages: managed,
        allocated_pages: allocated,
        ..Snapshot::default()
    };
    // Do not associate memory measured at an old slot with a replacement PID.
    // Metadata and VM locks are deliberately never held together.
    with_meta(|m| {
        for (slot, p) in records.iter().enumerate() {
            if p.threads != 0
                && m.processes[slot].as_ref().is_some_and(|live| {
                    live.pid.raw() == p.pid
                        && live.identity.instance == p.instance
                        && !live.starting
                })
            {
                output.processes[output.count] = *p;
                output.count += 1;
            }
        }
    });
    (output.reads, output.writes, output.flushes) = crate::platform_device::monitor_io()?;
    let mut bytes = output.encode()?;
    let length = a[1] as usize;
    if length == door_abi::telemetry::LEGACY_BYTES {
        bytes[..4].copy_from_slice(&1u32.to_le_bytes());
        bytes[4..8].copy_from_slice(&(output.count.min(8) as u32).to_le_bytes());
    }
    let bytes = &bytes[..length];
    with_vm(|v| {
        v.validate_private_output(r.pid.slot(), a[0], bytes.len())?;
        write(v, r, a[0], bytes)
    })?;
    Ok(bytes.len() as u64)
}
