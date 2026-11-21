//! Bounded image edits keep an owned undo image and never overwrite input files.
use super::*;
use door_app_ui::fluent::{self as f, Rect as R};
impl Preview {
    pub(super) fn editable_image(&self) -> bool {
        self.selected()
            .is_some_and(|t| Kind::of(&t.path, false) == Kind::Image && t.image.is_some())
    }
    fn copy_image(image: &Image) -> Option<Image> {
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(image.bgra.len()).ok()?;
        pixels.extend_from_slice(&image.bgra);
        Some(Image {
            width: image.width,
            height: image.height,
            source_width: image.source_width,
            source_height: image.source_height,
            bgra: pixels,
        })
    }
    pub(super) fn checkpoint_image(&mut self) -> bool {
        if !self.editable_image() {
            return false;
        }
        let tab = &self.tabs[self.active];
        let image = tab.image.as_ref().unwrap();
        if self
            .image_bytes()
            .saturating_sub(tab.backup.as_ref().map_or(0, |i| i.bgra.len()))
            .saturating_add(image.bgra.len())
            > IMAGE_BUDGET
        {
            self.message =
                "Close another image to free undo memory. The image is unchanged.".into();
            return false;
        }
        let Some(copy) = Self::copy_image(image) else {
            self.message =
                "Image edit could not reserve undo memory. The image is unchanged.".into();
            return false;
        };
        self.tabs[self.active].backup = Some(copy);
        true
    }
    pub(super) fn transform_image(&mut self, rotate: bool) -> bool {
        if !door_workspace::runtime::working_memory(16 * 1024 * 1024) {
            self.message =
                "Not enough shared app memory for this edit. Existing images are retained.".into();
            return false;
        }
        if !self.checkpoint_image() {
            return false;
        }
        if let Some(image) = self
            .tabs
            .get_mut(self.active)
            .and_then(|t| t.image.as_mut())
        {
            if rotate {
                image.rotate_clockwise();
            } else {
                image.flip_horizontal();
            }
        }
        true
    }
    pub(super) fn image_shortcut(&mut self, key: u32) -> bool {
        if self.modal() || self.busy() || (key >> 9) & door_abi::graphics::CONTROL == 0 {
            return false;
        }
        match key & 511 {
            19 => {
                self.transform_image(true);
                true
            }
            44 | 21 => {
                self.undo_image();
                true
            }
            _ => false,
        }
    }
    pub(super) fn undo_image(&mut self) {
        if let Some(t) = self.tabs.get_mut(self.active)
            && t.backup.is_some()
        {
            core::mem::swap(&mut t.image, &mut t.backup);
            self.fit = true;
            self.message = "Image edit reversed. Export creates a separate file.".into();
        }
    }
    pub(super) fn cancel_crop(&mut self) {
        self.cropping = false;
        self.crop_drag = false;
        self.crop = None;
    }
    fn image_box(&self) -> Option<(i32, i32, u32, u32)> {
        let image = self.selected()?.image.as_ref()?;
        let (x, y, w, h) = self.dimensions();
        let (w, h) = (w.saturating_sub(16), h.saturating_sub(16));
        if w == 0 || h == 0 {
            return None;
        }
        let (dw, dh) = if u64::from(w) * image.height as u64 <= u64::from(h) * image.width as u64 {
            (
                w,
                (u64::from(w) * image.height as u64 / image.width as u64).max(1) as u32,
            )
        } else {
            (
                (u64::from(h) * image.width as u64 / image.height as u64).max(1) as u32,
                h,
            )
        };
        Some((
            x + 8 + (w - dw) as i32 / 2,
            y + 8 + (h - dh) as i32 / 2,
            dw,
            dh,
        ))
    }
    pub(super) fn crop_pointer(&mut self, x: i32, y: i32, press: bool) -> bool {
        if !self.cropping {
            return false;
        }
        let Some((left, top, w, h)) = self.image_box() else {
            return false;
        };
        if press {
            if !(left..left + w as i32).contains(&x) || !(top..top + h as i32).contains(&y) {
                return false;
            }
            self.crop = Some((x, y, x, y));
            self.crop_drag = true;
        } else if self.crop_drag {
            if let Some(c) = self.crop.as_mut() {
                c.2 = x.clamp(left, left + w as i32 - 1);
                c.3 = y.clamp(top, top + h as i32 - 1);
            }
        } else {
            return false;
        }
        true
    }
    pub(super) fn apply_crop(&mut self) -> bool {
        let Some((x0, y0, x1, y1)) = self.crop else {
            return false;
        };
        let Some((left, top, w, h)) = self.image_box() else {
            return false;
        };
        let image = self.selected().and_then(|t| t.image.as_ref()).unwrap();
        let sx = |x: i32| {
            ((x - left).clamp(0, w as i32 - 1) as usize * image.width / w as usize)
                .min(image.width - 1)
        };
        let sy = |y: i32| {
            ((y - top).clamp(0, h as i32 - 1) as usize * image.height / h as usize)
                .min(image.height - 1)
        };
        let (l, r, t, b) = (
            sx(x0.min(x1)),
            sx(x0.max(x1)) + 1,
            sy(y0.min(y1)),
            sy(y0.max(y1)) + 1,
        );
        let (width, height) = (r - l, b - t);
        let mut pixels = Vec::new();
        if pixels.try_reserve_exact(width * height * 4).is_err() {
            self.message = "Crop could not reserve memory. The image is unchanged.".into();
            return false;
        }
        for y in t..b {
            pixels.extend_from_slice(
                &image.bgra[(y * image.width + l) * 4..(y * image.width + r) * 4],
            );
        }
        if !self.checkpoint_image() {
            return false;
        }
        self.tabs[self.active].image = Some(Image {
            width,
            height,
            source_width: width,
            source_height: height,
            bgra: pixels,
        });
        self.cancel_crop();
        self.fit = true;
        self.message = "Cropped view. Use Export to save a separate file; Ctrl+Z to undo.".into();
        true
    }
    pub(super) fn crop_controls(&self) -> [R; 2] {
        let x = self.dimensions().0;
        let width = self.width.saturating_sub(x as u32 + 16);
        let cell = (width / 2).min(100);
        let h = presentation::tool_h();
        [
            R::new(x, 72, cell, h),
            R::new(x + cell as i32 + 8, 72, cell, h),
        ]
    }
    pub(super) fn paint_crop(&self, v: &mut View<'_>) {
        if !self.cropping {
            return;
        }
        let x = self.dimensions().0;
        v.rect(
            x,
            60,
            v.width.saturating_sub(x as u32),
            presentation::tool_h() + 24,
            [252, 248, 243, 255],
        );
        let [apply, cancel] = self.crop_controls();
        f::button(v, apply, "Apply", None, true, self.crop.is_some());
        f::button(v, cancel, "Cancel", None, false, true);
        if v.width.saturating_sub(x as u32) > 540 {
            f::label(
                v,
                R::new(
                    x + 224,
                    72,
                    v.width.saturating_sub(x as u32 + 240),
                    presentation::tool_h(),
                ),
                "Drag a crop area. Enter to apply; Esc to cancel.",
                TextStyle::Regular14,
                MUTED,
            );
        }
        if let Some((x0, y0, x1, y1)) = self.crop {
            let (x, y, w, h) = (
                x0.min(x1),
                y0.min(y1),
                (x0 - x1).unsigned_abs() + 1,
                (y0 - y1).unsigned_abs() + 1,
            );
            v.rect(x, y, w, 1, ACCENT);
            v.rect(x, y + h as i32 - 1, w, 1, ACCENT);
            v.rect(x, y, 1, h, ACCENT);
            v.rect(x + w as i32 - 1, y, 1, h, ACCENT);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crop_uses_actual_displayed_image_coordinates_and_undo_keeps_original_pixels() {
        let mut app = Preview::new(1001);
        app.configure(1472, 952);
        let image = codec::decode_full(b"P3 2 2 255 255 0 0 0 255 0 0 0 255 255 255 255").unwrap();
        let original = image.bgra.clone();
        app.tabs.push(Tab {
            id: 1,
            zoom: 100,
            fit: true,
            path: "photo.ppm".into(),
            info: FileInfo {
                attributes: 0,
                object: 7,
                version: 8,
                bytes: 45,
                directory: false,
            },
            image: Some(image),
            backup: None,
            text: String::new(),
            error: String::new(),
            scroll: 0,
            details: Vec::new(),
            raw: Vec::new(),
            audio: None,
        });
        let rotate = 19 | (door_abi::graphics::CONTROL << 9);
        let undo = 44 | (door_abi::graphics::CONTROL << 9);
        assert!(app.image_shortcut(rotate));
        assert_ne!(
            app.selected().unwrap().image.as_ref().unwrap().bgra,
            original
        );
        assert!(app.image_shortcut(undo));
        assert_eq!(
            app.selected().unwrap().image.as_ref().unwrap().bgra,
            original
        );
        app.opening = true;
        assert!(!app.image_shortcut(rotate));
        assert!(!app.image_shortcut(undo));
        app.opening = false;
        app.notice_open = true;
        assert!(!app.image_shortcut(undo));
        app.notice_open = false;
        app.tabs[0].path = "wave.wav".into();
        assert!(app.image_shortcut(rotate));
        assert_eq!(
            app.selected().unwrap().image.as_ref().unwrap().bgra,
            original
        );
        app.tabs[0].path = "photo.ppm".into();
        app.cropping = true;
        let (x, y, w, h) = app.image_box().unwrap();
        assert!(!app.crop_pointer(x - 1, y, true));
        assert!(app.crop_pointer(x, y, true));
        app.crop_pointer(x + w as i32 / 2 - 1, y + h as i32 / 2 - 1, false);
        app.crop_drag = false;
        assert!(app.apply_crop());
        let selected = app.selected().unwrap();
        let cropped = selected.image.as_ref().unwrap();
        assert_eq!((cropped.width, cropped.height), (1, 1));
        assert_eq!(cropped.bgra, [0, 0, 255, 255]);
        assert_eq!(selected.info.version, 8);
        app.undo_image();
        assert_eq!(
            app.selected().unwrap().image.as_ref().unwrap().bgra,
            original
        );
        app.undo_image();
        assert_eq!(app.selected().unwrap().image.as_ref().unwrap().width, 1);
        app.cropping = true;
        app.crop = Some((x, y, x, y));
        app.cancel_crop();
        assert_eq!(app.selected().unwrap().image.as_ref().unwrap().width, 1);
    }
}
