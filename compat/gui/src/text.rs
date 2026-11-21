//! GDI text through Door Text: `LOGFONTW` requests are mapped onto the
//! shared font set (Segoe UI Variable / Segoe UI for proportional faces,
//! Cascadia Mono for fixed pitch), measured with HarfRust shaping and drawn
//! with the hinted rasterizer. Metrics derive from the real layout of each
//! request; nothing assumes that native text metrics equal Microsoft's GDI
//! bitmap-font metrics (the face substitution is a declared difference).
use crate::gdi::{LogFont, TextBackend, TextMetrics};
use crate::raster::Canvas;
use crate::region::Region;
use crate::{Point, Rect, Result};
use alloc::string::String;
use alloc::vec::Vec;
use door_graphics_text::{Family, FontSet, Target, TextFormat, TextSystem};

pub struct DoorText {
    system: TextSystem,
}

/// Em size in pixels for a `lfHeight`: negative values are character
/// (em) heights, positive values cell heights, zero the 9 pt default.
pub fn em_size(height: i32) -> f32 {
    match height {
        0 => 12.0,
        h if h < 0 => (h.unsigned_abs().min(1000)) as f32,
        // Segoe UI's cell (ascent + descent) is about 1.33 em.
        h => (h.min(1000) as f32 / 1.33).max(1.0),
    }
}

/// Round half away from zero (no_std has no `f32::round`).
fn round(v: f32) -> i32 {
    if v >= 0.0 {
        (v + 0.5) as i32
    } else {
        (v - 0.5) as i32
    }
}

fn utf8(text: &[u16]) -> String {
    char::decode_utf16(text.iter().copied())
        .map(|c| c.unwrap_or('\u{fffd}'))
        .collect()
}

impl DoorText {
    pub fn new(fonts: FontSet) -> Self {
        Self {
            system: TextSystem::new(fonts),
        }
    }
    fn format(font: &LogFont) -> TextFormat {
        let weight = if font.weight == 0 { 400 } else { font.weight };
        TextFormat::new(em_size(font.height))
            .with_weight(weight.clamp(100, 900) as u16)
            .with_family(if font.fixed_pitch() {
                Family::Monospace
            } else {
                Family::Ui
            })
    }
}

impl TextBackend for DoorText {
    fn metrics(&self, font: &LogFont) -> TextMetrics {
        let format = Self::format(font);
        let (ascent, descent, line, _) = self.system.line_metrics(&format).unwrap_or((
            format.size,
            format.size / 4.0,
            format.size * 1.25,
            format.size,
        ));
        let ascent = round(ascent);
        let descent = round(descent);
        let height = (ascent + descent).max(round(line));
        let em = round(format.size);
        let average = self
            .advances(font, &[u16::from(b'x')])
            .first()
            .copied()
            .unwrap_or(1);
        let widest = self
            .advances(font, &[u16::from(b'W')])
            .first()
            .copied()
            .unwrap_or(1);
        TextMetrics {
            height,
            ascent,
            descent: height - ascent,
            internal_leading: (height - em).max(0),
            external_leading: 0,
            ave_char_width: average.max(1),
            max_char_width: widest.max(average).max(1),
            weight: if font.weight == 0 { 400 } else { font.weight },
            italic: font.italic,
            underline: font.underline,
            strike_out: font.strike_out,
            pitch_family: font.pitch_family,
            charset: font.charset,
        }
    }
    fn advances(&self, font: &LogFont, text: &[u16]) -> Vec<i32> {
        let mut out = alloc::vec![0i32; text.len()];
        if text.is_empty() {
            return out;
        }
        let s = utf8(text);
        let Ok(layout) = self.system.layout_line(&s, &Self::format(font)) else {
            return out;
        };
        // Cluster widths are spread over their UTF-16 units; integer
        // positions are rounded cumulatively so the sum matches the layout.
        let mut prev = 0f32;
        let mut prev_round = 0i32;
        for c in layout.clusters() {
            let start = door_graphics_text::utf8_to_utf16(&s, c.text.start);
            let end = door_graphics_text::utf8_to_utf16(&s, c.text.end).min(text.len());
            let right = prev + c.width;
            let rounded = round(right);
            if start < end {
                out[start] = rounded - prev_round;
            }
            prev = right;
            prev_round = rounded;
        }
        out
    }
    fn draw(
        &self,
        font: &LogFont,
        text: &[u16],
        canvas: &mut Canvas,
        origin: Point,
        color: u32,
        clip: &Region,
    ) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        let s = utf8(text);
        let format = Self::format(font);
        let layout = self
            .system
            .layout_line(&s, &format)
            .map_err(|_| crate::Error::InvalidParameter)?;
        let (w, h) = layout.physical_size();
        // Render into a private BGRA band covering the text box, seeded with
        // the destination, then copy back only the clipped pixels.
        let pad = (format.size as i32) / 2 + 2;
        // Saturating: the origin is any device coordinate. Once the area is
        // non-empty the origin lies within the text box of the canvas, so the
        // offsets below stay small.
        let edge = |v: u32| i32::try_from(v).unwrap_or(i32::MAX).saturating_add(2 * pad);
        let area = Rect::from_size(
            origin.x.saturating_sub(pad),
            origin.y.saturating_sub(pad),
            edge(w),
            edge(h),
        )
        .intersect(canvas.bounds());
        if area.is_empty() || !clip.intersects(area) {
            return Ok(());
        }
        let (aw, ah) = (area.width() as u32, area.height() as u32);
        let block = canvas.read(area)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(block.px.len() * 4)
            .map_err(|_| crate::Error::Quota)?;
        for p in &block.px {
            bytes.extend_from_slice(&(p | 0xff00_0000).to_le_bytes());
        }
        // GDI text: ClearType, as Windows draws Win32 apps.
        let mut target = Target::new(&mut bytes, aw, ah, aw as usize * 4)
            .map_err(|_| crate::Error::InvalidParameter)?
            .cleartype();
        let bounds = target.bounds();
        let rgba = [
            (color & 0xff) as u8,
            (color >> 8 & 0xff) as u8,
            (color >> 16 & 0xff) as u8,
            255,
        ];
        self.system
            .draw(
                &layout,
                &mut target,
                bounds,
                origin.x - area.left,
                origin.y - area.top,
                rgba,
            )
            .map_err(|_| crate::Error::InvalidParameter)?;
        if font.underline || font.strike_out {
            let baseline = round(layout.baseline());
            for (on, y) in [
                (font.underline, baseline + 1),
                (font.strike_out, baseline * 2 / 3),
            ] {
                let row = origin.y - area.top + y;
                if on && row >= 0 && (row as u32) < ah {
                    for x in (origin.x - area.left).max(0)
                        ..(origin.x - area.left + w as i32).min(aw as i32)
                    {
                        let at = (row as usize * aw as usize + x as usize) * 4;
                        bytes[at..at + 4].copy_from_slice(&rgba);
                    }
                }
            }
        }
        for y in 0..ah as i32 {
            for x in 0..aw as i32 {
                let p = Point::new(area.left + x, area.top + y);
                if clip.contains(p) {
                    let at = (y as usize * aw as usize + x as usize) * 4;
                    let v = u32::from(bytes[at + 2]) << 16
                        | u32::from(bytes[at + 1]) << 8
                        | u32::from(bytes[at]);
                    canvas.put(p.x, p.y, v);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use door_graphics_text::FontRole;
    extern crate std;
    fn system() -> Option<DoorText> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/fonts/segoe-ui/SegUIVar.ttf");
        let bytes: &'static [u8] = std::fs::read(path).ok()?.leak();
        FontSet::from_files(&[(bytes, FontRole::UiVariable)])
            .ok()
            .map(DoorText::new)
    }
    #[test]
    fn em_sizes_follow_logfont_rules() {
        assert_eq!(em_size(-12), 12.0);
        assert_eq!(em_size(0), 12.0);
        assert!((em_size(16) - 12.03).abs() < 0.1);
        assert_eq!(em_size(i32::MIN), 1000.0);
    }
    #[test]
    fn real_font_metrics_extents_and_drawing() {
        let Some(t) = system() else {
            std::eprintln!("font asset unavailable; skipped");
            return;
        };
        let font = LogFont::named(-12, 400, false);
        let m = t.metrics(&font);
        assert!(m.height >= 12 && m.ascent > m.descent && m.ave_char_width > 0);
        assert_eq!(m.height, m.ascent + m.descent);
        let text: Vec<u16> = "Doors \u{3bb}".encode_utf16().collect();
        let adv = t.advances(&font, &text);
        assert_eq!(adv.len(), text.len());
        assert!(adv.iter().sum::<i32>() > 30);
        let bold = t.advances(&LogFont::named(-24, 700, false), &text);
        assert!(bold.iter().sum::<i32>() > adv.iter().sum::<i32>());
        let mut px = alloc::vec![0x00ff_ffffu32; 64 * 24];
        let mut c = Canvas::new(&mut px, 64, 24, 64).unwrap();
        let all = Region::from_rect(Rect::new(0, 0, 64, 24));
        t.draw(&font, &text, &mut c, Point::new(2, 2), 0, &all)
            .unwrap();
        assert!(px.iter().any(|p| *p != 0x00ff_ffff), "ink drawn");
        let mut px2 = alloc::vec![0x00ff_ffffu32; 64 * 24];
        let mut c = Canvas::new(&mut px2, 64, 24, 64).unwrap();
        let none = Region::from_rect(Rect::new(0, 0, 1, 1));
        t.draw(&font, &text, &mut c, Point::new(2, 2), 0, &none)
            .unwrap();
        assert!(px2.iter().all(|p| *p == 0x00ff_ffff), "clipped away");
    }
}
