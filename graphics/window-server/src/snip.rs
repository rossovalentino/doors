//! The snip overlay (Windows 11's screen clipping host): Win+Shift+S, Print
//! Screen or Snipping Tool's New freezes the screen, dims it and shows the
//! capture bar at the top centre (Rectangle, Window, Full screen, Free-form,
//! Close). A drag selects a rectangle or traces a free-form outline, a click
//! takes the window under the pointer or the whole screen. The result is
//! cropped from the frozen frame (free-form keeps only the traced area) and
//! goes to the clipboard as an image; Snipping Tool then opens it.
extern crate alloc;
use crate::paint;
use alloc::vec::Vec;
use door_graphics_core::{Canvas, Rect, Scale};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Rectangle,
    Window,
    FullScreen,
    FreeForm,
}
impl Mode {
    pub const ALL: [Self; 4] = [
        Self::Rectangle,
        Self::Window,
        Self::FullScreen,
        Self::FreeForm,
    ];
    pub fn from_word(word: u32) -> Option<Self> {
        Self::ALL.get(word as usize).copied()
    }
    fn mask(self) -> paint::Mask {
        match self {
            Self::Rectangle => crate::shell_mask!("snip-rectangle", 20),
            Self::Window => crate::shell_mask!("snip-window", 20),
            Self::FullScreen => crate::shell_mask!("snip-full-screen", 20),
            Self::FreeForm => crate::shell_mask!("snip-free-form", 20),
        }
    }
}
/// What a pointer press on the overlay hits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Mode(Mode),
    Close,
    Screen,
}
/// A finished snip: premultiplied BGRA rows.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}
impl Image {
    /// The clipboard's IMAGE format: `[width u32][height u32]` then rows.
    pub fn clipboard(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + self.pixels.len());
        out.extend_from_slice(&self.width.to_le_bytes());
        out.extend_from_slice(&self.height.to_le_bytes());
        out.extend_from_slice(&self.pixels);
        out
    }
}

pub struct Snip {
    pub mode: Mode,
    /// The frozen output (physical BGRA); empty until the frame after the
    /// delay is composed.
    pub frozen: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Physical drag start and current point.
    pub start: Option<(i32, i32)>,
    pub current: (i32, i32),
    pub lasso: Vec<(i32, i32)>,
    /// Physical outer rectangles of the windows, front to back (Window mode).
    pub windows: Vec<Rect>,
    /// Ticks before the screen freezes (Snipping Tool's delay).
    pub freeze_at: u64,
    /// The pid whose windows were hidden for the snip (restored after).
    pub requester: Option<u64>,
}
/// Logical bar metrics.
const BUTTON: u32 = 44;
const BAR_H: u32 = 48;
impl Snip {
    pub fn new(mode: Mode, freeze_at: u64, requester: Option<u64>) -> Self {
        Self {
            mode,
            frozen: Vec::new(),
            width: 0,
            height: 0,
            start: None,
            current: (0, 0),
            lasso: Vec::new(),
            windows: Vec::new(),
            freeze_at,
            requester,
        }
    }
    pub fn frozen(&self) -> bool {
        !self.frozen.is_empty()
    }
    /// Copy the composed output (stride = width x 4).
    pub fn freeze(&mut self, pixels: &[u8], width: u32, height: u32, windows: Vec<Rect>) {
        let mut copy = Vec::new();
        if copy.try_reserve_exact(pixels.len()).is_err() {
            return;
        }
        copy.extend_from_slice(pixels);
        self.frozen = copy;
        self.width = width;
        self.height = height;
        self.windows = windows;
    }
    /// The capture bar (logical), top centre of an output `width` wide.
    pub fn bar(width: u32) -> Rect {
        let w = 8 + 4 * BUTTON + 9 + BUTTON + 8;
        Rect::new((width as i32 - w as i32) / 2, 12, w, BAR_H).expect("bar")
    }
    fn button(width: u32, i: usize) -> Rect {
        let b = Self::bar(width);
        let x = b.x() + 8 + i as i32 * BUTTON as i32 + if i == 4 { 9 } else { 0 };
        Rect::new(x, b.y() + 4, BUTTON, BAR_H - 8).expect("button")
    }
    /// Logical hit test (`width` is the logical output width).
    pub fn hit(&self, width: u32, x: i32, y: i32) -> Hit {
        for (i, mode) in Mode::ALL.iter().enumerate() {
            if Self::button(width, i).contains(x, y) {
                return Hit::Mode(*mode);
            }
        }
        if Self::button(width, 4).contains(x, y) {
            return Hit::Close;
        }
        Hit::Screen
    }
    /// The physical selection so far.
    pub fn selection(&self) -> Option<Rect> {
        match self.mode {
            Mode::Rectangle => {
                let (x0, y0) = self.start?;
                let (x1, y1) = self.current;
                let (l, t) = (x0.min(x1).max(0), y0.min(y1).max(0));
                let (r, b) = (
                    x0.max(x1).min(self.width as i32),
                    y0.max(y1).min(self.height as i32),
                );
                Rect::new(l, t, (r - l).max(0) as u32, (b - t).max(0) as u32)
                    .ok()
                    .filter(|r| r.width() > 1 && r.height() > 1)
            }
            Mode::Window => self
                .windows
                .iter()
                .copied()
                .find(|w| w.contains(self.current.0, self.current.1))
                .and_then(|w| {
                    Rect::new(0, 0, self.width, self.height)
                        .ok()
                        .and_then(|screen| screen.intersection(w))
                }),
            Mode::FullScreen => Rect::new(0, 0, self.width, self.height).ok(),
            Mode::FreeForm => {
                let (mut l, mut t, mut r, mut b) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
                for &(x, y) in &self.lasso {
                    (l, t, r, b) = (l.min(x), t.min(y), r.max(x), b.max(y));
                }
                let (l, t) = (l.max(0), t.max(0));
                let (r, b) = (r.min(self.width as i32), b.min(self.height as i32));
                (self.lasso.len() >= 3 && r - l > 1 && b - t > 1)
                    .then(|| Rect::new(l, t, (r - l) as u32, (b - t) as u32).ok())
                    .flatten()
            }
        }
    }
    pub fn press(&mut self, x: i32, y: i32) {
        self.start = Some((x, y));
        self.current = (x, y);
        self.lasso.clear();
        self.lasso.push((x, y));
    }
    pub fn motion(&mut self, x: i32, y: i32) {
        self.current = (x, y);
        if self.start.is_some() && self.mode == Mode::FreeForm {
            let last = self.lasso.last().copied();
            if last.is_none_or(|(lx, ly)| (lx - x).abs() + (ly - y).abs() >= 2) {
                self.lasso.push((x, y));
            }
        }
    }
    /// The snip for the current selection.
    pub fn take(&self) -> Option<Image> {
        let r = self.selection()?;
        let (w, h) = (r.width() as usize, r.height() as usize);
        let stride = self.width as usize * 4;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(w * h * 4).ok()?;
        for y in r.y() as usize..r.bottom() as usize {
            let at = y * stride + r.x() as usize * 4;
            pixels.extend_from_slice(self.frozen.get(at..at + w * 4)?);
        }
        for p in pixels.chunks_exact_mut(4) {
            p[3] = 255;
        }
        if self.mode == Mode::FreeForm {
            // Only the traced area is kept; the rest is transparent.
            for yy in 0..h {
                let sy = (r.y() as usize + yy) as f32 + 0.5;
                let mut xs: Vec<f32> = Vec::new();
                let n = self.lasso.len();
                for i in 0..n {
                    let (a, b) = (self.lasso[i], self.lasso[(i + 1) % n]);
                    let (ay, by) = (a.1 as f32, b.1 as f32);
                    if (ay <= sy && by > sy) || (by <= sy && ay > sy) {
                        xs.push(a.0 as f32 + (sy - ay) / (by - ay) * (b.0 - a.0) as f32);
                    }
                }
                xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
                for xx in 0..w {
                    let sx = (r.x() as usize + xx) as f32 + 0.5;
                    let inside = xs.iter().filter(|x| **x < sx).count() % 2 == 1;
                    if !inside {
                        pixels[(yy * w + xx) * 4..][..4].fill(0);
                    }
                }
            }
        }
        Some(Image {
            width: w as u32,
            height: h as u32,
            pixels,
        })
    }
    /// Paint the frozen screen, the dim outside the selection and the bar.
    pub fn draw(&self, c: &mut Canvas<'_>, dark: bool, pointer: (i32, i32)) {
        if !self.frozen() {
            return;
        }
        let (pw, ph, stride) = c.physical_dimensions();
        if pw != self.width || ph != self.height {
            return;
        }
        let out = c.pixels_mut();
        for y in 0..ph as usize {
            let row = &self.frozen[y * pw as usize * 4..(y + 1) * pw as usize * 4];
            out[y * stride..y * stride + row.len()].copy_from_slice(row);
        }
        // Dim everything outside the selection to 45 %.
        let keep = self
            .selection()
            .filter(|_| self.start.is_some() || self.mode == Mode::Window);
        let lasso_in = |x: usize, y: usize| -> bool {
            let sy = y as f32 + 0.5;
            let n = self.lasso.len();
            let mut inside = false;
            for i in 0..n {
                let (a, b) = (self.lasso[i], self.lasso[(i + 1) % n]);
                let (ay, by) = (a.1 as f32, b.1 as f32);
                if (ay <= sy && by > sy) || (by <= sy && ay > sy) {
                    let xx = a.0 as f32 + (sy - ay) / (by - ay) * (b.0 - a.0) as f32;
                    if xx < x as f32 + 0.5 {
                        inside = !inside;
                    }
                }
            }
            inside
        };
        for y in 0..ph as usize {
            for x in 0..pw as usize {
                let bright = match keep {
                    Some(r) if self.mode == Mode::FreeForm => {
                        r.contains(x as i32, y as i32) && lasso_in(x, y)
                    }
                    Some(r) => r.contains(x as i32, y as i32),
                    None => false,
                };
                if !bright {
                    let at = y * stride + x * 4;
                    for c in &mut out[at..at + 3] {
                        *c = (*c as u32 * 115 / 255) as u8;
                    }
                }
            }
        }
        if let Some(r) = keep
            && self.mode != Mode::FreeForm
        {
            let _ = c.set_scale(Scale::ONE);
            paint::outline(c, r, [255, 255, 255, 255], 1);
        }
        if self.mode == Mode::FreeForm {
            let _ = c.set_scale(Scale::ONE);
            for pair in self.lasso.windows(2) {
                let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
                let n = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
                for i in 0..=n {
                    paint::rect(
                        c,
                        x0 + (x1 - x0) * i / n,
                        y0 + (y1 - y0) * i / n,
                        1,
                        1,
                        [255, 255, 255, 255],
                    );
                }
            }
        }
        // The capture bar.
        let scale = c.scale();
        let width = scale.logical_len(pw);
        let _ = c.set_scale(scale);
        let bar = Self::bar(width);
        paint::shadow(c, bar, 8, dark);
        let (body, edge, text, hover, selected) = if dark {
            (
                [44, 44, 44, 255],
                [70, 70, 70, 255],
                [255, 255, 255, 255],
                [60, 60, 60, 255],
                [64, 64, 64, 255],
            )
        } else {
            (
                [249, 249, 249, 255],
                [214, 214, 214, 255],
                [26, 26, 26, 255],
                [234, 234, 234, 255],
                [228, 228, 228, 255],
            )
        };
        paint::framed(c, bar, 8, edge, body);
        for (i, mode) in Mode::ALL.iter().enumerate() {
            let r = Self::button(width, i);
            if *mode == self.mode {
                paint::rounded(c, r, 4, selected);
                let pill = Rect::new(r.x() + r.width() as i32 / 2 - 8, r.bottom() - 4, 16, 3)
                    .expect("pill");
                paint::rounded(c, pill, 1, paint::Palette::new(dark).accent);
            } else if r.contains(pointer.0, pointer.1) {
                paint::rounded(c, r, 4, hover);
            }
            paint::glyph_in(c, r, 20, mode.mask(), text);
        }
        let sep = Self::button(width, 4);
        paint::rect(c, sep.x() - 5, bar.y() + 12, 1, BAR_H - 24, edge);
        if sep.contains(pointer.0, pointer.1) {
            paint::rounded(c, sep, 4, hover);
        }
        paint::glyph_in(c, sep, 16, crate::shell_mask!("snip-close", 16), text);
        let _ = c.set_scale(Scale::ONE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rectangle_and_free_form_snips_crop_the_frozen_frame() {
        let (w, h) = (40u32, 30u32);
        let mut frame = alloc::vec![0u8; (w * h * 4) as usize];
        for (i, p) in frame.chunks_exact_mut(4).enumerate() {
            p.copy_from_slice(&[(i % 40) as u8, (i / 40) as u8, 7, 255]);
        }
        let mut s = Snip::new(Mode::Rectangle, 0, None);
        s.freeze(&frame, w, h, alloc::vec![Rect::new(5, 5, 10, 10).unwrap()]);
        s.press(10, 8);
        s.motion(20, 18);
        let img = s.take().unwrap();
        assert_eq!((img.width, img.height), (10, 10));
        assert_eq!(&img.pixels[..4], &[10, 8, 7, 255]);
        assert_eq!(img.clipboard().len(), 8 + 400);
        s.mode = Mode::Window;
        s.current = (7, 7);
        assert_eq!(s.selection(), Some(Rect::new(5, 5, 10, 10).unwrap()));
        s.mode = Mode::FullScreen;
        assert_eq!(s.take().unwrap().width, 40);
        let mut f = Snip::new(Mode::FreeForm, 0, None);
        f.freeze(&frame, w, h, alloc::vec![]);
        f.press(0, 0);
        f.motion(20, 0);
        f.motion(20, 20);
        f.motion(0, 20);
        let img = f.take().unwrap();
        assert_eq!(img.pixels[((5 * img.width + 5) * 4 + 3) as usize], 255);
        assert_eq!(Snip::bar(1280).x(), (1280 - (8 + 4 * 44 + 9 + 44 + 8)) / 2);
        assert_eq!(
            s.hit(1280, Snip::bar(1280).x() + 10, 30),
            Hit::Mode(Mode::Rectangle)
        );
    }
}
