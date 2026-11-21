//! Glyph runs: outlines placed along a run (GetGlyphRunOutline) and their
//! coverage in device pixels (IDWriteGlyphRunAnalysis). Outlines are
//! unhinted (DirectWrite's natural symmetric rendering), at the run's em
//! size and variation location, with oblique and bold simulations.
use crate::font::{Face, OBLIQUE_SHEAR, SIMULATION_BOLD, SIMULATION_OBLIQUE};
use ab_glyph_rasterizer::{Rasterizer, point};
use alloc::vec::Vec;
#[allow(unused_imports)] // std builds (host tests) have these inherently.
use core_maths::CoreFloat;
use skrifa::{
    GlyphId, MetadataProvider,
    instance::Size,
    outline::{DrawSettings, OutlinePen},
};

/// A path command in run space (y down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    Move(f32, f32),
    Line(f32, f32),
    Quad(f32, f32, f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}
/// One positioned glyph: its id, advance and offsets (DWRITE_GLYPH_OFFSET).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Glyph {
    pub id: u16,
    pub advance: f32,
    pub advance_offset: f32,
    pub ascender_offset: f32,
}
/// DWRITE_MATRIX: x' = m11 x + m21 y + dx, y' = m12 x + m22 y + dy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix {
    pub m11: f32,
    pub m12: f32,
    pub m21: f32,
    pub m22: f32,
    pub dx: f32,
    pub dy: f32,
}
impl Matrix {
    pub const IDENTITY: Self = Self {
        m11: 1.0,
        m12: 0.0,
        m21: 0.0,
        m22: 1.0,
        dx: 0.0,
        dy: 0.0,
    };
    pub fn scale(self, s: f32) -> Self {
        Self {
            m11: self.m11 * s,
            m12: self.m12 * s,
            m21: self.m21 * s,
            m22: self.m22 * s,
            dx: self.dx * s,
            dy: self.dy * s,
        }
    }
    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.m11 * x + self.m21 * y + self.dx,
            self.m12 * x + self.m22 * y + self.dy,
        )
    }
}

/// Collects a glyph outline, y flipped to y-down, at `origin`.
struct Pen<'a> {
    out: &'a mut Vec<Command>,
    x: f32,
    y: f32,
    shear: f32,
}
impl Pen<'_> {
    fn p(&self, x: f32, y: f32) -> (f32, f32) {
        (self.x + x + y * self.shear, self.y - y)
    }
}
impl OutlinePen for Pen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let (a, b) = self.p(x, y);
        self.out.push(Command::Move(a, b));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (a, b) = self.p(x, y);
        self.out.push(Command::Line(a, b));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let (a, b) = self.p(cx, cy);
        let (c, d) = self.p(x, y);
        self.out.push(Command::Quad(a, b, c, d));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let (a, b) = self.p(cx0, cy0);
        let (c, d) = self.p(cx1, cy1);
        let (e, f) = self.p(x, y);
        self.out.push(Command::Cubic(a, b, c, d, e, f));
    }
    fn close(&mut self) {
        self.out.push(Command::Close);
    }
}

/// The outline of a run at `em` (DIPs), from the baseline origin, in run
/// space; right-to-left runs advance leftwards.
pub fn run_outline(face: &Face, em: f32, glyphs: &[Glyph], rtl: bool) -> Vec<Command> {
    let mut out = Vec::new();
    let Some(font) = face.font() else { return out };
    let outlines = font.outline_glyphs();
    let shear = if face.simulations & SIMULATION_OBLIQUE != 0 {
        OBLIQUE_SHEAR
    } else {
        0.0
    };
    let units = f32::from(face.metrics().units_per_em.max(1));
    let mut pen_x = 0.0f32;
    for g in glyphs {
        let x = if rtl {
            pen_x - g.advance - g.advance_offset
        } else {
            pen_x + g.advance_offset
        };
        if let Some(glyph) = outlines.get(GlyphId::new(u32::from(g.id))) {
            let start = out.len();
            let mut pen = Pen {
                out: &mut out,
                x,
                y: -g.ascender_offset,
                shear,
            };
            let settings = DrawSettings::unhinted(Size::new(em), face.location());
            if glyph.draw(settings, &mut pen).is_err() {
                out.truncate(start);
            } else if face.simulations & SIMULATION_BOLD != 0 {
                // Bold simulation: the glyph drawn again, shifted right by
                // the extra advance (a horizontal embolden).
                let shift =
                    em * crate::font::bold_extra(face.metrics().units_per_em) as f32 / units;
                let copy: Vec<Command> = out[start..]
                    .iter()
                    .map(|c| shift_command(*c, shift))
                    .collect();
                out.extend(copy);
            }
        }
        if rtl {
            pen_x -= g.advance;
        } else {
            pen_x += g.advance;
        }
    }
    out
}
fn shift_command(c: Command, dx: f32) -> Command {
    match c {
        Command::Move(x, y) => Command::Move(x + dx, y),
        Command::Line(x, y) => Command::Line(x + dx, y),
        Command::Quad(a, b, x, y) => Command::Quad(a + dx, b, x + dx, y),
        Command::Cubic(a, b, c, d, x, y) => Command::Cubic(a + dx, b, c + dx, d, x + dx, y),
        Command::Close => Command::Close,
    }
}

/// The commands mapped to device space by `m` from `origin`.
pub fn transform(commands: &[Command], m: &Matrix, origin: (f32, f32)) -> Vec<Command> {
    let t = |x: f32, y: f32| m.apply(x + origin.0, y + origin.1);
    commands
        .iter()
        .map(|c| match *c {
            Command::Move(x, y) => {
                let (a, b) = t(x, y);
                Command::Move(a, b)
            }
            Command::Line(x, y) => {
                let (a, b) = t(x, y);
                Command::Line(a, b)
            }
            Command::Quad(cx, cy, x, y) => {
                let (a, b) = t(cx, cy);
                let (c, d) = t(x, y);
                Command::Quad(a, b, c, d)
            }
            Command::Cubic(c0, c1, c2, c3, x, y) => {
                let (a, b) = t(c0, c1);
                let (c, d) = t(c2, c3);
                let (e, f) = t(x, y);
                Command::Cubic(a, b, c, d, e, f)
            }
            Command::Close => Command::Close,
        })
        .collect()
}
/// The pixel bounds (left, top, right, bottom) of device-space commands;
/// empty for no ink.
pub fn bounds(commands: &[Command]) -> [i32; 4] {
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    let mut add = |x: f32, y: f32| {
        b[0] = b[0].min(x);
        b[1] = b[1].min(y);
        b[2] = b[2].max(x);
        b[3] = b[3].max(y);
    };
    for c in commands {
        match *c {
            Command::Move(x, y) | Command::Line(x, y) => add(x, y),
            Command::Quad(a, b, x, y) => {
                add(a, b);
                add(x, y);
            }
            Command::Cubic(a, b, c, d, x, y) => {
                add(a, b);
                add(c, d);
                add(x, y);
            }
            Command::Close => (),
        }
    }
    if b[0] > b[2]
        || !(b[0].is_finite() && b[3].is_finite())
        || b[2] - b[0] > 16384.0
        || b[3] - b[1] > 16384.0
    {
        return [0; 4];
    }
    let r = [
        b[0].floor() as i32,
        b[1].floor() as i32,
        b[2].ceil() as i32,
        b[3].ceil() as i32,
    ];
    if r[0] == r[2] || r[1] == r[3] {
        [0; 4]
    } else {
        r
    }
}
/// Coverage (0..=255, one byte per pixel) of `commands` inside `rect`;
/// `aliased` thresholds it to 0 or 255.
pub fn coverage(commands: &[Command], rect: [i32; 4], aliased: bool) -> Vec<u8> {
    let width = (rect[2] - rect[0]).max(0) as usize;
    let height = (rect[3] - rect[1]).max(0) as usize;
    let mut out = alloc::vec![0u8; width * height];
    if width == 0 || height == 0 {
        return out;
    }
    let mut raster = Rasterizer::new(width, height);
    let (ox, oy) = (rect[0] as f32, rect[1] as f32);
    let p = |x: f32, y: f32| point(x - ox, y - oy);
    let mut start = (0.0, 0.0);
    let mut current = (0.0, 0.0);
    let mut open = false;
    for c in commands {
        match *c {
            Command::Move(x, y) => {
                if open && current != start {
                    raster.draw_line(p(current.0, current.1), p(start.0, start.1));
                }
                start = (x, y);
                current = start;
                open = true;
            }
            Command::Line(x, y) => {
                raster.draw_line(p(current.0, current.1), p(x, y));
                current = (x, y);
            }
            Command::Quad(a, b, x, y) => {
                raster.draw_quad(p(current.0, current.1), p(a, b), p(x, y));
                current = (x, y);
            }
            Command::Cubic(a, b, c, d, x, y) => {
                raster.draw_cubic(p(current.0, current.1), p(a, b), p(c, d), p(x, y));
                current = (x, y);
            }
            Command::Close => {
                if current != start {
                    raster.draw_line(p(current.0, current.1), p(start.0, start.1));
                }
                current = start;
                open = false;
            }
        }
    }
    if open && current != start {
        raster.draw_line(p(current.0, current.1), p(start.0, start.1));
    }
    raster.for_each_pixel(|index, alpha| {
        let a = alpha.clamp(0.0, 1.0);
        out[index] = if aliased {
            if a >= 0.5 { 255 } else { 0 }
        } else {
            (a * 255.0 + 0.5) as u8
        };
    });
    out
}
