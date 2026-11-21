//! Cached frosted taskbar backdrop. Hover never recomputes the scene blur.
extern crate alloc;
use alloc::vec::Vec;
use door_graphics_core::{Rect, Scale};
pub struct Taskbar {
    width: usize,
    height: usize,
    rows: usize,
    start: usize,
    region: Rect,
    sample_left: usize,
    sample_width: usize,
    sample_end: usize,
    scratch: Vec<u8>,
    pixels: Vec<u8>,
    /// Scene weight out of 255 (`set_look`); the tint takes the rest.
    scene_weight: u32,
}
impl Taskbar {
    /// "Taskbar look" (Doors extra, `preferences::taskbar::look`): 0 the
    /// Windows acrylic, 1 translucent, 2 opaque, 3 clear.
    pub fn set_look(&mut self, look: u32) -> bool {
        let weight = match look {
            1 => 110,
            2 => 0,
            3 => 235,
            _ => 24,
        };
        let changed = weight != self.scene_weight;
        self.scene_weight = weight;
        changed
    }
    pub fn new(width: u32, height: u32) -> Option<Self> {
        Self::new_scaled(width, height, Scale::ONE)
    }
    /// The bottom 48-DIP band of the logical output, snapped to physical
    /// pixels exactly as `taskbar::physical(layout.bar(), scale)`.
    pub fn new_scaled(width: u32, height: u32, scale: Scale) -> Option<Self> {
        if width == 0 || width > 3840 || height == 0 || height > 2160 {
            return None;
        }
        let (lw, lh) = (scale.logical_len(width), scale.logical_len(height));
        let band = Rect::new(0, lh as i32 - 48.min(lh as i32), lw, 48.min(lh)).ok()?;
        let region = scale.rect(band).ok()?;
        Self::for_region(width, height, region)
    }
    pub fn for_region(width: u32, height: u32, region: Rect) -> Option<Self> {
        let output = Rect::new(0, 0, width, height).ok()?;
        if width > 3840 || height > 2160 || output.intersection(region) != Some(region) {
            return None;
        }
        let (width, height) = (width as usize, height as usize);
        let rows = region.height() as usize;
        let start = (region.y() as usize).saturating_sub(12);
        let sample_end = (region.bottom() as usize + 12).min(height);
        let sample_left = (region.x() as usize).saturating_sub(12);
        let sample_width = (region.right() as usize + 12).min(width) - sample_left;
        let mut scratch = Vec::new();
        let mut pixels = Vec::new();
        scratch
            .try_reserve_exact(sample_width * (sample_end - start) * 3)
            .ok()?;
        pixels
            .try_reserve_exact(region.width() as usize * rows * 4)
            .ok()?;
        scratch.resize(sample_width * (sample_end - start) * 3, 0);
        pixels.resize(region.width() as usize * rows * 4, 0);
        Some(Self {
            width,
            height,
            rows,
            start,
            region,
            sample_left,
            sample_width,
            sample_end,
            scratch,
            pixels,
            scene_weight: 24,
        })
    }
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
    pub fn update(&mut self, scene: &[u8], dark: bool) -> bool {
        self.update_tinted(scene, dark, None)
    }
    /// As `update`, tinted with `accent` (Personalization > Colors > "Show
    /// accent color on Start and taskbar") instead of the neutral tint.
    pub fn update_tinted(&mut self, scene: &[u8], dark: bool, accent: Option<[u8; 3]>) -> bool {
        if scene.len() != self.width * self.height * 4 {
            return false;
        }
        let band = self.sample_end - self.start;
        for y in 0..band {
            for channel in 0..3 {
                let sample = |x: i32| {
                    scene[((y + self.start) * self.width
                        + x.clamp(0, self.width as i32 - 1) as usize)
                        * 4
                        + channel] as u32
                };
                let mut sum: u32 = (-12..=12)
                    .map(|dx| sample(self.sample_left as i32 + dx))
                    .sum();
                for x in 0..self.sample_width {
                    self.scratch[(y * self.sample_width + x) * 3 + channel] =
                        ((sum + 12) / 25) as u8;
                    sum = sum - sample((self.sample_left + x) as i32 - 12)
                        + sample((self.sample_left + x) as i32 + 13);
                }
            }
        }
        let center = (self.region.y() as usize - self.start) as i32;
        for x in 0..self.region.width() as usize {
            for channel in 0..3 {
                let sample = |y: i32| {
                    self.scratch[(y.clamp(0, band as i32 - 1) as usize * self.sample_width
                        + x
                        + self.region.x() as usize
                        - self.sample_left)
                        * 3
                        + channel] as u32
                };
                let mut sum: u32 = (-12..=12).map(|dy| sample(center + dy)).sum();
                let tint = match accent {
                    Some(a) => u32::from(a[channel]),
                    None if dark => 35,
                    None => [250, 245, 245][channel],
                };
                for y in 0..self.rows {
                    self.pixels[(y * self.region.width() as usize + x) * 4 + channel] =
                        ((((sum + 12) / 25) * self.scene_weight + tint * (255 - self.scene_weight))
                            / 255) as u8;
                    sum = sum - sample(center + y as i32 - 12) + sample(center + y as i32 + 13);
                }
            }
        }
        for p in self.pixels.chunks_exact_mut(4) {
            p[3] = 255;
        }
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crop_matches_full_reference_blur_at_every_output_edge() {
        let (w, h) = (96, 112);
        let mut scene = alloc::vec![255;w*h*4];
        for (i, p) in scene.chunks_exact_mut(4).enumerate() {
            p[..3].copy_from_slice(&[(i * 7) as u8, (i / 19) as u8, (i * 13) as u8]);
        }
        for dark in [false, true] {
            let mut full = Taskbar::for_region(
                w as u32,
                h as u32,
                Rect::new(0, 0, w as u32, h as u32).unwrap(),
            )
            .unwrap();
            assert!(full.update(&scene, dark));
            for r in [
                Rect::new(0, 0, 96, 40).unwrap(),
                Rect::new(0, 72, 96, 40).unwrap(),
                Rect::new(0, 0, 40, 112).unwrap(),
                Rect::new(56, 0, 40, 112).unwrap(),
            ] {
                let mut crop = Taskbar::for_region(w as u32, h as u32, r).unwrap();
                assert!(crop.update(&scene, dark));
                for y in 0..r.height() as usize {
                    let at = ((y + r.y() as usize) * w + r.x() as usize) * 4;
                    let row = r.width() as usize * 4;
                    assert_eq!(
                        &crop.pixels[y * row..(y + 1) * row],
                        &full.pixels[at..at + row]
                    );
                }
            }
        }
    }
    #[test]
    fn backdrop_blurs_scene_without_mutating_it_and_reuses_hover_cache() {
        let mut scene = alloc::vec![255;64*96*4];
        for y in 0..96 {
            for x in 0..64 {
                if x % 2 == 0 {
                    scene[(y * 64 + x) * 4..(y * 64 + x) * 4 + 3].fill(0);
                }
            }
        }
        let old = scene.clone();
        let mut bar = Taskbar::new(64, 96).unwrap();
        assert!(bar.update(&scene, false));
        assert_eq!(scene, old);
        let a = &bar.pixels[(24 * 64 + 24) * 4..(24 * 64 + 24) * 4 + 3];
        let b = &bar.pixels[(24 * 64 + 25) * 4..(24 * 64 + 25) * 4 + 3];
        assert!(a.iter().zip(b).all(|(&a, &b)| a.abs_diff(b) <= 1));
        assert!(bar.pixels.chunks_exact(4).all(|p| p[3] == 255));
        let ptr = bar.pixels.as_ptr();
        assert!(bar.update(&scene, true));
        assert_eq!(bar.pixels.as_ptr(), ptr);
        for height in [1, 24, 48] {
            let mut tiny = Taskbar::new(1, height).unwrap();
            assert!(tiny.update(&alloc::vec![255;height as usize*4], false));
        }
        assert!(!bar.update(&[], false));
    }
}
