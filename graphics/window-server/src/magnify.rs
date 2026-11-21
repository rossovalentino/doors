//! Magnifier's view of the composed output (`window::MAGNIFY`): full screen
//! (the whole output shows a 1/z region that follows the pointer), lens (a
//! rectangle around the pointer) or docked (a band across the top). The
//! source is sampled bilinearly ("smooth edges", Windows' default).
extern crate alloc;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    FullScreen,
    Lens,
    Docked,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Magnify {
    /// 101..=1600.
    pub percent: u32,
    pub view: View,
    /// The Magnifier process (the view ends with it).
    pub pid: u64,
}
/// A destination rectangle and the source origin it shows (physical).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    pub dest: (i32, i32, u32, u32),
    pub source: (f32, f32),
    pub zoom: f32,
}
impl Magnify {
    pub fn new(percent: u32, view: u32, pid: u64) -> Option<Self> {
        let view = match view {
            0 => View::FullScreen,
            1 => View::Lens,
            2 => View::Docked,
            _ => return None,
        };
        (101..=1600)
            .contains(&percent)
            .then_some(Self { percent, view, pid })
    }
    pub fn plan(&self, w: u32, h: u32, px: i32, py: i32) -> Plan {
        let z = self.percent as f32 / 100.0;
        let dest = match self.view {
            View::FullScreen => (0, 0, w, h),
            View::Lens => {
                let (lw, lh) = (w * 35 / 100, h * 30 / 100);
                let x = (px - lw as i32 / 2).clamp(0, (w - lw) as i32);
                let y = (py - lh as i32 / 2).clamp(0, (h - lh) as i32);
                (x, y, lw, lh)
            }
            View::Docked => (0, 0, w, h * 30 / 100),
        };
        let (sw, sh) = (dest.2 as f32 / z, dest.3 as f32 / z);
        let sx = (px as f32 - sw / 2.0).clamp(0.0, (w as f32 - sw).max(0.0));
        let sy = (py as f32 - sh / 2.0).clamp(0.0, (h as f32 - sh).max(0.0));
        Plan {
            dest,
            source: (sx, sy),
            zoom: z,
        }
    }
    /// Where the pointer appears: full screen moves it with the view; the
    /// lens and the dock leave it where it is.
    pub fn pointer(&self, w: u32, h: u32, px: i32, py: i32) -> (i32, i32) {
        if self.view != View::FullScreen {
            return (px, py);
        }
        let p = self.plan(w, h, px, py);
        (
            ((px as f32 - p.source.0) * p.zoom) as i32,
            ((py as f32 - p.source.1) * p.zoom) as i32,
        )
    }
    /// The magnified output (packed rows, `w` x `h`) from `input`.
    pub fn render(
        &self,
        input: &[u8],
        stride: usize,
        w: u32,
        h: u32,
        pointer: (i32, i32),
        out: &mut Vec<u8>,
    ) {
        let (wu, hu) = (w as usize, h as usize);
        out.clear();
        if out.try_reserve(wu * hu * 4).is_err() {
            return;
        }
        for y in 0..hu {
            out.extend_from_slice(&input[y * stride..y * stride + wu * 4]);
        }
        let p = self.plan(w, h, pointer.0, pointer.1);
        let (dx, dy, dw, dh) = p.dest;
        let sample = |sx: f32, sy: f32, c: usize| -> u8 {
            let x0 = (sx.max(0.0) as usize).min(wu - 1);
            let y0 = (sy.max(0.0) as usize).min(hu - 1);
            let (x1, y1) = ((x0 + 1).min(wu - 1), (y0 + 1).min(hu - 1));
            let (tx, ty) = (sx - x0 as f32, sy - y0 as f32);
            let at = |x: usize, y: usize| input[y * stride + x * 4 + c] as f32;
            let top = at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx;
            let bottom = at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx;
            (top * (1.0 - ty) + bottom * ty + 0.5) as u8
        };
        for y in 0..dh as usize {
            let sy = p.source.1 + (y as f32 + 0.5) / p.zoom - 0.5;
            let row = (dy as usize + y) * wu * 4;
            for x in 0..dw as usize {
                let sx = p.source.0 + (x as f32 + 0.5) / p.zoom - 0.5;
                let at = row + (dx as usize + x) * 4;
                for c in 0..3 {
                    out[at + c] = sample(sx, sy, c);
                }
                out[at + 3] = 255;
            }
        }
        // The lens and the dock carry a frame, as Windows draws them.
        if self.view != View::FullScreen {
            let edge = [215u8, 120, 0, 255];
            for x in dx as usize..(dx as usize + dw as usize) {
                for y in [
                    dy as usize,
                    dy as usize + 1,
                    dy as usize + dh as usize - 2,
                    dy as usize + dh as usize - 1,
                ] {
                    out[(y * wu + x) * 4..][..4].copy_from_slice(&edge);
                }
            }
            for y in dy as usize..(dy as usize + dh as usize) {
                for x in [
                    dx as usize,
                    dx as usize + 1,
                    dx as usize + dw as usize - 2,
                    dx as usize + dw as usize - 1,
                ] {
                    out[(y * wu + x) * 4..][..4].copy_from_slice(&edge);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_screen_follows_the_pointer_and_maps_it() {
        let m = Magnify::new(200, 0, 7).unwrap();
        let p = m.plan(1280, 720, 640, 360);
        assert_eq!(p.source, (320.0, 180.0));
        assert_eq!(m.pointer(1280, 720, 640, 360), (640, 360));
        // At the left edge the view stops at the screen edge.
        assert_eq!(m.plan(1280, 720, 10, 10).source, (0.0, 0.0));
        assert_eq!(m.pointer(1280, 720, 10, 10), (20, 20));
        assert!(Magnify::new(100, 0, 1).is_none());
        let lens = Magnify::new(300, 1, 1).unwrap();
        assert_eq!(lens.pointer(1280, 720, 100, 100), (100, 100));
        let mut out = Vec::new();
        let input = alloc::vec![9u8; 64 * 48 * 4];
        lens.render(&input, 64 * 4, 64, 48, (32, 24), &mut out);
        assert_eq!(out.len(), 64 * 48 * 4);
    }
}
