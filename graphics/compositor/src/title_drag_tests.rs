use super::*;

#[test]
fn title_drag_restores_saved_size_under_the_pointer_with_one_configure() {
    for scale in 1..=3 {
        for snap in [Snap::Maximize, Snap::Left, Snap::Right, Snap::TopLeft] {
            let mut e =
                Engine::new(1280 * scale, 720 * scale, Theme::Light, Limits::default()).unwrap();
            let original = Rect::new(
                180 * scale as i32,
                90 * scale as i32,
                320 * scale,
                220 * scale,
            )
            .unwrap();
            let w = create(&mut e, A, original);
            commit_color(&mut e, A, w, [20, 30, 40, 255]);
            let revision = e.snapshot(w).unwrap().revision;
            e.activate(w).unwrap();
            e.snap_window(A, w, snap, None).unwrap();
            drain(&mut e, A);
            let before = e.window_info(w).unwrap();
            let epoch = before.epoch;
            let pressed = (
                before.outer.x() + (before.outer.width() * 3 / 4) as i32,
                before.outer.y() + 10 * scale as i32,
            );
            let pointer = (640 * scale as i32, 160 * scale as i32);
            let restored = e.restore_title_drag(A, w, pressed, pointer).unwrap();
            assert_eq!(
                restored,
                Rect::new(
                    400 * scale as i32,
                    150 * scale as i32,
                    320 * scale,
                    220 * scale
                )
                .unwrap()
            );
            let after = e.window_info(w).unwrap();
            assert_eq!(after.state, WindowState::Normal);
            assert!(!after.snapped);
            assert_eq!(after.epoch, epoch + 1);
            assert_eq!(e.snapshot(w).unwrap().revision, revision);
            let events = drain(&mut e, A);
            assert_eq!(
                events
                    .iter()
                    .filter(
                        |event| matches!(event, Event::Configure { window, .. } if *window == w)
                    )
                    .count(),
                1
            );
            e.request_state(A, w, WindowState::Maximized).unwrap();
            e.restore_window(A, w).unwrap();
            assert_eq!(e.window_info(w).unwrap().outer, restored);
        }
    }
}

#[test]
fn title_restore_guards_authority_press_state_and_backpressure_without_losing_geometry() {
    let mut e = Engine::new(640, 480, Theme::Dark, Limits::default()).unwrap();
    let w = create(&mut e, A, rect(60, 70));
    assert_eq!(
        e.restore_title_drag(A, w, (80, 80), (100, 100)),
        Err(Error::InvalidState)
    );
    e.snap_window(A, w, Snap::Maximize, None).unwrap();
    e.activate(w).unwrap();
    drain(&mut e, A);
    let outer = e.window_info(w).unwrap().outer;
    let epoch = e.window_info(w).unwrap().epoch;
    assert_eq!(
        e.restore_title_drag(B, w, (100, 10), (200, 100)),
        Err(Error::WrongOwner)
    );
    for press in [(-1, 10), (100, 40), (outer.right(), 10)] {
        assert_eq!(
            e.restore_title_drag(A, w, press, (200, 100)),
            Err(Error::InvalidGeometry)
        );
    }
    while e
        .route_input(
            Input::Key {
                code: 30,
                down: true,
            },
            0,
        )
        .is_ok()
    {}
    assert_eq!(
        e.restore_title_drag(A, w, (100, 10), (200, 100)),
        Err(Error::QueueFull)
    );
    assert_eq!(e.window_info(w).unwrap().outer, outer);
    assert_eq!(e.window_info(w).unwrap().epoch, epoch);
    assert_eq!(e.window_info(w).unwrap().state, WindowState::Maximized);
    drain(&mut e, A);
    let restored = e
        .restore_title_drag(A, w, (100, 10), (i32::MIN, i32::MAX))
        .unwrap();
    assert_eq!(restored.x(), 80 - restored.width() as i32);
    assert_eq!(restored.y(), 480 - TITLE_HEIGHT as i32);
    e.close(A, w).unwrap();
    assert!(e.restore_title_drag(A, w, (100, 10), (200, 100)).is_err());
    assert_eq!(e.window_info(w).unwrap().state, WindowState::Closing);
}

#[test]
fn same_position_move_preserves_completed_scene_and_pending_reuse() {
    let mut e = engine();
    let w = create(&mut e, A, rect(40, 50));
    e.activate(w).unwrap();
    let serial = e.render().unwrap().serial;
    e.complete_transfer(serial, true, 0).unwrap();
    e.move_window(A, w, 40, 50).unwrap();
    assert!(!e.has_damage());
    e.move_window(A, w, 45, 55).unwrap();
    e.move_window(A, w, 45, 55).unwrap();
    let frame = e
        .render_damage_reusing_moves(true, |_| {}, |_, _| {}, |_| {})
        .unwrap();
    assert!(frame.reused_pixels > 0);
}
