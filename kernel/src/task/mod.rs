//! Bounded native scheduler metadata. The runtime holds one IRQ-safe SMP lock
//! across every operation and across wait registration through context saving.
//! No method switches stacks, blocks the host, allocates, or executes CPU I/O.
mod scheduler;
mod stop;
mod wait;

pub use scheduler::{Decision, ExitRecord, RetirementTicket, Scheduler, Stats, TaskSnapshot};
pub use stop::StopRequests;

/// Threads machine-wide (P18 M0h: 16 -> 128; a managed runtime starts about
/// ten per process). A task slot is a byte.
pub const MAX_TASKS: usize = 128;
pub const MAX_CPUS: usize = 4;
/// Native wait objects (events, mutexes, semaphores, conditions) machine-wide.
/// P18 raised 32 to 256, then (M0h, .NET and PowerShell) to 2048 (an object
/// slot is a u16); futex-style address waits need none.
pub const MAX_OBJECTS: usize = 2048;
pub const MAX_WAIT_OBJECTS: usize = 4;
pub const PRIORITY_LEVELS: u8 = 8;
pub const AGE_TICKS: u64 = 8;
/// Wake-sequence buckets of address waits (`Scheduler::wait_address`).
pub const ADDRESS_BUCKETS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskId {
    pub(super) slot: u8,
    pub(super) generation: u64,
}
impl TaskId {
    pub const fn slot(self) -> usize {
        self.slot as usize
    }
    pub const fn generation(self) -> u64 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectId {
    pub(super) slot: u16,
    pub(super) generation: u64,
}
impl ObjectId {
    pub const fn slot(self) -> usize {
        self.slot as usize
    }
    pub const fn generation(self) -> u64 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum State {
    New,
    Ready,
    Running,
    Blocked,
    Suspended,
    Exiting,
    Terminated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WakeReason {
    Signaled,
    Timeout,
    Cancelled,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WaitOutcome {
    pub reason: WakeReason,
    pub object: Option<ObjectId>,
    pub index: Option<u8>,
    pub abandoned: bool,
}
impl WaitOutcome {
    pub(super) const fn terminal(reason: WakeReason) -> Self {
        Self {
            reason,
            object: None,
            index: None,
            abandoned: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaitStatus {
    Completed(WaitOutcome),
    Blocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidCpu,
    InvalidTask,
    InvalidObject,
    InvalidState,
    InvalidContext,
    InvalidPriority,
    InvalidAffinity,
    Capacity,
    WrongObjectKind,
    InvalidCount,
    NotOwner,
    AlreadyOwned,
    Deadlock,
    EmptyWait,
    DuplicateWait,
    TooManyWaits,
    ClockReversed,
    RetirementPending,
    InvalidRetirement,
    NotRetired,
    Exhausted,
    Invariant,
}

#[derive(Clone, Copy)]
pub(super) struct WaitRegistration {
    objects: [Option<ObjectId>; MAX_WAIT_OBJECTS],
    count: u8,
    deadline: Option<u64>,
    sequence: u64,
    reacquire: Option<ObjectId>,
    completion: Option<WaitOutcome>,
    /// A futex-style wait on a user address (`wait_address`): the key
    /// names the process and address; no object is held.
    address: Option<u64>,
}
impl WaitRegistration {
    fn contains(self, object: ObjectId) -> bool {
        self.objects[..self.count as usize].contains(&Some(object))
    }
}

#[derive(Clone, Copy)]
pub(super) struct Task {
    id: TaskId,
    state: State,
    started: bool,
    suspended: bool,
    context: u64,
    base_priority: u8,
    effective_priority: u8,
    affinity: u8,
    current_cpu: Option<u8>,
    retiring_cpu: Option<u8>,
    queued_cpu: Option<u8>,
    last_cpu: Option<u8>,
    ready_since: u64,
    ready_sequence: u64,
    wait: Option<WaitRegistration>,
    wake: Option<WaitOutcome>,
    exit_result: Option<i64>,
    dispatches: u64,
    samples: u64,
    migrations: u64,
}

#[derive(Clone, Copy)]
pub(super) enum ObjectKind {
    Event {
        manual: bool,
        signaled: bool,
    },
    Semaphore {
        count: u32,
        maximum: u32,
    },
    Mutex {
        owner: Option<TaskId>,
        abandoned: bool,
    },
    Condition,
}
#[derive(Clone, Copy)]
pub(super) struct Object {
    id: ObjectId,
    kind: ObjectKind,
}

#[cfg(test)]
mod tests;
