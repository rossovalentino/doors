//! Windows 11 Snap Assist: after a window snaps into a zone of a snap
//! layout, the next empty zone of that layout offers thumbnails of the other
//! open windows. Picking one snaps it there and moves on to the next empty
//! zone; Esc or a click elsewhere dismisses it. Thumbnails are scaled copies
//! of the engine's captured snapshots (as in Task View): no producer memory
//! is borrowed. All geometry is logical work-area pixels.
extern crate alloc;
use crate::apps::App;
use crate::motion::{self, Transition};
use crate::paint::{self, Palette};
use alloc::{string::String, vec::Vec};
use door_graphics_core::{Canvas, Engine, Owner, Rect, WindowId, WindowState};
use door_graphics_text::{FontAtlas, TextStyle};

/// At most this many windows are offered (thumbnail memory stays bounded).
pub const MAX_CANDIDATES: usize = 8;
const PAD: i32 = 24;
const GAP: i32 = 16;
/// Title row above each thumbnail.
const TITLE: i32 = 32;
const MAX_THUMB: i32 = 288;
const MIN_THUMB: i32 = 96;
/// Entrance slide distance (logical px); none with reduced motion.
const SLIDE: i32 = 24;

pub struct Candidate {
    pub id: WindowId,
    pub owner: Owner,
    pub title: String,
    pub app: Option<App>,
    pixels: Vec<u8>,
    revision: Option<u64>,
    size: (u32, u32),
}
impl Candidate {
    pub fn new(id: WindowId, owner: Owner, title: &str, app: Option<App>) -> Self {
        Self {
            id,
            owner,
            title: title.into(),
            app,
            pixels: Vec::new(),
            revision: None,
            size: (0, 0),
        }
    }
}
/// What a key did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Moved,
    Pick(usize),
    Dismiss,
    /// Not a Snap Assist key.
    Ignored,
}
pub struct Assist {
    /// Snap layout (`snap::LAYOUTS` index).
    pub layout: usize,
    /// Empty zones still to fill (zone index, logical rect); the first one
    /// shows the chooser.
    pub zones: Vec<(usize, Rect)>,
    pub candidates: Vec<Candidate>,
    /// Keyboard selection (None until an arrow key is used).
    pub selected: Option<usize>,
    pub motion: Transition,
}
impl Assist {
    /// Snap Assist after `filled` of `layout` was snapped, or None when no
    /// zone is left or no other window is open.
    pub fn new(
        layout: usize,
        filled: usize,
        area: Rect,
        candidates: Vec<Candidate>,
        now: u64,
        reduce_motion: bool,
    ) -> Option<Self> {
        let zones: Vec<_> = (0..crate::snap::LAYOUTS.get(layout)?.len())
            .filter(|z| *z != filled)
            .filter_map(|z| crate::snap::zone_rect(area, layout, z).map(|r| (z, r)))
            .collect();
        let mut candidates = candidates;
        candidates.truncate(MAX_CANDIDATES);
        if zones.is_empty() || candidates.is_empty() {
            return None;
        }
        let mut motion = Transition::default();
        motion.set(true, now, reduce_motion);
        Some(Self {
            layout,
            zones,
            candidates,
            selected: None,
            motion,
        })
    }
    /// The zone offering windows now.
    pub fn zone(&self) -> Option<(usize, Rect)> {
        self.zones.first().copied()
    }
    /// Thumbnail size (logical) and columns for the current zone: the
    /// largest 16:10 thumbnails whose grid fits inside the zone.
    fn grid(&self) -> Option<(i32, i32, usize)> {
        let (_, zone) = self.zone()?;
        let n = self.candidates.len().max(1);
        let (w, h) = (
            zone.width() as i32 - 2 * PAD,
            zone.height() as i32 - 2 * PAD,
        );
        let mut width = MAX_THUMB;
        loop {
            let height = width * 10 / 16;
            let columns = ((w + GAP) / (width + GAP)).max(1) as usize;
            let rows = n.div_ceil(columns) as i32;
            if rows * (height + TITLE + GAP) - GAP <= h || width <= MIN_THUMB {
                return Some((width, height, columns.min(n)));
            }
            width -= 8;
        }
    }
    /// Card rectangles (title row and thumbnail) centered in the zone; cards
    /// that cannot fit are not shown.
    pub fn cards(&self) -> Vec<Rect> {
        let (Some((_, zone)), Some((width, height, columns))) = (self.zone(), self.grid()) else {
            return Vec::new();
        };
        let n = self.candidates.len();
        let rows = n.div_ceil(columns);
        let card_h = height + TITLE;
        let total_h = rows as i32 * (card_h + GAP) - GAP;
        let top = zone.y() + ((zone.height() as i32 - total_h) / 2).max(PAD);
        let mut out = Vec::new();
        for i in 0..n {
            let (row, col) = (i / columns, i % columns);
            let in_row = (n - row * columns).min(columns) as i32;
            let row_w = in_row * (width + GAP) - GAP;
            let x = zone.x() + (zone.width() as i32 - row_w) / 2 + col as i32 * (width + GAP);
            let y = top + row as i32 * (card_h + GAP);
            if y + card_h > zone.bottom() - 8 {
                break;
            }
            if let Ok(r) = Rect::new(x, y, width as u32, card_h as u32) {
                out.push(r);
            }
        }
        out
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.zone().is_some_and(|(_, z)| z.contains(x, y))
    }
    /// Candidate under the pointer.
    pub fn hit(&self, x: i32, y: i32) -> Option<usize> {
        self.cards().iter().position(|r| r.contains(x, y))
    }
    /// Region the chooser paints (for repaint and dismissal).
    pub fn region(&self) -> Option<Rect> {
        self.zone().map(|(_, z)| z)
    }
    /// Arrows move the selection, Enter/Space pick, Esc dismisses.
    /// `key`: scan code.
    pub fn key(&mut self, scan: u32) -> Key {
        let shown = self.cards().len();
        if shown == 0 {
            return if scan == 1 {
                Key::Dismiss
            } else {
                Key::Ignored
            };
        }
        let columns = self.grid().map_or(1, |g| g.2);
        let current = self.selected;
        let next = match scan {
            1 => return Key::Dismiss,
            28 | 0x11c | 57 => return current.map_or(Key::Ignored, Key::Pick),
            0x14b => current.map_or(0, |i| i.saturating_sub(1)),
            0x14d | 15 => current.map_or(0, |i| (i + 1) % shown),
            0x148 => current.map_or(0, |i| i.saturating_sub(columns)),
            0x150 => current.map_or(0, |i| if i + columns < shown { i + columns } else { i }),
            _ => return Key::Ignored,
        };
        self.selected = Some(next.min(shown - 1));
        Key::Moved
    }
    /// After candidate `index` was snapped into the current zone: the next
    /// empty zone offers the rest. Returns whether the chooser continues.
    pub fn picked(&mut self, index: usize) -> bool {
        if index < self.candidates.len() {
            self.candidates.remove(index);
        }
        if !self.zones.is_empty() {
            self.zones.remove(0);
        }
        self.selected = None;
        !self.zones.is_empty() && !self.candidates.is_empty()
    }
    /// Drop windows that closed or left this desktop; refresh thumbnails
    /// whose snapshot changed. Returns whether anything is left to offer.
    pub fn refresh(&mut self, engine: &Engine) -> bool {
        self.candidates.retain(|c| {
            engine.window_info(c.id).is_ok_and(|w| {
                w.owner == c.owner
                    && w.state != WindowState::Closing
                    && w.workspace == engine.active_workspace()
            })
        });
        let scale = engine.scale();
        let Some((width, height, _)) = self.grid() else {
            return false;
        };
        let size = (scale.len(width as u32), scale.len(height as u32));
        for c in &mut self.candidates {
            if let Ok(info) = engine.window_info(c.id)
                && !info.title.is_empty()
                && c.title != info.title
            {
                c.title = info.title.into();
            }
            let Some(snapshot) = engine.snapshot(c.id) else {
                continue;
            };
            if c.revision == Some(snapshot.revision) && c.size == size {
                continue;
            }
            let d = snapshot.descriptor;
            c.pixels.resize(size.0 as usize * size.1 as usize * 4, 0);
            fit(
                snapshot.bytes,
                (d.width as usize, d.height as usize, d.stride_bytes),
                &mut c.pixels,
                (size.0 as usize, size.1 as usize),
            );
            c.revision = Some(snapshot.revision);
            c.size = size;
        }
        if self.selected.is_some_and(|i| i >= self.candidates.len()) {
            self.selected = None;
        }
        !self.candidates.is_empty() && !self.zones.is_empty()
    }
    pub fn draw(
        &self,
        c: &mut Canvas<'_>,
        fonts: &FontAtlas,
        dark: bool,
        pointer: (i32, i32),
        now: u64,
    ) {
        let Some((_, zone)) = self.zone() else {
            return;
        };
        let palette = Palette::new(dark);
        let (offset, _) = motion::slide(self.motion.progress(now), SLIDE);
        // Translucent zone surface (acrylic-like tint) inset from the edges.
        if let Ok(surface) = Rect::new(
            zone.x() + 6,
            zone.y() + 6,
            zone.width().saturating_sub(12),
            zone.height().saturating_sub(12),
        ) {
            let tint = if palette.contrast {
                palette.surface
            } else if dark {
                tinted([32, 32, 32, 255], 235)
            } else {
                tinted([243, 243, 243, 255], 235)
            };
            paint::rounded(c, surface, 8, tint);
        }
        let hover = self.hit(pointer.0, pointer.1);
        for (i, card) in self.cards().iter().enumerate() {
            let Some(candidate) = self.candidates.get(i) else {
                break;
            };
            let Ok(r) = Rect::new(card.x(), card.y() + offset, card.width(), card.height()) else {
                continue;
            };
            let chosen = self.selected == Some(i);
            if (chosen || hover == Some(i))
                && let Ok(bg) = Rect::new(r.x() - 6, r.y() - 4, r.width() + 12, r.height() + 10)
            {
                paint::rounded(c, bg, 8, palette.subtle_hover);
                if chosen {
                    paint::focus_ring(c, bg, &palette);
                }
            }
            if let Some(app) = candidate.app {
                crate::apps::draw_icon(c, r.x(), r.y() + 8, app, 16);
            }
            let title_x = r.x() + if candidate.app.is_some() { 24 } else { 0 };
            let title_w = (r.right() - title_x).max(1) as u32;
            let title = paint::ellipsize(fonts, &candidate.title, TextStyle::Regular12, title_w);
            paint::text_row(
                c,
                fonts,
                title_x,
                r.y() + 4,
                24,
                &title,
                TextStyle::Regular12,
                palette.text,
                title_w,
            );
            let thumb =
                Rect::new(r.x(), r.y() + TITLE, r.width(), r.height() - TITLE as u32).unwrap();
            let s = c.scale();
            let size = (s.len(thumb.width()), s.len(thumb.height()));
            if candidate.pixels.is_empty() || candidate.size != size {
                paint::rounded(c, thumb, 4, palette.card);
            } else if let Ok(at) = Rect::new(s.px(thumb.x()), s.px(thumb.y()), size.0, size.1) {
                let _ = c.blit_bgra_physical(at, &candidate.pixels);
            }
        }
    }
}
fn tinted(color: [u8; 4], alpha: u8) -> [u8; 4] {
    let a = u32::from(alpha);
    [
        (u32::from(color[0]) * a / 255) as u8,
        (u32::from(color[1]) * a / 255) as u8,
        (u32::from(color[2]) * a / 255) as u8,
        alpha,
    ]
}
/// Bilinear copy of a premultiplied BGRA snapshot into `dst`, fitted with
/// its aspect ratio kept and transparent letterbox bars.
fn fit(
    src: &[u8],
    (sw, sh, stride): (usize, usize, usize),
    dst: &mut [u8],
    (width, height): (usize, usize),
) {
    dst.fill(0);
    if sw == 0 || sh == 0 || width == 0 || height == 0 || src.len() < stride * (sh - 1) + sw * 4 {
        return;
    }
    let dw = width.min(height * sw / sh).max(1);
    let dh = height.min(width * sh / sw).max(1);
    let (ox, oy) = ((width - dw) / 2, (height - dh) / 2);
    for y in 0..dh {
        let sy = (((y as u64 * 2 + 1) * sh as u64 * 32768 / dh as u64).saturating_sub(32768))
            .min(((sh - 1) as u64) << 16);
        let (iy, fy) = (sy as usize >> 16, sy & 65535);
        let iy2 = (iy + 1).min(sh - 1);
        for x in 0..dw {
            let sx = (((x as u64 * 2 + 1) * sw as u64 * 32768 / dw as u64).saturating_sub(32768))
                .min(((sw - 1) as u64) << 16);
            let (ix, fx) = (sx as usize >> 16, sx & 65535);
            let ix2 = (ix + 1).min(sw - 1);
            for channel in 0..4 {
                let v = |px: usize, py: usize| u64::from(src[py * stride + px * 4 + channel]);
                let a = v(ix, iy) * (65536 - fx) + v(ix2, iy) * fx;
                let b = v(ix, iy2) * (65536 - fx) + v(ix2, iy2) * fx;
                dst[((y + oy) * width + x + ox) * 4 + channel] =
                    ((a * (65536 - fy) + b * fy + (1 << 31)) >> 32) as u8;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use door_graphics_core::{Limits, Role, SurfaceDescriptor, Theme};
    fn owner(pid: u64) -> Owner {
        Owner {
            pid,
            session: 1,
            epoch: 1,
        }
    }
    #[test]
    fn remaining_zones_offer_other_windows_and_advance_after_each_pick() {
        let area = Rect::new(0, 0, 1920, 1032).unwrap();
        let candidates = (0..3)
            .map(|i| Candidate::new(WindowId(10 + i), owner(i + 1), "Window", Some(App::Notes)))
            .collect();
        // Quadrants: the top-left was filled; three zones remain.
        let mut a = Assist::new(3, 0, area, candidates, 0, true).unwrap();
        assert_eq!(a.zones.iter().map(|z| z.0).collect::<Vec<_>>(), [1, 2, 3]);
        let zone = a.zone().unwrap().1;
        assert_eq!(zone, Rect::new(960, 0, 960, 516).unwrap());
        let cards = a.cards();
        assert_eq!(cards.len(), 3);
        for r in &cards {
            assert_eq!(zone.intersection(*r), Some(*r), "card inside its zone");
        }
        assert!(cards.windows(2).all(|w| w[0].intersection(w[1]).is_none()));
        assert_eq!(a.hit(cards[1].x() + 4, cards[1].y() + 40), Some(1));
        assert!(!a.contains(10, 10));
        // Keyboard: the first arrow selects, Enter picks.
        assert_eq!(a.key(28), Key::Ignored);
        assert_eq!(a.key(0x14d), Key::Moved);
        assert_eq!(a.key(0x14d), Key::Moved);
        assert_eq!(a.key(28), Key::Pick(1));
        assert!(a.picked(1));
        assert_eq!(a.zone().unwrap().0, 2);
        assert_eq!(a.candidates.len(), 2);
        assert!(a.picked(0));
        assert!(!a.picked(0), "every zone filled");
        assert_eq!(a.key(1), Key::Dismiss);
        // Halves with no other window: nothing to offer.
        assert!(Assist::new(0, 0, area, Vec::new(), 0, true).is_none());
    }
    #[test]
    fn many_windows_shrink_to_fit_a_small_zone_and_motion_respects_reduction() {
        let area = Rect::new(0, 0, 1280, 672).unwrap();
        let many: Vec<_> = (0..8)
            .map(|i| Candidate::new(WindowId(i + 1), owner(i + 1), "W", None))
            .collect();
        let a = Assist::new(3, 0, area, many, 0, false).unwrap();
        let zone = a.zone().unwrap().1;
        let cards = a.cards();
        assert!(!cards.is_empty());
        for r in &cards {
            assert_eq!(zone.intersection(*r), Some(*r));
            assert!(r.width() as i32 >= MIN_THUMB);
        }
        assert!(a.motion.animating(1));
        let calm = Assist::new(
            3,
            0,
            area,
            alloc::vec![Candidate::new(WindowId(1), owner(1), "W", None)],
            0,
            true,
        )
        .unwrap();
        assert!(!calm.motion.animating(1));
    }
    #[test]
    fn thumbnails_come_from_committed_snapshots_and_track_closed_windows() {
        let mut e = Engine::new(1280, 720, Theme::Light, Limits::default()).unwrap();
        let o = owner(1);
        let id = e
            .create_window(o, Role::TopLevel, Rect::new(50, 50, 240, 176).unwrap())
            .unwrap();
        e.acknowledge_configure(o, id, 1).unwrap();
        let b = e
            .register_buffer(
                o,
                SurfaceDescriptor::new(240, 144, 960, 240 * 144 * 4).unwrap(),
            )
            .unwrap();
        e.attach(o, id, b, 1).unwrap();
        e.commit(o, id, [10, 20, 30, 255].repeat(240 * 144))
            .unwrap();
        let area = e.work_area();
        let mut a = Assist::new(
            0,
            0,
            area,
            alloc::vec![Candidate::new(id, o, "", None)],
            0,
            true,
        )
        .unwrap();
        assert!(a.refresh(&e));
        let c = &a.candidates[0];
        let (w, h) = c.size;
        let center = ((h as usize / 2) * w as usize + w as usize / 2) * 4;
        assert_eq!(&c.pixels[center..center + 4], &[10, 20, 30, 255]);
        let fonts = FontAtlas::windows_reference().unwrap();
        let mut bytes = alloc::vec![0u8; 1280 * 720 * 4];
        let mut canvas = Canvas::new(&mut bytes, 1280, 720, 5120).unwrap();
        a.draw(&mut canvas, &fonts, false, (0, 0), 0);
        let card = a.cards()[0];
        let at = (((card.y() + TITLE + card.height() as i32 / 3) as usize) * 1280
            + card.x() as usize
            + card.width() as usize / 2)
            * 4;
        assert_eq!(&bytes[at..at + 4], &[10, 20, 30, 255]);
        e.destroy_window(o, id).ok();
        assert!(!a.refresh(&e), "closed window leaves nothing to offer");
    }
}
