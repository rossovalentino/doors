use super::*;
#[path = "move_tests.rs"]
mod move_tests;
#[path = "title_drag_tests.rs"]
mod title_drag_tests;
#[test]
fn shell_edge_change_keeps_saved_restore_and_unsnap_titles_reachable() {
    for minimized in [false, true] {
        let mut e = Engine::new(640, 480, Theme::Light, Limits::default()).unwrap();
        let a = create(&mut e, A, rect(0, 0));
        let b = create(&mut e, B, rect(40, 0));
        e.snap_window(A, a, Snap::Maximize, None).unwrap();
        e.snap_window(B, b, Snap::Left, None).unwrap();
        if minimized {
            e.request_state(A, a, WindowState::Minimized).unwrap();
        }
        e.reflow_work_area(Rect::new(0, 48, 640, 432).unwrap())
            .unwrap();
        if minimized {
            e.restore_window(A, a).unwrap();
        }
        e.restore_window(A, a).unwrap();
        e.unsnap_window(B, b).unwrap();
        assert_eq!(e.window_info(a).unwrap().outer, rect(0, 48));
        assert_eq!(e.window_info(b).unwrap().outer, rect(40, 48));
    }
}
#[test]
fn changing_shell_edge_reflows_maximized_minimized_and_snapped_windows_atomically() {
    let mut e = Engine::new(640, 480, Theme::Light, Limits::default()).unwrap();
    let original = rect(40, 50);
    let a = create(&mut e, A, original);
    let b = create(&mut e, B, rect(200, 80));
    e.snap_window(A, a, Snap::Maximize, None).unwrap();
    e.request_state(A, a, WindowState::Minimized).unwrap();
    e.snap_window(B, b, Snap::Right, None).unwrap();
    drain(&mut e, A);
    drain(&mut e, B);
    let old = e.work_area();
    let old_a = e.window_info(a).unwrap().outer;
    let old_b = e.window_info(b).unwrap().outer;
    let area = Rect::new(48, 0, old.width() - 48, old.height()).unwrap();
    e.activate(b).unwrap();
    drain(&mut e, A);
    drain(&mut e, B);
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
    assert_eq!(e.reflow_work_area(area), Err(Error::QueueFull));
    assert_eq!(e.work_area(), old);
    assert_eq!(e.window_info(a).unwrap().outer, old_a);
    assert_eq!(e.window_info(b).unwrap().outer, old_b);
    drain(&mut e, B);
    e.reflow_work_area(area).unwrap();
    assert_eq!(e.window_info(a).unwrap().state, WindowState::Minimized);
    assert_eq!(e.window_info(a).unwrap().outer, area);
    assert!(e.window_info(b).unwrap().snapped);
    assert_eq!(e.window_info(b).unwrap().outer.right(), area.right());
    e.restore_window(A, a).unwrap();
    assert_eq!(e.window_info(a).unwrap().state, WindowState::Maximized);
    e.restore_window(A, a).unwrap();
    assert_eq!(e.window_info(a).unwrap().outer, original);
}
#[test]
fn window_outline_follows_all_rounded_corners_at_native_density() {
    for scale in [1, 2] {
        let mut e = Engine::new(640, 480, Theme::Light, Limits::default()).unwrap();
        e.set_scale(scale).unwrap();
        let r = Rect::new(50, 40, 360, 260).unwrap();
        let w = create(&mut e, A, r);
        e.activate(w).unwrap();
        let frame = e.render().unwrap();
        for (x, y) in [
            (r.x() + 3 * scale as i32, r.y() + 2 * scale as i32),
            (r.right() - 1 - 3 * scale as i32, r.y() + 2 * scale as i32),
            (r.x() + 3 * scale as i32, r.bottom() - 1 - 2 * scale as i32),
            (
                r.right() - 1 - 3 * scale as i32,
                r.bottom() - 1 - 2 * scale as i32,
            ),
        ] {
            let at = (y as usize * 640 + x as usize) * 4;
            assert!(
                (180..248).contains(&frame.pixels[at]),
                "corner contour missing: scale={scale} x={x} y={y} color={:?}",
                &frame.pixels[at..at + 4]
            );
        }
    }
}
#[test]
fn identical_captures_and_repeated_activation_preserve_idle_scene() {
    let mut e = engine();
    let w = create(&mut e, A, rect(30, 20));
    let b = commit_color(&mut e, A, w, [20, 30, 40, 255]);
    e.activate(w).unwrap();
    let serial = e.render().unwrap().serial;
    e.complete_transfer(serial, true, 0).unwrap();
    drain(&mut e, A);
    e.activate(w).unwrap();
    assert!(!e.has_damage());
    assert!(drain(&mut e, A).is_empty());
    let epoch = e.window_info(w).unwrap().epoch;
    let mut pixels = e.snapshot(w).unwrap().bytes.to_vec();
    e.attach(A, w, b, epoch).unwrap();
    e.commit(A, w, pixels.clone()).unwrap();
    assert!(!e.has_damage());
    assert!(drain(&mut e, A).contains(&Event::BufferReleased { buffer: b }));
    let content = e.window_info(w).unwrap().content;
    let outer = e.window_info(w).unwrap().outer;
    let at = (25 * content.width() as usize + 45) * 4;
    pixels[at..at + 4].copy_from_slice(&[10, 15, 20, 80]);
    e.attach(A, w, b, epoch).unwrap();
    e.commit(A, w, pixels).unwrap();
    let frame = e
        .render_damage_with_layers(|_| {}, |_, _| {}, |_| {})
        .unwrap();
    let damage = frame.damage.unwrap();
    assert!(damage.contains(content.x() + 45, content.y() + 25));
    assert!(damage.width() < outer.width() / 2);
    let got = frame.pixels.to_vec();
    let serial = frame.serial;
    e.complete_transfer(serial, true, 1).unwrap();
    let full = e.render().unwrap();
    assert_eq!(got, full.pixels);
}
#[test]
fn regional_composition_matches_full_scene_across_stacking_and_lifecycle() {
    fn assert_scene(e: &mut Engine) {
        let frame = e
            .render_damage_with_layers(
                |c| {
                    c.fill_rect(Rect::new(0, 0, 640, 480).unwrap(), [47, 39, 22, 255])
                        .unwrap();
                },
                |_, _| {},
                |_| {},
            )
            .unwrap();
        let got = frame.pixels.to_vec();
        let serial = frame.serial;
        e.complete_transfer(serial, true, 0).unwrap();
        let frame = e
            .render_with_background(
                |c| {
                    c.fill_rect(Rect::new(0, 0, 640, 480).unwrap(), [47, 39, 22, 255])
                        .unwrap();
                },
                |_, _| {},
            )
            .unwrap();
        assert_eq!(got, frame.pixels);
        let serial = frame.serial;
        e.complete_transfer(serial, true, 0).unwrap();
        drain(e, A);
        drain(e, B);
    }
    for scale in [1, 2] {
        let mut e = Engine::new(640, 480, Theme::Light, Limits::default()).unwrap();
        e.set_scale(scale).unwrap();
        let a = create(&mut e, A, Rect::new(15, 18, 360, 210).unwrap());
        let b = create(&mut e, B, Rect::new(103, 78, 360, 240).unwrap());
        let buffer = commit_color(&mut e, A, a, [10, 35, 60, 90]);
        commit_color(&mut e, B, b, [40, 70, 110, 255]);
        e.activate(b).unwrap();
        assert_scene(&mut e);
        for (x, y) in [(110, 89), (-24, 30), (270, 220), (97, 79)] {
            e.move_window(B, b, x, y).unwrap();
            assert_scene(&mut e);
        }
        e.activate(a).unwrap();
        assert_scene(&mut e);
        let epoch = e.window_info(a).unwrap().epoch;
        let mut pixels = e.snapshot(a).unwrap().bytes.to_vec();
        pixels[12..16].copy_from_slice(&[0, 0, 0, 0]);
        e.attach(A, a, buffer, epoch).unwrap();
        e.commit(A, a, pixels).unwrap();
        assert_scene(&mut e);
        e.request_state(B, b, WindowState::Minimized).unwrap();
        assert_scene(&mut e);
        e.restore_window(B, b).unwrap();
        assert_scene(&mut e);
        e.destroy_window(A, a).unwrap();
        assert_scene(&mut e);
        e.destroy_window(B, b).unwrap();
        assert_scene(&mut e);
    }
}
#[test]
fn snapshot_difference_ignores_padding_but_detects_alpha_and_disjoint_pixels() {
    let descriptor = SurfaceDescriptor::new(4, 3, 20, 60).unwrap();
    let a = vec![0; 60];
    let mut b = a.clone();
    b[19] = 255;
    b[59] = 255;
    assert_eq!(descriptor.changed_bounds(&a, &b), None);
    b[3] = 1;
    b[2 * 20 + 3 * 4 + 3] = 1;
    assert_eq!(
        descriptor.changed_bounds(&a, &b),
        Some(Rect::new(0, 0, 4, 3).unwrap())
    );
}
#[test]
fn constant_alpha_rows_match_pixel_composition_for_every_alpha_and_destination_byte() {
    let (width, height, stride) = (256, 4, 1036);
    let clip = Rect::new(0, 1, 256, 2).unwrap();
    for alpha in 0..=255u8 {
        for color in [
            [alpha, alpha, alpha, alpha],
            [0, 0, 0, alpha],
            [alpha / 3, alpha / 2, alpha, alpha],
        ] {
            let mut pixels = alloc::vec![0x5a; stride * height as usize + 16];
            // Distinct rows/channels, translucent and even invalid destinations:
            // Canvas accepts private bytes; preserve the old blend's saturation.
            for y in 0..height as usize {
                for x in 0..width as usize {
                    let at = y * stride + x * 4;
                    pixels[at..at + 4].copy_from_slice(&[
                        x as u8,
                        (255 - x) as u8,
                        (x + y * 37) as u8,
                        (x + y * 83) as u8,
                    ]);
                }
            }
            let mut expected = pixels.clone();
            let mut reference = Canvas::new(&mut expected, width, height, stride).unwrap();
            reference.set_clip(clip);
            for y in clip.y()..clip.bottom() {
                for x in clip.x()..clip.right() {
                    reference.pixel(x, y, color).unwrap();
                }
            }
            let mut canvas = Canvas::new(&mut pixels, width, height, stride).unwrap();
            canvas.set_clip(clip);
            canvas
                .fill_rect(Rect::new(-12, -5, 300, 20).unwrap(), color)
                .unwrap();
            assert_eq!(pixels, expected, "alpha={alpha} color={color:?}");
        }
    }
}
#[test]
fn translucent_rows_preserve_scaled_clip_padding_and_outside_bytes() {
    for scale in 1..=3 {
        for draw in [
            Rect::new(-4, -3, 20, 12).unwrap(),
            Rect::new(2, 1, 2, 3).unwrap(),
            Rect::new(99, 99, 1, 1).unwrap(),
            Rect::new(-99, -99, 1, 1).unwrap(),
        ] {
            let (width, height, stride) = (41, 27, 180);
            let clip = Rect::new(3, 2, 33, 21).unwrap();
            let mut pixels = alloc::vec![79; stride * height as usize + 20];
            for (index, pixel) in pixels.chunks_exact_mut(4).enumerate() {
                let a = (index * 17) as u8;
                pixel.copy_from_slice(&[a / 2, a / 3, a, a]);
            }
            let mut expected = pixels.clone();
            let physical = Rect::new(
                draw.x() * scale,
                draw.y() * scale,
                draw.width() * scale as u32,
                draw.height() * scale as u32,
            )
            .unwrap();
            let color = [30, 14, 4, 130]; // Actual sign-in tint.
            let mut reference = Canvas::new(&mut expected, width, height, stride).unwrap();
            reference.set_clip(clip);
            if let Some(area) = physical.intersection(clip) {
                for y in area.y()..area.bottom() {
                    for x in area.x()..area.right() {
                        reference.pixel(x, y, color).unwrap();
                    }
                }
            }
            let mut canvas = Canvas::new(&mut pixels, width, height, stride).unwrap();
            canvas
                .set_scale(Scale::from_factor(scale as u32).unwrap())
                .unwrap();
            canvas.set_clip(clip);
            canvas.fill_rect(draw, color).unwrap();
            assert_eq!(pixels, expected, "scale={scale} draw={draw:?}");
        }
    }
}
#[test]
fn opaque_scanline_fill_matches_clipped_reference_with_stride_and_scale() {
    for scale in [1, 2] {
        for color in [[23, 91, 187, 255], [20, 30, 40, 100], [0; 4]] {
            let mut pixels = alloc::vec![77; 108 * 18];
            let mut expected = pixels.clone();
            let draw = Rect::new(-2, 2, 13, 9).unwrap();
            let clip = Rect::new(3, 1, 15, 15).unwrap();
            let physical =
                Rect::new(-2 * scale, 2 * scale, 13 * scale as u32, 9 * scale as u32).unwrap();
            if let Some(area) = physical.intersection(clip) {
                for y in area.y()..area.bottom() {
                    for x in area.x()..area.right() {
                        let at = y as usize * 108 + x as usize * 4;
                        let dest = expected[at..at + 4].try_into().unwrap();
                        expected[at..at + 4].copy_from_slice(&blend_bgra(dest, color).unwrap());
                    }
                }
            }
            let mut canvas = Canvas::new(&mut pixels, 25, 18, 108).unwrap();
            canvas
                .set_scale(Scale::from_factor(scale as u32).unwrap())
                .unwrap();
            canvas.set_clip(clip);
            canvas.fill_rect(draw, color).unwrap();
            assert_eq!(pixels, expected);
        }
    }
}
#[test]
fn rounded_bitmaps_clip_source_and_keep_antialiased_corners() {
    let mut pixels = alloc::vec![255;16*16*4];
    let source = alloc::vec![0;8*8*3];
    {
        let mut canvas = Canvas::new(&mut pixels, 16, 16, 64).unwrap();
        canvas
            .blit_rounded_bgr(Rect::new(2, 2, 8, 8).unwrap(), 4, &source)
            .unwrap();
    }
    assert_eq!(&pixels[(2 * 16 + 2) * 4..(2 * 16 + 2) * 4 + 4], &[255; 4]);
    assert_eq!(
        &pixels[(6 * 16 + 6) * 4..(6 * 16 + 6) * 4 + 4],
        &[0, 0, 0, 255]
    );
    assert!(pixels.chunks_exact(4).any(|p| p[0] > 0 && p[0] < 255));
    let before = pixels.clone();
    let mut canvas = Canvas::new(&mut pixels, 16, 16, 64).unwrap();
    assert!(
        canvas
            .blit_rounded_bgr(Rect::new(0, 0, 8, 8).unwrap(), 4, &source[..191])
            .is_err()
    );
    assert_eq!(pixels, before);
    pixels.fill(255);
    let mut canvas = Canvas::new(&mut pixels, 16, 16, 64).unwrap();
    canvas.set_clip(Rect::new(4, 4, 2, 2).unwrap());
    canvas
        .blit_rounded_bgr(Rect::new(0, 0, 8, 8).unwrap(), 2, &source)
        .unwrap();
    for y in 0..16 {
        for x in 0..16 {
            if !(4..6).contains(&x) || !(4..6).contains(&y) {
                assert_eq!(pixels[(y * 16 + x) * 4], 255);
            }
        }
    }
}
#[test]
fn acknowledged_capture_release_cannot_cancel_the_next_queued_gesture() {
    let mut e = engine();
    let w = create(&mut e, A, rect(10, 10));
    e.activate(w).unwrap();
    drain(&mut e, A);
    e.set_capture(A, w).unwrap();
    e.release_capture(A, w).unwrap();
    e.route_input(
        Input::PointerButton {
            button: 0,
            down: true,
            x: 40,
            y: 70,
        },
        2,
    )
    .unwrap();
    e.set_capture(A, w).unwrap();
    let events = drain(&mut e, A);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::CaptureLost { .. }))
    );
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Input {
            input: Input::PointerButton { down: true, .. },
            ..
        }
    )));
    assert_eq!(e.captured_window(), Some(w));
}
#[test]
fn client_caption_submits_full_height_and_keeps_system_buttons_and_owner_checks() {
    let mut e = Engine::new(1024, 768, Theme::Light, Limits::default()).unwrap();
    let r = Rect::new(10, 20, 960, 640).unwrap();
    let layout = CaptionLayout {
        tabbed: false,
        height: 32,
        max_width: 516,
        side_margin: 300,
    };
    assert!(
        e.create_client_caption(
            A,
            r,
            CaptionLayout {
                tabbed: false,
                height: 32,
                max_width: 516,
                side_margin: 137
            }
        )
        .is_err()
    );
    let w = e.create_client_caption(A, r, layout).unwrap();
    let info = e.window_info(w).unwrap();
    assert!(info.client_caption);
    assert_eq!(info.content, r);
    let epoch = info.epoch;
    assert!(drain(&mut e, A).iter().any(|v| matches!(
        v,
        Event::Configure {
            content_height: 640,
            ..
        }
    )));
    let buffer = commit_color(&mut e, A, w, [80, 90, 100, 255]);
    assert_eq!(e.acknowledge_configure(B, w, epoch), Err(Error::WrongOwner));
    assert_eq!(e.hit_test(490, 38), Some((w, HitRegion::Client)));
    assert_eq!(e.hit_test(30, 38), Some((w, HitRegion::Client)));
    assert_eq!(e.hit_test(100, 38), Some((w, HitRegion::TitleBar)));
    assert_eq!(e.hit_test(947, 38), Some((w, HitRegion::Close)));
    e.route_input(
        Input::PointerButton {
            button: 0,
            down: true,
            x: 490,
            y: 38,
        },
        42,
    )
    .unwrap();
    assert!(drain(&mut e, A).iter().any(|v| matches!(
        v,
        Event::Input {
            input: Input::PointerButton { x: 480, y: 18, .. },
            ..
        }
    )));
    e.unregister_buffer(A, buffer).unwrap();
    e.request_state(A, w, WindowState::Maximized).unwrap();
    commit_color(&mut e, A, w, [80, 90, 100, 255]);
    assert_eq!(e.window_info(w).unwrap().content.height(), 768);
    e.render().unwrap();
}
#[test]
fn clipped_rounded_controls_never_paint_outside_their_scroll_viewport() {
    let mut pixels = vec![255u8; 64 * 64 * 4];
    let mut canvas = Canvas::new(&mut pixels, 64, 64, 64 * 4).unwrap();
    let clip = Rect::new(20, 30, 24, 20).unwrap();
    canvas.set_clip(clip);
    canvas
        .fill_rounded_rect(Rect::new(-10, -10, 100, 100).unwrap(), 8, [0, 0, 0, 255])
        .unwrap();
    for y in 0..64 {
        for x in 0..64 {
            let at = (y * 64 + x) * 4;
            assert_eq!(
                pixels[at],
                if clip.contains(x as i32, y as i32) {
                    0
                } else {
                    255
                }
            );
        }
    }
}
#[test]
fn opaque_row_copy_matches_blending_with_clipped_sources_and_rounded_edges() {
    let mut e = engine();
    let w = create(&mut e, A, rect(-20, 12));
    let b = commit_color(&mut e, A, w, [30, 60, 90, 255]);
    let frame = e.render().unwrap();
    let expected = frame.pixels.to_vec();
    let serial = frame.serial;
    e.complete_transfer(serial, true, 1).unwrap();
    let info = e.window_info(w).unwrap();
    let mut pixels: Vec<_> = (0..info.content.width() * info.content.height())
        .flat_map(|_| [30, 60, 90, 255])
        .collect();
    // The first source pixel is offscreen. Making only it transparent forces
    // the general source-over path without changing any visible pixel.
    pixels[..4].fill(0);
    e.attach(A, w, b, info.epoch).unwrap();
    e.commit(A, w, pixels).unwrap();
    assert_eq!(e.render().unwrap().pixels, expected);
}
#[test]
fn opaque_rows_preserve_uncovered_resized_content_against_general_source_over() {
    for theme in [Theme::Light, Theme::Dark] {
        for scale in 1..=3 {
            let compose = |force_general: bool| {
                let mut e =
                    Engine::new(320 * scale, 240 * scale, theme, Limits::default()).unwrap();
                e.set_scale(scale).unwrap();
                let original = Rect::new(
                    -20 * scale as i32,
                    12 * scale as i32,
                    180 * scale,
                    140 * scale,
                )
                .unwrap();
                let w = create(&mut e, A, original);
                let b = commit_color(&mut e, A, w, [30, 60, 90, 255]);
                if force_general {
                    let info = e.window_info(w).unwrap();
                    let mut pixels = e.snapshot(w).unwrap().bytes.to_vec();
                    // Entirely offscreen in both geometries. It selects the
                    // scalar source-over path without changing visible ink.
                    pixels[..4].fill(0);
                    e.attach(A, w, b, info.epoch).unwrap();
                    e.commit(A, w, pixels).unwrap();
                }
                let serial = e.render().unwrap().serial;
                e.complete_transfer(serial, true, 1).unwrap();
                e.configure(
                    A,
                    w,
                    Rect::new(original.x(), original.y(), 280 * scale, 200 * scale).unwrap(),
                )
                .unwrap();
                // A real configure retains the old safe snapshot until the
                // client publishes new content. Newly exposed rows/tails fill.
                e.render_damage_with_layers(|_| {}, |_, _| {}, |_| {})
                    .unwrap()
                    .pixels
                    .to_vec()
            };
            assert_eq!(
                compose(false),
                compose(true),
                "scale={scale} theme={theme:?}"
            );
        }
    }
}

#[test]
fn maximized_opaque_chrome_culls_only_fully_hidden_window_and_shadow() {
    let mut e = engine();
    let bottom = create(&mut e, A, rect(40, 50));
    commit_color(&mut e, A, bottom, [40, 60, 80, 255]);
    let top = create(&mut e, B, rect(70, 70));
    e.request_state(B, top, WindowState::Maximized).unwrap();
    commit_color(&mut e, B, top, [25, 50, 75, 128]);
    let mut painted = Vec::new();
    let frame = e
        .render_with_chrome(|info, _| painted.push(info.id))
        .unwrap();
    let expected = frame.pixels.to_vec();
    let serial = frame.serial;
    assert_eq!(painted, vec![top]);
    e.complete_transfer(serial, true, 1).unwrap();
    e.destroy_window(A, bottom).unwrap();
    assert_eq!(e.render().unwrap().pixels, expected);
}
#[test]
fn trusted_overlay_repaint_preserves_scene_and_does_not_notify_clients() {
    let mut e = engine();
    let w = create(&mut e, A, rect(0, 0));
    let frame = e.render().unwrap();
    let before = frame.pixels[..320 * 4].to_vec();
    let serial = frame.serial;
    e.complete_transfer(serial, true, 1).unwrap();
    while e.poll_event(A).is_some() {}
    let r = Rect::new(0, 200, 320, 40).unwrap();
    let frame = e
        .repaint_overlay(r, |c| c.fill_rect(r, [1, 2, 3, 255]).unwrap())
        .unwrap();
    assert_eq!(&frame.pixels[..320 * 4], before);
    assert_eq!(frame.damage, Some(r));
    let serial = frame.serial;
    e.complete_transfer(serial, true, 2).unwrap();
    assert!(e.poll_event(A).is_none());
    assert!(e.window_info(w).is_ok());
    assert!(!e.has_damage());
}

#[test]
fn closing_front_window_focuses_the_next_visible_application() {
    let mut e = engine();
    let a = create(&mut e, A, rect(0, 0));
    let b = create(&mut e, B, rect(20, 20));
    e.activate(b).unwrap();
    drain(&mut e, A);
    drain(&mut e, B);
    e.close(B, b).unwrap();
    assert_eq!(e.focused_window(), Some(a));
    assert!(drain(&mut e, A).contains(&Event::Focus {
        window: a,
        focused: true
    }));
}
#[test]
fn frameless_windows_have_no_trusted_buttons_border_or_title_bar() {
    let mut e = Engine::new(1024, 768, Theme::Light, Limits::default()).unwrap();
    let r = Rect::new(100, 100, 320, 200).unwrap();
    let layout = CaptionLayout {
        tabbed: false,
        height: 32,
        max_width: 1,
        side_margin: 138,
    };
    let w = e.create_client_caption(A, r, layout).unwrap();
    let close = (r.right() - 20, r.y() + 16);
    assert_eq!(e.hit_test(close.0, close.1), Some((w, HitRegion::Close)));
    assert_eq!(e.set_frameless(B, w, true), Err(Error::WrongOwner));
    e.set_frameless(A, w, true).unwrap();
    for (x, y) in [close, (r.x() + 1, r.y() + 100), (r.x() + 150, r.y() + 10)] {
        assert_eq!(e.hit_test(x, y), Some((w, HitRegion::Client)));
    }
    e.set_frameless(A, w, false).unwrap();
    assert_eq!(e.hit_test(close.0, close.1), Some((w, HitRegion::Close)));
}
#[test]
fn full_screen_covers_the_output_and_restores_the_prior_placement() {
    let mut e = Engine::new(1024, 768, Theme::Light, Limits::default()).unwrap();
    e.set_work_area(Rect::new(0, 0, 1024, 720).unwrap())
        .unwrap();
    let r = Rect::new(100, 100, 320, 200).unwrap();
    let w = create(&mut e, A, r);
    e.activate(w).unwrap();
    assert_eq!(e.set_fullscreen(B, w, true), Err(Error::WrongOwner));
    e.set_fullscreen(A, w, true).unwrap();
    let info = e.window_info(w).unwrap();
    assert_eq!(info.outer, e.output_rect());
    assert_eq!(info.state, WindowState::Maximized);
    assert!(e.fullscreen_focused());
    assert_eq!(
        e.hit_test(1010, 10),
        Some((w, HitRegion::Client)),
        "no caption buttons"
    );
    // A work-area change (taskbar edge) leaves it covering the output.
    e.reflow_work_area(Rect::new(0, 48, 1024, 720).unwrap())
        .unwrap();
    assert_eq!(e.window_info(w).unwrap().outer, e.output_rect());
    // Minimize and restore keeps full screen.
    e.request_state(A, w, WindowState::Minimized).unwrap();
    assert!(!e.fullscreen_focused());
    e.request_state(A, w, WindowState::Maximized).unwrap();
    assert_eq!(e.window_info(w).unwrap().outer, e.output_rect());
    e.set_fullscreen(A, w, false).unwrap();
    let info = e.window_info(w).unwrap();
    assert_eq!((info.outer, info.state), (r, WindowState::Normal));
    assert!(!e.fullscreen_focused());
    assert_eq!(
        e.hit_test(r.right() - 20, r.y() + 16),
        Some((w, HitRegion::Close))
    );
    // From maximized it returns to maximized over the work area.
    e.request_state(A, w, WindowState::Maximized).unwrap();
    e.set_fullscreen(A, w, true).unwrap();
    e.set_fullscreen(A, w, false).unwrap();
    let info = e.window_info(w).unwrap();
    assert_eq!(
        (info.outer, info.state),
        (e.work_area(), WindowState::Maximized)
    );
    // Restoring a full-screen window leaves full screen for its normal size.
    e.set_fullscreen(A, w, true).unwrap();
    e.request_state(A, w, WindowState::Normal).unwrap();
    assert!(!e.is_fullscreen(w));
    assert_eq!(e.window_info(w).unwrap().outer, r);
}
#[test]
fn destroying_or_disconnecting_the_active_window_activates_the_next_one() {
    // A Win32 application destroying its own window (no close request).
    let mut e = engine();
    let a = create(&mut e, A, rect(0, 0));
    let b = create(&mut e, B, rect(20, 20));
    e.activate(b).unwrap();
    drain(&mut e, A);
    e.destroy_window(B, b).unwrap();
    assert_eq!(e.focused_window(), Some(a));
    assert!(drain(&mut e, A).contains(&Event::Focus {
        window: a,
        focused: true
    }));
    // An application exiting with its window still active.
    let c = create(&mut e, B, rect(40, 40));
    e.activate(c).unwrap();
    e.disconnect(B);
    assert_eq!(e.focused_window(), Some(a));
    // Destroying an inactive window leaves the focus where it is.
    let d = create(&mut e, B, rect(60, 60));
    e.activate(a).unwrap();
    e.destroy_window(B, d).unwrap();
    assert_eq!(e.focused_window(), Some(a));
}
#[test]
fn keyboard_focus_change_revokes_capture_before_a_button_release() {
    let mut e = engine();
    let a = create(&mut e, A, rect(0, 0));
    let b = create(&mut e, B, rect(20, 20));
    e.activate(a).unwrap();
    e.set_capture(A, a).unwrap();
    drain(&mut e, A);
    e.activate(b).unwrap();
    assert_eq!(e.captured_window(), None);
    assert!(drain(&mut e, A).contains(&Event::CaptureLost { window: a }));
    assert_eq!(
        e.route_input(
            Input::PointerButton {
                button: 0,
                down: false,
                x: 40,
                y: 70
            },
            1
        )
        .unwrap(),
        Some(b)
    );
    assert_eq!(e.set_capture(A, a), Err(Error::InvalidState));
}

#[test]
fn pointer_coordinates_follow_moved_content_without_a_repaint_or_epoch_change() {
    let mut e = engine();
    let w = create(&mut e, A, rect(10, 10));
    drain(&mut e, A);
    let epoch = e.window_info(w).unwrap().epoch;
    e.move_window(A, w, 50, 40).unwrap();
    e.route_input(
        Input::PointerButton {
            button: 0,
            down: true,
            x: 71,
            y: 82,
        },
        37,
    )
    .unwrap();
    assert!(drain(&mut e, A).contains(&Event::Input {
        window: w,
        input: Input::PointerButton {
            button: 0,
            down: true,
            x: 21,
            y: 10
        },
        timestamp: 37
    }));
    assert_eq!(e.window_info(w).unwrap().epoch, epoch);
    e.set_capture(A, w).unwrap();
    e.route_input(Input::PointerMove { x: 0, y: 0 }, 38)
        .unwrap();
    assert!(drain(&mut e, A).contains(&Event::Input {
        window: w,
        input: Input::PointerMove { x: -50, y: -72 },
        timestamp: 38
    }));
}

#[test]
fn failed_save_can_cancel_close_without_losing_snapshot_or_authority() {
    let mut e = engine();
    let w = create(&mut e, A, rect(10, 10));
    drain(&mut e, A);
    e.close(A, w).unwrap();
    assert_eq!(e.cancel_close(B, w), Err(Error::WrongOwner));
    e.cancel_close(A, w).unwrap();
    assert_eq!(e.focused_window(), Some(w));
    assert_eq!(e.window_info(w).unwrap().state, WindowState::Normal);
    assert_eq!(e.cancel_close(A, w), Err(Error::InvalidState));
}
#[test]
fn canceled_close_requires_fresh_attachment_and_can_resume_an_interrupted_configure_ack() {
    let mut e = engine();
    let w = create(&mut e, A, rect(10, 10));
    let b = commit_color(&mut e, A, w, [20, 30, 40, 255]);
    let pixels = e.snapshot(w).unwrap().bytes.to_vec();
    let epoch = e.window_info(w).unwrap().epoch;
    drain(&mut e, A);
    e.attach(A, w, b, epoch).unwrap();
    e.close(A, w).unwrap();
    assert_eq!(e.commit(A, w, pixels.clone()), Err(Error::Closing));
    let events = drain(&mut e, A);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event,Event::BufferReleased {buffer} if *buffer==b))
    );
    e.cancel_close(A, w).unwrap();
    assert_eq!(e.commit(A, w, pixels.clone()), Err(Error::NoAttachment));
    // Even an already-ACKed epoch may safely be re-ACKed on close cancellation.
    e.acknowledge_configure(A, w, epoch).unwrap();
    e.attach(A, w, b, epoch).unwrap();
    e.commit(A, w, pixels.clone()).unwrap();
    drain(&mut e, A);
    let next = e.configure(A, w, rect(20, 20)).unwrap();
    drain(&mut e, A);
    e.close(A, w).unwrap();
    assert_eq!(e.acknowledge_configure(A, w, next), Err(Error::Closing));
    drain(&mut e, A);
    e.cancel_close(A, w).unwrap();
    assert_eq!(e.attach(A, w, b, next), Err(Error::NotAcknowledged));
    e.acknowledge_configure(A, w, next).unwrap();
    e.attach(A, w, b, next).unwrap();
    e.commit(A, w, pixels.clone()).unwrap();
    assert_eq!(e.snapshot(w).unwrap().bytes, pixels);
}
use alloc::{vec, vec::Vec};
#[test]
fn cursor_authority_and_inflight_repaint_do_not_consume_a_queued_close() {
    let mut e = engine();
    let w = create(&mut e, A, rect(10, 10));
    let epoch = e.window_info(w).unwrap().epoch;
    drain(&mut e, A);
    assert_eq!(e.set_cursor(B, w, CursorIcon::Text), Err(Error::WrongOwner));
    assert_eq!(
        e.set_cursor(A, w, CursorIcon::NorthSouth),
        Err(Error::InvalidState)
    );
    // The splitter shape the protocol offers (CURSOR_SIZE_WE).
    e.set_cursor(A, w, CursorIcon::EastWest).unwrap();
    assert_eq!(e.window_info(w).unwrap().cursor, CursorIcon::EastWest);
    e.set_cursor(A, w, CursorIcon::Text).unwrap();
    assert_eq!(e.window_info(w).unwrap().cursor, CursorIcon::Text);
    e.close(A, w).unwrap();
    assert_eq!(e.acknowledge_configure(A, w, epoch), Err(Error::Closing));
    assert_eq!(e.set_cursor(A, w, CursorIcon::Arrow), Err(Error::Closing));
    assert_eq!(e.commit(A, w, Vec::new()), Err(Error::Closing));
    assert_eq!(
        drain(&mut e, A)
            .iter()
            .filter(|event| matches!(event, Event::CloseRequested { window } if *window == w))
            .count(),
        1
    );
    e.destroy_window(A, w).unwrap();
    assert_eq!(e.window_info(w).unwrap_err(), Error::UnknownId);
}
const A: Owner = Owner {
    pid: 10,
    session: 2,
    epoch: 1,
};
const B: Owner = Owner {
    pid: 11,
    session: 2,
    epoch: 1,
};
#[test]
fn backpressured_commit_and_failed_resize_preserve_attachment_until_retry() {
    let mut e = Engine::new(
        320,
        240,
        Theme::Light,
        Limits {
            events_per_owner: 8,
            ..Limits::default()
        },
    )
    .unwrap();
    let w = create(&mut e, A, rect(0, 0));
    let b = commit_color(&mut e, A, w, [20, 30, 40, 255]);
    e.activate(w).unwrap();
    drain(&mut e, A);
    let info = e.window_info(w).unwrap();
    let epoch = info.epoch;
    let bytes = info.content.width() as usize * info.content.height() as usize * 4;
    e.attach(A, w, b, epoch).unwrap();
    for code in 0..8 {
        e.route_input(Input::Key { code, down: true }, 0).unwrap();
    }
    let captured = || [20, 30, 40, 255].into_iter().cycle().take(bytes).collect();
    assert_eq!(e.commit(A, w, captured()), Err(Error::QueueFull));
    assert_eq!(e.configure(A, w, rect(10, 10)), Err(Error::QueueFull));
    assert_eq!(e.window_info(w).unwrap().epoch, epoch);
    assert_eq!(e.unregister_buffer(A, b), Err(Error::Busy));
    drain(&mut e, A);
    e.commit(A, w, captured()).unwrap();
    assert_eq!(
        drain(&mut e, A)
            .iter()
            .filter(|event| matches!(event,Event::BufferReleased{buffer} if *buffer==b))
            .count(),
        1
    );
    e.unregister_buffer(A, b).unwrap();
}
#[test]
fn desktop_layers_and_reserved_work_area_obey_window_stacking() {
    let mut e = engine();
    let area = Rect::new(0, 0, 320, 192).unwrap();
    e.set_work_area(area).unwrap();
    let w = create(&mut e, A, rect(10, 10));
    assert_eq!(e.set_work_area(area), Err(Error::Busy));
    commit_color(&mut e, A, w, [0, 0, 0, 255]);
    e.request_state(A, w, WindowState::Maximized).unwrap();
    assert_eq!(e.window_info(w).unwrap().outer, area);
    let frame = e
        .render_with_layers(
            |c| {
                c.fill_rect(Rect::new(0, 0, 320, 240).unwrap(), [12, 34, 56, 255])
                    .unwrap();
            },
            |_, _| {},
            |c| {
                c.fill_rect(Rect::new(0, 230, 320, 10).unwrap(), [90, 80, 70, 255])
                    .unwrap();
            },
        )
        .unwrap();
    assert_eq!(
        &frame.pixels[(220 * 320 + 10) * 4..][..4],
        &[12, 34, 56, 255]
    );
    assert_eq!(
        &frame.pixels[(235 * 320 + 10) * 4..][..4],
        &[90, 80, 70, 255]
    );
    assert_ne!(
        &frame.pixels[(60 * 320 + 30) * 4..][..4],
        &[12, 34, 56, 255]
    );
}
#[test]
fn maximize_without_geometry_change_keeps_the_accepted_submission() {
    let mut e = engine();
    let area = rect(0, 0);
    e.set_work_area(area).unwrap();
    let w = create(&mut e, A, area);
    let b = commit_color(&mut e, A, w, [0, 0, 0, 255]);
    drain(&mut e, A);
    let epoch = e.window_info(w).unwrap().epoch;
    e.attach(A, w, b, epoch).unwrap();
    e.request_state(A, w, WindowState::Maximized).unwrap();
    assert_eq!(e.window_info(w).unwrap().epoch, epoch);
    let len = area.width() as usize * (area.height() - TITLE_HEIGHT) as usize * 4;
    e.commit(A, w, [0, 0, 0, 255].into_iter().cycle().take(len).collect())
        .unwrap();
    assert_eq!(
        drain(&mut e, A)
            .iter()
            .filter(|event| matches!(event,Event::BufferReleased{buffer} if *buffer==b))
            .count(),
        1
    );
}
fn engine() -> Engine {
    Engine::new(320, 240, Theme::Light, Limits::default()).unwrap()
}
fn rect(x: i32, y: i32) -> Rect {
    Rect::new(x, y, 180, 120).unwrap()
}
fn drain(e: &mut Engine, o: Owner) -> Vec<Event> {
    let mut events = Vec::new();
    while let Some(event) = e.poll_event(o) {
        events.push(event);
    }
    events
}
fn create(e: &mut Engine, o: Owner, r: Rect) -> WindowId {
    e.create_window(o, Role::TopLevel, r).unwrap()
}
pub(crate) fn commit_color(e: &mut Engine, o: Owner, w: WindowId, c: [u8; 4]) -> BufferId {
    let info = e.window_info(w).unwrap();
    let epoch = info.epoch;
    let width = info.content.width();
    let height = info.content.height();
    e.acknowledge_configure(o, w, epoch).unwrap();
    let descriptor = SurfaceDescriptor::new(
        width,
        height,
        width as usize * 4,
        width as usize * height as usize * 4,
    )
    .unwrap();
    let b = e.register_buffer(o, descriptor).unwrap();
    e.attach(o, w, b, epoch).unwrap();
    let pixels = (0..width * height).flat_map(|_| c).collect();
    e.commit(o, w, pixels).unwrap();
    b
}
#[test]
fn geometry_checks_arithmetic_and_half_open_edges() {
    assert_eq!(Rect::new(i32::MAX, 0, 1, 1), Err(Error::InvalidGeometry));
    assert!(Rect::new(0, 0, 0, 1).is_err());
    let a = rect(-10, -20);
    assert!(a.contains(-10, -20));
    assert!(!a.contains(170, 0));
    let clipped = a.intersection(Rect::new(0, 0, 320, 240).unwrap()).unwrap();
    assert_eq!(clipped, Rect::new(0, 0, 170, 100).unwrap());
    assert!(Rect::new(i32::MIN, 0, 1, 1).unwrap().expanded(12).is_none());
}
#[test]
fn descriptors_reject_overflow_stride_and_extents() {
    assert!(
        SurfaceDescriptor::new(
            MAX_WIDTH + 1,
            1,
            (MAX_WIDTH as usize + 1) * 4,
            (MAX_WIDTH as usize + 1) * 4
        )
        .is_err()
    );
    assert!(SurfaceDescriptor::new(1, MAX_HEIGHT + 1, 4, (MAX_HEIGHT as usize + 1) * 4).is_err());
    assert!(SurfaceDescriptor::new(4, 2, 12, 24).is_err());
    assert!(SurfaceDescriptor::new(4, 2, 17, 34).is_err());
    assert!(SurfaceDescriptor::new(4, 2, usize::MAX - 3, 32).is_err());
    assert!(SurfaceDescriptor::new(4, 2, 16, 31).is_err());
    assert!(SurfaceDescriptor::new(4, 2, 20, 40).is_ok());
}
#[test]
fn untrusted_roles_owners_and_obsolete_ids_are_denied() {
    let mut e = engine();
    assert_eq!(
        e.create_window(A, Role::LockScreen, rect(0, 0)),
        Err(Error::UnsupportedRole)
    );
    let w = create(&mut e, A, rect(0, 0));
    assert_eq!(e.set_title(B, w, "spoof"), Err(Error::WrongOwner));
    assert_eq!(
        e.create_window(B, Role::OwnedPopup(w), rect(10, 10)),
        Err(Error::WrongOwner)
    );
    let b = commit_color(&mut e, A, w, [1, 2, 3, 255]);
    assert_eq!(e.unregister_buffer(B, b), Err(Error::WrongOwner));
    e.destroy_window(A, w).unwrap();
    let replacement = create(&mut e, A, rect(0, 0));
    assert_ne!(w, replacement);
    assert_eq!(e.set_title(A, w, "old"), Err(Error::UnknownId));
    e.unregister_buffer(A, b).unwrap();
    let descriptor = SurfaceDescriptor::new(4, 2, 16, 32).unwrap();
    let b2 = e.register_buffer(A, descriptor).unwrap();
    assert_ne!(b, b2);
    assert_eq!(e.buffer_descriptor(A, b), Err(Error::UnknownId));
}
#[test]
fn configure_ack_attach_commit_is_atomic_and_keeps_previous_snapshot() {
    let mut e = engine();
    let w = create(&mut e, A, rect(5, 5));
    let b = commit_color(&mut e, A, w, [10, 20, 30, 255]);
    drain(&mut e, A);
    let new = Rect::new(5, 5, 200, 140).unwrap();
    let epoch = e.configure(A, w, new).unwrap();
    assert_eq!(
        e.acknowledge_configure(A, w, epoch - 1),
        Err(Error::StaleEpoch)
    );
    assert_eq!(e.attach(A, w, b, epoch), Err(Error::NotAcknowledged));
    e.acknowledge_configure(A, w, epoch).unwrap();
    assert_eq!(e.attach(A, w, b, epoch), Err(Error::InvalidSurface));
    assert_eq!(&e.snapshot(w).unwrap().bytes[..4], &[10, 20, 30, 255]);
    let descriptor = SurfaceDescriptor::new(200, 108, 800, 86400).unwrap();
    let fresh = e.register_buffer(A, descriptor).unwrap();
    e.attach(A, w, fresh, epoch).unwrap();
    assert_eq!(e.commit(A, w, vec![0; 8]), Err(Error::InvalidSurface));
    assert_eq!(e.unregister_buffer(A, fresh), Err(Error::Busy));
    let mut invalid = vec![0; 86400];
    invalid[0] = 255;
    assert_eq!(e.commit(A, w, invalid), Err(Error::InvalidPixels));
    assert_eq!(&e.snapshot(w).unwrap().bytes[..4], &[10, 20, 30, 255]);
    e.commit(A, w, vec![0; 86400]).unwrap();
    assert_eq!(e.snapshot(w).unwrap().descriptor.width, 200);
    assert!(drain(&mut e, A).contains(&Event::BufferReleased { buffer: fresh }));
}
#[test]
fn snapshot_outlives_buffer_registration_and_client_reuse() {
    let mut e = engine();
    let w = create(&mut e, A, rect(5, 5));
    let b = commit_color(&mut e, A, w, [3, 4, 5, 255]);
    e.unregister_buffer(A, b).unwrap();
    assert_eq!(&e.snapshot(w).unwrap().bytes[..4], &[3, 4, 5, 255]);
    assert!(drain(&mut e, A).contains(&Event::BufferReleased { buffer: b }));
}
#[test]
fn pending_old_attach_cancels_without_invalidating_snapshot() {
    let mut e = engine();
    let w = create(&mut e, A, rect(0, 0));
    let b = commit_color(&mut e, A, w, [0, 0, 0, 255]);
    e.attach(A, w, b, 1).unwrap();
    let epoch = e.configure(A, w, rect(10, 10)).unwrap();
    assert_eq!(e.attach(A, w, b, epoch - 1), Err(Error::StaleEpoch));
    e.unregister_buffer(A, b).unwrap();
    assert!(e.snapshot(w).is_some());
}
#[test]
fn damage_covers_old_new_and_shadow() {
    let mut e = engine();
    let w = create(&mut e, A, rect(20, 20));
    let serial = e.render().unwrap().serial;
    e.complete_transfer(serial, true, 1).unwrap();
    e.move_window(A, w, 60, 70).unwrap();
    let f = e.render().unwrap();
    let d = f.damage.unwrap();
    assert!(d.contains(8, 8));
    assert!(d.contains(251, 201));
    assert!(!d.contains(319, 239));
}
#[test]
fn clipping_and_premultiplication_are_exact() {
    assert_eq!(
        blend_bgra([100, 100, 100, 255], [25, 50, 100, 128]).unwrap(),
        [75, 100, 150, 255]
    );
    assert_eq!(
        blend_bgra([0, 0, 0, 255], [255, 0, 0, 0]),
        Err(Error::InvalidPixels)
    );
    let mut pixels = vec![0; 4 * 4 * 4];
    let mut c = Canvas::new(&mut pixels, 4, 4, 16).unwrap();
    c.fill_rect(Rect::new(-2, -2, 4, 4).unwrap(), [10, 20, 30, 255])
        .unwrap();
    c.pixel(i32::MAX, i32::MIN, [0, 0, 0, 0]).unwrap();
    assert_eq!(&pixels[..4], &[10, 20, 30, 255]);
    assert_eq!(&pixels[8..12], &[0, 0, 0, 0]);
}
#[test]
fn repeat_frames_are_deterministic_and_use_same_allocation() {
    let mut e = engine();
    let w = create(&mut e, A, rect(-5, 10));
    commit_color(&mut e, A, w, [45, 60, 80, 128]);
    drain(&mut e, A);
    let f = e.render().unwrap();
    let serial = f.serial;
    let pointer = f.pixels.as_ptr();
    let first = f.pixels.to_vec();
    e.complete_transfer(serial, true, 10).unwrap();
    let f = e.render().unwrap();
    assert_eq!(f.pixels, first);
    assert_eq!(f.pixels.as_ptr(), pointer);
    assert_eq!(f.damage, None);
}
#[test]
fn release_precedes_honest_transfer_feedback() {
    let mut e = engine();
    let w = create(&mut e, A, rect(0, 0));
    commit_color(&mut e, A, w, [0, 0, 0, 255]);
    let events = drain(&mut e, A);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::BufferReleased { .. }))
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::Presentation { .. }))
    );
    e.request_frame(A, w).unwrap();
    let serial = e.render().unwrap().serial;
    assert!(matches!(e.render(), Err(Error::Busy)));
    assert_eq!(
        e.complete_transfer(serial + 1, true, 1),
        Err(Error::UnknownFrame)
    );
    e.complete_transfer(serial, false, 123).unwrap();
    assert!(e.has_damage());
    let events = drain(&mut e, A);
    assert!(events.contains(&Event::Frame { window: w, serial }));
    assert!(events.contains(&Event::Presentation {
        window: w,
        serial,
        timestamp: 123,
        status: PresentationStatus::TransferFailed
    }));
}
#[test]
fn focus_capture_move_minimize_and_owned_popup_use_authoritative_bounds() {
    let mut e = engine();
    let a = create(&mut e, A, rect(0, 0));
    let b = create(&mut e, B, rect(10, 10));
    e.route_input(
        Input::PointerButton {
            button: 1,
            down: true,
            x: 30,
            y: 60,
        },
        1,
    )
    .unwrap();
    assert!(e.window_info(b).unwrap().active);
    e.set_capture(B, b).unwrap();
    assert_eq!(
        e.route_input(Input::PointerMove { x: -200, y: -200 }, 2)
            .unwrap(),
        Some(b)
    );
    e.request_state(B, b, WindowState::Minimized).unwrap();
    assert_eq!(
        e.route_input(
            Input::Key {
                code: 1,
                down: true
            },
            3
        )
        .unwrap(),
        None
    );
    assert_eq!(e.hit_test(30, 60).unwrap().0, a);
    let p = e
        .create_window(A, Role::OwnedPopup(a), rect(20, 20))
        .unwrap();
    e.activate(a).unwrap();
    assert_eq!(e.hit_test(40, 80).unwrap().0, p);
    e.move_window(A, p, 100, 100).unwrap();
    assert_eq!(e.hit_test(40, 80).unwrap().0, a);
    assert_eq!(e.hit_test(120, 160).unwrap().0, p);
    e.close(A, a).unwrap();
    assert!(e.hit_test(120, 160).is_none());
    e.destroy_window(A, a).unwrap();
    assert!(e.window_info(p).is_err());
}
#[test]
fn maximize_restore_and_close_during_resize() {
    let mut e = engine();
    let original = rect(10, 10);
    let w = create(&mut e, A, original);
    e.request_state(A, w, WindowState::Maximized).unwrap();
    assert_eq!(e.window_info(w).unwrap().outer, e.output_rect());
    e.request_state(A, w, WindowState::Normal).unwrap();
    assert_eq!(e.window_info(w).unwrap().outer, original);
    let epoch = e.window_info(w).unwrap().epoch;
    e.close(A, w).unwrap();
    assert_eq!(e.acknowledge_configure(A, w, epoch), Err(Error::Closing));
    e.destroy_window(A, w).unwrap();
}
#[test]
fn quotas_queue_backpressure_and_hung_client_do_not_block_peers() {
    let limits = Limits {
        windows_per_owner: 1,
        events_per_owner: 8,
        ..Limits::default()
    };
    let mut e = Engine::new(320, 240, Theme::Light, limits).unwrap();
    let a = create(&mut e, A, rect(0, 0));
    assert_eq!(
        e.create_window(A, Role::TopLevel, rect(10, 10)),
        Err(Error::Quota)
    );
    for n in 0..20 {
        e.configure(A, a, rect(n, n)).unwrap();
    }
    assert_eq!(e.pending_events(A), 1);
    let b = create(&mut e, B, rect(10, 10));
    commit_color(&mut e, B, b, [1, 1, 1, 255]);
    assert!(e.snapshot(b).is_some());
    assert!(
        e.register_buffer(
            A,
            SurfaceDescriptor {
                width: 1,
                height: 1,
                stride_bytes: 4,
                backing_bytes: MAX_SURFACE_BYTES + 1,
                format: PixelFormat::PremultipliedBgra8888
            }
        )
        .is_err()
    );
}
#[test]
fn disconnect_and_repeated_open_close_return_resources_to_baseline() {
    let mut e = engine();
    let mut old = None;
    for _ in 0..100 {
        let w = create(&mut e, A, rect(0, 0));
        if let Some(old) = old {
            assert_eq!(e.set_title(A, old, "stale"), Err(Error::UnknownId));
        }
        commit_color(&mut e, A, w, [10, 10, 10, 255]);
        e.disconnect(A);
        assert_eq!(e.resource_counts(), (0, 0, 0));
        old = Some(w);
    }
    let restarted = Owner { epoch: 2, ..A };
    let w = create(&mut e, restarted, rect(0, 0));
    assert_eq!(e.close(A, w), Err(Error::WrongOwner));
}

#[test]
fn rapid_configure_ack_attach_drop_rejects_superseded_sequences() {
    let mut e = engine();
    let w = create(&mut e, A, rect(0, 0));
    let b = commit_color(&mut e, A, w, [1, 2, 3, 255]);
    drain(&mut e, A);
    for n in 0..128 {
        let epoch = e.configure(A, w, rect(n % 60, n % 40)).unwrap();
        assert_eq!(
            e.acknowledge_configure(A, w, epoch - 1),
            Err(Error::StaleEpoch)
        );
        e.acknowledge_configure(A, w, epoch).unwrap();
        e.attach(A, w, b, epoch).unwrap();
        let latest = e.configure(A, w, rect((n + 1) % 60, (n + 1) % 40)).unwrap();
        assert_eq!(e.commit(A, w, vec![]), Err(Error::NoAttachment));
        assert_eq!(e.attach(A, w, b, epoch), Err(Error::StaleEpoch));
        e.acknowledge_configure(A, w, latest).unwrap();
        assert_eq!(&e.snapshot(w).unwrap().bytes[..4], &[1, 2, 3, 255]);
    }
    assert_eq!(e.pending_events(A), 1);
    e.unregister_buffer(A, b).unwrap();
    e.destroy_window(A, w).unwrap();
    assert_eq!(e.resource_counts(), (0, 0, 0));
}
#[test]
fn same_pid_different_session_or_epoch_has_no_authority() {
    let mut e = engine();
    let w = create(&mut e, A, rect(0, 0));
    let b = commit_color(&mut e, A, w, [0, 0, 0, 255]);
    for other in [
        Owner {
            session: A.session + 1,
            ..A
        },
        Owner {
            epoch: A.epoch + 1,
            ..A
        },
    ] {
        assert_eq!(e.acknowledge_configure(other, w, 1), Err(Error::WrongOwner));
        assert_eq!(e.buffer_descriptor(other, b), Err(Error::WrongOwner));
        e.disconnect(other);
        assert!(e.window_info(w).is_ok());
    }
}
#[test]
fn failed_transfer_retains_damage_and_next_transfer_completes() {
    let mut e = engine();
    let w = create(&mut e, A, rect(40, 40));
    commit_color(&mut e, A, w, [3, 4, 5, 255]);
    drain(&mut e, A);
    let frame = e.render().unwrap();
    let serial = frame.serial;
    let pixels = frame.pixels.to_vec();
    e.complete_transfer(serial, false, 10).unwrap();
    assert!(e.has_damage());
    let retry = e.render().unwrap();
    assert_eq!(retry.damage, Some(Rect::new(0, 0, 320, 240).unwrap()));
    assert_eq!(retry.pixels, pixels);
    let serial = retry.serial;
    e.complete_transfer(serial, true, 11).unwrap();
    assert!(!e.has_damage());
    assert!(drain(&mut e, A).iter().any(|event| matches!(
        event,
        Event::Presentation {
            status: PresentationStatus::CpuTransferred,
            timestamp: 11,
            ..
        }
    )));
}
#[test]
fn full_event_queue_backpressures_key_up_without_stalling_other_frames() {
    let mut e = Engine::new(
        320,
        240,
        Theme::Light,
        Limits {
            events_per_owner: 8,
            ..Limits::default()
        },
    )
    .unwrap();
    let a = create(&mut e, A, rect(0, 0));
    let b = create(&mut e, B, rect(20, 20));
    commit_color(&mut e, A, a, [0, 0, 0, 255]);
    commit_color(&mut e, B, b, [0, 0, 0, 255]);
    e.activate(a).unwrap();
    drain(&mut e, A);
    drain(&mut e, B);
    for code in 0..8 {
        e.route_input(Input::Key { code, down: true }, code as u64)
            .unwrap();
    }
    assert_eq!(e.pending_events(A), 8);
    assert_eq!(
        e.route_input(
            Input::Key {
                code: 0,
                down: false
            },
            9
        ),
        Err(Error::QueueFull)
    );
    assert_eq!(e.pending_events(A), 8);
    let serial = e.render().unwrap().serial;
    e.complete_transfer(serial, true, 10).unwrap();
    assert_eq!(e.pending_events(A), 8);
    assert_eq!(e.dropped_notifications(A), 1);
    assert!(
        drain(&mut e, B)
            .iter()
            .any(|event| matches!(event, Event::Presentation { .. }))
    );
    let accepted = drain(&mut e, A);
    assert_eq!(accepted.len(), 8);
    e.route_input(
        Input::Key {
            code: 0,
            down: false,
        },
        11,
    )
    .unwrap();
    assert!(matches!(
        e.poll_event(A),
        Some(Event::Input {
            input: Input::Key { down: false, .. },
            ..
        })
    ));
    let serial = e.render().unwrap().serial;
    e.complete_transfer(serial, true, 12).unwrap();
}
#[test]
fn keyboard_focus_cycle_restores_minimized_and_maximized_windows() {
    let mut e = engine();
    let a = create(&mut e, A, rect(0, 0));
    let b = create(&mut e, B, rect(10, 10));
    e.activate(a).unwrap();
    e.request_state(B, b, WindowState::Maximized).unwrap();
    e.request_state(B, b, WindowState::Minimized).unwrap();
    assert_eq!(e.cycle_focus(false).unwrap(), Some(b));
    assert_eq!(e.window_info(b).unwrap().state, WindowState::Maximized);
    assert_eq!(e.focused_window(), Some(b));
    assert_eq!(e.cycle_focus(true).unwrap(), Some(a));
    e.close(A, a).unwrap();
    assert_eq!(e.cycle_focus(false).unwrap(), Some(b));
}
#[test]
fn popup_damage_covers_outlying_bounds_on_parent_minimize_and_raise() {
    let mut e = engine();
    let parent = create(&mut e, A, rect(0, 0));
    e.create_window(A, Role::OwnedPopup(parent), rect(130, 110))
        .unwrap();
    let serial = e.render().unwrap().serial;
    e.complete_transfer(serial, true, 0).unwrap();
    e.request_state(A, parent, WindowState::Minimized).unwrap();
    let frame = e.render().unwrap();
    assert!(frame.damage.unwrap().contains(319, 239));
    let serial = frame.serial;
    e.complete_transfer(serial, true, 1).unwrap();
    e.restore_window(A, parent).unwrap();
    let serial = e.render().unwrap().serial;
    e.complete_transfer(serial, true, 2).unwrap();
    e.activate(parent).unwrap();
    assert!(e.render().unwrap().damage.unwrap().contains(319, 239));
}
#[test]
fn content_geometry_and_global_memory_quota_are_checked() {
    assert_eq!(
        Engine::content_rect(Rect::new(0, 0, 5, 5).unwrap()),
        Err(Error::InvalidGeometry)
    );
    let d = SurfaceDescriptor::new(4, 2, 20, 40).unwrap();
    assert_eq!(d.required_bytes().unwrap(), 40);
    let mut e = Engine::new(
        320,
        240,
        Theme::Light,
        Limits {
            total_bytes: 40,
            ..Limits::default()
        },
    )
    .unwrap();
    e.register_buffer(A, d).unwrap();
    assert_eq!(e.register_buffer(B, d), Err(Error::Quota));
}

#[test]
fn per_window_chrome_callback_respects_upper_window_occlusion_and_title_gutters() {
    let mut e = engine();
    let lower = create(&mut e, A, Rect::new(0, 40, 260, 120).unwrap());
    // Clear of the upper window's shadow (WINDOW_SHADOW: 52 px at the sides).
    let upper = create(&mut e, B, Rect::new(100, 0, 180, 120).unwrap());
    e.set_title(A, lower, "lower").unwrap();
    e.set_title(B, upper, "upper").unwrap();
    let mut visited = Vec::new();
    let frame = e
        .render_with_chrome(|info, canvas| {
            visited.push(info.id);
            if info.id == lower {
                let clip = Engine::title_clip(info.outer).unwrap();
                canvas.fill_rect(clip, [0, 0, 255, 255]).unwrap();
            }
        })
        .unwrap();
    assert_eq!(visited, vec![lower, upper]);
    let pixel = |x: usize, y: usize| -> &[u8] {
        &frame.pixels[y * frame.stride_bytes + x * 4..y * frame.stride_bytes + x * 4 + 4]
    };
    assert_eq!(pixel(20, 50), [0, 0, 255, 255]);
    assert_eq!(pixel(150, 50), Theme::Light.tokens().content);
    assert_eq!(pixel(20, 42), Theme::Light.tokens().inactive_chrome);
    assert_eq!(
        Engine::title_clip(rect(0, 0)).unwrap(),
        Rect::new(12, 4, 18, 24).unwrap()
    );
    assert!(Engine::title_clip(Rect::new(0, 0, 160, 64).unwrap()).is_none());
}

#[test]
fn captures_recycle_on_publish_destroy_and_disconnect_without_losing_failed_ownership() {
    let mut e = engine();
    let id = create(&mut e, A, rect(10, 10));
    let buffer = commit_color(&mut e, A, id, [1, 2, 3, 255]);
    let bytes = e.snapshot_capacity(id);
    let mut replacement = alloc::vec![255;bytes];
    let replacement_address = replacement.as_ptr();
    e.attach(A, id, buffer, e.window_info(id).unwrap().epoch)
        .unwrap();
    let rejected_address = replacement.as_ptr();
    assert_eq!(
        e.commit_recycling(B, id, &mut replacement),
        Err(Error::WrongOwner)
    );
    assert_eq!(replacement.as_ptr(), rejected_address);
    e.commit_recycling(A, id, &mut replacement).unwrap();
    assert_eq!(replacement.len(), bytes);
    assert_ne!(replacement.as_ptr(), replacement_address);
    let mut retired = Vec::new();
    e.destroy_window_recycling(A, id, &mut |pixels| retired.push(pixels))
        .unwrap();
    assert_eq!(retired.len(), 1);
    assert_eq!(retired[0].as_ptr(), replacement_address);
    let parent = create(&mut e, A, rect(20, 20));
    commit_color(&mut e, A, parent, [2, 3, 4, 255]);
    let child = e
        .create_window(A, Role::OwnedPopup(parent), rect(25, 25))
        .unwrap();
    let epoch = e.window_info(child).unwrap().epoch;
    e.acknowledge_configure(A, child, epoch).unwrap();
    commit_color(&mut e, A, child, [3, 4, 5, 255]);
    e.disconnect_recycling(A, &mut |pixels| retired.push(pixels));
    assert_eq!(retired.len(), 3);
    assert!(e.windows().next().is_none());
}

#[test]
fn high_dpi_rejects_too_short_geometry_before_publication_and_matches_chrome_hit() {
    let mut e = Engine::new(3840, 2160, Theme::Light, Limits::default()).unwrap();
    e.set_scale(2).unwrap();
    assert_eq!(
        e.create_window(A, Role::TopLevel, Rect::new(0, 0, 320, 64).unwrap()),
        Err(Error::InvalidGeometry)
    );
    assert!(e.windows().next().is_none());
    let id = e
        .create_window(A, Role::TopLevel, Rect::new(0, 0, 800, 400).unwrap())
        .unwrap();
    assert_eq!(
        e.window_info(id).unwrap().content,
        Rect::new(0, 64, 800, 336).unwrap()
    );
    let old = e.window_info(id).unwrap().epoch;
    assert_eq!(
        e.configure(A, id, Rect::new(0, 0, 800, 64).unwrap()),
        Err(Error::InvalidGeometry)
    );
    assert_eq!(e.window_info(id).unwrap().epoch, old);
    assert_eq!(e.hit_test(750, 30), Some((id, HitRegion::Close)));
    assert_eq!(e.hit_test(650, 30), Some((id, HitRegion::Maximize)));
    assert_eq!(e.hit_test(50, 60), Some((id, HitRegion::TitleBar)));
    assert_eq!(e.hit_test(50, 70), Some((id, HitRegion::Client)));
    // WS_MINIMIZEBOX alone: Maximize still shows but is greyed and inert.
    assert_eq!(
        e.set_caption_buttons(B, id, CAPTION_MINIMIZE),
        Err(Error::WrongOwner)
    );
    e.set_caption_buttons(A, id, CAPTION_MINIMIZE).unwrap();
    assert_eq!(e.hit_test(650, 30), Some((id, HitRegion::Maximize)));
    assert!(!e.caption_button_enabled(id, HitRegion::Maximize));
    assert!(e.caption_button_enabled(id, HitRegion::Minimize));
    // A dialog frame (neither): only Close; the rest is title bar.
    e.set_caption_buttons(A, id, 0).unwrap();
    assert_eq!(e.hit_test(750, 30), Some((id, HitRegion::Close)));
    assert_eq!(e.hit_test(650, 30), Some((id, HitRegion::TitleBar)));
    assert_eq!(e.hit_test(700, 30), Some((id, HitRegion::TitleBar)));
    assert_eq!(e.set_caption_buttons(A, id, 4), Err(Error::InvalidState));
}

fn caption_equal(actual: &[u8], expected: &[u8], width: usize) {
    assert_eq!(actual.len(), expected.len());
    if let Some(at) = actual.iter().zip(expected).position(|(a, b)| a != b) {
        let pixel = at / 4;
        panic!(
            "caption mismatch at {},{}: {:?} != {:?}",
            pixel % width,
            pixel / width,
            &actual[pixel * 4..pixel * 4 + 4],
            &expected[pixel * 4..pixel * 4 + 4]
        );
    }
}
pub(crate) fn caption_capture(e: &mut Engine, partial: bool) -> (Vec<u8>, Option<Rect>) {
    let frame = if partial {
        e.repaint_captions().unwrap()
    } else {
        e.render().unwrap()
    };
    let serial = frame.serial;
    let damage = frame.damage;
    let pixels = frame.pixels.to_vec();
    e.complete_transfer(serial, true, 123).unwrap();
    (pixels, damage)
}
fn caption_sample(pixels: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
    pixels[(y * width + x) * 4..(y * width + x + 1) * 4]
        .try_into()
        .unwrap()
}
#[test]
fn caption_hover_press_leave_repaints_only_the_button_without_client_notifications() {
    let mut e = Engine::new(640, 480, Theme::Light, Limits::default()).unwrap();
    let r = Rect::new(20, 20, 400, 240).unwrap();
    let id = create(&mut e, A, r);
    e.activate(id).unwrap();
    commit_color(&mut e, A, id, [100, 120, 140, 255]);
    let (normal, _) = caption_capture(&mut e, false);
    drain(&mut e, A);
    assert!(e.set_caption_pointer(397, 36, false));
    assert!(!e.has_damage());
    let (hover, damage) = caption_capture(&mut e, true);
    assert_eq!(damage, Some(Rect::new(374, 20, 46, 32).unwrap()));
    assert_eq!(caption_sample(&hover, 640, 380, 40), [35, 35, 232, 255]);
    assert!(
        caption_sample(&hover, 640, 396, 35)
            .iter()
            .all(|c| *c >= 248)
    );
    assert!(!e.set_caption_pointer(399, 37, false));
    assert_eq!(e.repaint_captions().err(), Some(Error::InvalidState));
    assert!(e.set_caption_pointer(397, 36, true));
    let (pressed, _) = caption_capture(&mut e, true);
    assert_eq!(caption_sample(&pressed, 640, 380, 40), [45, 45, 197, 255]);
    assert!(e.set_caption_pointer(280, 36, true));
    let (outside, _) = caption_capture(&mut e, true);
    assert_eq!(outside, normal);
    assert!(e.set_caption_pointer(397, 36, true));
    assert_eq!(caption_capture(&mut e, true).0, pressed);
    assert!(e.set_caption_pointer(397, 36, false));
    assert_eq!(caption_capture(&mut e, true).0, hover);
    assert!(e.set_caption_pointer(10, 10, false));
    assert_eq!(caption_capture(&mut e, true).0, normal);
    assert!(drain(&mut e, A).is_empty());
}
#[test]
fn dark_high_dpi_client_caption_restores_the_native_snapshot_and_holds_hover_on_commit() {
    let mut e = Engine::new(1280, 720, Theme::Dark, Limits::default()).unwrap();
    e.set_scale(2).unwrap();
    let r = Rect::new(20, 20, 1000, 500).unwrap();
    let id = e
        .create_client_caption(
            A,
            r,
            CaptionLayout {
                tabbed: false,
                height: 32,
                max_width: 1032,
                side_margin: 600,
            },
        )
        .unwrap();
    e.activate(id).unwrap();
    commit_color(&mut e, A, id, [80, 90, 100, 255]);
    let (normal, _) = caption_capture(&mut e, false);
    drain(&mut e, A);
    assert!(e.set_caption_pointer(790, 52, false));
    let (hover, damage) = caption_capture(&mut e, true);
    assert_eq!(damage, Some(Rect::new(744, 20, 92, 64).unwrap()));
    assert_eq!(caption_sample(&hover, 1280, 754, 40), [61, 61, 61, 255]);
    assert!(e.set_caption_pointer(790, 52, true));
    let (pressed, _) = caption_capture(&mut e, true);
    assert_eq!(caption_sample(&pressed, 1280, 754, 40), [50, 50, 50, 255]);
    assert!(e.set_caption_pointer(100, 100, false));
    assert_eq!(caption_capture(&mut e, true).0, normal);
    assert!(e.set_caption_pointer(974, 52, false));
    let (hover, _) = caption_capture(&mut e, true);
    assert_eq!(caption_sample(&hover, 1280, 938, 40), [35, 35, 232, 255]);
    assert_eq!(caption_capture(&mut e, false).0, hover);
    commit_color(&mut e, A, id, [60, 70, 80, 255]);
    let (changed, _) = caption_capture(&mut e, false);
    assert_eq!(caption_sample(&changed, 1280, 938, 40), [35, 35, 232, 255]);
    assert!(e.set_caption_pointer(100, 100, false));
    let (restored, _) = caption_capture(&mut e, true);
    assert_eq!(caption_sample(&restored, 1280, 938, 40), [60, 70, 80, 255]);
}
#[test]
fn caption_overlay_is_transfer_safe_and_does_not_arm_when_dragging_into_a_button() {
    let mut e = engine();
    let id = create(&mut e, A, rect(10, 10));
    e.activate(id).unwrap();
    caption_capture(&mut e, false);
    assert!(!e.set_caption_pointer(20, 60, true));
    assert!(!e.set_caption_pointer(167, 26, true));
    assert!(e.set_caption_pointer(167, 26, false));
    let frame = e.repaint_captions().unwrap();
    let serial = frame.serial;
    let immutable = frame.pixels.to_vec();
    assert!(e.set_caption_pointer(121, 26, false));
    assert_eq!(e.transfer_view(serial).unwrap().pixels, immutable);
    assert_eq!(e.repaint_captions().err(), Some(Error::Busy));
    assert_eq!(
        e.transfer_view(serial + 1)
            .err()
            .map(|_| Error::UnknownFrame),
        Some(Error::UnknownFrame)
    );
    e.complete_transfer(serial, true, 1).unwrap();
    let (next, _) = caption_capture(&mut e, true);
    assert_eq!(
        caption_sample(&next, 320, 151, 34),
        Theme::Light.tokens().chrome
    );
    assert_eq!(caption_sample(&next, 320, 104, 34), [225, 225, 225, 255]);
}
#[test]
fn partial_caption_pixels_match_full_composition_through_upper_windows_and_shadows() {
    let mut e = Engine::new(640, 480, Theme::Light, Limits::default()).unwrap();
    let bottom = create(&mut e, A, Rect::new(20, 20, 400, 200).unwrap());
    let upper = create(&mut e, B, Rect::new(400, 30, 180, 150).unwrap());
    commit_color(&mut e, A, bottom, [100, 120, 140, 255]);
    commit_color(&mut e, B, upper, [70, 80, 90, 255]);
    e.activate(upper).unwrap();
    caption_capture(&mut e, false);
    assert_eq!(e.hit_test(390, 25), Some((bottom, HitRegion::Close)));
    assert!(e.set_caption_pointer(390, 25, false));
    let (partial, _) = caption_capture(&mut e, true);
    caption_equal(&caption_capture(&mut e, false).0, &partial, 640);
    assert!(e.set_caption_pointer(10, 10, false));
    let (normal, _) = caption_capture(&mut e, true);
    caption_equal(&caption_capture(&mut e, false).0, &normal, 640);
    assert_eq!(e.hit_test(410, 40).unwrap().0, upper);
    assert!(!e.set_caption_pointer(410, 40, false));
}

#[test]
fn workspaces_revoke_input_preserve_snapshots_and_reject_recycled_ids() {
    let mut e = Engine::new(640, 480, Theme::Light, Limits::default()).unwrap();
    let first = e.active_workspace();
    let a = create(&mut e, A, rect(10, 10));
    commit_color(&mut e, A, a, [33, 55, 77, 255]);
    let epoch = e.window_info(a).unwrap().epoch;
    let popup = e
        .create_window(A, Role::OwnedPopup(a), Rect::new(20, 20, 180, 80).unwrap())
        .unwrap();
    e.activate(a).unwrap();
    e.set_capture(A, a).unwrap();
    drain(&mut e, A);
    let second = e.create_workspace().unwrap();
    e.switch_workspace(second).unwrap();
    assert_eq!(e.focused_window(), None);
    assert_eq!(e.captured_window(), None);
    assert!(!e.window_info(a).unwrap().visible);
    assert!(!e.window_info(popup).unwrap().visible);
    assert!(e.snapshot(a).is_some());
    assert_eq!(e.window_info(a).unwrap().epoch, epoch);
    assert!(e.activate(a).is_err());
    assert!(e.set_capture(A, a).is_err());
    assert!(e.move_to_workspace(B, a, second).is_err());
    let b = create(&mut e, B, rect(300, 40));
    e.activate(b).unwrap();
    assert_eq!(e.cycle_focus(false).unwrap(), Some(b));
    e.switch_workspace(first).unwrap();
    assert_eq!(e.cycle_focus(false).unwrap(), Some(a));
    e.move_to_workspace(A, a, second).unwrap();
    assert_eq!(e.focused_window(), None);
    assert_eq!(e.window_info(popup).unwrap().workspace, second);
    e.switch_workspace(second).unwrap();
    e.remove_workspace(second).unwrap();
    assert_eq!(e.active_workspace(), first);
    assert!(e.window_info(a).unwrap().visible);
    assert_eq!(e.window_info(popup).unwrap().workspace, first);
    let replacement = e.create_workspace().unwrap();
    assert_ne!(second, replacement);
    assert!(e.switch_workspace(second).is_err());
    assert!(e.remove_workspace(second).is_err());
    e.remove_workspace(replacement).unwrap();
    assert!(e.remove_workspace(first).is_err());
    for _ in 0..7 {
        e.create_workspace().unwrap();
    }
    assert_eq!(e.create_workspace(), Err(Error::Quota));
}
#[test]
fn snap_geometry_uses_work_area_and_restores_original_after_drag_or_maximize() {
    let mut e = Engine::new(641, 480, Theme::Light, Limits::default()).unwrap();
    e.set_work_area(Rect::new(0, 0, 641, 432).unwrap()).unwrap();
    let original = Rect::new(40, 50, 300, 200).unwrap();
    let a = e.create_window(A, Role::TopLevel, original).unwrap();
    drain(&mut e, A);
    e.snap_window(A, a, Snap::Right, None).unwrap();
    assert_eq!(
        e.window_info(a).unwrap().outer,
        Rect::new(320, 0, 321, 432).unwrap()
    );
    e.snap_window(A, a, Snap::TopLeft, None).unwrap();
    assert_eq!(
        e.window_info(a).unwrap().outer,
        Rect::new(0, 0, 320, 216).unwrap()
    );
    e.request_state(A, a, WindowState::Maximized).unwrap();
    e.restore_window(A, a).unwrap();
    assert_eq!(e.window_info(a).unwrap().outer, original);
    e.move_window(A, a, 120, 0).unwrap();
    e.snap_window(A, a, Snap::Maximize, Some(original)).unwrap();
    e.restore_window(A, a).unwrap();
    assert_eq!(e.window_info(a).unwrap().outer, original);
    e.snap_window(A, a, Snap::Left, None).unwrap();
    e.unsnap_window(A, a).unwrap();
    assert_eq!(e.window_info(a).unwrap().outer, original);
    assert!(e.snap_window(B, a, Snap::Right, None).is_err());
}

#[test]
fn close_during_title_drag_cannot_snap_revive_or_reconfigure_window() {
    let mut e = engine();
    let w = create(&mut e, A, rect(40, 50));
    drain(&mut e, A);
    e.close(A, w).unwrap();
    let before = e.window_info(w).unwrap();
    let (epoch, outer) = (before.epoch, before.outer);
    assert_eq!(
        e.snap_window(A, w, Snap::Maximize, Some(rect(40, 50))),
        Err(Error::Closing)
    );
    assert_eq!(e.unsnap_window(A, w), Err(Error::Closing));
    let after = e.window_info(w).unwrap();
    assert_eq!(
        (after.state, after.epoch, after.outer),
        (WindowState::Closing, epoch, outer)
    );
    assert!(
        drain(&mut e, A)
            .iter()
            .any(|event| matches!(event,Event::CloseRequested{window} if *window==w))
    );
}

#[test]
fn tab_drop_requires_captured_source_committed_same_owner_caption_and_safe_coordinates() {
    let mut e = Engine::new(1280, 800, Theme::Light, Limits::default()).unwrap();
    let caption = CaptionLayout {
        tabbed: true,
        height: 40,
        max_width: 240,
        side_margin: 8,
    };
    let source = e
        .create_client_caption(A, Rect::new(10, 300, 500, 300).unwrap(), caption)
        .unwrap();
    let dest = e
        .create_client_caption(A, Rect::new(600, 20, 500, 300).unwrap(), caption)
        .unwrap();
    commit_color(&mut e, A, source, [10, 20, 30, 255]);
    e.activate(source).unwrap();
    drain(&mut e, A);
    assert_eq!(
        e.tab_drop_target(A, source, 620, 40),
        Err(Error::InvalidState)
    );
    e.set_capture(A, source).unwrap();
    assert_eq!(
        e.tab_drop_target(B, source, 620, 40),
        Err(Error::WrongOwner)
    );
    assert_eq!(e.tab_drop_target(A, source, 620, 40), Ok(None));
    commit_color(&mut e, A, dest, [10, 20, 30, 255]);
    assert_eq!(
        e.tab_drop_target(A, source, 620, 40),
        Ok(Some((dest, 20, 20)))
    );
    assert_eq!(e.tab_drop_target(A, source, 1080, 40), Ok(None));
    assert_eq!(e.tab_drop_target(A, source, 620, 100), Ok(None));
    let foreign = e
        .create_client_caption(B, Rect::new(600, 20, 500, 300).unwrap(), caption)
        .unwrap();
    commit_color(&mut e, B, foreign, [10, 20, 30, 255]);
    e.activate(foreign).unwrap();
    assert_eq!(
        e.tab_drop_target(A, source, 620, 40),
        Err(Error::InvalidState)
    );
    e.activate(source).unwrap();
    drain(&mut e, A);
    e.set_capture(A, source).unwrap();
    assert_eq!(e.tab_drop_target(A, source, 620, 40), Ok(None));
}

#[test]
fn owned_activation_preserves_maximized_geometry_and_epoch_and_rejects_foreign_or_closing() {
    let mut e = Engine::new(1280, 800, Theme::Light, Limits::default()).unwrap();
    let a = create(&mut e, A, rect(0, 0));
    let b = create(&mut e, A, rect(30, 40));
    e.request_state(A, a, WindowState::Maximized).unwrap();
    let info = {
        let w = e.window_info(a).unwrap();
        (w.outer, w.epoch)
    };
    e.activate(b).unwrap();
    assert_eq!(e.activate_owned(B, a), Err(Error::WrongOwner));
    e.activate_owned(A, a).unwrap();
    let after = e.window_info(a).unwrap();
    assert_eq!(after.state, WindowState::Maximized);
    assert_eq!(after.outer, info.0);
    assert_eq!(after.epoch, info.1);
    e.close(A, a).unwrap();
    assert_eq!(e.activate_owned(A, a), Err(Error::Closing));
}
#[test]
fn band_commits_patch_only_validated_rows_and_match_full_captures() {
    let mut e = engine();
    let w = create(&mut e, A, rect(30, 20));
    let b = commit_color(&mut e, A, w, [20, 30, 40, 255]);
    let serial = e.render().unwrap().serial;
    e.complete_transfer(serial, true, 0).unwrap();
    drain(&mut e, A);
    let (width, height, epoch) = {
        let info = e.window_info(w).unwrap();
        (
            info.content.width() as usize,
            info.content.height(),
            info.epoch,
        )
    };
    let stride = width * 4;
    let mut expected = e.snapshot(w).unwrap().bytes.to_vec();
    let revision = e.snapshot(w).unwrap().revision;
    // Rows 5..8 change; the band carries rows 4..9.
    let mut band = expected[4 * stride..9 * stride].to_vec();
    for p in band[stride..4 * stride].chunks_exact_mut(4) {
        p.copy_from_slice(&[1, 2, 3, 255]);
    }
    expected[4 * stride..9 * stride].copy_from_slice(&band);
    e.attach(A, w, b, epoch).unwrap();
    assert_eq!(e.band_geometry(A, w), Some((stride, height)));
    e.commit_band(A, w, 4, &band).unwrap();
    let snap = e.snapshot(w).unwrap();
    assert_eq!(snap.bytes, &expected[..]);
    assert_eq!(snap.revision, revision + 1);
    assert!(e.has_damage());
    assert!(drain(&mut e, A).contains(&Event::BufferReleased { buffer: b }));
    // An unchanged band commits without damage.
    let serial = e.render().unwrap().serial;
    e.complete_transfer(serial, true, 0).unwrap();
    e.attach(A, w, b, epoch).unwrap();
    e.commit_band(A, w, 4, &expected[4 * stride..9 * stride])
        .unwrap();
    assert!(!e.has_damage());
    // Invalid premultiplied pixels and out-of-range bands are refused and
    // leave the snapshot untouched.
    let mut bad = expected[..stride].to_vec();
    bad[..4].copy_from_slice(&[200, 0, 0, 10]);
    e.attach(A, w, b, epoch).unwrap();
    assert_eq!(e.commit_band(A, w, 0, &bad), Err(Error::InvalidPixels));
    assert_eq!(
        e.commit_band(A, w, height - 1, &expected[..2 * stride]),
        Err(Error::InvalidSurface)
    );
    assert_eq!(e.snapshot(w).unwrap().bytes, &expected[..]);
    // A translucent band clears opacity until those rows become opaque again.
    let mut clear = expected[..stride].to_vec();
    clear[..4].copy_from_slice(&[0, 0, 0, 0]);
    e.commit_band(A, w, 0, &clear).unwrap();
    e.attach(A, w, b, epoch).unwrap();
    e.commit_band(A, w, 0, &expected[..stride]).unwrap();
    assert_eq!(e.snapshot(w).unwrap().bytes, &expected[..]);
}
#[test]
fn band_commits_require_a_snapshot_from_the_same_buffer() {
    let mut e = engine();
    let w = create(&mut e, A, rect(30, 20));
    let (width, height, epoch) = {
        let info = e.window_info(w).unwrap();
        (info.content.width(), info.content.height(), info.epoch)
    };
    let descriptor = SurfaceDescriptor::new(
        width,
        height,
        width as usize * 4,
        (width * height * 4) as usize,
    )
    .unwrap();
    e.acknowledge_configure(A, w, epoch).unwrap();
    let first = e.register_buffer(A, descriptor).unwrap();
    e.attach(A, w, first, epoch).unwrap();
    // No snapshot yet: the producer must send a full capture.
    assert_eq!(e.band_geometry(A, w), None);
    let row = alloc::vec![255u8; width as usize * 4];
    assert_eq!(e.commit_band(A, w, 0, &row), Err(Error::InvalidState));
    e.commit(A, w, alloc::vec![255; (width * height * 4) as usize])
        .unwrap();
    let second = e.register_buffer(A, descriptor).unwrap();
    e.attach(A, w, second, epoch).unwrap();
    assert_eq!(
        e.band_geometry(A, w),
        None,
        "a different buffer needs a full capture"
    );
}
#[test]
fn accent_title_bars_paint_only_the_active_window_title_and_border() {
    for theme in [Theme::Light, Theme::Dark] {
        let mut e = Engine::new(640, 480, theme, Limits::default()).unwrap();
        let a = create(&mut e, A, Rect::new(20, 20, 260, 200).unwrap());
        let b = create(&mut e, B, Rect::new(340, 20, 260, 200).unwrap());
        e.activate(b).unwrap();
        e.activate(a).unwrap();
        let neutral = |e: &mut Engine| {
            let frame = e.render().unwrap();
            let at = |x: usize, y: usize| (y * 640 + x) * 4;
            let title = frame.pixels[at(60, 30)..at(60, 30) + 4].to_vec();
            let serial = frame.serial;
            e.complete_transfer(serial, true, 0).unwrap();
            title
        };
        let before = neutral(&mut e);
        assert_eq!(e.title_accent(), None, "off by default");
        // Windows' default blue accent, BGRA.
        let accent = [0xd4, 0x78, 0x00, 255];
        e.set_title_accent(Some(accent));
        assert!(e.has_damage(), "live on change");
        let frame = e.render().unwrap();
        let px = |x: i32, y: i32| {
            let at = (y as usize * 640 + x as usize) * 4;
            [
                frame.pixels[at],
                frame.pixels[at + 1],
                frame.pixels[at + 2],
                frame.pixels[at + 3],
            ]
        };
        // The active title bar (away from its glyphs) is the accent, and so
        // is the 1 px border at the middle of each edge.
        assert_eq!(px(60, 30), accent, "{theme:?} active title");
        assert_eq!(px(20 + 130, 20), accent, "top border");
        assert_eq!(px(20, 20 + 100), accent, "left border");
        // The inactive window keeps the theme's neutral chrome.
        let tokens = theme.tokens();
        assert_eq!(
            px(380, 30),
            tokens.inactive_chrome,
            "{theme:?} inactive title"
        );
        assert_eq!(px(340 + 130, 20), tokens.border, "inactive border");
        let serial = frame.serial;
        e.complete_transfer(serial, true, 1).unwrap();
        // Activating the other window moves the accent with the focus.
        e.activate(b).unwrap();
        let frame = e.render().unwrap();
        let at = (30 * 640 + 380) * 4;
        assert_eq!(&frame.pixels[at..at + 4], &accent);
        let serial = frame.serial;
        e.complete_transfer(serial, true, 2).unwrap();
        e.set_title_accent(None);
        e.activate(a).unwrap();
        assert_eq!(neutral(&mut e), before, "off restores the theme chrome");
    }
    // Caption glyphs and text contrast with the accent.
    assert_eq!(accent_foreground([0xd4, 0x78, 0x00, 255]), [255; 4]);
    assert_eq!(accent_foreground([0x40, 0xd0, 0xff, 255]), [0, 0, 0, 255]);
}
