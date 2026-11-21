//! Fractional display scale: chrome geometry, hit testing, crisp edges,
//! composition at every supported percentage, and live rescale.
use super::*;
use std::vec::Vec;

const A: Owner = Owner {
    pid: 31,
    session: 1,
    epoch: 1,
};
const B: Owner = Owner {
    pid: 32,
    session: 1,
    epoch: 1,
};
const DESKTOP: [u8; 4] = [96, 96, 96, 255];

fn engine(width: u32, height: u32, scale: Scale) -> Engine {
    let limits = Limits {
        bytes_per_owner: 64 * 1024 * 1024,
        total_bytes: 128 * 1024 * 1024,
        ..Limits::default()
    };
    let mut e = Engine::new(width, height, Theme::Light, limits).unwrap();
    e.set_scale_percent(scale).unwrap();
    let taskbar = scale.len(48);
    e.set_work_area(Rect::new(0, 0, width, height - taskbar).unwrap())
        .unwrap();
    e
}
fn commit(e: &mut Engine, owner: Owner, id: WindowId, color: [u8; 4]) {
    let info = e.window_info(id).unwrap();
    let (epoch, width, height) = (info.epoch, info.content.width(), info.content.height());
    e.acknowledge_configure(owner, id, epoch).unwrap();
    let d = SurfaceDescriptor::new(
        width,
        height,
        width as usize * 4,
        width as usize * height as usize * 4,
    )
    .unwrap();
    let buffer = e.register_buffer(owner, d).unwrap();
    e.attach(owner, id, buffer, epoch).unwrap();
    e.commit(owner, id, (0..width * height).flat_map(|_| color).collect())
        .unwrap();
    e.unregister_buffer(owner, buffer).unwrap();
}
fn frame(e: &mut Engine) -> Vec<u8> {
    let output = e.output_rect();
    let frame = e
        .render_with_background(|c| c.fill_rect(output, DESKTOP).unwrap(), |_, _| {})
        .unwrap();
    let (pixels, serial) = (frame.pixels.to_vec(), frame.serial);
    e.complete_transfer(serial, true, 0).unwrap();
    while e.poll_event(A).is_some() {}
    while e.poll_event(B).is_some() {}
    pixels
}
fn px(pixels: &[u8], width: u32, x: i32, y: i32) -> [u8; 4] {
    let at = (y as usize * width as usize + x as usize) * 4;
    pixels[at..at + 4].try_into().unwrap()
}
fn all() -> impl Iterator<Item = Scale> {
    Scale::PERCENTS
        .into_iter()
        .map(|p| Scale::from_percent(p).unwrap())
}

#[test]
fn caption_buttons_and_hit_regions_share_snapped_edges_at_every_scale() {
    for s in all() {
        let mut e = engine(2560, 1600, s);
        let outer = s.rect(Rect::new(101, 77, 803, 517).unwrap()).unwrap();
        let id = e.create_window(A, Role::TopLevel, outer).unwrap();
        let window = e.window(id).unwrap();
        let rect = |region| Engine::caption_button_rect(window, region).unwrap();
        let (min, max, close) = (
            rect(HitRegion::Minimize),
            rect(HitRegion::Maximize),
            rect(HitRegion::Close),
        );
        // Abutting buttons: no seam, no overlap, flush with the right edge.
        assert_eq!((min.right(), max.right()), (max.x(), close.x()), "{s:?}");
        assert_eq!(close.right(), outer.right());
        assert_eq!(close.x(), outer.right() - s.len(46) as i32);
        for r in [min, max, close] {
            assert!(r.width().abs_diff(s.len(46)) <= 1);
            assert_eq!(r.height(), s.len(TITLE_HEIGHT));
        }
        // Every pixel of a painted button (inside the resize border) hits it.
        let border = s.len(4) as i32;
        for (region, r) in [
            (HitRegion::Minimize, min),
            (HitRegion::Maximize, max),
            (HitRegion::Close, close),
        ] {
            for y in r.y() + border..r.bottom() {
                for x in r.x()..r.right().min(outer.right() - border) {
                    assert_eq!(e.hit_test(x, y), Some((id, region)), "{s:?} {x},{y}");
                }
            }
        }
        // Resize border, title bar and client edges in physical pixels.
        let mid = outer.y() + s.len(TITLE_HEIGHT) as i32 / 2;
        assert_eq!(
            e.hit_test(outer.x() + border - 1, mid),
            Some((id, HitRegion::Resize))
        );
        assert_eq!(
            e.hit_test(outer.x() + border, mid),
            Some((id, HitRegion::TitleBar))
        );
        let content = e.window_info(id).unwrap().content;
        assert_eq!(content.y(), outer.y() + s.len(TITLE_HEIGHT) as i32);
        assert_eq!(
            e.hit_test(outer.x() + 50, content.y()),
            Some((id, HitRegion::Client))
        );
        assert_eq!(
            e.hit_test(outer.x() + 50, content.y() - 1),
            Some((id, HitRegion::TitleBar))
        );
        // The title text clip ends exactly where Minimize begins, less 12 DIPs.
        let clip = Engine::title_clip_scaled(outer, s).unwrap();
        assert_eq!(clip.right(), outer.right() - s.len(150) as i32);
        assert_eq!(clip.x(), outer.x() + s.len(12) as i32);
    }
}

#[test]
fn every_scale_composes_crisp_straight_edges_glyphs_and_maximized_coverage() {
    for s in all() {
        let mut e = engine(2560, 1600, s);
        let outer = s.rect(Rect::new(60, 40, 600, 400).unwrap()).unwrap();
        let id = e.create_window(A, Role::TopLevel, outer).unwrap();
        commit(&mut e, A, id, [20, 120, 220, 255]);
        e.activate(id).unwrap();
        let pixels = frame(&mut e);
        let row = outer.y() + outer.height() as i32 / 2;
        // A straight edge is one exact border pixel, then exact content:
        // logical edges land on whole physical pixels, never blended.
        let line = s.len(1) as i32;
        assert_eq!(
            px(&pixels, 2560, outer.x(), row),
            [180, 180, 180, 255],
            "{s:?}"
        );
        assert_eq!(
            px(&pixels, 2560, outer.x() + line, row),
            [20, 120, 220, 255],
            "{s:?}"
        );
        assert_eq!(
            px(&pixels, 2560, outer.right() - 1 - line, row),
            [20, 120, 220, 255]
        );
        // The title bar meets the content without a seam row.
        let content = e.window_info(id).unwrap().content;
        let col = outer.x() + outer.width() as i32 / 3;
        assert_eq!(px(&pixels, 2560, col, content.y()), [20, 120, 220, 255]);
        assert_ne!(px(&pixels, 2560, col, content.y() - 1), [20, 120, 220, 255]);
        // The Close glyph has ink at its centre column at every scale.
        let close = Engine::caption_button_rect(e.window(id).unwrap(), HitRegion::Close).unwrap();
        let center = (
            close.x() + close.width() as i32 / 2,
            close.y() + close.height() as i32 / 2,
        );
        let ink = (center.1 - s.len(6) as i32..center.1 + s.len(6) as i32)
            .any(|y| px(&pixels, 2560, center.0, y)[0] < 120);
        assert!(ink, "{s:?} close glyph");
        // A maximized window covers the whole work area: no desktop seam.
        e.request_state(A, id, WindowState::Maximized).unwrap();
        commit(&mut e, A, id, [20, 120, 220, 255]);
        let pixels = frame(&mut e);
        let area = e.work_area();
        for y in [area.y(), area.bottom() - 1] {
            for x in 0..2560 {
                assert_ne!(px(&pixels, 2560, x, y), DESKTOP, "{s:?} {x},{y}");
            }
        }
    }
}

#[test]
fn snapped_halves_and_tiled_zones_abut_without_a_gap_column() {
    for s in all() {
        let mut e = engine(2560, 1600, s);
        let left = e
            .create_window(
                A,
                Role::TopLevel,
                s.rect(Rect::new(10, 10, 400, 300).unwrap()).unwrap(),
            )
            .unwrap();
        let right = e
            .create_window(
                B,
                Role::TopLevel,
                s.rect(Rect::new(500, 10, 400, 300).unwrap()).unwrap(),
            )
            .unwrap();
        e.snap_window(A, left, Snap::Left, None).unwrap();
        e.snap_window(B, right, Snap::Right, None).unwrap();
        let (l, r) = (
            e.window_info(left).unwrap().outer,
            e.window_info(right).unwrap().outer,
        );
        assert_eq!(l.right(), r.x(), "{s:?}");
        assert_eq!(l.width() + r.width(), e.work_area().width());
    }
}

#[test]
fn live_rescale_keeps_logical_geometry_and_reconfigures_every_window() {
    let s200 = Scale::from_factor(2).unwrap();
    let s150 = Scale::from_percent(150).unwrap();
    let mut e = engine(2560, 1600, s200);
    let normal = e
        .create_window(
            A,
            Role::TopLevel,
            s200.rect(Rect::new(100, 100, 1000, 600).unwrap()).unwrap(),
        )
        .unwrap();
    commit(&mut e, A, normal, [1, 2, 3, 255]);
    let max = e
        .create_window(
            B,
            Role::TopLevel,
            s200.rect(Rect::new(40, 30, 500, 400).unwrap()).unwrap(),
        )
        .unwrap();
    e.request_state(B, max, WindowState::Maximized).unwrap();
    while e.poll_event(A).is_some() {}
    while e.poll_event(B).is_some() {}
    let epoch = e.window_info(normal).unwrap().epoch;
    let area = Rect::new(0, 0, 2560, 1600 - s150.len(48)).unwrap();
    e.rescale(s150, area).unwrap();
    assert_eq!(e.scale(), s150);
    assert_eq!(e.work_area(), area);
    let info = e.window_info(normal).unwrap();
    let (info_outer, info_epoch, info_title) =
        (info.outer, info.epoch, info.content.y() - info.outer.y());
    assert_eq!(info_outer, Rect::new(150, 150, 1500, 900).unwrap());
    assert_eq!(info_epoch, epoch + 1);
    assert_eq!(info_title, 48);
    let configured = std::iter::from_fn(|| e.poll_event(A)).find_map(|ev| match ev {
        Event::Configure { outer, .. } => Some(outer),
        _ => None,
    });
    assert_eq!(configured, Some(info_outer));
    assert_eq!(e.window_info(max).unwrap().outer, area);
    // Restoring the maximized window returns its pre-maximize logical rect.
    e.restore_window(B, max).unwrap();
    assert_eq!(
        e.window_info(max).unwrap().outer,
        s150.rect(Rect::new(40, 30, 500, 400).unwrap()).unwrap()
    );
    // Back to 200% restores the original physical geometry exactly.
    e.rescale(s200, Rect::new(0, 0, 2560, 1600 - 96).unwrap())
        .unwrap();
    assert_eq!(
        e.window_info(normal).unwrap().outer,
        Rect::new(200, 200, 2000, 1200).unwrap()
    );
    // A scene still composes after every change.
    frame(&mut e);
}

#[test]
fn rescale_is_atomic_when_a_queue_cannot_take_the_configure() {
    let s100 = Scale::ONE;
    let mut e = engine(1280, 720, s100);
    let id = e
        .create_window(A, Role::TopLevel, Rect::new(10, 10, 400, 300).unwrap())
        .unwrap();
    e.activate(id).unwrap();
    // The creation configure was read; only input fills the queue.
    while e.poll_event(A).is_some() {}
    while e
        .route_input(
            Input::Key {
                code: 30,
                down: true,
            },
            1,
        )
        .is_ok()
    {}
    let before = e.window_info(id).unwrap().outer;
    let s125 = Scale::from_percent(125).unwrap();
    assert_eq!(
        e.rescale(s125, Rect::new(0, 0, 1280, 720 - 60).unwrap()),
        Err(Error::QueueFull)
    );
    assert_eq!(e.scale(), s100);
    assert_eq!(e.window_info(id).unwrap().outer, before);
}

#[test]
fn fractional_cursors_are_area_filtered_with_scaled_hotspots() {
    for s in all() {
        let size = CursorIcon::size(s) as i32;
        assert_eq!(size as u32, s.len(32));
        for icon in [
            CursorIcon::Arrow,
            CursorIcon::Text,
            CursorIcon::NorthWestSouthEast,
        ] {
            let (hx, hy) = icon.hotspot_scaled(s);
            assert!((0..size).contains(&hx) && (0..size).contains(&hy));
            let mut ink = 0;
            for y in 0..size {
                for x in 0..size {
                    if let Some(p) = icon.pixel_scaled(x, y, s) {
                        assert!(p == [255, 255, 255, 0] || p[..3].iter().all(|c| *c <= p[3]));
                        ink += 1;
                    }
                }
            }
            assert!(ink > size, "{s:?} {icon:?}");
            assert_eq!(icon.pixel_scaled(size, 0, s), None);
        }
    }
    // The 150% arrow hotspot is the 2x hotspot scaled by 48/64.
    let (hx, hy) = CursorIcon::Arrow.hotspot_scaled(Scale::from_percent(200).unwrap());
    assert_eq!(
        CursorIcon::Arrow.hotspot_scaled(Scale::from_percent(150).unwrap()),
        ((hx * 48 + 32) / 64, (hy * 48 + 32) / 64)
    );
}

#[test]
fn normal_windows_resize_from_an_invisible_outer_band_and_corner_zones() {
    for s in all() {
        let mut e = engine(2560, 1600, s);
        let outer = s.rect(Rect::new(301, 277, 803, 517).unwrap()).unwrap();
        let id = e.create_window(A, Role::TopLevel, outer).unwrap();
        let band = s.len(CursorIcon::RESIZE_OUTSIDE) as i32;
        let mid = outer.y() + outer.height() as i32 / 2;
        // Just outside each side, up to the band; beyond it nothing.
        for (x, y) in [
            (outer.x() - 1, mid),
            (outer.x() - band, mid),
            (outer.right() + band - 1, mid),
            (outer.x() + 200, outer.y() - band),
            (outer.x() + 200, outer.bottom() + band - 1),
        ] {
            assert_eq!(
                e.hit_test(x, y),
                Some((id, HitRegion::Resize)),
                "{s:?} {x},{y}"
            );
        }
        assert_eq!(e.hit_test(outer.x() - band - 1, mid), None);
        assert_eq!(e.hit_test(outer.right() + band, mid), None);
        // One side alone mid-edge; both edges near a corner (inside or out).
        let edges = |x, y| CursorIcon::resize_edges_scaled(outer, x, y, s);
        assert_eq!(edges(outer.x() - 2, mid), 1);
        assert_eq!(edges(outer.right() + 2, mid), 2);
        assert_eq!(edges(outer.x() + 200, outer.y() - 2), 4);
        assert_eq!(edges(outer.x() + 200, outer.bottom() + 2), 8);
        let corner = s.len(CursorIcon::RESIZE_CORNER) as i32;
        assert_eq!(edges(outer.right() + 2, outer.bottom() - corner + 1), 2 | 8);
        assert_eq!(edges(outer.x() + corner - 1, outer.y() - 2), 1 | 4);
        assert_eq!(edges(outer.x() - 2, outer.y() + corner), 1);
        // Maximized windows have no band.
        e.request_state(A, id, WindowState::Maximized).unwrap();
        if e.window_info(id).unwrap().state == WindowState::Maximized {
            assert_ne!(
                e.hit_test(outer.x() - 1, mid),
                Some((id, HitRegion::Resize))
            );
        }
    }
}
