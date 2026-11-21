use super::*;

#[derive(Clone, Copy)]
struct RunQueue {
    entries: [Option<TaskId>; MAX_TASKS],
}
impl RunQueue {
    const EMPTY: Self = Self {
        entries: [None; MAX_TASKS],
    };
    fn len(&self) -> usize {
        self.entries.iter().flatten().count()
    }
}

/// Runtime acknowledges this only after changing stacks and ceasing all access
/// to the outgoing context. Selecting a different task alone is insufficient.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetirementTicket {
    pub cpu: u8,
    pub epoch: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Decision {
    pub previous: Option<TaskId>,
    pub next: Option<TaskId>,
    pub context: Option<u64>,
    pub retirement: Option<RetirementTicket>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExitRecord {
    pub result: i64,
    pub context: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskSnapshot {
    pub id: TaskId,
    pub state: State,
    pub context: u64,
    pub base_priority: u8,
    pub effective_priority: u8,
    pub affinity: u8,
    pub current_cpu: Option<u8>,
    pub queued_cpu: Option<u8>,
    pub retiring_cpu: Option<u8>,
    pub dispatches: u64,
    pub samples: u64,
    pub migrations: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Stats {
    pub total_samples: u64,
    pub busy_samples: u64,
    pub dispatches: u64,
    pub switches: u64,
    pub migrations: u64,
    pub completed_waits: u64,
    pub timer_wakes: u64,
    pub retirements: u64,
}

/// One caller-owned IRQ-safe lock serializes this complete state. Priorities are
/// native 0..7 (7 highest); a bounded scan picks the priority/FIFO head of each
/// per-CPU ready queue. No node can belong to two queues or be queued while live
/// in a CPU context. Aging raises a waiting task one level per AGE_TICKS.
pub struct Scheduler {
    pub(super) tasks: [Option<Task>; MAX_TASKS],
    task_generations: [u64; MAX_TASKS],
    pub(super) objects: [Option<Object>; MAX_OBJECTS],
    pub(super) object_generations: [u64; MAX_OBJECTS],
    /// Wake sequences of address waits, one per bucket.
    pub(super) address_sequences: [u64; ADDRESS_BUCKETS],
    queues: [RunQueue; MAX_CPUS],
    current: [Option<TaskId>; MAX_CPUS],
    pending: [Option<(TaskId, RetirementTicket)>; MAX_CPUS],
    epochs: [u64; MAX_CPUS],
    online: u8,
    pub(super) now: u64,
    sequence: u64,
    reschedule: u8,
    pub(super) stats: Stats,
}
impl Scheduler {
    pub fn new(online_cpus: u8) -> Result<Self, Error> {
        if !(1..=MAX_CPUS as u8).contains(&online_cpus) {
            return Err(Error::InvalidCpu);
        }
        Ok(Self {
            tasks: [None; MAX_TASKS],
            task_generations: [0; MAX_TASKS],
            objects: [None; MAX_OBJECTS],
            object_generations: [0; MAX_OBJECTS],
            address_sequences: [0; ADDRESS_BUCKETS],
            queues: [RunQueue::EMPTY; MAX_CPUS],
            current: [None; MAX_CPUS],
            pending: [None; MAX_CPUS],
            epochs: [0; MAX_CPUS],
            online: online_cpus,
            now: 0,
            sequence: 0,
            reschedule: 0,
            stats: Stats::default(),
        })
    }
    /// `new` in place: with 128 tasks the scheduler is tens of KiB, too
    /// large to build and move on the boot stack.
    ///
    /// # Safety
    /// `at` is valid, aligned storage owned by the caller; it is
    /// initialized only when this returns Ok.
    pub unsafe fn initialize_at(at: *mut Self, online_cpus: u8) -> Result<(), Error> {
        use core::ptr::addr_of_mut;
        if !(1..=MAX_CPUS as u8).contains(&online_cpus) {
            return Err(Error::InvalidCpu);
        }
        unsafe {
            for i in 0..MAX_TASKS {
                addr_of_mut!((*at).tasks)
                    .cast::<Option<Task>>()
                    .add(i)
                    .write(None);
                addr_of_mut!((*at).task_generations)
                    .cast::<u64>()
                    .add(i)
                    .write(0);
            }
            for i in 0..MAX_OBJECTS {
                addr_of_mut!((*at).objects)
                    .cast::<Option<Object>>()
                    .add(i)
                    .write(None);
                addr_of_mut!((*at).object_generations)
                    .cast::<u64>()
                    .add(i)
                    .write(0);
            }
            addr_of_mut!((*at).address_sequences).write([0; ADDRESS_BUCKETS]);
            for cpu in 0..MAX_CPUS {
                let queue = addr_of_mut!((*at).queues).cast::<RunQueue>().add(cpu);
                for i in 0..MAX_TASKS {
                    addr_of_mut!((*queue).entries)
                        .cast::<Option<TaskId>>()
                        .add(i)
                        .write(None);
                }
            }
            addr_of_mut!((*at).current).write([None; MAX_CPUS]);
            addr_of_mut!((*at).pending).write([None; MAX_CPUS]);
            addr_of_mut!((*at).epochs).write([0; MAX_CPUS]);
            addr_of_mut!((*at).online).write(online_cpus);
            addr_of_mut!((*at).now).write(0);
            addr_of_mut!((*at).sequence).write(0);
            addr_of_mut!((*at).reschedule).write(0);
            addr_of_mut!((*at).stats).write(Stats::default());
        }
        Ok(())
    }
    pub const fn online_cpus(&self) -> u8 {
        self.online
    }
    pub const fn now(&self) -> u64 {
        self.now
    }
    pub const fn stats(&self) -> Stats {
        self.stats
    }
    pub const fn reschedule_mask(&self) -> u8 {
        self.reschedule
    }
    pub fn current(&self, cpu: u8) -> Result<Option<TaskId>, Error> {
        self.cpu(cpu)?;
        Ok(self.current[cpu as usize])
    }
    pub fn create(&mut self, context: u64, priority: u8, affinity: u8) -> Result<TaskId, Error> {
        Self::context_valid(context)?;
        if priority >= PRIORITY_LEVELS {
            return Err(Error::InvalidPriority);
        }
        self.affinity_valid(affinity)?;
        let slot = (0..MAX_TASKS)
            .find(|&slot| self.tasks[slot].is_none() && self.task_generations[slot] != u64::MAX)
            .ok_or(Error::Capacity)?;
        let generation = self.task_generations[slot] + 1;
        let id = TaskId {
            slot: slot as u8,
            generation,
        };
        self.task_generations[slot] = generation;
        self.tasks[slot] = Some(Task {
            id,
            state: State::New,
            started: false,
            suspended: false,
            context,
            base_priority: priority,
            effective_priority: priority,
            affinity,
            current_cpu: None,
            retiring_cpu: None,
            queued_cpu: None,
            last_cpu: None,
            ready_since: self.now,
            ready_sequence: 0,
            wait: None,
            wake: None,
            exit_result: None,
            dispatches: 0,
            samples: 0,
            migrations: 0,
        });
        Ok(id)
    }
    pub fn start(&mut self, id: TaskId) -> Result<(), Error> {
        let mut task = self.task(id)?;
        if task.started || !matches!(task.state, State::New | State::Suspended) {
            return Err(Error::InvalidState);
        }
        task.started = true;
        task.state = if task.suspended {
            State::Suspended
        } else {
            State::Ready
        };
        self.put(task);
        if !task.suspended {
            self.enqueue(id)?;
        }
        Ok(())
    }
    pub fn snapshot(&self, id: TaskId) -> Result<TaskSnapshot, Error> {
        let task = self.task(id)?;
        Ok(TaskSnapshot {
            id,
            state: task.state,
            context: task.context,
            base_priority: task.base_priority,
            effective_priority: task.effective_priority,
            affinity: task.affinity,
            current_cpu: task.current_cpu,
            queued_cpu: task.queued_cpu,
            retiring_cpu: task.retiring_cpu,
            dispatches: task.dispatches,
            samples: task.samples,
            migrations: task.migrations,
        })
    }
    /// Save the outgoing context and choose the next. The runtime retains the
    /// scheduling lock through its architecture switch and retirement acknowledgement.
    /// A CPU without a managed current task passes zero for saved_context.
    pub fn schedule(&mut self, cpu: u8, saved_context: u64) -> Result<Decision, Error> {
        self.cpu(cpu)?;
        if self.pending[cpu as usize].is_some() {
            return Err(Error::RetirementPending);
        }
        let previous = self.current[cpu as usize];
        if previous.is_some() {
            Self::context_valid(saved_context)?;
        }
        self.recompute_priorities();
        let candidate = self.best_ready(cpu);
        if let Some(id) = previous {
            let mut old = self.task(id)?;
            let runnable = !old.suspended
                && (old.state == State::Running
                    || (old.state == State::Blocked && old.wait.is_none()));
            let may_continue = runnable && old.affinity & (1 << cpu) != 0;
            if may_continue
                && candidate.is_none_or(|next| self.ready_score(next) < old.effective_priority)
            {
                old.context = saved_context;
                old.state = State::Running;
                self.put(old);
                self.reschedule &= !(1 << cpu);
                return Ok(Decision {
                    previous,
                    next: previous,
                    context: Some(saved_context),
                    retirement: None,
                });
            }
            let epoch = self.epochs[cpu as usize]
                .checked_add(1)
                .ok_or(Error::Exhausted)?;
            let ticket = RetirementTicket { cpu, epoch };
            old.context = saved_context;
            old.current_cpu = None;
            old.retiring_cpu = Some(cpu);
            if old.state == State::Running || (old.state == State::Blocked && old.wait.is_none()) {
                old.state = if old.suspended {
                    State::Suspended
                } else {
                    State::Ready
                };
            }
            self.put(old);
            self.current[cpu as usize] = None;
            self.epochs[cpu as usize] = epoch;
            self.pending[cpu as usize] = Some((id, ticket));
        }
        let next = if let Some(id) = candidate {
            self.dispatch(cpu, id)?;
            Some(id)
        } else {
            None
        };
        self.reschedule &= !(1 << cpu);
        if previous != next {
            self.stats.switches = self.stats.switches.saturating_add(1);
        }
        Ok(Decision {
            previous,
            next,
            context: next.map(|id| self.tasks[id.slot()].expect("selected task").context),
            retirement: self.pending[cpu as usize].map(|(_, ticket)| ticket),
        })
    }
    /// Runtime calls after the outgoing stack/context is no longer accessed. It
    /// must still hold the same scheduling lock. Only then can another CPU run
    /// the saved context, or a terminated task be reaped.
    pub fn acknowledge_retirement(&mut self, ticket: RetirementTicket) -> Result<(), Error> {
        self.cpu(ticket.cpu)?;
        let (id, expected) = self.pending[ticket.cpu as usize].ok_or(Error::InvalidRetirement)?;
        if ticket != expected {
            return Err(Error::InvalidRetirement);
        }
        let mut task = self.task(id)?;
        if task.retiring_cpu != Some(ticket.cpu)
            || task.current_cpu.is_some()
            || task.queued_cpu.is_some()
        {
            return Err(Error::Invariant);
        }
        task.retiring_cpu = None;
        self.pending[ticket.cpu as usize] = None;
        self.stats.retirements = self.stats.retirements.saturating_add(1);
        self.put(task);
        if task.state == State::Exiting {
            self.finish_exit(id)?;
        } else if !task.suspended && task.wait.is_none() && task.started {
            self.tasks[id.slot()].as_mut().expect("live").state = State::Ready;
            self.enqueue(id)?;
        }
        Ok(())
    }
    pub fn set_affinity(&mut self, id: TaskId, affinity: u8) -> Result<(), Error> {
        self.affinity_valid(affinity)?;
        let mut task = self.task(id)?;
        if matches!(task.state, State::Exiting | State::Terminated) {
            return Err(Error::InvalidState);
        }
        let queued = task.queued_cpu;
        if queued.is_some() {
            self.dequeue(id)?;
            task.queued_cpu = None;
        }
        task.affinity = affinity;
        if let Some(cpu) = task.current_cpu {
            self.reschedule |= 1 << cpu;
        }
        self.put(task);
        if queued.is_some() {
            self.enqueue(id)?;
        }
        Ok(())
    }
    pub fn set_priority(&mut self, id: TaskId, priority: u8) -> Result<(), Error> {
        if priority >= PRIORITY_LEVELS {
            return Err(Error::InvalidPriority);
        }
        let mut task = self.task(id)?;
        if matches!(task.state, State::Exiting | State::Terminated) {
            return Err(Error::InvalidState);
        }
        task.base_priority = priority;
        self.put(task);
        self.recompute_priorities();
        self.reschedule |= (1 << self.online) - 1;
        Ok(())
    }
    pub fn suspend(&mut self, id: TaskId) -> Result<(), Error> {
        let mut task = self.task(id)?;
        if task.suspended || matches!(task.state, State::Exiting | State::Terminated) {
            return Err(Error::InvalidState);
        }
        if task.queued_cpu.is_some() {
            self.dequeue(id)?;
            task.queued_cpu = None;
        }
        task.suspended = true;
        task.state = State::Suspended;
        if let Some(cpu) = task.current_cpu {
            self.reschedule |= 1 << cpu;
        }
        self.put(task);
        Ok(())
    }
    pub fn resume(&mut self, id: TaskId) -> Result<(), Error> {
        let mut task = self.task(id)?;
        if !task.suspended || matches!(task.state, State::Exiting | State::Terminated) {
            return Err(Error::InvalidState);
        }
        task.suspended = false;
        task.state = if !task.started {
            State::New
        } else if task.wait.is_some() {
            State::Blocked
        } else if task.current_cpu.is_some() {
            State::Running
        } else {
            State::Ready
        };
        self.put(task);
        if task.state == State::Ready && task.retiring_cpu.is_none() {
            self.enqueue(id)?;
        }
        Ok(())
    }
    pub fn exit_current(&mut self, cpu: u8, result: i64) -> Result<(), Error> {
        let id = self.current(cpu)?.ok_or(Error::InvalidState)?;
        self.terminate(id, result)
    }
    pub fn terminate(&mut self, id: TaskId, result: i64) -> Result<(), Error> {
        let mut task = self.task(id)?;
        if matches!(task.state, State::Exiting | State::Terminated) {
            return Err(Error::InvalidState);
        }
        if task.queued_cpu.is_some() {
            self.dequeue(id)?;
            task.queued_cpu = None;
        }
        task.wait = None;
        task.wake = None;
        task.exit_result = Some(result);
        task.state = State::Exiting;
        if let Some(cpu) = task.current_cpu {
            self.reschedule |= 1 << cpu;
        }
        self.put(task);
        if task.current_cpu.is_none() && task.retiring_cpu.is_none() {
            self.finish_exit(id)?;
        }
        self.recompute_priorities();
        Ok(())
    }
    pub fn exit_result(&self, id: TaskId) -> Result<Option<i64>, Error> {
        let task = self.task(id)?;
        Ok(if task.state == State::Terminated {
            task.exit_result
        } else {
            None
        })
    }
    pub fn reap(&mut self, id: TaskId) -> Result<ExitRecord, Error> {
        let task = self.task(id)?;
        if task.state != State::Terminated
            || task.current_cpu.is_some()
            || task.retiring_cpu.is_some()
        {
            return Err(Error::NotRetired);
        }
        let result = task.exit_result.ok_or(Error::Invariant)?;
        self.tasks[id.slot()] = None;
        Ok(ExitRecord {
            result,
            context: task.context,
        })
    }
    pub fn take_wake(&mut self, id: TaskId) -> Result<Option<WaitOutcome>, Error> {
        self.task(id)?;
        Ok(self.tasks[id.slot()]
            .as_mut()
            .expect("live task")
            .wake
            .take())
    }
    /// Complete diagnostic invariant check; bounded and allocation-free.
    pub fn validate(&self) -> Result<(), Error> {
        for task in self.tasks.iter().flatten() {
            let queued = self
                .queues
                .iter()
                .flat_map(|q| q.entries.iter())
                .filter(|entry| **entry == Some(task.id))
                .count();
            let current = self
                .current
                .iter()
                .filter(|entry| **entry == Some(task.id))
                .count();
            let pending = self
                .pending
                .iter()
                .flatten()
                .filter(|(id, _)| *id == task.id)
                .count();
            if queued != usize::from(task.queued_cpu.is_some())
                || current != usize::from(task.current_cpu.is_some())
                || pending != usize::from(task.retiring_cpu.is_some())
                || queued + current + pending > 1
            {
                return Err(Error::Invariant);
            }
            if let Some(cpu) = task.queued_cpu
                && (task.state != State::Ready
                    || task.suspended
                    || task.wait.is_some()
                    || task.affinity & (1 << cpu) == 0
                    || !self.queues[cpu as usize].entries.contains(&Some(task.id)))
            {
                return Err(Error::Invariant);
            }
            if task.state == State::Ready
                && task.queued_cpu.is_none()
                && task.retiring_cpu.is_none()
            {
                return Err(Error::Invariant);
            }
            if task.state == State::Running && task.current_cpu.is_none() {
                return Err(Error::Invariant);
            }
            if task.state == State::Blocked
                && task.wait.is_none()
                && task.current_cpu.is_none()
                && task.retiring_cpu.is_none()
            {
                return Err(Error::Invariant);
            }
            if task.state == State::Terminated
                && (queued + current + pending != 0
                    || task.wait.is_some()
                    || task.exit_result.is_none())
            {
                return Err(Error::Invariant);
            }
        }
        for (cpu, queue) in self.queues.iter().enumerate() {
            for id in queue.entries.iter().flatten() {
                if self.task(*id)?.queued_cpu != Some(cpu as u8) {
                    return Err(Error::Invariant);
                }
            }
        }
        for (cpu, id) in self.current.iter().enumerate() {
            if let Some(id) = id
                && self.task(*id)?.current_cpu != Some(cpu as u8)
            {
                return Err(Error::Invariant);
            }
        }
        Ok(())
    }
    pub(super) fn task(&self, id: TaskId) -> Result<Task, Error> {
        self.tasks
            .get(id.slot())
            .copied()
            .flatten()
            .filter(|task| task.id == id)
            .ok_or(Error::InvalidTask)
    }
    pub(super) fn put(&mut self, task: Task) {
        self.tasks[task.id.slot()] = Some(task);
    }
    pub(super) fn request_reschedule(&mut self, cpus: u8) {
        self.reschedule |= cpus;
    }
    pub(super) fn running(&self, cpu: u8) -> Result<Task, Error> {
        let id = self.current(cpu)?.ok_or(Error::InvalidState)?;
        let task = self.task(id)?;
        if task.state != State::Running {
            return Err(Error::InvalidState);
        }
        Ok(task)
    }
    pub(super) fn sequence(&mut self) -> u64 {
        if self.sequence == u64::MAX {
            let mut orders = [0u64; MAX_TASKS * 2];
            for (index, task) in self.tasks.iter().enumerate() {
                if let Some(task) = task {
                    orders[index * 2] = task.ready_sequence;
                    orders[index * 2 + 1] = task.wait.map_or(0, |wait| wait.sequence);
                }
            }
            for task in self.tasks.iter_mut().flatten() {
                if task.ready_sequence != 0 {
                    task.ready_sequence = 1 + orders
                        .iter()
                        .filter(|&&value| value != 0 && value < task.ready_sequence)
                        .count() as u64;
                }
                if let Some(wait) = &mut task.wait {
                    wait.sequence = 1 + orders
                        .iter()
                        .filter(|&&value| value != 0 && value < wait.sequence)
                        .count() as u64;
                }
            }
            self.sequence = (MAX_TASKS * 2 + 1) as u64;
        }
        self.sequence += 1;
        self.sequence
    }
    pub(super) fn enqueue(&mut self, id: TaskId) -> Result<(), Error> {
        let mut task = self.task(id)?;
        if task.state != State::Ready
            || task.suspended
            || task.wait.is_some()
            || task.current_cpu.is_some()
            || task.retiring_cpu.is_some()
            || task.queued_cpu.is_some()
        {
            return Err(Error::Invariant);
        }
        let cpu = (0..self.online)
            .filter(|cpu| task.affinity & (1 << cpu) != 0)
            .min_by_key(|cpu| {
                (
                    self.queues[*cpu as usize].len(),
                    u8::from(task.last_cpu != Some(*cpu)),
                    *cpu,
                )
            })
            .ok_or(Error::InvalidAffinity)?;
        let slot = self.queues[cpu as usize]
            .entries
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Invariant)?;
        task.ready_sequence = self.sequence();
        task.ready_since = self.now;
        task.queued_cpu = Some(cpu);
        self.put(task);
        self.queues[cpu as usize].entries[slot] = Some(id);
        self.reschedule |= task.affinity;
        Ok(())
    }
    fn dequeue(&mut self, id: TaskId) -> Result<(), Error> {
        let mut task = self.task(id)?;
        let cpu = task.queued_cpu.ok_or(Error::Invariant)?;
        let slot = self.queues[cpu as usize]
            .entries
            .iter()
            .position(|entry| *entry == Some(id))
            .ok_or(Error::Invariant)?;
        self.queues[cpu as usize].entries[slot] = None;
        task.queued_cpu = None;
        self.put(task);
        Ok(())
    }
    fn best_ready(&self, cpu: u8) -> Option<TaskId> {
        self.queues
            .iter()
            .flat_map(|q| q.entries.iter())
            .flatten()
            .copied()
            .filter(|id| self.tasks[id.slot()].is_some_and(|task| task.affinity & (1 << cpu) != 0))
            .max_by_key(|id| {
                let task = self.tasks[id.slot()].expect("queued task");
                (
                    self.ready_score(*id),
                    u64::MAX - task.ready_sequence,
                    u8::from(task.queued_cpu == Some(cpu)),
                )
            })
    }
    fn ready_score(&self, id: TaskId) -> u8 {
        let task = self.tasks[id.slot()].expect("queued task");
        (u64::from(task.effective_priority) + self.now.saturating_sub(task.ready_since) / AGE_TICKS)
            .min(u64::from(PRIORITY_LEVELS - 1)) as u8
    }
    fn dispatch(&mut self, cpu: u8, id: TaskId) -> Result<(), Error> {
        self.dequeue(id)?;
        let mut task = self.task(id)?;
        if task.last_cpu.is_some_and(|last| last != cpu) {
            task.migrations = task.migrations.saturating_add(1);
            self.stats.migrations = self.stats.migrations.saturating_add(1);
        }
        task.state = State::Running;
        task.current_cpu = Some(cpu);
        task.last_cpu = Some(cpu);
        task.dispatches = task.dispatches.saturating_add(1);
        self.stats.dispatches = self.stats.dispatches.saturating_add(1);
        self.current[cpu as usize] = Some(id);
        self.put(task);
        Ok(())
    }
    fn finish_exit(&mut self, id: TaskId) -> Result<(), Error> {
        let mut task = self.task(id)?;
        if task.current_cpu.is_some() || task.retiring_cpu.is_some() {
            return Err(Error::NotRetired);
        }
        task.state = State::Terminated;
        task.wait = None;
        self.put(task);
        self.abandon_mutexes(id)?;
        Ok(())
    }
    fn cpu(&self, cpu: u8) -> Result<(), Error> {
        if cpu >= self.online {
            Err(Error::InvalidCpu)
        } else {
            Ok(())
        }
    }
    fn affinity_valid(&self, affinity: u8) -> Result<(), Error> {
        if affinity == 0 || affinity & !((1 << self.online) - 1) != 0 {
            Err(Error::InvalidAffinity)
        } else {
            Ok(())
        }
    }
    fn context_valid(context: u64) -> Result<(), Error> {
        if context == 0 || !context.is_multiple_of(8) {
            Err(Error::InvalidContext)
        } else {
            Ok(())
        }
    }
}
