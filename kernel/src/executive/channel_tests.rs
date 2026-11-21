use super::*;
#[test]
fn handle_values_are_multiples_of_four_above_the_compatibility_handles() {
    let mut e = Executive::boxed();
    let a = e.create_process().unwrap();
    let b = e.create_process().unwrap();
    let mut seen = [0u64; 16];
    for pair in seen.chunks_mut(2) {
        pair.copy_from_slice(&e.channel_pair(a, b).unwrap());
    }
    // Windows programs keep flags in the low two bits (an OVERLAPPED's
    // hEvent | 1), and console/registry handles live below 0x10000.
    assert!(seen.iter().all(|h| h % 4 == 0 && *h >= HANDLE_FIRST));
    assert!(seen.windows(2).all(|w| w[1] == w[0] + HANDLE_STRIDE));
}
#[test]
fn pair_reserves_both_global_incarnations_before_any_publication() {
    for same_owner in [false, true] {
        for remaining in 0..=2 {
            let mut e = Executive::boxed();
            let a = e.create_process().unwrap();
            let b = if same_owner {
                a
            } else {
                e.create_process().unwrap()
            };
            // The last values a stride of four can reach.
            let top = !(HANDLE_STRIDE - 1);
            e.next_handle = top - remaining * HANDLE_STRIDE;
            let original_objects = e.object_count();
            let original_object_counter = e.next_object;
            let result = e.channel_pair(a, b);
            if remaining < 2 {
                assert_eq!(result, Err(Status::ResourceExhausted));
                assert_eq!(e.object_count(), original_objects);
                assert_eq!(e.next_object, original_object_counter);
                assert_eq!(e.next_handle, top - remaining * HANDLE_STRIDE);
                assert_eq!(e.handle_count(a), Ok(0));
                assert_eq!(e.handle_count(b), Ok(0));
                assert!(e.channels.iter().all(Option::is_none));
            } else {
                let [left, right] = result.unwrap();
                assert_eq!(
                    [left, right],
                    [top - 2 * HANDLE_STRIDE, top - HANDLE_STRIDE]
                );
                assert_eq!(e.next_handle, top);
                e.close(a, left).unwrap();
                e.close(b, right).unwrap();
                assert_eq!(e.object_count(), original_objects);
                assert_eq!(e.channel_pair(a, b), Err(Status::ResourceExhausted));
            }
        }
    }
}
use door_abi::{CANCEL_HISTORY_DEPTH, Header, MAX_PACKET, Transfer};

fn packet(request_id: u64, handle_count: u16) -> [u8; 32] {
    let mut bytes = [0; 32];
    Header {
        opcode: 1,
        flags: 0,
        request_id,
        payload_bytes: 0,
        handle_count,
    }
    .encode(&mut bytes)
    .unwrap();
    bytes
}

fn pair() -> (std::boxed::Box<Executive>, ProcessId, ProcessId, u64, u64) {
    let mut executive = Executive::boxed();
    let sender = executive.create_process().unwrap();
    let receiver = executive.create_process().unwrap();
    let [send, receive] = executive.channel_pair(sender, receiver).unwrap();
    (executive, sender, receiver, send, receive)
}

fn moved_section(executive: &mut Executive, owner: ProcessId, key: u64) -> Transfer {
    let handle = executive
        .create_section(
            owner,
            key,
            4096,
            rights::READ | rights::TRANSFER | rights::QUERY,
        )
        .unwrap();
    Transfer {
        handle,
        rights: rights::READ | rights::QUERY,
        mode: 1,
    }
}

#[test]
fn cancellation_and_receive_have_one_commit_winner_in_both_orders() {
    for receive_first in [false, true] {
        let (mut e, a, b, send, recv) = pair();
        let transfer = moved_section(&mut e, a, 1);
        e.send_at(a, send, &packet(1, 1), &[transfer], 1, None)
            .unwrap();
        assert_eq!(e.inspect(a, transfer.handle), Err(Status::BadHandle));
        if receive_first {
            let message = e.receive_at(b, recv, MAX_PACKET, 1, 2).unwrap();
            assert_eq!(e.cancel(a, send, 1, 2), Ok(1));
            assert_eq!(e.cancel(a, send, 1, 3), Ok(1));
            assert!(e.inspect(b, message.transfers[0].handle).is_ok());
            assert_eq!(e.take_retired_section(), None);
            e.close(b, message.transfers[0].handle).unwrap();
        } else {
            assert_eq!(e.cancel(a, send, 1, 2), Ok(0));
            assert_eq!(e.cancel(a, send, 1, 3), Ok(2));
            assert!(matches!(
                e.receive_at(b, recv, MAX_PACKET, 1, 3),
                Err(Status::WouldBlock)
            ));
        }
        assert_eq!(e.inspect(a, transfer.handle), Err(Status::BadHandle));
        assert_eq!(e.take_retired_section(), Some(1));
        assert_eq!(e.take_retired_section(), None);
        assert_eq!(
            e.send(a, send, &packet(1, 0), &[]),
            Err(Status::InvalidArgument)
        );
    }
}

#[test]
fn elapsed_send_is_precommit_and_queue_expiry_releases_the_moved_grant() {
    let (mut e, a, b, send, recv) = pair();
    let transfer = moved_section(&mut e, a, 1);
    assert_eq!(
        e.send_at(a, send, &packet(1, 1), &[transfer], 10, Some(10)),
        Err(Status::Timeout)
    );
    assert!(e.inspect(a, transfer.handle).is_ok());
    assert_eq!(e.cancel(a, send, 1, 10), Err(Status::NotFound));
    e.send_at(a, send, &packet(1, 1), &[transfer], 10, Some(12))
        .unwrap();
    assert!(e.receive_info_at(b, recv, 11).is_ok());
    assert_eq!(e.receive_info_at(b, recv, 12), Err(Status::WouldBlock));
    assert_eq!(e.cancel(a, send, 1, 12), Ok(3));
    assert_eq!(e.cancel(a, send, 1, 13), Ok(3));
    assert_eq!(e.take_retired_section(), Some(1));
    assert_eq!(e.inspect(a, transfer.handle), Err(Status::BadHandle));
    assert_eq!(
        e.send_at(a, send, &packet(1, 0), &[], 13, None),
        Err(Status::InvalidArgument)
    );
}

#[test]
fn cancellation_and_deadline_boundary_choose_one_terminal_outcome() {
    for (cancel_at, outcome, repeat) in [(9, 0, 2), (10, 3, 3)] {
        let (mut e, a, b, send, recv) = pair();
        e.send_at(a, send, &packet(1, 0), &[], 0, Some(10)).unwrap();
        assert_eq!(e.cancel(a, send, 1, cancel_at), Ok(outcome));
        assert_eq!(e.cancel(a, send, 1, 11), Ok(repeat));
        assert_eq!(e.receive_info_at(b, recv, 11), Err(Status::WouldBlock));
    }
    let (mut e, a, b, send, recv) = pair();
    e.send_at(a, send, &packet(1, 0), &[], 0, Some(10)).unwrap();
    e.receive_at(b, recv, MAX_PACKET, 0, 9).unwrap();
    assert_eq!(e.cancel(a, send, 1, 10), Ok(1));
}

#[test]
fn expired_entries_release_capacity_without_reordering_live_bidirectional_traffic() {
    let (mut e, a, b, send, recv) = pair();
    for request in 1..=4 {
        e.send_at(
            a,
            send,
            &packet(request, 0),
            &[],
            0,
            if request % 2 == 0 { Some(5) } else { None },
        )
        .unwrap();
    }
    e.send_at(b, recv, &packet(1, 0), &[], 0, None).unwrap();
    // Send purges expired slots before testing queue capacity.
    e.send_at(a, send, &packet(5, 0), &[], 5, None).unwrap();
    assert_eq!(e.cancel(a, send, 2, 5), Ok(3));
    assert_eq!(e.cancel(a, send, 4, 5), Ok(3));
    assert_eq!(e.cancel(a, send, 3, 5), Ok(0));
    for request in [1, 5] {
        assert_eq!(
            e.receive_at(b, recv, MAX_PACKET, 0, 5)
                .unwrap()
                .metadata
                .request_id,
            request
        );
    }
    assert_eq!(
        e.receive_at(a, send, MAX_PACKET, 0, 5)
            .unwrap()
            .metadata
            .sender_pid,
        b.raw()
    );
    assert_eq!(e.receive_info_at(b, recv, 5), Err(Status::WouldBlock));
}

#[test]
fn cancellation_requires_the_callers_sending_endpoint_authority() {
    let (mut e, a, b, send, recv) = pair();
    e.send(a, send, &packet(1, 0), &[]).unwrap();
    assert_eq!(e.cancel(b, send, 1, 0), Err(Status::BadHandle));
    assert_eq!(e.cancel(b, recv, 1, 0), Err(Status::NotFound));
    assert_eq!(e.cancel(a, send, 0, 0), Err(Status::InvalidArgument));
    assert_eq!(e.cancel(a, send, 99, 0), Err(Status::NotFound));
    let read_only = e.duplicate(a, send, rights::READ).unwrap();
    assert_eq!(e.cancel(a, read_only, 1, 0), Err(Status::AccessDenied));
    e.receive_at(b, recv, MAX_PACKET, 0, 0).unwrap();
    assert_eq!(e.cancel(a, send, 1, 0), Ok(1));
}

#[test]
fn history_eviction_forgets_outcomes_without_permitting_request_reuse() {
    let (mut e, a, _, send, _) = pair();
    let last = CANCEL_HISTORY_DEPTH as u64 + 1;
    for request in 1..=last {
        e.send(a, send, &packet(request, 0), &[]).unwrap();
        assert_eq!(e.cancel(a, send, request, 0), Ok(0));
    }
    assert_eq!(e.cancel(a, send, 1, 0), Err(Status::NotFound));
    assert_eq!(e.cancel(a, send, 2, 0), Ok(2));
    assert_eq!(e.cancel(a, send, last, 0), Ok(2));
    assert_eq!(
        e.send(a, send, &packet(1, 0), &[]),
        Err(Status::InvalidArgument)
    );
    e.send(a, send, &packet(u64::MAX, 0), &[]).unwrap();
    assert_eq!(e.cancel(a, send, u64::MAX, 0), Ok(0));
    assert_eq!(
        e.send(a, send, &packet(18, 0), &[]),
        Err(Status::InvalidArgument)
    );
}

#[test]
fn process_death_and_endpoint_closure_release_pending_grants_once() {
    for receiver_dies_first in [false, true] {
        let (mut e, a, b, send, recv) = pair();
        let transfer = moved_section(&mut e, a, 1);
        e.send_at(a, send, &packet(1, 1), &[transfer], 0, Some(10))
            .unwrap();
        if receiver_dies_first {
            e.exit(b, 22).unwrap();
            assert_eq!(e.cancel(a, send, 1, 1), Ok(2));
            e.exit(a, 11).unwrap();
        } else {
            e.exit(a, 11).unwrap();
            // Sender death does not revoke an already committed message.
            assert!(e.receive_info_at(b, recv, 1).is_ok());
            assert_eq!(e.receive_info_at(b, recv, 10), Err(Status::PeerClosed));
            e.exit(b, 22).unwrap();
        }
        assert_eq!(e.take_retired_section(), Some(1));
        assert_eq!(e.take_retired_section(), None);
        e.reap(a).unwrap();
        e.reap(b).unwrap();
        assert_eq!(e.object_count(), 0);
        let new_a = e.create_process().unwrap();
        let new_b = e.create_process().unwrap();
        let [new_send, _] = e.channel_pair(new_a, new_b).unwrap();
        assert_ne!(new_send, send);
        e.send(new_a, new_send, &packet(1, 0), &[]).unwrap();
        e.exit(new_a, 0).unwrap();
        e.exit(new_b, 0).unwrap();
        e.reap(new_a).unwrap();
        e.reap(new_b).unwrap();
        assert_eq!(e.object_count(), 0);
    }
}
