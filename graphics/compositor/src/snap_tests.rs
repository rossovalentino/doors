//! Snap layout zones: arbitrary work-area rectangles keep restore-on-drag.
use super::*;
use crate::engine::animation::{SNAP_MS, STRETCH_HOLD_MS};
use std::vec::Vec;

const A: Owner = Owner {
    pid: 21,
    session: 1,
    epoch: 1,
};
const B: Owner = Owner {
    pid: 22,
    session: 1,
    epoch: 1,
};

fn engine() -> Engine {
    let mut e = Engine::new(1280, 720, Theme::Light, Limits::default()).unwrap();
    e.set_work_area(Rect::new(0, 0, 1280, 672).unwrap())
        .unwrap();
    e
}
fn commit(e: &mut Engine, owner: Owner, id: WindowId) {
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
    e.commit(
        owner,
        id,
        (0..width * height).flat_map(|_| [9, 9, 9, 255]).collect(),
    )
    .unwrap();
    e.unregister_buffer(owner, buffer).unwrap();
}
fn configures(e: &mut Engine, owner: Owner) -> Vec<Rect> {
    let mut out = Vec::new();
    while let Some(event) = e.poll_event(owner) {
        if let Event::Configure { outer, .. } = event {
            out.push(outer);
        }
    }
    out
}
fn render(e: &mut Engine) -> Vec<u8> {
    let frame = e.render().unwrap();
    let (pixels, serial) = (frame.pixels.to_vec(), frame.serial);
    e.complete_transfer(serial, true, 0).unwrap();
    pixels
}
/// The 2/3 + 1/3 layout's left zone of a 1280x672 work area.
fn two_thirds() -> Rect {
    Rect::new(0, 0, 853, 672).unwrap()
}

#[test]
fn zones_place_the_window_mark_it_snapped_and_keep_the_pre_snap_geometry() {
    let mut e = engine();
    let normal = Rect::new(200, 120, 640, 420).unwrap();
    let id = e.create_window(A, Role::TopLevel, normal).unwrap();
    commit(&mut e, A, id);
    configures(&mut e, A);
    e.snap_window_rect(A, id, two_thirds(), None).unwrap();
    let info = e.window_info(id).unwrap();
    assert_eq!(
        (info.outer, info.state, info.snapped),
        (two_thirds(), WindowState::Normal, true)
    );
    assert_eq!(configures(&mut e, A), [two_thirds()]);
    // Moving between zones keeps the original normal geometry.
    let stacked = Rect::new(640, 336, 640, 336).unwrap();
    e.snap_window_rect(A, id, stacked, None).unwrap();
    e.unsnap_window(A, id).unwrap();
    assert_eq!(e.window_info(id).unwrap().outer, normal);
    assert!(!e.window_info(id).unwrap().snapped);
    // A maximized window keeps its pre-maximize geometry as restore.
    e.request_state(A, id, WindowState::Maximized).unwrap();
    e.snap_window_rect(A, id, stacked, None).unwrap();
    let info = e.window_info(id).unwrap();
    assert_eq!((info.state, info.outer), (WindowState::Normal, stacked));
    e.unsnap_window(A, id).unwrap();
    assert_eq!(e.window_info(id).unwrap().outer, normal);
    // A title drag that started from the restored size records it.
    let dragged = Rect::new(300, 50, 500, 300).unwrap();
    e.snap_window_rect(A, id, two_thirds(), Some(dragged))
        .unwrap();
    e.unsnap_window(A, id).unwrap();
    assert_eq!(e.window_info(id).unwrap().outer, dragged);
}

#[test]
fn dragging_a_zone_snapped_window_restores_its_size_under_the_pointer() {
    let mut e = engine();
    let normal = Rect::new(200, 120, 640, 420).unwrap();
    let id = e.create_window(A, Role::TopLevel, normal).unwrap();
    commit(&mut e, A, id);
    let zone = Rect::new(853, 0, 427, 672).unwrap();
    e.snap_window_rect(A, id, zone, None).unwrap();
    // Press at 3/4 across the zone's title, then drag down-left.
    let pressed = (zone.x() + 320, 16);
    let restored = e.restore_title_drag(A, id, pressed, (700, 300)).unwrap();
    assert_eq!((restored.width(), restored.height()), (640, 420));
    // The press keeps its fraction of the title under the pointer.
    assert_eq!(restored.x(), 700 - 320 * 640 / 427);
    assert_eq!(restored.y(), 300 - 16);
    let info = e.window_info(id).unwrap();
    assert_eq!((info.outer, info.snapped), (restored, false));
}

#[test]
fn zone_snaps_check_owner_role_state_and_geometry() {
    let mut e = engine();
    let id = e
        .create_window(A, Role::TopLevel, Rect::new(10, 10, 400, 300).unwrap())
        .unwrap();
    let before = e.window_info(id).unwrap().outer;
    assert_eq!(
        e.snap_window_rect(B, id, two_thirds(), None),
        Err(Error::WrongOwner)
    );
    // Zones never leave the work area (the taskbar band is excluded).
    assert_eq!(
        e.snap_window_rect(A, id, Rect::new(0, 400, 640, 300).unwrap(), None),
        Err(Error::InvalidGeometry)
    );
    // Too small for a window.
    assert_eq!(
        e.snap_window_rect(A, id, Rect::new(0, 0, 100, 672).unwrap(), None),
        Err(Error::InvalidGeometry)
    );
    let popup = e
        .create_window(
            A,
            Role::OwnedPopup(id),
            Rect::new(20, 20, 200, 120).unwrap(),
        )
        .unwrap();
    assert_eq!(
        e.snap_window_rect(A, popup, two_thirds(), None),
        Err(Error::InvalidState)
    );
    e.request_state(A, id, WindowState::Minimized).unwrap();
    assert_eq!(
        e.snap_window_rect(A, id, two_thirds(), None),
        Err(Error::InvalidState)
    );
    e.restore_window(A, id).unwrap();
    assert_eq!(e.window_info(id).unwrap().outer, before);
    e.close(A, id).unwrap();
    assert_eq!(
        e.snap_window_rect(A, id, two_thirds(), None),
        Err(Error::Closing)
    );
    assert!(!e.window_info(id).unwrap().snapped);
}

#[test]
fn zone_snap_glides_from_retained_pixels_unless_motion_is_reduced() {
    let mut e = engine();
    let mut reference = engine();
    let normal = Rect::new(200, 120, 640, 420).unwrap();
    let id = e.create_window(A, Role::TopLevel, normal).unwrap();
    let still = reference.create_window(A, Role::TopLevel, normal).unwrap();
    commit(&mut e, A, id);
    commit(&mut reference, A, still);
    e.set_animations(true);
    e.advance(1_000);
    e.snap_window_rect(A, id, two_thirds(), None).unwrap();
    reference
        .snap_window_rect(A, still, two_thirds(), None)
        .unwrap();
    assert!(e.animating());
    // Input already targets the zone while pixels move.
    assert_eq!(e.hit_test(100, 400).map(|h| h.0), Some(id));
    let start = e.motion.live[0].drawn.unwrap();
    assert!(
        start.x() > 100 && start.width() < 853,
        "starts at the old bounds"
    );
    e.advance(1_080);
    render(&mut e);
    let middle = e.motion.live[0].drawn.unwrap();
    assert!(middle.x() < start.x() && middle.width() > start.width());
    e.advance(1_300);
    assert!(!e.animating(), "the glide is over; nothing animates");
    // Until the client repaints at the zone size, its last frame is shown
    // stretched over the zone instead of exposing the background colour.
    let exposed = (700, 532);
    let held = render(&mut e);
    let at = |pixels: &[u8]| {
        let i = (exposed.1 * 1280 + exposed.0) * 4;
        <[u8; 4]>::try_from(&pixels[i..i + 4]).unwrap()
    };
    assert_eq!(at(&held), [9, 9, 9, 255]);
    assert_eq!(at(&render(&mut reference)), [255, 255, 255, 255]);
    e.advance(1_400);
    assert!(!e.has_damage(), "the held frame is static");
    // The repaint at the new size ends the hold.
    commit(&mut e, A, id);
    commit(&mut reference, A, still);
    e.advance(1_420);
    assert!(e.motion.live.is_empty());
    assert!(render(&mut e) == render(&mut reference));
    // A client that never repaints gets the plain composition back.
    e.advance(5_000);
    e.snap_window_rect(A, id, Rect::new(853, 0, 427, 672).unwrap(), None)
        .unwrap();
    e.advance(5_300);
    assert_eq!(e.motion.live.len(), 1);
    e.advance(5_000 + SNAP_MS + STRETCH_HOLD_MS + 10);
    assert!(e.motion.live.is_empty() && e.has_damage());
    // Reduced motion: zone snaps are immediate.
    e.set_animations(false);
    e.snap_window_rect(A, id, two_thirds(), None).unwrap();
    assert!(!e.animating());
}
