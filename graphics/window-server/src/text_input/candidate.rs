//! Windows 11 IME candidate window, drawn by the window server as a shell
//! overlay at the active context's caret: horizontal for Pinyin (numbered
//! candidates with page chevrons), vertical for Japanese (numbered rows with
//! the selection pill and a page counter). Geometry follows the output scale
//! (fractional), colours follow the shell palette including contrast themes
//! (`paint::Palette::new`), and text is shaped and drawn by `TextSystem`
//! (CJK through its font fallback). The window flips above the caret when it
//! would leave the work area.
extern crate alloc;
use crate::paint;
use alloc::{format, string::String, vec::Vec};
use door_graphics_text::{Clip, TextSystem};
use door_service_protocol::{text_input as ti, window::Rect};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateWindow {
    pub serial: u32,
    pub items: Vec<String>,
    pub selected: usize,
    pub page: u8,
    pub pages: u8,
    pub vertical: bool,
    /// Caret rectangle in output physical pixels.
    pub anchor: Rect,
    /// Output scale percent.
    pub scale: u32,
}
/// What a text run is (it selects size and colour).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// "1".."9" (12 px).
    Number,
    /// A candidate (16 px).
    Candidate,
    /// "3/12" page counter (12 px).
    Page,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub x: i32,
    pub top: i32,
    pub text: String,
    pub role: Role,
    pub selected: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// Window frame in output physical pixels.
    pub frame: Rect,
    /// Per item: its highlight cell.
    pub cells: Vec<Rect>,
    pub runs: Vec<Run>,
    /// Page chevrons (horizontal): previous and next button cells.
    pub chevrons: Option<(Rect, Rect)>,
    pub radius: u32,
}
/// Logical metrics at 100% (Windows 11 candidate UI).
const H_HEIGHT: u32 = 36;
const H_PAD: u32 = 4;
const CELL_PAD: u32 = 8;
const NUMBER_GAP: u32 = 4;
const CHEVRON: u32 = 24;
const V_ROW: u32 = 32;
const V_MIN_WIDTH: u32 = 168;
const V_NUMBER: u32 = 28;
const V_FOOTER: u32 = 24;
const RADIUS: u32 = 8;
const GAP: u32 = 4;
pub const NUMBER_PX: u32 = 12;
pub const CANDIDATE_PX: u32 = 16;
/// Premultiplied BGRA colours, from the shell palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub background: [u8; 4],
    pub border: [u8; 4],
    pub text: [u8; 4],
    pub secondary: [u8; 4],
    pub selection: [u8; 4],
    pub selection_text: [u8; 4],
    pub accent: [u8; 4],
    /// Flyout elevation shadow alpha per layer (0 in contrast themes,
    /// which draw no shadows).
    pub shadow: u8,
}
impl Palette {
    /// Flyout colours; a contrast theme uses Highlight/HighlightText for the
    /// selected candidate (Windows forced colours).
    pub fn from_shell(p: &paint::Palette) -> Self {
        Self {
            background: p.menu,
            border: p.menu_border,
            text: p.text,
            secondary: p.text_secondary,
            selection: if p.contrast { p.accent } else { p.subtle_hover },
            selection_text: if p.contrast { p.accent_text } else { p.text },
            accent: if p.contrast { p.accent_text } else { p.accent },
            shadow: if p.contrast {
                0
            } else if p.dark {
                12
            } else {
                6
            },
        }
    }
    pub fn current(dark: bool) -> Self {
        Self::from_shell(&paint::Palette::new(dark))
    }
}
fn px(logical: u32, scale: u32) -> u32 {
    (logical * scale.max(100) + 50) / 100
}
/// Accessibility > Text size: candidate and number font sizes, rows and
/// the number column grow with it (identity at 100%); padding, chevrons,
/// radius and gaps keep their size.
fn text_px(logical: u32) -> u32 {
    paint::text_len(logical)
}
fn text_row(base: u32, style: door_graphics_text::TextStyle) -> u32 {
    paint::text_grow(base, style)
}
impl CandidateWindow {
    pub fn new(c: ti::Candidates<'_>, items: &[&str], anchor: Rect, scale: u32) -> Self {
        Self {
            serial: c.serial,
            items: items.iter().map(|s| String::from(*s)).collect(),
            selected: c.selected as usize,
            page: c.page,
            pages: c.pages,
            vertical: c.flags & ti::CANDIDATES_VERTICAL != 0,
            anchor,
            scale: scale.clamp(100, 300),
        }
    }
    /// Place the window at the caret inside `work`; `measure(text, px)`
    /// returns a run's advance in physical pixels at that logical size.
    pub fn layout(&self, work: Rect, measure: &dyn Fn(&str, u32) -> u32) -> Layout {
        let s = self.scale;
        use door_graphics_text::TextStyle::{Regular12, Regular16};
        let (num_size, cand_size) = (text_px(NUMBER_PX), text_px(CANDIDATE_PX));
        let (num_px, cand_px) = (px(num_size, s), px(cand_size, s));
        let (v_row, h_height) = (text_row(V_ROW, Regular16), text_row(H_HEIGHT, Regular16));
        let (v_number, v_footer) = (text_px(V_NUMBER), text_row(V_FOOTER, Regular12));
        let mut cells = Vec::new();
        let mut runs = Vec::new();
        let mut chevrons = None;
        let (width, height);
        if self.vertical {
            let row = px(v_row, s);
            let widest = self
                .items
                .iter()
                .map(|t| measure(t, cand_size))
                .max()
                .unwrap_or(0);
            width = px(V_MIN_WIDTH, s).max(px(H_PAD + v_number + CELL_PAD, s) + widest);
            height = px(H_PAD * 2 + v_footer, s) + row * self.items.len() as u32;
            for (i, text) in self.items.iter().enumerate() {
                let top = (px(H_PAD, s) + row * i as u32) as i32;
                cells.push(Rect {
                    x: px(H_PAD, s) as i32,
                    y: top,
                    width: width - px(H_PAD * 2, s),
                    height: row,
                });
                let selected = i == self.selected;
                runs.push(Run {
                    x: px(H_PAD + CELL_PAD, s) as i32,
                    top: top + (row.saturating_sub(num_px) / 2) as i32,
                    text: format!("{}", i + 1),
                    role: Role::Number,
                    selected,
                });
                runs.push(Run {
                    x: px(H_PAD + v_number, s) as i32,
                    top: top + (row.saturating_sub(cand_px) / 2) as i32,
                    text: text.clone(),
                    role: Role::Candidate,
                    selected,
                });
            }
            let label = format!("{}/{}", self.page as u32 + 1, self.pages.max(1));
            let lw = measure(&label, num_size);
            let footer = px(v_footer, s);
            runs.push(Run {
                x: width as i32 - (px(CELL_PAD, s) + lw) as i32,
                top: (height - px(H_PAD, s) - footer + footer.saturating_sub(num_px) / 2) as i32,
                text: label,
                role: Role::Page,
                selected: false,
            });
        } else {
            height = px(h_height, s);
            let mut x = px(H_PAD, s) as i32;
            for (i, text) in self.items.iter().enumerate() {
                let number = format!("{}", i + 1);
                let nw = measure(&number, num_size);
                let tw = measure(text, cand_size);
                let cell_w = px(CELL_PAD * 2 + NUMBER_GAP, s) + nw + tw;
                let selected = i == self.selected;
                runs.push(Run {
                    x: x + px(CELL_PAD, s) as i32,
                    top: (height.saturating_sub(num_px) / 2) as i32,
                    text: number,
                    role: Role::Number,
                    selected,
                });
                runs.push(Run {
                    x: x + (px(CELL_PAD + NUMBER_GAP, s) + nw) as i32,
                    top: (height.saturating_sub(cand_px) / 2) as i32,
                    text: text.clone(),
                    role: Role::Candidate,
                    selected,
                });
                cells.push(Rect {
                    x,
                    y: px(H_PAD, s) as i32,
                    width: cell_w,
                    height: height - px(H_PAD * 2, s),
                });
                x += cell_w as i32;
            }
            if self.pages > 1 {
                let c = px(CHEVRON, s);
                let (y, h) = (px(H_PAD, s) as i32, height - px(H_PAD * 2, s));
                chevrons = Some((
                    Rect {
                        x,
                        y,
                        width: c,
                        height: h,
                    },
                    Rect {
                        x: x + c as i32,
                        y,
                        width: c,
                        height: h,
                    },
                ));
                x += 2 * c as i32;
            }
            width = x as u32 + px(H_PAD, s);
        }
        // Below the caret, the first candidate's text under the caret; above
        // it when the work area ends first.
        let gap = px(GAP, s) as i32;
        let text_offset = if self.vertical {
            px(H_PAD + v_number, s)
        } else {
            px(H_PAD + CELL_PAD, s)
        };
        let below = self.anchor.y + self.anchor.height as i32 + gap;
        let (work_bottom, work_right) = (work.y + work.height as i32, work.x + work.width as i32);
        let y = if below + height as i32 <= work_bottom
            || self.anchor.y - gap - (height as i32) < work.y
        {
            below.min(work_bottom - height as i32)
        } else {
            self.anchor.y - gap - height as i32
        };
        let x = (self.anchor.x - text_offset as i32)
            .min(work_right - width as i32)
            .max(work.x);
        let frame = Rect {
            x,
            y: y.max(work.y),
            width,
            height,
        };
        for r in &mut runs {
            r.x += frame.x;
            r.top += frame.y;
        }
        let shift = |r: &mut Rect| {
            r.x += frame.x;
            r.y += frame.y;
        };
        cells.iter_mut().for_each(shift);
        if let Some((a, b)) = chevrons.as_mut() {
            shift(a);
            shift(b);
        }
        Layout {
            frame,
            cells,
            runs,
            chevrons,
            radius: px(RADIUS, s),
        }
    }
}
/// A premultiplied BGRA target covering `area` of the output.
pub struct Target<'a> {
    pub pixels: &'a mut [u8],
    pub stride: usize,
    pub area: Rect,
}
impl Target<'_> {
    fn blend(&mut self, x: i32, y: i32, color: [u8; 4], coverage: u32) {
        let (lx, ly) = (x - self.area.x, y - self.area.y);
        if lx < 0
            || ly < 0
            || lx >= self.area.width as i32
            || ly >= self.area.height as i32
            || coverage == 0
        {
            return;
        }
        let at = ly as usize * self.stride + lx as usize * 4;
        let Some(p) = self.pixels.get_mut(at..at + 4) else {
            return;
        };
        let a = (u32::from(color[3]) * coverage + 127) / 255;
        for i in 0..3 {
            let c = (u32::from(color[i]) * coverage + 127) / 255;
            p[i] = (c + u32::from(p[i]) * (255 - a) / 255).min(255) as u8;
        }
        p[3] = 255;
    }
    /// Fill a rounded rectangle with antialiased corners. The corner circle
    /// centres are pixel centres `rad` from each edge, so the radius is
    /// clamped to `(w-1)/2` and `(h-1)/2` (narrow pills at fractional
    /// scales, empty rectangles).
    pub fn rounded(&mut self, r: Rect, radius: u32, color: [u8; 4]) {
        let rad = radius
            .min(r.width.saturating_sub(1) / 2)
            .min(r.height.saturating_sub(1) / 2) as i32;
        let (right, bottom) = (r.x + r.width as i32, r.y + r.height as i32);
        for y in r.y..bottom {
            for x in r.x..right {
                let cx = x.clamp(r.x + rad, right - rad - 1);
                let cy = y.clamp(r.y + rad, bottom - rad - 1);
                let coverage = if cx == x || cy == y {
                    255
                } else {
                    // Distance (1/16 px) from the corner circle centre.
                    let (dx, dy) = ((x - cx) * 16, (y - cy) * 16);
                    let d = isqrt((dx * dx + dy * dy) as u32) as i32;
                    ((rad * 16 + 8 - d).clamp(0, 16) * 255 / 16) as u32
                };
                self.blend(x, y, color, coverage);
            }
        }
    }
    /// A chevron (‹ or ›) centred in `cell`.
    pub fn chevron(&mut self, cell: Rect, left: bool, scale: u32, color: [u8; 4]) {
        let h = px(4, scale) as i32;
        let (cx, cy) = (
            cell.x + cell.width as i32 / 2,
            cell.y + cell.height as i32 / 2,
        );
        for i in 0..=h {
            let dx = if left { (h - i) / 2 } else { (i - h) / 2 };
            for (x, y) in [(cx + dx, cy - h + i), (cx + dx, cy + h - i)] {
                for w in 0..px(1, scale) as i32 {
                    self.blend(x + w, y, color, 255);
                }
            }
        }
    }
}
fn isqrt(n: u32) -> u32 {
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}
/// Windows 11 flyout elevation under the frame: a soft shadow offset
/// downwards (none in contrast themes). Painted before `paint`.
pub fn shadow(
    window: &CandidateWindow,
    layout: &Layout,
    palette: &Palette,
    target: &mut Target<'_>,
) {
    if palette.shadow == 0 {
        return;
    }
    let f = layout.frame;
    let (spread, drop) = (px(8, window.scale), px(2, window.scale) as i32);
    for i in (1..=spread).rev() {
        let r = Rect {
            x: f.x - i as i32,
            y: f.y - i as i32 + drop,
            width: f.width + 2 * i,
            height: f.height + 2 * i,
        };
        target.rounded(r, layout.radius + i, [0, 0, 0, palette.shadow]);
    }
}
/// Paint the chrome (border, background, selection, chevrons).
pub fn paint(
    window: &CandidateWindow,
    layout: &Layout,
    palette: &Palette,
    target: &mut Target<'_>,
) {
    let f = layout.frame;
    target.rounded(f, layout.radius, palette.border);
    let one = px(1, window.scale);
    let inner = Rect {
        x: f.x + one as i32,
        y: f.y + one as i32,
        width: f.width - 2 * one,
        height: f.height - 2 * one,
    };
    target.rounded(inner, layout.radius.saturating_sub(one), palette.background);
    if let Some(cell) = layout.cells.get(window.selected) {
        target.rounded(*cell, px(4, window.scale), palette.selection);
        if window.vertical {
            // Windows 11 list selection indicator: a short accent pill.
            let bar = Rect {
                x: cell.x,
                y: cell.y + cell.height as i32 / 2 - px(8, window.scale) as i32,
                width: px(3, window.scale),
                height: px(16, window.scale),
            };
            target.rounded(bar, px(2, window.scale), palette.accent);
        }
    }
    if let Some((prev, next)) = layout.chevrons {
        let pick = |on: bool| if on { palette.text } else { palette.secondary };
        target.chevron(prev, true, window.scale, pick(window.page > 0));
        target.chevron(
            next,
            false,
            window.scale,
            pick(window.page + 1 < window.pages),
        );
    }
}
fn color(role: Role, selected: bool, p: &Palette) -> [u8; 4] {
    match (role, selected) {
        (Role::Number, true) => p.accent,
        (Role::Candidate, true) => p.selection_text,
        (Role::Candidate, false) => p.text,
        _ => p.secondary,
    }
}
fn size(role: Role) -> u32 {
    text_px(if role == Role::Candidate {
        CANDIDATE_PX
    } else {
        NUMBER_PX
    })
}
/// The window's layout with runs measured by `text` (the shell's damage
/// region uses the same frame `render` paints).
pub fn measure(window: &CandidateWindow, work: Rect, text: &TextSystem, locale: &str) -> Layout {
    let scale = window.scale as f32 / 100.0;
    let measure = |s: &str, size: u32| {
        text.layout_line(s, &text.format(size as f32, 400, scale, locale))
            .map_or(0, |l| l.physical_size().0)
    };
    window.layout(work, &measure)
}
/// Lay out, paint and draw the candidate window with `text` (the session
/// TextSystem; `locale` picks CJK fallback, e.g. "zh-CN"/"ja-JP").
pub fn render(
    window: &CandidateWindow,
    work: Rect,
    palette: &Palette,
    text: &TextSystem,
    locale: &str,
    target: &mut Target<'_>,
) -> Layout {
    let scale = window.scale as f32 / 100.0;
    let layout = measure(window, work, text, locale);
    shadow(window, &layout, palette, target);
    paint(window, &layout, palette, target);
    let area = target.area;
    if let Ok(mut t) =
        door_graphics_text::Target::new(target.pixels, area.width, area.height, target.stride)
    {
        let clip = Clip::new(
            layout.frame.x - area.x,
            layout.frame.y - area.y,
            layout.frame.width,
            layout.frame.height,
        );
        for run in &layout.runs {
            let format = text.format(size(run.role) as f32, 400, scale, locale);
            if let Ok(l) = text.layout_line(&run.text, &format) {
                let _ = text.draw(
                    &l,
                    &mut t,
                    clip,
                    run.x - area.x,
                    run.top - area.y,
                    color(run.role, run.selected, palette),
                );
            }
        }
    }
    layout
}
#[cfg(test)]
mod shadow_tests {
    use super::*;
    use door_service_protocol::desktop::ContrastTheme;
    #[test]
    fn the_elevation_shadow_hugs_the_frame_and_contrast_themes_draw_none() {
        let window = CandidateWindow {
            serial: 1,
            items: alloc::vec!["你".into(), "呢".into()],
            selected: 0,
            page: 0,
            pages: 1,
            vertical: false,
            anchor: Rect {
                x: 100,
                y: 40,
                width: 1,
                height: 18,
            },
            scale: 100,
        };
        let work = Rect {
            x: 0,
            y: 0,
            width: 400,
            height: 300,
        };
        let layout = window.layout(work, &|s: &str, px: u32| s.chars().count() as u32 * px);
        let f = layout.frame;
        for (palette, expect) in [
            (Palette::from_shell(&paint::Palette::themed(false)), true),
            (Palette::from_shell(&paint::Palette::themed(true)), true),
            (
                Palette::from_shell(&paint::Palette::contrast_theme(ContrastTheme::NightSky)),
                false,
            ),
        ] {
            let mut pixels = alloc::vec![0u8; 400 * 300 * 4];
            let mut t = Target {
                pixels: &mut pixels,
                stride: 1600,
                area: work,
            };
            shadow(&window, &layout, &palette, &mut t);
            let mut any = false;
            for (i, px) in pixels.chunks_exact(4).enumerate() {
                if px[3] != 0 {
                    let (x, y) = ((i % 400) as i32, (i / 400) as i32);
                    assert!(
                        x >= f.x - 8
                            && x < f.x + f.width as i32 + 8
                            && y >= f.y - 6
                            && y < f.y + f.height as i32 + 10
                    );
                    any = true;
                }
            }
            assert_eq!(any, expect);
        }
    }
}
