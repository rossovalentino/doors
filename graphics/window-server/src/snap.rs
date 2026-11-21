//! Windows 11 snap layouts: the flyout shown at the top centre of the screen
//! while a window is dragged there, while its Maximize button is hovered and
//! with Win+Z. Each pane maps to a real zone of the work area; choosing one
//! snaps the window, and the hovered pane shows the window's app icon.
use crate::paint::{self, Palette};
use door_graphics_core::{Canvas, Rect, Scale};
/// Zones in units of 1/6 of the work area: (x, y, width, height).
type Zone = (u8, u8, u8, u8);
/// Halves, two-thirds/one-third, half + stacked quarters, quadrants.
pub const LAYOUTS: [&[Zone]; 4] = [
    &[(0, 0, 3, 6), (3, 0, 3, 6)],
    &[(0, 0, 4, 6), (4, 0, 2, 6)],
    &[(0, 0, 3, 6), (3, 0, 3, 3), (3, 3, 3, 3)],
    &[(0, 0, 3, 3), (3, 0, 3, 3), (0, 3, 3, 3), (3, 3, 3, 3)],
];
const PANE_W: i32 = 98;
const PANE_H: i32 = 64;
const GAP: i32 = 4;
const SPACING: i32 = 13;
const MARGIN: i32 = 12;
pub const WIDTH: u32 = (2 * MARGIN + 4 * PANE_W + 3 * SPACING) as u32;
pub const HEIGHT: u32 = (2 * MARGIN + PANE_H) as u32;
/// Flyout rectangle centered horizontally at `cx`, top at `y`, kept inside
/// `bounds`.
pub fn flyout(bounds: Rect, cx: i32, y: i32) -> Rect {
    // Never panics on outputs narrower than the flyout: left/top win.
    let x = (cx - WIDTH as i32 / 2)
        .min(bounds.right() - WIDTH as i32)
        .max(bounds.x());
    let y = y.min(bounds.bottom() - HEIGHT as i32).max(bounds.y());
    Rect::new(x, y, WIDTH, HEIGHT).unwrap()
}
/// Slide distance (logical px, downwards) of the flyout's entrance.
pub const SLIDE: i32 = -16;
/// Hover slack around the flyout (logical px).
pub const SLACK: i32 = 24;
/// After the pointer leaves the Maximize button, the corridor and the
/// flyout, a hover-opened flyout stays this long (ticks of 10 ms) so a fast
/// or imprecise move to it does not close it.
pub const GRACE: u64 = 40;
/// The Maximize caption button of a window in logical pixels. Windows
/// caption buttons are 46 px wide and right-aligned (Minimize, Maximize,
/// Close); the engine's hit test uses the same layout. `outer` is physical;
/// `caption_height` is the window's custom caption height (0: standard).
pub fn maximize_button(outer: Rect, scale: Scale, caption_height: u32) -> Option<Rect> {
    let title = if caption_height == 0 {
        door_graphics_core::TITLE_HEIGHT
    } else {
        caption_height
    };
    // Buttons are laid out from the window's physical right edge.
    let left = scale.logical(outer.right() - scale.len(92) as i32);
    Rect::new(left, scale.logical(outer.y()), 46, title).ok()
}
fn cross(o: (i64, i64), a: (i64, i64), b: (i64, i64)) -> i64 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}
/// The pointer is inside the convex hull of `a` and `b` (edges included):
/// the corridor a pointer crosses travelling from one to the other.
pub fn in_corridor(a: Rect, b: Rect, x: i32, y: i32) -> bool {
    let mut p = [(0i64, 0i64); 8];
    for (i, r) in [a, b].iter().enumerate() {
        let (l, t, rr, bb) = (
            i64::from(r.x()),
            i64::from(r.y()),
            i64::from(r.right()),
            i64::from(r.bottom()),
        );
        p[i * 4..i * 4 + 4].copy_from_slice(&[(l, t), (rr, t), (l, bb), (rr, bb)]);
    }
    p.sort_unstable();
    // Andrew's monotone chain: hull[..k - 1] in counter-clockwise order.
    let mut hull = [(0i64, 0i64); 16];
    let mut k = 0;
    for &q in &p {
        while k >= 2 && cross(hull[k - 2], hull[k - 1], q) <= 0 {
            k -= 1;
        }
        hull[k] = q;
        k += 1;
    }
    let lower = k + 1;
    for &q in p.iter().rev().skip(1) {
        while k >= lower && cross(hull[k - 2], hull[k - 1], q) <= 0 {
            k -= 1;
        }
        hull[k] = q;
        k += 1;
    }
    let point = (i64::from(x), i64::from(y));
    k >= 3 && (0..k - 1).all(|i| cross(hull[i], hull[i + 1], point) >= 0)
}
/// Hover-opened flyout lifetime (Windows): it stays while the pointer is on
/// the Maximize button, on or near the flyout, in the corridor between them,
/// and for `GRACE` after leaving all of them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Linger(Option<u64>);
impl Linger {
    /// `button` and `flyout` are logical; the pointer (`x`, `y`) too.
    pub fn keep(&mut self, button: Rect, flyout: Rect, x: i32, y: i32, now: u64) -> bool {
        let slack = Rect::new(
            flyout.x() - SLACK,
            flyout.y() - SLACK,
            flyout.width() + 2 * SLACK as u32,
            flyout.height() + 2 * SLACK as u32,
        )
        .unwrap_or(flyout);
        if button.contains(x, y) || slack.contains(x, y) || in_corridor(button, flyout, x, y) {
            self.0 = None;
            return true;
        }
        let since = *self.0.get_or_insert(now);
        now.saturating_sub(since) < GRACE
    }
}
/// Where a primary press goes while a non-drag snap layouts flyout for
/// `window` is shown. `x`, `y` are physical.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlyoutPress {
    /// One of the window's own caption buttons: it acts (Maximize maximizes
    /// or restores) and the flyout closes.
    Caption,
    /// A layout pane: snap the window there.
    Zone(usize, usize),
    /// The flyout background.
    Inside,
    /// Anywhere else: the flyout closes and the press passes through.
    Outside,
}
pub fn flyout_press(
    engine: &door_graphics_core::Engine,
    window: door_graphics_core::WindowId,
    flyout: Rect,
    x: i32,
    y: i32,
) -> FlyoutPress {
    use door_graphics_core::HitRegion;
    if matches!(
        engine.hit_test(x, y),
        Some((id, HitRegion::Minimize | HitRegion::Maximize | HitRegion::Close)) if id == window
    ) {
        return FlyoutPress::Caption;
    }
    let s = engine.scale();
    let (lx, ly) = (s.logical(x), s.logical(y));
    if !flyout.contains(lx, ly) {
        return FlyoutPress::Outside;
    }
    hit(flyout, lx, ly).map_or(FlyoutPress::Inside, |(l, z)| FlyoutPress::Zone(l, z))
}
/// Whether an open Snap layouts flyout takes keyboard input. Only the
/// keyboard-opened flyout (Win+Z) does, as in Windows 11: one shown by
/// hovering Maximize (or a drag to the top edge) leaves every key, Alt+F4
/// and Start navigation included, to the focused window and the shell.
pub fn flyout_takes_keys(from_hover: bool, drag: bool) -> bool {
    !from_hover && !drag
}
/// Maximize-button hover (Windows: the flyout opens after hovering Maximize
/// for 500 ms; after a click on the button it stays closed until the pointer
/// leaves the button).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MaximizeHover(Option<(door_graphics_core::WindowId, u64)>);
impl MaximizeHover {
    /// Hover delay in monotonic ticks (10 ms).
    pub const DELAY: u64 = 50;
    /// The pointer is over `over`'s Maximize button (None: elsewhere) at
    /// tick `now`. True when the flyout should open.
    pub fn over(&mut self, over: Option<door_graphics_core::WindowId>, now: u64) -> bool {
        match (over, self.0) {
            (None, _) => {
                self.0 = None;
                false
            }
            (Some(id), Some((window, since))) if window == id => {
                since != u64::MAX && now.saturating_sub(since) >= Self::DELAY
            }
            (Some(id), _) => {
                self.0 = Some((id, now));
                false
            }
        }
    }
    /// A press on the button: no flyout until the pointer leaves it.
    pub fn pressed(&mut self, id: door_graphics_core::WindowId) {
        self.0 = Some((id, u64::MAX));
    }
    pub fn reset(&mut self) {
        self.0 = None;
    }
}
/// Start suspends Maximize hover until it has left the screen. Enter on a
/// running app in Start clears the launcher at once, yet Start still fades
/// out (`motion::CLOSE_US`). Counting the hover delay from the activation let
/// a window raised under the resting pointer open its Snap layouts flyout
/// before Start's overlay-free dismissal frame reached the screen (a 2560x1600
/// frame takes ~250 ms), and the flyout then replaced that frame for good.
/// `now_us` is shell time (`motion::Transition`).
pub fn start_suspends_hover(start: &crate::motion::Transition, now_us: u64) -> bool {
    start.visible(now_us) || start.animating(now_us)
}
/// The top-center snap bar while dragging (8 px below the top edge).
pub fn drag_flyout(area: Rect) -> Rect {
    flyout(area, area.x() + area.width() as i32 / 2, area.y() + 8)
}
/// Pointer region that reveals the drag flyout: the top 40 px over the
/// flyout's span plus a little slack, as Windows does.
pub fn reveals(area: Rect, x: i32, y: i32) -> bool {
    let f = drag_flyout(area);
    y < area.y() + 40 && x >= f.x() - 40 && x < f.right() + 40
}
fn layout_box(f: Rect, layout: usize) -> Rect {
    Rect::new(
        f.x() + MARGIN + layout as i32 * (PANE_W + SPACING),
        f.y() + MARGIN,
        PANE_W as u32,
        PANE_H as u32,
    )
    .unwrap()
}
/// Miniature pane for zone `zone` of `layout` inside the flyout `f`.
pub fn pane(f: Rect, layout: usize, zone: usize) -> Option<Rect> {
    let (zx, zy, zw, zh) = *LAYOUTS.get(layout)?.get(zone)?;
    let b = layout_box(f, layout);
    let unit_w = |u: u8| (PANE_W + GAP) * u as i32 / 6;
    let unit_h = |u: u8| (PANE_H + GAP) * u as i32 / 6;
    let x = b.x() + unit_w(zx);
    let y = b.y() + unit_h(zy);
    let w = unit_w(zx + zw) - unit_w(zx) - GAP;
    let h = unit_h(zy + zh) - unit_h(zy) - GAP;
    Rect::new(x, y, w as u32, h as u32).ok()
}
pub fn hit(f: Rect, x: i32, y: i32) -> Option<(usize, usize)> {
    (0..LAYOUTS.len()).find_map(|l| {
        (0..LAYOUTS[l].len())
            .find_map(|z| pane(f, l, z).filter(|r| r.contains(x, y)).map(|_| (l, z)))
    })
}
/// Physical work-area rectangle for a zone (exact partition: adjacent zones
/// share edges and together cover the whole area).
pub fn zone_rect(area: Rect, layout: usize, zone: usize) -> Option<Rect> {
    let (zx, zy, zw, zh) = *LAYOUTS.get(layout)?.get(zone)?;
    let w = area.width() as i64;
    let h = area.height() as i64;
    let x0 = area.x() + (w * zx as i64 / 6) as i32;
    let x1 = area.x() + (w * (zx + zw) as i64 / 6) as i32;
    let y0 = area.y() + (h * zy as i64 / 6) as i32;
    let y1 = area.y() + (h * (zy + zh) as i64 / 6) as i32;
    Rect::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32).ok()
}
/// The engine's built-in snap for zones that are halves or quarters; other
/// zones use `Engine::snap_window_rect` with `zone_rect`. Both keep the
/// pre-snap size for restore-on-drag.
pub fn builtin(layout: usize, zone: usize) -> Option<door_graphics_core::Snap> {
    use door_graphics_core::Snap;
    match LAYOUTS.get(layout)?.get(zone)? {
        (0, 0, 3, 6) => Some(Snap::Left),
        (3, 0, 3, 6) => Some(Snap::Right),
        (0, 0, 3, 3) => Some(Snap::TopLeft),
        (3, 0, 3, 3) => Some(Snap::TopRight),
        (0, 3, 3, 3) => Some(Snap::BottomLeft),
        (3, 3, 3, 3) => Some(Snap::BottomRight),
        _ => None,
    }
}
/// Paint the flyout at `f`; the hovered pane is accented and shows `icon`
/// (the snapping window's app) centred in it, as Windows does.
pub fn draw(
    c: &mut Canvas<'_>,
    f: Rect,
    hover: Option<(usize, usize)>,
    dark: bool,
    icon: Option<crate::apps::App>,
) {
    let palette = Palette::new(dark);
    paint::shadow(c, f, 8, dark);
    let (border, body, pane_fill, pane_border) = if palette.contrast {
        (
            palette.surface_border,
            palette.surface,
            palette.surface,
            palette.text,
        )
    } else if dark {
        (
            paint::rgb(58, 58, 58),
            paint::rgb(34, 34, 34),
            paint::rgb(70, 70, 70),
            paint::rgb(127, 127, 127),
        )
    } else {
        (
            paint::rgb(212, 212, 212),
            paint::rgb(243, 243, 243),
            paint::rgb(255, 255, 255),
            paint::rgb(160, 160, 160),
        )
    };
    paint::framed(c, f, 8, border, body);
    for (l, zones) in LAYOUTS.iter().enumerate() {
        for z in 0..zones.len() {
            let Some(r) = pane(f, l, z) else { continue };
            let hot = hover == Some((l, z));
            let fill = if hot { palette.accent } else { pane_fill };
            let edge = if hot { palette.accent } else { pane_border };
            paint::framed(c, r, 4, edge, fill);
            if hot && let Some(app) = icon {
                let want = if r.height() >= 40 && r.width() >= 40 {
                    24
                } else {
                    16
                };
                let size = crate::apps::icon_size(app, want, c.scale()) as i32;
                crate::apps::draw_icon(
                    c,
                    r.x() + (r.width() as i32 - size) / 2,
                    r.y() + (r.height() as i32 - size) / 2,
                    app,
                    size as u32,
                );
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_the_keyboard_flyout_takes_keys() {
        // Win+Z: digits pick a layout and Esc dismisses it.
        assert!(flyout_takes_keys(false, false));
        // Hovering Maximize (desktop-resolutions restores one window under
        // a maximized one's Restore button) or dragging to the top edge:
        // Alt+F4 and Start navigation must reach the focused window.
        assert!(!flyout_takes_keys(true, false));
        assert!(!flyout_takes_keys(false, true));
    }
    #[test]
    fn flyout_matches_reference_and_panes_hit_their_zone() {
        assert_eq!((WIDTH, HEIGHT), (455, 88));
        let area = Rect::new(0, 0, 1280, 672).unwrap();
        let f = drag_flyout(area);
        assert!((f.x() * 2 + WIDTH as i32 - 1280).abs() <= 1);
        for (l, zones) in LAYOUTS.iter().enumerate() {
            let b = layout_box(f, l);
            let mut covered = 0u64;
            for z in 0..zones.len() {
                let p = pane(f, l, z).unwrap();
                assert_eq!(b.intersection(p), Some(p));
                assert_eq!(hit(f, p.x() + 1, p.y() + 1), Some((l, z)));
                let r = zone_rect(area, l, z).unwrap();
                covered += r.width() as u64 * r.height() as u64;
                for other in 0..z {
                    assert!(zone_rect(area, l, other).unwrap().intersection(r).is_none());
                    assert!(pane(f, l, other).unwrap().intersection(p).is_none());
                }
            }
            assert_eq!(covered, 1280 * 672, "layout {l} partitions the work area");
        }
        assert_eq!(builtin(0, 0), Some(door_graphics_core::Snap::Left));
        assert_eq!(builtin(1, 0), None);
        assert!(reveals(area, 640, 5));
        assert!(!reveals(area, 640, 200));
        assert!(!reveals(area, 10, 5));
    }
    /// Every trigger shows the same top-centred flyout; a press on the
    /// window's own caption buttons always reaches them (also where the
    /// flyout overlaps them), at 1x and 3x, for windows at the initial
    /// placement, at the right edge, at the top centre and near the bottom.
    #[test]
    fn hover_flyout_is_top_centred_and_never_takes_a_caption_press() {
        use door_graphics_core::{Engine, HitRegion, Limits, Owner, Role, Theme};
        for scale in [1u32, 3] {
            let s = scale as i32;
            let mut engine =
                Engine::new(1280 * scale, 720 * scale, Theme::Light, Limits::default()).unwrap();
            engine.set_scale(scale).unwrap();
            let area = Rect::new(0, 0, 1280, 672).unwrap();
            engine
                .set_work_area(Rect::new(0, 0, 1280 * scale, 672 * scale).unwrap())
                .unwrap();
            let f = drag_flyout(area);
            assert_eq!(
                (f.y(), (f.x() * 2 + WIDTH as i32 - 1280).abs() <= 1),
                (8, true)
            );
            for (pid, (x, y, w, h)) in [
                (190, 24, 900, 624), // Notes/Settings initial placement
                (640, 40, 640, 480), // right edge of the output
                (440, 0, 450, 300),  // Maximize under the top-centred flyout
                (300, 600, 640, 400),
            ]
            .into_iter()
            .enumerate()
            {
                let owner = Owner {
                    pid: pid as u64 + 1,
                    session: 1,
                    epoch: 1,
                };
                let outer = Rect::new(x * s, y * s, w * scale, h * scale).unwrap();
                let id = engine.create_window(owner, Role::TopLevel, outer).unwrap();
                engine.activate(id).unwrap();
                let info = engine.window_info(id).unwrap();
                let button = maximize_button(
                    info.outer,
                    door_graphics_core::Scale::from_factor(scale).unwrap(),
                    info.caption_height,
                )
                .unwrap();
                let (bx, by) = (
                    button.x() + button.width() as i32 / 2,
                    button.y() + button.height() as i32 / 2,
                );
                assert_eq!(
                    engine.hit_test(bx * s, by * s),
                    Some((id, HitRegion::Maximize)),
                    "{scale}x {x},{y}"
                );
                // Every physical point of the Maximize button routes to it,
                // overlapped by the flyout or not.
                for py in (button.y() * s)..(button.bottom() * s) {
                    for px in (button.x() * s)..(button.right() * s) {
                        if engine.hit_test(px, py) == Some((id, HitRegion::Maximize)) {
                            assert_eq!(flyout_press(&engine, id, f, px, py), FlyoutPress::Caption);
                        }
                    }
                }
                if x == 440 {
                    assert!(f.intersection(button).is_some(), "this case overlaps");
                }
                // Panes and background of the flyout away from the button.
                let p = pane(f, 3, 2).unwrap();
                if !button.contains(p.x() + 2, p.y() + 2) {
                    assert_eq!(
                        flyout_press(&engine, id, f, (p.x() + 2) * s, (p.y() + 2) * s),
                        FlyoutPress::Zone(3, 2)
                    );
                }
                assert_eq!(
                    flyout_press(&engine, id, f, (f.x() + 2) * s, (f.bottom() - 3) * s),
                    FlyoutPress::Inside
                );
                assert_eq!(
                    flyout_press(&engine, id, f, (f.x() - 30) * s, (f.bottom() + 40) * s),
                    FlyoutPress::Outside
                );
                engine.destroy_window(owner, id).unwrap();
            }
        }
        // Hover: 500 ms opens; a click on the button keeps it closed until
        // the pointer leaves; another window restarts the delay.
        let (a, b) = (
            door_graphics_core::WindowId(1),
            door_graphics_core::WindowId(2),
        );
        let mut hover = MaximizeHover::default();
        assert!(!hover.over(Some(a), 100));
        assert!(!hover.over(Some(a), 149));
        assert!(hover.over(Some(a), 150));
        hover.pressed(a);
        assert!(!hover.over(Some(a), 10_000));
        assert!(!hover.over(None, 10_001));
        assert!(!hover.over(Some(a), 10_002));
        assert!(hover.over(Some(a), 10_052));
        assert!(!hover.over(Some(b), 10_053));
        assert!(hover.over(Some(b), 10_103));
    }
    /// Enter on a running app in Start raises its window with Maximize under
    /// the resting pointer while Start fades out (desktop-resolutions). One
    /// shell iteration per frame, slow ones included (~250 ms at 2560x1600):
    /// Start's overlay-free dismissal frame reaches the screen before the
    /// hover flyout opens.
    #[test]
    fn start_dismissal_presents_overlay_free_before_the_hover_flyout() {
        use crate::motion::{OPEN_US, Transition};
        let window = door_graphics_core::WindowId(2);
        for frame_us in [16_667, 250_000, 500_000, 700_000] {
            let mut start = Transition::default();
            start.set(true, 0, false);
            let activated = OPEN_US + 40_000;
            start.set(false, activated, false);
            let mut hover = MaximizeHover::default();
            let (mut clean, mut opened) = (false, false);
            let mut now = activated;
            while now < activated + 5_000_000 && !opened {
                // The activation already cleared the launcher.
                opened = if start_suspends_hover(&start, now) {
                    hover.reset();
                    false
                } else {
                    hover.over(Some(window), now / 10_000)
                };
                assert!(
                    !opened || clean,
                    "flyout replaced Start's dismissal ({frame_us} µs frames)"
                );
                // This iteration's frame: overlay-free once Start is gone.
                clean |= !opened && !start.visible(now);
                now += frame_us;
            }
            assert!(
                clean && opened,
                "dismissal, then the flyout ({frame_us} µs)"
            );
        }
    }
    /// The pointer travels from a Maximize button to the top-centred flyout
    /// without it closing: a straight path stays in the corridor; a sloppy
    /// detour is covered by the grace period; leaving for good closes it.
    #[test]
    fn hover_flyout_survives_the_trip_from_the_button() {
        let area = Rect::new(0, 0, 1280, 672).unwrap();
        let f = drag_flyout(area);
        for button in [
            Rect::new(998, 24, 46, 32).unwrap(),  // initial placement
            Rect::new(1188, 0, 46, 32).unwrap(),  // maximized window
            Rect::new(140, 300, 46, 32).unwrap(), // left, lower window
        ] {
            let mut linger = Linger::default();
            let (sx, sy) = (button.x() + 23, button.y() + 16);
            let (tx, ty) = (f.x() + f.width() as i32 / 2, f.y() + f.height() as i32 / 2);
            for step in 0..=40 {
                let x = sx + (tx - sx) * step / 40;
                let y = sy + (ty - sy) * step / 40;
                assert!(in_corridor(button, f, x, y), "{button:?} step {step}");
                assert!(linger.keep(button, f, x, y, 1000 + step as u64));
            }
        }
        let button = Rect::new(998, 24, 46, 32).unwrap();
        let mut linger = Linger::default();
        assert!(!in_corridor(button, f, 900, 400));
        assert!(linger.keep(button, f, 900, 400, 100), "grace starts");
        assert!(linger.keep(button, f, 900, 400, 139));
        assert!(
            linger.keep(button, f, 700, 60, 140),
            "back in the corridor resets it"
        );
        assert!(linger.keep(button, f, 900, 400, 200));
        assert!(!linger.keep(button, f, 900, 400, 240), "left for good");
        // Corridor edges and the far side of the flyout.
        assert!(in_corridor(button, f, f.x(), f.y()));
        assert!(!in_corridor(button, f, f.x() - 1, f.bottom() + 1));
    }
}
