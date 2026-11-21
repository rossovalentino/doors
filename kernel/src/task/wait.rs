use super::*;

impl Scheduler {
    pub fn create_event(&mut self, manual_reset: bool, signaled: bool) -> Result<ObjectId, Error> {
        self.create_object(ObjectKind::Event {
            manual: manual_reset,
            signaled,
        })
    }
    pub fn create_semaphore(&mut self, initial: u32, maximum: u32) -> Result<ObjectId, Error> {
        if maximum == 0 || initial > maximum {
            return Err(Error::InvalidCount);
        }
        self.create_object(ObjectKind::Semaphore {
            count: initial,
            maximum,
        })
    }
    pub fn create_mutex(&mut self) -> Result<ObjectId, Error> {
        self.create_object(ObjectKind::Mutex {
            owner: None,
            abandoned: false,
        })
    }
    pub fn create_condition(&mut self) -> Result<ObjectId, Error> {
        self.create_object(ObjectKind::Condition)
    }
    pub fn mutex_owner(&self, id: ObjectId) -> Result<Option<TaskId>, Error> {
        match self.object(id)?.kind {
            ObjectKind::Mutex { owner, .. } => Ok(owner),
            _ => Err(Error::WrongObjectKind),
        }
    }
    pub fn semaphore_count(&self, id: ObjectId) -> Result<u32, Error> {
        match self.object(id)?.kind {
            ObjectKind::Semaphore { count, .. } => Ok(count),
            _ => Err(Error::WrongObjectKind),
        }
    }
    /// Observe availability, consume one result, or register and block atomically
    /// under the runtime scheduling lock. The runtime keeps that lock until the
    /// subsequent schedule() has captured the current stack/context. Wait-any
    /// uses the lowest ready input index. Condition variables use condition_wait.
    pub fn wait_current(
        &mut self,
        cpu: u8,
        objects: &[ObjectId],
        deadline: Option<u64>,
    ) -> Result<WaitStatus, Error> {
        let mut task = self.running(cpu)?;
        self.validate_wait(task.id, objects)?;
        if let Some((index, object)) = self.ready_candidate(task.id, objects) {
            let abandoned = self.consume(task.id, object)?;
            let outcome = WaitOutcome {
                reason: WakeReason::Signaled,
                object: Some(object),
                index: Some(index),
                abandoned,
            };
            task.wake = Some(outcome);
            self.put(task);
            self.stats.completed_waits = self.stats.completed_waits.saturating_add(1);
            self.recompute_priorities();
            return Ok(WaitStatus::Completed(outcome));
        }
        if deadline.is_some_and(|deadline| deadline <= self.now) {
            let outcome = WaitOutcome::terminal(WakeReason::Timeout);
            task.wake = Some(outcome);
            self.put(task);
            self.stats.completed_waits = self.stats.completed_waits.saturating_add(1);
            self.stats.timer_wakes = self.stats.timer_wakes.saturating_add(1);
            return Ok(WaitStatus::Completed(outcome));
        }
        // Reject a provable nonrecursive mutex cycle. Wait-any can resolve via
        // another object, so its more general application deadlock policy is not
        // inferred from one locked mutex in the set.
        if objects.len() == 1
            && let ObjectKind::Mutex {
                owner: Some(owner), ..
            } = self.object(objects[0])?.kind
            && self.would_deadlock(task.id, owner)
        {
            return Err(Error::Deadlock);
        }
        let sequence = self.sequence();
        let mut registration = WaitRegistration {
            objects: [None; MAX_WAIT_OBJECTS],
            count: objects.len() as u8,
            deadline,
            sequence,
            reacquire: None,
            completion: None,
            address: None,
        };
        for (slot, object) in registration.objects.iter_mut().zip(objects.iter()) {
            *slot = Some(*object);
        }
        task.wait = Some(registration);
        task.wake = None;
        task.state = State::Blocked;
        self.put(task);
        self.recompute_priorities();
        Ok(WaitStatus::Blocked)
    }
    /// Atomically register on a condition and release an owned nonrecursive
    /// mutex. Every outcome reacquires that mutex before becoming runnable,
    /// except explicit closure of the mutex (Closed then grants no ownership)
    /// or the privileged forced-stop path, which must never return to user code.
    /// Cancellation/timeout decides the condition outcome once; reacquisition
    /// itself cannot be cancelled a second time or time out.
    pub fn condition_wait(
        &mut self,
        cpu: u8,
        condition: ObjectId,
        mutex: ObjectId,
        deadline: Option<u64>,
    ) -> Result<WaitStatus, Error> {
        let mut task = self.running(cpu)?;
        if !matches!(self.object(condition)?.kind, ObjectKind::Condition) {
            return Err(Error::WrongObjectKind);
        }
        if self.mutex_owner(mutex)? != Some(task.id) {
            return Err(Error::NotOwner);
        }
        task.wait = Some(WaitRegistration {
            objects: [Some(condition), None, None, None],
            count: 1,
            deadline,
            sequence: self.sequence(),
            reacquire: Some(mutex),
            completion: None,
            address: None,
        });
        task.wake = None;
        task.state = State::Blocked;
        self.put(task);
        self.release_owned_mutex(task.id, mutex)?;
        if deadline.is_some_and(|deadline| deadline <= self.now) {
            self.complete_wait(task.id, WaitOutcome::terminal(WakeReason::Timeout), false)?;
        }
        self.recompute_priorities();
        // Even immediate reacquisition stays under the same save/switch protocol;
        // returning Blocked requires the runtime to call schedule before unlock.
        Ok(WaitStatus::Blocked)
    }
    /// The wake sequence of `key`'s bucket. A futex-style waiter reads it,
    /// then the user word (outside this lock), then calls `wait_address`
    /// with it: a wake of the same bucket in between makes the wait return
    /// at once (a spurious wake WaitOnAddress allows), so no user memory is
    /// read under the scheduling lock and no wake is lost.
    pub fn address_sequence(&self, key: u64) -> u64 {
        self.address_sequences[address_bucket(key)]
    }
    /// Block the running task on `key` (a process-qualified user address)
    /// unless its bucket was woken since `sequence` was read.
    pub fn wait_address(
        &mut self,
        cpu: u8,
        key: u64,
        sequence: u64,
        deadline: Option<u64>,
    ) -> Result<WaitStatus, Error> {
        let mut task = self.running(cpu)?;
        let immediate = if self.address_sequences[address_bucket(key)] != sequence {
            Some(WakeReason::Signaled)
        } else if deadline.is_some_and(|deadline| deadline <= self.now) {
            Some(WakeReason::Timeout)
        } else {
            None
        };
        if let Some(reason) = immediate {
            let outcome = WaitOutcome::terminal(reason);
            task.wake = Some(outcome);
            self.put(task);
            self.stats.completed_waits = self.stats.completed_waits.saturating_add(1);
            return Ok(WaitStatus::Completed(outcome));
        }
        task.wait = Some(WaitRegistration {
            objects: [None; MAX_WAIT_OBJECTS],
            count: 0,
            deadline,
            sequence: self.sequence(),
            reacquire: None,
            completion: None,
            address: Some(key),
        });
        task.wake = None;
        task.state = State::Blocked;
        self.put(task);
        self.recompute_priorities();
        Ok(WaitStatus::Blocked)
    }
    /// Wake up to `count` tasks waiting on `key`, oldest first; the number
    /// woken. Every call advances the bucket's sequence first.
    pub fn wake_address(&mut self, key: u64, count: usize) -> Result<usize, Error> {
        let bucket = address_bucket(key);
        self.address_sequences[bucket] = self.address_sequences[bucket].wrapping_add(1);
        let mut woken = 0;
        while woken < count {
            let oldest = self
                .tasks
                .iter()
                .flatten()
                .filter_map(|task| {
                    task.wait
                        .filter(|w| w.completion.is_none() && w.address == Some(key))
                        .map(|w| (w.sequence, task.id))
                })
                .min_by_key(|(sequence, _)| *sequence);
            let Some((_, id)) = oldest else {
                break;
            };
            self.complete_wait(id, WaitOutcome::terminal(WakeReason::Signaled), false)?;
            woken += 1;
        }
        if woken > 0 {
            self.recompute_priorities();
        }
        Ok(woken)
    }
    pub fn signal_event(&mut self, id: ObjectId) -> Result<usize, Error> {
        let mut object = self.object(id)?;
        match &mut object.kind {
            ObjectKind::Event { signaled, .. } => *signaled = true,
            _ => return Err(Error::WrongObjectKind),
        }
        self.objects[id.slot()] = Some(object);
        self.service_object(id)
    }
    pub fn reset_event(&mut self, id: ObjectId) -> Result<(), Error> {
        let mut object = self.object(id)?;
        match &mut object.kind {
            ObjectKind::Event { signaled, .. } => *signaled = false,
            _ => return Err(Error::WrongObjectKind),
        }
        self.objects[id.slot()] = Some(object);
        Ok(())
    }
    pub fn release_semaphore(&mut self, id: ObjectId, amount: u32) -> Result<usize, Error> {
        if amount == 0 {
            return Err(Error::InvalidCount);
        }
        let mut object = self.object(id)?;
        match &mut object.kind {
            ObjectKind::Semaphore { count, maximum } => {
                *count = count
                    .checked_add(amount)
                    .filter(|count| *count <= *maximum)
                    .ok_or(Error::InvalidCount)?
            }
            _ => return Err(Error::WrongObjectKind),
        }
        self.objects[id.slot()] = Some(object);
        self.service_object(id)
    }
    pub fn unlock_mutex(&mut self, cpu: u8, id: ObjectId) -> Result<usize, Error> {
        let task = self.running(cpu)?;
        self.release_owned_mutex(task.id, id)
    }
    pub fn signal_condition(&mut self, id: ObjectId, broadcast: bool) -> Result<usize, Error> {
        if !matches!(self.object(id)?.kind, ObjectKind::Condition) {
            return Err(Error::WrongObjectKind);
        }
        let mut woken = 0;
        while let Some(task) = self.best_waiter(id, false) {
            let outcome = WaitOutcome {
                reason: WakeReason::Signaled,
                object: Some(id),
                index: Some(0),
                abandoned: false,
            };
            self.complete_wait(task, outcome, false)?;
            woken += 1;
            if !broadcast {
                break;
            }
        }
        self.recompute_priorities();
        Ok(woken)
    }
    /// Returns false once another cause has won or condition reacquisition has
    /// begun. A recorded outcome cannot be replaced by a later signal/cancel.
    pub fn cancel_wait(&mut self, id: TaskId) -> Result<bool, Error> {
        let task = self.task(id)?;
        if task.wait.is_none_or(|wait| wait.completion.is_some()) {
            return Ok(false);
        }
        self.complete_wait(id, WaitOutcome::terminal(WakeReason::Cancelled), false)?;
        self.recompute_priorities();
        Ok(true)
    }
    /// Resume a stopped task's kernel continuation without acquiring any wait
    /// object. Unlike ordinary cancellation, this also aborts condition mutex
    /// reacquisition, even after another completion cause has already won.
    ///
    /// The caller must have irrevocably requested termination and must prevent
    /// the continuation from returning to user code. It must run far enough to
    /// release syscall leases before exiting through normal stack retirement.
    /// No mutex ownership is granted or revoked here, and a context still owned
    /// by a CPU cannot become runnable until its retirement is acknowledged.
    pub fn abort_wait_for_stop(&mut self, id: TaskId) -> Result<bool, Error> {
        if self.task(id)?.wait.is_none() {
            return Ok(false);
        }
        self.finish_wait(id, WaitOutcome::terminal(WakeReason::Cancelled))?;
        self.recompute_priorities();
        Ok(true)
    }
    pub fn close_object(&mut self, id: ObjectId) -> Result<(), Error> {
        self.object(id)?;
        self.objects[id.slot()] = None;
        for index in 0..MAX_TASKS {
            let Some(task) = self.tasks[index] else {
                continue;
            };
            let Some(wait) = task.wait else {
                continue;
            };
            if !wait.contains(id) && wait.reacquire != Some(id) {
                continue;
            }
            let outcome = WaitOutcome {
                reason: WakeReason::Closed,
                object: Some(id),
                index: wait.objects[..wait.count as usize]
                    .iter()
                    .position(|object| *object == Some(id))
                    .map(|n| n as u8),
                abandoned: false,
            };
            if wait.reacquire == Some(id) {
                self.finish_wait(task.id, outcome)?;
            } else {
                self.complete_wait(task.id, outcome, false)?;
            }
        }
        self.recompute_priorities();
        Ok(())
    }
    pub fn advance_time(&mut self, monotonic_ticks: u64) -> Result<usize, Error> {
        if monotonic_ticks < self.now {
            return Err(Error::ClockReversed);
        }
        // One coarse observation per advancing BSP timer, not elapsed-time
        // charging. Lost timer observations are not invented as CPU residency.
        if monotonic_ticks > self.now {
            self.stats.total_samples = self
                .stats
                .total_samples
                .saturating_add(self.online_cpus() as u64);
            for cpu in 0..self.online_cpus() as usize {
                if let Some(id) = self.current(cpu as u8)? {
                    self.stats.busy_samples = self.stats.busy_samples.saturating_add(1);
                    let mut task = self.task(id)?;
                    task.samples = task.samples.saturating_add(1);
                    self.put(task);
                }
            }
        }
        self.now = monotonic_ticks;
        let mut count = 0;
        for slot in 0..MAX_TASKS {
            let Some(task) = self.tasks[slot] else {
                continue;
            };
            if task.wait.is_some_and(|wait| {
                wait.completion.is_none()
                    && wait.deadline.is_some_and(|deadline| deadline <= self.now)
            }) {
                self.complete_wait(task.id, WaitOutcome::terminal(WakeReason::Timeout), false)?;
                count += 1;
            }
        }
        self.recompute_priorities();
        Ok(count)
    }
    pub(super) fn recompute_priorities(&mut self) {
        let previous: [u8; MAX_TASKS] =
            core::array::from_fn(|slot| self.tasks[slot].map_or(0, |task| task.effective_priority));
        for task in self.tasks.iter_mut().flatten() {
            task.effective_priority = task.base_priority;
        }
        // At most MAX_TASKS passes propagate the longest acyclic donation chain;
        // a cycle still converges because priorities only rise within 0..7.
        for _ in 0..MAX_TASKS {
            let mut changed = false;
            for slot in 0..MAX_TASKS {
                let Some(task) = self.tasks[slot] else {
                    continue;
                };
                let Some(wait) = task.wait else {
                    continue;
                };
                for id in wait.objects[..wait.count as usize].iter().flatten() {
                    if let Ok(Object {
                        kind:
                            ObjectKind::Mutex {
                                owner: Some(owner), ..
                            },
                        ..
                    }) = self.object(*id)
                        && let Some(owner_task) = self.tasks[owner.slot()]
                            .as_mut()
                            .filter(|value| value.id == owner)
                        && owner_task.effective_priority < task.effective_priority
                    {
                        owner_task.effective_priority = task.effective_priority;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        let mut changed_cpus = 0;
        for (slot, task) in self.tasks.iter().enumerate() {
            if let Some(task) = task
                && task.effective_priority != previous[slot]
            {
                changed_cpus |= task.affinity;
            }
        }
        self.request_reschedule(changed_cpus);
    }
    pub(super) fn abandon_mutexes(&mut self, owner: TaskId) -> Result<(), Error> {
        for slot in 0..MAX_OBJECTS {
            let Some(mut object) = self.objects[slot] else {
                continue;
            };
            if let ObjectKind::Mutex {
                owner: current,
                abandoned,
            } = &mut object.kind
                && *current == Some(owner)
            {
                *current = None;
                *abandoned = true;
                self.objects[slot] = Some(object);
                self.service_object(object.id)?;
            }
        }
        self.recompute_priorities();
        Ok(())
    }
    fn create_object(&mut self, kind: ObjectKind) -> Result<ObjectId, Error> {
        let slot = (0..MAX_OBJECTS)
            .find(|&slot| self.objects[slot].is_none() && self.object_generations[slot] != u64::MAX)
            .ok_or(Error::Capacity)?;
        let generation = self.object_generations[slot] + 1;
        let id = ObjectId {
            slot: slot as u16,
            generation,
        };
        self.object_generations[slot] = generation;
        self.objects[slot] = Some(Object { id, kind });
        Ok(id)
    }
    fn object(&self, id: ObjectId) -> Result<Object, Error> {
        self.objects
            .get(id.slot())
            .copied()
            .flatten()
            .filter(|object| object.id == id)
            .ok_or(Error::InvalidObject)
    }
    fn validate_wait(&self, task: TaskId, objects: &[ObjectId]) -> Result<(), Error> {
        if objects.is_empty() {
            return Err(Error::EmptyWait);
        }
        if objects.len() > MAX_WAIT_OBJECTS {
            return Err(Error::TooManyWaits);
        }
        for (index, id) in objects.iter().enumerate() {
            if objects[..index].contains(id) {
                return Err(Error::DuplicateWait);
            }
            match self.object(*id)?.kind {
                ObjectKind::Condition => return Err(Error::WrongObjectKind),
                ObjectKind::Mutex {
                    owner: Some(owner), ..
                } if owner == task => return Err(Error::AlreadyOwned),
                _ => (),
            }
        }
        Ok(())
    }
    fn ready_candidate(&self, task: TaskId, objects: &[ObjectId]) -> Option<(u8, ObjectId)> {
        objects.iter().enumerate().find_map(|(index, id)| {
            let ready = match self.object(*id).ok()?.kind {
                ObjectKind::Event { signaled, .. } => signaled,
                ObjectKind::Semaphore { count, .. } => count > 0,
                ObjectKind::Mutex { owner, .. } => owner.is_none(),
                ObjectKind::Condition => false,
            };
            let _ = task;
            ready.then_some((index as u8, *id))
        })
    }
    fn registration_ready(&self, task: Task) -> Option<(u8, ObjectId)> {
        let wait = task.wait?;
        let mut objects = [ObjectId {
            slot: 0,
            generation: 0,
        }; MAX_WAIT_OBJECTS];
        for (index, object) in wait.objects[..wait.count as usize].iter().enumerate() {
            objects[index] = (*object)?;
        }
        self.ready_candidate(task.id, &objects[..wait.count as usize])
    }
    fn consume(&mut self, task: TaskId, id: ObjectId) -> Result<bool, Error> {
        let mut object = self.object(id)?;
        let mut was_abandoned = false;
        match &mut object.kind {
            ObjectKind::Event { manual, signaled } if *signaled => {
                if !*manual {
                    *signaled = false;
                }
            }
            ObjectKind::Semaphore { count, .. } if *count > 0 => *count -= 1,
            ObjectKind::Mutex { owner, abandoned } if owner.is_none() => {
                *owner = Some(task);
                was_abandoned = *abandoned;
                *abandoned = false;
            }
            _ => return Err(Error::InvalidState),
        }
        self.objects[id.slot()] = Some(object);
        Ok(was_abandoned)
    }
    fn best_waiter(&self, object: ObjectId, require_ready: bool) -> Option<TaskId> {
        self.tasks
            .iter()
            .flatten()
            .filter(|task| {
                task.wait.is_some_and(|wait| wait.contains(object))
                    && (!require_ready || self.registration_ready(**task).is_some())
            })
            .max_by_key(|task| {
                (
                    task.effective_priority,
                    u64::MAX - task.wait.expect("waiting").sequence,
                )
            })
            .map(|task| task.id)
    }
    fn service_object(&mut self, id: ObjectId) -> Result<usize, Error> {
        let mut count = 0;
        while let Some(task) = self.best_waiter(id, true) {
            let (index, ready) = self
                .registration_ready(self.task(task)?)
                .ok_or(Error::Invariant)?;
            let abandoned = self.consume(task, ready)?;
            self.complete_wait(
                task,
                WaitOutcome {
                    reason: WakeReason::Signaled,
                    object: Some(ready),
                    index: Some(index),
                    abandoned,
                },
                true,
            )?;
            count += 1;
            self.recompute_priorities();
        }
        self.recompute_priorities();
        Ok(count)
    }
    fn complete_wait(
        &mut self,
        id: TaskId,
        mut outcome: WaitOutcome,
        acquired: bool,
    ) -> Result<(), Error> {
        let mut task = self.task(id)?;
        let mut wait = task.wait.ok_or(Error::InvalidState)?;
        if let Some(mutex) = wait.reacquire {
            if let Some(original) = wait.completion {
                if !acquired {
                    return Err(Error::InvalidState);
                }
                outcome = WaitOutcome {
                    abandoned: outcome.abandoned,
                    ..original
                };
            } else {
                let Ok(object) = self.object(mutex) else {
                    return self.finish_wait(
                        id,
                        WaitOutcome {
                            reason: WakeReason::Closed,
                            object: Some(mutex),
                            index: None,
                            abandoned: false,
                        },
                    );
                };
                match object.kind {
                    ObjectKind::Mutex { owner: None, .. } => {
                        outcome.abandoned = self.consume(id, mutex)?
                    }
                    ObjectKind::Mutex { .. } => {
                        wait.objects = [Some(mutex), None, None, None];
                        wait.count = 1;
                        wait.deadline = None;
                        wait.sequence = self.sequence();
                        wait.completion = Some(outcome);
                        task.wait = Some(wait);
                        self.put(task);
                        self.recompute_priorities();
                        return Ok(());
                    }
                    _ => return Err(Error::Invariant),
                }
            }
        }
        self.finish_wait(id, outcome)
    }
    fn finish_wait(&mut self, id: TaskId, outcome: WaitOutcome) -> Result<(), Error> {
        let mut task = self.task(id)?;
        if task.wait.is_none() {
            return Err(Error::InvalidState);
        }
        task.wait = None;
        task.wake = Some(outcome);
        task.state = if task.suspended {
            State::Suspended
        } else if task.current_cpu.is_some() || task.retiring_cpu.is_some() {
            State::Blocked
        } else {
            State::Ready
        };
        self.put(task);
        self.stats.completed_waits = self.stats.completed_waits.saturating_add(1);
        if outcome.reason == WakeReason::Timeout {
            self.stats.timer_wakes = self.stats.timer_wakes.saturating_add(1);
        }
        if task.state == State::Ready {
            self.enqueue(id)?;
        }
        Ok(())
    }
    fn release_owned_mutex(&mut self, owner: TaskId, id: ObjectId) -> Result<usize, Error> {
        let mut object = self.object(id)?;
        match &mut object.kind {
            ObjectKind::Mutex { owner: current, .. } if *current == Some(owner) => *current = None,
            ObjectKind::Mutex { .. } => return Err(Error::NotOwner),
            _ => return Err(Error::WrongObjectKind),
        }
        self.objects[id.slot()] = Some(object);
        self.service_object(id)
    }
    fn would_deadlock(&self, waiter: TaskId, mut owner: TaskId) -> bool {
        for _ in 0..MAX_TASKS {
            if owner == waiter {
                return true;
            }
            let Ok(task) = self.task(owner) else {
                return false;
            };
            let Some(wait) = task.wait.filter(|wait| wait.count == 1) else {
                return false;
            };
            let Some(object) = wait.objects[0] else {
                return false;
            };
            let Ok(Object {
                kind:
                    ObjectKind::Mutex {
                        owner: Some(next), ..
                    },
                ..
            }) = self.object(object)
            else {
                return false;
            };
            owner = next;
        }
        true
    }
}
/// The wake-sequence bucket of an address-wait key.
fn address_bucket(key: u64) -> usize {
    let mixed = (key >> 2) ^ (key >> 13) ^ (key >> 29) ^ (key >> 47);
    (mixed as usize) % ADDRESS_BUCKETS
}
