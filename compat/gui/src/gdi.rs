//! The GDI object and device-context model: generational `HGDIOBJ`/`HDC`
//! tables, stock objects, selection ownership, DC state with `SaveDC`/
//! `RestoreDC`, mapping, clipping and the drawing operations of gdi32.
//!
//! Pixels live either in owned surfaces (compatible bitmaps) or in memory the
//! DLL owns outside this crate (32-bpp DIB sections, window back buffers);
//! the latter are reached through `External`, keyed by an opaque value, so
//! this crate never holds a pointer. Text goes through `TextBackend`.
use crate::handles::{Handle, Table, kind};
use crate::msg::{bk, brush as bs, hs, mm, obj, ps, r2, rop, stock, ta};
use crate::raster::{self, Block, Canvas, Fill};
use crate::region::{Op, Region};
use crate::surface::{Surface, convert_depth};
use crate::units::Mapping;
use crate::{ColorRef, Error, Point, Rect, Result, Size};
use alloc::vec::Vec;

pub const MAX_OBJECTS: usize = 10_000;
pub const MAX_DCS: usize = 512;
pub const MAX_SAVED: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pen {
    pub style: u32,
    pub width: i32,
    pub color: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Brush {
    Solid(u32),
    Null,
    Hatch(u32, u32),
    /// Pixels captured from the pattern bitmap at creation. A monochrome
    /// pattern paints its 0 bits in the DC's text color and its 1 bits in the
    /// DC's background color when it is used.
    Pattern {
        width: u32,
        height: u32,
        px: Vec<u32>,
        mono: bool,
    },
    /// DC_BRUSH: the DC's brush color.
    DcBrush,
}

/// The fields of `LOGFONTW` that affect rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogFont {
    pub height: i32,
    pub width: i32,
    pub escapement: i32,
    pub orientation: i32,
    pub weight: i32,
    pub italic: bool,
    pub underline: bool,
    pub strike_out: bool,
    pub charset: u8,
    pub pitch_family: u8,
    pub face: [u16; 32],
}
impl LogFont {
    pub const fn named(height: i32, weight: i32, fixed: bool) -> Self {
        Self {
            height,
            width: 0,
            escapement: 0,
            orientation: 0,
            weight,
            italic: false,
            underline: false,
            strike_out: false,
            charset: 1,
            pitch_family: if fixed { 0x31 } else { 0x22 },
            face: [0; 32],
        }
    }
    pub fn fixed_pitch(&self) -> bool {
        self.pitch_family & 3 == 1
    }
    /// Decode the 92-byte `LOGFONTW`.
    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        if b.len() < 92 {
            return Err(Error::InvalidParameter);
        }
        let i = |at: usize| i32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
        let mut face = [0u16; 32];
        for (n, f) in face.iter_mut().enumerate() {
            *f = u16::from_le_bytes([b[28 + 2 * n], b[29 + 2 * n]]);
        }
        face[31] = 0;
        Ok(Self {
            height: i(0),
            width: i(4),
            escapement: i(8),
            orientation: i(12),
            weight: i(16).clamp(0, 1000),
            italic: b[20] != 0,
            underline: b[21] != 0,
            strike_out: b[22] != 0,
            charset: b[23],
            pitch_family: b[27],
            face,
        })
    }
    pub fn to_bytes(&self) -> [u8; 92] {
        let mut b = [0u8; 92];
        for (at, v) in [
            (0, self.height),
            (4, self.width),
            (8, self.escapement),
            (12, self.orientation),
            (16, self.weight),
        ] {
            b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        b[20] = self.italic as u8;
        b[21] = self.underline as u8;
        b[22] = self.strike_out as u8;
        b[23] = self.charset;
        b[27] = self.pitch_family;
        for (n, f) in self.face.iter().enumerate() {
            b[28 + 2 * n..30 + 2 * n].copy_from_slice(&f.to_le_bytes());
        }
        b
    }
}

/// `TEXTMETRICW` values in device pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextMetrics {
    pub height: i32,
    pub ascent: i32,
    pub descent: i32,
    pub internal_leading: i32,
    pub external_leading: i32,
    pub ave_char_width: i32,
    pub max_char_width: i32,
    pub weight: i32,
    pub italic: bool,
    pub underline: bool,
    pub strike_out: bool,
    pub pitch_family: u8,
    pub charset: u8,
}
impl TextMetrics {
    /// The 60-byte `TEXTMETRICW` (x64 packing).
    pub fn to_bytes(&self) -> [u8; 60] {
        let mut b = [0u8; 60];
        for (at, v) in [
            (0, self.height),
            (4, self.ascent),
            (8, self.descent),
            (12, self.internal_leading),
            (16, self.external_leading),
            (20, self.ave_char_width),
            (24, self.max_char_width),
            (28, self.weight),
            (32, 0),
            (36, 96),
            (40, 96),
        ] {
            b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        // tmFirstChar, tmLastChar, tmDefaultChar, tmBreakChar (WCHAR).
        for (at, v) in [(44u16, 0x20u16), (46, 0xfffc), (48, 0x1f), (50, 0x20)] {
            b[at as usize..at as usize + 2].copy_from_slice(&v.to_le_bytes());
        }
        b[52] = self.italic as u8;
        b[53] = self.underline as u8;
        b[54] = self.strike_out as u8;
        // TMPF_VECTOR | TMPF_TRUETYPE, and TMPF_FIXED_PITCH means variable.
        let fixed = self.pitch_family & 3 == 1;
        b[55] = (self.pitch_family & 0xf0) | 0x06 | if fixed { 0 } else { 1 };
        b[56] = self.charset;
        b
    }
}

/// Text measurement and rendering for GDI (Door Text in the DLL).
pub trait TextBackend {
    fn metrics(&self, font: &LogFont) -> TextMetrics;
    /// Advance of every UTF-16 unit (surrogate halves split the pair's width).
    fn advances(&self, font: &LogFont, text: &[u16]) -> Vec<i32>;
    /// Draw with the cell's top-left at `origin` (device coordinates).
    fn draw(
        &self,
        font: &LogFont,
        text: &[u16],
        canvas: &mut Canvas,
        origin: Point,
        color: u32,
        clip: &Region,
    ) -> Result<()>;
}

/// Raw memory owned by the DLL (DIB sections, window back buffers).
pub trait External {
    fn pixels(&mut self, key: u64) -> Option<&mut [u32]>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pixels {
    Owned(Surface),
    External {
        key: u64,
        width: u32,
        height: u32,
        stride: usize,
        flip: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bitmap {
    pub pixels: Pixels,
    pub bits_pixel: u16,
    /// Application address of DIB-section bits (0 for compatible bitmaps).
    pub dib_address: u64,
}
impl Bitmap {
    /// A 1-bpp device-dependent bitmap (black/white store).
    pub fn is_mono(&self) -> bool {
        self.bits_pixel == 1
    }
    pub fn size(&self) -> (u32, u32) {
        match &self.pixels {
            Pixels::Owned(s) => (s.width(), s.height()),
            Pixels::External { width, height, .. } => (*width, *height),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Object {
    Pen(Pen),
    Brush(Brush),
    Font(LogFont),
    Bitmap(Bitmap),
    Region(Region),
    Palette,
}
impl Object {
    pub fn type_code(&self) -> u32 {
        match self {
            Self::Pen(_) => obj::PEN,
            Self::Brush(_) => obj::BRUSH,
            Self::Font(_) => obj::FONT,
            Self::Bitmap(_) => obj::BITMAP,
            Self::Region(_) => obj::REGION,
            Self::Palette => obj::PAL,
        }
    }
}

struct Entry {
    object: Object,
    stock: bool,
    selected: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DcState {
    pub pen: Handle,
    pub brush: Handle,
    pub font: Handle,
    pub bitmap: Option<Handle>,
    pub bk_color: u32,
    pub bk_mode: i32,
    pub text_color: u32,
    pub rop2: u32,
    pub text_align: u32,
    pub map: Mapping,
    pub pos: Point,
    pub brush_org: Point,
    /// Application clip region in DC device coordinates.
    pub clip: Option<Region>,
    pub stretch_mode: i32,
    pub poly_fill: i32,
    pub char_extra: i32,
    pub dc_brush: u32,
    pub dc_pen: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Memory DC drawing into its selected bitmap.
    Memory,
    /// Window DC: an external surface, the DC origin within it and the
    /// visible region in surface coordinates.
    Surface {
        key: u64,
        width: u32,
        height: u32,
        stride: usize,
        origin: Point,
        visible: Region,
    },
    /// Screen/information DC: metrics only, no drawing.
    Display,
}

pub struct Dc {
    pub target: Target,
    pub state: DcState,
    saved: Vec<DcState>,
}

pub struct Gdi {
    objects: Table<Entry>,
    dcs: Table<Dc>,
    stock: [Option<Handle>; 20],
    default_bitmap: Handle,
}

fn colorref(value: u32) -> u32 {
    ColorRef::validated(value)
        .unwrap_or(ColorRef(value & 0xff_ffff))
        .0
}
fn pixel(color: u32) -> u32 {
    ColorRef(color).pixel()
}

impl Gdi {
    pub fn new() -> Result<Self> {
        let mut objects = Table::new(kind::GDI, MAX_OBJECTS);
        let mut stock = [None; 20];
        let add = |objects: &mut Table<Entry>, o: Object| {
            objects.insert(Entry {
                object: o,
                stock: true,
                selected: 0,
            })
        };
        let gray = |v: u8| ColorRef::rgb(v, v, v).0;
        let list: [(i32, Object); 18] = [
            (stock::WHITE_BRUSH, Object::Brush(Brush::Solid(gray(255)))),
            (stock::LTGRAY_BRUSH, Object::Brush(Brush::Solid(gray(192)))),
            (stock::GRAY_BRUSH, Object::Brush(Brush::Solid(gray(128)))),
            (stock::DKGRAY_BRUSH, Object::Brush(Brush::Solid(gray(64)))),
            (stock::BLACK_BRUSH, Object::Brush(Brush::Solid(0))),
            (stock::NULL_BRUSH, Object::Brush(Brush::Null)),
            (stock::WHITE_PEN, Object::Pen(pen(ps::SOLID, 0, gray(255)))),
            (stock::BLACK_PEN, Object::Pen(pen(ps::SOLID, 0, 0))),
            (stock::NULL_PEN, Object::Pen(pen(ps::NULL, 0, 0))),
            (
                stock::OEM_FIXED_FONT,
                Object::Font(LogFont::named(12, 400, true)),
            ),
            (
                stock::ANSI_FIXED_FONT,
                Object::Font(LogFont::named(12, 400, true)),
            ),
            (
                stock::ANSI_VAR_FONT,
                Object::Font(LogFont::named(-11, 400, false)),
            ),
            (
                stock::SYSTEM_FONT,
                Object::Font(LogFont::named(16, 700, false)),
            ),
            (
                stock::DEVICE_DEFAULT_FONT,
                Object::Font(LogFont::named(16, 700, false)),
            ),
            (stock::DEFAULT_PALETTE, Object::Palette),
            (
                stock::SYSTEM_FIXED_FONT,
                Object::Font(LogFont::named(15, 400, true)),
            ),
            (
                stock::DEFAULT_GUI_FONT,
                Object::Font(LogFont::named(-12, 400, false)),
            ),
            (stock::DC_BRUSH, Object::Brush(Brush::DcBrush)),
        ];
        for (index, o) in list {
            stock[index as usize] = Some(add(&mut objects, o)?);
        }
        stock[stock::DC_PEN as usize] = Some(add(&mut objects, Object::Pen(pen(ps::SOLID, 0, 0)))?);
        let default_bitmap = add(
            &mut objects,
            Object::Bitmap(Bitmap {
                pixels: Pixels::Owned(Surface::new_mono(1, 1)?),
                bits_pixel: 1,
                dib_address: 0,
            }),
        )?;
        Ok(Self {
            objects,
            dcs: Table::new(kind::DC, MAX_DCS),
            stock,
            default_bitmap,
        })
    }
    pub fn stock(&self, index: i32) -> Option<Handle> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.stock.get(i).copied().flatten())
    }
    /// The stock 1x1 monochrome bitmap of every new memory DC (also what
    /// CreateBitmap/CreateCompatibleBitmap return for a zero extent).
    pub fn default_bitmap(&self) -> Handle {
        self.default_bitmap
    }
    /// Does `h` name a 1-bpp bitmap?
    pub fn is_mono_bitmap(&self, h: Handle) -> bool {
        matches!(self.object(h), Some(Object::Bitmap(b)) if b.is_mono())
    }
    /// Does the DC draw into a 1-bpp bitmap?
    pub fn is_mono_dc(&self, dc: Handle) -> bool {
        self.dcs.get(dc).is_some_and(|d| {
            d.target == Target::Memory && d.state.bitmap.is_some_and(|b| self.is_mono_bitmap(b))
        })
    }
    /// Live non-stock objects plus DCs (`GetGuiResources(GR_GDIOBJECTS)`).
    pub fn object_count(&self) -> usize {
        self.objects.iter().filter(|(_, e)| !e.stock).count() + self.dcs.len()
    }
    pub fn create(&mut self, object: Object) -> Result<Handle> {
        self.objects.insert(Entry {
            object,
            stock: false,
            selected: 0,
        })
    }
    /// Objects a DLL creates for its own lifetime (system color brushes).
    pub fn create_permanent(&mut self, object: Object) -> Result<Handle> {
        self.objects.insert(Entry {
            object,
            stock: true,
            selected: 0,
        })
    }
    pub fn resolve(&self, value: u64) -> Option<Handle> {
        self.objects.resolve(value)
    }
    pub fn object(&self, h: Handle) -> Option<&Object> {
        self.objects.get(h).map(|e| &e.object)
    }
    pub fn object_mut(&mut self, h: Handle) -> Option<&mut Object> {
        self.objects.get_mut(h).map(|e| &mut e.object)
    }
    pub fn is_stock(&self, h: Handle) -> bool {
        self.objects.get(h).is_some_and(|e| e.stock)
    }
    /// DeleteObject: stock objects are a successful no-op; an object
    /// selected into a DC is refused.
    pub fn delete(&mut self, h: Handle) -> Result<Option<Object>> {
        let e = self.objects.get(h).ok_or(Error::InvalidHandle)?;
        if e.stock {
            return Ok(None);
        }
        if e.selected != 0 {
            return Err(Error::InvalidParameter);
        }
        Ok(self.objects.remove(h).map(|e| e.object))
    }
    fn select_count(&mut self, h: Handle, delta: i32) {
        if let Some(e) = self.objects.get_mut(h) {
            e.selected = e.selected.saturating_add_signed(delta);
        }
    }

    // ---- device contexts ------------------------------------------------

    fn default_state(&self) -> DcState {
        DcState {
            pen: self.stock[stock::BLACK_PEN as usize].unwrap(),
            brush: self.stock[stock::WHITE_BRUSH as usize].unwrap(),
            font: self.stock[stock::SYSTEM_FONT as usize].unwrap(),
            bitmap: None,
            bk_color: ColorRef::WHITE.0,
            bk_mode: bk::OPAQUE,
            text_color: 0,
            rop2: r2::COPYPEN,
            text_align: 0,
            map: Mapping::default(),
            pos: Point::new(0, 0),
            brush_org: Point::new(0, 0),
            clip: None,
            stretch_mode: 1,
            poly_fill: 1,
            char_extra: 0,
            dc_brush: ColorRef::WHITE.0,
            dc_pen: 0,
        }
    }
    fn add_dc(&mut self, target: Target, bitmap: Option<Handle>) -> Result<Handle> {
        let mut state = self.default_state();
        state.bitmap = bitmap;
        let h = self.dcs.insert(Dc {
            target,
            state,
            saved: Vec::new(),
        })?;
        let st = self.dcs.get(h).unwrap().state.clone();
        for o in [Some(st.pen), Some(st.brush), Some(st.font), st.bitmap]
            .into_iter()
            .flatten()
        {
            self.select_count(o, 1);
        }
        Ok(h)
    }
    pub fn create_memory_dc(&mut self) -> Result<Handle> {
        self.add_dc(Target::Memory, Some(self.default_bitmap))
    }
    pub fn create_display_dc(&mut self) -> Result<Handle> {
        self.add_dc(Target::Display, None)
    }
    pub fn create_surface_dc(
        &mut self,
        key: u64,
        width: u32,
        height: u32,
        stride: usize,
        origin: Point,
        visible: Region,
    ) -> Result<Handle> {
        if (stride as u64) < u64::from(width) {
            return Err(Error::InvalidParameter);
        }
        self.add_dc(
            Target::Surface {
                key,
                width,
                height,
                stride,
                origin,
                visible,
            },
            None,
        )
    }
    /// Re-point every DC drawing into `key` (resized or released surface).
    pub fn retarget(&mut self, key: u64, size: Option<(u32, u32, usize)>) {
        let handles = self.dcs.handles();
        for h in handles {
            let dc = self.dcs.get_mut(h).unwrap();
            if let Target::Surface {
                key: k,
                width,
                height,
                stride,
                visible,
                ..
            } = &mut dc.target
                && *k == key
            {
                match size {
                    Some((w, hh, s)) => {
                        *width = w;
                        *height = hh;
                        *stride = s;
                    }
                    None => {
                        dc.target = Target::Display;
                        continue;
                    }
                }
                if let Ok(v) =
                    visible.intersect_rect(Rect::new(0, 0, *width as i32, *height as i32))
                {
                    *visible = v;
                }
            }
        }
    }
    pub fn resolve_dc(&self, value: u64) -> Option<Handle> {
        self.dcs.resolve(value)
    }
    pub fn dc(&self, h: Handle) -> Option<&Dc> {
        self.dcs.get(h)
    }
    pub fn dc_mut(&mut self, h: Handle) -> Option<&mut Dc> {
        self.dcs.get_mut(h)
    }
    pub fn state(&self, h: Handle) -> Result<&DcState> {
        self.dcs
            .get(h)
            .map(|d| &d.state)
            .ok_or(Error::InvalidHandle)
    }
    pub fn state_mut(&mut self, h: Handle) -> Result<&mut DcState> {
        self.dcs
            .get_mut(h)
            .map(|d| &mut d.state)
            .ok_or(Error::InvalidHandle)
    }
    pub fn delete_dc(&mut self, h: Handle) -> Result<()> {
        let dc = self.dcs.remove(h).ok_or(Error::InvalidHandle)?;
        for st in dc.saved.iter().chain(core::iter::once(&dc.state)) {
            for o in [Some(st.pen), Some(st.brush), Some(st.font), st.bitmap]
                .into_iter()
                .flatten()
            {
                self.select_count(o, -1);
            }
        }
        Ok(())
    }
    /// SelectObject for pens, brushes, fonts and bitmaps; returns the
    /// previous object. Regions go through `select_clip`.
    pub fn select(&mut self, dc: Handle, h: Handle) -> Result<Handle> {
        let code = self.object(h).ok_or(Error::InvalidHandle)?.type_code();
        let (memory, current_bitmap) = {
            let d = self.dcs.get(dc).ok_or(Error::InvalidHandle)?;
            (d.target == Target::Memory, d.state.bitmap)
        };
        if code == obj::BITMAP {
            if !memory {
                return Err(Error::InvalidParameter);
            }
            // A bitmap can be selected into only one DC at a time.
            if Some(h) != current_bitmap
                && h != self.default_bitmap
                && self.objects.get(h).is_some_and(|e| e.selected != 0)
            {
                return Err(Error::InvalidParameter);
            }
        }
        let st = &mut self.dcs.get_mut(dc).unwrap().state;
        let old = match code {
            obj::PEN => core::mem::replace(&mut st.pen, h),
            obj::BRUSH => core::mem::replace(&mut st.brush, h),
            obj::FONT => core::mem::replace(&mut st.font, h),
            obj::BITMAP => st.bitmap.replace(h).ok_or(Error::InvalidParameter)?,
            _ => return Err(Error::InvalidParameter),
        };
        self.select_count(old, -1);
        self.select_count(h, 1);
        Ok(old)
    }
    pub fn save(&mut self, dc: Handle) -> Result<i32> {
        let d = self.dcs.get_mut(dc).ok_or(Error::InvalidHandle)?;
        if d.saved.len() >= MAX_SAVED {
            return Err(Error::Quota);
        }
        d.saved.try_reserve(1).map_err(|_| Error::Quota)?;
        let st = d.state.clone();
        d.saved.push(st.clone());
        for o in [Some(st.pen), Some(st.brush), Some(st.font), st.bitmap]
            .into_iter()
            .flatten()
        {
            self.select_count(o, 1);
        }
        Ok(self.dcs.get(dc).unwrap().saved.len() as i32)
    }
    /// RestoreDC: positive levels are absolute, negative relative.
    pub fn restore(&mut self, dc: Handle, level: i32) -> Result<()> {
        let d = self.dcs.get_mut(dc).ok_or(Error::InvalidHandle)?;
        let depth = d.saved.len() as i32;
        let target = if level < 0 { depth + level } else { level - 1 };
        if level == 0 || target < 0 || target >= depth {
            return Err(Error::InvalidParameter);
        }
        let mut released = Vec::new();
        released.push(core::mem::replace(
            &mut d.state,
            d.saved[target as usize].clone(),
        ));
        released.extend(d.saved.drain(target as usize..));
        for st in released {
            for o in [Some(st.pen), Some(st.brush), Some(st.font), st.bitmap]
                .into_iter()
                .flatten()
            {
                self.select_count(o, -1);
            }
        }
        let st = self.dcs.get(dc).unwrap().state.clone();
        for o in [Some(st.pen), Some(st.brush), Some(st.font), st.bitmap]
            .into_iter()
            .flatten()
        {
            self.select_count(o, 1);
        }
        Ok(())
    }

    // ---- clipping -----------------------------------------------------------

    fn surface_size(&self, dc: &Dc) -> Option<(u32, u32)> {
        match &dc.target {
            Target::Memory => {
                let b = dc.state.bitmap?;
                match self.object(b)? {
                    Object::Bitmap(bm) => Some(bm.size()),
                    _ => None,
                }
            }
            Target::Surface { width, height, .. } => Some((*width, *height)),
            Target::Display => None,
        }
    }
    fn origin(dc: &Dc) -> Point {
        match &dc.target {
            Target::Surface { origin, .. } => *origin,
            _ => Point::new(0, 0),
        }
    }
    /// Effective clip in surface coordinates.
    fn effective_clip(&self, dc: &Dc) -> Result<Region> {
        let (w, h) = self.surface_size(dc).ok_or(Error::Unsupported)?;
        let mut clip = Region::from_rect(Rect::new(0, 0, w as i32, h as i32));
        if let Target::Surface { visible, .. } = &dc.target {
            clip = clip.combine(visible, Op::And)?;
        }
        if let Some(app) = &dc.state.clip {
            let mut app = app.clone();
            let o = Self::origin(dc);
            app.offset(o.x, o.y);
            clip = clip.combine(&app, Op::And)?;
        }
        Ok(clip)
    }
    /// The device-space clip box (`GetClipBox`, before the logical mapping).
    pub fn clip_box(&self, dc: Handle) -> Result<(i32, Rect)> {
        let d = self.dcs.get(dc).ok_or(Error::InvalidHandle)?;
        let region = match self.effective_clip(d) {
            Ok(r) => r,
            Err(_) => Region::from_rect(Rect::new(0, 0, 1, 1)),
        };
        let o = Self::origin(d);
        Ok((region.kind(), region.bounds().offset(-o.x, -o.y)))
    }
    /// SelectClipRgn/ExtSelectClipRgn with a region in DC device coordinates.
    pub fn select_clip(&mut self, dc: Handle, region: Option<&Region>, op: Op) -> Result<i32> {
        let d = self.dcs.get_mut(dc).ok_or(Error::InvalidHandle)?;
        let next = match (region, op) {
            (None, Op::Copy) => None,
            (None, _) => return Err(Error::InvalidParameter),
            (Some(r), Op::Copy) => Some(r.clone()),
            (Some(r), op) => {
                let base = d.state.clip.clone().unwrap_or_else(|| {
                    Region::from_rect(Rect::new(
                        -0x0800_0000,
                        -0x0800_0000,
                        0x0800_0000,
                        0x0800_0000,
                    ))
                });
                Some(base.combine(r, op)?)
            }
        };
        d.state.clip = next;
        Ok(d.state
            .clip
            .as_ref()
            .map_or(crate::region::SIMPLEREGION, |r| r.kind()))
    }
    /// IntersectClipRect/ExcludeClipRect with a logical rectangle.
    pub fn clip_rect(&mut self, dc: Handle, r: Rect, exclude: bool) -> Result<i32> {
        let device = self.state(dc)?.map.rect_to_dp(r);
        let op = if exclude { Op::Diff } else { Op::And };
        self.select_clip(dc, Some(&Region::from_rect(device)), op)
    }

    // ---- drawing --------------------------------------------------------------

    fn fill_for(&self, st: &DcState, brush: Handle) -> Result<Fill> {
        let b = match self.object(brush) {
            Some(Object::Brush(b)) => b,
            _ => return Err(Error::InvalidHandle),
        };
        Ok(match b {
            Brush::Solid(c) => Fill::Solid(pixel(*c)),
            Brush::Null => Fill::Null,
            Brush::DcBrush => Fill::Solid(pixel(st.dc_brush)),
            Brush::Hatch(style, c) => Fill::Hatch {
                style: (*style).min(hs::DIAGCROSS) as usize,
                fg: pixel(*c),
                bg: (st.bk_mode == bk::OPAQUE).then(|| pixel(st.bk_color)),
            },
            Brush::Pattern {
                width,
                height,
                px,
                mono,
            } => {
                let mut px = px.clone();
                convert_depth(
                    &mut px,
                    *mono,
                    false,
                    pixel(st.text_color),
                    pixel(st.bk_color),
                );
                Fill::Pattern {
                    width: *width,
                    height: *height,
                    px,
                }
            }
        })
    }
    /// Map blit source pixels between monochrome and color DCs (BitBlt's
    /// text/background color rules; a no-op between equal depths).
    fn convert_block(&self, src: Handle, dst: Handle, px: &mut [u32]) -> Result<()> {
        let (from, to) = (self.is_mono_dc(src), self.is_mono_dc(dst));
        if from != to {
            // Mono sources take the destination's colors; color sources are
            // compared with their own background color.
            let st = self.state(if from { dst } else { src })?;
            convert_depth(px, from, to, pixel(st.text_color), pixel(st.bk_color));
        }
        Ok(())
    }
    fn pen_of(&self, st: &DcState) -> Pen {
        match self.object(st.pen) {
            Some(Object::Pen(p)) if Some(st.pen) == self.stock[stock::DC_PEN as usize] => Pen {
                color: st.dc_pen,
                ..*p
            },
            Some(Object::Pen(p)) => *p,
            _ => pen(ps::SOLID, 0, 0),
        }
    }
    /// Run `f` on the DC's drawing canvas.
    fn with_canvas<R>(
        &mut self,
        dc: Handle,
        ext: &mut dyn External,
        f: impl FnOnce(&mut Canvas, &Region, Point) -> Result<R>,
    ) -> Result<R> {
        let d = self.dcs.get(dc).ok_or(Error::InvalidHandle)?;
        let clip = self.effective_clip(d)?;
        let origin = Self::origin(d);
        match d.target.clone() {
            Target::Display => Err(Error::Unsupported),
            Target::Surface {
                key,
                width,
                height,
                stride,
                ..
            } => {
                let px = ext.pixels(key).ok_or(Error::InvalidHandle)?;
                let mut c = Canvas::new(px, width, height, stride)?;
                f(&mut c, &clip, origin)
            }
            Target::Memory => {
                let b = d.state.bitmap.ok_or(Error::InvalidHandle)?;
                let bitmap = match self.objects.get_mut(b).map(|e| &mut e.object) {
                    Some(Object::Bitmap(bm)) => bm,
                    _ => return Err(Error::InvalidHandle),
                };
                match &mut bitmap.pixels {
                    Pixels::Owned(s) => {
                        let (w, h, mono) = (s.width(), s.height(), s.is_mono());
                        let mut c = Canvas::new(s.pixels_mut(), w, h, w as usize)?.with_mono(mono);
                        f(&mut c, &clip, origin)
                    }
                    Pixels::External {
                        key,
                        width,
                        height,
                        stride,
                        flip,
                    } => {
                        let px = ext.pixels(*key).ok_or(Error::InvalidHandle)?;
                        let mut c = Canvas::with_order(px, *width, *height, *stride, *flip)?;
                        f(&mut c, &clip, origin)
                    }
                }
            }
        }
    }
    fn device(&self, dc: Handle, p: Point) -> Result<Point> {
        let d = self.dcs.get(dc).ok_or(Error::InvalidHandle)?;
        let o = Self::origin(d);
        Ok(d.state.map.lp_to_dp(p).offset(o.x, o.y))
    }
    fn device_rect(&self, dc: Handle, r: Rect) -> Result<Rect> {
        let d = self.dcs.get(dc).ok_or(Error::InvalidHandle)?;
        let o = Self::origin(d);
        Ok(d.state.map.rect_to_dp(r).offset(o.x, o.y))
    }
    fn brush_origin(&self, dc: Handle) -> Result<Point> {
        let d = self.dcs.get(dc).ok_or(Error::InvalidHandle)?;
        let o = Self::origin(d);
        Ok(d.state.brush_org.offset(o.x, o.y))
    }
    /// FillRect: PATCOPY with an explicit brush, ignoring ROP2.
    pub fn fill_rect(
        &mut self,
        dc: Handle,
        r: Rect,
        brush: Handle,
        ext: &mut dyn External,
    ) -> Result<()> {
        let st = self.state(dc)?.clone();
        let fill = self.fill_for(&st, brush)?;
        let rect = self.device_rect(dc, r)?;
        let org = self.brush_origin(dc)?;
        self.with_canvas(dc, ext, |c, clip, _| {
            raster::blt(c, rect, None, &fill, org, rop::PATCOPY, clip)
        })
    }
    pub fn pat_blt(
        &mut self,
        dc: Handle,
        r: Rect,
        code: u32,
        ext: &mut dyn External,
    ) -> Result<()> {
        if raster::uses_source(code) {
            return Err(Error::InvalidParameter);
        }
        let st = self.state(dc)?.clone();
        let fill = self.fill_for(&st, st.brush)?;
        let rect = self.device_rect(dc, r)?;
        let org = self.brush_origin(dc)?;
        self.with_canvas(dc, ext, |c, clip, _| {
            raster::blt(c, rect, None, &fill, org, code, clip)
        })
    }
    /// Read source pixels for a blit (device rectangle of the source DC).
    fn read_source(&mut self, src: Handle, r: Rect, ext: &mut dyn External) -> Result<Block> {
        self.with_canvas(src, ext, |c, _, _| c.read(r))
    }
    #[allow(clippy::too_many_arguments)]
    pub fn bit_blt(
        &mut self,
        dst: Handle,
        r: Rect,
        src: Option<(Handle, Point)>,
        code: u32,
        ext: &mut dyn External,
    ) -> Result<()> {
        let code = code & !(rop::NOMIRRORBITMAP | rop::CAPTUREBLT);
        let st = self.state(dst)?.clone();
        let fill = if raster::uses_pattern(code) {
            self.fill_for(&st, st.brush)?
        } else {
            Fill::Null
        };
        let rect = self.device_rect(dst, r)?;
        let org = self.brush_origin(dst)?;
        let block = match src {
            Some((s, at)) if raster::uses_source(code) => {
                let start = self.device(s, at)?;
                let area = Rect::from_size(start.x, start.y, rect.width(), rect.height());
                let mut block = self.read_source(s, area, ext)?;
                self.convert_block(s, dst, &mut block.px)?;
                Some((block, start))
            }
            None if raster::uses_source(code) => return Err(Error::InvalidParameter),
            _ => None,
        };
        self.with_canvas(dst, ext, |c, clip, _| {
            raster::blt(
                c,
                rect,
                block.as_ref().map(|(b, p)| (b, *p)),
                &fill,
                org,
                code,
                clip,
            )
        })
    }
    #[allow(clippy::too_many_arguments)]
    pub fn stretch_blt(
        &mut self,
        dst: Handle,
        d: (i32, i32, i32, i32),
        src: Handle,
        s: (i32, i32, i32, i32),
        code: u32,
        ext: &mut dyn External,
    ) -> Result<()> {
        let code = code & !(rop::NOMIRRORBITMAP | rop::CAPTUREBLT);
        if !raster::uses_source(code) {
            return self.pat_blt(
                dst,
                Rect::from_size(d.0, d.1, d.2, d.3).normalized(),
                code,
                ext,
            );
        }
        let st = self.state(dst)?.clone();
        let fill = if raster::uses_pattern(code) {
            self.fill_for(&st, st.brush)?
        } else {
            Fill::Null
        };
        // Far corners saturate: hostile extents clip instead of wrapping.
        let a = self.device(dst, Point::new(d.0, d.1))?;
        let b = self.device(dst, Point::new(d.0, d.1).offset(d.2, d.3))?;
        let sa = self.device(src, Point::new(s.0, s.1))?;
        let sb = self.device(src, Point::new(s.0, s.1).offset(s.2, s.3))?;
        let mut block =
            self.read_source(src, Rect::new(sa.x, sa.y, sb.x, sb.y).normalized(), ext)?;
        self.convert_block(src, dst, &mut block.px)?;
        let org = self.brush_origin(dst)?;
        self.with_canvas(dst, ext, |c, clip, _| {
            raster::stretch(c, (a, b), &block, (sa, sb), &fill, org, code, clip)
        })
    }
    /// StretchDIBits / SetDIBitsToDevice: pixels the caller decoded
    /// (`block`, origin 0,0, top-down) with the source corners `s` in the
    /// block, onto the logical rectangle `d`, through raster operation `code`.
    pub fn stretch_pixels(
        &mut self,
        dst: Handle,
        d: (i32, i32, i32, i32),
        block: &Block,
        s: (Point, Point),
        code: u32,
        ext: &mut dyn External,
    ) -> Result<()> {
        let code = code & !(rop::NOMIRRORBITMAP | rop::CAPTUREBLT);
        let st = self.state(dst)?.clone();
        let fill = if raster::uses_pattern(code) {
            self.fill_for(&st, st.brush)?
        } else {
            Fill::Null
        };
        let a = self.device(dst, Point::new(d.0, d.1))?;
        let b = self.device(dst, Point::new(d.0, d.1).offset(d.2, d.3))?;
        let org = self.brush_origin(dst)?;
        self.with_canvas(dst, ext, |c, clip, _| {
            raster::stretch(c, (a, b), block, s, &fill, org, code, clip)
        })
    }
    /// AlphaBlend between DCs: `src_alpha` is AC_SRC_ALPHA (the source's
    /// premultiplied per-pixel alpha). The source rectangle must lie inside
    /// the source surface, and neither extent may be negative.
    #[allow(clippy::too_many_arguments)]
    pub fn alpha_blend(
        &mut self,
        dst: Handle,
        d: (i32, i32, i32, i32),
        src: Handle,
        s: (i32, i32, i32, i32),
        constant: u8,
        src_alpha: bool,
        ext: &mut dyn External,
    ) -> Result<()> {
        if d.2 < 0 || d.3 < 0 || s.2 < 0 || s.3 < 0 {
            return Err(Error::InvalidParameter);
        }
        let a = self.device(dst, Point::new(d.0, d.1))?;
        let b = self.device(dst, Point::new(d.0, d.1).offset(d.2, d.3))?;
        let sa = self.device(src, Point::new(s.0, s.1))?;
        let sb = self.device(src, Point::new(s.0, s.1).offset(s.2, s.3))?;
        let area = Rect::new(sa.x, sa.y, sb.x, sb.y).normalized();
        let block = self.with_canvas(src, ext, |c, _, _| {
            if !c.bounds().contains_rect(area) {
                return Err(Error::InvalidParameter);
            }
            c.read_alpha(area)
        })?;
        self.draw_block(
            dst,
            Rect::new(a.x, a.y, b.x, b.y),
            &block,
            area,
            constant,
            src_alpha,
            ext,
        )
    }
    /// DrawIconEx and friends: premultiplied `0xAARRGGBB` pixels the
    /// caller owns, `width` x `height` top-down, stretched onto the logical
    /// rectangle `d` and composited (`use_alpha` false: opaque).
    pub fn draw_pixels(
        &mut self,
        dst: Handle,
        d: (i32, i32, i32, i32),
        px: Vec<u32>,
        size: (u32, u32),
        use_alpha: bool,
        ext: &mut dyn External,
    ) -> Result<()> {
        if px.len() != size.0 as usize * size.1 as usize {
            return Err(Error::InvalidParameter);
        }
        let a = self.device(dst, Point::new(d.0, d.1))?;
        let b = self.device(dst, Point::new(d.0, d.1).offset(d.2, d.3))?;
        let block = Block {
            origin: Point::new(0, 0),
            width: size.0,
            height: size.1,
            px,
        };
        let area = Rect::new(0, 0, size.0 as i32, size.1 as i32);
        self.draw_block(
            dst,
            Rect::new(a.x, a.y, b.x, b.y),
            &block,
            area,
            255,
            use_alpha,
            ext,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn draw_block(
        &mut self,
        dst: Handle,
        device: Rect,
        block: &Block,
        area: Rect,
        constant: u8,
        use_alpha: bool,
        ext: &mut dyn External,
    ) -> Result<()> {
        self.with_canvas(dst, ext, |c, clip, _| {
            raster::blend(
                c,
                device.normalized(),
                raster::BlendSource {
                    block,
                    use_alpha,
                    constant,
                },
                area,
                clip,
            )
        })
    }
    pub fn move_to(&mut self, dc: Handle, p: Point) -> Result<Point> {
        let st = self.state_mut(dc)?;
        Ok(core::mem::replace(&mut st.pos, p))
    }
    fn stroke(&mut self, dc: Handle, a: Point, b: Point, ext: &mut dyn External) -> Result<()> {
        let st = self.state(dc)?.clone();
        let pen = self.pen_of(&st);
        if pen.style & ps::STYLE_MASK == ps::NULL {
            return Ok(());
        }
        let (da, db) = (self.device(dc, a)?, self.device(dc, b)?);
        let width = st.map.scale_x(pen.width).max(1);
        let color = pixel(pen.color);
        self.with_canvas(dc, ext, |c, clip, _| {
            if width <= 1 {
                raster::line(c, da, db, color, st.rop2, clip);
                Ok(())
            } else {
                raster::wide_line(c, da, db, width, color, st.rop2, clip)
            }
        })
    }
    pub fn line_to(&mut self, dc: Handle, p: Point, ext: &mut dyn External) -> Result<()> {
        let from = self.state(dc)?.pos;
        self.stroke(dc, from, p, ext)?;
        self.state_mut(dc)?.pos = p;
        Ok(())
    }
    pub fn rectangle(&mut self, dc: Handle, r: Rect, ext: &mut dyn External) -> Result<()> {
        let r = r.normalized();
        let st = self.state(dc)?.clone();
        let pen = self.pen_of(&st);
        let null_pen = pen.style & ps::STYLE_MASK == ps::NULL;
        let fill = self.fill_for(&st, st.brush)?;
        let (right, bottom) = (r.right.saturating_sub(1), r.bottom.saturating_sub(1));
        let inner = if null_pen {
            Rect::new(r.left, r.top, right, bottom)
        } else {
            Rect::new(
                r.left.saturating_add(1),
                r.top.saturating_add(1),
                right,
                bottom,
            )
        };
        let device_inner = self.device_rect(dc, inner)?;
        let org = self.brush_origin(dc)?;
        if !fill.is_null() && !inner.is_empty() {
            self.with_canvas(dc, ext, |c, clip, _| {
                raster::fill_rop2(c, device_inner, &fill, org, st.rop2, clip);
                Ok(())
            })?;
        }
        if !null_pen && r.width() > 0 && r.height() > 0 {
            let (l, t, rr, b) = (r.left, r.top, r.right - 1, r.bottom - 1);
            if rr == l || b == t {
                // Degenerate: the outline is the whole one-pixel rectangle.
                let color = pixel(pen.color);
                let device = self.device_rect(dc, r)?;
                self.with_canvas(dc, ext, |c, clip, _| {
                    raster::fill_rop2(c, device, &Fill::Solid(color), org, st.rop2, clip);
                    Ok(())
                })?;
            } else {
                for (a, z) in [
                    ((l, t), (rr, t)),
                    ((rr, t), (rr, b)),
                    ((rr, b), (l, b)),
                    ((l, b), (l, t)),
                ] {
                    self.stroke(dc, Point::new(a.0, a.1), Point::new(z.0, z.1), ext)?;
                }
            }
        }
        Ok(())
    }
    pub fn set_pixel(
        &mut self,
        dc: Handle,
        p: Point,
        color: u32,
        ext: &mut dyn External,
    ) -> Result<u32> {
        let at = self.device(dc, p)?;
        let px = pixel(colorref(color));
        self.with_canvas(dc, ext, |c, clip, _| {
            if raster::plot(c, at, px, r2::COPYPEN, clip) {
                // The color actually stored (black or white on a mono DC).
                Ok(ColorRef::from_pixel(c.get(at.x, at.y).unwrap_or(px)).0)
            } else {
                Err(Error::InvalidParameter)
            }
        })
    }
    /// GetPixel: CLR_INVALID outside the clip region or surface.
    pub fn get_pixel(&mut self, dc: Handle, p: Point, ext: &mut dyn External) -> Result<u32> {
        let at = self.device(dc, p)?;
        self.with_canvas(dc, ext, |c, clip, _| {
            if !clip.contains(at) {
                return Err(Error::InvalidParameter);
            }
            c.get(at.x, at.y)
                .map(|v| ColorRef::from_pixel(v).0)
                .ok_or(Error::InvalidParameter)
        })
    }
    pub fn fill_region(
        &mut self,
        dc: Handle,
        region: &Region,
        brush: Handle,
        ext: &mut dyn External,
    ) -> Result<()> {
        for r in region.rects().to_vec() {
            self.fill_rect(dc, r, brush, ext)?;
        }
        Ok(())
    }
    pub fn font(&self, dc: Handle) -> Result<LogFont> {
        match self.object(self.state(dc)?.font) {
            Some(Object::Font(f)) => Ok(*f),
            _ => Err(Error::InvalidHandle),
        }
    }
    /// GetTextExtentPoint32W (device units of the logical extent).
    pub fn text_extent(&self, dc: Handle, text: &[u16], backend: &dyn TextBackend) -> Result<Size> {
        let font = self.font(dc)?;
        let extra = self.state(dc)?.char_extra;
        let width = text_width(&backend.advances(&font, text), extra);
        Ok(Size::new(width, backend.metrics(&font).height))
    }
    pub fn text_metrics(&self, dc: Handle, backend: &dyn TextBackend) -> Result<TextMetrics> {
        Ok(backend.metrics(&self.font(dc)?))
    }
    /// ExtTextOutW/TextOutW. `opaque` is an optional logical rectangle
    /// filled with the background color (ETO_OPAQUE); `clip_to` clips the
    /// text to it (ETO_CLIPPED).
    #[allow(clippy::too_many_arguments)]
    pub fn text_out(
        &mut self,
        dc: Handle,
        at: Point,
        text: &[u16],
        opaque: Option<Rect>,
        clip_to: Option<Rect>,
        backend: &dyn TextBackend,
        ext: &mut dyn External,
    ) -> Result<()> {
        let st = self.state(dc)?.clone();
        let font = self.font(dc)?;
        let metrics = backend.metrics(&font);
        let width = text_width(&backend.advances(&font, text), st.char_extra);
        let start = if st.text_align & ta::UPDATECP != 0 {
            st.pos
        } else {
            at
        };
        let mut origin = self.device(dc, start)?;
        match st.text_align & ta::CENTER {
            ta::RIGHT => origin.x = origin.x.saturating_sub(width),
            ta::CENTER => origin.x = origin.x.saturating_sub(width / 2),
            _ => (),
        }
        match st.text_align & ta::BASELINE {
            ta::BASELINE => origin.y = origin.y.saturating_sub(metrics.ascent),
            ta::BOTTOM => origin.y = origin.y.saturating_sub(metrics.height),
            _ => (),
        }
        let cell = Rect::from_size(origin.x, origin.y, width, metrics.height);
        let opaque = opaque.map(|r| self.device_rect(dc, r)).transpose()?;
        let clip_rect = clip_to.map(|r| self.device_rect(dc, r)).transpose()?;
        let bk_pixel = pixel(st.bk_color);
        let fg = pixel(st.text_color);
        self.with_canvas(dc, ext, |c, clip, _| {
            let mut clip = clip.clone();
            if let Some(r) = clip_rect {
                clip = clip.intersect_rect(r)?;
            }
            if let Some(r) = opaque {
                raster::blt(
                    c,
                    r,
                    None,
                    &Fill::Solid(bk_pixel),
                    Point::default(),
                    rop::PATCOPY,
                    &clip,
                )?;
            }
            if st.bk_mode == bk::OPAQUE {
                raster::blt(
                    c,
                    cell,
                    None,
                    &Fill::Solid(bk_pixel),
                    Point::default(),
                    rop::PATCOPY,
                    &clip,
                )?;
            }
            backend.draw(&font, text, c, origin, fg, &clip)
        })?;
        if st.text_align & ta::UPDATECP != 0 {
            let d = self.dcs.get_mut(dc).unwrap();
            d.state.pos = Point::new(start.x.saturating_add(width), start.y);
        }
        Ok(())
    }
    /// Snapshot a bitmap's pixels (pattern brushes, GetDIBits).
    pub fn bitmap_pixels(
        &mut self,
        h: Handle,
        ext: &mut dyn External,
    ) -> Result<(u32, u32, Vec<u32>)> {
        self.bitmap_read(h, ext, false)
    }
    /// As `bitmap_pixels`, keeping each pixel's upper byte (a 32-bpp DIB
    /// section's alpha: CreateIconIndirect, GetDIBits).
    pub fn bitmap_argb(
        &mut self,
        h: Handle,
        ext: &mut dyn External,
    ) -> Result<(u32, u32, Vec<u32>)> {
        self.bitmap_read(h, ext, true)
    }
    fn bitmap_read(
        &mut self,
        h: Handle,
        ext: &mut dyn External,
        alpha: bool,
    ) -> Result<(u32, u32, Vec<u32>)> {
        let read = |c: Canvas, r: Rect| if alpha { c.read_alpha(r) } else { c.read(r) };
        let bitmap = match self.objects.get_mut(h).map(|e| &mut e.object) {
            Some(Object::Bitmap(b)) => b,
            _ => return Err(Error::InvalidHandle),
        };
        let (w, hh) = bitmap.size();
        let block = match &mut bitmap.pixels {
            Pixels::Owned(s) => {
                let (sw, sh) = (s.width(), s.height());
                read(
                    Canvas::new(s.pixels_mut(), sw, sh, sw as usize)?,
                    Rect::new(0, 0, sw as i32, sh as i32),
                )?
            }
            Pixels::External {
                key,
                width,
                height,
                stride,
                flip,
            } => {
                let px = ext.pixels(*key).ok_or(Error::InvalidHandle)?;
                read(
                    Canvas::with_order(px, *width, *height, *stride, *flip)?,
                    Rect::new(0, 0, *width as i32, *height as i32),
                )?
            }
        };
        Ok((w, hh, block.px))
    }
    /// Is this map mode one of the documented values?
    pub fn valid_map_mode(mode: i32) -> bool {
        (mm::TEXT..=mm::ANISOTROPIC).contains(&mode)
    }
    pub fn brush_style_supported(style: u32) -> bool {
        matches!(style, bs::SOLID | bs::NULL | bs::HATCHED)
    }
}

pub const fn pen(style: u32, width: i32, color: u32) -> Pen {
    Pen {
        style,
        width,
        color,
    }
}

/// Width of a string from its advances plus the character extra, saturated
/// to the `i32` range (fonts and extras come from the application).
fn text_width(advances: &[i32], extra: i32) -> i32 {
    let sum: i64 = advances.iter().map(|a| i64::from(*a)).sum::<i64>()
        + i64::from(extra) * advances.len() as i64;
    sum.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    struct NoExt;
    impl External for NoExt {
        fn pixels(&mut self, _: u64) -> Option<&mut [u32]> {
            None
        }
    }
    struct Mem(Vec<u32>);
    impl External for Mem {
        fn pixels(&mut self, key: u64) -> Option<&mut [u32]> {
            (key == 7).then_some(&mut self.0[..])
        }
    }
    /// Fixed 8x16 cells: enough to test placement and opaque backgrounds.
    struct Cells;
    impl TextBackend for Cells {
        fn metrics(&self, _: &LogFont) -> TextMetrics {
            TextMetrics {
                height: 16,
                ascent: 12,
                descent: 4,
                ave_char_width: 8,
                max_char_width: 8,
                weight: 400,
                ..TextMetrics::default()
            }
        }
        fn advances(&self, _: &LogFont, text: &[u16]) -> Vec<i32> {
            text.iter().map(|_| 8).collect()
        }
        fn draw(
            &self,
            _: &LogFont,
            text: &[u16],
            c: &mut Canvas,
            o: Point,
            color: u32,
            clip: &Region,
        ) -> Result<()> {
            for i in 0..text.len() as i32 {
                raster::plot(c, Point::new(o.x + 8 * i, o.y), color, r2::COPYPEN, clip);
            }
            Ok(())
        }
    }
    fn dib(gdi: &mut Gdi, w: u32, h: u32) -> Handle {
        gdi.create(Object::Bitmap(Bitmap {
            pixels: Pixels::External {
                key: 7,
                width: w,
                height: h,
                stride: w as usize,
                flip: false,
            },
            bits_pixel: 32,
            dib_address: 0x1000,
        }))
        .unwrap()
    }
    #[test]
    fn stock_objects_selection_and_counts() {
        let mut g = Gdi::new().unwrap();
        let before = g.object_count();
        let white = g.stock(stock::WHITE_BRUSH).unwrap();
        assert_eq!(g.delete(white), Ok(None), "stock delete is a no-op");
        assert!(g.object(white).is_some());
        let dc = g.create_memory_dc().unwrap();
        let red = g.create(Object::Brush(Brush::Solid(0xff))).unwrap();
        assert_eq!(g.select(dc, red), Ok(white));
        assert_eq!(g.delete(red), Err(Error::InvalidParameter), "selected");
        assert_eq!(g.select(dc, white), Ok(red));
        assert!(g.delete(red).unwrap().is_some());
        assert_eq!(g.object(red), None);
        let b = dib(&mut g, 4, 4);
        let dc2 = g.create_memory_dc().unwrap();
        assert!(g.select(dc, b).is_ok());
        assert_eq!(
            g.select(dc2, b),
            Err(Error::InvalidParameter),
            "one DC per bitmap"
        );
        let rgn = g.create(Object::Region(Region::new())).unwrap();
        assert_eq!(g.select(dc, rgn), Err(Error::InvalidParameter));
        g.delete_dc(dc).unwrap();
        g.delete_dc(dc2).unwrap();
        assert!(g.delete(b).unwrap().is_some());
        g.delete(rgn).unwrap();
        assert_eq!(g.object_count(), before);
        assert_eq!(g.delete_dc(dc), Err(Error::InvalidHandle));
    }
    #[test]
    fn save_restore_levels() {
        let mut g = Gdi::new().unwrap();
        let dc = g.create_memory_dc().unwrap();
        g.state_mut(dc).unwrap().text_color = 1;
        assert_eq!(g.save(dc), Ok(1));
        g.state_mut(dc).unwrap().text_color = 2;
        assert_eq!(g.save(dc), Ok(2));
        g.state_mut(dc).unwrap().text_color = 3;
        g.restore(dc, -1).unwrap();
        assert_eq!(g.state(dc).unwrap().text_color, 2);
        g.restore(dc, 1).unwrap();
        assert_eq!(g.state(dc).unwrap().text_color, 1);
        assert_eq!(g.restore(dc, 1), Err(Error::InvalidParameter));
        assert_eq!(g.restore(dc, 0), Err(Error::InvalidParameter));
    }
    #[test]
    fn rectangles_lines_and_pixels_on_a_dib() {
        let mut g = Gdi::new().unwrap();
        let mut ext = Mem(alloc::vec![0x0020_4060; 64 * 32]);
        let dc = g.create_memory_dc().unwrap();
        let b = dib(&mut g, 64, 32);
        g.select(dc, b).unwrap();
        let yellow = g
            .create(Object::Pen(pen(ps::SOLID, 1, ColorRef::rgb(255, 255, 0).0)))
            .unwrap();
        let red = g
            .create(Object::Brush(Brush::Solid(ColorRef::rgb(200, 30, 30).0)))
            .unwrap();
        g.select(dc, yellow).unwrap();
        g.select(dc, red).unwrap();
        g.rectangle(dc, Rect::new(4, 4, 40, 28), &mut ext).unwrap();
        let px = |g: &mut Gdi, ext: &mut Mem, x, y| g.get_pixel(dc, Point::new(x, y), ext).unwrap();
        assert_eq!(px(&mut g, &mut ext, 10, 10), ColorRef::rgb(200, 30, 30).0);
        assert_eq!(px(&mut g, &mut ext, 4, 4), ColorRef::rgb(255, 255, 0).0);
        assert_eq!(px(&mut g, &mut ext, 4, 27), ColorRef::rgb(255, 255, 0).0);
        assert_eq!(px(&mut g, &mut ext, 39, 27), ColorRef::rgb(255, 255, 0).0);
        assert_eq!(
            px(&mut g, &mut ext, 40, 10),
            ColorRef::rgb(0x20, 0x40, 0x60).0
        );
        g.move_to(dc, Point::new(2, 30)).unwrap();
        g.line_to(dc, Point::new(62, 30), &mut ext).unwrap();
        assert_eq!(px(&mut g, &mut ext, 61, 30), ColorRef::rgb(255, 255, 0).0);
        assert_eq!(
            px(&mut g, &mut ext, 62, 30),
            ColorRef::rgb(0x20, 0x40, 0x60).0,
            "end excluded"
        );
        assert_eq!(g.state(dc).unwrap().pos, Point::new(62, 30));
        assert!(g.get_pixel(dc, Point::new(64, 0), &mut ext).is_err());
        // Viewport origins move drawing; clip regions restrict it.
        g.state_mut(dc).unwrap().map.viewport_org = Point::new(1, 1);
        assert_eq!(g.set_pixel(dc, Point::new(0, 0), 0xff, &mut ext), Ok(0xff));
        g.state_mut(dc).unwrap().map.viewport_org = Point::new(0, 0);
        assert_eq!(px(&mut g, &mut ext, 1, 1), 0xff);
        g.clip_rect(dc, Rect::new(0, 0, 2, 2), false).unwrap();
        assert!(g.set_pixel(dc, Point::new(5, 5), 0xff, &mut ext).is_err());
        assert_eq!(g.clip_box(dc).unwrap().1, Rect::new(0, 0, 2, 2));
        g.select_clip(dc, None, Op::Copy).unwrap();
        assert_eq!(g.clip_box(dc).unwrap().1, Rect::new(0, 0, 64, 32));
    }
    #[test]
    fn hatch_patblt_respects_background_mode_and_blits_copy() {
        let mut g = Gdi::new().unwrap();
        let mut ext = Mem(alloc::vec![0; 16 * 8]);
        let dc = g.create_memory_dc().unwrap();
        let b = dib(&mut g, 16, 8);
        g.select(dc, b).unwrap();
        let hatch = g
            .create(Object::Brush(Brush::Hatch(hs::DIAGCROSS, 0x0000_80ff)))
            .unwrap();
        g.select(dc, hatch).unwrap();
        g.state_mut(dc).unwrap().bk_color = 0x0040_0040;
        g.pat_blt(dc, Rect::new(0, 0, 8, 8), rop::PATCOPY, &mut ext)
            .unwrap();
        assert_eq!(g.get_pixel(dc, Point::new(0, 0), &mut ext), Ok(0x0000_80ff));
        assert_eq!(g.get_pixel(dc, Point::new(1, 0), &mut ext), Ok(0x0040_0040));
        g.state_mut(dc).unwrap().bk_mode = bk::TRANSPARENT;
        g.pat_blt(dc, Rect::new(8, 0, 16, 8), rop::PATCOPY, &mut ext)
            .unwrap();
        assert_eq!(
            g.get_pixel(dc, Point::new(9, 0), &mut ext),
            Ok(0),
            "transparent gaps"
        );
        assert!(
            g.pat_blt(dc, Rect::new(0, 0, 1, 1), rop::SRCCOPY, &mut ext)
                .is_err()
        );
        // Memory-to-memory blit between an owned bitmap and the DIB.
        let src_dc = g.create_memory_dc().unwrap();
        let mut s = Surface::new(2, 2).unwrap();
        s.fill(0x0012_3456);
        let owned = g
            .create(Object::Bitmap(Bitmap {
                pixels: Pixels::Owned(s),
                bits_pixel: 32,
                dib_address: 0,
            }))
            .unwrap();
        g.select(src_dc, owned).unwrap();
        g.bit_blt(
            dc,
            Rect::new(4, 4, 6, 6),
            Some((src_dc, Point::new(0, 0))),
            rop::SRCINVERT,
            &mut ext,
        )
        .unwrap();
        assert_eq!(
            g.get_pixel(dc, Point::new(4, 5), &mut ext),
            Ok(ColorRef::from_pixel(0x0012_3456 ^ 0x0040_0040).0)
        );
        let (w, h, px) = g.bitmap_pixels(owned, &mut ext).unwrap();
        assert_eq!((w, h, px.len()), (2, 2, 4));
        g.stretch_blt(
            dc,
            (0, 0, 4, 4),
            src_dc,
            (0, 0, 2, 2),
            rop::SRCCOPY,
            &mut ext,
        )
        .unwrap();
        assert_eq!(
            g.get_pixel(dc, Point::new(3, 3), &mut ext),
            Ok(ColorRef::from_pixel(0x0012_3456).0)
        );
        // A display DC has no canvas.
        let screen = g.create_display_dc().unwrap();
        assert_eq!(
            g.pat_blt(screen, Rect::new(0, 0, 1, 1), rop::BLACKNESS, &mut NoExt),
            Err(Error::Unsupported)
        );
    }
    #[test]
    fn surface_dcs_clip_to_their_visible_region_and_text_places_cells() {
        let mut g = Gdi::new().unwrap();
        let mut ext = Mem(alloc::vec![0; 32 * 32]);
        let visible = Region::from_rect(Rect::new(8, 8, 24, 24));
        let dc = g
            .create_surface_dc(7, 32, 32, 32, Point::new(8, 8), visible)
            .unwrap();
        let black = g.stock(stock::BLACK_BRUSH).unwrap();
        let white = g.stock(stock::WHITE_BRUSH).unwrap();
        g.fill_rect(dc, Rect::new(-100, -100, 100, 100), white, &mut ext)
            .unwrap();
        assert_eq!(ext.0[8 * 32 + 8], 0x00ff_ffff);
        assert_eq!(ext.0[7 * 32 + 7], 0, "outside the visible region");
        assert_eq!(ext.0[24 * 32 + 24], 0);
        g.fill_rect(dc, Rect::new(0, 0, 1, 1), black, &mut ext)
            .unwrap();
        assert_eq!(ext.0[8 * 32 + 8], 0, "DC origin offsets drawing");
        g.state_mut(dc).unwrap().text_color = 0x0000_00ff;
        g.text_out(
            dc,
            Point::new(2, 2),
            &[65, 66],
            None,
            None,
            &Cells,
            &mut ext,
        )
        .unwrap();
        assert_eq!(ext.0[10 * 32 + 10], 0x00ff_0000);
        assert_eq!(ext.0[11 * 32 + 10], 0x00ff_ffff, "opaque background");
        assert_eq!(
            g.text_extent(dc, &[65, 66, 67], &Cells).unwrap(),
            Size::new(24, 16)
        );
        g.retarget(7, None);
        assert_eq!(
            g.fill_rect(dc, Rect::new(0, 0, 1, 1), black, &mut ext),
            Err(Error::Unsupported)
        );
        let lf = LogFont::from_bytes(&LogFont::named(-12, 700, false).to_bytes()).unwrap();
        assert_eq!(lf.weight, 700);
        assert!(LogFont::from_bytes(&[0; 10]).is_err());
        let tm = Cells.metrics(&lf).to_bytes();
        assert_eq!(i32::from_le_bytes(tm[..4].try_into().unwrap()), 16);
    }
    fn owned(g: &mut Gdi, s: Surface) -> Handle {
        let bits_pixel = if s.is_mono() { 1 } else { 32 };
        g.create(Object::Bitmap(Bitmap {
            pixels: Pixels::Owned(s),
            bits_pixel,
            dib_address: 0,
        }))
        .unwrap()
    }
    #[test]
    fn monochrome_bitmaps_follow_the_text_and_background_rules() {
        let mut g = Gdi::new().unwrap();
        let mut ext = NoExt;
        let w = 0x00ff_ffff;
        // A new memory DC holds the stock 1x1 mono bitmap.
        let mono_dc = g.create_memory_dc().unwrap();
        assert!(g.is_mono_dc(mono_dc));
        assert!(g.is_mono_bitmap(g.default_bitmap()));
        // 1 0 / 0 1 mono source.
        let mask = Surface::from_ddb(2, 2, 1, Some(&[0x80, 0, 0x40, 0])).unwrap();
        let mask = owned(&mut g, mask);
        g.select(mono_dc, mask).unwrap();
        let color_dc = g.create_memory_dc().unwrap();
        let color = owned(&mut g, Surface::new(2, 2).unwrap());
        g.select(color_dc, color).unwrap();
        assert!(!g.is_mono_dc(color_dc));
        // Mono -> color: 0 bits take the destination text color, 1 bits its
        // background color.
        g.state_mut(color_dc).unwrap().text_color = ColorRef::rgb(0x11, 0x22, 0x33).0;
        g.state_mut(color_dc).unwrap().bk_color = ColorRef::rgb(0x44, 0x55, 0x66).0;
        g.bit_blt(
            color_dc,
            Rect::new(0, 0, 2, 2),
            Some((mono_dc, Point::new(0, 0))),
            rop::SRCCOPY,
            &mut ext,
        )
        .unwrap();
        let at = |g: &mut Gdi, dc, x, y| g.get_pixel(dc, Point::new(x, y), &mut NoExt).unwrap();
        assert_eq!(
            at(&mut g, color_dc, 0, 0),
            ColorRef::rgb(0x44, 0x55, 0x66).0
        );
        assert_eq!(
            at(&mut g, color_dc, 1, 0),
            ColorRef::rgb(0x11, 0x22, 0x33).0
        );
        assert_eq!(
            at(&mut g, color_dc, 1, 1),
            ColorRef::rgb(0x44, 0x55, 0x66).0
        );
        // Color -> mono: the source background color becomes white, every
        // other color black.
        g.state_mut(color_dc).unwrap().bk_color = ColorRef::rgb(0x11, 0x22, 0x33).0;
        g.bit_blt(
            mono_dc,
            Rect::new(0, 0, 2, 2),
            Some((color_dc, Point::new(0, 0))),
            rop::SRCCOPY,
            &mut ext,
        )
        .unwrap();
        assert_eq!(at(&mut g, mono_dc, 1, 0), w);
        assert_eq!(at(&mut g, mono_dc, 0, 0), 0);
        // StretchBlt converts the same way.
        g.stretch_blt(
            color_dc,
            (0, 0, 2, 2),
            mono_dc,
            (0, 0, 1, 1),
            rop::SRCCOPY,
            &mut ext,
        )
        .unwrap();
        assert_eq!(
            at(&mut g, color_dc, 1, 1),
            ColorRef::rgb(0x11, 0x22, 0x33).0
        );
        // Drawing into a mono DC realizes colors to black or white.
        let light = g
            .create(Object::Brush(Brush::Solid(ColorRef::rgb(200, 200, 200).0)))
            .unwrap();
        g.fill_rect(mono_dc, Rect::new(0, 0, 2, 2), light, &mut ext)
            .unwrap();
        assert_eq!(at(&mut g, mono_dc, 0, 1), w);
        assert_eq!(
            g.set_pixel(
                mono_dc,
                Point::new(0, 0),
                ColorRef::rgb(255, 0, 0).0,
                &mut ext
            ),
            Ok(0)
        );
        // A mono pattern brush paints 0 bits in the text color and 1 bits in
        // the background color of the DC it draws on. The mask is now
        // black at (0, 0) and white elsewhere.
        let (pw, ph, px) = g.bitmap_pixels(mask, &mut ext).unwrap();
        assert_eq!(px, [0, w, w, w]);
        let pattern = g
            .create(Object::Brush(Brush::Pattern {
                width: pw,
                height: ph,
                px,
                mono: g.is_mono_bitmap(mask),
            }))
            .unwrap();
        g.state_mut(color_dc).unwrap().bk_color = ColorRef::rgb(0, 0, 0xff).0;
        g.fill_rect(color_dc, Rect::new(0, 0, 2, 2), pattern, &mut ext)
            .unwrap();
        assert_eq!(
            at(&mut g, color_dc, 0, 0),
            ColorRef::rgb(0x11, 0x22, 0x33).0
        );
        assert_eq!(at(&mut g, color_dc, 1, 0), ColorRef::rgb(0, 0, 0xff).0);
    }
    #[test]
    fn hostile_coordinates_clip_or_fail_but_never_overflow() {
        let mut g = Gdi::new().unwrap();
        let mut ext = Mem(alloc::vec![0; 16 * 16]);
        let dc = g.create_memory_dc().unwrap();
        let b = dib(&mut g, 16, 16);
        g.select(dc, b).unwrap();
        let src = g.create_memory_dc().unwrap();
        let sb = owned(&mut g, Surface::new(4, 4).unwrap());
        g.select(src, sb).unwrap();
        let (min, max) = (i32::MIN, i32::MAX);
        let extremes = [min, min + 1, -1, 0, 1, 15, max - 1, max];
        let wide = g.create(Object::Pen(pen(ps::SOLID, max, 0xff))).unwrap();
        let thin = g.create(Object::Pen(pen(ps::SOLID, 1, 0xff00))).unwrap();
        for &x in &extremes {
            for &y in &extremes {
                let r = Rect::new(x, y, y, x);
                let _ = g.rectangle(dc, r, &mut ext);
                let _ = g.rectangle(dc, Rect::new(x, x, x, x), &mut ext);
                let _ = g.pat_blt(dc, r, rop::PATINVERT, &mut ext);
                let _ = g.bit_blt(dc, r, Some((src, Point::new(y, x))), rop::SRCCOPY, &mut ext);
                let _ = g.stretch_blt(dc, (x, y, y, x), src, (y, x, x, y), rop::SRCCOPY, &mut ext);
                let _ = g.stretch_blt(
                    dc,
                    (x, y, max, min),
                    src,
                    (0, 0, x, y),
                    0x00b8_074a,
                    &mut ext,
                );
                let _ = g.set_pixel(dc, Point::new(x, y), 0xff, &mut ext);
                let _ = g.get_pixel(dc, Point::new(x, y), &mut ext);
                for p in [wide, thin] {
                    g.select(dc, p).unwrap();
                    g.move_to(dc, Point::new(y, x)).unwrap();
                    g.line_to(dc, Point::new(x, y), &mut ext).unwrap();
                }
                g.state_mut(dc).unwrap().text_align = ta::UPDATECP | ta::RIGHT | ta::BOTTOM;
                g.state_mut(dc).unwrap().char_extra = x;
                g.state_mut(dc).unwrap().brush_org = Point::new(x, y);
                let _ = g.text_out(dc, Point::new(x, y), &[65; 4], None, None, &Cells, &mut ext);
                let _ = g.text_extent(dc, &[65; 4], &Cells);
            }
        }
        // A scalable map mode with extreme extents scales pen widths
        // without overflowing either.
        let st = g.state_mut(dc).unwrap();
        st.map.set_mode(mm::ANISOTROPIC);
        st.map.set_window_ext(Size::new(1, 1));
        st.map.set_viewport_ext(Size::new(min, min));
        g.select(dc, wide).unwrap();
        g.move_to(dc, Point::new(0, 0)).unwrap();
        g.line_to(dc, Point::new(3, 3), &mut ext).unwrap();
        assert_eq!(g.state(dc).unwrap().map.scale_x(max), max);
    }
}
