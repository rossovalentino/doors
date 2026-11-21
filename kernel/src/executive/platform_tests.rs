use super::*;
use crate::task::Scheduler;
use door_abi::{Header, Transfer};

fn system(e: &mut Executive, manifest: u64) -> ProcessId {
    let p = e.create_process().unwrap();
    e.assign_authority(
        p,
        Authority {
            uid: 1,
            integrity: 2,
            groups: 4,
            manifest,
            instance: p.raw(),
            capabilities: 1 << (manifest - 1),
            ..Authority::default()
        },
    )
    .unwrap();
    p
}
fn copy(e: &mut Executive, source: ProcessId, handle: u64, target: ProcessId, access: u64) -> u64 {
    e.copy_grant_to(source, handle, target, access).unwrap()
}
fn make_session(e: &mut Executive, init: ProcessId, auth: ProcessId, uid: u32) -> (u64, u64) {
    let permit = e.provision_permit(init, uid).unwrap();
    let permit = copy(e, init, permit, auth, rights::READ);
    let authority = e.create_role_authority(auth, HandleKind::Session).unwrap();
    let session = e.create_session(auth, authority, permit).unwrap();
    (authority, session)
}
fn make_user(e: &mut Executive, init: ProcessId, auth: ProcessId, session: u64) -> ProcessId {
    let session = copy(
        e,
        auth,
        session,
        init,
        rights::QUERY | rights::READ | rights::TRANSFER,
    );
    let launch = e.create_role_authority(init, HandleKind::Launch).unwrap();
    let authority = e
        .authority_for_launch(init, launch, 5, session, 99)
        .unwrap();
    let p = e.create_process().unwrap();
    e.assign_authority(p, authority).unwrap();
    p
}

#[test]
fn session_query_requires_a_live_query_grant_and_never_mints_authority() {
    let mut e = Executive::boxed();
    let init = system(&mut e, 1);
    let auth = system(&mut e, 3);
    let (authority, session) = make_session(&mut e, init, auth, 1001);
    let (id, uid) = e.session_info(auth, session).unwrap();
    assert_ne!(id, 0);
    assert_eq!(uid, 1001);
    let reduced = copy(&mut e, auth, session, init, rights::READ);
    assert_eq!(e.session_info(init, reduced), Err(Status::AccessDenied));
    let queried = copy(&mut e, auth, session, init, rights::QUERY);
    assert_eq!(e.session_info(init, queried), Ok((id, 1001)));
    assert_eq!(
        e.session_info(auth, authority),
        Err(Status::InvalidArgument)
    );
    e.revoke_session(auth, authority, session).unwrap();
    assert_eq!(e.session_info(init, queried), Err(Status::AccessDenied));
}

#[test]
fn port_connect_preserves_owner_and_reduced_grants_reject_impersonation() {
    let mut e = Executive::boxed();
    let server = system(&mut e, 1);
    let client = system(&mut e, 2);
    let port = e.create_port(server).unwrap();
    let connect = copy(&mut e, server, port, client, rights::QUERY | rights::WRITE);
    assert_eq!(e.accept_port(client, connect), Err(Status::AccessDenied));
    let client_end = e.connect_port(client, connect).unwrap();
    let server_end = e.accept_port(server, port).unwrap();
    assert_eq!(e.channel_peer(server, server_end), Ok(client.raw()));
    assert_eq!(e.channel_peer(client, client_end), Ok(server.raw()));
    let identity = e.channel_identity(server, server_end).unwrap();
    assert_eq!(identity.1.manifest, 2);
    assert_eq!(e.accept_port(server, port), Err(Status::WouldBlock));
    e.exit(client, 0).unwrap();
    e.reap(client).unwrap();
    e.exit(server, 0).unwrap();
    e.reap(server).unwrap();
    assert_eq!(e.object_count(), 0);
}

#[test]
fn full_pending_port_is_precommit_and_server_death_closes_unaccepted_endpoints() {
    let mut e = Executive::boxed();
    let server = system(&mut e, 1);
    let client = system(&mut e, 2);
    let port = e.create_port(server).unwrap();
    let connect = copy(&mut e, server, port, client, rights::WRITE);
    let channels: [u64; 4] = core::array::from_fn(|_| e.connect_port(client, connect).unwrap());
    let baseline = (
        e.object_count(),
        e.handle_count(server).unwrap(),
        e.handle_count(client).unwrap(),
        e.next_handle,
    );
    assert_eq!(e.connect_port(client, connect), Err(Status::WouldBlock));
    assert_eq!(
        (
            e.object_count(),
            e.handle_count(server).unwrap(),
            e.handle_count(client).unwrap(),
            e.next_handle
        ),
        baseline
    );
    e.exit(server, 0).unwrap();
    e.reap(server).unwrap();
    assert_eq!(e.connect_port(client, connect), Err(Status::PeerClosed));
    for channel in channels {
        assert_eq!(e.receive_info(client, channel), Err(Status::PeerClosed));
    }
    e.exit(client, 0).unwrap();
    e.reap(client).unwrap();
    assert_eq!(e.object_count(), 0);
}

#[test]
fn permit_consumption_is_object_global_and_failed_session_creation_does_not_consume() {
    let mut e = Executive::boxed();
    let init = system(&mut e, 1);
    let auth = system(&mut e, 3);
    let permit = e.provision_permit(init, 1001).unwrap();
    let permit_auth = copy(&mut e, init, permit, auth, rights::READ);
    let another = copy(&mut e, init, permit, auth, rights::READ);
    let authority = e.create_role_authority(auth, HandleKind::Session).unwrap();
    let initial_objects = e.object_count();
    while e.handle_count(auth).unwrap() < MAX_HANDLES {
        e.copy_grant_to(init, permit, auth, rights::READ).unwrap();
    }
    assert_eq!(
        e.create_session(auth, authority, permit_auth),
        Err(Status::ResourceExhausted)
    );
    assert_eq!(e.object_count(), initial_objects);
    e.close(auth, another).unwrap();
    let session = e.create_session(auth, authority, permit_auth).unwrap();
    let copied = e.copy_grant_to(init, permit, auth, rights::READ).unwrap();
    assert_eq!(
        e.create_session(auth, authority, copied),
        Err(Status::AccessDenied)
    );
    assert!(e.inspect(auth, session).is_ok());
    assert_eq!(
        e.create_session(init, permit, permit),
        Err(Status::AccessDenied)
    );
}

#[test]
fn revoked_session_denies_existing_peer_identity_and_new_launch_without_aliasing_reuse() {
    let mut e = Executive::boxed();
    let init = system(&mut e, 1);
    let auth = system(&mut e, 3);
    let (authority, session) = make_session(&mut e, init, auth, 1001);
    let user = make_user(&mut e, init, auth, session);
    let pair = e.channel_pair(user, auth).unwrap();
    let before = e.channel_identity(auth, pair[1]).unwrap().1;
    assert_eq!(before.uid, 1001);
    e.revoke_session(auth, authority, session).unwrap();
    assert_eq!(e.authority(user), Err(Status::AccessDenied));
    assert_eq!(e.channel_identity(auth, pair[1]), Err(Status::AccessDenied));
    assert_eq!(e.session_processes(before.session)[user.slot()], Some(user));
    // Reuse the bounded session storage with a new nonwrapping identity.
    let (_, second) = make_session(&mut e, init, auth, 1002);
    let second_user = make_user(&mut e, init, auth, second);
    assert_ne!(e.authority(second_user).unwrap().session, before.session);
    assert_eq!(e.channel_identity(auth, pair[1]), Err(Status::AccessDenied));
    assert_eq!(e.authority(second_user).unwrap().uid, 1002);
}

#[test]
fn an_administrator_accounts_applications_carry_the_deny_only_group() {
    use door_abi::platform::{group, manifest};
    let mut e = Executive::boxed();
    let init = system(&mut e, manifest::INIT);
    let auth = system(&mut e, manifest::SESSION);
    let launch = e.create_role_authority(init, HandleKind::Launch).unwrap();
    let session_authority = e.create_role_authority(auth, HandleKind::Session).unwrap();
    // Account 1001 is an administrator (bit 0), 1002 a standard user.
    let mut sessions = [0; 2];
    for (i, uid) in [1001, 1002].into_iter().enumerate() {
        let permit = e.provision_permit(init, uid).unwrap();
        let permit = copy(&mut e, init, permit, auth, rights::READ);
        assert_eq!(
            e.create_session_typed(auth, session_authority, permit, 4),
            Err(Status::InvalidArgument)
        );
        let session = e
            .create_session_typed(auth, session_authority, permit, 1)
            .unwrap();
        sessions[i] = copy(&mut e, auth, session, init, rights::READ);
    }
    let admin = e
        .authority_for_launch(init, launch, manifest::WINDOWS, sessions[0], 100)
        .unwrap();
    let standard = e
        .authority_for_launch(init, launch, manifest::WINDOWS, sessions[1], 101)
        .unwrap();
    assert_eq!(admin.groups, group::USERS | group::ADMINISTRATOR_ACCOUNT);
    assert_eq!(standard.groups, group::USERS);
    // The loader cannot claim or drop the account's type.
    let child = e.create_process().unwrap();
    let forged = Authority {
        groups: group::USERS | group::ADMINISTRATOR_ACCOUNT,
        ..standard
    };
    assert_eq!(e.assign_authority(child, forged), Err(Status::AccessDenied));
    let dropped = Authority {
        groups: group::USERS,
        ..admin
    };
    assert_eq!(
        e.assign_authority(child, dropped),
        Err(Status::AccessDenied)
    );
    // A full token keeps the account bit beside the enabled group.
    let full = Authority {
        groups: admin.groups | group::ADMINISTRATORS,
        ..admin
    };
    e.assign_authority(child, full).unwrap();
}
#[test]
fn windows_launch_requires_a_live_user_session_and_cannot_mint_service_authority() {
    use door_abi::platform::manifest;
    let mut e = Executive::boxed();
    let init = system(&mut e, manifest::INIT);
    let auth = system(&mut e, manifest::SESSION);
    let launch = e.create_role_authority(init, HandleKind::Launch).unwrap();
    assert_eq!(
        e.authority_for_launch(init, launch, manifest::WINDOWS, 0, 100),
        Err(Status::AccessDenied)
    );
    let (session_authority, session) = make_session(&mut e, init, auth, 1001);
    let grant = copy(&mut e, auth, session, init, rights::READ);
    let authority = e
        .authority_for_launch(init, launch, manifest::WINDOWS, grant, 100)
        .unwrap();
    assert_eq!(
        (authority.uid, authority.integrity, authority.groups),
        (1001, 1, 2)
    );
    assert_eq!(authority.capabilities, 0);
    assert_ne!(authority.session, 0);
    assert_eq!(authority.epoch, authority.session);
    let child = e.create_process().unwrap();
    e.assign_authority(child, authority).unwrap();
    let cwd = e.create_directory(child).unwrap();
    assert_eq!(e.directory_uid(child, cwd), Ok(1001));
    for kind in [
        HandleKind::Launch,
        HandleKind::Block,
        HandleKind::Session,
        HandleKind::Console,
    ] {
        assert_eq!(
            e.create_role_authority(child, kind),
            Err(Status::AccessDenied)
        );
    }
    // The same session cannot turn a user executable into a privileged service.
    assert_eq!(
        e.authority_for_launch(init, launch, manifest::STORAGE, grant, 101),
        Err(Status::AccessDenied)
    );
    e.revoke_session(auth, session_authority, session).unwrap();
    assert_eq!(e.authority(child), Err(Status::AccessDenied));
    assert_eq!(
        e.authority_for_launch(init, launch, manifest::WINDOWS, grant, 102),
        Err(Status::AccessDenied)
    );
    for process in [child, auth, init] {
        e.exit(process, 0).unwrap();
        e.reap(process).unwrap();
    }
    assert_eq!(e.object_count(), 0);
    assert_eq!(e.active_sessions(), 0);
}

#[test]
fn launch_publication_revalidates_session_and_later_revocation_stops_new_task() {
    for revoke_before_publication in [true, false] {
        let mut e = Executive::boxed();
        let mut scheduler = Scheduler::new(1).unwrap();
        let stops = crate::task::StopRequests::new();
        let init = system(&mut e, 1);
        let auth = system(&mut e, 3);
        let (authority, session) = make_session(&mut e, init, auth, 1001);
        let child = make_user(&mut e, init, auth, session);
        let baseline = e.object_count() - 1; // Exclude the new process object.
        let session_id = e.authority(child).unwrap().session;
        e.create_directory(child).unwrap();
        let event = scheduler.create_event(true, false).unwrap();
        let task = scheduler.create(0x1000, 4, 1).unwrap();
        if revoke_before_publication {
            e.revoke_session(auth, authority, session).unwrap();
            // VM/context construction took place outside the metadata guard.
            // The actual final publication method must reject this child.
            assert_eq!(e.set_process_wait(child, event), Err(Status::AccessDenied));
            assert!(matches!(
                e.object(e.process(child).unwrap().object).kind,
                Kind::Process { wait: None, .. }
            ));
            scheduler.terminate(task, -1).unwrap();
            scheduler.reap(task).unwrap();
            e.exit(child, -1).unwrap();
            e.reap(child).unwrap();
            // Rejected publication never transferred event ownership; rollback
            // closes it directly, without a second queued retirement.
            assert_eq!(e.take_retired_wait(), None);
            scheduler.close_object(event).unwrap();
        } else {
            e.set_process_wait(child, event).unwrap();
            // Runtime task/VM records publish under that same metadata guard.
            e.revoke_session(auth, authority, session).unwrap();
            assert_eq!(e.session_processes(session_id)[child.slot()], Some(child));
            assert_eq!(
                scheduler.snapshot(task).unwrap().state,
                crate::task::State::New
            );
            stops.request(&mut scheduler, task, -13).unwrap();
            scheduler.start(task).unwrap();
            assert_eq!(scheduler.schedule(0, 0).unwrap().next, Some(task));
            // The request survives start and forces the first runtime safe point.
            let code = stops.requested(task).unwrap();
            assert_eq!(code, -13);
            scheduler.terminate(task, code).unwrap();
            let leaving = scheduler.schedule(0, 0x2000).unwrap();
            assert_eq!(scheduler.reap(task), Err(crate::task::Error::NotRetired));
            scheduler
                .acknowledge_retirement(leaving.retirement.unwrap())
                .unwrap();
            assert_eq!(scheduler.reap(task).unwrap().result, -13);
            e.exit(child, code).unwrap();
            scheduler.signal_event(event).unwrap();
            e.reap(child).unwrap();
            assert_eq!(e.take_retired_wait(), Some(event));
            scheduler.close_object(event).unwrap();
        }
        assert_eq!(e.take_retired_wait(), None);
        assert_eq!(e.object_count(), baseline);
        scheduler.validate().unwrap();
    }
}

#[test]
fn wait_and_thread_leases_keep_objects_alive_and_exit_results_outlive_slot_reuse() {
    let mut scheduler = Scheduler::new(1).unwrap();
    let mut e = Executive::boxed();
    let p = system(&mut e, 1);
    let event = scheduler.create_event(true, false).unwrap();
    let handle = e.create_wait(p, HandleKind::Event, event).unwrap();
    let retained = e.acquire_wait(p, handle, rights::WAIT).unwrap();
    e.close(p, handle).unwrap();
    assert!(e.take_retired_wait().is_none());
    e.release_lease(retained.lease);
    assert_eq!(e.take_retired_wait(), Some(event));
    scheduler.close_object(event).unwrap();
    let event = scheduler.create_event(true, false).unwrap();
    let (handle, runtime) = e.create_thread(p, 55, event).unwrap();
    let joined = e.acquire_thread(p, handle).unwrap();
    e.close(p, handle).unwrap();
    e.complete_thread(&runtime, 71).unwrap();
    e.release_lease(runtime);
    assert_eq!(e.thread_result(&joined.lease), Ok(Some(71)));
    assert!(e.take_retired_wait().is_none());
    e.release_lease(joined.lease);
    assert_eq!(e.take_retired_wait(), Some(event));
}

#[test]
fn thread_completion_retains_event_across_last_handle_close_until_signal() {
    for close_before_completion in [true, false] {
        let mut scheduler = Scheduler::new(1).unwrap();
        let mut e = Executive::boxed();
        let process = system(&mut e, 1);
        let baseline = e.object_count();
        let event = scheduler.create_event(true, false).unwrap();
        let (handle, runtime) = e.create_thread(process, 55, event).unwrap();
        let task = scheduler.create(0x1000, 4, 1).unwrap();
        scheduler.start(task).unwrap();
        assert_eq!(scheduler.schedule(0, 0).unwrap().next, Some(task));
        if close_before_completion {
            e.close(process, handle).unwrap();
        }
        scheduler.exit_current(0, -13).unwrap();
        let decision = scheduler.schedule(0, 0x2000).unwrap();
        assert_eq!(scheduler.reap(task), Err(crate::task::Error::NotRetired));
        scheduler
            .acknowledge_retirement(decision.retirement.unwrap())
            .unwrap();
        let result = scheduler.reap(task).unwrap().result;
        e.complete_thread(&runtime, result).unwrap();
        if !close_before_completion {
            // A joiner can observe the published result and close the final
            // user handle while the reaper is preempted before signaling.
            let joined = e.acquire_thread(process, handle).unwrap();
            assert_eq!(e.thread_result(&joined.lease), Ok(Some(-13)));
            e.close(process, handle).unwrap();
            e.release_lease(joined.lease);
        }
        // Model cleanup running in the publication-to-signal window. The
        // runtime lease must exclude this event from its retirement queue.
        assert_eq!(e.handle_count(process).unwrap(), 0);
        assert_eq!(e.take_retired_wait(), None);
        assert_eq!(scheduler.signal_event(event), Ok(0));
        e.release_lease(runtime);
        assert_eq!(e.take_retired_wait(), Some(event));
        scheduler.close_object(event).unwrap();
        assert_eq!(e.take_retired_wait(), None);
        assert_eq!(e.object_count(), baseline);
        let replacement = scheduler.create_event(true, false).unwrap();
        assert_eq!(replacement.slot(), event.slot());
        assert_ne!(replacement.generation(), event.generation());
        assert_eq!(
            scheduler.signal_event(event),
            Err(crate::task::Error::InvalidObject)
        );
        scheduler.close_object(replacement).unwrap();
        scheduler.validate().unwrap();
    }
}

#[test]
fn queued_port_transfer_releases_after_owner_death_and_cannot_transfer_role_authority() {
    let mut e = Executive::boxed();
    let a = system(&mut e, 1);
    let b = system(&mut e, 2);
    let pair = e.channel_pair(a, b).unwrap();
    let port = e.create_port(a).unwrap();
    let authority = e.create_role_authority(a, HandleKind::Launch).unwrap();
    let mut packet = [0; 32];
    Header {
        opcode: 1,
        flags: 0,
        request_id: 1,
        payload_bytes: 0,
        handle_count: 1,
    }
    .encode(&mut packet)
    .unwrap();
    assert_eq!(
        e.send(
            a,
            pair[0],
            &packet,
            &[Transfer {
                handle: authority,
                rights: rights::WRITE,
                mode: 0
            }]
        ),
        Err(Status::AccessDenied)
    );
    e.send(
        a,
        pair[0],
        &packet,
        &[Transfer {
            handle: port,
            rights: rights::WRITE,
            mode: 1,
        }],
    )
    .unwrap();
    e.exit(a, 0).unwrap();
    e.reap(a).unwrap();
    let delivered = e.receive(b, pair[1], 32, 1).unwrap();
    assert_eq!(
        e.connect_port(b, delivered.transfers[0].handle),
        Err(Status::PeerClosed)
    );
    e.exit(b, 0).unwrap();
    e.reap(b).unwrap();
    assert_eq!(e.object_count(), 0);
}

#[test]
fn session_publication_failure_and_process_retirement_reclaim_bounded_slots() {
    let mut e = Executive::boxed();
    let init = system(&mut e, 1);
    let auth = system(&mut e, 3);
    let manager = e.create_role_authority(auth, HandleKind::Session).unwrap();
    let launch = e.create_role_authority(init, HandleKind::Launch).unwrap();
    let baseline = e.object_count();
    for cycle in 0..32 {
        let permit = e.provision_permit(init, 1001).unwrap();
        let delivered = copy(&mut e, init, permit, auth, rights::READ);
        e.close(init, permit).unwrap();
        let session = e.create_session(auth, manager, delivered).unwrap();
        if cycle % 2 == 0 {
            // Models a failed response or discarded queued grant: the final
            // capability reference is gone before any process binds it.
            e.close(auth, session).unwrap();
        } else {
            let parent = copy(&mut e, auth, session, init, rights::READ);
            let authority = e
                .authority_for_launch(init, launch, 5, parent, 100 + cycle)
                .unwrap();
            let process = e.create_process().unwrap();
            e.assign_authority(process, authority).unwrap();
            e.close(init, parent).unwrap();
            e.close(auth, session).unwrap();
            assert_eq!(e.authority(process), Ok(authority));
            e.exit(process, 0).unwrap();
            // The execution owner still holds process/session state until its
            // architecture and VM retirement permit the executive reap.
            assert!(e.validate_authority(authority).is_ok());
            e.reap(process).unwrap();
            assert_eq!(e.validate_authority(authority), Err(Status::AccessDenied));
        }
        assert_eq!(e.object_count(), baseline);
        assert_eq!(e.active_sessions(), 0);
    }
}

#[test]
fn file_app_admission_revalidates_session_and_cannot_create_service_grants() {
    use door_abi::platform::manifest;
    for id in [
        manifest::SHELL,
        manifest::APPLICATION,
        manifest::WINDOWS_APPLICATION,
    ] {
        let mut e = Executive::boxed();
        let init = system(&mut e, manifest::INIT);
        let auth = system(&mut e, manifest::SESSION);
        let launch = e.create_role_authority(init, HandleKind::Launch).unwrap();
        assert_eq!(
            e.authority_for_launch(init, launch, id, 0, 100),
            Err(Status::AccessDenied)
        );
        let (session_authority, session) = make_session(&mut e, init, auth, 1001);
        let grant = copy(&mut e, auth, session, init, rights::READ);
        let authority = e
            .authority_for_launch(init, launch, id, grant, 100)
            .unwrap();
        let child = e.create_process().unwrap();
        e.assign_authority(child, authority).unwrap();
        for kind in [
            HandleKind::Launch,
            HandleKind::Block,
            HandleKind::Session,
            HandleKind::Console,
        ] {
            assert_eq!(
                e.create_role_authority(child, kind),
                Err(Status::AccessDenied)
            );
        }
        e.revoke_session(auth, session_authority, session).unwrap();
        assert_eq!(
            e.set_process_wait(
                child,
                crate::task::Scheduler::new(1)
                    .unwrap()
                    .create_event(true, false)
                    .unwrap()
            ),
            Err(Status::AccessDenied)
        );
    }
}

#[test]
fn a_permit_is_renewed_for_init_only_after_its_accounts_session_ends() {
    let mut e = Executive::boxed();
    let init = system(&mut e, 1);
    let auth = system(&mut e, 3);
    let launch = e.create_role_authority(init, HandleKind::Launch).unwrap();
    let (authority, session) = make_session(&mut e, init, auth, 1001);
    // The account is signed in: no second permit beside its session.
    assert_eq!(
        e.renew_permit(init, launch, 1001),
        Err(Status::AccessDenied)
    );
    // Another account, or a process without the Launch authority: never.
    assert!(e.renew_permit(init, launch, 1002).is_ok());
    assert_eq!(
        e.renew_permit(auth, authority, 1001),
        Err(Status::AccessDenied)
    );
    assert_eq!(e.renew_permit(init, launch, 7), Err(Status::AccessDenied));
    // Signed out: a fresh permit admits a new session.
    e.revoke_session(auth, authority, session).unwrap();
    let permit = e.renew_permit(init, launch, 1001).unwrap();
    let permit = copy(&mut e, init, permit, auth, rights::READ);
    let again = e.create_session(auth, authority, permit).unwrap();
    assert_eq!(e.session_info(auth, again).unwrap().1, 1001);
}

#[test]
fn the_service_account_signs_in_once_as_an_administrator() {
    let mut e = Executive::boxed();
    let init = system(&mut e, 1);
    let auth = system(&mut e, 3);
    let launch = e.create_role_authority(init, HandleKind::Launch).unwrap();
    let service = door_abi::platform::SERVICE_ACCOUNT;
    let (_, session) = make_session(&mut e, init, auth, service);
    assert_eq!(e.session_info(auth, session).unwrap().1, service);
    // Its applications are administrators' (Administrators for deny only
    // until the full token is asked for), as LocalSystem's services.
    let p = make_user(&mut e, init, auth, session);
    let authority = e.authority(p).unwrap();
    assert_eq!(authority.uid, service);
    assert_ne!(
        authority.groups & door_abi::platform::group::ADMINISTRATOR_ACCOUNT,
        0
    );
    // No second permit, ever: the service account has no interactive
    // sign-in to renew.
    assert_eq!(
        e.renew_permit(init, launch, service),
        Err(Status::AccessDenied)
    );
    // Only the boot loader's Init gets its permit.
    assert_eq!(e.provision_permit(auth, service), Err(Status::AccessDenied));
    assert_eq!(e.provision_permit(init, 19), Err(Status::AccessDenied));
}
