//! Host compositor timings. Ignored by default; run in release:
//! `cargo test -p door-graphics-core --release --lib bench -- --ignored --nocapture`.
//! Host numbers only rank hot paths; they do not establish guest frame times.
use super::*;
use std::{time::Instant, vec::Vec};

const OWNER: Owner = Owner {
    pid: 7,
    session: 1,
    epoch: 1,
};
const OTHER: Owner = Owner {
    pid: 8,
    session: 1,
    epoch: 1,
};

fn limits() -> Limits {
    Limits {
        windows_per_owner: 8,
        buffers_per_owner: 8,
        bytes_per_owner: 64 * 1024 * 1024,
        total_bytes: 128 * 1024 * 1024,
        events_per_owner: 4096,
        total_windows: 32,
        total_buffers: 32,
        total_owners: 16,
    }
}

fn wallpaper(width: u32, height: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&[
                (x * 255 / width) as u8,
                (y * 255 / height) as u8,
                ((x ^ y) & 255) as u8,
                255,
            ]);
        }
    }
    pixels
}

/// Commit a frame whose title band and left navigation pane are translucent
/// material tints (as native tabbed apps paint them) over opaque content.
fn commit_app(e: &mut Engine, owner: Owner, id: WindowId, translucent: bool) {
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
            let tint = translucent && (y < 48 || x < 240);
            pixels.extend_from_slice(if tint {
                &[18, 18, 18, 40]
            } else {
                &[250, 250, 250, 255]
            });
        }
    }
    e.commit(owner, id, pixels).unwrap();
}

fn tabbed(e: &mut Engine, owner: Owner, r: Rect) -> WindowId {
    e.create_client_caption(
        owner,
        r,
        CaptionLayout {
            tabbed: true,
            height: 40,
            side_margin: 0,
            max_width: 2048,
        },
    )
    .unwrap()
}

pub(crate) struct Scene {
    pub engine: Engine,
    pub wallpaper: Vec<u8>,
    pub translucent: WindowId,
    pub front: WindowId,
}

pub(crate) fn scene() -> Scene {
    let (width, height) = (1920, 1080);
    let mut e = Engine::new(width, height, Theme::Light, limits()).unwrap();
    let wallpaper = wallpaper(width, height);
    e.set_wallpaper_material(&wallpaper).unwrap();
    if std::env::var("BENCH_NO_SPLIT").is_err() {
        e.set_damage_regions(MAX_DAMAGE_RECTS);
    }
    let back = tabbed(&mut e, OTHER, Rect::new(60, 40, 900, 640).unwrap());
    commit_app(&mut e, OTHER, back, false);
    let translucent = tabbed(&mut e, OWNER, Rect::new(160, 120, 1200, 800).unwrap());
    commit_app(&mut e, OWNER, translucent, true);
    let front = tabbed(&mut e, OTHER, Rect::new(900, 300, 900, 600).unwrap());
    commit_app(&mut e, OTHER, front, false);
    e.activate(front).unwrap();
    Scene {
        engine: e,
        wallpaper,
        translucent,
        front,
    }
}

pub(crate) fn full(s: &mut Scene) {
    let wallpaper = &s.wallpaper;
    let frame = s
        .engine
        .render_with_background(
            |c| {
                let _ = c.restore_bgra(wallpaper);
            },
            |_, _| {},
        )
        .unwrap();
    let serial = frame.serial;
    s.engine.complete_transfer(serial, true, 0).unwrap();
    while s.engine.poll_event(OWNER).is_some() {}
    while s.engine.poll_event(OTHER).is_some() {}
}

pub(crate) fn partial(s: &mut Scene, reuse: bool) {
    let wallpaper = &s.wallpaper;
    let frame = s
        .engine
        .render_damage_reusing_moves(
            reuse,
            |c| {
                let _ = c.restore_bgra(wallpaper);
            },
            |_, _| {},
            |_| {},
        )
        .unwrap();
    let serial = frame.serial;
    if std::env::var("BENCH_PROBE").is_ok() {
        std::println!(
            "PROBE reused={} damage={:?} regions={:?}",
            frame.reused_pixels,
            frame.damage,
            frame.regions
        );
    }
    s.engine.complete_transfer(serial, true, 0).unwrap();
    while s.engine.poll_event(OWNER).is_some() {}
    while s.engine.poll_event(OTHER).is_some() {}
}

fn time(label: &str, iterations: u32, mut f: impl FnMut(u32)) -> f64 {
    f(0);
    let started = Instant::now();
    for i in 0..iterations {
        f(i + 1);
    }
    let ms = started.elapsed().as_secs_f64() * 1000.0 / f64::from(iterations);
    std::println!("BENCH {label}: {ms:.3} ms");
    ms
}

#[test]
#[ignore = "host timing; run explicitly in release"]
fn bench_compositor_paths() {
    let mut s = scene();
    time("full_frame_1080p_three_windows", 20, |_| full(&mut s));
    let translucent = s.translucent;
    // Raise the translucent window: its whole area becomes visible content.
    s.engine.activate(translucent).unwrap();
    full(&mut s);
    time("partial_translucent_region_400x300", 40, |_| {
        s.engine
            .invalidate_region(Rect::new(300, 400, 400, 300).unwrap());
        partial(&mut s, false);
    });
    time("partial_opaque_region_400x300", 40, |_| {
        s.engine
            .invalidate_region(Rect::new(800, 500, 400, 300).unwrap());
        partial(&mut s, false);
    });
    time("full_window_translucent_1200x800", 20, |_| {
        s.engine
            .invalidate_region(Rect::new(160, 120, 1200, 800).unwrap());
        partial(&mut s, false);
    });
    // Caption hover on the translucent window's close button and back.
    let outer = s.engine.window_info(translucent).unwrap().outer;
    time("caption_hover_toggle", 200, |i| {
        let (x, y) = if i % 2 == 0 {
            (outer.right() - 20, outer.y() + 16)
        } else {
            (outer.x() + 400, outer.y() + 400)
        };
        s.engine.set_caption_pointer(x, y, false);
        if s.engine.has_caption_damage() {
            let frame = s.engine.repaint_captions().unwrap();
            let serial = frame.serial;
            s.engine.complete_transfer(serial, true, 0).unwrap();
        } else if s.engine.has_damage() {
            partial(&mut s, false);
        }
    });
    let front = s.front;
    s.engine.activate(front).unwrap();
    full(&mut s);
    let start = s.engine.window_info(front).unwrap().outer;
    time("drag_move_front_window_3px", 60, |i| {
        let dx = (i % 40) as i32 * 3 - 60;
        s.engine
            .move_window(OTHER, front, start.x() + dx, start.y() + dx / 3)
            .unwrap();
        partial(&mut s, true);
    });
    // Two small disjoint updates far apart (caret blink + status change).
    time("two_disjoint_small_updates", 40, |_| {
        s.engine
            .invalidate_region(Rect::new(200, 160, 24, 24).unwrap());
        s.engine
            .invalidate_region(Rect::new(1700, 850, 24, 24).unwrap());
        partial(&mut s, false);
    });
}

#[test]
#[ignore = "host timing; run explicitly in release"]
fn bench_acrylic_and_transitions() {
    let mut s = scene();
    let translucent = s.translucent;
    s.engine.activate(translucent).unwrap();
    s.engine
        .set_material(OWNER, translucent, Material::Acrylic)
        .unwrap();
    full(&mut s);
    time("acrylic_window_recompose_1200x800", 20, |_| {
        s.engine
            .invalidate_region(Rect::new(400, 400, 8, 8).unwrap());
        partial(&mut s, false);
    });
    s.engine
        .set_material(OWNER, translucent, Material::Default)
        .unwrap();
    s.engine.set_animations(true);
    let mut now = 10_000;
    s.engine.advance(now);
    let id = tabbed(
        &mut s.engine,
        OWNER,
        Rect::new(300, 200, 1000, 640).unwrap(),
    );
    commit_app(&mut s.engine, OWNER, id, false);
    time("open_transition_frame_1000x640", 24, |_| {
        now += 8;
        s.engine.advance(now);
        partial(&mut s, false);
    });
    s.engine.advance(now + 1_000);
    partial(&mut s, false);
    s.engine.close(OWNER, id).unwrap();
    time("close_ghost_frame_1000x640", 18, |_| {
        now += 8;
        s.engine.advance(now);
        partial(&mut s, false);
    });
}

#[test]
#[ignore = "host timing; run explicitly in release"]
fn bench_layer_blit() {
    let bounds = Rect::new(100, 100, 1024, 664).unwrap();
    let layer = crate::engine::animation::Layer {
        bounds,
        width: 1024,
        height: 664,
        pixels: (0..1024 * 664)
            .flat_map(|i| [i as u8 / 2, 9, 9, 200])
            .collect(),
        key: None,
    };
    let mut pixels = std::vec![0u8; 1920 * 1080 * 4];
    let mut canvas = Canvas::new(&mut pixels, 1920, 1080, 1920 * 4).unwrap();
    let scaled = Rect::new(115, 110, 993, 644).unwrap();
    time("layer_blit_scaled_1024x664", 30, |_| {
        crate::render::draw_layer(&mut canvas, &layer, scaled, 180);
    });
    time("layer_blit_unscaled_1024x664", 30, |_| {
        crate::render::draw_layer(&mut canvas, &layer, bounds, 180);
    });
}

/// The owner's display (2560x1600 at 150 %) with three overlapping normal
/// windows: full frames and a drag of the front window, whose shadow band is
/// recomposed every frame. Run before and after shadow changes.
#[test]
#[ignore = "host timing; run explicitly in release"]
fn bench_window_shadows_150() {
    let (width, height) = (2560, 1600);
    let mut e = Engine::new(width, height, Theme::Dark, limits()).unwrap();
    e.set_scale_percent(Scale::from_percent(150).unwrap())
        .unwrap();
    let wallpaper = wallpaper(width, height);
    e.set_wallpaper_material(&wallpaper).unwrap();
    e.set_damage_regions(MAX_DAMAGE_RECTS);
    let back = tabbed(&mut e, OTHER, Rect::new(90, 60, 1350, 960).unwrap());
    commit_app(&mut e, OTHER, back, false);
    let middle = tabbed(&mut e, OWNER, Rect::new(500, 300, 1500, 1000).unwrap());
    commit_app(&mut e, OWNER, middle, false);
    let front = tabbed(&mut e, OTHER, Rect::new(1100, 450, 1200, 900).unwrap());
    commit_app(&mut e, OTHER, front, false);
    e.activate(front).unwrap();
    let mut s = Scene {
        engine: e,
        wallpaper,
        translucent: middle,
        front,
    };
    time("full_frame_1600p_150_three_windows", 10, |_| full(&mut s));
    let start = s.engine.window_info(front).unwrap().outer;
    time("drag_move_front_window_150_4px", 60, |i| {
        let dx = (i % 40) as i32 * 4 - 80;
        s.engine
            .move_window(OTHER, front, start.x() + dx, start.y() + dx / 3)
            .unwrap();
        partial(&mut s, true);
    });
    time("activate_toggle_shadow_150", 20, |i| {
        let id = if i % 2 == 0 { middle } else { front };
        s.engine.activate(id).unwrap();
        partial(&mut s, false);
    });
}
