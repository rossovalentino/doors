//! Original ContentDialog-style renderer. Text and targets use logical units;
//! glyphs still come from the density-specific hinted atlas.
use super::*;
use door_workspace::dialog::{Dialog, Layout};
impl View<'_> {
    pub fn dialog(&mut self, dialog: &Dialog, title: &str, body: &str, labels: &[&str]) {
        if !dialog.open() {
            return;
        }
        self.reset_viewport();
        // Dim the existing opaque client once. Text in the panel is painted on
        // a fresh opaque surface so it can use RGB subpixel coverage.
        let clip = self.physical_clip();
        if let Ok(mut canvas) = door_graphics_core::Canvas::new(
            self.pixels,
            self.physical_width,
            self.physical_height,
            self.physical_width as usize * 4,
        ) {
            let _ = canvas.fill_rect(clip, [0, 0, 0, 80]);
        }
        let l = Layout::new(self.width, self.height, dialog.count());
        // The current toolkit fonts cover ASCII only. Convert by character
        // before byte-based wrapping so arbitrary UTF-8 never splits a scalar.
        let title = display_ascii(title);
        let body = display_ascii(body);
        let compact = l.panel.width < 336;
        let p = l.panel;
        self.rounded(p.x, p.y, p.width, p.height, 8, [190, 190, 190, 255]);
        self.rounded(
            p.x + 1,
            p.y + 1,
            p.width.saturating_sub(2),
            p.height.saturating_sub(2),
            7,
            [255; 4],
        );
        if let Ok(clip) =
            door_graphics_core::Rect::new(l.text.x, l.text.y, l.text.width, l.text.height)
        {
            self.viewport(clip, 0);
            let mut y = l.text.y;
            for (text, style, gap) in [
                (
                    title.as_str(),
                    if compact {
                        TextStyle::Semibold14
                    } else {
                        TextStyle::Semibold20
                    },
                    if compact { 8 } else { 12 },
                ),
                (
                    body.as_str(),
                    if compact {
                        TextStyle::Regular12
                    } else {
                        TextStyle::Regular14
                    },
                    0,
                ),
            ] {
                let lines = door_workspace::dialog::lines(text, l.text.width, |s| {
                    self.fonts.measure(s, style).map_or(0, |m| m.advance_width)
                });
                for line in lines {
                    self.text(l.text.x, y, &text[line], style, [32, 32, 32, 255]);
                    y += self.fonts.line_height(style) as i32;
                }
                y += gap;
            }
            self.reset_viewport();
        }
        for (i, label) in labels.iter().enumerate().take(dialog.count()) {
            let b = l.buttons[i];
            let hover = self.hover.is_some_and(|(x, y)| b.contains(x, y));
            let down = hover && self.pressed.is_some_and(|(x, y)| b.contains(x, y));
            let accent = i == 0;
            let bg = if accent {
                if down {
                    [160, 80, 0, 255]
                } else if hover {
                    [180, 94, 0, 255]
                } else {
                    [196, 103, 0, 255]
                }
            } else if down {
                [230, 230, 230, 255]
            } else if hover {
                [243, 243, 243, 255]
            } else {
                [249, 249, 249, 255]
            };
            self.rounded_asset(
                b.x,
                b.y,
                b.width,
                b.height,
                4,
                if dialog.focus() == Some(i) {
                    self.palette.foreground([32, 32, 32, 255])
                } else {
                    self.palette.surface([190, 190, 190, 255])
                },
            );
            self.rounded(
                b.x + 1,
                b.y + 1,
                b.width.saturating_sub(2),
                b.height.saturating_sub(2),
                3,
                bg,
            );
            if dialog.focus() == Some(i) {
                self.rounded_asset(
                    b.x + 2,
                    b.y + 2,
                    b.width.saturating_sub(4),
                    b.height.saturating_sub(4),
                    2,
                    if accent {
                        [255; 4]
                    } else {
                        self.palette.foreground([32, 32, 32, 255])
                    },
                );
                self.rounded(
                    b.x + 3,
                    b.y + 3,
                    b.width.saturating_sub(6),
                    b.height.saturating_sub(6),
                    1,
                    bg,
                );
            }
            let style = if compact {
                TextStyle::Regular12
            } else {
                TextStyle::Regular14
            };
            self.text_in_box(
                (b.x + 4, b.y, b.width.saturating_sub(8), b.height),
                label,
                style,
                if accent { [255; 4] } else { [32, 32, 32, 255] },
            );
        }
    }
}
