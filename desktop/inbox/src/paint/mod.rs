//! Paint: Windows 11 26H2 Paint and Paint 3D in one app. The window is
//! measured from Paint 11.2605 at 150%: a 32 px title bar, the 36 px menu
//! bar (File, Edit, View; Save, Share, Undo, Redo; feedback and Settings on
//! the right), the 110 px ribbon card (Selection, Image, Tools, Brushes,
//! Shapes, Colors, then Paint 3D's 3D shapes and Layers, each group labelled
//! under a divider), the canvas over the tinted backdrop with the floating
//! size and opacity sliders on the left, and the 46 px status bar (pointer
//! position, selection and canvas size, fit, zoom). The canvas is real:
//! layers, the pencil, nine brushes and the eraser, shapes with outline and
//! fill, flood fill, text, the colour picker, selections that move, cut,
//! copy, paste and crop, rotation and flips, resizing, lit 3D shapes, zoom
//! and undo, saved as PNG in Pictures.
pub mod raster;
pub(crate) mod view;

use alloc::{format, string::String, vec, vec::Vec};
use door_app_ui::{Application, View};
use door_graphics_text::FontAtlas;
use door_service_client::{Error, Storage};
use raster::{Bounds, Brush, Doc, Edit, History, Rgba, Shape, Solid, Tip, Turn};

pub const TITLE: i32 = 32;
const MENU_Y: i32 = 32;
const CARD_Y: i32 = 70;
const CARD_H: i32 = 110;
const AREA_Y: i32 = CARD_Y + CARD_H;
const STATUS_H: i32 = 46;
/// Floating slider panels over the canvas backdrop.
const RAIL_X: i32 = 19;
const RAIL_W: u32 = 40;
/// Layers pane width (right of the canvas).
const LAYERS_W: u32 = 236;
/// Canvas margin inside the backdrop (logical).
const MARGIN: i64 = 24;
pub const ZOOMS: [u32; 12] = [
    125, 250, 500, 750, 1000, 2000, 3000, 4000, 5000, 6000, 7000, 8000,
];

/// Paint's 20 palette colours (two rows) as Paint 11 lists them.
pub const PALETTE: [Rgba; 20] = [
    Rgba::rgb(0, 0, 0),
    Rgba::rgb(127, 127, 127),
    Rgba::rgb(136, 0, 21),
    Rgba::rgb(237, 28, 36),
    Rgba::rgb(255, 127, 39),
    Rgba::rgb(255, 242, 0),
    Rgba::rgb(34, 177, 76),
    Rgba::rgb(0, 162, 232),
    Rgba::rgb(63, 72, 204),
    Rgba::rgb(163, 73, 164),
    Rgba::rgb(255, 255, 255),
    Rgba::rgb(195, 195, 195),
    Rgba::rgb(185, 122, 87),
    Rgba::rgb(255, 174, 201),
    Rgba::rgb(255, 201, 14),
    Rgba::rgb(239, 228, 176),
    Rgba::rgb(181, 230, 29),
    Rgba::rgb(153, 217, 234),
    Rgba::rgb(112, 146, 190),
    Rgba::rgb(200, 191, 231),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    FreeSelect,
    Pencil,
    Fill,
    Text,
    Eraser,
    Picker,
    Magnifier,
    Brush,
    Shape,
    Solid,
}
impl Tool {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Select => "Rectangle selection",
            Self::FreeSelect => "Free-form selection",
            Self::Pencil => "Pencil",
            Self::Fill => "Fill",
            Self::Text => "Text",
            Self::Eraser => "Eraser",
            Self::Picker => "Color picker",
            Self::Magnifier => "Magnifier",
            Self::Brush => "Brushes",
            Self::Shape => "Shapes",
            Self::Solid => "3D shapes",
        }
    }
    /// The left rail: (size slider, opacity slider).
    const fn rails(self) -> (bool, bool) {
        match self {
            Self::Pencil | Self::Brush | Self::Shape | Self::Text => (true, true),
            Self::Eraser => (true, false),
            Self::Fill => (true, true),
            _ => (false, false),
        }
    }
}

/// Open menus and flyouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    File,
    Edit,
    View,
    Selection,
    Rotate,
    Flip,
    Brushes,
    Outline,
    Fill,
    Stroke,
    Solids,
    Zoom,
    Colors,
    LayerMore(usize),
    Notice,
    /// A collapsed ribbon group's dropdown (`view::IMAGE`, ...).
    Group(u8),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dialog {
    /// Resize and skew: percentage or pixels, aspect locked.
    Resize {
        pixels: bool,
        h: String,
        v: String,
        lock: bool,
        field: u8,
    },
    /// Image properties: the canvas size in pixels.
    Properties {
        w: String,
        h: String,
        field: u8,
    },
    /// Open: image files found in the user's folders.
    Open {
        files: Vec<String>,
        selected: usize,
        first: usize,
    },
    /// Save as: a file name in Pictures.
    SaveAs {
        name: String,
    },
    /// Do you want to save your work? (on close or New/Open).
    Unsaved {
        then: After,
    },
    Settings,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum After {
    Close,
    New,
    Open,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    None,
    Stroke,
    Shape {
        x0: f32,
        y0: f32,
    },
    Bend {
        a: (f32, f32),
        b: (f32, f32),
    },
    Select {
        x0: i32,
        y0: i32,
    },
    Lasso,
    Move {
        dx: i32,
        dy: i32,
    },
    Slider {
        size: bool,
    },
    Spectrum,
    Hue,
    Resize {
        left: bool,
        top: bool,
        right: bool,
        bottom: bool,
    },
    /// A scrollbar thumb, grabbed this far (logical) from its start.
    ScrollX(i32),
    ScrollY(i32),
    Solid {
        x0: f32,
        y0: f32,
    },
}

/// A selection: its rectangle and, once moved, the lifted pixels.
/// A scroll bar: its track and its thumb.
type Bar = (view::Rect, view::Rect);
pub struct Selection {
    pub bounds: Bounds,
    /// Free-form coverage over `bounds` (None: the whole rectangle).
    pub mask: Option<Vec<u8>>,
    pub floating: Option<Floating>,
}
pub struct Floating {
    pub px: Vec<u8>,
    pub w: usize,
    pub h: usize,
    pub x: i32,
    pub y: i32,
    /// The layer before lifting (history) and the lifted rectangle.
    base: Vec<u8>,
    origin: Bounds,
}
pub struct TextBox {
    pub x: i32,
    pub y: i32,
    pub text: String,
    pub size: u32,
}
/// The colour editor's HSV.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsv {
    pub h: f32,
    pub s: f32,
    pub v: f32,
}

pub struct Paint {
    pub width: u32,
    pub height: u32,
    percent: u32,
    ready: bool,
    uid: u32,
    /// 0 light, 1 dark, 2 the system setting.
    pub theme: u8,
    pub doc: Doc,
    comp: Vec<u8>,
    pub history: History,
    pub tool: Tool,
    pub brush: Brush,
    pub shape: Shape,
    pub solid: Solid,
    pub outline_solid: bool,
    pub fill_solid: bool,
    pub stroke: u32,
    pub size_pencil: u32,
    pub size_brush: u32,
    pub size_eraser: u32,
    pub text_size: u32,
    pub tolerance: u32,
    /// Percent.
    pub opacity: u32,
    pub primary: Rgba,
    pub secondary: Rgba,
    pub editing_secondary: bool,
    pub custom: [Option<Rgba>; 10],
    /// Tenths of a percent (1000 = 100%: one canvas pixel per physical
    /// pixel, as Paint's 100%).
    pub zoom: u32,
    scroll: (i64, i64),
    edit: Option<Edit>,
    drag: Drag,
    pub hover: Option<(i32, i32)>,
    pub cursor: Option<(i32, i32)>,
    pub selection: Option<Selection>,
    lasso: Vec<(f32, f32)>,
    polygon: Vec<(f32, f32)>,
    clipboard: Option<(Vec<u8>, usize, usize)>,
    pub text: Option<TextBox>,
    pub menu: Option<Menu>,
    pub dialog: Option<Dialog>,
    pub layers_open: bool,
    pub gridlines: bool,
    pub rulers: bool,
    pub status_bar: bool,
    pub shapes_row: usize,
    pub path: Option<String>,
    identity: Option<door_service_protocol::files::FileInfo>,
    pub dirty: bool,
    title: String,
    pub notice: String,
    pub hsv: Hsv,
    /// A canvas resize in progress: the new bounds in canvas coordinates.
    pub resize_preview: Option<Bounds>,
    discard: bool,
    close_now: bool,
    close_declined: bool,
    solid_icons: Vec<(Solid, u32, Vec<u8>)>,
    modifiers_control: bool,
    modifiers_shift: bool,
    discard_after_save: bool,
    pending_after: After,
    /// The system clipboard: a copy to publish, a paste to fetch.
    sys_copy: bool,
    sys_paste: bool,
}

fn opaque_hsv(c: Rgba) -> Hsv {
    let (r, g, b) = (c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    Hsv {
        h: if h < 0.0 { h + 360.0 } else { h },
        s: if max == 0.0 { 0.0 } else { d / max },
        v: max,
    }
}
pub fn hsv_rgb(c: Hsv) -> Rgba {
    let x = c.v * c.s;
    let hp = (c.h % 360.0) / 60.0;
    let y = x * (1.0 - ((hp % 2.0) - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (x, y, 0.0),
        1 => (y, x, 0.0),
        2 => (0.0, x, y),
        3 => (0.0, y, x),
        4 => (y, 0.0, x),
        _ => (x, 0.0, y),
    };
    let m = c.v - x;
    let k = |v: f32| ((v + m) * 255.0 + 0.5) as u8;
    Rgba::rgb(k(r), k(g), k(b))
}

/// Fast square root (bit estimate and Newton steps).
pub(crate) fn sqrt_f(v: f32) -> f32 {
    if v <= 0.0 {
        return 0.0;
    }
    let mut x = f32::from_bits((v.to_bits() >> 1) + 0x1fbd_1df5);
    x = 0.5 * (x + v / x);
    x = 0.5 * (x + v / x);
    0.5 * (x + v / x)
}
/// floor and round without libm.
pub(crate) fn floor_f(v: f32) -> f32 {
    let t = v as i64 as f32;
    if t > v { t - 1.0 } else { t }
}
pub(crate) fn round_f(v: f32) -> f32 {
    floor_f(v + 0.5)
}
pub(crate) fn sin_cos_f(t: f32) -> (f32, f32) {
    let tau = core::f32::consts::TAU;
    let mut x = t % tau;
    if x > core::f32::consts::PI {
        x -= tau;
    } else if x < -core::f32::consts::PI {
        x += tau;
    }
    let series = |x: f32, mut term: f32, start: u32| {
        let mut sum = term;
        let mut n = start;
        for _ in 0..8 {
            term *= -x * x / ((n + 1) * (n + 2)) as f32;
            sum += term;
            n += 2;
        }
        sum
    };
    (series(x, x, 1), series(x, 1.0, 0))
}

impl Paint {
    pub fn new(uid: u32) -> Self {
        let (w, h) = (1152, 648);
        let mut p = Self {
            width: 1200,
            height: 660,
            percent: 100,
            ready: false,
            uid,
            theme: 2,
            doc: Doc::new(w, h),
            comp: Vec::new(),
            history: History::default(),
            tool: Tool::Brush,
            brush: Brush::Brush,
            shape: Shape::Rectangle,
            solid: Solid::Cube,
            outline_solid: true,
            fill_solid: false,
            stroke: 3,
            size_pencil: 1,
            size_brush: 8,
            size_eraser: 16,
            text_size: 24,
            tolerance: 10,
            opacity: 100,
            primary: Rgba::BLACK,
            secondary: Rgba::WHITE,
            editing_secondary: false,
            custom: [None; 10],
            zoom: 1000,
            scroll: (0, 0),
            edit: None,
            drag: Drag::None,
            hover: None,
            cursor: None,
            selection: None,
            lasso: Vec::new(),
            polygon: Vec::new(),
            clipboard: None,
            text: None,
            menu: None,
            dialog: None,
            layers_open: false,
            gridlines: false,
            rulers: false,
            status_bar: true,
            shapes_row: 0,
            path: None,
            identity: None,
            dirty: false,
            title: String::new(),
            notice: String::new(),
            hsv: Hsv {
                h: 0.0,
                s: 0.0,
                v: 0.0,
            },
            resize_preview: None,
            discard: false,
            close_now: false,
            close_declined: false,
            solid_icons: Vec::new(),
            modifiers_control: false,
            modifiers_shift: false,
            discard_after_save: false,
            pending_after: After::Close,
            sys_copy: false,
            sys_paste: false,
        };
        p.recompose();
        p.retitle();
        p
    }
    pub fn name(&self) -> &str {
        self.path
            .as_deref()
            .map_or("Untitled", |p| p.rsplit('/').next().unwrap_or(p))
    }
    fn retitle(&mut self) {
        self.title = format!("{} - Paint", self.name());
    }
    pub fn recompose(&mut self) {
        self.doc.composite(&mut self.comp);
    }
    pub fn composite(&self) -> &[u8] {
        &self.comp
    }
    fn mark_dirty(&mut self) {
        self.dirty = true;
        self.recompose();
    }
    fn home(&self) -> String {
        format!("/users/{}", self.uid)
    }

    // ---- geometry ----
    fn status_h(&self) -> i32 {
        if self.status_bar { STATUS_H } else { 0 }
    }
    /// The canvas backdrop (logical).
    pub fn area(&self) -> (i32, i32, u32, u32) {
        let right = if self.layers_open { LAYERS_W } else { 0 };
        let h = (self.height as i32 - AREA_Y - self.status_h()).max(0) as u32;
        (0, AREA_Y, self.width.saturating_sub(right), h)
    }
    fn phys(&self, logical: i64) -> i64 {
        logical * self.percent as i64 / 100
    }
    /// The canvas's physical size at the current zoom.
    fn canvas_size(&self) -> (i64, i64) {
        let (w, h) = (self.doc.w, self.doc.h);
        (
            (w as i64 * self.zoom as i64 / 1000).max(1),
            (h as i64 * self.zoom as i64 / 1000).max(1),
        )
    }
    /// Physical top-left of the canvas (centred when it fits, else
    /// scrolled).
    pub fn origin(&self) -> (i64, i64) {
        let (ax, ay, aw, ah) = self.area();
        let (ax, ay, aw, ah) = (
            self.phys(ax as i64),
            self.phys(ay as i64),
            self.phys(aw as i64),
            self.phys(ah as i64),
        );
        let m = self.phys(MARGIN);
        let (cw, ch) = self.canvas_size();
        let x = if cw + 2 * m <= aw {
            ax + (aw - cw) / 2
        } else {
            ax + m - self.scroll.0
        };
        let y = if ch + 2 * m <= ah {
            ay + (ah - ch) / 2
        } else {
            ay + m - self.scroll.1
        };
        (x, y)
    }
    fn clamp_scroll(&mut self) {
        let (_, _, aw, ah) = self.area();
        let (aw, ah) = (self.phys(aw as i64), self.phys(ah as i64));
        let m = self.phys(MARGIN);
        let (cw, ch) = self.canvas_size();
        self.scroll.0 = self.scroll.0.clamp(0, (cw + 2 * m - aw).max(0));
        self.scroll.1 = self.scroll.1.clamp(0, (ch + 2 * m - ah).max(0));
    }
    /// Canvas coordinates of a logical point.
    pub fn to_canvas(&self, x: i32, y: i32) -> (f32, f32) {
        let (ox, oy) = self.origin();
        let px = x as f32 * self.percent as f32 / 100.0;
        let py = y as f32 * self.percent as f32 / 100.0;
        (
            (px - ox as f32) * 1000.0 / self.zoom as f32,
            (py - oy as f32) * 1000.0 / self.zoom as f32,
        )
    }
    /// Logical point of a canvas coordinate.
    pub fn to_logical(&self, cx: f32, cy: f32) -> (i32, i32) {
        let (ox, oy) = self.origin();
        let px = ox as f32 + cx * self.zoom as f32 / 1000.0;
        let py = oy as f32 + cy * self.zoom as f32 / 1000.0;
        (
            (px * 100.0 / self.percent as f32) as i32,
            (py * 100.0 / self.percent as f32) as i32,
        )
    }
    fn over_canvas(&self, x: i32, y: i32) -> bool {
        let (ax, ay, aw, ah) = self.area();
        x >= ax && y >= ay && x < ax + aw as i32 && y < ay + ah as i32 && !self.over_rail(x, y)
    }
    pub fn rail(&self, size: bool) -> Option<(i32, i32, u32, u32)> {
        let (on_size, on_opacity) = self.tool.rails();
        if (size && !on_size) || (!size && !on_opacity) {
            return None;
        }
        let (_, ay, _, ah) = self.area();
        let both = on_size && on_opacity;
        // A short canvas puts the two panels side by side at full height,
        // as Paint does; a tall one stacks them.
        if (ah as i32) < 600 {
            let h = ah as i32 - 28;
            if h < 120 {
                return None;
            }
            let x = if size || !both {
                RAIL_X - 2
            } else {
                RAIL_X - 2 + RAIL_W as i32 + 10
            };
            return Some((x, ay + 14, RAIL_W, h as u32));
        }
        let top = ay + 40;
        let avail = ah as i32 - 82;
        let each = if both {
            (avail - 17) / 2
        } else {
            avail.min(360)
        };
        let y = if size || !both { top } else { top + each + 17 };
        Some((RAIL_X, y, RAIL_W, each as u32))
    }
    /// The canvas scrollbars when the canvas is larger than the view:
    /// (track, thumb) for horizontal and vertical (logical).
    pub fn scrollbars(&self) -> (Option<Bar>, Option<Bar>) {
        let (ax, ay, aw, ah) = self.area();
        let m = self.phys(MARGIN);
        let (cw, ch) = self.canvas_size();
        let (vw, vh) = (self.phys(aw as i64), self.phys(ah as i64));
        let bar = |len: u32, content: i64, view: i64, scroll: i64| -> Option<(i32, u32)> {
            let content = content + 2 * m;
            if content <= view || len < 40 {
                return None;
            }
            let thumb = ((len as i64 * view / content) as u32).max(24).min(len);
            let pos = ((len - thumb) as i64 * scroll / (content - view).max(1)) as i32;
            Some((pos, thumb))
        };
        let h = bar(aw.saturating_sub(28), cw, vw, self.scroll.0).map(|(pos, thumb)| {
            let track = view::Rect::new(ax + 8, ay + ah as i32 - 12, aw.saturating_sub(28), 10);
            (track, view::Rect::new(track.x + pos, track.y + 2, thumb, 6))
        });
        let v = bar(ah.saturating_sub(28), ch, vh, self.scroll.1).map(|(pos, thumb)| {
            let track = view::Rect::new(ax + aw as i32 - 12, ay + 8, 10, ah.saturating_sub(28));
            (track, view::Rect::new(track.x + 2, track.y + pos, 6, thumb))
        });
        (h, v)
    }
    fn scrollbar_hit(&self, x: i32, y: i32) -> Option<Drag> {
        let (h, v) = self.scrollbars();
        if let Some((track, thumb)) = h
            && track.contains(x, y)
        {
            return Some(Drag::ScrollX(if thumb.contains(x, y) {
                x - thumb.x
            } else {
                thumb.w as i32 / 2
            }));
        }
        if let Some((track, thumb)) = v
            && track.contains(x, y)
        {
            return Some(Drag::ScrollY(if thumb.contains(x, y) {
                y - thumb.y
            } else {
                thumb.h as i32 / 2
            }));
        }
        None
    }
    /// Move a scrollbar thumb's start to logical `at`.
    fn scroll_to(&mut self, horizontal: bool, at: i32) {
        let (h, v) = self.scrollbars();
        let m = self.phys(MARGIN);
        let (cw, ch) = self.canvas_size();
        let (_, _, aw, ah) = self.area();
        if horizontal && let Some((track, thumb)) = h {
            let span = (track.w - thumb.w).max(1) as i64;
            let f = (at - track.x).clamp(0, span as i32) as i64;
            self.scroll.0 = f * (cw + 2 * m - self.phys(aw as i64)) / span;
        } else if !horizontal && let Some((track, thumb)) = v {
            let span = (track.h - thumb.h).max(1) as i64;
            let f = (at - track.y).clamp(0, span as i32) as i64;
            self.scroll.1 = f * (ch + 2 * m - self.phys(ah as i64)) / span;
        }
        self.clamp_scroll();
    }
    fn over_rail(&self, x: i32, y: i32) -> bool {
        [true, false]
            .into_iter()
            .filter_map(|s| self.rail(s))
            .any(|(rx, ry, rw, rh)| x >= rx && y >= ry && x < rx + rw as i32 && y < ry + rh as i32)
    }
    /// The slider's track inside a rail: (x, top, bottom).
    pub fn track(&self, size: bool) -> Option<(i32, i32, i32)> {
        let (x, y, w, h) = self.rail(size)?;
        Some((x + w as i32 / 2, y + 44, y + h as i32 - 16))
    }
    pub fn size_value(&self) -> (u32, u32, u32) {
        match self.tool {
            Tool::Pencil => (self.size_pencil, 1, 32),
            Tool::Eraser => (self.size_eraser, 1, 100),
            Tool::Shape => (self.stroke, 1, 40),
            Tool::Text => (self.text_size, 8, 144),
            Tool::Fill => (self.tolerance, 0, 100),
            _ => (self.size_brush, 1, 100),
        }
    }
    fn set_size_value(&mut self, v: u32) {
        match self.tool {
            Tool::Pencil => self.size_pencil = v,
            Tool::Eraser => self.size_eraser = v,
            Tool::Shape => self.stroke = v,
            Tool::Text => {
                self.text_size = v;
                if let Some(t) = &mut self.text {
                    t.size = v;
                }
            }
            Tool::Fill => self.tolerance = v,
            _ => self.size_brush = v,
        }
    }
    fn slider_at(&mut self, size: bool, y: i32) {
        let Some((_, top, bottom)) = self.track(size) else {
            return;
        };
        let t = 1.0 - ((y - top) as f32 / (bottom - top).max(1) as f32).clamp(0.0, 1.0);
        if size {
            let (_, lo, hi) = self.size_value();
            // Sizes follow a square curve: fine control at small sizes.
            let v = lo as f32 + (hi - lo) as f32 * t * t;
            self.set_size_value(round_f(v) as u32);
        } else {
            self.opacity = round_f(t * 100.0) as u32;
        }
    }
    pub fn slider_fraction(&self, size: bool) -> f32 {
        if size {
            let (v, lo, hi) = self.size_value();
            sqrt_f((v.saturating_sub(lo)) as f32 / (hi - lo).max(1) as f32)
        } else {
            self.opacity as f32 / 100.0
        }
    }

    // ---- colours ----
    fn active_color(&self) -> Rgba {
        if self.editing_secondary {
            self.secondary
        } else {
            self.primary
        }
    }
    fn set_active_color(&mut self, c: Rgba) {
        if self.editing_secondary {
            self.secondary = c;
        } else {
            self.primary = c;
        }
    }
    fn alpha(&self) -> u8 {
        (self.opacity.min(100) * 255 / 100) as u8
    }

    // ---- selection ----
    /// Commit a moved selection (or a text box) into the layer.
    pub fn commit(&mut self) {
        self.commit_text();
        if let Some(sel) = self.selection.take()
            && let Some(f) = sel.floating
        {
            let b = Bounds::new(f.x, f.y, f.x + f.w as i32, f.y + f.h as i32);
            raster::stamp_tile(&mut self.doc, f.x, f.y, f.w, f.h, &f.px);
            self.history.patch(&self.doc, &f.base, f.origin.union(b));
            self.mark_dirty();
        }
        self.polygon.clear();
    }
    fn erase_rect(&mut self, b: Bounds, mask: Option<&[u8]>) {
        let bg = self.doc.active == 0;
        let fill = self.secondary.bgra();
        let w = self.doc.w;
        let layer = self.doc.layer_mut();
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let k = mask.map_or(255, |m| {
                    m[(y - b.y0) as usize * b.width() + (x - b.x0) as usize] as u32
                });
                if k == 0 {
                    continue;
                }
                let at = (y as usize * w + x as usize) * 4;
                let d = &mut layer[at..at + 4];
                for c in 0..4 {
                    let target = if bg { fill[c] as u32 } else { 0 };
                    d[c] = ((d[c] as u32 * (255 - k) + target * k) / 255) as u8;
                }
            }
        }
    }
    /// Lift the selection's pixels so they move.
    fn lift(&mut self) {
        let Some(sel) = self.selection.as_mut() else {
            return;
        };
        if sel.floating.is_some() {
            return;
        }
        let b = sel.bounds;
        let base = self.doc.layer().to_vec();
        let mut px = raster::crop(&base, self.doc.w, b);
        if let Some(m) = &sel.mask {
            for (p, k) in px.chunks_exact_mut(4).zip(m) {
                for c in p.iter_mut() {
                    *c = (*c as u32 * *k as u32 / 255) as u8;
                }
            }
        }
        let mask = sel.mask.clone();
        sel.floating = Some(Floating {
            px,
            w: b.width(),
            h: b.height(),
            x: b.x0,
            y: b.y0,
            base,
            origin: b,
        });
        self.erase_rect(b, mask.as_deref());
        self.recompose();
    }
    pub fn select_all(&mut self) {
        self.commit();
        self.tool = Tool::Select;
        self.selection = Some(Selection {
            bounds: Bounds::new(0, 0, self.doc.w as i32, self.doc.h as i32),
            mask: None,
            floating: None,
        });
    }
    pub fn delete_selection(&mut self) {
        let Some(sel) = self.selection.take() else {
            return;
        };
        match sel.floating {
            Some(f) => self.history.patch(&self.doc, &f.base, f.origin),
            None => {
                let base = self.doc.layer().to_vec();
                self.erase_rect(sel.bounds, sel.mask.as_deref());
                self.history.patch(&self.doc, &base, sel.bounds);
            }
        }
        self.mark_dirty();
    }
    pub fn copy(&mut self) {
        let Some(sel) = &self.selection else {
            return;
        };
        self.clipboard = Some(match &sel.floating {
            Some(f) => (f.px.clone(), f.w, f.h),
            None => {
                let mut px = raster::crop(self.doc.layer(), self.doc.w, sel.bounds);
                if let Some(m) = &sel.mask {
                    for (p, k) in px.chunks_exact_mut(4).zip(m) {
                        for c in p.iter_mut() {
                            *c = (*c as u32 * *k as u32 / 255) as u8;
                        }
                    }
                }
                (px, sel.bounds.width(), sel.bounds.height())
            }
        });
        self.sys_copy = true;
    }
    pub fn paste(&mut self) {
        let Some((px, w, h)) = self.clipboard.clone() else {
            return;
        };
        self.commit();
        // Grow the canvas when the pasted image is larger (as Paint asks).
        if w > self.doc.w || h > self.doc.h {
            self.resize_canvas(w.max(self.doc.w), h.max(self.doc.h));
        }
        self.tool = Tool::Select;
        let base = self.doc.layer().to_vec();
        let b = Bounds::new(0, 0, w as i32, h as i32);
        self.selection = Some(Selection {
            bounds: b,
            mask: None,
            floating: Some(Floating {
                px,
                w,
                h,
                x: 0,
                y: 0,
                base,
                origin: Bounds::new(0, 0, 0, 0),
            }),
        });
        self.dirty = true;
    }
    pub fn crop(&mut self) {
        let Some(b) = self.selection.as_ref().map(|s| s.bounds) else {
            return;
        };
        self.commit();
        let before = raster::Snapshot::of(&self.doc);
        for l in &mut self.doc.layers {
            l.px = raster::crop(&l.px, self.doc.w, b);
        }
        self.doc.w = b.width();
        self.doc.h = b.height();
        self.history.push(raster::Change::Whole {
            before,
            after: raster::Snapshot::of(&self.doc),
        });
        self.mark_dirty();
    }
    pub fn resize_canvas(&mut self, w: usize, h: usize) {
        self.resize_canvas_to(Bounds::new(0, 0, w as i32, h as i32));
    }
    pub fn turn(&mut self, turn: Turn) {
        if let Some(sel) = self.selection.as_mut() {
            if sel.floating.is_none() {
                self.lift();
            }
            if let Some(f) = self.selection.as_mut().and_then(|s| s.floating.as_mut()) {
                let (px, w, h) = raster::transform(&f.px, f.w, f.h, turn);
                f.px = px;
                f.w = w;
                f.h = h;
                self.dirty = true;
            }
            return;
        }
        self.commit();
        let before = raster::Snapshot::of(&self.doc);
        let (w, h) = (self.doc.w, self.doc.h);
        let mut size = (w, h);
        for l in &mut self.doc.layers {
            let (px, nw, nh) = raster::transform(&l.px, w, h, turn);
            l.px = px;
            size = (nw, nh);
        }
        (self.doc.w, self.doc.h) = size;
        self.history.push(raster::Change::Whole {
            before,
            after: raster::Snapshot::of(&self.doc),
        });
        self.clamp_scroll();
        self.mark_dirty();
    }
    pub fn resample_to(&mut self, w: usize, h: usize) {
        let (w, h) = (w.clamp(1, 4096), h.clamp(1, 4096));
        if w * h > door_workspace::explorer_preview::MAX_PIXELS * 2 {
            self.notice = "That size is larger than Paint can hold here.".into();
            self.menu = Some(Menu::Notice);
            return;
        }
        if let Some(f) = self.selection.as_mut().and_then(|s| s.floating.as_mut()) {
            f.px = raster::resample(&f.px, f.w, f.h, w, h);
            f.w = w;
            f.h = h;
            self.dirty = true;
            return;
        }
        self.commit();
        let before = raster::Snapshot::of(&self.doc);
        let (ow, oh) = (self.doc.w, self.doc.h);
        for l in &mut self.doc.layers {
            l.px = raster::resample(&l.px, ow, oh, w, h);
        }
        self.doc.w = w;
        self.doc.h = h;
        self.history.push(raster::Change::Whole {
            before,
            after: raster::Snapshot::of(&self.doc),
        });
        self.clamp_scroll();
        self.mark_dirty();
    }

    // ---- text ----
    fn commit_text(&mut self) {
        let Some(t) = self.text.take() else {
            return;
        };
        if t.text.trim().is_empty() {
            return;
        }
        let Some(system) = door_workspace::assets::text_system() else {
            return;
        };
        let base = self.doc.layer().to_vec();
        let mut c = self.primary;
        c.a = self.alpha();
        let color = c.bgra();
        let (w, h) = (self.doc.w, self.doc.h);
        let format = system.format(t.size as f32, 400, 1.0, "en-US");
        let line_h = (t.size as f32 * 1.33) as i32;
        let mut right = t.x;
        let mut lines = 0;
        for (i, line) in t.text.split('\n').enumerate() {
            lines = i + 1;
            if line.is_empty() {
                continue;
            }
            let Ok(layout) = system.layout_line(line, &format) else {
                continue;
            };
            right = right.max(t.x + layout.physical_size().0 as i32);
            let layer = self.doc.layer_mut();
            if let Ok(mut target) =
                door_graphics_text::Target::new(layer, w as u32, h as u32, w * 4)
            {
                let clip = door_graphics_text::Clip::new(0, 0, w as u32, h as u32);
                let _ = system.draw(
                    &layout,
                    &mut target,
                    clip,
                    t.x,
                    t.y + i as i32 * line_h,
                    color,
                );
            }
        }
        let b = Bounds::new(t.x - 2, t.y - 2, right + 4, t.y + lines as i32 * line_h + 4);
        self.history.patch(&self.doc, &base, b);
        self.mark_dirty();
    }

    // ---- zoom ----
    pub fn set_zoom(&mut self, zoom: u32, anchor: Option<(i32, i32)>) {
        let zoom = zoom.clamp(ZOOMS[0], ZOOMS[ZOOMS.len() - 1]);
        let (ax, ay, aw, ah) = self.area();
        let (lx, ly) = anchor.unwrap_or((ax + aw as i32 / 2, ay + ah as i32 / 2));
        let (cx, cy) = self.to_canvas(lx, ly);
        self.zoom = zoom;
        // Keep the canvas point under the anchor.
        let (ox, oy) = self.origin();
        let want_x = self.phys(lx as i64) - (cx * zoom as f32 / 1000.0) as i64;
        let want_y = self.phys(ly as i64) - (cy * zoom as f32 / 1000.0) as i64;
        self.scroll.0 += ox - want_x;
        self.scroll.1 += oy - want_y;
        self.clamp_scroll();
    }
    pub fn zoom_step(&mut self, up: bool, anchor: Option<(i32, i32)>) {
        let next = if up {
            ZOOMS
                .iter()
                .copied()
                .find(|z| *z > self.zoom)
                .unwrap_or(self.zoom)
        } else {
            ZOOMS
                .iter()
                .rev()
                .copied()
                .find(|z| *z < self.zoom)
                .unwrap_or(self.zoom)
        };
        self.set_zoom(next, anchor);
    }
    pub fn fit(&mut self) {
        let (_, _, aw, ah) = self.area();
        let (aw, ah) = (
            self.phys(aw as i64 - 2 * MARGIN),
            self.phys(ah as i64 - 2 * MARGIN),
        );
        let z = (aw * 1000 / self.doc.w as i64)
            .min(ah * 1000 / self.doc.h as i64)
            .clamp(ZOOMS[0] as i64, ZOOMS[ZOOMS.len() - 1] as i64);
        self.zoom = z as u32;
        self.scroll = (0, 0);
    }

    // ---- 3D icons ----
    fn refresh_solid_icons(&mut self) {
        let px = 20 * self.percent.div_ceil(100).clamp(1, 3);
        if self.solid_icons.first().is_some_and(|i| i.1 == px) {
            return;
        }
        self.solid_icons = Solid::ALL
            .iter()
            .map(|s| {
                (
                    *s,
                    px,
                    raster::render_solid(*s, px as usize, px as usize, Rgba::rgb(0, 120, 212)),
                )
            })
            .collect();
    }
    pub fn solid_icon(&self, s: Solid) -> Option<(u32, &[u8])> {
        self.solid_icons
            .iter()
            .find(|i| i.0 == s)
            .map(|i| (i.1, i.2.as_slice()))
    }

    // ---- files ----
    fn new_image(&mut self) {
        self.commit();
        let (w, h) = self.fitted_canvas();
        self.doc = Doc::new(w, h);
        self.history.clear();
        self.path = None;
        self.identity = None;
        self.dirty = false;
        self.zoom = 1000;
        self.scroll = (0, 0);
        self.recompose();
        self.retitle();
    }
    /// A new canvas that fills the view at 100%.
    fn fitted_canvas(&self) -> (usize, usize) {
        let (_, _, aw, ah) = self.area();
        let w = (self.phys(aw as i64 - 2 * MARGIN) as usize).clamp(64, 1920);
        let h = (self.phys(ah as i64 - 2 * MARGIN) as usize).clamp(64, 1080);
        (w, h)
    }
    fn scan_images(&mut self, s: &mut Storage) -> Vec<String> {
        let mut out = Vec::new();
        for folder in [
            "Pictures",
            "Pictures/Screenshots",
            "Desktop",
            "Documents",
            "Downloads",
        ] {
            let path = format!("{}/{folder}", self.home());
            let mut after = 0;
            while let Ok(Some(e)) = s.list(&path, after) {
                after = e.cookie;
                let name: String = core::char::decode_utf16(e.name[..e.name_len].iter().copied())
                    .map(|c| c.unwrap_or('�'))
                    .collect();
                let lower = name.to_ascii_lowercase();
                if [".png", ".jpg", ".jpeg", ".bmp", ".qoi"]
                    .iter()
                    .any(|x| lower.ends_with(x))
                    && !e.info.directory
                {
                    out.push(format!("{path}/{name}"));
                }
                if out.len() >= 200 {
                    return out;
                }
            }
        }
        out
    }
    fn open_path(&mut self, s: &mut Storage, path: &str) -> Result<(), Error> {
        let bytes = read_file(s, path, door_workspace::explorer_preview::MAX_VIEW_INPUT)?;
        match door_workspace::explorer_preview::decode_full(&bytes) {
            Ok(img) => {
                self.commit();
                let mut doc = Doc::new(img.width, img.height);
                // Paint shows transparency over white on the background.
                let mut px = img.bgra;
                for p in px.chunks_exact_mut(4) {
                    let ia = 255 - p[3] as u32;
                    for c in p.iter_mut().take(3) {
                        *c = (*c as u32 + ia).min(255) as u8;
                    }
                    p[3] = 255;
                }
                doc.layers[0].px = px;
                self.doc = doc;
                self.history.clear();
                self.path = Some(path.into());
                self.identity = None;
                self.dirty = false;
                self.scroll = (0, 0);
                self.zoom = 1000;
                self.recompose();
                if self.canvas_size().0 > self.phys(self.area().2 as i64) {
                    self.fit();
                }
                self.retitle();
                door_workspace::recent::note_opened(s, path, door_workspace::recent::Kind::File);
                Ok(())
            }
            Err(message) => {
                self.notice = format!("Paint can't read this file. {message}");
                self.menu = Some(Menu::Notice);
                Ok(())
            }
        }
    }
    /// Save as PNG to `path` (a new file, or replacing the one opened).
    pub fn save_to(&mut self, s: &mut Storage, path: &str) -> Result<(), Error> {
        self.commit();
        let image = door_workspace::explorer_preview::Image {
            width: self.doc.w,
            height: self.doc.h,
            source_width: self.doc.w,
            source_height: self.doc.h,
            bgra: self.doc.flatten(),
        };
        let png = door_workspace::explorer_preview::encode_png(&image)
            .map_err(|_| Error::Service(door_service_protocol::Code::Unsupported))?;
        if let Some((dir, _)) = path.rsplit_once('/') {
            crate::ensure_dirs(s, dir);
        }
        let same = self.path.as_deref() == Some(path);
        let info = write_file(s, path, &png, if same { self.identity } else { None })?;
        self.identity = Some(info);
        self.path = Some(path.into());
        self.dirty = false;
        self.retitle();
        door_rt::diagnostic(format_args!(
            "DOOR_INBOX PAINT saved bytes={} size={}x{}\n",
            png.len(),
            self.doc.w,
            self.doc.h
        ));
        Ok(())
    }
    pub fn save(&mut self, s: &mut Storage) -> Result<(), Error> {
        match self.path.clone() {
            Some(p) if p.to_ascii_lowercase().ends_with(".png") => self.save_to(s, &p),
            _ => {
                self.dialog = Some(Dialog::SaveAs {
                    name: self.unique_name(s),
                });
                Ok(())
            }
        }
    }
    fn unique_name(&self, s: &mut Storage) -> String {
        let dir = format!("{}/Pictures", self.home());
        let mut taken = |n: &str| s.stat(&format!("{dir}/{n}")).is_ok();
        let stem = self.name().rsplit_once('.').map_or(self.name(), |(a, _)| a);
        let stem = String::from(stem);
        let mut name = format!("{stem}.png");
        let mut i = 2;
        while taken(&name) && i < 1000 {
            name = format!("{stem} ({i}).png");
            i += 1;
        }
        name
    }
    fn after_unsaved(&mut self, then: After, s: &mut Storage) {
        match then {
            After::Close => {
                self.discard = true;
                self.close_now = true;
            }
            After::New => self.new_image(),
            After::Open => {
                let files = self.scan_images(s);
                self.dialog = Some(Dialog::Open {
                    files,
                    selected: 0,
                    first: 0,
                });
            }
        }
    }
}

/// Read a whole file (bounded).
fn read_file(s: &mut Storage, path: &str, limit: usize) -> Result<Vec<u8>, Error> {
    use door_service_client::{Disposition, READ, SHARE_ALL};
    let file = s.open(path, Disposition::OpenExisting, READ, SHARE_ALL)?;
    let result = (|| {
        let info = s.info(&file)?;
        if info.directory || info.bytes as usize > limit {
            return Err(Error::Service(door_service_protocol::Code::Resources));
        }
        let mut bytes = vec![0u8; info.bytes as usize];
        let mut at = 0;
        while at < bytes.len() {
            let end = (at + 192).min(bytes.len());
            let n = s.read(&file, at as u64, &mut bytes[at..end])?;
            if n == 0 {
                break;
            }
            at += n;
        }
        bytes.truncate(at);
        Ok(bytes)
    })();
    let _ = s.close(file);
    result
}
/// Create or replace a file atomically.
fn write_file(
    s: &mut Storage,
    path: &str,
    bytes: &[u8],
    identity: Option<door_service_protocol::files::FileInfo>,
) -> Result<door_service_protocol::files::FileInfo, Error> {
    use door_service_client::{Disposition, READ, SHARE_ALL, WRITE};
    let file = s.open(
        path,
        if identity.is_some() {
            Disposition::OpenExisting
        } else {
            Disposition::OpenAlways
        },
        READ | WRITE,
        SHARE_ALL,
    )?;
    let result = (|| {
        let expected = match identity {
            Some(i) => i,
            None => s.info(&file)?,
        };
        let prepared = door_workspace::document_save::prepare(s, &file, expected, bytes)?;
        door_workspace::document_save::commit(s, prepared)
    })();
    let closed = s.close(file);
    let info = result?;
    closed?;
    Ok(info)
}

/// A canvas resize handle: its logical centre and the edges it moves
/// (left, top, right, bottom).
pub type Handle = ((i32, i32), (bool, bool, bool, bool));
/// Target ids.
pub mod id {
    pub const MENU: u32 = 1; // File, Edit, View
    pub const SAVE: u32 = 5;
    pub const SHARE: u32 = 6;
    pub const UNDO: u32 = 7;
    pub const REDO: u32 = 8;
    pub const FEEDBACK: u32 = 9;
    pub const SETTINGS: u32 = 10;
    pub const SELECT: u32 = 20;
    pub const SELECT_MENU: u32 = 21;
    pub const CROP: u32 = 22;
    pub const ROTATE: u32 = 23;
    pub const ROTATE_MENU: u32 = 24;
    pub const REMOVE_BG: u32 = 25;
    pub const FLIP: u32 = 26;
    pub const FLIP_MENU: u32 = 27;
    pub const RESIZE: u32 = 28;
    pub const TOOL: u32 = 30; // pencil, fill, text, eraser, picker, magnifier
    pub const BRUSH: u32 = 40;
    pub const BRUSH_MENU: u32 = 41;
    pub const SHAPE: u32 = 50; // 50..=72 by gallery index
    pub const SHAPES_UP: u32 = 75;
    pub const SHAPES_DOWN: u32 = 76;
    pub const OUTLINE: u32 = 77;
    pub const FILL: u32 = 78;
    pub const STROKE: u32 = 79;
    pub const PRIMARY: u32 = 80;
    pub const SECONDARY: u32 = 81;
    pub const PALETTE: u32 = 82; // 82..=101
    pub const CUSTOM: u32 = 102; // 102..=111
    pub const EDIT_COLORS: u32 = 112;
    pub const SOLIDS: u32 = 113;
    pub const LAYERS: u32 = 114;
    /// Collapsed groups: GROUP + the group bit's index (Image, Tools,
    /// Shapes, Colors: 115..=118).
    pub const GROUP: u32 = 115;
    pub const FIT: u32 = 120;
    pub const ZOOM_MENU: u32 = 121;
    pub const ZOOM_OUT: u32 = 122;
    pub const ZOOM_IN: u32 = 123;
    pub const ZOOM_SLIDER: u32 = 124;
    /// Menu and flyout rows: 200 + row.
    pub const ITEM: u32 = 200;
    pub const LAYER_ADD: u32 = 300;
    pub const LAYER: u32 = 310; // 310..=315 rows
    pub const LAYER_EYE: u32 = 320; // 320..=325
    pub const LAYER_MORE: u32 = 330; // 330..=335
    /// Dialog controls: 400 + index.
    pub const DIALOG: u32 = 400;
    /// Clicks that only close a menu.
    pub const BACKDROP: u32 = 499;
}

/// A menu row: label, accelerator, enabled, checked.
pub struct Row {
    pub label: &'static str,
    pub key: &'static str,
    pub enabled: bool,
    pub checked: bool,
    pub separator_before: bool,
    pub icon: Option<&'static str>,
}
const fn row(label: &'static str, key: &'static str, enabled: bool) -> Row {
    Row {
        label,
        key,
        enabled,
        checked: false,
        separator_before: false,
        icon: None,
    }
}

impl Paint {
    /// The rows of a list menu.
    pub fn rows(&self, menu: Menu) -> Vec<Row> {
        let sel = self.selection.is_some();
        let sep = |mut r: Row| {
            r.separator_before = true;
            r
        };
        let check = |mut r: Row, on: bool| {
            r.checked = on;
            r
        };
        match menu {
            Menu::File => vec![
                row("New", "Ctrl+N", true),
                row("Open", "Ctrl+O", true),
                sep(row("Save", "Ctrl+S", true)),
                row("Save as", "Ctrl+Shift+S", true),
                sep(row("Image properties", "Ctrl+E", true)),
                sep(row("Exit", "", true)),
            ],
            Menu::Edit => vec![
                row("Undo", "Ctrl+Z", self.history.can_undo()),
                row("Redo", "Ctrl+Y", self.history.can_redo()),
                sep(row("Cut", "Ctrl+X", sel)),
                row("Copy", "Ctrl+C", sel),
                row("Paste", "Ctrl+V", true),
                sep(row("Select all", "Ctrl+A", true)),
                row("Delete selection", "Delete", sel),
            ],
            Menu::View => vec![
                row("Zoom in", "Ctrl+Plus", self.zoom < ZOOMS[ZOOMS.len() - 1]),
                row("Zoom out", "Ctrl+Minus", self.zoom > ZOOMS[0]),
                row("Fit to window", "Ctrl+Shift+0", true),
                row("Actual size", "Ctrl+0", true),
                sep(check(row("Rulers", "Ctrl+R", true), self.rulers)),
                check(row("Gridlines", "Ctrl+G", true), self.gridlines),
                check(row("Status bar", "", true), self.status_bar),
            ],
            Menu::Selection => vec![
                check(row("Rectangle", "", true), self.tool == Tool::Select),
                check(row("Free-form", "", true), self.tool == Tool::FreeSelect),
                sep(row("Select all", "Ctrl+A", true)),
                row("Delete", "Delete", sel),
            ],
            Menu::Rotate => vec![
                row("Rotate right 90°", "", true),
                row("Rotate left 90°", "", true),
                row("Rotate 180°", "", true),
            ],
            Menu::Flip => vec![
                row("Flip vertical", "", true),
                row("Flip horizontal", "", true),
            ],
            Menu::Outline => vec![
                check(row("No outline", "", true), !self.outline_solid),
                check(row("Solid color", "", true), self.outline_solid),
            ],
            Menu::Fill => vec![
                check(row("No fill", "", true), !self.fill_solid),
                check(row("Solid color", "", true), self.fill_solid),
            ],
            Menu::Zoom => {
                let mut v: Vec<Row> = ZOOM_LABELS.iter().map(|l| row(l, "", true)).collect();
                for (r, z) in v.iter_mut().zip(ZOOMS) {
                    r.checked = z == self.zoom;
                }
                v.push(sep(row("Fit to window", "", true)));
                v
            }
            Menu::LayerMore(i) => vec![
                row(
                    "Duplicate layer",
                    "",
                    self.doc.layers.len() < raster::MAX_LAYERS,
                ),
                row("Merge down", "", i > 0),
                row("Delete layer", "", self.doc.layers.len() > 1),
            ],
            _ => Vec::new(),
        }
    }
    fn menu_action(&mut self, menu: Menu, index: usize, s: &mut Storage) -> Result<(), Error> {
        self.menu = None;
        match (menu, index) {
            (Menu::File, 0) => {
                if self.dirty {
                    self.dialog = Some(Dialog::Unsaved { then: After::New });
                } else {
                    self.new_image();
                }
            }
            (Menu::File, 1) => {
                if self.dirty {
                    self.dialog = Some(Dialog::Unsaved { then: After::Open });
                } else {
                    let files = self.scan_images(s);
                    self.dialog = Some(Dialog::Open {
                        files,
                        selected: 0,
                        first: 0,
                    });
                }
            }
            (Menu::File, 2) => self.save(s)?,
            (Menu::File, 3) => {
                self.dialog = Some(Dialog::SaveAs {
                    name: self.unique_name(s),
                })
            }
            (Menu::File, 4) => {
                self.dialog = Some(Dialog::Properties {
                    w: format!("{}", self.doc.w),
                    h: format!("{}", self.doc.h),
                    field: 0,
                })
            }
            (Menu::File, 5) => {
                self.close_now = !self.dirty || {
                    self.dialog = Some(Dialog::Unsaved { then: After::Close });
                    false
                }
            }
            (Menu::Edit, 0) => self.undo(),
            (Menu::Edit, 1) => self.redo(),
            (Menu::Edit, 2) => {
                self.copy();
                self.delete_selection();
            }
            (Menu::Edit, 3) => self.copy(),
            (Menu::Edit, 4) => self.sys_paste = true,
            (Menu::Edit, 5) | (Menu::Selection, 2) => self.select_all(),
            (Menu::Edit, 6) | (Menu::Selection, 3) => self.delete_selection(),
            (Menu::View, 0) => self.zoom_step(true, None),
            (Menu::View, 1) => self.zoom_step(false, None),
            (Menu::View, 2) | (Menu::Zoom, 12) => self.fit(),
            (Menu::View, 3) => self.set_zoom(1000, None),
            (Menu::View, 4) => self.rulers = !self.rulers,
            (Menu::View, 5) => self.gridlines = !self.gridlines,
            (Menu::View, 6) => {
                self.status_bar = !self.status_bar;
                self.clamp_scroll();
            }
            (Menu::Selection, 0) => self.pick_tool(Tool::Select),
            (Menu::Selection, 1) => self.pick_tool(Tool::FreeSelect),
            (Menu::Rotate, 0) => self.turn(Turn::Right),
            (Menu::Rotate, 1) => self.turn(Turn::Left),
            (Menu::Rotate, 2) => self.turn(Turn::Half),
            (Menu::Flip, 0) => self.turn(Turn::FlipVertical),
            (Menu::Flip, 1) => self.turn(Turn::FlipHorizontal),
            (Menu::Outline, i) => self.outline_solid = i == 1,
            (Menu::Fill, i) => self.fill_solid = i == 1,
            (Menu::Zoom, i) if i < ZOOMS.len() => self.set_zoom(ZOOMS[i], None),
            (Menu::Brushes, i) if i < Brush::ALL.len() => {
                self.brush = Brush::ALL[i];
                self.pick_tool(Tool::Brush);
            }
            (Menu::Stroke, i) => self.stroke = [1, 3, 5, 8][i.min(3)],
            (Menu::Solids, i) if i < Solid::ALL.len() => {
                self.solid = Solid::ALL[i];
                self.pick_tool(Tool::Solid);
            }
            (Menu::LayerMore(l), 0) => self.duplicate_layer(l),
            (Menu::LayerMore(l), 1) => self.merge_down(l),
            (Menu::LayerMore(l), 2) => self.delete_layer(l),
            _ => {}
        }
        Ok(())
    }
    pub fn undo(&mut self) {
        self.commit();
        if self.history.undo(&mut self.doc) {
            self.clamp_scroll();
            self.mark_dirty();
        }
    }
    pub fn redo(&mut self) {
        self.commit();
        if self.history.redo(&mut self.doc) {
            self.clamp_scroll();
            self.mark_dirty();
        }
    }
    pub fn pick_tool(&mut self, tool: Tool) {
        if tool != self.tool {
            self.commit();
        }
        self.tool = tool;
    }

    // ---- layers ----
    fn whole<F: FnOnce(&mut Doc)>(&mut self, f: F) {
        self.commit();
        let before = raster::Snapshot::of(&self.doc);
        f(&mut self.doc);
        self.history.push(raster::Change::Whole {
            before,
            after: raster::Snapshot::of(&self.doc),
        });
        self.mark_dirty();
    }
    pub fn add_layer(&mut self) {
        if self.doc.layers.len() >= raster::MAX_LAYERS {
            return;
        }
        self.whole(|d| {
            let px = vec![0; d.w * d.h * 4];
            d.layers
                .insert(d.active + 1, raster::Layer { px, visible: true });
            d.active += 1;
        });
    }
    fn duplicate_layer(&mut self, i: usize) {
        if self.doc.layers.len() >= raster::MAX_LAYERS {
            return;
        }
        self.whole(|d| {
            let px = d.layers[i].px.clone();
            d.layers.insert(i + 1, raster::Layer { px, visible: true });
            d.active = i + 1;
        });
    }
    fn merge_down(&mut self, i: usize) {
        if i == 0 {
            return;
        }
        self.whole(|d| {
            let top = d.layers.remove(i);
            if top.visible {
                for (dst, s) in d.layers[i - 1]
                    .px
                    .chunks_exact_mut(4)
                    .zip(top.px.chunks_exact(4))
                {
                    raster::over(dst, [s[0], s[1], s[2], s[3]]);
                }
            }
            d.active = i - 1;
        });
    }
    fn delete_layer(&mut self, i: usize) {
        if self.doc.layers.len() <= 1 {
            return;
        }
        self.whole(|d| {
            d.layers.remove(i);
            d.active = d.active.min(d.layers.len() - 1);
        });
    }

    // ---- canvas input ----
    fn canvas_down(&mut self, x: i32, y: i32) {
        let (cx, cy) = self.to_canvas(x, y);
        let (ix, iy) = (floor_f(cx) as i32, floor_f(cy) as i32);
        // Canvas resize handles (right edge, bottom edge, corner).
        if let Some((left, top, right, bottom)) = self.handle_at(x, y) {
            self.commit();
            self.drag = Drag::Resize {
                left,
                top,
                right,
                bottom,
            };
            self.resize_preview = Some(Bounds::new(0, 0, self.doc.w as i32, self.doc.h as i32));
            return;
        }
        match self.tool {
            Tool::Select | Tool::FreeSelect => {
                let inside = self.selection.as_ref().is_some_and(|s| {
                    let b = match &s.floating {
                        Some(f) => Bounds::new(f.x, f.y, f.x + f.w as i32, f.y + f.h as i32),
                        None => s.bounds,
                    };
                    ix >= b.x0 && iy >= b.y0 && ix < b.x1 && iy < b.y1
                });
                if inside {
                    self.lift();
                    if let Some(f) = self.selection.as_ref().and_then(|s| s.floating.as_ref()) {
                        self.drag = Drag::Move {
                            dx: ix - f.x,
                            dy: iy - f.y,
                        };
                    }
                } else {
                    self.commit();
                    if self.tool == Tool::Select {
                        self.drag = Drag::Select {
                            x0: ix.clamp(0, self.doc.w as i32),
                            y0: iy.clamp(0, self.doc.h as i32),
                        };
                    } else {
                        self.lasso.clear();
                        self.lasso.push((cx, cy));
                        self.drag = Drag::Lasso;
                    }
                }
            }
            Tool::Pencil | Tool::Brush | Tool::Eraser => {
                let (tip, size) = match self.tool {
                    Tool::Pencil => (Tip::Pencil, self.size_pencil),
                    Tool::Eraser => (Tip::Eraser, self.size_eraser),
                    _ => (Tip::Brush(self.brush), self.size_brush),
                };
                let erase_to =
                    (self.tool == Tool::Eraser && self.doc.active == 0).then_some(self.secondary);
                let opacity = if self.tool == Tool::Eraser {
                    255
                } else {
                    self.alpha()
                };
                let mut e = Edit::begin(&self.doc, tip, size, self.primary, opacity, erase_to);
                let (sx, sy) = if self.tool == Tool::Pencil {
                    (ix as f32 + 0.5, iy as f32 + 0.5)
                } else {
                    (cx, cy)
                };
                e.start(&mut self.doc, sx, sy);
                self.edit = Some(e);
                self.drag = Drag::Stroke;
                self.recompose();
            }
            Tool::Fill => {
                let base = self.doc.layer().to_vec();
                let alpha = self.alpha();
                if let Some(b) =
                    raster::flood(&mut self.doc, ix, iy, self.primary, self.tolerance, alpha)
                {
                    self.history.patch(&self.doc, &base, b);
                    self.mark_dirty();
                }
            }
            Tool::Picker => {
                if let Some(p) = self.doc.pixel(ix, iy) {
                    let mut c = Rgba::from_bgra(p);
                    c.a = 255;
                    self.set_active_color(c);
                    self.pick_tool(Tool::Pencil);
                }
            }
            Tool::Magnifier => self.zoom_step(true, Some((x, y))),
            Tool::Text => {
                let inside = self.text.as_ref().is_some_and(|t| {
                    ix >= t.x
                        && iy >= t.y
                        && ix < t.x + (t.size as i32 * 10)
                        && iy < t.y + t.size as i32 * 2
                });
                if !inside {
                    self.commit_text();
                    if ix >= 0
                        && iy >= 0
                        && (ix as usize) < self.doc.w
                        && (iy as usize) < self.doc.h
                    {
                        self.text = Some(TextBox {
                            x: ix,
                            y: iy,
                            text: String::new(),
                            size: self.text_size,
                        });
                    }
                }
            }
            Tool::Shape => {
                if self.shape == Shape::Polygon {
                    self.polygon_click(cx, cy);
                    return;
                }
                let e = Edit::begin(
                    &self.doc,
                    Tip::Brush(Brush::Brush),
                    self.stroke,
                    self.primary,
                    self.alpha(),
                    None,
                );
                self.edit = Some(e);
                self.drag = Drag::Shape { x0: cx, y0: cy };
            }
            Tool::Solid => self.drag = Drag::Solid { x0: cx, y0: cy },
        }
    }
    fn polygon_click(&mut self, cx: f32, cy: f32) {
        if self.polygon.len() >= 3 {
            let (fx, fy) = self.polygon[0];
            let near = (fx - cx).abs() * self.zoom as f32 / 1000.0 < 8.0
                && (fy - cy).abs() * self.zoom as f32 / 1000.0 < 8.0;
            if near {
                self.finish_polygon();
                return;
            }
        }
        if self.polygon.is_empty() {
            self.edit = Some(Edit::begin(
                &self.doc,
                Tip::Brush(Brush::Brush),
                self.stroke,
                self.primary,
                self.alpha(),
                None,
            ));
        }
        self.polygon.push((cx, cy));
        self.preview_polygon(false);
    }
    fn preview_polygon(&mut self, closed: bool) {
        let outline = self.outline_solid.then_some((self.primary, self.stroke));
        let fill = (closed && self.fill_solid).then_some(self.secondary);
        if let Some(e) = self.edit.as_mut() {
            let poly = self.polygon.clone();
            e.shape(
                &mut self.doc,
                &poly,
                closed,
                outline.or(Some((self.primary, self.stroke))),
                fill,
            );
        }
        self.recompose();
    }
    pub fn finish_polygon(&mut self) {
        if self.polygon.len() >= 3 {
            self.preview_polygon(true);
        }
        if let Some(e) = self.edit.take()
            && let Some(b) = e.dirty
        {
            self.history.patch(&self.doc, &e.base, b);
            self.dirty = true;
        }
        self.polygon.clear();
        self.recompose();
    }
    fn canvas_motion(&mut self, x: i32, y: i32) -> bool {
        let (cx, cy) = self.to_canvas(x, y);
        let (ix, iy) = (floor_f(cx) as i32, floor_f(cy) as i32);
        match self.drag {
            Drag::Stroke => {
                let pencil = self.tool == Tool::Pencil;
                if let Some(e) = self.edit.as_mut() {
                    let (sx, sy) = if pencil {
                        (ix as f32 + 0.5, iy as f32 + 0.5)
                    } else {
                        (cx, cy)
                    };
                    e.to(&mut self.doc, sx, sy);
                }
                self.recompose();
                true
            }
            Drag::Shape { x0, y0 } => {
                let (mut x1, mut y1) = (cx, cy);
                if self.shape == Shape::Curve || self.shape == Shape::Line {
                    // Lines keep their end; nothing to constrain.
                } else if x1 == x0 || y1 == y0 {
                    x1 += 1.0;
                    y1 += 1.0;
                }
                let poly = raster::outline(self.shape, x0, y0, x1, y1);
                let closed = !self.shape.open();
                let outline = self.outline_solid.then_some((self.primary, self.stroke));
                let fill = (closed && self.fill_solid).then_some(self.secondary);
                // A shape without outline or fill still previews its outline.
                let outline = if outline.is_none() && fill.is_none() {
                    Some((self.primary, self.stroke))
                } else {
                    outline
                };
                if let Some(e) = self.edit.as_mut() {
                    e.shape(&mut self.doc, &poly, closed, outline, fill);
                }
                self.recompose();
                true
            }
            Drag::Bend { a, b } => {
                let poly = raster::bezier(a, (cx, cy), b);
                if let Some(e) = self.edit.as_mut() {
                    e.shape(
                        &mut self.doc,
                        &poly,
                        false,
                        Some((self.primary, self.stroke)),
                        None,
                    );
                }
                self.recompose();
                true
            }
            Drag::Select { x0, y0 } => {
                let (x1, y1) = (
                    ix.clamp(0, self.doc.w as i32),
                    iy.clamp(0, self.doc.h as i32),
                );
                let b = Bounds::new(x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1));
                self.selection = (b.width() > 0 && b.height() > 0).then_some(Selection {
                    bounds: b,
                    mask: None,
                    floating: None,
                });
                true
            }
            Drag::Lasso => {
                self.lasso.push((
                    cx.clamp(0.0, self.doc.w as f32),
                    cy.clamp(0.0, self.doc.h as f32),
                ));
                true
            }
            Drag::Move { dx, dy } => {
                if let Some(f) = self.selection.as_mut().and_then(|s| s.floating.as_mut()) {
                    f.x = ix - dx;
                    f.y = iy - dy;
                }
                true
            }
            Drag::Resize {
                left,
                top,
                right,
                bottom,
            } => {
                let mut b = self.resize_preview.unwrap_or(Bounds::new(
                    0,
                    0,
                    self.doc.w as i32,
                    self.doc.h as i32,
                ));
                let (rx, ry) = (round_f(cx) as i32, round_f(cy) as i32);
                if left {
                    b.x0 = rx.min(b.x1 - 1).max(b.x1 - 4096);
                }
                if right {
                    b.x1 = rx.max(b.x0 + 1).min(b.x0 + 4096);
                }
                if top {
                    b.y0 = ry.min(b.y1 - 1).max(b.y1 - 4096);
                }
                if bottom {
                    b.y1 = ry.max(b.y0 + 1).min(b.y0 + 4096);
                }
                self.resize_preview = Some(b);
                true
            }
            Drag::Solid { .. } => true,
            _ => false,
        }
    }
    fn canvas_up(&mut self) {
        match self.drag {
            Drag::Stroke => {
                if let Some(e) = self.edit.take()
                    && let Some(b) = e.dirty
                {
                    self.history.patch(&self.doc, &e.base, b);
                    self.dirty = true;
                }
            }
            Drag::Shape { x0, y0 } => {
                if self.shape == Shape::Curve {
                    // A curve bends after its line: the next drag sets the bend.
                    if let Some((cx, cy)) = self.cursor.map(|(x, y)| self.to_canvas(x, y)) {
                        self.drag = Drag::Bend {
                            a: (x0, y0),
                            b: (cx, cy),
                        };
                        return;
                    }
                }
                if let Some(e) = self.edit.take()
                    && let Some(b) = e.dirty
                {
                    self.history.patch(&self.doc, &e.base, b);
                    self.dirty = true;
                }
            }
            Drag::Bend { .. } => {
                if let Some(e) = self.edit.take()
                    && let Some(b) = e.dirty
                {
                    self.history.patch(&self.doc, &e.base, b);
                    self.dirty = true;
                }
            }
            Drag::Lasso => {
                let poly = core::mem::take(&mut self.lasso);
                let mut cov = Vec::new();
                if poly.len() >= 3
                    && let Some(b) = raster::fill_polygon(&poly, self.doc.w, self.doc.h, &mut cov)
                {
                    self.selection = Some(Selection {
                        bounds: b,
                        mask: Some(cov),
                        floating: None,
                    });
                }
            }
            Drag::Resize { .. } => {
                if let Some(b) = self.resize_preview.take() {
                    self.resize_canvas_to(b);
                }
            }
            Drag::Solid { x0, y0 } => {
                if let Some((x, y)) = self.cursor {
                    let (cx, cy) = self.to_canvas(x, y);
                    let (l, t) = (x0.min(cx) as i32, y0.min(cy) as i32);
                    let mut side = (cx - x0).abs().max((cy - y0).abs()) as usize;
                    if side < 8 {
                        side = 160;
                    }
                    let side = side.min(1024);
                    let tile = raster::render_solid(self.solid, side, side, self.primary);
                    let base = self.doc.layer().to_vec();
                    if let Some(b) = raster::stamp_tile(&mut self.doc, l, t, side, side, &tile) {
                        self.history.patch(&self.doc, &base, b);
                        self.mark_dirty();
                    }
                }
            }
            _ => {}
        }
        if !matches!(self.drag, Drag::Bend { .. }) || self.edit.is_none() {
            self.drag = Drag::None;
        }
        self.recompose();
    }
    /// The eight canvas resize handles (corners and edge midpoints) as
    /// logical centres with the edges each moves (left, top, right, bottom).
    pub fn handles(&self) -> [Handle; 8] {
        let (l, t) = self.to_logical(0.0, 0.0);
        let (r, b) = self.to_logical(self.doc.w as f32, self.doc.h as f32);
        let (mx, my) = ((l + r) / 2, (t + b) / 2);
        [
            ((l, t), (true, true, false, false)),
            ((mx, t), (false, true, false, false)),
            ((r, t), (false, true, true, false)),
            ((l, my), (true, false, false, false)),
            ((r, my), (false, false, true, false)),
            ((l, b), (true, false, false, true)),
            ((mx, b), (false, false, false, true)),
            ((r, b), (false, false, true, true)),
        ]
    }
    fn handle_at(&self, x: i32, y: i32) -> Option<(bool, bool, bool, bool)> {
        self.handles()
            .into_iter()
            .find(|((hx, hy), _)| (x - hx).abs() <= 5 && (y - hy).abs() <= 5)
            .map(|(_, e)| e)
    }
    /// Resize the canvas to `b` (canvas coordinates): growing adds white
    /// on the background (transparency on layers), shrinking crops; content
    /// keeps its place.
    pub fn resize_canvas_to(&mut self, b: Bounds) {
        let (w, h) = (b.width().clamp(1, 4096), b.height().clamp(1, 4096));
        if w * h > door_workspace::explorer_preview::MAX_PIXELS * 2
            || b == Bounds::new(0, 0, self.doc.w as i32, self.doc.h as i32)
        {
            return;
        }
        let before = raster::Snapshot::of(&self.doc);
        let (ow, oh) = (self.doc.w, self.doc.h);
        // The old canvas's overlap with the new bounds, in old coordinates.
        let keep = Bounds::new(
            b.x0.max(0),
            b.y0.max(0),
            b.x1.min(ow as i32),
            b.y1.min(oh as i32),
        );
        for (i, l) in self.doc.layers.iter_mut().enumerate() {
            let mut px = if i == 0 {
                raster::white(w, h)
            } else {
                vec![0; w * h * 4]
            };
            if keep.x0 < keep.x1 && keep.y0 < keep.y1 {
                let part = raster::crop(&l.px, ow, keep);
                let at = Bounds::new(
                    keep.x0 - b.x0,
                    keep.y0 - b.y0,
                    keep.x1 - b.x0,
                    keep.y1 - b.y0,
                );
                raster::paste(&mut px, w, at, &part);
            }
            l.px = px;
        }
        self.doc.w = w;
        self.doc.h = h;
        self.history.push(raster::Change::Whole {
            before,
            after: raster::Snapshot::of(&self.doc),
        });
        self.clamp_scroll();
        self.mark_dirty();
    }

    // ---- ribbon actions ----
    fn activate(&mut self, id: u32, x: i32, y: i32, s: &mut Storage) -> Result<bool, Error> {
        use id::*;
        let toggle =
            |p: &mut Self, m: Menu| p.menu = if p.menu == Some(m) { None } else { Some(m) };
        let from_group = matches!(self.menu, Some(Menu::Group(_)));
        let opened = self.menu;
        if from_group && !(GROUP..=GROUP + 3).contains(&id) && id != BACKDROP && id != u32::MAX - 1
        {
            // A command picked from a collapsed group's dropdown closes it
            // (unless it opens its own menu).
            self.menu = None;
        }
        let _ = opened;
        match id {
            BACKDROP => self.menu = None,
            MENU..=3 => toggle(
                self,
                [Menu::File, Menu::Edit, Menu::View][(id - MENU) as usize],
            ),
            SAVE => self.save(s)?,
            SHARE => {
                self.notice = "Sharing isn't available on this device yet. Save your image to Pictures and share it from File Explorer.".into();
                self.menu = Some(Menu::Notice);
            }
            UNDO => self.undo(),
            REDO => self.redo(),
            FEEDBACK => {
                self.notice = "Feedback Hub isn't available on this device.".into();
                self.menu = Some(Menu::Notice);
            }
            SETTINGS => self.dialog = Some(Dialog::Settings),
            SELECT => self.pick_tool(if self.tool == Tool::FreeSelect {
                Tool::FreeSelect
            } else {
                Tool::Select
            }),
            SELECT_MENU => toggle(self, Menu::Selection),
            CROP => self.crop(),
            ROTATE => self.turn(Turn::Right),
            ROTATE_MENU => toggle(self, Menu::Rotate),
            REMOVE_BG => self.remove_background(),
            FLIP => self.turn(Turn::FlipHorizontal),
            FLIP_MENU => toggle(self, Menu::Flip),
            RESIZE => {
                self.dialog = Some(Dialog::Resize {
                    pixels: false,
                    h: "100".into(),
                    v: "100".into(),
                    lock: true,
                    field: 0,
                })
            }
            TOOL..=35 => {
                let t = [
                    Tool::Pencil,
                    Tool::Fill,
                    Tool::Text,
                    Tool::Eraser,
                    Tool::Picker,
                    Tool::Magnifier,
                ][(id - TOOL) as usize];
                self.pick_tool(t);
            }
            BRUSH => self.pick_tool(Tool::Brush),
            BRUSH_MENU => toggle(self, Menu::Brushes),
            SHAPE..=72 => {
                self.shape = Shape::ALL[(id - SHAPE) as usize];
                self.pick_tool(Tool::Shape);
            }
            SHAPES_UP => {
                self.shapes_row = self.shapes_row.saturating_sub(1);
                if from_group {
                    self.menu = Some(Menu::Group(view::SHAPES));
                }
            }
            SHAPES_DOWN => {
                let cols = self.live_shape_cols();
                let rows = Shape::ALL.len().div_ceil(cols);
                if self.shapes_row + 3 < rows {
                    self.shapes_row += 1;
                }
                if from_group {
                    self.menu = Some(Menu::Group(view::SHAPES));
                }
            }
            OUTLINE => toggle(self, Menu::Outline),
            FILL => toggle(self, Menu::Fill),
            STROKE => toggle(self, Menu::Stroke),
            PRIMARY => self.editing_secondary = false,
            SECONDARY => self.editing_secondary = true,
            PALETTE..=101 => self.set_active_color(super::paint::PALETTE[(id - PALETTE) as usize]),
            CUSTOM..=111 => {
                if let Some(c) = self.custom[(id - CUSTOM) as usize] {
                    self.set_active_color(c);
                } else {
                    self.hsv = opaque_hsv(self.active_color());
                    self.menu = Some(Menu::Colors);
                }
            }
            EDIT_COLORS => {
                self.hsv = opaque_hsv(self.active_color());
                toggle(self, Menu::Colors);
            }
            SOLIDS => toggle(self, Menu::Solids),
            g if (GROUP..=GROUP + 3).contains(&g) => toggle(self, Menu::Group(1 << (g - GROUP))),
            LAYERS => {
                self.layers_open = !self.layers_open;
                self.clamp_scroll();
            }
            FIT => self.fit(),
            ZOOM_MENU => toggle(self, Menu::Zoom),
            ZOOM_OUT => self.zoom_step(false, None),
            ZOOM_IN => self.zoom_step(true, None),
            ZOOM_SLIDER => {
                if let Some((sx, _, sw, _)) = self.zoom_slider() {
                    let t = ((x - sx) as f32 / sw as f32).clamp(0.0, 1.0);
                    let i = round_f(t * (ZOOMS.len() - 1) as f32) as usize;
                    self.set_zoom(ZOOMS[i], None);
                }
            }
            ITEM..=299 => {
                let index = (id - ITEM) as usize;
                if let Some(m) = self.menu {
                    match m {
                        Menu::Colors => self.colors_action(index),
                        _ => self.menu_action(m, index, s)?,
                    }
                }
            }
            LAYER_ADD => self.add_layer(),
            LAYER..=315 => {
                let i = (id - LAYER) as usize;
                if i < self.doc.layers.len() {
                    self.commit();
                    self.doc.active = i;
                }
            }
            LAYER_EYE..=325 => {
                let i = (id - LAYER_EYE) as usize;
                if i < self.doc.layers.len() {
                    self.whole(|d| d.layers[i].visible = !d.layers[i].visible);
                }
            }
            LAYER_MORE..=335 => {
                let i = (id - LAYER_MORE) as usize;
                self.menu = Some(Menu::LayerMore(i));
            }
            DIALOG..=498 => self.dialog_action((id - DIALOG) as usize, x, y, s)?,
            _ => return Ok(false),
        }
        let _ = y;
        Ok(true)
    }
    /// Paint 11's Remove background: the background colour connected to
    /// the edges (sampled at the corners) becomes transparent on a new
    /// layer holding the subject.
    fn remove_background(&mut self) {
        self.commit();
        if self.doc.layers.len() >= raster::MAX_LAYERS {
            return;
        }
        let (w, h) = (self.doc.w, self.doc.h);
        self.whole(|d| {
            let mut px = d.layers[d.active].px.clone();
            let mut layer = raster::Doc {
                w,
                h,
                layers: vec![raster::Layer {
                    px: core::mem::take(&mut px),
                    visible: true,
                }],
                active: 0,
            };
            for (x, y) in [
                (0, 0),
                (w as i32 - 1, 0),
                (0, h as i32 - 1),
                (w as i32 - 1, h as i32 - 1),
            ] {
                if layer.layers[0].px[(y as usize * w + x as usize) * 4 + 3] != 0 {
                    let _ = raster::flood(
                        &mut layer,
                        x,
                        y,
                        Rgba {
                            r: 0,
                            g: 0,
                            b: 0,
                            a: 0,
                        },
                        12,
                        255,
                    );
                }
            }
            let subject = core::mem::take(&mut layer.layers[0].px);
            d.layers[d.active].visible = false;
            d.layers.insert(
                d.active + 1,
                raster::Layer {
                    px: subject,
                    visible: true,
                },
            );
            d.active += 1;
        });
        self.layers_open = true;
    }
    fn colors_action(&mut self, index: usize) {
        match index {
            // OK: the colour becomes active and fills the next custom slot.
            0 => {
                let c = hsv_rgb(self.hsv);
                self.set_active_color(c);
                if !self.custom.contains(&Some(c)) {
                    let slot = self.custom.iter().position(Option::is_none).unwrap_or(9);
                    if slot == 9 && self.custom[9].is_some() {
                        self.custom.rotate_left(1);
                    }
                    self.custom[slot] = Some(c);
                }
                self.menu = None;
            }
            _ => self.menu = None,
        }
    }
    fn dialog_action(
        &mut self,
        index: usize,
        _x: i32,
        _y: i32,
        s: &mut Storage,
    ) -> Result<(), Error> {
        let Some(dialog) = self.dialog.clone() else {
            return Ok(());
        };
        match dialog {
            Dialog::Resize {
                pixels,
                h,
                v,
                lock,
                field,
            } => match index {
                0 | 1 => {
                    let px = index == 1;
                    if px != pixels {
                        let (h, v) = if px {
                            (
                                format!("{}", self.target_w()),
                                format!("{}", self.target_h()),
                            )
                        } else {
                            ("100".into(), "100".into())
                        };
                        self.dialog = Some(Dialog::Resize {
                            pixels: px,
                            h,
                            v,
                            lock,
                            field,
                        });
                    }
                }
                2 | 3 => {
                    self.dialog = Some(Dialog::Resize {
                        pixels,
                        h,
                        v,
                        lock,
                        field: index as u8 - 2,
                    })
                }
                4 => {
                    self.dialog = Some(Dialog::Resize {
                        pixels,
                        h,
                        v,
                        lock: !lock,
                        field,
                    })
                }
                5 => {
                    let parse = |t: &str| t.parse::<usize>().unwrap_or(0);
                    let (w0, h0) = (self.target_w(), self.target_h());
                    let (nw, nh) = if pixels {
                        (parse(&h), parse(&v))
                    } else {
                        (w0 * parse(&h) / 100, h0 * parse(&v) / 100)
                    };
                    self.dialog = None;
                    if nw > 0 && nh > 0 {
                        self.resample_to(nw, nh);
                    }
                }
                _ => self.dialog = None,
            },
            Dialog::Properties { w, h, .. } => match index {
                0 | 1 => {
                    self.dialog = Some(Dialog::Properties {
                        w,
                        h,
                        field: index as u8,
                    })
                }
                2 => {
                    self.dialog = None;
                    if let (Ok(w), Ok(h)) = (w.parse::<usize>(), h.parse::<usize>())
                        && w > 0
                        && h > 0
                    {
                        self.commit();
                        self.resize_canvas(w, h);
                    }
                }
                _ => self.dialog = None,
            },
            Dialog::Open {
                files,
                selected,
                first,
            } => match index {
                0 => {
                    self.dialog = None;
                    if let Some(p) = files.get(selected).cloned() {
                        self.open_path(s, &p)?;
                    }
                }
                1 => self.dialog = None,
                i if i >= 10 => {
                    let pick = first + i - 10;
                    if pick < files.len() {
                        if pick == selected {
                            self.dialog = None;
                            let p = files[pick].clone();
                            self.open_path(s, &p)?;
                        } else {
                            self.dialog = Some(Dialog::Open {
                                files,
                                selected: pick,
                                first,
                            });
                        }
                    }
                }
                _ => {}
            },
            Dialog::SaveAs { name } => match index {
                0 => {
                    let mut name = String::from(name.trim());
                    if name.is_empty() {
                        return Ok(());
                    }
                    if !name.to_ascii_lowercase().ends_with(".png") {
                        name.push_str(".png");
                    }
                    self.dialog = None;
                    let path = format!("{}/Pictures/{name}", self.home());
                    if self.path.as_deref() != Some(path.as_str()) {
                        self.identity = None;
                    }
                    if let Err(e) = self.save_to(s, &path) {
                        self.notice = format!("Paint couldn't save {name} ({e:?}).");
                        self.menu = Some(Menu::Notice);
                    } else if self.discard_after_save {
                        self.discard_after_save = false;
                        self.after_unsaved(self.pending_after, s);
                    }
                }
                _ => {
                    self.dialog = None;
                    self.discard_after_save = false;
                }
            },
            Dialog::Unsaved { then } => match index {
                // Save
                0 => {
                    self.dialog = None;
                    let had_path = self
                        .path
                        .as_deref()
                        .is_some_and(|p| p.to_ascii_lowercase().ends_with(".png"));
                    self.save(s)?;
                    if had_path {
                        self.after_unsaved(then, s);
                    } else {
                        self.discard_after_save = true;
                        self.pending_after = then;
                    }
                }
                // Don't save
                1 => {
                    self.dialog = None;
                    self.dirty = false;
                    self.after_unsaved(then, s);
                }
                _ => {
                    self.dialog = None;
                    if then == After::Close {
                        self.close_declined = true;
                    }
                }
            },
            Dialog::Settings => match index {
                0..=2 => self.theme = [0, 1, 2][index],
                _ => self.dialog = None,
            },
        }
        Ok(())
    }
    fn target_w(&self) -> usize {
        self.selection
            .as_ref()
            .and_then(|s| s.floating.as_ref())
            .map_or(self.doc.w, |f| f.w)
    }
    fn target_h(&self) -> usize {
        self.selection
            .as_ref()
            .and_then(|s| s.floating.as_ref())
            .map_or(self.doc.h, |f| f.h)
    }
    /// Typing into the focused dialog field.
    fn dialog_key(
        &mut self,
        scan: u32,
        text: u8,
        control: bool,
        s: &mut Storage,
    ) -> Result<bool, Error> {
        let Some(d) = self.dialog.as_mut() else {
            return Ok(false);
        };
        let enter = scan == 28 || scan == 0x11c;
        if scan == 1 {
            let then = if let Dialog::Unsaved { then } = d {
                Some(*then)
            } else {
                None
            };
            self.dialog = None;
            if then == Some(After::Close) {
                self.close_declined = true;
            }
            return Ok(true);
        }
        let edit = |t: &mut String, digits: bool| {
            if scan == 14 {
                t.pop();
            } else if (32..127).contains(&text)
                && !control
                && (!digits || text.is_ascii_digit())
                && t.len() < 64
            {
                t.push(text as char);
            }
        };
        let mut lock_sync: Option<(bool, bool)> = None;
        match d {
            Dialog::Resize {
                h,
                v,
                field,
                lock,
                pixels,
            } => {
                if enter {
                    return self.dialog_action(5, 0, 0, s).map(|_| true);
                }
                if scan == 15 {
                    *field ^= 1;
                } else if *field == 0 {
                    edit(h, true);
                    lock_sync = Some((*lock, *pixels));
                } else {
                    edit(v, true);
                }
            }
            Dialog::Properties { w, h, field } => {
                if enter {
                    return self.dialog_action(2, 0, 0, s).map(|_| true);
                }
                if scan == 15 {
                    *field ^= 1;
                } else if *field == 0 {
                    edit(w, true);
                } else {
                    edit(h, true);
                }
            }
            Dialog::SaveAs { name } => {
                if enter {
                    return self.dialog_action(0, 0, 0, s).map(|_| true);
                }
                if !matches!(
                    text,
                    b'/' | b'\\' | b':' | b'*' | b'?' | b'"' | b'<' | b'>' | b'|'
                ) {
                    edit(name, false);
                }
            }
            Dialog::Open {
                files,
                selected,
                first,
            } => {
                match scan {
                    0x148 => *selected = selected.saturating_sub(1),
                    0x150 => *selected = (*selected + 1).min(files.len().saturating_sub(1)),
                    _ if enter => return self.dialog_action(0, 0, 0, s).map(|_| true),
                    _ => {}
                }
                if *selected < *first {
                    *first = *selected;
                } else if *selected >= *first + 8 {
                    *first = *selected - 7;
                }
            }
            Dialog::Unsaved { .. } => match (scan, text) {
                (_, b's' | b'S') => return self.dialog_action(0, 0, 0, s).map(|_| true),
                _ if enter => return self.dialog_action(0, 0, 0, s).map(|_| true),
                (_, b'n' | b'N') => return self.dialog_action(1, 0, 0, s).map(|_| true),
                _ => {}
            },
            Dialog::Settings => {}
        }
        // Percentages keep the aspect: vertical follows horizontal.
        if let Some((true, pixels)) = lock_sync
            && let Some(Dialog::Resize { h, v, .. }) = self.dialog.as_mut()
        {
            *v = if pixels {
                let w0 = self
                    .selection
                    .as_ref()
                    .and_then(|s| s.floating.as_ref())
                    .map_or(self.doc.w, |f| f.w)
                    .max(1);
                let h0 = self
                    .selection
                    .as_ref()
                    .and_then(|s| s.floating.as_ref())
                    .map_or(self.doc.h, |f| f.h);
                format!("{}", h.parse::<usize>().unwrap_or(0) * h0 / w0)
            } else {
                h.clone()
            };
        }
        Ok(true)
    }
}

pub const ZOOM_LABELS: [&str; 12] = [
    "12.5%", "25%", "50%", "75%", "100%", "200%", "300%", "400%", "500%", "600%", "700%", "800%",
];

impl Application for Paint {
    fn title(&self) -> &str {
        &self.title
    }
    fn options(&self) -> door_workspace::runtime::WindowOptions {
        door_workspace::runtime::WindowOptions {
            width: 1200,
            height: 660,
            caption_height: TITLE as u32,
            caption_left: 260,
            centered: true,
        }
    }
    fn caption(&self) -> (u32, u32) {
        (260, 1)
    }
    fn display(&mut self, _w: u32, _h: u32, percent: u32) {
        self.percent = percent.clamp(100, 300);
        self.refresh_solid_icons();
    }
    fn configure(&mut self, width: u32, height: u32) {
        self.width = width.max(500);
        self.height = height.max(300);
        // A new image's first canvas fills the view at 100%, as Paint sizes
        // it to the window.
        if !self.ready && !self.dirty && self.path.is_none() && !self.history.can_undo() {
            let (w, h) = self.fitted_canvas();
            self.doc = Doc::new(w, h);
            self.recompose();
        }
        self.refresh_solid_icons();
        self.clamp_scroll();
        if !self.ready {
            self.ready = true;
            door_rt::diagnostic(format_args!("DOOR_INBOX READY app=paint\n"));
        }
    }
    fn draw(&self, v: &mut View<'_>) {
        self.paint(v);
    }
    fn target(&self, x: i32, y: i32) -> Option<u32> {
        self.hits()
            .into_iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, t)| t)
    }
    fn click_at(
        &mut self,
        id: u32,
        x: i32,
        y: i32,
        s: &mut Storage,
        _f: &FontAtlas,
    ) -> Result<bool, Error> {
        let result = self.activate(id, x, y, s);
        if let Err(e) = &result {
            self.notice = format!("That didn't work ({e:?}).");
            self.menu = Some(Menu::Notice);
            return Ok(true);
        }
        result
    }
    fn click(&mut self, id: u32, s: &mut Storage) -> Result<bool, Error> {
        self.activate(id, 0, 0, s)
    }
    fn pointer_down(&mut self, x: i32, y: i32, _f: &FontAtlas) -> bool {
        self.cursor = Some((x, y));
        if self.dialog.is_some() {
            return false;
        }
        if self.menu == Some(Menu::Colors) {
            if let Some(r) = self.spectrum_rect()
                && r.contains(x, y)
            {
                self.drag = Drag::Spectrum;
                self.spectrum_at(x, y);
                return true;
            }
            if let Some(r) = self.hue_rect()
                && r.contains(x, y)
            {
                self.drag = Drag::Hue;
                self.hue_at(x);
                return true;
            }
        }
        if self.menu.is_some() {
            return false;
        }
        if let Some(hit) = self.scrollbar_hit(x, y) {
            self.drag = hit;
            return true;
        }
        for size in [true, false] {
            if let Some((tx, top, bottom)) = self.track(size)
                && (x - tx).abs() <= 12
                && y >= top - 8
                && y <= bottom + 8
            {
                self.drag = Drag::Slider { size };
                self.slider_at(size, y);
                return true;
            }
        }
        if let Drag::Bend { a, b } = self.drag {
            // The curve's second drag sets the bend.
            let (cx, cy) = self.to_canvas(x, y);
            let poly = raster::bezier(a, (cx, cy), b);
            if let Some(e) = self.edit.as_mut() {
                e.shape(
                    &mut self.doc,
                    &poly,
                    false,
                    Some((self.primary, self.stroke)),
                    None,
                );
            }
            self.recompose();
            return true;
        }
        if self.over_canvas(x, y) {
            self.canvas_down(x, y);
            return true;
        }
        false
    }
    fn pointer_motion(&mut self, x: i32, y: i32, _f: &FontAtlas) -> bool {
        let before = self.cursor;
        self.cursor = Some((x, y));
        match self.drag {
            Drag::ScrollX(grab) => {
                self.scroll_to(true, x - grab);
                true
            }
            Drag::ScrollY(grab) => {
                self.scroll_to(false, y - grab);
                true
            }
            Drag::Slider { size } => {
                self.slider_at(size, y);
                true
            }
            Drag::Spectrum => {
                self.spectrum_at(x, y);
                true
            }
            Drag::Hue => {
                self.hue_at(x);
                true
            }
            Drag::None => {
                // The status bar position and the brush outline follow the
                // pointer over the canvas.
                self.over_canvas(x, y) || before.is_some_and(|(bx, by)| self.over_canvas(bx, by))
            }
            _ => self.canvas_motion(x, y),
        }
    }
    fn pointer_up(&mut self) {
        match self.drag {
            Drag::Slider { .. }
            | Drag::Spectrum
            | Drag::Hue
            | Drag::ScrollX(_)
            | Drag::ScrollY(_) => self.drag = Drag::None,
            Drag::None => {}
            _ => self.canvas_up(),
        }
    }
    fn wheel(&mut self, x: i32, y: i32, notches: i32, _f: &FontAtlas) -> bool {
        if !self.over_canvas(x, y) || self.menu.is_some() || self.dialog.is_some() {
            return false;
        }
        let step = self.phys(48) * notches.signum() as i64 * notches.unsigned_abs().min(4) as i64;
        if self.modifiers_control {
            self.zoom_step(notches > 0, Some((x, y)));
        } else if self.modifiers_shift {
            self.scroll.0 -= step;
        } else {
            self.scroll.1 -= step;
        }
        self.clamp_scroll();
        true
    }
    fn key(&mut self, key: u32, s: &mut Storage) -> Result<bool, Error> {
        let scan = key & 511;
        let m = key >> 9;
        let control = m & door_abi::graphics::CONTROL != 0;
        let shift = m & door_abi::graphics::SHIFT != 0;
        let text = (key >> 16) as u8;
        self.modifiers_control = control;
        self.modifiers_shift = shift;
        if self.dialog.is_some() {
            return self.dialog_key(scan, text, control, s);
        }
        if scan == 1 && self.menu.is_some() {
            self.menu = None;
            return Ok(true);
        }
        // Typing into a text box.
        if let Some(t) = self.text.as_mut()
            && !control
        {
            match scan {
                1 => self.commit_text(),
                14 => {
                    t.text.pop();
                }
                28 | 0x11c => t.text.push('\n'),
                _ if (32..127).contains(&text) => t.text.push(text as char),
                _ => return Ok(false),
            }
            return Ok(true);
        }
        match (scan, control, shift) {
            (44, true, false) => self.undo(),
            (21, true, _) | (44, true, true) => self.redo(),
            (49, true, _) => self.menu_action(Menu::File, 0, s)?,
            (24, true, _) => self.menu_action(Menu::File, 1, s)?,
            (31, true, false) => self.save(s)?,
            (31, true, true) => self.menu_action(Menu::File, 3, s)?,
            (18, true, _) => self.menu_action(Menu::File, 4, s)?,
            (30, true, _) => self.select_all(),
            (45, true, _) => {
                self.copy();
                self.delete_selection();
            }
            (46, true, _) => self.copy(),
            (47, true, _) => self.sys_paste = true,
            (13, true, _) | (78, true, _) => self.zoom_step(true, None),
            (12, true, _) | (74, true, _) => self.zoom_step(false, None),
            (11, true, true) => self.fit(),
            (11, true, false) => self.set_zoom(1000, None),
            (19, true, _) => self.rulers = !self.rulers,
            (34, true, _) => self.gridlines = !self.gridlines,
            (17, true, _) => {
                self.dialog = Some(Dialog::Resize {
                    pixels: false,
                    h: "100".into(),
                    v: "100".into(),
                    lock: true,
                    field: 0,
                })
            }
            (0x153, _, _) => self.delete_selection(),
            (1, _, _) => {
                if !self.polygon.is_empty() {
                    self.finish_polygon();
                } else if let Drag::Bend { .. } = self.drag {
                    self.canvas_up_force();
                } else if self.selection.is_some() {
                    self.commit();
                } else {
                    return Ok(false);
                }
            }
            (28 | 0x11c, _, _) => {
                if !self.polygon.is_empty() {
                    self.finish_polygon();
                } else {
                    self.commit();
                }
            }
            // Arrow keys nudge a moved selection.
            (0x148 | 0x150 | 0x14b | 0x14d, false, _) if self.selection.is_some() => {
                self.lift();
                if let Some(f) = self.selection.as_mut().and_then(|s| s.floating.as_mut()) {
                    match scan {
                        0x148 => f.y -= 1,
                        0x150 => f.y += 1,
                        0x14b => f.x -= 1,
                        _ => f.x += 1,
                    }
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn tick(&mut self, _s: &mut Storage, _now: u64) -> Result<bool, Error> {
        Ok(false)
    }
    fn capabilities(&self) -> u32 {
        door_service_protocol::clipboard::CAP_CLIPBOARD
    }
    fn update(
        &mut self,
        _s: &mut Storage,
        c: &mut door_service_client::window::WindowClient,
        window: u64,
    ) -> Result<bool, Error> {
        let mut changed = false;
        if core::mem::take(&mut self.sys_copy)
            && let Some((px, w, h)) = self.clipboard.clone()
        {
            let mut bytes = Vec::with_capacity(8 + px.len());
            bytes.extend_from_slice(&(w as u32).to_le_bytes());
            bytes.extend_from_slice(&(h as u32).to_le_bytes());
            bytes.extend_from_slice(&px);
            if let Ok(section) = door_workspace::section_from(&bytes) {
                let _ = c.clipboard_set_with(
                    window,
                    &[],
                    &[(
                        door_service_protocol::clipboard::IMAGE,
                        section,
                        bytes.len() as u32,
                    )],
                );
                let _ = door_sys::close(section);
            }
        }
        if core::mem::take(&mut self.sys_paste) {
            // A picture on the clipboard (a snip, another app's copy) wins
            // over Paint's own last copy.
            if let Some((w, h, px)) = crate::snipping::read_clipboard_image(c) {
                self.clipboard = Some((px, w, h));
            }
            self.paste();
            changed = true;
        }
        Ok(changed)
    }
    fn closing(&mut self, _s: &mut Storage) -> Result<bool, Error> {
        if !self.dirty || self.discard {
            return Ok(true);
        }
        self.menu = None;
        self.dialog = Some(Dialog::Unsaved { then: After::Close });
        Ok(false)
    }
    fn wants_close(&mut self) -> bool {
        core::mem::take(&mut self.close_now)
    }
    fn take_close_declined(&mut self) -> bool {
        core::mem::take(&mut self.close_declined)
    }
    fn modal(&self) -> bool {
        self.dialog.is_some()
    }
    fn menu_open(&self) -> bool {
        self.menu.is_some()
    }
    fn dismiss_menu(&mut self) {
        self.menu = None;
    }
}
impl Paint {
    fn canvas_up_force(&mut self) {
        if let Some(e) = self.edit.take()
            && let Some(b) = e.dirty
        {
            self.history.patch(&self.doc, &e.base, b);
            self.dirty = true;
        }
        self.drag = Drag::None;
        self.recompose();
    }
}
