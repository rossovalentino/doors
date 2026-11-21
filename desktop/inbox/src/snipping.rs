//! Snipping Tool, measured from Windows 11 26H2 Snipping Tool 11.2608 at
//! 150%: a 32 px title bar over the toolbar (New; the Snip/Record switch;
//! the snipping mode and delay menus; text actions; markup tools, zoom,
//! save, copy, share and "See more" once a snip is open) and the content
//! card ("Press Win + Shift + S to start a snip", or the snip). New asks the
//! shell for its snip overlay (`window::SNIP_BEGIN`); the snip comes back
//! on the clipboard (SNIP_READY), is saved automatically to Pictures\
//! Screenshots as PNG, and can be marked up with the ballpoint pen and
//! highlighter, erased, cropped, copied and saved.
use crate::paint::raster::{self, Bounds, Brush, Doc, Edit, History, Rgba, Tip};
use crate::paint::view::{Rect, button, glyph, ink};
use alloc::{format, string::String, vec, vec::Vec};
use door_app_ui::{Application, View};
use door_graphics_text::{FontAtlas, TextStyle};
use door_service_client::{Error, Storage, window::WindowClient};
use door_service_protocol::window as w;

pub const TITLE: i32 = 32;
const BAR_Y: i32 = 32;
const BAR_H: i32 = 44;
const CARD_Y: i32 = BAR_Y + BAR_H + 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Markup {
    Pen,
    Highlighter,
    Eraser,
    Crop,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    Mode,
    Delay,
    More,
    PenColor,
    Notice,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dialog {
    SaveAs { name: String },
    Settings,
}
const MODES: [(u32, &str, &str); 4] = [
    (w::snip::RECTANGLE, "Rectangle", "snip-rectangle"),
    (w::snip::WINDOW, "Window", "snip-window"),
    (w::snip::FULL_SCREEN, "Full screen", "snip-full-screen"),
    (w::snip::FREE_FORM, "Free-form", "snip-free-form"),
];
const DELAYS: [(u32, &str, &str); 4] = [
    (0, "No delay", "snip-delay"),
    (3, "Wait 3 seconds", "snip-delay-three"),
    (5, "Wait 5 seconds", "snip-delay-five"),
    (10, "Wait 10 seconds", "snip-delay-ten"),
];
const PEN_COLORS: [Rgba; 8] = [
    Rgba::rgb(0, 0, 0),
    Rgba::rgb(255, 255, 255),
    Rgba::rgb(232, 17, 35),
    Rgba::rgb(255, 185, 0),
    Rgba::rgb(16, 124, 16),
    Rgba::rgb(0, 120, 212),
    Rgba::rgb(135, 100, 184),
    Rgba::rgb(255, 140, 0),
];

mod id {
    pub const NEW: u32 = 1;
    pub const CAMERA: u32 = 2;
    pub const VIDEO: u32 = 3;
    pub const MODE: u32 = 4;
    pub const DELAY: u32 = 5;
    pub const TEXT: u32 = 6;
    pub const MORE: u32 = 7;
    pub const MARKUP: u32 = 10; // pen, highlighter, eraser, crop
    pub const UNDO: u32 = 14;
    pub const REDO: u32 = 15;
    pub const ZOOM: u32 = 16;
    pub const SAVE: u32 = 17;
    pub const COPY: u32 = 18;
    pub const SHARE: u32 = 19;
    pub const ITEM: u32 = 100;
    pub const DIALOG: u32 = 200;
    pub const BACKDROP: u32 = 299;
}

pub struct Snipping {
    pub width: u32,
    pub height: u32,
    percent: u32,
    ready: bool,
    uid: u32,
    pub theme: u8,
    pub mode: u32,
    pub delay: u32,
    /// The snip: layer 0 the capture, layer 1 the markup.
    pub image: Option<Doc>,
    comp: Vec<u8>,
    history: History,
    pub tool: Markup,
    pub pen: Rgba,
    pub highlighter: Rgba,
    edit: Option<Edit>,
    crop_from: Option<(i32, i32)>,
    pub crop: Option<Bounds>,
    pub actual: bool,
    pub menu: Option<Menu>,
    pub dialog: Option<Dialog>,
    pub notice: String,
    pub auto_save: bool,
    pub saved: Option<String>,
    pending_new: bool,
    fetch: bool,
    copy: bool,
    grow: bool,
    window: u64,
}

impl Snipping {
    pub fn new(uid: u32) -> Self {
        Self {
            width: 515,
            height: 237,
            percent: 100,
            ready: false,
            uid,
            theme: 2,
            mode: w::snip::RECTANGLE,
            delay: 0,
            image: None,
            comp: Vec::new(),
            history: History::default(),
            tool: Markup::Pen,
            pen: Rgba::rgb(0, 0, 0),
            highlighter: Rgba::rgb(255, 235, 0),
            edit: None,
            crop_from: None,
            crop: None,
            actual: false,
            menu: None,
            dialog: None,
            notice: String::new(),
            auto_save: true,
            saved: None,
            pending_new: false,
            fetch: false,
            copy: false,
            grow: false,
            window: 0,
        }
    }
    fn home(&self) -> String {
        format!("/users/{}", self.uid)
    }
    /// Show a snip (premultiplied BGRA) as the open document.
    pub fn open_snip(&mut self, width: usize, height: usize, pixels: Vec<u8>) {
        let mut doc = Doc::new(width, height);
        doc.layers[0].px = pixels;
        doc.layers.push(raster::Layer {
            px: vec![0; width * height * 4],
            visible: true,
        });
        doc.active = 1;
        self.image = Some(doc);
        self.history.clear();
        self.crop = None;
        self.actual = false;
        self.recompose();
    }
    fn recompose(&mut self) {
        if let Some(d) = &self.image {
            d.composite(&mut self.comp);
        }
    }
    fn flattened(&self) -> Option<(usize, usize, Vec<u8>)> {
        let d = self.image.as_ref()?;
        let mut px = Vec::new();
        d.composite(&mut px);
        Some((d.w, d.h, px))
    }
    /// The card the content sits in (logical).
    pub fn card(&self) -> Rect {
        Rect::new(
            8,
            CARD_Y,
            self.width.saturating_sub(16),
            (self.height as i32 - CARD_Y - 8).max(1) as u32,
        )
    }
    /// The snip's physical placement: top-left and scale (num/den).
    fn placement(&self) -> Option<(i64, i64, u32, u32)> {
        let d = self.image.as_ref()?;
        let c = self.card();
        let p = self.percent as i64;
        let (cw, ch) = ((c.w as i64 - 32) * p / 100, (c.h as i64 - 32) * p / 100);
        let (num, den) = if self.actual {
            (1, 1)
        } else {
            // Fit, never above 100%.
            let fit_w = cw * 1000 / d.w as i64;
            let fit_h = ch * 1000 / d.h as i64;
            (fit_w.min(fit_h).clamp(50, 1000) as u32, 1000)
        };
        let (dw, dh) = (
            d.w as i64 * num as i64 / den as i64,
            d.h as i64 * num as i64 / den as i64,
        );
        let ox = c.x as i64 * p / 100 + (c.w as i64 * p / 100 - dw) / 2;
        let oy = c.y as i64 * p / 100 + (c.h as i64 * p / 100 - dh).max(32 * p / 100) / 2;
        Some((ox, oy, num, den))
    }
    pub fn to_image(&self, x: i32, y: i32) -> Option<(f32, f32)> {
        let (ox, oy, num, den) = self.placement()?;
        let px = x as f32 * self.percent as f32 / 100.0;
        let py = y as f32 * self.percent as f32 / 100.0;
        Some((
            (px - ox as f32) * den as f32 / num as f32,
            (py - oy as f32) * den as f32 / num as f32,
        ))
    }
    fn to_logical(&self, ix: f32, iy: f32) -> (i32, i32) {
        let Some((ox, oy, num, den)) = self.placement() else {
            return (0, 0);
        };
        let px = ox as f32 + ix * num as f32 / den as f32;
        let py = oy as f32 + iy * num as f32 / den as f32;
        (
            (px * 100.0 / self.percent as f32) as i32,
            (py * 100.0 / self.percent as f32) as i32,
        )
    }

    // ---- layout ----
    fn new_button() -> Rect {
        Rect::new(24, BAR_Y + 6, 76, 32)
    }
    fn left_buttons(&self) -> Vec<(Rect, u32)> {
        vec![
            (Self::new_button(), id::NEW),
            (Rect::new(117, BAR_Y + 6, 37, 32), id::CAMERA),
            (Rect::new(155, BAR_Y + 6, 37, 32), id::VIDEO),
            (Rect::new(206, BAR_Y + 6, 50, 32), id::MODE),
            (Rect::new(262, BAR_Y + 6, 50, 32), id::DELAY),
            (Rect::new(330, BAR_Y + 6, 34, 32), id::TEXT),
            (
                Rect::new(self.width as i32 - 52, BAR_Y + 6, 36, 32),
                id::MORE,
            ),
        ]
    }
    fn snip_buttons(&self) -> Vec<(Rect, u32)> {
        if self.image.is_none() {
            return Vec::new();
        }
        let w = self.width as i32;
        let mut v = Vec::new();
        // Markup in the middle (when there is room), file actions right.
        let right = [id::ZOOM, id::SAVE, id::COPY, id::SHARE];
        for (i, target) in right.iter().enumerate() {
            v.push((
                Rect::new(w - 52 - 36 * (4 - i as i32), BAR_Y + 6, 34, 32),
                *target,
            ));
        }
        let tools = [
            id::MARKUP,
            id::MARKUP + 1,
            id::MARKUP + 2,
            id::MARKUP + 3,
            id::UNDO,
            id::REDO,
        ];
        let span = 6 * 36 + 9;
        let left = 372;
        let room = w - 52 - 36 * 4 - 8 - left;
        if room >= span {
            let mut x = left + (room - span) / 2;
            for (i, target) in tools.iter().enumerate() {
                if i == 4 {
                    x += 9;
                }
                v.push((Rect::new(x, BAR_Y + 6, 34, 32), *target));
                x += 36;
            }
        }
        v
    }
    fn menu_frame(&self, m: Menu) -> Rect {
        match m {
            Menu::Mode => Rect::new(206, BAR_Y + 40, 200, 4 * 36 + 8),
            Menu::Delay => Rect::new(262, BAR_Y + 40, 200, 4 * 36 + 8),
            Menu::More => Rect::new(self.width as i32 - 216, BAR_Y + 40, 200, 2 * 36 + 8),
            Menu::PenColor => {
                let x = self
                    .snip_buttons()
                    .iter()
                    .find(|(_, t)| *t == id::MARKUP)
                    .map_or(372, |(r, _)| r.x);
                Rect::new(x, BAR_Y + 40, 4 * 36 + 8, 2 * 36 + 8)
            }
            Menu::Notice => {
                let wd = 360u32.min(self.width - 24);
                Rect::new((self.width as i32 - wd as i32) / 2, CARD_Y + 12, wd, 120)
            }
        }
    }
    fn menu_items(&self, m: Menu) -> Vec<(Rect, u32)> {
        let f = self.menu_frame(m);
        match m {
            Menu::PenColor => (0..8)
                .map(|i| {
                    (
                        Rect::new(f.x + 4 + (i % 4) * 36, f.y + 4 + (i / 4) * 36, 36, 36),
                        id::ITEM + i as u32,
                    )
                })
                .collect(),
            Menu::Notice => vec![(
                Rect::new(f.x + f.w as i32 - 100, f.y + f.h as i32 - 44, 88, 32),
                id::BACKDROP,
            )],
            _ => {
                let n = match m {
                    Menu::More => 2,
                    _ => 4,
                };
                (0..n)
                    .map(|i| {
                        (
                            Rect::new(f.x + 4, f.y + 4 + i * 36, f.w - 8, 36),
                            id::ITEM + i as u32,
                        )
                    })
                    .collect()
            }
        }
    }
    fn dialog_frame(&self) -> Option<Rect> {
        let (dw, dh) = match self.dialog.as_ref()? {
            Dialog::SaveAs { .. } => (420, 200),
            Dialog::Settings => (440, 300),
        };
        let (dw, dh) = (
            dw.min(self.width.saturating_sub(16)),
            dh.min(self.height.saturating_sub(40)),
        );
        Some(Rect::new(
            (self.width as i32 - dw as i32) / 2,
            (self.height as i32 - dh as i32) / 2 + 8,
            dw,
            dh,
        ))
    }
    fn dialog_items(&self) -> Vec<(Rect, u32)> {
        let Some(f) = self.dialog_frame() else {
            return Vec::new();
        };
        let y = f.y + f.h as i32 - 52;
        match self.dialog.as_ref().unwrap() {
            Dialog::SaveAs { .. } => vec![
                (Rect::new(f.x + f.w as i32 - 228, y, 100, 32), id::DIALOG),
                (
                    Rect::new(f.x + f.w as i32 - 120, y, 100, 32),
                    id::DIALOG + 1,
                ),
            ],
            Dialog::Settings => vec![
                (Rect::new(f.x + 24, f.y + 92, 200, 32), id::DIALOG + 2),
                (Rect::new(f.x + 24, f.y + 124, 200, 32), id::DIALOG + 3),
                (Rect::new(f.x + 24, f.y + 156, 200, 32), id::DIALOG + 4),
                (
                    Rect::new(f.x + f.w as i32 - 70, f.y + 200, 48, 28),
                    id::DIALOG + 5,
                ),
                (
                    Rect::new(f.x + f.w as i32 - 120, y, 100, 32),
                    id::DIALOG + 1,
                ),
            ],
        }
    }
    fn hits(&self) -> Vec<(Rect, u32)> {
        if self.dialog.is_some() {
            return self.dialog_items();
        }
        if let Some(m) = self.menu {
            let mut v = self.menu_items(m);
            v.push((self.menu_frame(m), u32::MAX - 1));
            v.push((Rect::new(0, 0, self.width, self.height), id::BACKDROP));
            return v;
        }
        let mut v = self.left_buttons();
        v.extend(self.snip_buttons());
        v
    }

    // ---- actions ----
    fn activate(&mut self, target: u32, s: &mut Storage) -> Result<bool, Error> {
        let toggle =
            |p: &mut Self, m: Menu| p.menu = if p.menu == Some(m) { None } else { Some(m) };
        match target {
            id::BACKDROP => self.menu = None,
            id::NEW => self.pending_new = true,
            id::CAMERA => {}
            id::VIDEO => {
                self.notice = "Screen recording isn't available on this device yet.".into();
                self.menu = Some(Menu::Notice);
            }
            id::MODE => toggle(self, Menu::Mode),
            id::DELAY => toggle(self, Menu::Delay),
            id::TEXT => {
                self.notice =
                    "Text actions need text recognition, which isn't available on this device yet."
                        .into();
                self.menu = Some(Menu::Notice);
            }
            id::MORE => toggle(self, Menu::More),
            t if (id::MARKUP..id::MARKUP + 4).contains(&t) => {
                let tool = [
                    Markup::Pen,
                    Markup::Highlighter,
                    Markup::Eraser,
                    Markup::Crop,
                ][(t - id::MARKUP) as usize];
                if tool == self.tool && tool == Markup::Pen {
                    self.menu = Some(Menu::PenColor);
                } else if tool == Markup::Crop && self.tool == Markup::Crop {
                    self.apply_crop();
                } else {
                    self.tool = tool;
                    self.crop = None;
                }
            }
            id::UNDO => self.undo(),
            id::REDO => self.redo(),
            id::ZOOM => self.actual = !self.actual,
            id::SAVE => {
                self.dialog = Some(Dialog::SaveAs {
                    name: screenshot_name(),
                });
            }
            id::COPY => self.copy = true,
            id::SHARE => {
                self.notice = "Sharing isn't available on this device yet. Your snip is saved in Pictures\\Screenshots.".into();
                self.menu = Some(Menu::Notice);
            }
            t if (id::ITEM..id::ITEM + 8).contains(&t) => {
                let i = (t - id::ITEM) as usize;
                match self.menu.take() {
                    Some(Menu::Mode) => self.mode = MODES[i].0,
                    Some(Menu::Delay) => self.delay = DELAYS[i].0,
                    Some(Menu::PenColor) => self.pen = PEN_COLORS[i],
                    Some(Menu::More) => {
                        if i == 0 {
                            self.dialog = Some(Dialog::Settings);
                        } else {
                            self.notice = format!(
                                "Snipping Tool for Doors. Snips are saved in Pictures\\Screenshots{}.",
                                if self.auto_save {
                                    ""
                                } else {
                                    " when you save them"
                                }
                            );
                            self.menu = Some(Menu::Notice);
                        }
                    }
                    _ => {}
                }
            }
            t if (id::DIALOG..id::BACKDROP).contains(&t) => {
                self.dialog_action(t - id::DIALOG, s)?
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn dialog_action(&mut self, i: u32, s: &mut Storage) -> Result<(), Error> {
        match (self.dialog.clone(), i) {
            (Some(Dialog::SaveAs { name }), 0) => {
                let mut name = String::from(name.trim());
                if name.is_empty() {
                    return Ok(());
                }
                if !name.to_ascii_lowercase().ends_with(".png") {
                    name.push_str(".png");
                }
                self.dialog = None;
                let path = format!("{}/Pictures/{name}", self.home());
                if let Err(e) = self.save_png(s, &path) {
                    self.notice = format!("Snipping Tool couldn't save {name} ({e:?}).");
                    self.menu = Some(Menu::Notice);
                }
            }
            (_, 1) => self.dialog = None,
            (Some(Dialog::Settings), 2..=4) => self.theme = [0, 1, 2][(i - 2) as usize],
            (Some(Dialog::Settings), 5) => self.auto_save = !self.auto_save,
            _ => {}
        }
        Ok(())
    }
    fn save_png(&mut self, s: &mut Storage, path: &str) -> Result<(), Error> {
        let Some((w, h, px)) = self.flattened() else {
            return Ok(());
        };
        let image = door_workspace::explorer_preview::Image {
            width: w,
            height: h,
            source_width: w,
            source_height: h,
            bgra: px,
        };
        let png = door_workspace::explorer_preview::encode_png(&image)
            .map_err(|_| Error::Service(door_service_protocol::Code::Unsupported))?;
        let dir = path.rsplit_once('/').map_or("", |(d, _)| d);
        crate::ensure_dirs(s, dir);
        let file = s.open(
            path,
            door_service_client::Disposition::CreateAlways,
            door_service_client::READ | door_service_client::WRITE,
            door_service_client::SHARE_ALL,
        )?;
        let result = (|| {
            let info = s.info(&file)?;
            let prepared = door_workspace::document_save::prepare(s, &file, info, &png)?;
            door_workspace::document_save::commit(s, prepared)
        })();
        let closed = s.close(file);
        result?;
        closed?;
        door_rt::diagnostic(format_args!(
            "DOOR_INBOX SNIP saved bytes={} size={w}x{h}\n",
            png.len()
        ));
        self.saved = Some(path.into());
        Ok(())
    }
    fn undo(&mut self) {
        if let Some(d) = self.image.as_mut()
            && self.history.undo(d)
        {
            self.recompose();
        }
    }
    fn redo(&mut self) {
        if let Some(d) = self.image.as_mut()
            && self.history.redo(d)
        {
            self.recompose();
        }
    }
    fn apply_crop(&mut self) {
        let (Some(b), Some(d)) = (self.crop.take(), self.image.as_mut()) else {
            self.tool = Markup::Pen;
            return;
        };
        let Some(b) = b.clamp(d.w, d.h) else {
            return;
        };
        let before = raster::Snapshot::of(d);
        for l in &mut d.layers {
            l.px = raster::crop(&l.px, d.w, b);
        }
        d.w = b.width();
        d.h = b.height();
        self.history.push(raster::Change::Whole {
            before,
            after: raster::Snapshot::of(d),
        });
        self.tool = Markup::Pen;
        self.recompose();
    }
}

fn screenshot_name() -> String {
    match crate::local_now(0) {
        Some((t, _, _)) => format!(
            "Screenshot {:04}-{:02}-{:02} {:02}{:02}{:02}.png",
            t.year, t.month, t.day, t.hour, t.minute, t.second
        ),
        None => String::from("Screenshot.png"),
    }
}

impl Snipping {
    pub fn paint(&self, v: &mut View<'_>) {
        let dark = match self.theme {
            0 => false,
            1 => true,
            _ => v.palette.dark,
        };
        v.palette.dark = dark;
        let k = ink(dark);
        if v.palette.system().is_some() {
            v.rect(0, 0, self.width, self.height, k.mica);
        } else {
            v.material_rect(0, 0, self.width, self.height, k.mica, 230);
        }
        if let Some(px) = crate::icon_frames(crate::App::SnippingTool, v.scale) {
            v.color_pixels(16, 8, 16, px);
        }
        v.text_clip_centered(
            46,
            (0, TITLE as u32),
            300,
            "Snipping Tool",
            TextStyle::Regular12,
            k.text,
        );
        let hover = v.hover;
        let quiet = self.menu.is_none() && self.dialog.is_none();
        let hovered = |r: Rect| quiet && hover.is_some_and(|(x, y)| r.contains(x, y));
        // New.
        let n = Self::new_button();
        v.rounded_asset(n.x, n.y, n.w, n.h, 4, k.stroke);
        v.rounded_asset(
            n.x + 1,
            n.y + 1,
            n.w - 2,
            n.h - 2,
            3,
            if hovered(n) { k.hover } else { k.card },
        );
        glyph(v, "snip-new", n.x + 12, n.y + 8, 16, k.accent);
        v.text_clip_centered(
            n.x + 36,
            (n.y, n.h),
            40,
            "New",
            TextStyle::Regular14,
            k.text,
        );
        // Snip / Record.
        let seg = Rect::new(116, BAR_Y + 5, 78, 34);
        v.rounded_asset(seg.x, seg.y, seg.w, seg.h, 5, k.stroke);
        v.rounded_asset(seg.x + 1, seg.y + 1, seg.w - 2, seg.h - 2, 4, k.card);
        for (r, t) in self.left_buttons() {
            match t {
                id::CAMERA => {
                    v.rounded_asset(r.x + 1, r.y + 1, r.w - 2, r.h - 2, 4, k.pressed);
                    glyph(v, "snip-camera", r.x + 10, r.y + 7, 16, k.text);
                    v.rounded_asset(
                        r.x + r.w as i32 / 2 - 8,
                        r.y + r.h as i32 - 4,
                        16,
                        3,
                        1,
                        k.accent,
                    );
                }
                id::VIDEO => {
                    if hovered(r) {
                        v.rounded_asset(r.x + 1, r.y + 1, r.w - 2, r.h - 2, 4, k.hover);
                    }
                    glyph(v, "snip-video", r.x + 10, r.y + 7, 16, k.secondary);
                }
                id::MODE | id::DELAY => {
                    if hovered(r)
                        || self.menu
                            == Some(if t == id::MODE {
                                Menu::Mode
                            } else {
                                Menu::Delay
                            })
                    {
                        v.rounded_asset(r.x, r.y, r.w, r.h, 4, k.hover);
                    }
                    let name = if t == id::MODE {
                        MODES
                            .iter()
                            .find(|m| m.0 == self.mode)
                            .map_or("snip-rectangle", |m| m.2)
                    } else {
                        DELAYS
                            .iter()
                            .find(|d| d.0 == self.delay)
                            .map_or("snip-delay", |d| d.2)
                    };
                    glyph(v, name, r.x + 8, r.y + 8, 16, k.text);
                    glyph(v, "snip-chevron", r.x + 30, r.y + 10, 12, k.text);
                }
                id::TEXT => {
                    v.rect(r.x - 9, r.y + 6, 1, 20, k.divider);
                    glyph(v, "snip-text", r.x + 9, r.y + 8, 16, k.disabled);
                }
                id::MORE => {
                    if hovered(r) || self.menu == Some(Menu::More) {
                        v.rounded_asset(r.x, r.y, r.w, r.h, 4, k.hover);
                    }
                    glyph(v, "snip-more", r.x + 10, r.y + 8, 16, k.text);
                }
                _ => {}
            }
        }
        for (r, t) in self.snip_buttons() {
            let (name, selected, enabled) = match t {
                id::UNDO => ("snip-undo", false, self.history.can_undo()),
                id::REDO => ("snip-redo", false, self.history.can_redo()),
                id::ZOOM => ("snip-zoom", self.actual, true),
                id::SAVE => ("snip-save", false, true),
                id::COPY => ("snip-copy", false, true),
                id::SHARE => ("snip-share", false, true),
                _ => {
                    let i = (t - id::MARKUP) as usize;
                    let tool = [
                        Markup::Pen,
                        Markup::Highlighter,
                        Markup::Eraser,
                        Markup::Crop,
                    ][i];
                    (
                        ["snip-pen", "snip-highlighter", "snip-eraser", "snip-crop"][i],
                        self.tool == tool,
                        true,
                    )
                }
            };
            if t == id::UNDO {
                v.rect(r.x - 6, r.y + 6, 1, 20, k.divider);
            }
            if selected {
                v.rounded_asset(r.x, r.y, r.w, r.h, 4, k.pressed);
            } else if enabled && hovered(r) {
                v.rounded_asset(r.x, r.y, r.w, r.h, 4, k.hover);
            }
            let color = if !enabled {
                k.disabled
            } else if t == id::MARKUP {
                self.pen.bgra()
            } else if t == id::MARKUP + 1 {
                rgb(240, 200, 0)
            } else {
                k.text
            };
            // The pen and highlighter carry their colour (as Windows does);
            // dark ink on a dark bar keeps an outline colour.
            let color = if t == id::MARKUP && dark && self.pen == Rgba::BLACK {
                k.text
            } else {
                color
            };
            glyph(v, name, r.x + 9, r.y + 8, 16, color);
            if selected {
                v.rounded_asset(
                    r.x + r.w as i32 / 2 - 6,
                    r.y + r.h as i32 - 3,
                    12,
                    2,
                    1,
                    k.accent,
                );
            }
        }
        // The content card.
        let c = self.card();
        v.rounded_asset(c.x, c.y, c.w, c.h, 8, k.stroke);
        v.rounded_asset(
            c.x + 1,
            c.y + 1,
            c.w - 2,
            c.h - 2,
            7,
            if dark {
                rgb(43, 43, 43)
            } else {
                rgb(252, 252, 252)
            },
        );
        match &self.image {
            None => self.paint_hint(v, &k, c),
            Some(d) => {
                if let Some((ox, oy, num, den)) = self.placement() {
                    v.image_scaled(
                        (c.x + 1, c.y + 1, c.w - 2, c.h - 2),
                        ox,
                        oy,
                        num,
                        den,
                        d.w,
                        d.h,
                        &self.comp,
                    );
                }
                if let Some(b) = self.crop {
                    let (x0, y0) = self.to_logical(b.x0 as f32, b.y0 as f32);
                    let (x1, y1) = self.to_logical(b.x1 as f32, b.y1 as f32);
                    let (x0, y0, x1, y1) = (x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1));
                    for (x, y, ww, hh) in [
                        (x0, y0, (x1 - x0) as u32, 2u32),
                        (x0, y1 - 2, (x1 - x0) as u32, 2),
                        (x0, y0, 2, (y1 - y0) as u32),
                        (x1 - 2, y0, 2, (y1 - y0) as u32),
                    ] {
                        v.rect(x, y, ww, hh, rgb(255, 255, 255));
                    }
                }
            }
        }
        if let Some(m) = self.menu {
            self.paint_menu(v, &k, m);
        }
        if self.dialog.is_some() {
            self.paint_dialog(v, &k);
        }
    }
    fn paint_hint(&self, v: &mut View<'_>, k: &crate::paint::view::Ink, c: Rect) {
        let parts: [(&str, TextStyle); 6] = [
            ("Press ", TextStyle::Regular14),
            ("", TextStyle::Regular14),
            (" + Shift", TextStyle::Semibold14),
            (" + ", TextStyle::Regular14),
            ("S", TextStyle::Semibold14),
            (" to start a snip", TextStyle::Regular14),
        ];
        let widths: Vec<i32> = parts
            .iter()
            .map(|(t, s)| {
                if t.is_empty() {
                    16
                } else {
                    v.measure_style(t, *s) as i32
                }
            })
            .collect();
        let total: i32 = widths.iter().sum();
        let mut x = c.x + (c.w as i32 - total) / 2;
        let row = (c.y, c.h);
        for ((t, s), wd) in parts.iter().zip(widths) {
            if t.is_empty() {
                glyph(v, "snip-windows", x, c.y + c.h as i32 / 2 - 8, 16, k.text);
            } else if *t == " + Shift" {
                v.text_clip_centered(x, row, wd as u32 + 2, " + ", TextStyle::Regular14, k.text);
                let plus = v.measure_style(" + ", TextStyle::Regular14) as i32;
                v.text_clip_centered(
                    x + plus,
                    row,
                    wd as u32,
                    "Shift",
                    TextStyle::Semibold14,
                    k.text,
                );
            } else {
                v.text_clip_centered(x, row, wd as u32 + 2, t, *s, k.text);
            }
            x += wd;
        }
    }
    fn paint_menu(&self, v: &mut View<'_>, k: &crate::paint::view::Ink, m: Menu) {
        let f = self.menu_frame(m);
        v.elevation(f.x, f.y, f.w, f.h, if k.dark { 140 } else { 70 });
        v.rounded_asset(f.x, f.y, f.w, f.h, 8, k.flyout_edge);
        v.rounded_asset(f.x + 1, f.y + 1, f.w - 2, f.h - 2, 7, k.flyout);
        let hover = v.hover;
        let hovered = |r: Rect| hover.is_some_and(|(x, y)| r.contains(x, y));
        let items = self.menu_items(m);
        match m {
            Menu::PenColor => {
                for (i, (r, _)) in items.iter().enumerate() {
                    if PEN_COLORS[i] == self.pen {
                        v.rounded_asset(r.x + 4, r.y + 4, r.w - 8, r.h - 8, 14, k.accent);
                    }
                    v.rounded_asset(r.x + 7, r.y + 7, r.w - 14, r.h - 14, 11, rgb(128, 128, 128));
                    v.rounded_asset(
                        r.x + 8,
                        r.y + 8,
                        r.w - 16,
                        r.h - 16,
                        10,
                        PEN_COLORS[i].bgra(),
                    );
                }
            }
            Menu::Notice => {
                let words: Vec<&str> = self.notice.split(' ').collect();
                let mut lines: Vec<String> = Vec::new();
                let mut cur = String::new();
                for wd in words {
                    let trial = if cur.is_empty() {
                        String::from(wd)
                    } else {
                        format!("{cur} {wd}")
                    };
                    if v.measure_style(&trial, TextStyle::Regular14) > f.w - 40 && !cur.is_empty() {
                        lines.push(core::mem::take(&mut cur));
                        cur = String::from(wd);
                    } else {
                        cur = trial;
                    }
                }
                lines.push(cur);
                for (i, l) in lines.iter().take(3).enumerate() {
                    v.text_clip(
                        f.x + 20,
                        f.y + 16 + i as i32 * 20,
                        f.w - 40,
                        l,
                        TextStyle::Regular14,
                        k.text,
                    );
                }
                let (r, _) = items[0];
                button(v, k, r, "OK", true, hovered(r));
            }
            _ => {
                for (i, (r, _)) in items.iter().enumerate() {
                    let (label, icon, checked) = match m {
                        Menu::Mode => (MODES[i].1, Some(MODES[i].2), MODES[i].0 == self.mode),
                        Menu::Delay => (DELAYS[i].1, Some(DELAYS[i].2), DELAYS[i].0 == self.delay),
                        _ => (
                            ["Settings", "About Snipping Tool"][i],
                            Some(["snip-settings", "snip-pointer"][i]),
                            false,
                        ),
                    };
                    if checked {
                        v.rounded_asset(r.x, r.y + 2, r.w, r.h - 4, 4, k.pressed);
                        v.rounded_asset(r.x, r.y + 11, 3, 14, 1, k.accent);
                    } else if hovered(*r) {
                        v.rounded_asset(r.x, r.y + 2, r.w, r.h - 4, 4, k.hover);
                    }
                    if let Some(name) = icon {
                        glyph(v, name, r.x + 12, r.y + 10, 16, k.text);
                    }
                    v.text_clip_centered(
                        r.x + 40,
                        (r.y, r.h),
                        r.w - 48,
                        label,
                        TextStyle::Regular14,
                        k.text,
                    );
                }
            }
        }
    }
    fn paint_dialog(&self, v: &mut View<'_>, k: &crate::paint::view::Ink) {
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
        v.rect(f.x + 1, f.y + f.h as i32 - 76, f.w - 2, 68, k.footer);
        v.rounded_asset(f.x + 1, f.y + f.h as i32 - 16, f.w - 2, 15, 7, k.footer);
        let hover = v.hover;
        let hovered = |r: Rect| hover.is_some_and(|(x, y)| r.contains(x, y));
        let items = self.dialog_items();
        match self.dialog.as_ref().unwrap() {
            Dialog::SaveAs { name } => {
                v.text_clip(
                    f.x + 24,
                    f.y + 22,
                    f.w - 48,
                    "Save as",
                    TextStyle::Semibold20,
                    k.text,
                );
                v.text_clip(
                    f.x + 24,
                    f.y + 58,
                    f.w - 48,
                    "Save to Pictures as PNG",
                    TextStyle::Regular12,
                    k.secondary,
                );
                let r = Rect::new(f.x + 24, f.y + 80, f.w - 48, 32);
                v.rounded_asset(r.x, r.y, r.w, r.h, 4, k.field_edge);
                v.rounded_asset(r.x + 1, r.y + 1, r.w - 2, r.h - 2, 3, k.field);
                v.rounded_asset(r.x + 1, r.y + r.h as i32 - 2, r.w - 2, 2, 1, k.accent);
                v.text_clip_centered(
                    r.x + 10,
                    (r.y, r.h),
                    r.w - 20,
                    name,
                    TextStyle::Regular14,
                    k.text,
                );
                button(v, k, items[0].0, "Save", true, hovered(items[0].0));
                button(v, k, items[1].0, "Cancel", false, hovered(items[1].0));
            }
            Dialog::Settings => {
                v.text_clip(
                    f.x + 24,
                    f.y + 22,
                    f.w - 48,
                    "Settings",
                    TextStyle::Semibold20,
                    k.text,
                );
                v.text_clip(
                    f.x + 24,
                    f.y + 66,
                    f.w - 48,
                    "App theme",
                    TextStyle::Semibold14,
                    k.text,
                );
                for (i, label) in ["Light", "Dark", "Use system setting"].iter().enumerate() {
                    let r = items[i].0;
                    let on = self.theme as usize == i;
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
                }
                v.text_clip_centered(
                    f.x + 24,
                    (f.y + 200, 28),
                    f.w - 120,
                    "Automatically save screenshots",
                    TextStyle::Regular14,
                    k.text,
                );
                let t = items[3].0;
                crate::switch(
                    v,
                    (t.x, t.y + 4, 40, 20),
                    self.auto_save,
                    k.accent,
                    k.secondary,
                    k.on_accent,
                    k.field,
                );
                button(v, k, items[4].0, "Close", true, hovered(items[4].0));
            }
        }
    }
}
const fn rgb(r: u8, g: u8, b: u8) -> [u8; 4] {
    [b, g, r, 255]
}

impl Application for Snipping {
    fn title(&self) -> &str {
        "Snipping Tool"
    }
    fn options(&self) -> door_workspace::runtime::WindowOptions {
        door_workspace::runtime::WindowOptions {
            width: 515,
            height: 237,
            caption_height: TITLE as u32,
            caption_left: 200,
            centered: true,
        }
    }
    fn caption(&self) -> (u32, u32) {
        (200, 1)
    }
    fn display(&mut self, _w: u32, _h: u32, percent: u32) {
        self.percent = percent.clamp(100, 300);
    }
    fn configure(&mut self, width: u32, height: u32) {
        self.width = width.max(360);
        self.height = height.max(160);
        if !self.ready {
            self.ready = true;
            door_rt::diagnostic(format_args!("DOOR_INBOX READY app=snipping\n"));
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
    fn click(&mut self, id: u32, s: &mut Storage) -> Result<bool, Error> {
        self.activate(id, s)
    }
    fn pointer_down(&mut self, x: i32, y: i32, _f: &FontAtlas) -> bool {
        if self.menu.is_some() || self.dialog.is_some() || !self.card().contains(x, y) {
            return false;
        }
        let Some((ix, iy)) = self.to_image(x, y) else {
            return false;
        };
        let Some(d) = self.image.as_mut() else {
            return false;
        };
        if ix < 0.0 || iy < 0.0 || ix >= d.w as f32 || iy >= d.h as f32 {
            return false;
        }
        match self.tool {
            Markup::Crop => {
                self.crop_from = Some((ix as i32, iy as i32));
                self.crop = Some(Bounds::new(
                    ix as i32,
                    iy as i32,
                    ix as i32 + 1,
                    iy as i32 + 1,
                ));
            }
            tool => {
                let (tip, size, color, opacity) = match tool {
                    Markup::Pen => (Tip::Brush(Brush::Brush), 4, self.pen, 255),
                    Markup::Highlighter => (Tip::Brush(Brush::Brush), 18, self.highlighter, 110),
                    _ => (Tip::Eraser, 16, Rgba::BLACK, 255),
                };
                let mut e = Edit::begin(d, tip, size, color, opacity, None);
                e.start(d, ix, iy);
                self.edit = Some(e);
                self.recompose();
            }
        }
        true
    }
    fn pointer_motion(&mut self, x: i32, y: i32, _f: &FontAtlas) -> bool {
        let Some((ix, iy)) = self.to_image(x, y) else {
            return false;
        };
        if let Some((x0, y0)) = self.crop_from {
            let (x1, y1) = (ix as i32, iy as i32);
            self.crop = Some(Bounds::new(x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)));
            return true;
        }
        if let (Some(e), Some(d)) = (self.edit.as_mut(), self.image.as_mut()) {
            e.to(d, ix, iy);
            self.recompose();
            return true;
        }
        false
    }
    fn pointer_up(&mut self) {
        self.crop_from = None;
        if let (Some(e), Some(d)) = (self.edit.take(), self.image.as_ref())
            && let Some(b) = e.dirty
        {
            self.history.patch(d, &e.base, b);
        }
    }
    fn key(&mut self, key: u32, s: &mut Storage) -> Result<bool, Error> {
        let scan = key & 511;
        let m = key >> 9;
        let control = m & door_abi::graphics::CONTROL != 0;
        let text = (key >> 16) as u8;
        if let Some(Dialog::SaveAs { name }) = self.dialog.as_mut() {
            match scan {
                1 => self.dialog = None,
                14 => {
                    name.pop();
                }
                28 | 0x11c => return self.activate(id::DIALOG, s),
                _ if (32..127).contains(&text)
                    && !control
                    && !matches!(
                        text,
                        b'/' | b'\\' | b':' | b'*' | b'?' | b'"' | b'<' | b'>' | b'|'
                    ) =>
                {
                    name.push(text as char)
                }
                _ => return Ok(false),
            }
            return Ok(true);
        }
        match (scan, control) {
            (1, _) if self.menu.is_some() || self.dialog.is_some() => {
                self.menu = None;
                self.dialog = None;
            }
            (1, _) if self.crop.is_some() => self.crop = None,
            (28 | 0x11c, _) if self.tool == Markup::Crop => self.apply_crop(),
            (49, true) => self.pending_new = true,
            (44, true) => self.undo(),
            (21, true) => self.redo(),
            (46, true) if self.image.is_some() => self.copy = true,
            (31, true) if self.image.is_some() => {
                self.dialog = Some(Dialog::SaveAs {
                    name: screenshot_name(),
                })
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn event(&mut self, e: w::Event) -> Result<bool, Error> {
        if e.kind == w::SNIP_READY {
            self.window = e.window;
            self.fetch = true;
            return Ok(true);
        }
        Ok(false)
    }
    fn update(
        &mut self,
        s: &mut Storage,
        c: &mut WindowClient,
        window: u64,
    ) -> Result<bool, Error> {
        self.window = window;
        let mut changed = false;
        if core::mem::take(&mut self.pending_new) {
            self.menu = None;
            if let Err(e) = c.snip_begin(self.mode, self.delay) {
                self.notice = format!("A snip couldn't start ({e:?}).");
                self.menu = Some(Menu::Notice);
            }
            changed = true;
        }
        if core::mem::take(&mut self.fetch) {
            changed = true;
            match read_clipboard_image(c) {
                Some((w, h, px)) => {
                    self.open_snip(w, h, px);
                    door_rt::diagnostic(format_args!("DOOR_INBOX SNIP opened size={w}x{h}\n"));
                    if self.auto_save {
                        let path =
                            format!("{}/Pictures/Screenshots/{}", self.home(), screenshot_name());
                        if let Err(e) = self.save_png(s, &path) {
                            door_rt::diagnostic(format_args!(
                                "DOOR_INBOX SNIP autosave_failed {e:?}\n"
                            ));
                        }
                    }
                    self.grow = true;
                }
                None => {
                    self.notice = "The snip couldn't be read from the clipboard.".into();
                    self.menu = Some(Menu::Notice);
                }
            }
        }
        if core::mem::take(&mut self.grow)
            && let Some(d) = &self.image
        {
            // Grow the window to fit the snip (as Snipping Tool opens it).
            let p = self.percent;
            let want_w = (d.w as u32 * 100 / p + 48).clamp(640, 1180);
            let want_h = (d.h as u32 * 100 / p + CARD_Y as u32 + 48).clamp(420, 640);
            if want_w > self.width || want_h > self.height {
                let pw = want_w * p / 100;
                let ph = want_h * p / 100;
                let _ = c.configure(
                    window,
                    w::Rect {
                        x: 40,
                        y: 20,
                        width: pw,
                        height: ph,
                    },
                );
            }
        }
        if core::mem::take(&mut self.copy)
            && let Some((w, h, px)) = self.flattened()
        {
            let mut bytes = Vec::with_capacity(8 + px.len());
            bytes.extend_from_slice(&(w as u32).to_le_bytes());
            bytes.extend_from_slice(&(h as u32).to_le_bytes());
            bytes.extend_from_slice(&px);
            let result = match door_workspace::section_from(&bytes) {
                Ok(section) => {
                    let r = c.clipboard_set_with(
                        window,
                        &[],
                        &[(
                            door_service_protocol::clipboard::IMAGE,
                            section,
                            bytes.len() as u32,
                        )],
                    );
                    let _ = door_sys::close(section);
                    r
                }
                Err(_) => {
                    c.clipboard_set(window, &[(door_service_protocol::clipboard::IMAGE, &bytes)])
                }
            };
            door_rt::diagnostic(format_args!(
                "DOOR_INBOX SNIP copied ok={}\n",
                u8::from(result.is_ok())
            ));
            changed = true;
        }
        Ok(changed)
    }
    fn menu_open(&self) -> bool {
        self.menu.is_some()
    }
    fn dismiss_menu(&mut self) {
        self.menu = None;
    }
    fn modal(&self) -> bool {
        self.dialog.is_some()
    }
    fn capabilities(&self) -> u32 {
        door_service_protocol::clipboard::CAP_CLIPBOARD
    }
}

/// The clipboard's IMAGE format as (width, height, premultiplied BGRA).
pub(crate) fn read_clipboard_image(c: &mut WindowClient) -> Option<(usize, usize, Vec<u8>)> {
    let info = c.clipboard_info().ok()?;
    let size = info.size(door_service_protocol::clipboard::IMAGE) as usize;
    if size < 8 {
        return None;
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(size).ok()?;
    bytes.resize(size, 0);
    c.clipboard_read_into(&info, door_service_protocol::clipboard::IMAGE, &mut bytes)
        .ok()?;
    let w = u32::from_le_bytes(bytes[..4].try_into().ok()?) as usize;
    let h = u32::from_le_bytes(bytes[4..8].try_into().ok()?) as usize;
    (w > 0 && h > 0 && bytes.len() == 8 + w * h * 4).then(|| (w, h, bytes.split_off(8)))
}
