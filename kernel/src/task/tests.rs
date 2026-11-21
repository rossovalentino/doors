use super::*;

fn spawn(s: &mut Scheduler, priority: u8, affinity: u8, number: u64) -> TaskId {
    let id = s
        .create(0x1000 + number * 0x100, priority, affinity)
        .unwrap();
    s.start(id).unwrap();
    id
}
fn context(s: &Scheduler, cpu: u8) -> u64 {
    s.current(cpu)
        .unwrap()
        .map_or(0, |id| s.snapshot(id).unwrap().context)
}
fn switch(s: &mut Scheduler, cpu: u8) -> Decision {
    let decision = s.schedule(cpu, context(s, cpu)).unwrap();
    if let Some(ticket) = decision.retirement {
        s.acknowledge_retirement(ticket).unwrap();
    }
    s.validate().unwrap();
    decision
}
fn blocked_event() -> (Scheduler, TaskId, ObjectId) {
    let mut s = Scheduler::new(1).unwrap();
    let task = spawn(&mut s, 2, 1, 0);
    let event = s.create_event(false, false).unwrap();
    switch(&mut s, 0);
    assert_eq!(
        s.wait_current(0, &[event], Some(10)).unwrap(),
        WaitStatus::Blocked
    );
    assert_eq!(switch(&mut s, 0).next, None);
    (s, task, event)
}

#[test]
fn equal_priority_round_robin_and_bounded_aging_make_progress() {
    let mut s = Scheduler::new(1).unwrap();
    let a = spawn(&mut s, 7, 1, 0);
    let b = spawn(&mut s, 7, 1, 1);
    let low = spawn(&mut s, 0, 1, 2);
    assert_eq!(switch(&mut s, 0).next, Some(a));
    assert_eq!(switch(&mut s, 0).next, Some(b));
    assert_eq!(switch(&mut s, 0).next, Some(a));
    for tick in 1..=u64::from(PRIORITY_LEVELS) * AGE_TICKS {
        s.advance_time(tick).unwrap();
        switch(&mut s, 0);
        if s.snapshot(low).unwrap().dispatches > 0 {
            break;
        }
    }
    assert!(s.snapshot(low).unwrap().dispatches > 0);
    assert!(s.snapshot(a).unwrap().dispatches > 1);
    assert!(s.snapshot(b).unwrap().dispatches > 1);
}

#[test]
fn outgoing_context_cannot_migrate_or_be_reaped_before_arch_retirement() {
    let mut s = Scheduler::new(2).unwrap();
    let a = spawn(&mut s, 3, 1, 0);
    let b = spawn(&mut s, 3, 1, 1);
    assert_eq!(switch(&mut s, 0).next, Some(a));
    s.set_affinity(a, 2).unwrap();
    let decision = s.schedule(0, 0x8888).unwrap();
    assert_eq!(decision.next, Some(b));
    assert_eq!(s.snapshot(a).unwrap().retiring_cpu, Some(0));
    assert_eq!(s.snapshot(a).unwrap().queued_cpu, None);
    assert_eq!(s.schedule(1, 0).unwrap().next, None);
    assert_eq!(s.reap(a), Err(Error::NotRetired));
    assert_eq!(s.schedule(0, 0x9998), Err(Error::RetirementPending));
    let ticket = decision.retirement.unwrap();
    assert_eq!(
        s.acknowledge_retirement(RetirementTicket {
            epoch: ticket.epoch + 1,
            ..ticket
        }),
        Err(Error::InvalidRetirement)
    );
    s.acknowledge_retirement(ticket).unwrap();
    assert_eq!(switch(&mut s, 1).next, Some(a));
    assert_eq!(s.snapshot(a).unwrap().context, 0x8888);
    assert_eq!(s.snapshot(a).unwrap().migrations, 1);
    assert_eq!(
        s.acknowledge_retirement(ticket),
        Err(Error::InvalidRetirement)
    );
    s.validate().unwrap();
}

#[test]
fn task_exit_result_stays_stable_and_reused_ids_cannot_target_a_new_task() {
    let mut s = Scheduler::new(1).unwrap();
    let a = spawn(&mut s, 1, 1, 0);
    switch(&mut s, 0);
    s.exit_current(0, -17).unwrap();
    assert_eq!(s.exit_result(a).unwrap(), None);
    assert_eq!(s.terminate(a, 42), Err(Error::InvalidState));
    let decision = s.schedule(0, 0x2228).unwrap();
    assert_eq!(s.reap(a), Err(Error::NotRetired));
    s.acknowledge_retirement(decision.retirement.unwrap())
        .unwrap();
    assert_eq!(s.exit_result(a).unwrap(), Some(-17));
    assert_eq!(
        s.reap(a).unwrap(),
        ExitRecord {
            result: -17,
            context: 0x2228
        }
    );
    let replacement = spawn(&mut s, 1, 1, 1);
    assert_eq!(a.slot(), replacement.slot());
    assert_ne!(a.generation(), replacement.generation());
    assert_eq!(s.cancel_wait(a), Err(Error::InvalidTask));
    assert_eq!(s.terminate(a, 0), Err(Error::InvalidTask));
    assert_eq!(s.reap(a), Err(Error::InvalidTask));
    s.validate().unwrap();
}

#[test]
fn delayed_stop_snapshot_cannot_erase_a_replacement_tasks_stop_request() {
    let mut s = Scheduler::new(1).unwrap();
    let stops = StopRequests::new();
    let original = s.create(0x1000, 4, 1).unwrap();
    stops.request(&mut s, original, -13).unwrap();
    s.start(original).unwrap();
    assert_eq!(switch(&mut s, 0).next, Some(original));
    assert_eq!(stops.requested(original), Some(-13));
    s.exit_current(0, -13).unwrap();
    switch(&mut s, 0);
    s.reap(original).unwrap();
    let replacement = s.create(0x2000, 4, 1).unwrap();
    assert_eq!(replacement.slot(), original.slot());
    assert_eq!(stops.requested(replacement), None);
    stops.request(&mut s, replacement, -17).unwrap();
    // An older process's captured task list resumes after this slot is reused.
    assert_eq!(
        stops.request(&mut s, original, -99),
        Err(Error::InvalidTask)
    );
    assert_eq!(stops.requested(replacement), Some(-17));
    s.start(replacement).unwrap();
    assert_eq!(switch(&mut s, 0).next, Some(replacement));
    assert_eq!(stops.requested(replacement), Some(-17));
    s.validate().unwrap();
}

#[test]
fn event_signal_before_registration_is_consumed_exactly_once() {
    let mut s = Scheduler::new(1).unwrap();
    let id = spawn(&mut s, 1, 1, 0);
    switch(&mut s, 0);
    let auto = s.create_event(false, false).unwrap();
    s.signal_event(auto).unwrap();
    assert!(matches!(
        s.wait_current(0, &[auto], None).unwrap(),
        WaitStatus::Completed(WaitOutcome {
            reason: WakeReason::Signaled,
            ..
        })
    ));
    assert_eq!(
        s.wait_current(0, &[auto], None).unwrap(),
        WaitStatus::Blocked
    );
    s.signal_event(auto).unwrap(); // adversarial registration-to-save window
    assert_eq!(s.snapshot(id).unwrap().queued_cpu, None);
    assert_eq!(switch(&mut s, 0).next, Some(id));
    assert_eq!(
        s.take_wake(id).unwrap().unwrap().reason,
        WakeReason::Signaled
    );
    assert_eq!(s.take_wake(id).unwrap(), None);
    let manual = s.create_event(true, true).unwrap();
    assert!(matches!(
        s.wait_current(0, &[manual], None).unwrap(),
        WaitStatus::Completed(_)
    ));
    assert!(matches!(
        s.wait_current(0, &[manual], None).unwrap(),
        WaitStatus::Completed(_)
    ));
    s.reset_event(manual).unwrap();
    assert_eq!(
        s.wait_current(0, &[manual], None).unwrap(),
        WaitStatus::Blocked
    );
}

#[test]
fn every_signal_timeout_cancel_close_order_has_one_winner() {
    for first in 0..4 {
        for second in 0..4 {
            for third in 0..4 {
                for fourth in 0..4 {
                    let order = [first, second, third, fourth];
                    if order
                        .iter()
                        .enumerate()
                        .any(|(i, value)| order[..i].contains(value))
                    {
                        continue;
                    }
                    let (mut s, task, event) = blocked_event();
                    for action in order {
                        match action {
                            0 => {
                                let _ = s.signal_event(event);
                            }
                            1 => {
                                s.advance_time(10).unwrap();
                            }
                            2 => {
                                s.cancel_wait(task).unwrap();
                            }
                            3 => {
                                s.close_object(event).unwrap();
                            }
                            _ => unreachable!(),
                        }
                        s.validate().unwrap();
                    }
                    let expected = [
                        WakeReason::Signaled,
                        WakeReason::Timeout,
                        WakeReason::Cancelled,
                        WakeReason::Closed,
                    ][first];
                    assert_eq!(s.take_wake(task).unwrap().unwrap().reason, expected);
                    assert_eq!(s.stats().completed_waits, 1);
                    assert_eq!(s.take_wake(task).unwrap(), None);
                }
            }
        }
    }
}

#[test]
fn wake_and_termination_during_retirement_never_duplicate_queue_membership() {
    for cause in 0..4 {
        let mut s = Scheduler::new(2).unwrap();
        let task = spawn(&mut s, 1, 3, 0);
        let event = s.create_event(false, false).unwrap();
        switch(&mut s, 0);
        s.wait_current(0, &[event], Some(4)).unwrap();
        let decision = s.schedule(0, 0x3330).unwrap();
        match cause {
            0 => {
                s.signal_event(event).unwrap();
            }
            1 => {
                s.advance_time(4).unwrap();
            }
            2 => {
                s.cancel_wait(task).unwrap();
            }
            3 => {
                s.close_object(event).unwrap();
            }
            _ => unreachable!(),
        }
        s.validate().unwrap();
        assert_eq!(s.snapshot(task).unwrap().queued_cpu, None);
        assert_eq!(s.schedule(1, 0).unwrap().next, None);
        s.acknowledge_retirement(decision.retirement.unwrap())
            .unwrap();
        assert_eq!(switch(&mut s, 1).next, Some(task));
        s.terminate(task, 9).unwrap();
        let leaving = s.schedule(1, 0x4440).unwrap();
        assert_eq!(s.reap(task), Err(Error::NotRetired));
        assert!(!s.cancel_wait(task).unwrap());
        s.acknowledge_retirement(leaving.retirement.unwrap())
            .unwrap();
        assert_eq!(s.reap(task).unwrap().result, 9);
        s.validate().unwrap();
    }
}

#[test]
fn wait_any_prefers_lowest_ready_index_and_rejects_stale_or_duplicate_objects() {
    let mut s = Scheduler::new(1).unwrap();
    spawn(&mut s, 1, 1, 0);
    switch(&mut s, 0);
    let a = s.create_event(false, true).unwrap();
    let b = s.create_semaphore(2, 3).unwrap();
    let result = s.wait_current(0, &[a, b], Some(0)).unwrap();
    assert!(
        matches!(result, WaitStatus::Completed(WaitOutcome { index: Some(0), object: Some(id), .. }) if id == a)
    );
    assert_eq!(s.semaphore_count(b).unwrap(), 2);
    assert_eq!(s.wait_current(0, &[b, b], None), Err(Error::DuplicateWait));
    s.close_object(a).unwrap();
    let replacement = s.create_event(false, true).unwrap();
    assert_eq!(replacement.slot(), a.slot());
    assert_ne!(replacement.generation(), a.generation());
    assert_eq!(s.wait_current(0, &[a], None), Err(Error::InvalidObject));
    assert_eq!(s.signal_event(a), Err(Error::InvalidObject));
    assert!(matches!(
        s.wait_current(0, &[replacement], None),
        Ok(WaitStatus::Completed(_))
    ));
    s.validate().unwrap();
}

#[test]
fn semaphore_waiters_use_priority_then_fifo_without_losing_counts() {
    let mut s = Scheduler::new(1).unwrap();
    let sem = s.create_semaphore(0, 3).unwrap();
    let a = spawn(&mut s, 2, 1, 0);
    switch(&mut s, 0);
    s.wait_current(0, &[sem], None).unwrap();
    switch(&mut s, 0);
    let b = spawn(&mut s, 7, 1, 1);
    switch(&mut s, 0);
    s.wait_current(0, &[sem], None).unwrap();
    switch(&mut s, 0);
    let c = spawn(&mut s, 2, 1, 2);
    switch(&mut s, 0);
    s.wait_current(0, &[sem], None).unwrap();
    switch(&mut s, 0);
    assert_eq!(s.release_semaphore(sem, 1).unwrap(), 1);
    assert!(s.take_wake(b).unwrap().is_some());
    assert_eq!(s.take_wake(a).unwrap(), None);
    assert_eq!(s.release_semaphore(sem, 1).unwrap(), 1);
    assert!(s.take_wake(a).unwrap().is_some());
    assert_eq!(s.take_wake(c).unwrap(), None);
    assert_eq!(s.release_semaphore(sem, 3).unwrap(), 1);
    assert_eq!(s.semaphore_count(sem).unwrap(), 2);
    assert_eq!(s.release_semaphore(sem, 2), Err(Error::InvalidCount));
    assert_eq!(s.semaphore_count(sem).unwrap(), 2);
    s.validate().unwrap();
}

#[test]
fn mutex_donation_abandonment_and_owner_retirement_are_explicit() {
    let mut s = Scheduler::new(2).unwrap();
    let mutex = s.create_mutex().unwrap();
    let owner = spawn(&mut s, 1, 1, 0);
    switch(&mut s, 0);
    s.wait_current(0, &[mutex], None).unwrap();
    assert_eq!(s.wait_current(0, &[mutex], None), Err(Error::AlreadyOwned));
    let waiter = spawn(&mut s, 7, 2, 1);
    switch(&mut s, 1);
    assert_eq!(s.unlock_mutex(1, mutex), Err(Error::NotOwner));
    s.wait_current(1, &[mutex], None).unwrap();
    switch(&mut s, 1);
    assert_eq!(s.snapshot(owner).unwrap().effective_priority, 7);
    s.exit_current(0, 0).unwrap();
    let outgoing = s.schedule(0, 0x5550).unwrap();
    assert_eq!(s.mutex_owner(mutex).unwrap(), Some(owner));
    assert_eq!(s.take_wake(waiter).unwrap(), None);
    s.acknowledge_retirement(outgoing.retirement.unwrap())
        .unwrap();
    assert_eq!(s.mutex_owner(mutex).unwrap(), Some(waiter));
    assert!(s.take_wake(waiter).unwrap().unwrap().abandoned);
    assert_eq!(switch(&mut s, 1).next, Some(waiter));
    s.unlock_mutex(1, mutex).unwrap();
    assert_eq!(s.mutex_owner(mutex).unwrap(), None);
    s.validate().unwrap();
}

#[test]
fn transitive_donation_is_removed_on_cancel_and_mutex_cycles_are_rejected() {
    let mut s = Scheduler::new(3).unwrap();
    let m1 = s.create_mutex().unwrap();
    let m2 = s.create_mutex().unwrap();
    let a = spawn(&mut s, 1, 1, 0);
    switch(&mut s, 0);
    s.wait_current(0, &[m1], None).unwrap();
    let b = spawn(&mut s, 2, 2, 1);
    switch(&mut s, 1);
    s.wait_current(1, &[m2], None).unwrap();
    s.wait_current(1, &[m1], None).unwrap();
    switch(&mut s, 1);
    assert_eq!(s.wait_current(0, &[m2], None), Err(Error::Deadlock));
    let c = spawn(&mut s, 7, 4, 2);
    switch(&mut s, 2);
    s.wait_current(2, &[m2], None).unwrap();
    switch(&mut s, 2);
    assert_eq!(s.snapshot(a).unwrap().effective_priority, 7);
    assert_eq!(s.snapshot(b).unwrap().effective_priority, 7);
    s.cancel_wait(c).unwrap();
    assert_eq!(s.snapshot(a).unwrap().effective_priority, 2);
    assert_eq!(s.snapshot(b).unwrap().effective_priority, 2);
    s.cancel_wait(b).unwrap();
    assert_eq!(s.snapshot(a).unwrap().effective_priority, 1);
    s.validate().unwrap();
}

fn condition_setup() -> (Scheduler, TaskId, TaskId, ObjectId, ObjectId) {
    let mut s = Scheduler::new(2).unwrap();
    let mutex = s.create_mutex().unwrap();
    let condition = s.create_condition().unwrap();
    let a = spawn(&mut s, 7, 1, 0);
    switch(&mut s, 0);
    s.wait_current(0, &[mutex], None).unwrap();
    s.condition_wait(0, condition, mutex, Some(20)).unwrap();
    switch(&mut s, 0);
    let b = spawn(&mut s, 1, 2, 1);
    switch(&mut s, 1);
    s.wait_current(1, &[mutex], None).unwrap();
    (s, a, b, mutex, condition)
}

#[test]
fn condition_signal_timeout_cancel_and_close_reacquire_mutex_before_resume() {
    for cause in 0..4 {
        let (mut s, a, b, mutex, condition) = condition_setup();
        match cause {
            0 => {
                s.signal_condition(condition, false).unwrap();
            }
            1 => {
                s.advance_time(20).unwrap();
            }
            2 => {
                s.cancel_wait(a).unwrap();
            }
            3 => {
                s.close_object(condition).unwrap();
            }
            _ => unreachable!(),
        }
        assert_eq!(s.take_wake(a).unwrap(), None);
        assert_eq!(s.snapshot(a).unwrap().state, State::Blocked);
        assert_eq!(s.snapshot(b).unwrap().effective_priority, 7);
        assert!(!s.cancel_wait(a).unwrap());
        s.advance_time(100).unwrap();
        assert_eq!(s.take_wake(a).unwrap(), None);
        s.unlock_mutex(1, mutex).unwrap();
        assert_eq!(s.mutex_owner(mutex).unwrap(), Some(a));
        let expected = [
            WakeReason::Signaled,
            WakeReason::Timeout,
            WakeReason::Cancelled,
            WakeReason::Closed,
        ][cause];
        assert_eq!(s.take_wake(a).unwrap().unwrap().reason, expected);
        assert_eq!(switch(&mut s, 0).next, Some(a));
        s.validate().unwrap();
    }
}

#[test]
fn closing_condition_mutex_ends_reacquisition_without_granting_ownership() {
    let (mut s, a, _, mutex, condition) = condition_setup();
    s.signal_condition(condition, false).unwrap();
    s.close_object(mutex).unwrap();
    assert_eq!(s.take_wake(a).unwrap().unwrap().reason, WakeReason::Closed);
    assert_eq!(s.mutex_owner(mutex), Err(Error::InvalidObject));
    assert_eq!(switch(&mut s, 0).next, Some(a));
    s.validate().unwrap();
}

#[test]
fn forced_stop_escapes_foreign_condition_mutex_without_reacquiring_it() {
    for cause in 0..5 {
        let (mut s, victim, foreign, mutex, condition) = condition_setup();
        match cause {
            0 => (), // Still on the condition, before any ordinary completion.
            1 => {
                s.signal_condition(condition, false).unwrap();
            }
            2 => {
                s.advance_time(20).unwrap();
            }
            3 => {
                s.cancel_wait(victim).unwrap();
            }
            4 => {
                s.close_object(condition).unwrap();
            }
            _ => unreachable!(),
        }
        assert_eq!(s.snapshot(victim).unwrap().state, State::Blocked);
        if cause != 0 {
            assert!(!s.cancel_wait(victim).unwrap());
            assert_eq!(s.snapshot(foreign).unwrap().effective_priority, 7);
        }
        let completed = s.stats().completed_waits;
        assert!(s.abort_wait_for_stop(victim).unwrap());
        assert!(!s.abort_wait_for_stop(victim).unwrap());
        assert_eq!(s.stats().completed_waits, completed + 1);
        assert_eq!(s.snapshot(foreign).unwrap().effective_priority, 1);
        assert_eq!(s.mutex_owner(mutex).unwrap(), Some(foreign));
        assert_eq!(s.snapshot(victim).unwrap().state, State::Ready);
        assert_eq!(
            s.take_wake(victim).unwrap(),
            Some(WaitOutcome::terminal(WakeReason::Cancelled))
        );
        s.advance_time(20_000).unwrap();
        assert_eq!(s.stats().completed_waits, completed + 1);
        assert_eq!(switch(&mut s, 0).next, Some(victim));
        // Model the resumed syscall releasing its leases, then reaching its
        // terminal safe point. The foreign owner never unlocks or exits.
        s.exit_current(0, -13).unwrap();
        let retiring = s.schedule(0, 0x9990).unwrap();
        assert_eq!(s.reap(victim), Err(Error::NotRetired));
        s.acknowledge_retirement(retiring.retirement.unwrap())
            .unwrap();
        assert_eq!(s.reap(victim).unwrap().result, -13);
        assert_eq!(s.mutex_owner(mutex).unwrap(), Some(foreign));
        assert_eq!(s.current(1).unwrap(), Some(foreign));
        s.validate().unwrap();
    }
}

#[test]
fn forced_wait_abort_preserves_both_context_retirement_windows() {
    for before_save in [true, false] {
        let mut s = Scheduler::new(3).unwrap();
        let mutex = s.create_mutex().unwrap();
        let condition = s.create_condition().unwrap();
        let victim = spawn(&mut s, 7, 5, 0);
        assert_eq!(switch(&mut s, 0).next, Some(victim));
        s.wait_current(0, &[mutex], None).unwrap();
        s.condition_wait(0, condition, mutex, None).unwrap();
        // Migration makes a premature enqueue observable on CPU 2.
        s.set_affinity(victim, 4).unwrap();
        let retiring = if before_save {
            None
        } else {
            Some(s.schedule(0, 0x9990).unwrap())
        };
        let foreign = spawn(&mut s, 1, 2, 1);
        assert_eq!(switch(&mut s, 1).next, Some(foreign));
        s.wait_current(1, &[mutex], None).unwrap();
        s.signal_condition(condition, false).unwrap();
        assert!(s.abort_wait_for_stop(victim).unwrap());
        assert_eq!(s.snapshot(victim).unwrap().state, State::Blocked);
        assert_eq!(s.snapshot(victim).unwrap().queued_cpu, None);
        assert_eq!(s.schedule(2, 0).unwrap().next, None);
        assert_eq!(s.reap(victim), Err(Error::NotRetired));
        let retiring = retiring.unwrap_or_else(|| s.schedule(0, 0x9990).unwrap());
        s.acknowledge_retirement(retiring.retirement.unwrap())
            .unwrap();
        assert_eq!(switch(&mut s, 2).next, Some(victim));
        assert_eq!(
            s.take_wake(victim).unwrap().unwrap().reason,
            WakeReason::Cancelled
        );
        assert_eq!(s.mutex_owner(mutex).unwrap(), Some(foreign));
        s.validate().unwrap();
    }
}

#[test]
fn suspended_waiters_record_completion_and_running_suspend_retires_safely() {
    let (mut s, task, event) = blocked_event();
    s.suspend(task).unwrap();
    s.signal_event(event).unwrap();
    assert_eq!(s.snapshot(task).unwrap().state, State::Suspended);
    assert_eq!(s.snapshot(task).unwrap().queued_cpu, None);
    s.resume(task).unwrap();
    assert_eq!(switch(&mut s, 0).next, Some(task));
    s.suspend(task).unwrap();
    let retiring = s.schedule(0, 0x7778).unwrap();
    s.resume(task).unwrap();
    assert_eq!(s.snapshot(task).unwrap().queued_cpu, None);
    s.acknowledge_retirement(retiring.retirement.unwrap())
        .unwrap();
    assert_eq!(switch(&mut s, 0).next, Some(task));
    s.validate().unwrap();
}

#[test]
fn invalid_requests_and_capacity_failure_leave_live_state_consistent() {
    assert!(matches!(Scheduler::new(0), Err(Error::InvalidCpu)));
    let mut s = Scheduler::new(1).unwrap();
    assert_eq!(s.create(0, 1, 1), Err(Error::InvalidContext));
    assert_eq!(s.create(8, 8, 1), Err(Error::InvalidPriority));
    assert_eq!(s.create(8, 1, 2), Err(Error::InvalidAffinity));
    for number in 0..MAX_TASKS {
        spawn(&mut s, 1, 1, number as u64);
    }
    assert_eq!(s.create(8, 1, 1), Err(Error::Capacity));
    for _ in 0..MAX_OBJECTS {
        s.create_event(false, false).unwrap();
    }
    assert_eq!(s.create_event(false, false), Err(Error::Capacity));
    switch(&mut s, 0);
    assert_eq!(s.wait_current(0, &[], None), Err(Error::EmptyWait));
    s.advance_time(10).unwrap();
    assert_eq!(s.advance_time(9), Err(Error::ClockReversed));
    s.validate().unwrap();
}

#[test]
fn repeated_smp_dispatch_migration_and_wait_completion_preserve_all_memberships() {
    let mut s = Scheduler::new(4).unwrap();
    let mut tasks = [TaskId {
        slot: 0,
        generation: 0,
    }; MAX_TASKS];
    for (index, task) in tasks.iter_mut().enumerate() {
        *task = spawn(&mut s, (index % 8) as u8, 15, index as u64);
    }
    let event = s.create_event(true, false).unwrap();
    for tick in 0..512 {
        s.advance_time(tick).unwrap();
        for cpu in 0..4 {
            if let Some(current) = s.current(cpu).unwrap() {
                if tick % 7 == 0 {
                    s.wait_current(cpu, &[event], Some(tick + 2)).unwrap();
                } else if tick % 11 == 0 {
                    s.set_affinity(current, 1 << ((cpu + 1) % 4)).unwrap();
                }
            }
            switch(&mut s, cpu);
        }
        if tick % 5 == 0 {
            s.signal_event(event).unwrap();
            s.reset_event(event).unwrap();
        }
        s.validate().unwrap();
    }
    for task in tasks {
        assert!(s.snapshot(task).unwrap().dispatches > 0);
    }
    assert!(s.stats().migrations > 0);
    assert!(s.stats().completed_waits > 0);
}

#[test]
fn observational_cpu_samples_count_actual_current_tasks_and_ignore_duplicate_ticks() {
    let mut s = Scheduler::new(1).unwrap();
    s.advance_time(1).unwrap();
    assert_eq!((s.stats().total_samples, s.stats().busy_samples), (1, 0));
    let task = spawn(&mut s, 2, 1, 0);
    switch(&mut s, 0);
    s.advance_time(2).unwrap();
    assert_eq!(s.snapshot(task).unwrap().samples, 1);
    s.advance_time(2).unwrap();
    assert_eq!(s.snapshot(task).unwrap().samples, 1);
    assert_eq!((s.stats().total_samples, s.stats().busy_samples), (2, 1));
    let event = s.create_event(false, false).unwrap();
    s.wait_current(0, &[event], Some(10)).unwrap();
    switch(&mut s, 0);
    s.advance_time(3).unwrap();
    assert_eq!((s.stats().total_samples, s.stats().busy_samples), (3, 1));
    assert_eq!(s.advance_time(1), Err(Error::ClockReversed));
    assert_eq!(s.stats().total_samples, 3);
}

#[test]
fn address_waits_wake_oldest_first_and_never_lose_a_wake() {
    let mut s = Scheduler::new(1).unwrap();
    let a = spawn(&mut s, 3, 1, 0);
    let b = spawn(&mut s, 3, 1, 1);
    let key = 0x0007_0000_1234_5678;
    // a reads the sequence and blocks.
    assert_eq!(switch(&mut s, 0).next, Some(a));
    let seen = s.address_sequence(key);
    assert_eq!(
        s.wait_address(0, key, seen, None).unwrap(),
        WaitStatus::Blocked
    );
    // b reads the sequence, a wake lands before b blocks: b returns at once.
    assert_eq!(switch(&mut s, 0).next, Some(b));
    let stale = s.address_sequence(key);
    assert_eq!(s.wake_address(key, 1).unwrap(), 1);
    assert!(matches!(
        s.wait_address(0, key, stale, None).unwrap(),
        WaitStatus::Completed(WaitOutcome {
            reason: WakeReason::Signaled,
            ..
        })
    ));
    assert_eq!(s.snapshot(a).unwrap().state, State::Ready);
    // A wake with nobody waiting wakes nobody; another key wakes nobody.
    assert_eq!(s.wake_address(key, usize::MAX).unwrap(), 0);
    let seen = s.address_sequence(key);
    assert_eq!(
        s.wait_address(0, key, seen, Some(5)).unwrap(),
        WaitStatus::Blocked
    );
    assert_eq!(s.wake_address(key + 4, usize::MAX).unwrap(), 0);
    // The deadline still applies.
    s.advance_time(5).unwrap();
    assert_eq!(s.snapshot(b).unwrap().state, State::Blocked);
    switch(&mut s, 0);
    assert_ne!(s.snapshot(b).unwrap().state, State::Blocked);
    s.validate().unwrap();
}

#[test]
fn a_scheduler_built_in_place_matches_new() {
    extern crate std;
    let mut storage = std::boxed::Box::new(core::mem::MaybeUninit::<Scheduler>::uninit());
    unsafe { Scheduler::initialize_at(storage.as_mut_ptr(), 2) }.unwrap();
    let s = unsafe { storage.assume_init_mut() };
    s.validate().unwrap();
    assert_eq!(s.online_cpus(), 2);
    let a = spawn(s, 3, 3, 0);
    assert_eq!(switch(s, 0).next, Some(a));
    let event = s.create_event(false, false).unwrap();
    assert_eq!(s.signal_event(event).unwrap(), 0);
    // Every task slot is usable.
    for number in 1..MAX_TASKS as u64 {
        spawn(s, 3, 3, number);
    }
    assert_eq!(s.create(0x9000, 3, 3), Err(Error::Capacity));
    let mut bad = std::boxed::Box::new(core::mem::MaybeUninit::<Scheduler>::uninit());
    assert_eq!(
        unsafe { Scheduler::initialize_at(bad.as_mut_ptr(), 0) },
        Err(Error::InvalidCpu)
    );
}
