//! Paint's layout, hit testing and painting (see `super` for the measured
//! metrics).
use super::raster::{Brush, Rgba, Shape, Solid};
use super::*;
use door_graphics_text::TextStyle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}
impl Rect {
    pub const fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }
    pub fn contains(self, px: i32, py: i32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w as i32 && py < self.y + self.h as i32
    }
    /// A square of `side` centred on (cx, cy).
    pub const fn centred(cx: i32, cy: i32, side: u32) -> Self {
        Self::new(cx - side as i32 / 2, cy - side as i32 / 2, side, side)
    }
}

/// Theme colours (premultiplied BGRA).
pub struct Ink {
    pub dark: bool,
    pub mica: [u8; 4],
    pub backdrop: [u8; 4],
    pub card: [u8; 4],
    pub stroke: [u8; 4],
    pub divider: [u8; 4],
    pub panel: [u8; 4],
    pub text: [u8; 4],
    pub secondary: [u8; 4],
    pub disabled: [u8; 4],
    pub hover: [u8; 4],
    pub pressed: [u8; 4],
    pub accent: [u8; 4],
    pub on_accent: [u8; 4],
    pub flyout: [u8; 4],
    pub flyout_edge: [u8; 4],
    pub field: [u8; 4],
    pub field_edge: [u8; 4],
    pub smoke: [u8; 4],
    pub footer: [u8; 4],
    pub track: [u8; 4],
}
const fn rgb(r: u8, g: u8, b: u8) -> [u8; 4] {
    [b, g, r, 255]
}
pub fn ink(dark: bool) -> Ink {
    if dark {
        Ink {
            dark,
            mica: rgb(30, 32, 36),
            backdrop: rgb(27, 31, 42),
            card: rgb(38, 39, 42),
            stroke: rgb(28, 28, 28),
            divider: rgb(61, 62, 66),
            panel: rgb(46, 46, 46),
            text: rgb(255, 255, 255),
            secondary: rgb(207, 207, 207),
            disabled: rgb(110, 110, 110),
            hover: rgb(52, 53, 57),
            pressed: rgb(60, 61, 65),
            accent: rgb(76, 194, 255),
            on_accent: rgb(0, 0, 0),
            flyout: rgb(44, 44, 44),
            flyout_edge: rgb(25, 25, 25),
            field: rgb(45, 45, 45),
            field_edge: rgb(70, 70, 70),
            smoke: [0, 0, 0, 110],
            footer: rgb(32, 32, 32),
            track: rgb(160, 160, 160),
        }
    } else {
        Ink {
            dark,
            mica: rgb(243, 243, 243),
            backdrop: rgb(234, 237, 242),
            card: rgb(251, 251, 251),
            stroke: rgb(229, 229, 229),
            divider: rgb(224, 224, 224),
            panel: rgb(255, 255, 255),
            text: rgb(26, 26, 26),
            secondary: rgb(93, 93, 93),
            disabled: rgb(160, 160, 160),
            hover: rgb(240, 240, 240),
            pressed: rgb(232, 232, 232),
            accent: rgb(0, 95, 184),
            on_accent: rgb(255, 255, 255),
            flyout: rgb(249, 249, 249),
            flyout_edge: rgb(214, 214, 214),
            field: rgb(255, 255, 255),
            field_edge: rgb(208, 208, 208),
            smoke: [0, 0, 0, 77],
            footer: rgb(243, 243, 243),
            track: rgb(134, 134, 134),
        }
    }
}

/// Paint's own toolbar artwork (`assets/icons/paint`) at `size` logical px.
pub fn icon(v: &mut View<'_>, key: &str, dark: bool, x: i32, y: i32, size: u32) {
    let scale = v.scale.clamp(1, 3);
    let theme = if dark { "dark" } else { "light" };
    let px = size * scale;
    let bytes = door_workspace::assets::get(&format!(
        "../../../assets/icons/paint/{key}.{theme}.{px}.bgra"
    ));
    if bytes.len() == (px * px * 4) as usize {
        v.color_pixels(x, y, size as usize, bytes);
    }
}
/// A Segoe Fluent / Paint Fluent glyph (`paint-*` in the shell manifest).
pub fn glyph(v: &mut View<'_>, name: &str, x: i32, y: i32, size: u32, color: [u8; 4]) {
    let scale = v.scale.clamp(1, 3);
    let px = size * scale;
    let bytes = door_workspace::assets::get(&format!(
        "../../../assets/icons/windows-shell/{name}.{px}.mask"
    ));
    if bytes.len() == (px * px) as usize {
        v.mask_pixels(x, y, size as usize, bytes, color, None);
    }
}

/// Group positions across the ribbon card.
pub struct Ribbon {
    pub shape_cols: usize,
    /// Groups shown as one dropdown button (`IMAGE`, `TOOLS`, `SHAPES`,
    /// `COLORS`), as Paint collapses them when the window narrows.
    pub collapsed: u8,
    pub selection: i32,
    pub image: i32,
    pub tools: i32,
    pub brushes: i32,
    pub shapes: i32,
    pub colors: i32,
    pub solids: i32,
    pub layers: i32,
    pub end: i32,
}
pub const IMAGE: u8 = 1;
pub const TOOLS: u8 = 2;
pub const SHAPES: u8 = 4;
pub const COLORS: u8 = 8;
const GROUP_SELECTION: i32 = 75;
const GROUP_IMAGE: i32 = 160;
const GROUP_TOOLS: i32 = 137;
const GROUP_BRUSHES: i32 = 68;
const GROUP_COLORS: i32 = 352;
const GROUP_SOLIDS: i32 = 72;
const GROUP_LAYERS: i32 = 66;
const GROUP_COLLAPSED: i32 = 66;
const SHAPE_PITCH: i32 = 22;
fn shapes_width(cols: usize) -> i32 {
    cols as i32 * SHAPE_PITCH + 17 + 120
}
fn group_width(group: u8, collapsed: u8, cols: usize) -> i32 {
    if collapsed & group != 0 {
        return GROUP_COLLAPSED;
    }
    match group {
        IMAGE => GROUP_IMAGE,
        TOOLS => GROUP_TOOLS,
        SHAPES => shapes_width(cols),
        _ => GROUP_COLORS,
    }
}
const ROW1: i32 = CARD_Y + 25;
const ROW2: i32 = CARD_Y + 65;
const LABEL_Y: i32 = CARD_Y + 84;

impl Paint {
    pub fn ribbon(&self) -> Ribbon {
        let w = self.width as i32;
        let fixed = 8 + GROUP_SELECTION + GROUP_BRUSHES + GROUP_SOLIDS + GROUP_LAYERS + 16;
        // Paint collapses the Shapes gallery first, then Image, Tools and
        // finally the palette.
        let mut chosen = (SHAPES | IMAGE | TOOLS | COLORS, 7usize);
        for collapsed in [
            0,
            SHAPES,
            SHAPES | IMAGE,
            SHAPES | IMAGE | TOOLS,
            SHAPES | IMAGE | TOOLS | COLORS,
        ] {
            let others = fixed
                + group_width(IMAGE, collapsed, 0)
                + group_width(TOOLS, collapsed, 0)
                + group_width(COLORS, collapsed, 0);
            if collapsed & SHAPES == 0 {
                let cols = (w - others - shapes_width(0)) / SHAPE_PITCH;
                if cols >= 5 {
                    chosen = (collapsed, cols.min(7) as usize);
                    break;
                }
            } else if others + GROUP_COLLAPSED <= w {
                chosen = (collapsed, 7);
                break;
            }
        }
        let (collapsed, cols) = chosen;
        let mut x = 8;
        let mut next = |w: i32| {
            let at = x;
            x += w;
            at
        };
        Ribbon {
            shape_cols: cols,
            collapsed,
            selection: next(GROUP_SELECTION),
            image: next(group_width(IMAGE, collapsed, cols)),
            tools: next(group_width(TOOLS, collapsed, cols)),
            brushes: next(GROUP_BRUSHES),
            shapes: next(group_width(SHAPES, collapsed, cols)),
            colors: next(group_width(COLORS, collapsed, cols)),
            solids: next(GROUP_SOLIDS),
            layers: next(GROUP_LAYERS),
            end: x,
        }
    }
    /// Gallery columns shown now (a collapsed gallery's flyout has seven).
    pub fn live_shape_cols(&self) -> usize {
        let r = self.ribbon();
        if r.collapsed & SHAPES != 0 {
            7
        } else {
            r.shape_cols
        }
    }
    fn menu_buttons() -> [(Rect, &'static str); 3] {
        [
            (Rect::new(8, MENU_Y + 2, 48, 32), "File"),
            (Rect::new(57, MENU_Y + 2, 51, 32), "Edit"),
            (Rect::new(109, MENU_Y + 2, 57, 32), "View"),
        ]
    }
    fn bar_buttons(&self) -> [(Rect, u32); 6] {
        let w = self.width as i32;
        [
            (Rect::centred(193, MENU_Y + 18, 36), id::SAVE),
            (Rect::centred(237, MENU_Y + 18, 36), id::SHARE),
            (Rect::centred(283, MENU_Y + 18, 36), id::UNDO),
            (Rect::centred(322, MENU_Y + 18, 36), id::REDO),
            (Rect::centred(w - 64, MENU_Y + 18, 36), id::FEEDBACK),
            (Rect::centred(w - 28, MENU_Y + 18, 36), id::SETTINGS),
        ]
    }
    /// One expandable group's controls with its left edge at `g`, shifted
    /// down by `dy` (0 in the ribbon; a dropdown's offset otherwise).
    fn group_buttons(&self, group: u8, g: i32, dy: i32, cols: usize) -> Vec<(Rect, u32)> {
        let mut out = Vec::new();
        let (r1, r2) = (ROW1 + dy, ROW2 + dy);
        match group {
            IMAGE => {
                out.push((Rect::centred(g + 28, r1, 32), id::CROP));
                out.push((Rect::new(g + 52, r1 - 16, 32, 32), id::ROTATE));
                out.push((Rect::new(g + 84, r1 - 16, 16, 32), id::ROTATE_MENU));
                out.push((Rect::centred(g + 28, r2, 32), id::REMOVE_BG));
                out.push((Rect::new(g + 52, r2 - 16, 32, 32), id::FLIP));
                out.push((Rect::new(g + 84, r2 - 16, 16, 32), id::FLIP_MENU));
                out.push((Rect::new(g + 108, CARD_Y + 18 + dy, 46, 56), id::RESIZE));
            }
            TOOLS => {
                for (i, (cx, cy)) in [(27, r1), (67, r1), (108, r1), (27, r2), (67, r2), (108, r2)]
                    .into_iter()
                    .enumerate()
                {
                    out.push((Rect::centred(g + cx, cy, 34), id::TOOL + i as u32));
                }
            }
            SHAPES => {
                for i in 0..3 * cols {
                    let index = self.shapes_row * cols + i;
                    if index >= Shape::ALL.len() {
                        break;
                    }
                    let (c, row) = ((i % cols) as i32, (i / cols) as i32);
                    out.push((
                        Rect::centred(g + 24 + c * SHAPE_PITCH, CARD_Y + 22 + dy + row * 23, 22),
                        id::SHAPE + index as u32,
                    ));
                }
                let gallery_right = g + 9 + cols as i32 * SHAPE_PITCH + 17;
                out.push((
                    Rect::new(gallery_right - 14, CARD_Y + 8 + dy, 14, 38),
                    id::SHAPES_UP,
                ));
                out.push((
                    Rect::new(gallery_right - 14, CARD_Y + 46 + dy, 14, 38),
                    id::SHAPES_DOWN,
                ));
                out.push((Rect::new(gallery_right + 8, r1 - 16, 52, 32), id::OUTLINE));
                out.push((Rect::new(gallery_right + 8, r2 - 16, 52, 32), id::FILL));
                out.push((
                    Rect::new(gallery_right + 66, CARD_Y + 18 + dy, 40, 56),
                    id::STROKE,
                ));
            }
            _ => {
                out.push((Rect::centred(g + 28, r1, 36), id::PRIMARY));
                out.push((Rect::centred(g + 28, r2, 32), id::SECONDARY));
                for i in 0..20i32 {
                    let (c, row) = (i % 10, i / 10);
                    out.push((
                        Rect::centred(g + 65 + c * 24, CARD_Y + 21 + dy + row * 24, 22),
                        id::PALETTE + i as u32,
                    ));
                }
                for i in 0..10 {
                    out.push((
                        Rect::centred(g + 65 + i * 24, CARD_Y + 69 + dy, 22),
                        id::CUSTOM + i as u32,
                    ));
                }
                out.push((
                    Rect::centred(g + 319, CARD_Y + 45 + dy, 36),
                    id::EDIT_COLORS,
                ));
            }
        }
        out
    }
    /// The collapsed button of a group.
    fn collapsed_button(gx: i32) -> Rect {
        Rect::new(gx + 8, CARD_Y + 8, 50, 70)
    }
    /// Ribbon buttons: (rect, id).
    fn ribbon_buttons(&self) -> Vec<(Rect, u32)> {
        let r = self.ribbon();
        let mut out = Vec::new();
        out.push((Rect::new(r.selection + 6, CARD_Y + 8, 60, 46), id::SELECT));
        out.push((
            Rect::new(r.selection + 6, CARD_Y + 54, 60, 22),
            id::SELECT_MENU,
        ));
        for (group, gx) in [(IMAGE, r.image), (TOOLS, r.tools)] {
            if r.collapsed & group != 0 {
                out.push((
                    Self::collapsed_button(gx),
                    id::GROUP + group.trailing_zeros(),
                ));
            } else {
                out.extend(self.group_buttons(group, gx, 0, r.shape_cols));
            }
        }
        let g = r.brushes;
        out.push((Rect::new(g + 13, CARD_Y + 18, 40, 34), id::BRUSH));
        out.push((Rect::new(g + 13, CARD_Y + 52, 40, 22), id::BRUSH_MENU));
        for (group, gx) in [(SHAPES, r.shapes), (COLORS, r.colors)] {
            if r.collapsed & group != 0 {
                out.push((
                    Self::collapsed_button(gx),
                    id::GROUP + group.trailing_zeros(),
                ));
            } else {
                out.extend(self.group_buttons(group, gx, 0, r.shape_cols));
            }
        }
        out.push((Rect::new(r.solids + 10, CARD_Y + 8, 52, 70), id::SOLIDS));
        out.push((Rect::new(r.layers + 8, CARD_Y + 8, 50, 70), id::LAYERS));
        out
    }
    /// An open collapsed group's dropdown: (group, frame, left edge, dy).
    pub fn group_flyout(&self) -> Option<(u8, Rect, i32, i32)> {
        let Some(Menu::Group(group)) = self.menu else {
            return None;
        };
        let r = self.ribbon();
        let gx = match group {
            IMAGE => r.image,
            TOOLS => r.tools,
            SHAPES => r.shapes,
            _ => r.colors,
        };
        let w = group_width(group, 0, 7) as u32 + 8;
        let x = (gx + 8).min(self.width as i32 - w as i32 - 8).max(8);
        let y = CARD_Y + CARD_H - 30;
        let frame = Rect::new(x, y, w, 96);
        Some((group, frame, x + 4, y + 4 - CARD_Y - 4))
    }
    /// Where a group's sub-menus open: under its controls, or under its
    /// collapsed button.
    fn group_x(&self, group: u8, expanded: i32) -> i32 {
        let r = self.ribbon();
        if r.collapsed & group == 0 {
            return expanded;
        }
        match group {
            IMAGE => r.image + 8,
            TOOLS => r.tools + 8,
            SHAPES => r.shapes + 8,
            _ => r.colors + 8,
        }
    }
    pub fn zoom_slider(&self) -> Option<(i32, i32, u32, u32)> {
        if !self.status_bar {
            return None;
        }
        let w = self.width as i32;
        Some((w - 177, self.height as i32 - STATUS_H, 127, STATUS_H as u32))
    }
    fn status_buttons(&self) -> Vec<(Rect, u32)> {
        if !self.status_bar {
            return Vec::new();
        }
        let (w, y) = (self.width as i32, self.height as i32 - STATUS_H / 2);
        let mut v = vec![
            (Rect::centred(w - 343, y, 32), id::FIT),
            (Rect::new(w - 314, y - 16, 90, 32), id::ZOOM_MENU),
            (Rect::centred(w - 200, y, 32), id::ZOOM_OUT),
            (Rect::centred(w - 30, y, 32), id::ZOOM_IN),
        ];
        if let Some((x, top, sw, sh)) = self.zoom_slider() {
            v.push((Rect::new(x, top, sw, sh), id::ZOOM_SLIDER));
        }
        v
    }

    // ---- flyouts ----
    /// A list menu's anchor (top-left) and width.
    fn menu_anchor(&self, menu: Menu) -> (i32, i32, u32) {
        let r = self.ribbon();
        let below = CARD_Y + CARD_H - 30;
        match menu {
            Menu::File => (8, MENU_Y + 36, 280),
            Menu::Edit => (57, MENU_Y + 36, 260),
            Menu::View => (109, MENU_Y + 36, 260),
            Menu::Selection => (r.selection + 6, below, 220),
            Menu::Rotate => (self.group_x(IMAGE, r.image + 52), CARD_Y + 42, 200),
            Menu::Flip => (self.group_x(IMAGE, r.image + 52), CARD_Y + 82, 200),
            Menu::Outline => {
                let gr = r.shapes + 9 + r.shape_cols as i32 * SHAPE_PITCH + 17;
                (self.group_x(SHAPES, gr + 8), CARD_Y + 42, 180)
            }
            Menu::Fill => {
                let gr = r.shapes + 9 + r.shape_cols as i32 * SHAPE_PITCH + 17;
                (self.group_x(SHAPES, gr + 8), CARD_Y + 82, 180)
            }
            Menu::Zoom => {
                let n = ZOOMS.len() as i32 + 1;
                (
                    self.width as i32 - 314,
                    self.height as i32 - STATUS_H - 8 - n * 32 - 17,
                    160,
                )
            }
            Menu::LayerMore(i) => {
                let (x, y) = self
                    .layer_row(i)
                    .map_or((0, 0), |r| (r.x + r.w as i32 - 180, r.y + r.h as i32));
                (x, y, 200)
            }
            _ => (0, 0, 200),
        }
    }
    /// Flyout rows' rectangles for list menus.
    pub fn menu_rows(&self, menu: Menu) -> Vec<Rect> {
        let (x, mut y, w) = self.menu_anchor(menu);
        y += 4;
        let mut out = Vec::new();
        for r in self.rows(menu) {
            if r.separator_before {
                y += 9;
            }
            out.push(Rect::new(x + 4, y, w - 8, 32));
            y += 32;
        }
        out
    }
    fn menu_frame(&self, menu: Menu) -> Option<Rect> {
        match menu {
            Menu::Brushes => {
                let r = self.ribbon();
                Some(Rect::new(
                    r.brushes,
                    CARD_Y + CARD_H - 30,
                    3 * 48 + 8,
                    3 * 48 + 8,
                ))
            }
            Menu::Stroke => {
                let r = self.ribbon();
                let gr = r.shapes + 9 + r.shape_cols as i32 * SHAPE_PITCH + 17;
                Some(Rect::new(
                    self.group_x(SHAPES, gr + 40),
                    CARD_Y + CARD_H - 30,
                    160,
                    4 * 40 + 8,
                ))
            }
            Menu::Group(_) => self.group_flyout().map(|g| g.1),
            Menu::Solids => {
                let r = self.ribbon();
                Some(Rect::new(r.solids, CARD_Y + CARD_H - 30, 200, 6 * 40 + 8))
            }
            Menu::Colors => {
                let r = self.ribbon();
                let x = self
                    .group_x(COLORS, r.colors + 319 - 150)
                    .min(self.width as i32 - 312)
                    .max(8);
                Some(Rect::new(x, CARD_Y + CARD_H - 30, 304, 300))
            }
            Menu::Notice => {
                let w = 380u32.min(self.width - 40);
                Some(Rect::new(
                    (self.width as i32 - w as i32) / 2,
                    AREA_Y + 40,
                    w,
                    132,
                ))
            }
            m => {
                let rows = self.menu_rows(m);
                let (x, y, w) = self.menu_anchor(m);
                let bottom = rows.last().map_or(y + 8, |r| r.y + r.h as i32 + 4);
                Some(Rect::new(x, y, w, (bottom - y) as u32))
            }
        }
    }
    fn flyout_items(&self, menu: Menu) -> Vec<(Rect, u32)> {
        let Some(f) = self.menu_frame(menu) else {
            return Vec::new();
        };
        match menu {
            Menu::Brushes => (0..9)
                .map(|i: i32| {
                    (
                        Rect::new(f.x + 4 + (i % 3) * 48, f.y + 4 + (i / 3) * 48, 48, 48),
                        id::ITEM + i as u32,
                    )
                })
                .collect(),
            Menu::Stroke => (0..4)
                .map(|i| {
                    (
                        Rect::new(f.x + 4, f.y + 4 + i * 40, f.w - 8, 40),
                        id::ITEM + i as u32,
                    )
                })
                .collect(),
            Menu::Solids => (0..6)
                .map(|i| {
                    (
                        Rect::new(f.x + 4, f.y + 4 + i * 40, f.w - 8, 40),
                        id::ITEM + i as u32,
                    )
                })
                .collect(),
            Menu::Colors => vec![
                (
                    Rect::new(f.x + f.w as i32 - 200, f.y + f.h as i32 - 44, 92, 32),
                    id::ITEM,
                ),
                (
                    Rect::new(f.x + f.w as i32 - 100, f.y + f.h as i32 - 44, 92, 32),
                    id::ITEM + 1,
                ),
            ],
            Menu::Notice => vec![(
                Rect::new(f.x + f.w as i32 - 104, f.y + f.h as i32 - 44, 92, 32),
                id::BACKDROP,
            )],
            Menu::Group(group) => {
                let (_, _, gx, dy) = self.group_flyout().unwrap_or((group, f, f.x, 0));
                self.group_buttons(group, gx, dy, 7)
            }
            m => {
                let rows = self.rows(m);
                self.menu_rows(m)
                    .into_iter()
                    .zip(rows)
                    .enumerate()
                    .filter(|(_, (_, r))| r.enabled)
                    .map(|(i, (rect, _))| (rect, id::ITEM + i as u32))
                    .collect()
            }
        }
    }
    pub fn spectrum_rect(&self) -> Option<Rect> {
        let f = self.menu_frame(Menu::Colors)?;
        Some(Rect::new(f.x + 16, f.y + 16, f.w - 32, 150))
    }
    pub fn hue_rect(&self) -> Option<Rect> {
        let f = self.menu_frame(Menu::Colors)?;
        Some(Rect::new(f.x + 16, f.y + 178, f.w - 32, 14))
    }
    pub fn spectrum_at(&mut self, x: i32, y: i32) {
        if let Some(r) = self.spectrum_rect() {
            self.hsv.s = ((x - r.x) as f32 / r.w as f32).clamp(0.0, 1.0);
            self.hsv.v = 1.0 - ((y - r.y) as f32 / r.h as f32).clamp(0.0, 1.0);
        }
    }
    pub fn hue_at(&mut self, x: i32) {
        if let Some(r) = self.hue_rect() {
            self.hsv.h = ((x - r.x) as f32 / r.w as f32).clamp(0.0, 0.9999) * 360.0;
        }
    }

    // ---- layers pane ----
    fn layers_pane(&self) -> Option<Rect> {
        self.layers_open.then(|| {
            let (_, ay, aw, ah) = self.area();
            Rect::new(aw as i32, ay, LAYERS_W, ah)
        })
    }
    /// Row of layer `i` (top layer first).
    pub fn layer_row(&self, i: usize) -> Option<Rect> {
        let p = self.layers_pane()?;
        let n = self.doc.layers.len();
        let pos = n.checked_sub(1 + i)? as i32;
        Some(Rect::new(p.x + 8, p.y + 52 + pos * 72, p.w - 16, 68))
    }

    // ---- dialogs ----
    pub fn dialog_frame(&self) -> Option<Rect> {
        let (w, h) = match self.dialog.as_ref()? {
            Dialog::Resize { .. } => (420, 330),
            Dialog::Properties { .. } => (380, 250),
            Dialog::Open { .. } => (520, 440),
            Dialog::SaveAs { .. } => (440, 210),
            Dialog::Unsaved { .. } => (440, 190),
            Dialog::Settings => (440, 300),
        };
        let (w, h) = (
            w.min(self.width - 24),
            h.min(self.height - TITLE as u32 - 12),
        );
        Some(Rect::new(
            (self.width as i32 - w as i32) / 2,
            (self.height as i32 - h as i32) / 2 + TITLE / 2,
            w,
            h,
        ))
    }
    /// Footer buttons (right-aligned, primary first) and other controls.
    fn dialog_items(&self) -> Vec<(Rect, u32)> {
        let Some(f) = self.dialog_frame() else {
            return Vec::new();
        };
        let footer = |labels: usize, first: u32| -> Vec<(Rect, u32)> {
            let bw = ((f.w as i32 - 48 - 8 * (labels as i32 - 1)) / labels as i32).min(130);
            let mut x = f.x + f.w as i32 - 24 - bw * labels as i32 - 8 * (labels as i32 - 1);
            let y = f.y + f.h as i32 - 56;
            (0..labels)
                .map(|i| {
                    let r = (
                        Rect::new(x, y, bw as u32, 32),
                        id::DIALOG + first + i as u32,
                    );
                    x += bw + 8;
                    r
                })
                .collect()
        };
        let mut v = Vec::new();
        match self.dialog.as_ref().unwrap() {
            Dialog::Resize { .. } => {
                v.push((Rect::new(f.x + 24, f.y + 84, 140, 32), id::DIALOG));
                v.push((Rect::new(f.x + 172, f.y + 84, 140, 32), id::DIALOG + 1));
                v.push((Rect::new(f.x + 150, f.y + 132, 120, 32), id::DIALOG + 2));
                v.push((Rect::new(f.x + 150, f.y + 172, 120, 32), id::DIALOG + 3));
                v.push((Rect::new(f.x + 24, f.y + 214, 240, 28), id::DIALOG + 4));
                v.extend(footer(2, 5));
            }
            Dialog::Properties { .. } => {
                v.push((Rect::new(f.x + 120, f.y + 76, 120, 32), id::DIALOG));
                v.push((Rect::new(f.x + 120, f.y + 120, 120, 32), id::DIALOG + 1));
                v.extend(footer(2, 2));
            }
            Dialog::Open { files, first, .. } => {
                for i in 0..8.min(files.len().saturating_sub(*first)) {
                    v.push((
                        Rect::new(f.x + 20, f.y + 72 + i as i32 * 34, f.w - 40, 32),
                        id::DIALOG + 10 + i as u32,
                    ));
                }
                v.extend(footer(2, 0));
            }
            Dialog::SaveAs { .. } => v.extend(footer(2, 0)),
            Dialog::Unsaved { .. } => v.extend(footer(3, 0)),
            Dialog::Settings => {
                for i in 0..3 {
                    v.push((
                        Rect::new(f.x + 24, f.y + 98 + i * 36, f.w - 48, 32),
                        id::DIALOG + i as u32,
                    ));
                }
                v.extend(footer(1, 3));
            }
        }
        v
    }

    /// Every hit target in front-to-back order.
    pub fn hits(&self) -> Vec<(Rect, u32)> {
        if self.dialog.is_some() {
            return self.dialog_items();
        }
        let full = Rect::new(0, 0, self.width, self.height);
        if let Some(m) = self.menu {
            let mut v = self.flyout_items(m);
            if let Some(f) = self.menu_frame(m) {
                v.push((f, u32::MAX - 1));
            }
            // A click on the same menu button closes it.
            for (i, (r, _)) in Self::menu_buttons().into_iter().enumerate() {
                v.push((r, id::MENU + i as u32));
            }
            v.push((full, id::BACKDROP));
            return v;
        }
        let mut v: Vec<(Rect, u32)> = Self::menu_buttons()
            .into_iter()
            .enumerate()
            .map(|(i, (r, _))| (r, id::MENU + i as u32))
            .collect();
        v.extend(self.bar_buttons().into_iter().filter(|(_, i)| match *i {
            id::UNDO => self.history.can_undo() || self.selection.is_some() || self.text.is_some(),
            id::REDO => self.history.can_redo(),
            _ => true,
        }));
        v.extend(self.ribbon_buttons());
        if self.layers_open {
            if let Some(p) = self.layers_pane() {
                v.push((
                    Rect::centred(p.x + p.w as i32 - 28, p.y + 24, 32),
                    id::LAYER_ADD,
                ));
            }
            for i in 0..self.doc.layers.len() {
                if let Some(r) = self.layer_row(i) {
                    v.push((
                        Rect::centred(r.x + r.w as i32 - 52, r.y + r.h as i32 / 2, 28),
                        id::LAYER_EYE + i as u32,
                    ));
                    v.push((
                        Rect::centred(r.x + r.w as i32 - 20, r.y + r.h as i32 / 2, 28),
                        id::LAYER_MORE + i as u32,
                    ));
                    v.push((r, id::LAYER + i as u32));
                }
            }
        }
        v.extend(self.status_buttons());
        v
    }

    // ---- painting ----
    pub fn dark(&self, v: &View<'_>) -> bool {
        match self.theme {
            0 => false,
            1 => true,
            _ => v.palette.dark,
        }
    }
    pub fn paint(&self, v: &mut View<'_>) {
        let dark = self.dark(v);
        v.palette.dark = dark;
        let k = ink(dark);
        let (w, h) = (self.width, self.height);
        if v.palette.system().is_some() {
            v.rect(0, 0, w, h, k.mica);
        } else {
            v.material_rect(0, 0, w, h, k.mica, 230);
        }
        // Title: the app icon and the document name.
        if let Some(px) = crate::icon_frames(crate::App::Paint, v.scale) {
            v.color_pixels(10, 8, 16, px);
        }
        v.text_clip_centered(
            34,
            (0, TITLE as u32),
            400,
            &self.title,
            TextStyle::Regular12,
            k.text,
        );
        self.paint_menu_bar(v, &k);
        self.paint_area(v, &k);
        self.paint_ribbon(v, &k);
        if self.layers_open {
            self.paint_layers(v, &k);
        }
        if self.status_bar {
            self.paint_status(v, &k);
        }
        if let Some(m) = self.menu {
            self.paint_flyout(v, &k, m);
        }
        if self.dialog.is_some() {
            self.paint_dialog(v, &k);
        }
    }
    fn hovered(&self, v: &View<'_>, r: Rect) -> bool {
        (self.menu.is_none() || matches!(self.menu, Some(Menu::Group(_))))
            && self.dialog.is_none()
            && v.hover.is_some_and(|(x, y)| r.contains(x, y))
    }
    fn button_back(&self, v: &mut View<'_>, k: &Ink, r: Rect, selected: bool) {
        let hover = self.hovered(v, r);
        if selected {
            v.rounded_asset(r.x, r.y, r.w, r.h, 4, k.accent);
            v.rounded_asset(
                r.x + 1,
                r.y + 1,
                r.w - 2,
                r.h - 2,
                3,
                if hover { k.pressed } else { k.hover },
            );
        } else if hover {
            v.rounded_asset(r.x, r.y, r.w, r.h, 4, k.hover);
        }
    }
    fn paint_menu_bar(&self, v: &mut View<'_>, k: &Ink) {
        for (i, (r, label)) in Self::menu_buttons().into_iter().enumerate() {
            let open = self.menu == Some([Menu::File, Menu::Edit, Menu::View][i]);
            if open {
                v.rounded_asset(r.x, r.y, r.w, r.h, 4, k.pressed);
            } else if self.hovered(v, r) {
                v.rounded_asset(r.x, r.y, r.w, r.h, 4, k.hover);
            }
            let tw = v.measure_style(label, TextStyle::Regular14) as i32;
            v.text_clip_centered(
                r.x + (r.w as i32 - tw) / 2,
                (r.y, r.h),
                r.w,
                label,
                TextStyle::Regular14,
                k.text,
            );
        }
        for x in [170, 215, 260] {
            v.rect(x, MENU_Y + 9, 1, 18, k.divider);
        }
        for (r, i) in self.bar_buttons() {
            let enabled = match i {
                id::UNDO => {
                    self.history.can_undo() || self.selection.is_some() || self.text.is_some()
                }
                id::REDO => self.history.can_redo(),
                _ => true,
            };
            if enabled && self.hovered(v, r) {
                v.rounded_asset(r.x + 2, r.y + 2, r.w - 4, r.h - 4, 4, k.hover);
            }
            let name = match i {
                id::SAVE => "paint-save",
                id::SHARE => "paint-share",
                id::UNDO => "paint-undo",
                id::REDO => "paint-redo",
                id::FEEDBACK => "paint-feedback",
                _ => "paint-settings",
            };
            glyph(
                v,
                name,
                r.x + 8,
                r.y + 8,
                20,
                if enabled { k.text } else { k.disabled },
            );
        }
    }
    fn paint_ribbon(&self, v: &mut View<'_>, k: &Ink) {
        let cw = self.width - 14;
        v.rounded_asset(7, CARD_Y, cw, CARD_H as u32, 8, k.stroke);
        v.rounded_asset(8, CARD_Y + 1, cw - 2, CARD_H as u32 - 2, 7, k.card);
        let r = self.ribbon();
        let dark = k.dark;
        let label = |v: &mut View<'_>, x: i32, w: i32, text: &str| {
            let tw = v.measure_style(text, TextStyle::Regular12) as i32;
            v.text_clip_centered(
                x + (w - tw) / 2,
                (LABEL_Y - 10, 20),
                w as u32,
                text,
                TextStyle::Regular12,
                k.secondary,
            );
        };
        let divider = |v: &mut View<'_>, x: i32| v.rect(x, CARD_Y + 10, 1, 95, k.divider);
        let chevron = |v: &mut View<'_>, cx: i32, cy: i32| {
            glyph(v, "paint-chevron-down", cx - 6, cy - 6, 12, k.text)
        };
        let buttons = self.ribbon_buttons();
        let rect_of = |target: u32| buttons.iter().find(|(_, i)| *i == target).map(|(r, _)| *r);
        // Selection.
        let g = r.selection;
        let selecting = matches!(self.tool, Tool::Select | Tool::FreeSelect);
        if let (Some(a), Some(b)) = (rect_of(id::SELECT), rect_of(id::SELECT_MENU)) {
            self.button_back(v, k, Rect::new(a.x, a.y, a.w, a.h + b.h), selecting);
        }
        icon(
            v,
            if self.tool == Tool::FreeSelect {
                "freeform"
            } else {
                "select"
            },
            dark,
            g + 28,
            CARD_Y + 17,
            20,
        );
        chevron(v, g + 38, CARD_Y + 63);
        label(v, g, GROUP_SELECTION - 2, "Selection");
        divider(v, g + GROUP_SELECTION - 2);
        let collapsed_group = |v: &mut View<'_>, group: u8, gx: i32, name: &str| {
            let b = Self::collapsed_button(gx);
            let open = self.menu == Some(Menu::Group(group));
            let selected = match group {
                TOOLS => matches!(
                    self.tool,
                    Tool::Pencil
                        | Tool::Fill
                        | Tool::Text
                        | Tool::Eraser
                        | Tool::Picker
                        | Tool::Magnifier
                ),
                SHAPES => self.tool == Tool::Shape,
                _ => false,
            };
            if open {
                v.rounded_asset(b.x, b.y, b.w, b.h, 4, k.pressed);
            } else {
                self.button_back(v, k, b, selected);
            }
            match group {
                IMAGE => icon(v, "resize", dark, b.x + 15, b.y + 14, 20),
                TOOLS => icon(v, "tools", dark, b.x + 17, b.y + 16, 16),
                SHAPES => icon(v, "shapes", dark, b.x + 15, b.y + 14, 20),
                _ => {
                    let c = self.active_color();
                    v.rounded_asset(
                        b.x + 13,
                        b.y + 12,
                        24,
                        24,
                        12,
                        if k.dark {
                            rgb(90, 90, 90)
                        } else {
                            rgb(190, 190, 190)
                        },
                    );
                    v.rounded_asset(b.x + 14, b.y + 13, 22, 22, 11, c.bgra());
                }
            }
            chevron(v, b.x + 25, b.y + 50);
            label(v, gx, GROUP_COLLAPSED - 2, name);
            divider(v, gx + GROUP_COLLAPSED - 2);
        };
        // Image and Tools.
        for (group, gx, name, width) in [
            (IMAGE, r.image, "Image", GROUP_IMAGE),
            (TOOLS, r.tools, "Tools", GROUP_TOOLS),
        ] {
            if r.collapsed & group != 0 {
                collapsed_group(v, group, gx, name);
            } else {
                self.paint_group(v, k, group, gx, 0, r.shape_cols, &buttons);
                label(v, gx, width - 2, name);
                divider(v, gx + width - 2);
            }
        }
        // Brushes.
        let g = r.brushes;
        if let (Some(a), Some(b)) = (rect_of(id::BRUSH), rect_of(id::BRUSH_MENU)) {
            self.button_back(
                v,
                k,
                Rect::new(a.x, a.y, a.w, a.h + b.h),
                self.tool == Tool::Brush,
            );
            icon(v, self.brush.icon(), dark, a.x + 10, a.y + 9, 20);
            chevron(v, b.x + 20, b.y + 10);
        }
        label(v, g, GROUP_BRUSHES - 2, "Brushes");
        divider(v, g + GROUP_BRUSHES - 2);
        // Shapes and Colors.
        for (group, gx, name, width) in [
            (SHAPES, r.shapes, "Shapes", shapes_width(r.shape_cols)),
            (COLORS, r.colors, "Colors", GROUP_COLORS),
        ] {
            if r.collapsed & group != 0 {
                collapsed_group(v, group, gx, name);
            } else {
                self.paint_group(v, k, group, gx, 0, r.shape_cols, &buttons);
                label(v, gx, width - 2, name);
                divider(v, gx + width - 2);
            }
        }
        // Paint 3D: 3D shapes.
        let g = r.solids;
        if let Some(b) = rect_of(id::SOLIDS) {
            self.button_back(v, k, b, self.tool == Tool::Solid);
            if let Some((px, tile)) = self.solid_icon(self.solid) {
                let scale = v.scale.clamp(1, 3);
                if px == 20 * scale {
                    v.color_pixels(b.x + 16, b.y + 14, 20, tile);
                }
            }
            chevron(v, b.x + 26, b.y + 56);
        }
        label(v, g, GROUP_SOLIDS - 2, "3D shapes");
        divider(v, g + GROUP_SOLIDS - 2);
        // Layers.
        let g = r.layers;
        if let Some(b) = rect_of(id::LAYERS) {
            self.button_back(v, k, b, self.layers_open);
            icon(v, "layers", dark, b.x + 15, b.y + 22, 20);
        }
        label(v, g, GROUP_LAYERS - 2, "Layers");
    }
    /// One expandable group's controls (in the ribbon or its dropdown).
    #[allow(clippy::too_many_arguments)]
    fn paint_group(
        &self,
        v: &mut View<'_>,
        k: &Ink,
        group: u8,
        g: i32,
        dy: i32,
        cols: usize,
        buttons: &[(Rect, u32)],
    ) {
        let dark = k.dark;
        let chevron = |v: &mut View<'_>, cx: i32, cy: i32| {
            glyph(v, "paint-chevron-down", cx - 6, cy - 6, 12, k.text)
        };
        let rect_of = |target: u32| buttons.iter().find(|(_, i)| *i == target).map(|(r, _)| *r);
        match group {
            IMAGE => {
                for (target, key) in [
                    (id::CROP, "crop"),
                    (id::ROTATE, "rotate"),
                    (id::REMOVE_BG, "remove-background"),
                    (id::FLIP, "flip"),
                ] {
                    if let Some(b) = rect_of(target) {
                        let pair = match target {
                            id::ROTATE => rect_of(id::ROTATE_MENU),
                            id::FLIP => rect_of(id::FLIP_MENU),
                            _ => None,
                        };
                        let whole = pair.map_or(b, |p| Rect::new(b.x, b.y, b.w + p.w, b.h));
                        self.button_back(v, k, whole, false);
                        if key == "crop" {
                            glyph(
                                v,
                                "paint-crop",
                                b.x + 8,
                                b.y + 8,
                                16,
                                if self.selection.is_some() {
                                    k.text
                                } else {
                                    k.disabled
                                },
                            );
                        } else {
                            icon(v, key, dark, b.x + 8, b.y + 8, 16);
                        }
                        if pair.is_some() {
                            chevron(v, b.x + b.w as i32 + 8, b.y + 16);
                        }
                    }
                }
                if let Some(b) = rect_of(id::RESIZE) {
                    self.button_back(v, k, b, false);
                    icon(v, "resize", dark, b.x + 13, b.y + 18, 20);
                }
            }
            TOOLS => {
                for (i, (tool, key)) in [
                    (Tool::Pencil, "pencil"),
                    (Tool::Fill, "fill"),
                    (Tool::Text, "text"),
                    (Tool::Eraser, "eraser"),
                    (Tool::Picker, "picker"),
                    (Tool::Magnifier, "magnifier"),
                ]
                .into_iter()
                .enumerate()
                {
                    if let Some(b) = rect_of(id::TOOL + i as u32) {
                        self.button_back(v, k, b, self.tool == tool);
                        icon(v, key, dark, b.x + 9, b.y + 9, 16);
                    }
                }
            }
            SHAPES => {
                let gw = (cols as i32 * SHAPE_PITCH + 17) as u32;
                v.rounded_asset(g + 9, CARD_Y + 7 + dy, gw, 78, 4, k.divider);
                v.rounded_asset(
                    g + 10,
                    CARD_Y + 8 + dy,
                    gw - 2,
                    76,
                    3,
                    if dy == 0 { k.card } else { k.flyout },
                );
                for (b, target) in buttons.iter().filter(|(_, t)| (id::SHAPE..=72).contains(t)) {
                    let shape = Shape::ALL[(target - id::SHAPE) as usize];
                    self.button_back(v, k, *b, self.tool == Tool::Shape && self.shape == shape);
                    icon(v, shape.icon(), dark, b.x + 3, b.y + 3, 16);
                }
                let gallery_right = g + 9 + gw as i32;
                let rows = Shape::ALL.len().div_ceil(cols);
                let up = self.shapes_row > 0;
                let down = self.shapes_row + 3 < rows;
                glyph(
                    v,
                    "paint-chevron-up",
                    gallery_right - 13,
                    CARD_Y + 20 + dy,
                    12,
                    if up { k.text } else { k.disabled },
                );
                glyph(
                    v,
                    "paint-chevron-down",
                    gallery_right - 13,
                    CARD_Y + 60 + dy,
                    12,
                    if down { k.text } else { k.disabled },
                );
                let shape_tool = self.tool == Tool::Shape;
                for (target, name) in [(id::OUTLINE, "paint-outline"), (id::FILL, "paint-fill")] {
                    if let Some(b) = rect_of(target) {
                        self.button_back(v, k, b, false);
                        glyph(
                            v,
                            name,
                            b.x + 8,
                            b.y + 6,
                            20,
                            if shape_tool { k.text } else { k.disabled },
                        );
                        chevron(v, b.x + 40, b.y + 16);
                    }
                }
                if let Some(b) = rect_of(id::STROKE) {
                    self.button_back(v, k, b, false);
                    icon(
                        v,
                        if shape_tool {
                            "stroke-size"
                        } else {
                            "stroke-size-disabled"
                        },
                        dark,
                        b.x + 12,
                        b.y + 12,
                        16,
                    );
                    chevron(v, b.x + 20, b.y + 44);
                }
            }
            _ => {
                let back = if dy == 0 { k.card } else { k.flyout };
                let ring = |v: &mut View<'_>, cx: i32, cy: i32, d: u32, c: Rgba, active: bool| {
                    if active {
                        v.rounded_asset(
                            cx - d as i32 / 2 - 4,
                            cy - d as i32 / 2 - 4,
                            d + 8,
                            d + 8,
                            d / 2 + 4,
                            k.accent,
                        );
                        v.rounded_asset(
                            cx - d as i32 / 2 - 2,
                            cy - d as i32 / 2 - 2,
                            d + 4,
                            d + 4,
                            d / 2 + 2,
                            back,
                        );
                    }
                    v.rounded_asset(
                        cx - d as i32 / 2,
                        cy - d as i32 / 2,
                        d,
                        d,
                        d / 2,
                        if k.dark {
                            rgb(90, 90, 90)
                        } else {
                            rgb(190, 190, 190)
                        },
                    );
                    v.rounded_asset(
                        cx - d as i32 / 2 + 1,
                        cy - d as i32 / 2 + 1,
                        d - 2,
                        d - 2,
                        d / 2 - 1,
                        c.bgra(),
                    );
                };
                ring(
                    v,
                    g + 28,
                    ROW1 + dy,
                    28,
                    self.primary,
                    !self.editing_secondary,
                );
                ring(
                    v,
                    g + 28,
                    ROW2 + dy,
                    24,
                    self.secondary,
                    self.editing_secondary,
                );
                for (i, c) in PALETTE.iter().enumerate() {
                    let (cx, cy) = (
                        g + 65 + (i % 10) as i32 * 24,
                        CARD_Y + 21 + dy + (i / 10) as i32 * 24,
                    );
                    let hot = v
                        .hover
                        .is_some_and(|(x, y)| Rect::centred(cx, cy, 22).contains(x, y))
                        && self.dialog.is_none();
                    if hot {
                        v.rounded_asset(cx - 10, cy - 10, 20, 20, 10, k.accent);
                    }
                    ring(v, cx, cy, 16, *c, false);
                }
                for (i, c) in self.custom.iter().enumerate() {
                    let (cx, cy) = (g + 65 + i as i32 * 24, CARD_Y + 69 + dy);
                    match c {
                        Some(c) => ring(v, cx, cy, 16, *c, false),
                        None => {
                            v.rounded_asset(
                                cx - 8,
                                cy - 8,
                                16,
                                16,
                                8,
                                if k.dark {
                                    rgb(110, 110, 110)
                                } else {
                                    rgb(170, 170, 170)
                                },
                            );
                            v.rounded_asset(cx - 7, cy - 7, 14, 14, 7, back);
                        }
                    }
                }
                if let Some(b) = rect_of(id::EDIT_COLORS) {
                    self.button_back(v, k, b, self.menu == Some(Menu::Colors));
                    self.paint_wheel(v, b.x + 6, b.y + 6, 24);
                }
            }
        }
    }
    /// The Edit colors button's colour wheel (hue around, white centre).
    fn paint_wheel(&self, v: &mut View<'_>, x: i32, y: i32, d: u32) {
        let scale = v.scale.clamp(1, 3);
        let n = (d * scale) as usize;
        let mut px = vec![0u8; n * n * 4];
        let r = n as f32 / 2.0;
        for j in 0..n {
            for i in 0..n {
                let (dx, dy) = (i as f32 + 0.5 - r, j as f32 + 0.5 - r);
                let dist = sqrt_f(dx * dx + dy * dy);
                let cover = (r - dist).clamp(0.0, 1.0);
                if cover <= 0.0 {
                    continue;
                }
                let hue = crate::atan2(dy, dx).to_degrees_f() + 180.0;
                let c = hsv_rgb(Hsv {
                    h: hue,
                    s: (dist / r).clamp(0.0, 1.0),
                    v: 1.0,
                });
                let a = (cover * 255.0) as u32;
                let at = (j * n + i) * 4;
                px[at] = (c.b as u32 * a / 255) as u8;
                px[at + 1] = (c.g as u32 * a / 255) as u8;
                px[at + 2] = (c.r as u32 * a / 255) as u8;
                px[at + 3] = a as u8;
            }
        }
        v.color_pixels(x, y, d as usize, &px);
    }
    fn paint_area(&self, v: &mut View<'_>, k: &Ink) {
        let (ax, ay, aw, ah) = self.area();
        v.rect(ax, ay, aw, ah, k.backdrop);
        let (ox, oy) = self.origin();
        let (cw, ch) = self.canvas_size();
        let p = self.percent as i64;
        // Logical canvas rectangle (for the shadow and overlays).
        let (l, t) = ((ox * 100 / p) as i32, (oy * 100 / p) as i32);
        let (lw, lh) = ((cw * 100 / p) as u32, (ch * 100 / p) as u32);
        v.viewport(
            door_graphics_core::Rect::new(ax, ay, aw.max(1), ah.max(1)).expect("area"),
            0,
        );
        v.elevation(l, t, lw, lh, if k.dark { 90 } else { 50 });
        // Transparency shows as a checkerboard under the image.
        if !self.doc.layers[0].visible {
            let cell = 8;
            for yy in (0..lh as i32).step_by(cell) {
                for xx in (0..lw as i32).step_by(cell) {
                    let on = ((xx / cell as i32) + (yy / cell as i32)) % 2 == 0;
                    v.rect(
                        l + xx,
                        t + yy,
                        cell as u32,
                        cell as u32,
                        if on {
                            rgb(255, 255, 255)
                        } else {
                            rgb(204, 204, 204)
                        },
                    );
                }
            }
        }
        v.image_scaled(
            (ax, ay, aw, ah),
            ox,
            oy,
            self.zoom,
            1000,
            self.doc.w,
            self.doc.h,
            self.composite(),
        );
        // A moved selection floats above the canvas.
        if let Some(f) = self.selection.as_ref().and_then(|s| s.floating.as_ref()) {
            let fx = ox + f.x as i64 * self.zoom as i64 / 1000;
            let fy = oy + f.y as i64 * self.zoom as i64 / 1000;
            v.image_scaled((ax, ay, aw, ah), fx, fy, self.zoom, 1000, f.w, f.h, &f.px);
        }
        if self.gridlines && self.zoom >= 4000 {
            let step = self.zoom as f32 / 1000.0 * 100.0 / self.percent as f32;
            let grid = if k.dark {
                [60, 60, 60, 255]
            } else {
                [208, 208, 208, 255]
            };
            let mut gx = l as f32;
            while gx < (l + lw as i32) as f32 && gx < (ax + aw as i32) as f32 {
                if gx >= ax as f32 {
                    v.rect(gx as i32, t.max(ay), 1, lh.min(ah), grid);
                }
                gx += step;
            }
            let mut gy = t as f32;
            while gy < (t + lh as i32) as f32 && gy < (ay + ah as i32) as f32 {
                if gy >= ay as f32 {
                    v.rect(l.max(ax), gy as i32, lw.min(aw), 1, grid);
                }
                gy += step;
            }
        }
        // Selection outline (dashed).
        if let Some(sel) = &self.selection {
            let b = match &sel.floating {
                Some(f) => Bounds::new(f.x, f.y, f.x + f.w as i32, f.y + f.h as i32),
                None => sel.bounds,
            };
            let (x0, y0) = self.to_logical(b.x0 as f32, b.y0 as f32);
            let (x1, y1) = self.to_logical(b.x1 as f32, b.y1 as f32);
            dashed(
                v,
                x0,
                y0,
                (x1 - x0).max(1) as u32,
                (y1 - y0).max(1) as u32,
                k.accent,
            );
        }
        if matches!(self.drag, Drag::Lasso) {
            for pair in self.lasso.windows(2) {
                let (a, b) = (
                    self.to_logical(pair[0].0, pair[0].1),
                    self.to_logical(pair[1].0, pair[1].1),
                );
                line(v, a, b, k.accent);
            }
        }
        // Canvas resize: the eight handles, and the new bounds while dragging.
        if self.selection.is_none() && self.text.is_none() {
            for ((hx, hy), _) in self.handles() {
                v.rect(
                    hx - 3,
                    hy - 3,
                    7,
                    7,
                    if k.dark {
                        rgb(20, 20, 20)
                    } else {
                        rgb(100, 100, 100)
                    },
                );
                v.rect(hx - 2, hy - 2, 5, 5, rgb(255, 255, 255));
            }
        }
        if let Some(b) = self.resize_preview {
            let (x0, y0) = self.to_logical(b.x0 as f32, b.y0 as f32);
            let (x1, y1) = self.to_logical(b.x1 as f32, b.y1 as f32);
            dashed(
                v,
                x0,
                y0,
                (x1 - x0).max(1) as u32,
                (y1 - y0).max(1) as u32,
                k.text,
            );
        }
        // The text box being typed.
        if let Some(tb) = &self.text {
            self.paint_text_box(v, k, tb);
        }
        // The brush outline under the pointer.
        if let Some((x, y)) = self.cursor
            && self.drag == Drag::None
            && self.menu.is_none()
            && self.dialog.is_none()
            && self.over_canvas(x, y)
            && matches!(self.tool, Tool::Brush | Tool::Eraser | Tool::Pencil)
        {
            let size = match self.tool {
                Tool::Pencil => self.size_pencil,
                Tool::Eraser => self.size_eraser,
                _ => self.size_brush,
            };
            let d = (size as f32 * self.zoom as f32 / 1000.0 * 100.0 / self.percent as f32) as u32;
            if d >= 6 {
                if self.tool == Tool::Eraser {
                    dashed(
                        v,
                        x - d as i32 / 2,
                        y - d as i32 / 2,
                        d,
                        d,
                        if k.dark {
                            rgb(230, 230, 230)
                        } else {
                            rgb(40, 40, 40)
                        },
                    );
                } else {
                    circle(
                        v,
                        x,
                        y,
                        d as f32 / 2.0,
                        if k.dark {
                            rgb(230, 230, 230)
                        } else {
                            rgb(40, 40, 40)
                        },
                    );
                }
            }
        }
        if self.rulers {
            self.paint_rulers(v, k, l, t);
        }
        // Scrollbars when the canvas is larger than the view.
        let (hbar, vbar) = self.scrollbars();
        for (track, thumb) in hbar.into_iter().chain(vbar) {
            let hot = v.hover.is_some_and(|(x, y)| track.contains(x, y));
            if hot {
                v.rounded_asset(
                    track.x,
                    track.y,
                    track.w,
                    track.h,
                    5,
                    if k.dark {
                        [40, 40, 40, 200]
                    } else {
                        [235, 235, 235, 220]
                    },
                );
            }
            v.rounded_asset(
                thumb.x,
                thumb.y,
                thumb.w,
                thumb.h,
                3,
                if k.dark {
                    rgb(159, 159, 159)
                } else {
                    rgb(133, 133, 133)
                },
            );
        }
        v.reset_viewport();
        // The floating slider panels.
        for size in [true, false] {
            self.paint_rail(v, k, size);
        }
    }
    fn paint_rulers(&self, v: &mut View<'_>, k: &Ink, l: i32, t: i32) {
        let (ax, ay, aw, _) = self.area();
        v.rect(ax, ay, aw, 20, k.card);
        v.rect(ax, ay + 20, aw, 1, k.stroke);
        let step_px = self.zoom as f32 / 1000.0 * 100.0 / self.percent as f32;
        let major = [10, 25, 50, 100, 250, 500, 1000]
            .into_iter()
            .find(|m| *m as f32 * step_px >= 60.0)
            .unwrap_or(1000);
        let mut n = 0;
        while (n as f32 * step_px) as i32 + l < ax + aw as i32 {
            let x = l + (n as f32 * step_px) as i32;
            if x >= ax {
                let long = n % major == 0;
                v.rect(
                    x,
                    ay + if long { 6 } else { 14 },
                    1,
                    if long { 14 } else { 6 },
                    k.secondary,
                );
                if long {
                    v.text_clip(
                        x + 3,
                        ay + 2,
                        40,
                        &format!("{n}"),
                        TextStyle::Regular12,
                        k.secondary,
                    );
                }
            }
            n += (major / 5).max(1);
        }
        let _ = t;
    }
    fn paint_text_box(&self, v: &mut View<'_>, k: &Ink, tb: &TextBox) {
        let Some(system) = door_workspace::assets::text_system() else {
            return;
        };
        let scale = self.zoom as f32 / 1000.0;
        let logical_size = tb.size as f32 * scale * 100.0 / self.percent as f32;
        let format = system.format(
            logical_size.max(4.0),
            400,
            v.zoom.percent() as f32 / 100.0,
            "en-US",
        );
        let line_h = (logical_size * 1.33) as i32;
        let (x, y) = self.to_logical(tb.x as f32, tb.y as f32);
        let mut width = (logical_size * 4.0) as i32;
        let mut c = self.primary;
        c.a = self.alpha().max(1);
        let lines: Vec<&str> = tb.text.split('\n').collect();
        for (i, line) in lines.iter().enumerate() {
            if line.is_empty() {
                continue;
            }
            if let Ok(layout) = system.layout_line(line, &format) {
                let (pw, _) = layout.physical_size();
                width = width.max((pw as f32 * 100.0 / v.zoom.percent() as f32) as i32 + 8);
                v.text_layout(x, y + i as i32 * line_h, &system, &layout, c.bgra());
            }
        }
        let height = (lines.len().max(1) as i32 * line_h + 6) as u32;
        dashed(v, x - 4, y - 3, width as u32 + 8, height, k.accent);
        // The caret after the last character.
        let last = lines.last().copied().unwrap_or("");
        let cx = if last.is_empty() {
            x
        } else {
            system.layout_line(last, &format).map_or(x, |l| {
                x + (l.physical_size().0 as f32 * 100.0 / v.zoom.percent() as f32) as i32
            })
        };
        v.rect(
            cx + 1,
            y + (lines.len() as i32 - 1) * line_h,
            1,
            line_h.max(8) as u32,
            k.text,
        );
    }
    fn paint_rail(&self, v: &mut View<'_>, k: &Ink, size: bool) {
        let Some((x, y, w, h)) = self.rail(size) else {
            return;
        };
        v.rounded_asset(x, y, w, h, 6, k.stroke);
        v.rounded_asset(x + 1, y + 1, w - 2, h - 2, 5, k.panel);
        let key = match (size, self.tool) {
            (true, Tool::Fill) => "tolerance",
            (true, _) => "size",
            (false, _) => "opacity",
        };
        icon(v, key, k.dark, x + 12, y + 14, 16);
        let Some((tx, top, bottom)) = self.track(size) else {
            return;
        };
        let f = self.slider_fraction(size);
        let knob = bottom - ((bottom - top) as f32 * f) as i32;
        v.rounded_asset(tx - 2, top, 4, (bottom - top) as u32, 2, k.track);
        v.rounded_asset(tx - 2, knob, 4, (bottom - knob).max(0) as u32, 2, k.accent);
        // WinUI slider thumb: 20 px disc with an accent centre.
        v.rounded_asset(
            tx - 10,
            knob - 10,
            20,
            20,
            10,
            if k.dark {
                rgb(69, 69, 69)
            } else {
                rgb(255, 255, 255)
            },
        );
        v.rounded_asset(
            tx - 10,
            knob - 10,
            20,
            20,
            10,
            if k.dark {
                rgb(69, 69, 69)
            } else {
                rgb(229, 229, 229)
            },
        );
        v.rounded_asset(
            tx - 9,
            knob - 9,
            18,
            18,
            9,
            if k.dark {
                rgb(69, 69, 69)
            } else {
                rgb(255, 255, 255)
            },
        );
        let dot = if matches!(self.drag, Drag::Slider { size: s } if s == size) {
            7
        } else {
            6
        };
        v.rounded_asset(
            tx - dot,
            knob - dot,
            (dot * 2) as u32,
            (dot * 2) as u32,
            dot as u32,
            k.accent,
        );
        // The value while dragging.
        if matches!(self.drag, Drag::Slider { size: s } if s == size) {
            let text = if size {
                let (val, _, _) = self.size_value();
                if self.tool == Tool::Fill {
                    format!("{val}%")
                } else {
                    format!("{val}px")
                }
            } else {
                format!("{}%", self.opacity)
            };
            let tw = v.measure_style(&text, TextStyle::Regular12) + 16;
            v.rounded_asset(x + w as i32 + 6, knob - 14, tw, 28, 4, k.flyout_edge);
            v.rounded_asset(x + w as i32 + 7, knob - 13, tw - 2, 26, 3, k.flyout);
            v.text_clip_centered(
                x + w as i32 + 14,
                (knob - 14, 28),
                tw,
                &text,
                TextStyle::Regular12,
                k.text,
            );
        }
    }
    fn paint_status(&self, v: &mut View<'_>, k: &Ink) {
        let (w, h) = (self.width, self.height as i32);
        let y = h - STATUS_H;
        v.rect(0, y, w, STATUS_H as u32, k.backdrop);
        v.rect(0, y, w, 1, k.stroke);
        let row = (y, STATUS_H as u32);
        icon(v, "cursor", k.dark, 16, y + 15, 16);
        if let Some((cx, cy)) = self.cursor.filter(|(x, y)| self.over_canvas(*x, *y)) {
            let (px, py) = self.to_canvas(cx, cy);
            if px >= 0.0 && py >= 0.0 && (px as usize) < self.doc.w && (py as usize) < self.doc.h {
                v.text_clip_centered(
                    40,
                    row,
                    100,
                    &format!("{}, {}px", px as i32, py as i32),
                    TextStyle::Regular12,
                    k.text,
                );
            }
        }
        icon(v, "object-size", k.dark, 149, y + 15, 16);
        if let Some(sel) = &self.selection {
            let (sw, sh) = sel
                .floating
                .as_ref()
                .map_or((sel.bounds.width(), sel.bounds.height()), |f| (f.w, f.h));
            v.text_clip_centered(
                173,
                row,
                100,
                &format!("{sw} \u{00d7} {sh}px"),
                TextStyle::Regular12,
                k.text,
            );
        }
        icon(v, "canvas-size", k.dark, 281, y + 15, 16);
        let (dw, dh) = self
            .resize_preview
            .map_or((self.doc.w, self.doc.h), |b| (b.width(), b.height()));
        v.text_clip_centered(
            303,
            row,
            140,
            &format!("{dw} \u{00d7} {dh}px"),
            TextStyle::Regular12,
            k.text,
        );
        let wi = w as i32;
        for (r, target) in self.status_buttons() {
            match target {
                id::FIT => {
                    self.button_back(v, k, r, false);
                    glyph(v, "paint-fit", r.x + 8, r.y + 8, 16, k.text);
                }
                id::ZOOM_MENU => {
                    v.rounded_asset(r.x, r.y, r.w, r.h, 4, k.field_edge);
                    v.rounded_asset(
                        r.x + 1,
                        r.y + 1,
                        r.w - 2,
                        r.h - 2,
                        3,
                        if self.hovered(v, r) { k.hover } else { k.field },
                    );
                    let label = zoom_label(self.zoom);
                    v.text_clip_centered(
                        r.x + 12,
                        (r.y, r.h),
                        60,
                        &label,
                        TextStyle::Regular12,
                        k.text,
                    );
                    glyph(
                        v,
                        "paint-chevron-down",
                        r.x + r.w as i32 - 22,
                        r.y + 10,
                        12,
                        k.text,
                    );
                }
                id::ZOOM_OUT => {
                    self.button_back(v, k, r, false);
                    glyph(v, "paint-zoom-out", r.x + 8, r.y + 8, 16, k.text);
                }
                id::ZOOM_IN => {
                    self.button_back(v, k, r, false);
                    glyph(v, "paint-zoom-in", r.x + 8, r.y + 8, 16, k.text);
                }
                _ => {}
            }
        }
        // Zoom slider (logarithmic over the zoom steps).
        if let Some((sx, _, sw, _)) = self.zoom_slider() {
            let cy = h - STATUS_H / 2;
            let i = ZOOMS
                .iter()
                .position(|z| *z >= self.zoom)
                .unwrap_or(ZOOMS.len() - 1) as f32;
            let f = i / (ZOOMS.len() - 1) as f32;
            let kx = sx + (sw as f32 * f) as i32;
            v.rounded_asset(sx, cy - 2, sw, 4, 2, k.track);
            v.rounded_asset(sx, cy - 2, (kx - sx).max(0) as u32, 4, 2, k.accent);
            v.rounded_asset(
                kx - 10,
                cy - 10,
                20,
                20,
                10,
                if k.dark {
                    rgb(69, 69, 69)
                } else {
                    rgb(229, 229, 229)
                },
            );
            v.rounded_asset(
                kx - 9,
                cy - 9,
                18,
                18,
                9,
                if k.dark {
                    rgb(69, 69, 69)
                } else {
                    rgb(255, 255, 255)
                },
            );
            v.rounded_asset(kx - 6, cy - 6, 12, 12, 6, k.accent);
        }
        let _ = wi;
    }
    fn paint_layers(&self, v: &mut View<'_>, k: &Ink) {
        let Some(p) = self.layers_pane() else {
            return;
        };
        v.rect(p.x, p.y, p.w, p.h, k.card);
        v.rect(p.x, p.y, 1, p.h, k.stroke);
        v.text_clip_centered(
            p.x + 16,
            (p.y + 8, 32),
            120,
            "Layers",
            TextStyle::Semibold14,
            k.text,
        );
        let add = Rect::centred(p.x + p.w as i32 - 28, p.y + 24, 32);
        let can_add = self.doc.layers.len() < raster::MAX_LAYERS;
        if can_add && self.hovered(v, add) {
            v.rounded_asset(add.x, add.y, add.w, add.h, 4, k.hover);
        }
        glyph(
            v,
            "paint-add",
            add.x + 8,
            add.y + 8,
            16,
            if can_add { k.text } else { k.disabled },
        );
        for i in 0..self.doc.layers.len() {
            let Some(r) = self.layer_row(i) else {
                continue;
            };
            let active = i == self.doc.active;
            if active {
                v.rounded_asset(r.x, r.y, r.w, r.h, 6, k.accent);
                v.rounded_asset(r.x + 1, r.y + 1, r.w - 2, r.h - 2, 5, k.pressed);
            } else if self.hovered(v, r) {
                v.rounded_asset(r.x, r.y, r.w, r.h, 6, k.hover);
            }
            // Thumbnail.
            let (tw, th) = (72usize, 48usize);
            let thumb = self.thumbnail(i, tw, th);
            let (tx, ty) = (r.x + 8, r.y + (r.h as i32 - th as i32) / 2);
            for yy in (0..th as i32).step_by(6) {
                for xx in (0..tw as i32).step_by(6) {
                    let on = ((xx / 6) + (yy / 6)) % 2 == 0;
                    v.rect(
                        tx + xx,
                        ty + yy,
                        6,
                        6,
                        if on {
                            rgb(255, 255, 255)
                        } else {
                            rgb(214, 214, 214)
                        },
                    );
                }
            }
            v.image_scaled(
                (tx, ty, tw as u32, th as u32),
                (tx as i64 * self.percent as i64) / 100,
                (ty as i64 * self.percent as i64) / 100,
                self.percent,
                100,
                tw,
                th,
                &thumb,
            );
            v.rect(tx, ty, tw as u32, 1, k.stroke);
            v.rect(tx, ty + th as i32 - 1, tw as u32, 1, k.stroke);
            v.rect(tx, ty, 1, th as u32, k.stroke);
            v.rect(tx + tw as i32 - 1, ty, 1, th as u32, k.stroke);
            let name = if i == 0 {
                String::from("Background")
            } else {
                format!("Layer {}", i)
            };
            v.text_clip_centered(
                tx + tw as i32 + 10,
                (r.y, r.h),
                70,
                &name,
                TextStyle::Regular12,
                k.text,
            );
            let eye = Rect::centred(r.x + r.w as i32 - 52, r.y + r.h as i32 / 2, 28);
            icon(
                v,
                if self.doc.layers[i].visible {
                    "eye-on"
                } else {
                    "eye-off"
                },
                k.dark,
                eye.x + 6,
                eye.y + 6,
                16,
            );
            let more = Rect::centred(r.x + r.w as i32 - 20, r.y + r.h as i32 / 2, 28);
            glyph(v, "paint-more", more.x + 6, more.y + 6, 16, k.text);
        }
    }
    fn thumbnail(&self, i: usize, tw: usize, th: usize) -> Vec<u8> {
        let l = &self.doc.layers[i].px;
        let (w, h) = (self.doc.w, self.doc.h);
        let mut out = vec![0u8; tw * th * 4];
        let s = (w as f32 / tw as f32).max(h as f32 / th as f32);
        let (dw, dh) = ((w as f32 / s) as usize, (h as f32 / s) as usize);
        let (ox, oy) = ((tw - dw.min(tw)) / 2, (th - dh.min(th)) / 2);
        for y in 0..dh.min(th) {
            for x in 0..dw.min(tw) {
                let sx = ((x as f32 * s) as usize).min(w - 1);
                let sy = ((y as f32 * s) as usize).min(h - 1);
                out[((y + oy) * tw + x + ox) * 4..][..4]
                    .copy_from_slice(&l[(sy * w + sx) * 4..][..4]);
            }
        }
        out
    }
    fn flyout_frame(&self, v: &mut View<'_>, k: &Ink, f: Rect) {
        v.elevation(f.x, f.y, f.w, f.h, if k.dark { 140 } else { 70 });
        v.rounded_asset(f.x, f.y, f.w, f.h, 8, k.flyout_edge);
        v.rounded_asset(f.x + 1, f.y + 1, f.w - 2, f.h - 2, 7, k.flyout);
    }
    fn paint_flyout(&self, v: &mut View<'_>, k: &Ink, m: Menu) {
        let Some(f) = self.menu_frame(m) else {
            return;
        };
        self.flyout_frame(v, k, f);
        let hv = v.hover;
        let hover = |r: Rect| hv.is_some_and(|(x, y)| r.contains(x, y));
        match m {
            Menu::Brushes => {
                for (i, b) in Brush::ALL.iter().enumerate() {
                    let r = Rect::new(
                        f.x + 4 + (i % 3) as i32 * 48,
                        f.y + 4 + (i / 3) as i32 * 48,
                        48,
                        48,
                    );
                    let selected = *b == self.brush;
                    if selected {
                        v.rounded_asset(r.x + 2, r.y + 2, r.w - 4, r.h - 4, 4, k.accent);
                        v.rounded_asset(r.x + 3, r.y + 3, r.w - 6, r.h - 6, 3, k.pressed);
                    } else if hover(r) {
                        v.rounded_asset(r.x + 2, r.y + 2, r.w - 4, r.h - 4, 4, k.hover);
                    }
                    icon(v, b.icon(), k.dark, r.x + 14, r.y + 14, 20);
                }
            }
            Menu::Stroke => {
                for (i, s) in [1u32, 3, 5, 8].iter().enumerate() {
                    let r = Rect::new(f.x + 4, f.y + 4 + i as i32 * 40, f.w - 8, 40);
                    if self.stroke == *s {
                        v.rounded_asset(r.x, r.y + 2, r.w, r.h - 4, 4, k.pressed);
                        v.rounded_asset(r.x + 1, r.y + 12, 3, 16, 1, k.accent);
                    } else if hover(r) {
                        v.rounded_asset(r.x, r.y + 2, r.w, r.h - 4, 4, k.hover);
                    }
                    v.text_clip_centered(
                        r.x + 14,
                        (r.y, r.h),
                        40,
                        &format!("{s}px"),
                        TextStyle::Regular12,
                        k.text,
                    );
                    v.rect(r.x + 56, r.y + 20 - *s as i32 / 2, r.w - 72, *s, k.text);
                }
            }
            Menu::Solids => {
                for (i, s) in Solid::ALL.iter().enumerate() {
                    let r = Rect::new(f.x + 4, f.y + 4 + i as i32 * 40, f.w - 8, 40);
                    if self.solid == *s && self.tool == Tool::Solid {
                        v.rounded_asset(r.x, r.y + 2, r.w, r.h - 4, 4, k.pressed);
                        v.rounded_asset(r.x + 1, r.y + 12, 3, 16, 1, k.accent);
                    } else if hover(r) {
                        v.rounded_asset(r.x, r.y + 2, r.w, r.h - 4, 4, k.hover);
                    }
                    if let Some((px, tile)) = self.solid_icon(*s)
                        && px == 20 * v.scale.clamp(1, 3)
                    {
                        v.color_pixels(r.x + 12, r.y + 10, 20, tile);
                    }
                    v.text_clip_centered(
                        r.x + 44,
                        (r.y, r.h),
                        r.w - 52,
                        s.label(),
                        TextStyle::Regular14,
                        k.text,
                    );
                }
            }
            Menu::Colors => self.paint_colors(v, k, f),
            Menu::Group(group) => {
                if let Some((_, _, gx, dy)) = self.group_flyout() {
                    let buttons = self.group_buttons(group, gx, dy, 7);
                    self.paint_group(v, k, group, gx, dy, 7, &buttons);
                }
            }
            Menu::Notice => {
                let lines = wrap(v, &self.notice, f.w - 40);
                for (i, l) in lines.iter().take(3).enumerate() {
                    v.text_clip(
                        f.x + 20,
                        f.y + 18 + i as i32 * 20,
                        f.w - 40,
                        l,
                        TextStyle::Regular14,
                        k.text,
                    );
                }
                let ok = Rect::new(f.x + f.w as i32 - 104, f.y + f.h as i32 - 44, 92, 32);
                button(v, k, ok, "OK", true, hover(ok));
            }
            _ => {
                let rows = self.rows(m);
                let rects = self.menu_rows(m);
                let any_check = rows.iter().any(|r| r.checked);
                for (r, rect) in rows.iter().zip(rects) {
                    if r.separator_before {
                        v.rect(f.x + 1, rect.y - 5, f.w - 2, 1, k.divider);
                    }
                    if r.enabled && hover(rect) {
                        v.rounded_asset(rect.x, rect.y + 1, rect.w, rect.h - 2, 4, k.hover);
                    }
                    let color = if r.enabled { k.text } else { k.disabled };
                    let text_x = if any_check { rect.x + 40 } else { rect.x + 12 };
                    if r.checked {
                        glyph(v, "paint-check", rect.x + 12, rect.y + 8, 16, color);
                    }
                    v.text_clip_centered(
                        text_x,
                        (rect.y, rect.h),
                        rect.w - 100,
                        r.label,
                        TextStyle::Regular14,
                        color,
                    );
                    if !r.key.is_empty() {
                        let kw = v.measure_style(r.key, TextStyle::Regular12) as i32;
                        v.text_clip_centered(
                            rect.x + rect.w as i32 - 12 - kw,
                            (rect.y, rect.h),
                            kw as u32 + 2,
                            r.key,
                            TextStyle::Regular12,
                            k.secondary,
                        );
                    }
                }
            }
        }
    }
    fn paint_colors(&self, v: &mut View<'_>, k: &Ink, f: Rect) {
        let Some(s) = self.spectrum_rect() else {
            return;
        };
        // Saturation across, value down, at the current hue.
        let scale = v.scale.clamp(1, 3) as usize;
        let (pw, ph) = (s.w as usize * scale, s.h as usize * scale);
        let mut px = vec![0u8; pw * ph * 4];
        for y in 0..ph {
            for x in 0..pw {
                let c = hsv_rgb(Hsv {
                    h: self.hsv.h,
                    s: x as f32 / pw as f32,
                    v: 1.0 - y as f32 / ph as f32,
                });
                px[(y * pw + x) * 4..][..4].copy_from_slice(&c.bgra());
            }
        }
        v.color_image(s.x, s.y, s.w as usize, s.h as usize, &px);
        let mx = s.x + (self.hsv.s * s.w as f32) as i32;
        let my = s.y + ((1.0 - self.hsv.v) * s.h as f32) as i32;
        circle(v, mx, my, 7.0, rgb(255, 255, 255));
        circle(v, mx, my, 8.0, rgb(0, 0, 0));
        let hr = self.hue_rect().unwrap();
        let mut hp = vec![0u8; hr.w as usize * scale * hr.h as usize * scale * 4];
        let hw = hr.w as usize * scale;
        for y in 0..hr.h as usize * scale {
            for x in 0..hw {
                let c = hsv_rgb(Hsv {
                    h: x as f32 / hw as f32 * 360.0,
                    s: 1.0,
                    v: 1.0,
                });
                hp[(y * hw + x) * 4..][..4].copy_from_slice(&c.bgra());
            }
        }
        v.color_image(hr.x, hr.y, hr.w as usize, hr.h as usize, &hp);
        let hx = hr.x + (self.hsv.h / 360.0 * hr.w as f32) as i32;
        v.rounded_asset(hx - 3, hr.y - 3, 6, hr.h + 6, 3, rgb(255, 255, 255));
        v.rect(hx - 1, hr.y - 1, 2, hr.h + 2, rgb(0, 0, 0));
        // Preview and hex.
        let c = hsv_rgb(self.hsv);
        v.rounded_asset(f.x + 16, f.y + 206, 40, 40, 4, k.field_edge);
        v.rounded_asset(f.x + 17, f.y + 207, 38, 38, 3, c.bgra());
        let hex = format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b);
        v.rounded_asset(f.x + 68, f.y + 210, 110, 32, 4, k.field_edge);
        v.rounded_asset(f.x + 69, f.y + 211, 108, 30, 3, k.field);
        v.text_clip_centered(
            f.x + 80,
            (f.y + 210, 32),
            96,
            &hex,
            TextStyle::Regular14,
            k.text,
        );
        v.text_clip_centered(
            f.x + 188,
            (f.y + 210, 32),
            100,
            &format!("{}, {}, {}", c.r, c.g, c.b),
            TextStyle::Regular12,
            k.secondary,
        );
        let hv = v.hover;
        let hover = |r: Rect| hv.is_some_and(|(x, y)| r.contains(x, y));
        let ok = Rect::new(f.x + f.w as i32 - 200, f.y + f.h as i32 - 44, 92, 32);
        let cancel = Rect::new(f.x + f.w as i32 - 100, f.y + f.h as i32 - 44, 92, 32);
        let (ho, hc) = (hover(ok), hover(cancel));
        button(v, k, ok, "OK", true, ho);
        button(v, k, cancel, "Cancel", false, hc);
    }
    fn paint_dialog(&self, v: &mut View<'_>, k: &Ink) {
        let Some(f) = self.dialog_frame() else {
            return;
        };
        v.rounded_asset(0, 0, self.width, self.height, 0, k.smoke);
        v.elevation(f.x, f.y, f.w, f.h, if k.dark { 160 } else { 90 });
        v.rounded_asset(f.x, f.y, f.w, f.h, 8, k.flyout_edge);
        v.rounded_asset(
            f.x + 1,
            f.y + 1,
            f.w - 2,
            f.h - 2,
            7,
            if k.dark {
                rgb(43, 43, 43)
            } else {
                rgb(255, 255, 255)
            },
        );
        // Footer band.
        v.rect(f.x + 1, f.y + f.h as i32 - 80, f.w - 2, 72, k.footer);
        v.rounded_asset(f.x + 1, f.y + f.h as i32 - 16, f.w - 2, 15, 7, k.footer);
        v.rect(f.x + 1, f.y + f.h as i32 - 81, f.w - 2, 1, k.stroke);
        let items = self.dialog_items();
        let hv = v.hover;
        let hover = |r: Rect| hv.is_some_and(|(x, y)| r.contains(x, y));
        let title = |v: &mut View<'_>, t: &str| {
            v.text_clip(
                f.x + 24,
                f.y + 22,
                f.w - 48,
                t,
                TextStyle::Semibold20,
                k.text,
            )
        };
        let field = |v: &mut View<'_>, r: Rect, text: &str, focused: bool| {
            v.rounded_asset(r.x, r.y, r.w, r.h, 4, k.field_edge);
            v.rounded_asset(r.x + 1, r.y + 1, r.w - 2, r.h - 2, 3, k.field);
            if focused {
                v.rounded_asset(r.x + 1, r.y + r.h as i32 - 2, r.w - 2, 2, 1, k.accent);
            }
            v.text_clip_centered(
                r.x + 10,
                (r.y, r.h),
                r.w - 20,
                text,
                TextStyle::Regular14,
                k.text,
            );
            if focused {
                let tw = v.measure_style(text, TextStyle::Regular14) as i32;
                v.rect(r.x + 11 + tw, r.y + 8, 1, r.h - 16, k.text);
            }
        };
        let radio = |v: &mut View<'_>, r: Rect, label: &str, on: bool| {
            let cy = r.y + r.h as i32 / 2;
            v.rounded_asset(
                r.x,
                cy - 10,
                20,
                20,
                10,
                if on { k.accent } else { k.field_edge },
            );
            v.rounded_asset(
                r.x + 1,
                cy - 9,
                18,
                18,
                9,
                if on { k.accent } else { k.field },
            );
            if on {
                v.rounded_asset(r.x + 6, cy - 4, 8, 8, 4, k.on_accent);
            }
            v.text_clip_centered(
                r.x + 30,
                (r.y, r.h),
                r.w - 30,
                label,
                TextStyle::Regular14,
                k.text,
            );
        };
        let footer_labels: Vec<&str> = match self.dialog.as_ref().unwrap() {
            Dialog::Resize {
                pixels,
                h,
                v: vv,
                lock,
                field: fo,
            } => {
                title(v, "Resize and skew");
                v.text_clip(
                    f.x + 24,
                    f.y + 62,
                    200,
                    "Resize",
                    TextStyle::Semibold14,
                    k.text,
                );
                radio(v, items[0].0, "Percentage", !*pixels);
                radio(v, items[1].0, "Pixels", *pixels);
                v.text_clip_centered(
                    f.x + 24,
                    (f.y + 132, 32),
                    120,
                    "Horizontal",
                    TextStyle::Regular14,
                    k.text,
                );
                v.text_clip_centered(
                    f.x + 24,
                    (f.y + 172, 32),
                    120,
                    "Vertical",
                    TextStyle::Regular14,
                    k.text,
                );
                field(v, items[2].0, h, *fo == 0);
                field(v, items[3].0, vv, *fo == 1);
                let unit = if *pixels { "px" } else { "%" };
                v.text_clip_centered(
                    f.x + 278,
                    (f.y + 132, 32),
                    40,
                    unit,
                    TextStyle::Regular14,
                    k.secondary,
                );
                v.text_clip_centered(
                    f.x + 278,
                    (f.y + 172, 32),
                    40,
                    unit,
                    TextStyle::Regular14,
                    k.secondary,
                );
                checkbox(v, k, items[4].0, "Maintain aspect ratio", *lock);
                vec!["OK", "Cancel"]
            }
            Dialog::Properties { w, h, field: fo } => {
                title(v, "Image properties");
                v.text_clip_centered(
                    f.x + 24,
                    (f.y + 76, 32),
                    90,
                    "Width",
                    TextStyle::Regular14,
                    k.text,
                );
                v.text_clip_centered(
                    f.x + 24,
                    (f.y + 120, 32),
                    90,
                    "Height",
                    TextStyle::Regular14,
                    k.text,
                );
                field(v, items[0].0, w, *fo == 0);
                field(v, items[1].0, h, *fo == 1);
                v.text_clip_centered(
                    f.x + 248,
                    (f.y + 76, 32),
                    40,
                    "px",
                    TextStyle::Regular14,
                    k.secondary,
                );
                v.text_clip_centered(
                    f.x + 248,
                    (f.y + 120, 32),
                    40,
                    "px",
                    TextStyle::Regular14,
                    k.secondary,
                );
                vec!["OK", "Cancel"]
            }
            Dialog::Open {
                files,
                selected,
                first,
            } => {
                title(v, "Open");
                if files.is_empty() {
                    v.text_clip(
                        f.x + 24,
                        f.y + 76,
                        f.w - 48,
                        "No pictures found in Pictures, Desktop, Documents or Downloads.",
                        TextStyle::Regular14,
                        k.secondary,
                    );
                }
                let home = self.home();
                for (r, target) in items.iter().filter(|(_, t)| *t >= id::DIALOG + 10) {
                    let i = first + (*target - id::DIALOG - 10) as usize;
                    let Some(path) = files.get(i) else {
                        continue;
                    };
                    if i == *selected {
                        v.rounded_asset(r.x, r.y, r.w, r.h, 4, k.pressed);
                        v.rounded_asset(r.x, r.y + 8, 3, 16, 1, k.accent);
                    } else if hover(*r) {
                        v.rounded_asset(r.x, r.y, r.w, r.h, 4, k.hover);
                    }
                    let shown = path
                        .strip_prefix(&home)
                        .map_or(path.as_str(), |p| p.trim_start_matches('/'));
                    v.text_clip_centered(
                        r.x + 14,
                        (r.y, r.h),
                        r.w - 28,
                        shown,
                        TextStyle::Regular14,
                        k.text,
                    );
                }
                vec!["Open", "Cancel"]
            }
            Dialog::SaveAs { name } => {
                title(v, "Save as");
                v.text_clip(
                    f.x + 24,
                    f.y + 62,
                    f.w - 48,
                    "Save to Pictures as PNG",
                    TextStyle::Regular12,
                    k.secondary,
                );
                field(v, Rect::new(f.x + 24, f.y + 86, f.w - 48, 32), name, true);
                vec!["Save", "Cancel"]
            }
            Dialog::Unsaved { .. } => {
                title(v, "Paint");
                v.text_clip(
                    f.x + 24,
                    f.y + 64,
                    f.w - 48,
                    &format!("Do you want to save changes to {}?", self.name()),
                    TextStyle::Regular14,
                    k.text,
                );
                vec!["Save", "Don't save", "Cancel"]
            }
            Dialog::Settings => {
                title(v, "Settings");
                v.text_clip(
                    f.x + 24,
                    f.y + 66,
                    f.w - 48,
                    "App theme",
                    TextStyle::Semibold14,
                    k.text,
                );
                for (i, label) in ["Light", "Dark", "Use system setting"].iter().enumerate() {
                    radio(v, items[i].0, label, self.theme as usize == i);
                }
                vec!["Close"]
            }
        };
        let buttons: Vec<Rect> = items
            .iter()
            .rev()
            .take(footer_labels.len())
            .rev()
            .map(|(r, _)| *r)
            .collect();
        for (i, (r, label)) in buttons.iter().zip(footer_labels).enumerate() {
            button(v, k, *r, label, i == 0, hover(*r));
        }
    }
}

fn zoom_label(zoom: u32) -> String {
    if zoom.is_multiple_of(10) {
        format!("{}%", zoom / 10)
    } else {
        format!("{}.{}%", zoom / 10, zoom % 10)
    }
}
pub(crate) fn button(v: &mut View<'_>, k: &Ink, r: Rect, label: &str, primary: bool, hover: bool) {
    if primary {
        v.rounded_asset(r.x, r.y, r.w, r.h, 4, k.accent);
        if hover {
            v.rounded_asset(
                r.x,
                r.y,
                r.w,
                r.h,
                4,
                if k.dark {
                    [30, 30, 30, 30]
                } else {
                    [20, 20, 20, 20]
                },
            );
        }
    } else {
        v.rounded_asset(r.x, r.y, r.w, r.h, 4, k.field_edge);
        v.rounded_asset(
            r.x + 1,
            r.y + 1,
            r.w - 2,
            r.h - 2,
            3,
            if hover { k.hover } else { k.field },
        );
    }
    let tw = v.measure_style(label, TextStyle::Regular14) as i32;
    v.text_clip_centered(
        r.x + (r.w as i32 - tw) / 2,
        (r.y, r.h),
        r.w,
        label,
        TextStyle::Regular14,
        if primary { k.on_accent } else { k.text },
    );
}
fn checkbox(v: &mut View<'_>, k: &Ink, r: Rect, label: &str, on: bool) {
    let cy = r.y + r.h as i32 / 2;
    if on {
        v.rounded_asset(r.x, cy - 10, 20, 20, 4, k.accent);
        glyph(v, "paint-check", r.x + 2, cy - 8, 16, k.on_accent);
    } else {
        v.rounded_asset(r.x, cy - 10, 20, 20, 4, k.field_edge);
        v.rounded_asset(r.x + 1, cy - 9, 18, 18, 3, k.field);
    }
    v.text_clip_centered(
        r.x + 30,
        (r.y, r.h),
        r.w - 30,
        label,
        TextStyle::Regular14,
        k.text,
    );
}
/// A 1 px dashed rectangle (4 on, 4 off) in `c` over white.
fn dashed(v: &mut View<'_>, x: i32, y: i32, w: u32, h: u32, c: [u8; 4]) {
    let (w, h) = (w as i32, h as i32);
    let mut i = 0;
    while i < w {
        let n = 4.min(w - i) as u32;
        let col = if (i / 4) % 2 == 0 {
            c
        } else {
            [255, 255, 255, 255]
        };
        v.rect(x + i, y, n, 1, col);
        v.rect(x + i, y + h - 1, n, 1, col);
        i += 4;
    }
    let mut j = 0;
    while j < h {
        let n = 4.min(h - j) as u32;
        let col = if (j / 4) % 2 == 0 {
            c
        } else {
            [255, 255, 255, 255]
        };
        v.rect(x, y + j, 1, n, col);
        v.rect(x + w - 1, y + j, 1, n, col);
        j += 4;
    }
}
fn line(v: &mut View<'_>, a: (i32, i32), b: (i32, i32), c: [u8; 4]) {
    let n = (b.0 - a.0).abs().max((b.1 - a.1).abs()).max(1);
    for i in 0..=n {
        v.rect(
            a.0 + (b.0 - a.0) * i / n,
            a.1 + (b.1 - a.1) * i / n,
            1,
            1,
            c,
        );
    }
}
fn circle(v: &mut View<'_>, cx: i32, cy: i32, r: f32, c: [u8; 4]) {
    let n = ((r * 6.0) as usize).clamp(16, 400);
    let mut last = None;
    for i in 0..=n {
        let (s, co) = sin_cos_f(core::f32::consts::TAU * i as f32 / n as f32);
        let p = (cx + (co * r) as i32, cy + (s * r) as i32);
        if last != Some(p) {
            v.rect(p.0, p.1, 1, 1, c);
            last = Some(p);
        }
    }
}
/// Greedy word wrap to `width`.
fn wrap(v: &View<'_>, text: &str, width: u32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split(' ') {
        let trial = if cur.is_empty() {
            String::from(word)
        } else {
            format!("{cur} {word}")
        };
        if v.measure_style(&trial, TextStyle::Regular14) > width && !cur.is_empty() {
            lines.push(core::mem::take(&mut cur));
            cur = String::from(word);
        } else {
            cur = trial;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}
trait Degrees {
    fn to_degrees_f(self) -> f32;
}
impl Degrees for f32 {
    fn to_degrees_f(self) -> f32 {
        self * 180.0 / core::f32::consts::PI
    }
}
