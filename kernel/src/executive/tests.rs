use super::*;
use door_abi::{Header, MAX_PACKET, Transfer};
#[test]
fn diagnostic_budget_is_process_owned_and_failed_charges_do_not_consume_credit() {
    let mut e = Executive::boxed();
    let a = e.create_process().unwrap();
    let b = e.create_process().unwrap();
    assert_eq!(
        e.charge_diagnostic(a, door_abi::MAX_DIAGNOSTIC + 1),
        Err(Status::InvalidArgument)
    );
    for _ in 0..door_abi::PROCESS_DIAGNOSTIC_BUDGET / door_abi::MAX_DIAGNOSTIC {
        e.charge_diagnostic(a, door_abi::MAX_DIAGNOSTIC).unwrap();
    }
    assert_eq!(e.charge_diagnostic(a, 1), Err(Status::ResourceExhausted));
    e.charge_diagnostic(b, door_abi::MAX_DIAGNOSTIC).unwrap();
    e.exit(a, 0).unwrap();
    e.reap(a).unwrap();
    let c = e.create_process().unwrap();
    e.charge_diagnostic(c, door_abi::MAX_DIAGNOSTIC).unwrap();
    assert_eq!(e.charge_diagnostic(a, 1), Err(Status::NotFound));
}
fn packet(request: u64, count: u16) -> [u8; 32] {
    let mut p = [0; 32];
    Header {
        opcode: 1,
        flags: 0,
        request_id: request,
        payload_bytes: 0,
        handle_count: count,
    }
    .encode(&mut p)
    .unwrap();
    p
}
fn section(e: &mut Executive, p: ProcessId) -> u64 {
    e.create_section(
        p,
        1,
        4096,
        rights::QUERY
            | rights::READ
            | rights::WRITE
            | rights::MAP
            | rights::DUPLICATE
            | rights::TRANSFER,
    )
    .unwrap()
}
#[test]
fn authority_is_local_reduced_and_stale_handles_never_alias() {
    let mut e = Executive::boxed();
    let a = e.create_process().unwrap();
    let b = e.create_process().unwrap();
    let h = section(&mut e, a);
    assert_eq!(e.inspect(b, h), Err(Status::BadHandle));
    let reduced = e
        .duplicate(a, h, rights::QUERY | rights::READ | rights::MAP)
        .unwrap();
    assert_eq!(
        e.duplicate(a, reduced, rights::ALL),
        Err(Status::AccessDenied)
    );
    assert!(matches!(
        e.acquire_section(a, reduced, rights::WRITE | rights::READ),
        Err(Status::AccessDenied)
    ));
    e.close(a, h).unwrap();
    let next = e.duplicate(a, reduced, rights::READ);
    assert_eq!(next, Err(Status::AccessDenied));
    assert_eq!(e.inspect(a, h), Err(Status::BadHandle));
    e.close(a, reduced).unwrap();
    assert_eq!(e.take_retired_section(), Some(1));
}
#[test]
fn shared_mapping_lease_survives_last_handle_until_unmap() {
    let mut e = Executive::boxed();
    let a = e.create_process().unwrap();
    let h = section(&mut e, a);
    let lease = e.acquire_section(a, h, rights::READ).unwrap();
    e.close(a, h).unwrap();
    assert_eq!(e.take_retired_section(), None);
    e.release_lease(lease.lease);
    assert_eq!(e.take_retired_section(), Some(1));
    assert_eq!(e.take_retired_section(), None);
}
#[test]
fn ipc_identity_rights_and_move_commit_are_atomic() {
    let mut e = Executive::boxed();
    let a = e.create_process().unwrap();
    let b = e.create_process().unwrap();
    let [send, recv] = e.channel_pair(a, b).unwrap();
    let h = section(&mut e, a);
    let transfer = Transfer {
        handle: h,
        rights: rights::QUERY | rights::READ | rights::MAP,
        mode: 1,
    };
    let bad = Transfer {
        rights: rights::ALL,
        ..transfer
    };
    assert_eq!(
        e.send(a, send, &packet(1, 1), &[bad]),
        Err(Status::AccessDenied)
    );
    assert!(e.inspect(a, h).is_ok());
    e.send(a, send, &packet(1, 1), &[transfer]).unwrap();
    assert_eq!(e.inspect(a, h), Err(Status::BadHandle));
    assert!(matches!(
        e.receive(b, recv, 31, 4),
        Err(Status::InvalidArgument)
    ));
    assert_eq!(e.receive_info(b, recv).unwrap().transfers, 1);
    let m = e.receive(b, recv, MAX_PACKET, 4).unwrap();
    assert_eq!(m.metadata.sender_pid, a.raw());
    assert_eq!(m.metadata.request_id, 1);
    let got = e.inspect(b, m.transfers[0].handle).unwrap();
    assert_eq!(got.rights, transfer.rights);
    assert!(matches!(
        e.acquire_section(b, m.transfers[0].handle, rights::READ | rights::WRITE),
        Err(Status::AccessDenied)
    ));
    e.exit(a, 11).unwrap();
    e.exit(b, 22).unwrap();
    assert_eq!(e.take_retired_section(), Some(1));
    assert_eq!(e.object_count(), 2);
    e.reap(a).unwrap();
    e.reap(b).unwrap();
    assert_eq!(e.object_count(), 0);
}
#[test]
fn queue_backpressure_does_not_consume_request_or_move_grants() {
    let mut e = Executive::boxed();
    let a = e.create_process().unwrap();
    let b = e.create_process().unwrap();
    let [s, r] = e.channel_pair(a, b).unwrap();
    let h = section(&mut e, a);
    for id in 1..=4 {
        e.send(a, s, &packet(id, 0), &[]).unwrap();
    }
    let t = Transfer {
        handle: h,
        rights: rights::READ,
        mode: 1,
    };
    assert_eq!(e.send(a, s, &packet(5, 1), &[t]), Err(Status::WouldBlock));
    assert!(e.inspect(a, h).is_ok());
    e.receive(b, r, MAX_PACKET, 4).unwrap();
    e.send(a, s, &packet(5, 1), &[t]).unwrap();
    assert_eq!(e.inspect(a, h), Err(Status::BadHandle));
    for id in 2..=5 {
        assert_eq!(
            e.receive(b, r, MAX_PACKET, 4).unwrap().metadata.request_id,
            id
        );
    }
    assert_eq!(
        e.send(a, s, &packet(5, 0), &[]),
        Err(Status::InvalidArgument)
    );
}
#[test]
fn receive_capacity_failure_retains_queue_and_grants() {
    let mut e = Executive::boxed();
    let a = e.create_process().unwrap();
    let b = e.create_process().unwrap();
    let [s, r] = e.channel_pair(a, b).unwrap();
    let h = section(&mut e, a);
    let t = Transfer {
        handle: h,
        rights: rights::READ | rights::QUERY,
        mode: 0,
    };
    e.send(a, s, &packet(1, 1), &[t]).unwrap();
    let mut handles = [0; MAX_HANDLES - 1];
    for h in &mut handles {
        *h = e.duplicate(b, r, rights::QUERY).unwrap();
    }
    assert!(matches!(
        e.receive(b, r, MAX_PACKET, 4),
        Err(Status::ResourceExhausted)
    ));
    assert_eq!(e.receive_info(b, r).unwrap().transfers, 1);
    e.close(b, handles[0]).unwrap();
    let m = e.receive(b, r, MAX_PACKET, 4).unwrap();
    assert_eq!(m.metadata.sender_pid, a.raw());
    assert_eq!(
        e.inspect(b, m.transfers[0].handle).unwrap().rights,
        t.rights
    );
}
#[test]
fn peer_closure_drains_committed_messages_and_discards_unread_grants() {
    let mut e = Executive::boxed();
    let a = e.create_process().unwrap();
    let b = e.create_process().unwrap();
    let [s, r] = e.channel_pair(a, b).unwrap();
    let h = section(&mut e, a);
    e.send(
        a,
        s,
        &packet(1, 1),
        &[Transfer {
            handle: h,
            rights: rights::READ,
            mode: 1,
        }],
    )
    .unwrap();
    e.close(a, s).unwrap();
    assert!(e.receive_info(b, r).is_ok());
    e.close(b, r).unwrap();
    assert_eq!(e.take_retired_section(), Some(1));
    let [s, r] = e.channel_pair(a, b).unwrap();
    e.send(a, s, &packet(1, 0), &[]).unwrap();
    e.close(a, s).unwrap();
    e.receive(b, r, MAX_PACKET, 4).unwrap();
    assert_eq!(e.receive_info(b, r), Err(Status::PeerClosed));
}
#[test]
fn process_wait_record_outlives_target_reap_and_pid_reuse() {
    let mut e = Executive::boxed();
    let a = e.create_process().unwrap();
    let b = e.create_process().unwrap();
    let h = e
        .process_handle(a, b, rights::WAIT | rights::QUERY)
        .unwrap();
    assert_eq!(e.wait_process(a, h), Ok((b.raw(), None)));
    e.exit(b, -14).unwrap();
    e.reap(b).unwrap();
    let c = e.create_process().unwrap();
    assert_ne!(c.raw(), b.raw());
    assert_eq!(e.wait_process(a, h), Ok((b.raw(), Some(-14))));
    e.exit(a, 0).unwrap();
    e.reap(a).unwrap();
    e.exit(c, 0).unwrap();
    e.reap(c).unwrap();
    assert_eq!(e.object_count(), 0);
}
#[test]
fn duplicate_transfer_and_endpoint_cycles_rejected_without_mutation() {
    let mut e = Executive::boxed();
    let a = e.create_process().unwrap();
    let [s, _] = e.channel_pair(a, a).unwrap();
    let h = section(&mut e, a);
    let t = Transfer {
        handle: h,
        rights: rights::READ,
        mode: 1,
    };
    assert_eq!(
        e.send(a, s, &packet(1, 2), &[t, t]),
        Err(Status::InvalidArgument)
    );
    assert!(e.inspect(a, h).is_ok());
    assert_eq!(
        e.send(
            a,
            s,
            &packet(1, 1),
            &[Transfer {
                handle: s,
                rights: rights::READ,
                mode: 0
            }]
        ),
        Err(Status::AccessDenied)
    );
    assert!(matches!(e.receive_info(a, s), Err(Status::WouldBlock)));
}

#[test]
fn highest_process_slot_retires_and_reuses_without_reviving_stale_identity() {
    let mut e = Executive::boxed();
    let ids: [ProcessId; MAX_PROCESSES] = core::array::from_fn(|_| e.create_process().unwrap());
    assert_eq!(e.create_process(), Err(Status::ResourceExhausted));
    let last = ids[MAX_PROCESSES - 1];
    assert_eq!(last.slot(), MAX_PROCESSES - 1);
    e.exit(last, 0).unwrap();
    e.reap(last).unwrap();
    let replacement = e.create_process().unwrap();
    assert_eq!(replacement.slot(), last.slot());
    assert_ne!(replacement.raw(), last.raw());
    assert_eq!(e.charge_diagnostic(last, 1), Err(Status::NotFound));
    e.charge_diagnostic(replacement, 1).unwrap();
    for id in ids[..MAX_PROCESSES - 1]
        .iter()
        .copied()
        .chain(core::iter::once(replacement))
    {
        e.exit(id, 0).unwrap();
        e.reap(id).unwrap();
    }
    assert!(e.processes.iter().all(Option::is_none));
}
#[test]
fn resource_sections_are_one_pinned_read_only_object_shared_by_grants() {
    let mut e = Executive::boxed();
    let a = e.create_process().unwrap();
    let b = e.create_process().unwrap();
    assert_eq!(
        e.create_resource_section(7, 4095),
        Err(Status::InvalidArgument)
    );
    e.create_resource_section(7, 8192).unwrap();
    assert_eq!(
        e.create_resource_section(7, 8192),
        Err(Status::AlreadyExists)
    );
    // A process section with the same rights is never mistaken for a resource.
    e.create_section(a, 9, 4096, RESOURCE_RIGHTS).unwrap();
    assert_eq!(e.grant_resource_section(b, 9), Err(Status::NotFound));
    let ha = e.grant_resource_section(a, 7).unwrap();
    let hb = e.grant_resource_section(b, 7).unwrap();
    for (p, h) in [(a, ha), (b, hb)] {
        let info = e.inspect(p, h).unwrap();
        assert_eq!(info.rights, RESOURCE_RIGHTS);
        assert_eq!(info.kind, HandleKind::Section);
        assert!(matches!(
            e.acquire_section(p, h, rights::READ | rights::WRITE),
            Err(Status::AccessDenied)
        ));
        assert_eq!(e.duplicate(p, h, rights::READ), Err(Status::AccessDenied));
        let lease = e.acquire_section(p, h, rights::READ).unwrap();
        assert_eq!((lease.key, lease.bytes), (7, 8192));
        e.release_lease(lease.lease);
    }
    assert_eq!(
        e.inspect(a, ha).unwrap().object,
        e.inspect(b, hb).unwrap().object
    );
    // Grantees come and go; the pin keeps the one copy alive.
    e.exit(a, 0).unwrap();
    e.reap(a).unwrap();
    assert_eq!(e.take_retired_section(), Some(9));
    assert_eq!(e.take_retired_section(), None);
    let c = e.create_process().unwrap();
    let hc = e.grant_resource_section(c, 7).unwrap();
    // Shutdown: release the pin; the last handle then retires the section.
    e.release_resource_section(7).unwrap();
    assert_eq!(e.release_resource_section(7), Err(Status::NotFound));
    assert_eq!(e.grant_resource_section(c, 7), Err(Status::NotFound));
    e.close(b, hb).unwrap();
    assert_eq!(e.take_retired_section(), None);
    let lease = e.acquire_section(c, hc, rights::READ).unwrap();
    e.close(c, hc).unwrap();
    assert_eq!(
        e.take_retired_section(),
        None,
        "a mapped view still leases it"
    );
    e.release_lease(lease.lease);
    assert_eq!(e.take_retired_section(), Some(7));
    for key in 20..20 + MAX_RESOURCES as u64 {
        e.create_resource_section(key, 4096).unwrap();
    }
    assert_eq!(
        e.create_resource_section(99, 4096),
        Err(Status::ResourceExhausted)
    );
}
