//! Boxed labels are optically centred: the painted ink's horizontal centre
//! and the cap band's vertical centre sit within half a logical pixel of the
//! box centre, at 1x and 2x, through every shared label painter.
use door_graphics_text::{FontAtlas, TextStyle};
use door_workspace::fluent::{self as f, Rect as R};
use door_workspace::ui::View;

const W: u32 = 360;
const H: u32 = 120;
const BG: [u8; 4] = [255, 255, 255, 255];

pub struct Canvas {
    pub pixels: Vec<u8>,
    pub scale: u32,
}
impl Canvas {
    pub fn paint(scale: u32, draw: impl FnOnce(&mut View<'_>)) -> Self {
        let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
        let mut pixels = vec![0; (W * H * scale * scale * 4) as usize];
        {
            let mut v = View::new_scaled(&mut pixels, W * scale, H * scale, &fonts, scale);
            v.rect(0, 0, W, H, BG);
            draw(&mut v);
        }
        Self { pixels, scale }
    }
    fn at(&self, x: u32, y: u32) -> &[u8] {
        let i = ((y * W * self.scale + x) * 4) as usize;
        &self.pixels[i..i + 4]
    }
    /// Physical bounding box `[x0, x1) x [y0, y1)` of pixels inside the
    /// logical box that differ from `background`.
    pub fn ink(&self, r: R, background: [u8; 4]) -> (u32, u32, u32, u32) {
        let s = self.scale;
        let (mut x0, mut x1, mut y0, mut y1) = (u32::MAX, 0, u32::MAX, 0);
        for y in r.y as u32 * s..(r.y as u32 + r.h) * s {
            for x in r.x as u32 * s..(r.x as u32 + r.w) * s {
                if self.at(x, y) != background {
                    x0 = x0.min(x);
                    x1 = x1.max(x + 1);
                    y0 = y0.min(y);
                    y1 = y1.max(y + 1);
                }
            }
        }
        assert!(x0 < x1, "no ink in {r:?} at {s}x");
        (x0, x1, y0, y1)
    }
}
/// |ink centre - box centre| <= 0.5 logical px, compared in doubled
/// physical units so no rounding hides an error.
pub fn centred(lo: u32, hi: u32, start: i32, len: u32, scale: u32, what: &str) {
    let twice_ink = i64::from(lo + hi);
    let twice_box = (2 * i64::from(start) + i64::from(len)) * i64::from(scale);
    assert!(
        (twice_ink - twice_box).abs() <= i64::from(scale),
        "{what}: ink {lo}..{hi} vs box {start}+{len} at {scale}x"
    );
}

#[test]
fn text_in_box_centres_ink_horizontally_and_cap_height_vertically() {
    for scale in [1, 2] {
        for (style, h) in [
            (TextStyle::Regular12, 24),
            (TextStyle::Regular14, 32),
            (TextStyle::Semibold14, 36),
            (TextStyle::Regular16, 40),
            (TextStyle::Semibold20, 48),
        ] {
            for (x, w) in [(10, 120), (33, 61), (150, 97)] {
                let r = R::new(x, 20, w, h);
                for label in ["File", "Edit", "View", "OK", "Cancel", "Apply", "Hg"] {
                    let c = Canvas::paint(scale, |v| {
                        v.text_in_box((r.x, r.y, r.w, r.h), label, style, [24, 24, 24, 255])
                    });
                    let (x0, x1, _, _) = c.ink(r, BG);
                    centred(x0, x1, r.x, r.w, scale, label);
                }
                // The cap band: a caps-only label is the band itself.
                let c = Canvas::paint(scale, |v| {
                    v.text_in_box((r.x, r.y, r.w, r.h), "HE", style, [24, 24, 24, 255])
                });
                let (_, _, y0, y1) = c.ink(r, BG);
                centred(y0, y1, r.y, r.h, scale, "cap height");
            }
        }
    }
}

#[test]
fn rows_share_one_baseline_regardless_of_label_text() {
    // Vertical placement depends only on style and row, so a boxed label and
    // a left-aligned label in the same row share their caps exactly.
    for scale in [1, 2] {
        let c = Canvas::paint(scale, |v| {
            v.text_in_box((10, 20, 80, 32), "HH", TextStyle::Regular14, [0, 0, 0, 255]);
            v.text_clip_centered(
                200,
                (20, 32),
                80,
                "HH",
                TextStyle::Regular14,
                [0, 0, 0, 255],
            );
        });
        let a = c.ink(R::new(10, 20, 80, 32), BG);
        let b = c.ink(R::new(200, 20, 80, 32), BG);
        assert_eq!((a.2, a.3), (b.2, b.3));
    }
}

#[test]
fn fluent_buttons_and_labels_centre_their_text() {
    for scale in [1, 2] {
        for (r, label) in [
            (R::new(20, 30, 96, 32), "OK"),
            (R::new(20, 30, 120, 36), "Cancel"),
            (R::new(40, 40, 200, 38), "Export a copy"),
        ] {
            // Plain button: label alone, centred in the button.
            let c = Canvas::paint(scale, |v| f::button(v, r, label, None, false, true));
            let (x0, x1, _, _) = c.ink(r.inset(2), BG);
            centred(x0, x1, r.x, r.w, scale, label);
            // Accent button: white ink on accent fill.
            let c = Canvas::paint(scale, |v| f::button(v, r, label, None, true, true));
            let (x0, x1, _, _) = c.ink(r.inset(6), f::ACCENT);
            centred(x0, x1, r.x, r.w, scale, label);
            // centered_label14 (quick-action captions, empty states).
            let c = Canvas::paint(scale, |v| {
                f::centered_label14(v, r, label, [24, 24, 24, 255])
            });
            let (x0, x1, _, _) = c.ink(r, BG);
            centred(x0, x1, r.x, r.w, scale, label);
        }
        // Icon buttons centre the icon-plus-label group.
        let r = R::new(20, 30, 220, 38);
        let c = Canvas::paint(scale, |v| {
            f::button(v, r, "Export a copy", Some(f::Glyph::Save), false, true)
        });
        let (x0, x1, _, _) = c.ink(r.inset(2), BG);
        assert!(
            (i64::from(x0 + x1) - (2 * i64::from(r.x) + i64::from(r.w)) * i64::from(scale)).abs()
                <= 2 * i64::from(scale),
            "icon button group off centre at {scale}x"
        );
        // Caps of a button label are centred vertically.
        let c = Canvas::paint(scale, |v| f::button(v, r, "HH", None, false, true));
        let (_, _, y0, y1) = c.ink(r.inset(8), BG);
        centred(y0, y1, r.y, r.h, scale, "button caps");
    }
}

#[test]
fn dialog_buttons_centre_their_labels() {
    for scale in [1, 2] {
        let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
        let (w, h) = (742u32, 498u32);
        let mut pixels = vec![255; (w * h * scale * scale * 4) as usize];
        {
            let mut v = View::new_scaled(&mut pixels, w * scale, h * scale, &fonts, scale);
            let mut dialog = door_workspace::dialog::Dialog::default();
            dialog.show(2);
            v.dialog(&dialog, "Title", "Body", &["Save", "Cancel"]);
        }
        let layout = door_workspace::dialog::Layout::new(w, h, 2);
        for (i, label) in ["Save", "Cancel"].iter().enumerate() {
            let b = layout.buttons[i];
            let s = scale;
            let fill = {
                let x = (b.x as u32 + 4) * s;
                let y = (b.y as u32 + b.height / 2) * s;
                let at = ((y * w * s + x) * 4) as usize;
                [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
            };
            let (mut x0, mut x1) = (u32::MAX, 0);
            for y in (b.y as u32 + 6) * s..(b.y as u32 + b.height - 6) * s {
                for x in (b.x as u32 + 6) * s..(b.x as u32 + b.width - 6) * s {
                    let at = ((y * w * s + x) * 4) as usize;
                    if pixels[at..at + 4] != fill {
                        x0 = x0.min(x);
                        x1 = x1.max(x + 1);
                    }
                }
            }
            centred(x0, x1, b.x, b.width, scale, label);
        }
    }
}
