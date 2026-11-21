//! Wheel routing, live acrylic, interior span parity and window transitions.
use super::*;
use std::vec::Vec;

const A: Owner = Owner {
    pid: 11,
    session: 1,
    epoch: 1,
};
const B: Owner = Owner {
    pid: 12,
    session: 1,
    epoch: 1,
};

fn tabbed(e: &mut Engine, owner: Owner, r: Rect) -> WindowId {
    e.create_client_caption(
        owner,
        r,
        CaptionLayout {
            tabbed: true,
            height: 32,
            side_margin: 0,
            max_width: 2048,
        },
    )
    .unwrap()
}
/// Commit `pixel(x, y)` over the window's content (premultiplied BGRA).
fn commit_with(
    e: &mut Engine,
    owner: Owner,
    id: WindowId,
    pixel: impl Fn(u32, u32) -> [u8; 4],
) -> BufferId {
    let info = e.window_info(id).unwrap();
    let (epoch, width, height) = (info.epoch, info.content.width(), info.content.height());
    e.acknowledge_configure(owner, id, epoch).unwrap();
    let descriptor = SurfaceDescriptor::new(
        width,
        height,
        width as usize * 4,
        width as usize * height as usize * 4,
    )
    .unwrap();
    let buffer = e.register_buffer(owner, descriptor).unwrap();
    e.attach(owner, id, buffer, epoch).unwrap();
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&pixel(x, y));
        }
    }
    e.commit(owner, id, pixels).unwrap();
    buffer
}
fn drain(e: &mut Engine, owner: Owner) -> Vec<Event> {
    let mut events = Vec::new();
    while let Some(event) = e.poll_event(owner) {
        events.push(event);
    }
    events
}
fn gradient(width: u32, height: u32) -> Vec<u8> {
    let mut pixels = Vec::new();
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&[(x * 7 % 251) as u8, (y * 3 % 241) as u8, 90, 255]);
        }
    }
    pixels
}
/// Full frame with a flat gray desktop, returning (pixels, damage).
fn frame(e: &mut Engine, partial: bool) -> (Vec<u8>, Option<Rect>) {
    let output = e.output_rect();
    let background = |c: &mut Canvas<'_>| {
        c.fill_rect(output, [96, 96, 96, 255]).unwrap();
    };
    let frame = if partial {
        e.render_damage_with_layers(background, |_, _| {}, |_| {})
            .unwrap()
    } else {
        e.render_with_background(background, |_, _| {}).unwrap()
    };
    let (pixels, damage, serial) = (frame.pixels.to_vec(), frame.damage, frame.serial);
    e.complete_transfer(serial, true, 0).unwrap();
    drain(e, A);
    drain(e, B);
    (pixels, damage)
}
fn px(pixels: &[u8], width: u32, x: i32, y: i32) -> [u8; 4] {
    let at = (y as usize * width as usize + x as usize) * 4;
    pixels[at..at + 4].try_into().unwrap()
}

#[test]
fn wheel_targets_the_window_under_the_pointer_without_activation_and_coalesces() {
    let mut e = Engine::new(640, 480, Theme::Light, Limits::default()).unwrap();
    let a = e
        .create_window(A, Role::TopLevel, Rect::new(10, 10, 240, 200).unwrap())
        .unwrap();
    let b = e
        .create_window(B, Role::TopLevel, Rect::new(300, 10, 240, 200).unwrap())
        .unwrap();
    commit_with(&mut e, A, a, |_, _| [1, 2, 3, 255]);
    commit_with(&mut e, B, b, |_, _| [1, 2, 3, 255]);
    e.activate(b).unwrap();
    drain(&mut e, A);
    drain(&mut e, B);
    let content = e.window_info(a).unwrap().content;
    let wheel = |x, y, delta| Input::Wheel { x, y, delta };
    assert_eq!(e.route_input(wheel(50, 100, 1), 7), Ok(Some(a)));
    assert_eq!(e.route_input(wheel(52, 101, 2), 8), Ok(Some(a)));
    assert_eq!(e.focused_window(), Some(b), "the wheel never activates");
    let events = drain(&mut e, A);
    assert_eq!(
        events,
        [Event::Input {
            window: a,
            input: wheel(52 - content.x(), 101 - content.y(), 3),
            timestamp: 8,
        }]
    );
    // A button between two wheels keeps its order (no merge across it).
    e.route_input(wheel(50, 100, -1), 9).unwrap();
    e.route_input(
        Input::PointerButton {
            button: 0,
            down: false,
            x: 50,
            y: 100,
        },
        10,
    )
    .unwrap();
    e.route_input(wheel(50, 100, -1), 11).unwrap();
    let events = drain(&mut e, A);
    assert_eq!(events.len(), 3);
    assert!(matches!(
        events[2],
        Event::Input {
            input: Input::Wheel { delta: -1, .. },
            ..
        }
    ));
    // Saturation and nothing outside windows.
    for _ in 0..300 {
        e.route_input(wheel(50, 100, 1), 12).unwrap();
    }
    assert!(matches!(
        drain(&mut e, A)[..],
        [Event::Input {
            input: Input::Wheel {
                delta: MAX_WHEEL_DELTA,
                ..
            },
            ..
        }]
    ));
    assert_eq!(e.route_input(wheel(620, 460, 1), 13), Ok(None));
    // The capture owner receives the wheel wherever the pointer is.
    e.set_capture(B, b).unwrap();
    assert_eq!(e.route_input(wheel(50, 100, 1), 14), Ok(Some(b)));
    e.release_capture(B, b).unwrap();
    drain(&mut e, B);
    // A saturated queue drops the advisory wheel instead of failing.
    e.activate(a).unwrap();
    while e
        .route_input(
            Input::Key {
                code: 30,
                down: true,
            },
            15,
        )
        .is_ok()
    {}
    let dropped = e.dropped_notifications(A);
    e.route_input(Input::PointerMove { x: 60, y: 60 }, 16).err();
    assert_eq!(e.route_input(wheel(50, 100, 1), 17), Ok(None));
    assert_eq!(e.dropped_notifications(A), dropped + 1);
}

#[test]
fn translucent_row_spans_match_the_scalar_contour_reference() {
    for scale in 1..=3u32 {
        for theme in [Theme::Light, Theme::Dark] {
            for (tabbed_caption, material) in [
                (false, Material::Default),
                (true, Material::Default),
                (true, Material::Acrylic),
                (false, Material::Acrylic),
                (false, Material::Mica),
                (true, Material::Mica),
            ] {
                let compose = |scalar: bool| {
                    let (w, h) = (320 * scale, 240 * scale);
                    let mut e = Engine::new(w, h, theme, Limits::default()).unwrap();
                    e.set_scale(scale).unwrap();
                    e.set_wallpaper_material(&gradient(w, h)).unwrap();
                    e.scalar_reference = scalar;
                    let s = scale as i32;
                    let below = e
                        .create_window(
                            B,
                            Role::TopLevel,
                            Rect::new(100 * s, 90 * s, 200 * scale, 140 * scale).unwrap(),
                        )
                        .unwrap();
                    commit_with(&mut e, B, below, |x, y| {
                        [(x % 200) as u8, (y % 180) as u8, 200, 255]
                    });
                    let outer = Rect::new(-12 * s, 20 * s, 220 * scale, 160 * scale).unwrap();
                    let id = if tabbed_caption {
                        tabbed(&mut e, A, outer)
                    } else {
                        e.create_window(A, Role::TopLevel, outer).unwrap()
                    };
                    e.set_material(A, id, material).unwrap();
                    commit_with(&mut e, A, id, |x, y| match (y / 7) % 4 {
                        0 => [0, 0, 0, 0],
                        1 => [30, 40, 50, 255],
                        2 => [((x % 3) * 20) as u8, 20, 60, 90 + (x % 5) as u8 * 30],
                        _ if x % 2 == 0 => [5, 5, 5, 255],
                        _ => [9, 10, 11, 40],
                    });
                    e.activate(id).unwrap();
                    let full = frame(&mut e, false).0;
                    // Newly exposed tails after a configure keep the old snapshot.
                    e.configure(
                        A,
                        id,
                        Rect::new(-12 * s, 20 * s, 260 * scale, 190 * scale).unwrap(),
                    )
                    .unwrap();
                    let resized = frame(&mut e, true).0;
                    e.invalidate_region(Rect::new(40 * s, 60 * s, 30 * scale, 20 * scale).unwrap());
                    let partial = frame(&mut e, true).0;
                    (full, resized, partial)
                };
                assert!(
                    compose(false) == compose(true),
                    "scale={scale} theme={theme:?} tabbed={tabbed_caption} material={material:?}"
                );
            }
        }
    }
}

fn acrylic_scene() -> (Engine, WindowId, WindowId) {
    let mut e = Engine::new(640, 480, Theme::Light, Limits::default()).unwrap();
    let red = e
        .create_window(B, Role::TopLevel, Rect::new(20, 20, 300, 300).unwrap())
        .unwrap();
    commit_with(&mut e, B, red, |_, _| [0, 0, 255, 255]);
    // Left half opaque green, right half fully transparent material.
    let glass = e
        .create_window(A, Role::TopLevel, Rect::new(60, 60, 200, 160).unwrap())
        .unwrap();
    e.set_material(A, glass, Material::Acrylic).unwrap();
    commit_with(&mut e, A, glass, |x, _| {
        if x < 100 {
            [0, 255, 0, 255]
        } else {
            [0, 0, 0, 0]
        }
    });
    e.activate(glass).unwrap();
    (e, red, glass)
}

#[test]
fn acrylic_material_follows_the_scene_beneath_and_never_samples_itself() {
    let (mut e, _red, glass) = acrylic_scene();
    let (pixels, _) = frame(&mut e, false);
    let content = e.window_info(glass).unwrap().content;
    // Deep inside the red window: the backdrop is exactly red, and the
    // window's own opaque green half never bleeds into its material.
    let (x, y) = (content.x() + 150, content.y() + 60);
    assert_eq!(px(&pixels, 640, x, y), [0, 0, 255, 255]);
    let (x, y) = (content.x() + 104, content.y() + 60);
    let near = px(&pixels, 640, x, y);
    assert_eq!(near[1], 0, "no green from the window itself: {near:?}");
    // Move over the flat desktop: the same pixel now shows desktop gray.
    e.move_window(A, glass, 360, 280).unwrap();
    let (pixels, _) = frame(&mut e, true);
    let content = e.window_info(glass).unwrap().content;
    assert_eq!(
        px(&pixels, 640, content.x() + 150, content.y() + 60),
        [96, 96, 96, 255]
    );
    // Partly over the red window: a blurred mix that differs from both.
    e.move_window(A, glass, 230, 100).unwrap();
    let (pixels, _) = frame(&mut e, true);
    let content = e.window_info(glass).unwrap().content;
    let mix = px(&pixels, 640, 320 + 2, content.y() + 60);
    assert!(
        mix != [96, 96, 96, 255] && mix != [0, 0, 255, 255],
        "{mix:?}"
    );
    // Partial damage reaching the window recomposes it and its sampled
    // margin, so retained pixels (including its own) are never sampled.
    let full = frame(&mut e, false).0;
    e.invalidate_region(Rect::new(content.x() + 180, content.y() + 100, 4, 4).unwrap());
    let (partial, damage) = frame(&mut e, true);
    let reach = e
        .window_info(glass)
        .unwrap()
        .outer
        .expanded(acrylic::margin(Scale::ONE))
        .unwrap()
        .intersection(e.output_rect())
        .unwrap();
    assert_eq!(damage.and_then(|d| d.intersection(reach)), Some(reach));
    assert_eq!(partial, full);
}

#[test]
fn acrylic_captions_use_normal_damage_reduced_transparency_is_solid_and_never_culls() {
    let (mut e, red, glass) = acrylic_scene();
    frame(&mut e, false);
    let outer = e.window_info(glass).unwrap().outer;
    assert!(e.set_caption_pointer(outer.right() - 20, outer.y() + 16, false));
    assert!(!e.has_caption_damage());
    assert!(e.has_damage());
    let (hovered, _) = frame(&mut e, true);
    assert_eq!(
        px(&hovered, 640, outer.right() - 40, outer.y() + 4),
        [35, 35, 232, 255]
    );
    // Reduced transparency: the material is the opaque system fallback.
    e.set_reduce_transparency(true);
    let (pixels, _) = frame(&mut e, false);
    let content = e.window_info(glass).unwrap().content;
    assert_eq!(
        px(&pixels, 640, content.x() + 150, content.y() + 60),
        [243, 243, 243, 255]
    );
    e.set_reduce_transparency(false);
    // A maximized live acrylic window does not cull what shows through it.
    e.request_state(A, glass, WindowState::Maximized).unwrap();
    commit_with(&mut e, A, glass, |_, _| [0, 0, 0, 0]);
    let (pixels, _) = frame(&mut e, false);
    let red_outer = e.window_info(red).unwrap().outer;
    let inside = px(&pixels, 640, red_outer.x() + 150, red_outer.y() + 150);
    assert_eq!(inside, [0, 0, 255, 255]);
}

fn motion_engine() -> Engine {
    let mut e = Engine::new(640, 480, Theme::Light, Limits::default()).unwrap();
    e.set_animations(true);
    e.advance(1_000);
    e
}
fn settle(e: &mut Engine, now: u64) {
    e.advance(now);
    frame(e, true);
}

#[test]
fn open_waits_for_the_first_frame_then_scales_and_fades_in_from_retained_pixels() {
    let mut e = motion_engine();
    let mut reference = Engine::new(640, 480, Theme::Light, Limits::default()).unwrap();
    let r = Rect::new(100, 80, 300, 200).unwrap();
    let id = tabbed(&mut e, A, r);
    let still = tabbed(&mut reference, A, r);
    let (pixels, _) = frame(&mut e, false);
    assert_eq!(px(&pixels, 640, 250, 180), [96, 96, 96, 255], "invisible");
    // Input still reaches the window at its final geometry.
    assert_eq!(e.hit_test(250, 180).map(|h| h.0), Some(id));
    commit_with(&mut e, A, id, |_, _| [10, 20, 30, 255]);
    commit_with(&mut reference, A, still, |_, _| [10, 20, 30, 255]);
    assert!(e.animating());
    e.advance(1_000);
    let (start, _) = frame(&mut e, true);
    assert_eq!(px(&start, 640, 250, 180), [96, 96, 96, 255]);
    e.advance(1_080);
    let (middle, _) = frame(&mut e, true);
    let mid = px(&middle, 640, 250, 180);
    assert!(mid[0] < 96 && mid[0] > 10, "fading in: {mid:?}");
    // The scaled image starts inside the final outline: its corner shows
    // only (faded) shadow over the desktop, not window chrome.
    let corner = px(&middle, 640, r.x() + 2, r.y() + 2);
    assert!(corner[0] <= 96 && corner[0] >= 80, "{corner:?}");
    // No client repaint is requested while animating.
    assert!(drain(&mut e, A).is_empty());
    e.advance(1_200);
    assert!(!e.animating());
    let (done, _) = frame(&mut e, true);
    let (expected, _) = frame(&mut reference, false);
    assert!(
        done == expected,
        "settled frame equals the unanimated scene"
    );
    // Idle: advancing time queues nothing.
    e.advance(5_000);
    assert!(!e.has_damage());
}

#[test]
fn close_fades_a_ghost_without_input_and_releases_it_afterwards() {
    let mut e = motion_engine();
    let r = Rect::new(100, 80, 300, 200).unwrap();
    let id = tabbed(&mut e, A, r);
    commit_with(&mut e, A, id, |_, _| [10, 20, 30, 255]);
    settle(&mut e, 1_300);
    let (empty, _) = {
        let mut blank = Engine::new(640, 480, Theme::Light, Limits::default()).unwrap();
        frame(&mut blank, false)
    };
    e.close(A, id).unwrap();
    assert_eq!(e.hit_test(250, 180), None, "removed from input immediately");
    assert_eq!(e.captured_window(), None);
    assert_eq!(e.transition_resources().0, 1);
    e.advance(1_310);
    let (closing, _) = frame(&mut e, true);
    assert!(px(&closing, 640, 250, 180)[0] < 60, "ghost still visible");
    // The client destroys the window; only the bounded ghost remains.
    e.destroy_window(A, id).unwrap();
    assert_eq!(e.resource_counts().0, 0);
    assert_eq!(e.resource_counts().2, 0, "snapshot released to the owner");
    e.advance(1_380);
    let (late, _) = frame(&mut e, true);
    let late = px(&late, 640, 250, 180);
    assert!(late[0] > px(&closing, 640, 250, 180)[0], "fading: {late:?}");
    e.advance(1_460);
    assert!(!e.animating());
    assert_eq!(e.transition_resources(), (0, 0));
    let (gone, _) = frame(&mut e, true);
    assert!(gone == empty, "no ghost surface is left behind");
}

#[test]
fn rapid_open_close_never_leaks_ghosts_capture_or_snapshots() {
    let mut e = motion_engine();
    let mut now = 2_000;
    for i in 0..60 {
        let r = Rect::new(20 + (i % 5) * 30, 30, 240, 180).unwrap();
        let id = tabbed(&mut e, A, r);
        let buffer = (i % 3 != 0).then(|| commit_with(&mut e, A, id, |_, _| [1, 2, 3, 255]));
        if i % 4 == 0 {
            e.activate(id).unwrap();
            e.set_capture(A, id).unwrap();
        }
        now += 3;
        e.advance(now);
        if i % 2 == 0 {
            frame(&mut e, true);
        }
        e.close(A, id).unwrap();
        assert!(e.transition_resources().0 <= 6);
        e.destroy_window(A, id).unwrap();
        if let Some(buffer) = buffer {
            e.unregister_buffer(A, buffer).unwrap();
        }
        drain(&mut e, A);
    }
    assert_eq!(e.captured_window(), None);
    assert_eq!(e.resource_counts(), (0, 0, 0));
    e.advance(now + 1_000);
    frame(&mut e, true);
    assert!(!e.animating());
    assert_eq!(e.transition_resources(), (0, 0));
    e.advance(now + 2_000);
    assert!(!e.has_damage());
}

#[test]
fn minimize_flies_to_the_taskbar_anchor_and_restore_interrupts_from_its_pose() {
    let mut e = motion_engine();
    let r = Rect::new(100, 80, 300, 200).unwrap();
    let id = tabbed(&mut e, A, r);
    commit_with(&mut e, A, id, |_, _| [10, 20, 30, 255]);
    settle(&mut e, 1_300);
    let anchor = Rect::new(300, 440, 40, 40).unwrap();
    e.set_taskbar_anchor(id, Some(anchor));
    e.request_state(A, id, WindowState::Minimized).unwrap();
    assert_eq!(e.hit_test(250, 180), None);
    e.advance(1_300);
    let start = e.motion.ghosts[0].drawn.unwrap();
    e.advance(1_420);
    let middle = e.motion.ghosts[0].drawn.unwrap();
    let center = |r: Rect| (r.x() + r.width() as i32 / 2, r.y() + r.height() as i32 / 2);
    assert!(
        center(middle).1 > center(start).1,
        "moves toward the taskbar"
    );
    assert!(middle.width() < start.width(), "and shrinks");
    frame(&mut e, true);
    // Restore before the ghost finishes: the window returns from that pose.
    e.restore_window(A, id).unwrap();
    assert!(e.motion.ghosts.is_empty());
    assert_eq!(e.motion.live[0].drawn, Some(middle));
    e.advance(1_500);
    frame(&mut e, true);
    e.advance(1_700);
    assert!(!e.animating());
    assert_eq!(e.hit_test(250, 180).map(|h| h.0), Some(id));
}

#[test]
fn reduce_motion_completes_transitions_immediately() {
    let mut e = motion_engine();
    let id = tabbed(&mut e, A, Rect::new(100, 80, 300, 200).unwrap());
    commit_with(&mut e, A, id, |_, _| [10, 20, 30, 255]);
    e.advance(1_050);
    assert!(e.animating());
    e.set_animations(false);
    assert!(!e.animating());
    assert!(e.has_damage());
    let (pixels, _) = frame(&mut e, true);
    assert_eq!(px(&pixels, 640, 250, 180), [10, 20, 30, 255]);
    e.close(A, id).unwrap();
    assert_eq!(e.transition_resources(), (0, 0));
    let id = tabbed(&mut e, A, Rect::new(100, 80, 300, 200).unwrap());
    let (pixels, _) = frame(&mut e, true);
    // Without motion a new window draws its chrome before the first frame.
    assert_ne!(px(&pixels, 640, 250, 180), [96, 96, 96, 255]);
    assert!(e.window_info(id).unwrap().visible);
}

#[test]
fn split_damage_composes_distant_regions_separately_and_matches_a_full_frame() {
    let build = |split: bool| {
        let mut e = Engine::new(640, 480, Theme::Light, Limits::default()).unwrap();
        e.set_wallpaper_material(&gradient(640, 480)).unwrap();
        if split {
            e.set_damage_regions(MAX_DAMAGE_RECTS);
        }
        let a = tabbed(&mut e, A, Rect::new(10, 10, 260, 200).unwrap());
        commit_with(&mut e, A, a, |x, y| {
            [(x % 251) as u8, (y % 241) as u8, 9, 255]
        });
        let b = tabbed(&mut e, B, Rect::new(360, 250, 260, 200).unwrap());
        commit_with(&mut e, B, b, |x, _| [9, 9, (x % 120) as u8, 120]);
        frame(&mut e, false);
        e
    };
    let mut split = build(true);
    let mut single = build(false);
    // Two tiny, distant updates (a caret blink and a counter).
    let mut overlay_clips = Vec::new();
    for e in [&mut split, &mut single] {
        e.invalidate_region(Rect::new(60, 80, 4, 16).unwrap());
        e.invalidate_region(Rect::new(500, 400, 30, 12).unwrap());
    }
    let frame = split
        .render_damage_with_layers(
            |c| {
                c.fill_rect(Rect::new(0, 0, 640, 480).unwrap(), [96, 96, 96, 255])
                    .unwrap()
            },
            |_, _| {},
            |c| overlay_clips.push(c.clip_rect()),
        )
        .unwrap();
    let regions: Vec<_> = frame.regions.iter().flatten().copied().collect();
    let (pixels, damage, serial) = (frame.pixels.to_vec(), frame.damage, frame.serial);
    split.complete_transfer(serial, true, 0).unwrap();
    assert_eq!(regions.len(), 2, "{regions:?}");
    let area: u64 = regions
        .iter()
        .map(|r| u64::from(r.width()) * u64::from(r.height()))
        .sum();
    assert!(area < 4_000, "only the two small regions: {regions:?}");
    // The overlay runs exactly once, over the bounding box.
    assert_eq!(overlay_clips, [damage]);
    let (unsplit, single_damage) = frame_with(&mut single);
    assert!(single_damage.unwrap().width() > 400);
    assert!(pixels == unsplit, "split regions compose the same scene");
    // The engine invalidation path still reports one full region.
    split.invalidate();
    let frame = split.render().unwrap();
    assert_eq!(frame.regions[0], Some(split_output()));
    assert_eq!(frame.regions[1], None);
}
fn split_output() -> Rect {
    Rect::new(0, 0, 640, 480).unwrap()
}
fn frame_with(e: &mut Engine) -> (Vec<u8>, Option<Rect>) {
    frame(e, true)
}

fn leave_scene() -> (Engine, WindowId, WindowId) {
    let mut e = Engine::new(640, 480, Theme::Light, Limits::default()).unwrap();
    let a = e
        .create_window(A, Role::TopLevel, Rect::new(10, 10, 240, 200).unwrap())
        .unwrap();
    let b = e
        .create_window(B, Role::TopLevel, Rect::new(300, 10, 240, 200).unwrap())
        .unwrap();
    commit_with(&mut e, A, a, |_, _| [1, 2, 3, 255]);
    commit_with(&mut e, B, b, |_, _| [1, 2, 3, 255]);
    e.activate(b).unwrap();
    drain(&mut e, A);
    drain(&mut e, B);
    (e, a, b)
}
fn moves(events: &[Event]) -> Vec<(WindowId, i32, i32)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Input {
                window,
                input: Input::PointerMove { x, y },
                ..
            } => Some((*window, *x, *y)),
            _ => None,
        })
        .collect()
}
const LEAVE: (i32, i32) = (-1, -1);

#[test]
fn pointer_leave_reaches_the_previous_window_once_when_moving_to_another_window() {
    let (mut e, a, b) = leave_scene();
    let mv = |x, y| Input::PointerMove { x, y };
    e.route_input(mv(50, 100), 1).unwrap();
    e.route_input(mv(60, 100), 2).unwrap();
    assert_eq!(moves(&drain(&mut e, A)).len(), 1, "queued motion merges");
    assert_eq!(e.route_input(mv(350, 100), 3), Ok(Some(b)));
    let events = drain(&mut e, A);
    assert_eq!(
        events,
        [Event::Input {
            window: a,
            input: mv(LEAVE.0, LEAVE.1),
            timestamp: 3,
        }]
    );
    assert_eq!(moves(&drain(&mut e, B)).len(), 1);
    // Focus and activation are untouched.
    assert_eq!(e.focused_window(), Some(b));
    // Further motion inside B sends nothing more to A.
    e.route_input(mv(360, 110), 4).unwrap();
    assert!(drain(&mut e, A).is_empty());
    // Back to A: now B is left.
    e.route_input(mv(40, 40), 5).unwrap();
    assert_eq!(moves(&drain(&mut e, B)), [(b, LEAVE.0, LEAVE.1)]);
    // A queued in-window motion is superseded by the leave (one per window).
    e.route_input(mv(41, 40), 6).unwrap();
    e.route_input(mv(360, 110), 7).unwrap();
    assert_eq!(moves(&drain(&mut e, A)), [(a, LEAVE.0, LEAVE.1)]);
}

#[test]
fn pointer_leave_reaches_the_window_when_moving_to_the_desktop_or_shell() {
    let (mut e, a, _) = leave_scene();
    let mv = |x, y| Input::PointerMove { x, y };
    e.route_input(mv(50, 100), 1).unwrap();
    drain(&mut e, A);
    assert_eq!(e.route_input(mv(600, 400), 2), Ok(None));
    assert_eq!(moves(&drain(&mut e, A)), [(a, LEAVE.0, LEAVE.1)]);
    e.route_input(mv(610, 410), 3).unwrap();
    assert!(drain(&mut e, A).is_empty(), "sent once");
    // Trusted shell UI (taskbar, Start) takes the pointer: same leave.
    e.route_input(mv(50, 100), 4).unwrap();
    drain(&mut e, A);
    e.pointer_leave(5);
    assert_eq!(moves(&drain(&mut e, A)), [(a, LEAVE.0, LEAVE.1)]);
    e.pointer_leave(6);
    assert!(drain(&mut e, A).is_empty());
    // A wheel or button over a window also counts as entering it.
    e.route_input(
        Input::Wheel {
            x: 50,
            y: 100,
            delta: 1,
        },
        7,
    )
    .unwrap();
    drain(&mut e, A);
    e.route_input(mv(600, 400), 8).unwrap();
    assert_eq!(moves(&drain(&mut e, A)), [(a, LEAVE.0, LEAVE.1)]);
}

#[test]
fn captured_drags_receive_no_leave_until_capture_is_released() {
    let (mut e, a, b) = leave_scene();
    let mv = |x, y| Input::PointerMove { x, y };
    e.route_input(
        Input::PointerButton {
            button: 0,
            down: true,
            x: 50,
            y: 100,
        },
        1,
    )
    .unwrap();
    e.set_capture(A, a).unwrap();
    drain(&mut e, A);
    drain(&mut e, B);
    // Dragging over B and the desktop keeps real motion flowing to A.
    let content = e.window_info(a).unwrap().content;
    e.route_input(mv(350, 100), 2).unwrap();
    e.pointer_leave(3);
    e.route_input(mv(600, 400), 4).unwrap();
    assert_eq!(
        moves(&drain(&mut e, A)),
        [(a, 600 - content.x(), 400 - content.y())]
    );
    assert!(drain(&mut e, B).is_empty());
    // After release, the pointer is over the desktop: A is left once.
    e.release_capture(A, a).unwrap();
    e.route_input(mv(350, 100), 5).unwrap();
    assert_eq!(moves(&drain(&mut e, A)), [(a, LEAVE.0, LEAVE.1)]);
    assert_eq!(moves(&drain(&mut e, B)).len(), 1);
    let _ = b;
}

#[test]
fn closed_destroyed_and_saturated_windows_get_no_leave() {
    let mv = |x, y| Input::PointerMove { x, y };
    // Closing: removed from input immediately; no leave follows.
    let (mut e, a, _) = leave_scene();
    e.route_input(mv(50, 100), 1).unwrap();
    e.close(A, a).unwrap();
    drain(&mut e, A);
    e.route_input(mv(350, 100), 2).unwrap();
    assert!(moves(&drain(&mut e, A)).is_empty());
    // Destroyed: nothing is queued for a window that no longer exists.
    let (mut e, a, _) = leave_scene();
    e.route_input(mv(50, 100), 1).unwrap();
    e.destroy_window(A, a).unwrap();
    drain(&mut e, A);
    e.route_input(mv(350, 100), 2).unwrap();
    e.pointer_leave(3);
    assert!(drain(&mut e, A).is_empty());
    // Full queue holding A's motion: the leave supersedes it in place.
    let fill = |e: &mut Engine| {
        while e
            .route_input(
                Input::Key {
                    code: 30,
                    down: true,
                },
                2,
            )
            .is_ok()
        {}
    };
    let (mut e, a, _) = leave_scene();
    e.activate(a).unwrap();
    drain(&mut e, A);
    e.route_input(mv(50, 100), 1).unwrap();
    fill(&mut e);
    let queued = e.pending_events(A);
    e.route_input(mv(600, 400), 3).unwrap();
    assert_eq!(e.pending_events(A), queued);
    assert_eq!(moves(&drain(&mut e, A)), [(a, LEAVE.0, LEAVE.1)]);
    // Full queue without a motion to replace: the advisory leave is dropped
    // and counted; the queue never exceeds its bound.
    let (mut e, a, _) = leave_scene();
    e.activate(a).unwrap();
    e.route_input(mv(50, 100), 1).unwrap();
    drain(&mut e, A);
    fill(&mut e);
    let (queued, dropped) = (e.pending_events(A), e.dropped_notifications(A));
    e.route_input(mv(600, 400), 3).unwrap();
    assert_eq!(e.pending_events(A), queued);
    assert_eq!(e.dropped_notifications(A), dropped + 1);
    assert!(moves(&drain(&mut e, A)).is_empty());
}

#[test]
fn transition_activity_reports_kinds_and_start_until_the_final_pose() {
    let mut e = motion_engine();
    assert_eq!(e.transition_activity().kinds, 0);
    let r = Rect::new(100, 80, 300, 200).unwrap();
    let id = tabbed(&mut e, A, r);
    // Waiting for the first frame is not a transition yet.
    assert_eq!(e.transition_activity().kinds, 0);
    commit_with(&mut e, A, id, |_, _| [10, 20, 30, 255]);
    e.advance(1_050);
    let open = e.transition_activity();
    assert_eq!(
        (open.kinds, open.start, open.now),
        (TRANSITION_OPEN, 1_000, 1_050)
    );
    settle(&mut e, 1_200);
    assert_eq!(
        e.transition_activity(),
        TransitionActivity {
            kinds: 0,
            start: 0,
            now: 1_200
        }
    );
    e.set_taskbar_anchor(id, Some(Rect::new(300, 440, 40, 40).unwrap()));
    e.request_state(A, id, WindowState::Minimized).unwrap();
    e.advance(1_300);
    assert_eq!(e.transition_activity().kinds, TRANSITION_MINIMIZE);
    e.restore_window(A, id).unwrap();
    e.advance(1_350);
    let restore = e.transition_activity();
    assert_eq!((restore.kinds, restore.start), (TRANSITION_RESTORE, 1_300));
    settle(&mut e, 1_600);
    assert_eq!(e.transition_activity().kinds, 0);
    e.snap_window(A, id, Snap::Left, None).unwrap();
    e.advance(1_650);
    assert_eq!(e.transition_activity().kinds, TRANSITION_RELOCATE);
    settle(&mut e, 1_900);
    e.close(A, id).unwrap();
    e.advance(1_950);
    assert_eq!(e.transition_activity().kinds, TRANSITION_CLOSE);
    settle(&mut e, 2_200);
    assert_eq!(e.transition_activity().kinds, 0);
    // Reduce motion starts nothing.
    e.set_animations(false);
    let id = tabbed(&mut e, A, r);
    commit_with(&mut e, A, id, |_, _| [10, 20, 30, 255]);
    e.advance(2_300);
    assert_eq!(e.transition_activity().kinds, 0);
}

/// Mica: the wallpaper under the theme's luminosity tint behind transparent
/// client pixels and the trusted title bar of the active window; the solid
/// fallback while inactive or with transparency reduced.
#[test]
fn mica_tints_the_wallpaper_behind_client_and_title_and_falls_back_when_inactive() {
    let (w, h) = (640, 480);
    for (theme, tint) in [
        (Theme::Dark, [0x20, 0x20, 0x20, 255]),
        (Theme::Light, [0xf3, 0xf3, 0xf3, 255]),
    ] {
        // A neutral wallpaper: Mica is exactly the WinUI tint colour.
        let mut e = Engine::new(w, h, theme, Limits::default()).unwrap();
        let gray: Vec<u8> = (0..w * h).flat_map(|_| [128, 128, 128, 255]).collect();
        e.set_wallpaper_material(&gray).unwrap();
        let id = e
            .create_window(A, Role::TopLevel, Rect::new(100, 80, 300, 220).unwrap())
            .unwrap();
        e.set_material(A, id, Material::Mica).unwrap();
        let card = [13, 13, 13, 13];
        commit_with(&mut e, A, id, |x, _| if x < 150 { [0; 4] } else { card });
        e.activate(id).unwrap();
        let (pixels, _) = frame(&mut e, false);
        let content = e.window_info(id).unwrap().content;
        assert_eq!(px(&pixels, w, content.x() + 40, content.y() + 60), tint);
        // Title bar (outside the client) shows the same material.
        assert_eq!(px(&pixels, w, 160, 80 + 16), tint);
        // A translucent card composes over Mica.
        assert_eq!(
            px(&pixels, w, content.x() + 200, content.y() + 60),
            crate::render::blend_bgra(tint, card).unwrap()
        );
        // Inactive: the solid Mica fallback.
        let other = e
            .create_window(B, Role::TopLevel, Rect::new(420, 300, 200, 160).unwrap())
            .unwrap();
        e.activate(other).unwrap();
        let (pixels, _) = frame(&mut e, false);
        let fallback = e.transparency_fallback();
        assert_eq!(px(&pixels, w, content.x() + 40, content.y() + 60), fallback);
        assert_eq!(px(&pixels, w, 160, 80 + 16), fallback);
    }
    // A coloured wallpaper keeps its hue at the tint's luminosity (dark:
    // a purple wallpaper gives a dark purple, as Settings shows over the
    // owner's wallpaper, reports/artifacts/reference-26h2/settings-display.png).
    let mut e = Engine::new(w, h, Theme::Dark, Limits::default()).unwrap();
    let purple: Vec<u8> = (0..w * h).flat_map(|_| [167, 58, 142, 255]).collect();
    e.set_wallpaper_material(&purple).unwrap();
    let id = e
        .create_window(A, Role::TopLevel, Rect::new(100, 80, 300, 220).unwrap())
        .unwrap();
    e.set_material(A, id, Material::Mica).unwrap();
    commit_with(&mut e, A, id, |_, _| [0; 4]);
    e.activate(id).unwrap();
    let (pixels, _) = frame(&mut e, false);
    let [b, g, r, a] = px(&pixels, w, 250, 200);
    let luma = (19 * u32::from(b) + 183 * u32::from(g) + 54 * u32::from(r) + 128) >> 8;
    assert_eq!(a, 255);
    assert!((30..=34).contains(&luma), "{b} {g} {r}");
    assert!(r > g + 8 && b > g + 8, "purple kept: {b} {g} {r}");
}

#[test]
fn a_full_screen_acrylic_client_caption_window_renders() {
    let mut e = Engine::new(640, 480, Theme::Light, Limits::default()).unwrap();
    e.set_work_area(Rect::new(0, 0, 640, 432).unwrap()).unwrap();
    let layout = CaptionLayout {
        tabbed: true,
        height: 40,
        max_width: 400,
        side_margin: 138,
    };
    let w = e
        .create_client_caption(A, Rect::new(60, 60, 400, 300).unwrap(), layout)
        .unwrap();
    e.set_material(A, w, Material::Acrylic).unwrap();
    commit_with(&mut e, A, w, |_, _| [20, 20, 20, 200]);
    e.activate(w).unwrap();
    let _ = frame(&mut e, false);
    e.set_fullscreen(A, w, true).unwrap();
    let _ = frame(&mut e, true);
    commit_with(&mut e, A, w, |_, _| [20, 20, 20, 200]);
    let _ = frame(&mut e, true);
    e.set_fullscreen(A, w, false).unwrap();
    let _ = frame(&mut e, true);
}
