use super::*;
use super::{A, B, create, drain};

fn scene(scale: u32, theme: Theme) -> (Engine, WindowId) {
    let mut e = Engine::new(640 * scale, 480 * scale, theme, Limits::default()).unwrap();
    e.set_scale(scale).unwrap();
    create(
        &mut e,
        A,
        Rect::new(20, 20, 350 * scale, 290 * scale).unwrap(),
    );
    let w = create(
        &mut e,
        B,
        Rect::new(
            100 * scale as i32,
            90 * scale as i32,
            220 * scale,
            180 * scale,
        )
        .unwrap(),
    );
    let info = e.window_info(w).unwrap();
    let epoch = info.epoch;
    let (width, height) = (info.content.width(), info.content.height());
    e.acknowledge_configure(B, w, epoch).unwrap();
    let desc = SurfaceDescriptor::new(
        width,
        height,
        width as usize * 4,
        width as usize * height as usize * 4,
    )
    .unwrap();
    let b = e.register_buffer(B, desc).unwrap();
    e.attach(B, w, b, epoch).unwrap();
    let pixels = (0..height)
        .flat_map(|y| {
            (0..width)
                .flat_map(move |x| [(x % 91) as u8, (y % 87) as u8, ((x + y) % 79) as u8, 128])
        })
        .collect();
    e.commit(B, w, pixels).unwrap();
    e.activate(w).unwrap();
    drain(&mut e, A);
    drain(&mut e, B);
    paint(&mut e, false);
    (e, w)
}

fn background(canvas: &mut Canvas<'_>) {
    let _ = canvas.fill_rect(Rect::new(75, 35, 29, 400).unwrap(), [45, 60, 90, 255]);
}
fn chrome(info: WindowInfo<'_>, canvas: &mut Canvas<'_>) {
    let _ = canvas.fill_rect(
        Rect::new(info.outer.x() + 40, info.outer.y() + 4, 27, 12).unwrap(),
        [(info.outer.x() % 128) as u8, 80, 90, 255],
    );
}
fn paint(e: &mut Engine, reuse: bool) -> (alloc::vec::Vec<u8>, u64) {
    let frame = e
        .render_damage_reusing_moves(reuse, background, chrome, |_| {})
        .unwrap();
    let result = (frame.pixels.to_vec(), frame.reused_pixels);
    let serial = frame.serial;
    e.complete_transfer(serial, true, 0).unwrap();
    drain(e, A);
    drain(e, B);
    result
}

#[test]
fn translated_opaque_client_matches_full_redraw_in_all_directions_and_densities() {
    for scale in 1..=3 {
        for theme in [Theme::Light, Theme::Dark] {
            let (mut reused, a) = scene(scale, theme);
            let (mut ordinary, b) = scene(scale, theme);
            for (dx, dy) in [
                (7, 9),
                (-8, 6),
                (-9, -11),
                (15, -8),
                (4, 0),
                (-7, 0),
                (0, 5),
                (0, -9),
                (200, 100),
                (-195, -95),
            ] {
                let old = reused.window_info(a).unwrap().outer;
                let (x, y) = (old.x() + dx, old.y() + dy);
                reused.move_window(B, a, x, y).unwrap();
                ordinary.move_window(B, b, x, y).unwrap();
                // Coalesce more input while one frame is pending.
                reused.move_window(B, a, x + 1, y + 2).unwrap();
                ordinary.move_window(B, b, x + 1, y + 2).unwrap();
                let expected = paint(&mut ordinary, false);
                let actual = paint(&mut reused, true);
                assert!(actual.1 > 0, "scale={scale}, delta={dx},{dy}");
                assert!(actual.0 == expected.0, "scale={scale}, delta={dx},{dy}");
            }
        }
    }
}

#[test]
fn reuse_is_canceled_by_scene_changes_occlusion_and_failed_transfer() {
    for change in 0..8 {
        let (mut e, w) = scene(1, Theme::Light);
        e.move_window(B, w, 110, 100).unwrap();
        match change {
            0 => e.invalidate_region(Rect::new(0, 0, 1, 1).unwrap()),
            1 => e.set_theme(Theme::Dark),
            2 => {
                e.set_title(B, w, "changed").unwrap();
            }
            3 => {
                e.configure(B, w, Rect::new(110, 100, 230, 190).unwrap())
                    .unwrap();
            }
            4 => {
                create(&mut e, A, Rect::new(80, 80, 200, 180).unwrap());
            }
            5 => {
                e.move_window(B, w, -5, 100).unwrap();
            }
            6 => {
                // Input during an in-flight transfer queues a new plan. A failed
                // transfer must force full reconstruction before using it.
                let serial = e.render().unwrap().serial;
                e.move_window(B, w, 115, 105).unwrap();
                e.complete_transfer(serial, false, 0).unwrap();
            }
            7 => {
                e.request_frame(B, w).unwrap();
            }
            _ => unreachable!(),
        }
        let actual = paint(&mut e, true);
        assert_eq!(actual.1, 0, "change={change}");
        e.invalidate();
        assert!(actual.0 == paint(&mut e, false).0, "change={change}");
    }
}

#[test]
fn moves_queued_during_transfer_do_not_mutate_the_inflight_pixels() {
    let (mut e, w) = scene(1, Theme::Light);
    let frame = e
        .render_damage_reusing_moves(false, background, chrome, |_| {})
        .unwrap();
    let serial = frame.serial;
    let before = frame.pixels.to_vec();
    e.move_window(B, w, 130, 120).unwrap();
    e.move_window(B, w, 150, 135).unwrap();
    assert_eq!(e.transfer_view(serial).unwrap().pixels, before);
    e.complete_transfer(serial, true, 0).unwrap();
    let actual = paint(&mut e, true);
    assert!(actual.1 > 0);
    e.invalidate();
    assert!(actual.0 == paint(&mut e, false).0);
}

#[test]
fn work_area_edges_keep_overlay_once_on_the_completed_scene_and_original_clip() {
    for area in [
        Rect::new(0, 0, 640, 432).unwrap(),
        Rect::new(0, 48, 640, 432).unwrap(),
        Rect::new(48, 0, 592, 480).unwrap(),
        Rect::new(0, 0, 592, 480).unwrap(),
    ] {
        let (mut e, w) = scene(1, Theme::Dark);
        e.reflow_work_area(area).unwrap();
        paint(&mut e, false);
        for (x, y) in [
            (area.x(), area.y()),
            (area.right() - 220, area.bottom() - 180),
        ] {
            e.move_window(B, w, x, y).unwrap();
            let mut calls = 0;
            let mut clip = None;
            let frame = e
                .render_damage_reusing_moves(true, background, chrome, |canvas| {
                    calls += 1;
                    clip = canvas.clip_rect();
                })
                .unwrap();
            assert!(frame.reused_pixels > 0);
            assert_eq!(calls, 1);
            assert_eq!(clip, frame.damage);
            let actual = frame.pixels.to_vec();
            let serial = frame.serial;
            e.complete_transfer(serial, true, 0).unwrap();
            drain(&mut e, B);
            e.invalidate();
            assert!(actual == paint(&mut e, false).0);
        }
    }
}
