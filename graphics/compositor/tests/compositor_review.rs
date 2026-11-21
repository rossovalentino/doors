//! Host-rendered compositor captures (live acrylic, reduced transparency and
//! window transitions) written as PPM files to `target/compositor-review/`.
//! They review appearance only; host captures do not establish guest timing.
use door_graphics_core::{
    CaptionLayout, Engine, Limits, Material, Owner, Rect, SurfaceDescriptor, Theme, WindowId,
};

const APP: Owner = Owner {
    pid: 3,
    session: 1,
    epoch: 1,
};
const TERMINAL: Owner = Owner {
    pid: 4,
    session: 1,
    epoch: 1,
};
const W: u32 = 1024;
const H: u32 = 768;

fn wallpaper() -> Vec<u8> {
    let bgr = include_bytes!("../../../assets/wallpapers/windows/bloom-1024x768.bgr");
    bgr.chunks_exact(3)
        .flat_map(|p| [p[0], p[1], p[2], 255])
        .collect()
}
fn save(name: &str, pixels: &[u8]) {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/compositor-review");
    std::fs::create_dir_all(&dir).unwrap();
    let mut ppm = format!("P6\n{W} {H}\n255\n").into_bytes();
    for p in pixels.chunks_exact(4) {
        ppm.extend_from_slice(&[p[2], p[1], p[0]]);
    }
    std::fs::write(dir.join(name), ppm).unwrap();
}
fn window(e: &mut Engine, owner: Owner, r: Rect, pixel: impl Fn(u32, u32) -> [u8; 4]) -> WindowId {
    let id = e
        .create_client_caption(
            owner,
            r,
            CaptionLayout {
                tabbed: true,
                height: 40,
                side_margin: 0,
                max_width: 2048,
            },
        )
        .unwrap();
    let info = e.window_info(id).unwrap();
    let (epoch, width, height) = (info.epoch, info.content.width(), info.content.height());
    e.acknowledge_configure(owner, id, epoch).unwrap();
    let d = SurfaceDescriptor::new(
        width,
        height,
        width as usize * 4,
        (width * height * 4) as usize,
    )
    .unwrap();
    let buffer = e.register_buffer(owner, d).unwrap();
    e.attach(owner, id, buffer, epoch).unwrap();
    let pixels = (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .flat_map(|(x, y)| pixel(x, y))
        .collect();
    e.commit(owner, id, pixels).unwrap();
    id
}
fn render(e: &mut Engine, wallpaper: &[u8]) -> Vec<u8> {
    let frame = e
        .render_with_background(
            |c| {
                c.restore_bgra(wallpaper).unwrap();
            },
            |_, _| {},
        )
        .unwrap();
    let (pixels, serial) = (frame.pixels.to_vec(), frame.serial);
    e.complete_transfer(serial, true, 0).unwrap();
    while e.poll_event(APP).is_some() {}
    while e.poll_event(TERMINAL).is_some() {}
    pixels
}
/// Explorer-like opaque window: light title band, white content with rows.
fn explorer(x: u32, y: u32) -> [u8; 4] {
    if y < 40 {
        [243, 243, 243, 255]
    } else if x < 200 {
        [238, 238, 238, 255]
    } else if (y / 28).is_multiple_of(2) {
        [255, 255, 255, 255]
    } else {
        [250, 246, 242, 255]
    }
}
/// Terminal-like acrylic content: dark tint at ~80% over the material, with
/// a few opaque "text" bars.
fn terminal(x: u32, y: u32) -> [u8; 4] {
    if y > 60 && (y - 60) % 22 < 12 && x > 16 && x < 16 + (y * 37 % 300) {
        [204, 204, 204, 255]
    } else {
        [10, 10, 10, 205]
    }
}

#[test]
fn render_compositor_review_captures() {
    let wallpaper = wallpaper();
    let mut e = Engine::new(W, H, Theme::Dark, Limits::default()).unwrap();
    e.set_wallpaper_material(&wallpaper).unwrap();
    window(&mut e, APP, Rect::new(60, 60, 560, 420).unwrap(), explorer);
    let term = window(
        &mut e,
        TERMINAL,
        Rect::new(380, 240, 560, 380).unwrap(),
        terminal,
    );
    e.set_material(TERMINAL, term, Material::Acrylic).unwrap();
    e.activate(term).unwrap();
    let acrylic = render(&mut e, &wallpaper);
    save("acrylic-terminal.ppm", &acrylic);
    // The same window with the wallpaper material: no windows behind show.
    e.set_material(TERMINAL, term, Material::Default).unwrap();
    save(
        "wallpaper-material-terminal.ppm",
        &render(&mut e, &wallpaper),
    );
    e.set_material(TERMINAL, term, Material::Acrylic).unwrap();
    e.set_reduce_transparency(true);
    save(
        "reduced-transparency-terminal.ppm",
        &render(&mut e, &wallpaper),
    );
    e.set_reduce_transparency(false);
    // Transitions: open at 30% progress, minimize halfway to its button.
    e.set_animations(true);
    e.advance(1_000);
    let opened = window(
        &mut e,
        APP,
        Rect::new(200, 120, 600, 440).unwrap(),
        explorer,
    );
    e.advance(1_045);
    save("open-transition.ppm", &render(&mut e, &wallpaper));
    e.advance(1_400);
    render(&mut e, &wallpaper);
    e.set_taskbar_anchor(opened, Some(Rect::new(480, 724, 40, 40).unwrap()));
    e.request_state(APP, opened, door_graphics_core::WindowState::Minimized)
        .unwrap();
    e.advance(1_520);
    save("minimize-transition.ppm", &render(&mut e, &wallpaper));
    e.advance(2_000);
    let settled = render(&mut e, &wallpaper);
    assert_eq!(e.transition_resources(), (0, 0));
    assert_eq!(acrylic.len(), settled.len());
}
