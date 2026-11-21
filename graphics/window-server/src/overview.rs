//! Trusted Task View over actual captured app surfaces. No producer memory is
//! borrowed, and desktop IDs retain their generations through pointer gestures.
extern crate alloc;
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use door_graphics_core::{Canvas, Engine, Owner, Rect, Role, WindowId, WindowState, WorkspaceId};
use door_graphics_text::{FontAtlas, TextStyle};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Window(WindowId),
    CloseWindow(WindowId),
    Desktop(WorkspaceId),
    CloseDesktop(WorkspaceId),
    NewDesktop,
    Dismiss,
}
pub struct Preview {
    id: WindowId,
    owner: Owner,
    title: String,
    /// Shell icon reference (`door_service_protocol::shell::icon`).
    icon: u16,
    /// Logical client size (thumbnail aspect).
    source: (u32, u32),
    /// Logical thumbnail box the pixels were rendered for.
    boxed: (u32, u32),
    pixels: Vec<u8>,
    revision: Option<u64>,
}
/// Windows 11 26H2 Alt+Tab metrics, logical px
/// (`reports/artifacts/reference-26h2/shell-alttab.uia.tsv` and its capture, px / 1.5).
pub mod switch {
    /// Item: a 40 px header (16 px app icon, 12 px title) above the
    /// thumbnail; 357 px = 238 logical per item.
    pub const ITEM_HEIGHT: u32 = 238;
    pub const HEADER: u32 = 40;
    pub const THUMB_HEIGHT: u32 = ITEM_HEIGHT - HEADER;
    /// Thumbnail widths follow the window aspect within these bounds.
    pub const MIN_WIDTH: u32 = 132;
    pub const MAX_WIDTH: u32 = 420;
    /// Space between items, and from the plate edge to the items.
    pub const GAP: i32 = 24;
    pub const PAD: i32 = 56;
    /// Minimum space between the plate and the work-area edges.
    pub const MARGIN: i32 = 48;
    pub const RADIUS: u32 = 8;
    /// Selection: an accent ring (4 px) with an inner contrast stroke (2 px)
    /// whose outer edge lies 10 px outside the item, outer radius 14.
    pub const RING_OUTSET: i32 = 10;
    pub const RING: u32 = 4;
    pub const RING_INNER: u32 = 2;
    pub const RING_RADIUS: u32 = 14;
    /// Header layout: icon inset 12 px, title at 36 px, close 40x39 at the
    /// header's right edge (shown on hover or selection).
    pub const ICON_INSET: i32 = 12;
    pub const TITLE_X: i32 = 36;
    pub const CLOSE_W: u32 = 40;
    pub const CLOSE_H: u32 = 39;
}
/// Aggregate preview pixel budget: twelve Task View previews at 3x, or
/// Alt+Tab thumbnails at 2x.
const PREVIEW_BUDGET: usize = 12 * 1024 * 1024;
/// Alt+Tab thumbnail width (logical) for a client of logical size `source`:
/// the window aspect at the fixed thumbnail height, within bounds.
pub fn switch_width(source: (u32, u32)) -> u32 {
    let (w, h) = (u64::from(source.0.max(1)), u64::from(source.1.max(1)));
    ((u64::from(switch::THUMB_HEIGHT) * w / h).min(u64::from(switch::MAX_WIDTH)) as u32)
        .max(switch::MIN_WIDTH)
}
/// Alt+Tab rows for a logical work area `size` and client sizes `sources`:
/// (first item, end, content width) per row, and how many rows fit.
pub fn switch_rows(size: (u32, u32), sources: &[(u32, u32)]) -> (Vec<(usize, usize, i32)>, usize) {
    let work_w = size.0 as i32;
    let work_h = size.1 as i32;
    let max_row = (work_w - 2 * switch::MARGIN - 2 * switch::PAD).max(switch::MIN_WIDTH as i32);
    let mut rows = Vec::new();
    let (mut start, mut width) = (0, 0);
    for (i, source) in sources.iter().enumerate() {
        let w = (switch_width(*source) as i32).min(max_row);
        let next = if i == start {
            w
        } else {
            width + switch::GAP + w
        };
        if i > start && next > max_row {
            rows.push((start, i, width));
            start = i;
            width = w;
        } else {
            width = next;
        }
    }
    if start < sources.len() {
        rows.push((start, sources.len(), width));
    }
    let pitch = switch::ITEM_HEIGHT as i32 + switch::GAP;
    let fit = ((work_h - 2 * switch::MARGIN - 2 * switch::PAD + switch::GAP) / pitch).max(1);
    (rows, fit as usize)
}
/// Alt+Tab plate and item rectangles (logical) for windows with client
/// sizes `sources` in a work area at `origin` of logical `size`, scrolled to
/// row `scroll_row`: rows centred on a plate centred in the work area. The
/// guest scenarios derive their probe points from this.
pub fn switch_layout(
    origin: (i32, i32),
    size: (u32, u32),
    sources: &[(u32, u32)],
    scroll_row: u32,
) -> Option<SwitchLayout> {
    let (rows, fit) = switch_rows(size, sources);
    if rows.is_empty() {
        return None;
    }
    let shown = rows.len().min(fit);
    let first = (scroll_row as usize).min(rows.len() - shown);
    let content = rows.iter().map(|r| r.2).max()?;
    let pitch = switch::ITEM_HEIGHT as i32 + switch::GAP;
    let plate_w = content + 2 * switch::PAD;
    let plate_h = shown as i32 * pitch - switch::GAP + 2 * switch::PAD;
    let plate = Rect::new(
        origin.0 + (size.0 as i32 - plate_w) / 2,
        origin.1 + (size.1 as i32 - plate_h) / 2,
        plate_w as u32,
        plate_h as u32,
    )
    .ok()?;
    let mut items = alloc::vec![None; sources.len()];
    for (row, &(start, end, width)) in rows.iter().enumerate().skip(first).take(shown) {
        let mut x = plate.x() + (plate_w - width) / 2;
        let y = plate.y() + switch::PAD + (row - first) as i32 * pitch;
        for (i, item) in items.iter_mut().enumerate().take(end).skip(start) {
            let w = (switch_width(sources[i]) as i32).min(content);
            *item = Rect::new(x, y, w as u32, switch::ITEM_HEIGHT).ok();
            x += w + switch::GAP;
        }
    }
    Some(SwitchLayout { plate, items })
}
/// Alt+Tab plate and per-item rectangles (logical; None = scrolled away).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwitchLayout {
    pub plate: Rect,
    pub items: Vec<Option<Rect>>,
}
#[derive(Default)]
pub struct Overview {
    previews: Vec<Preview>,
    /// App icon per window owner (catalog apps); others use the generic
    /// Windows-app icon.
    icons: Vec<(Owner, u16)>,
    pub selected: usize,
    pub pressed: Option<Action>,
    pub press_origin: Option<(i32, i32)>,
    hovered: Option<Action>,
    width: u32,
    height: u32,
    origin_x: i32,
    origin_y: i32,
    scroll_row: u32,
    switcher: bool,
}
impl Overview {
    pub fn switcher(engine: &Engine, backwards: bool) -> Self {
        let mut view = Self {
            switcher: true,
            ..Self::default()
        };
        view.refresh(engine);
        // Freeze MRU ordering for the gesture; raising only happens on commit.
        let mut recent: Vec<_> = engine.windows().map(|w| w.id).collect();
        recent.reverse();
        view.previews
            .sort_by_key(|p| recent.iter().position(|id| *id == p.id));
        match engine
            .focused_window()
            .filter(|f| view.previews.iter().any(|p| p.id == *f))
        {
            Some(active) => {
                view.select_window(Some(active));
                view.step(backwards);
            }
            // From the desktop (no active window, e.g. after minimizing the
            // active one) Alt+Tab starts at the most recent window, as on
            // Windows; Alt+Shift+Tab at the least recent.
            None if !view.previews.is_empty() => {
                view.selected = if backwards {
                    view.previews.len() - 1
                } else {
                    0
                };
                view.ensure_visible();
            }
            None => (),
        }
        view
    }
    pub fn is_switcher(&self) -> bool {
        self.switcher
    }
    fn card_bounds(&self) -> (i32, i32) {
        if self.switcher {
            (self.origin_y + 48, self.origin_y + self.height as i32 - 64)
        } else {
            (self.origin_y + 60, self.origin_y + self.height as i32 - 186)
        }
    }

    pub fn refresh(&mut self, engine: &Engine) {
        let area = engine.work_area();
        let scale = engine.scale();
        // Logical edges of the physical work area.
        self.origin_x = scale.logical(area.x());
        self.origin_y = scale.logical(area.y());
        self.width = (scale.logical(area.right()) - self.origin_x) as u32;
        // Retain the established bottom-bar layout while expressing all content
        // relative to the actual work area, including alternate shell edges.
        self.height = (scale.logical(area.bottom()) - self.origin_y) as u32 + 48;
        self.previews.retain(|p| {
            engine.window_info(p.id).is_ok_and(|w| {
                w.workspace == engine.active_workspace() && w.state != WindowState::Closing
            })
        });
        let mut budget = 0usize;
        for info in engine
            .windows()
            .filter(|w| {
                w.role == Role::TopLevel
                    && w.workspace == engine.active_workspace()
                    && w.state != WindowState::Closing
            })
            .take(12)
        {
            if !self.previews.iter().any(|p| p.id == info.id) {
                self.previews.push(Preview {
                    id: info.id,
                    owner: info.owner,
                    title: String::new(),
                    icon: door_service_protocol::shell::icon::WINDOWS_APP,
                    source: (0, 0),
                    boxed: (0, 0),
                    pixels: Vec::new(),
                    revision: None,
                });
            }
            let icon = self
                .icons
                .iter()
                .find(|(owner, _)| *owner == info.owner)
                .map_or(door_service_protocol::shell::icon::WINDOWS_APP, |v| v.1);
            let switcher = self.switcher;
            let p = self.previews.iter_mut().find(|p| p.id == info.id).unwrap();
            p.icon = icon;
            if p.title != info.title {
                p.title = info.title.to_string();
            }
            let scale = engine.scale();
            p.source = (
                scale.logical_len(info.content.width()).max(1),
                scale.logical_len(info.content.height()).max(1),
            );
            let boxed = if switcher {
                (switch_width(p.source), switch::THUMB_HEIGHT)
            } else {
                (240, 144)
            };
            let (width, height) = (scale.len(boxed.0) as usize, scale.len(boxed.1) as usize);
            // Each preview is bounded (2 MiB); records past the aggregate
            // budget keep a header with the app icon rather than allocating
            // another full-size client surface.
            let bytes = width * height * 4;
            if bytes > 2 * 1024 * 1024 || budget + bytes > PREVIEW_BUDGET {
                p.pixels = Vec::new();
                p.revision = None;
            } else {
                budget += bytes;
                let Some(snapshot) = engine.snapshot(info.id) else {
                    continue;
                };
                if p.revision == Some(snapshot.revision)
                    && p.boxed == boxed
                    && p.pixels.len() == width * height * 4
                {
                    continue;
                }
                p.boxed = boxed;
                p.pixels.resize(width * height * 4, 0);
                let desc = snapshot.descriptor;
                let (sw, sh) = (desc.width as usize, desc.height as usize);
                p.pixels.fill(0);
                let dw = width.min(height * sw / sh).max(1);
                let dh = height.min(width * sh / sw).max(1);
                let (ox, oy) = ((width - dw) / 2, (height - dh) / 2);
                for y in 0..dh {
                    for x in 0..dw {
                        let sx = (((x as u64 * 2 + 1) * sw as u64 * 32768 / dw as u64)
                            .saturating_sub(32768))
                        .min(((sw - 1) as u64) << 16);
                        let sy = (((y as u64 * 2 + 1) * sh as u64 * 32768 / dh as u64)
                            .saturating_sub(32768))
                        .min(((sh - 1) as u64) << 16);
                        let (ix, iy, fx, fy) =
                            (sx as usize >> 16, sy as usize >> 16, sx & 65535, sy & 65535);
                        for channel in 0..4 {
                            let value = |dx: usize, dy: usize| {
                                u64::from(snapshot.bytes[dy * desc.stride_bytes + dx * 4 + channel])
                            };
                            let a =
                                value(ix, iy) * (65536 - fx) + value((ix + 1).min(sw - 1), iy) * fx;
                            let b = value(ix, (iy + 1).min(sh - 1)) * (65536 - fx)
                                + value((ix + 1).min(sw - 1), (iy + 1).min(sh - 1)) * fx;
                            p.pixels[((y + oy) * width + x + ox) * 4 + channel] =
                                ((a * (65536 - fy) + b * fy + (1 << 31)) >> 32) as u8;
                        }
                    }
                }
                p.revision = Some(snapshot.revision);
            }
        }
        self.selected = self.selected.min(self.previews.len().saturating_sub(1));
        self.ensure_visible();
    }
    pub fn selected_window(&self) -> Option<WindowId> {
        if self.switcher {
            return self
                .switch_layout()
                .and_then(|l| l.items.get(self.selected).copied().flatten())
                .and_then(|_| self.previews.get(self.selected).map(|p| p.id));
        }
        let r = self.card(self.width, self.selected);
        (r.y() >= self.card_bounds().0 && r.bottom() <= self.card_bounds().1)
            .then(|| self.previews.get(self.selected).map(|p| p.id))
            .flatten()
    }
    pub fn select_window(&mut self, id: Option<WindowId>) {
        if let Some(i) = id.and_then(|id| self.previews.iter().position(|p| p.id == id)) {
            self.selected = i;
            self.ensure_visible();
        }
    }
    /// App icons of the windows' owners (catalog apps); call before
    /// `refresh` so the headers show them.
    pub fn set_icons(&mut self, icons: Vec<(Owner, u16)>) {
        self.icons = icons;
        for p in &mut self.previews {
            if let Some((_, icon)) = self.icons.iter().find(|(o, _)| *o == p.owner) {
                p.icon = *icon;
            }
        }
    }
    fn ensure_visible(&mut self) {
        if self.switcher {
            let (rows, fit) = self.switch_rows();
            let Some(row) = rows
                .iter()
                .position(|r| (r.0..r.1).contains(&self.selected))
            else {
                return;
            };
            let row = row as u32;
            if row < self.scroll_row {
                self.scroll_row = row;
            }
            if row >= self.scroll_row + fit as u32 {
                self.scroll_row = row + 1 - fit as u32;
            }
            return;
        }
        let columns = ((self.width.saturating_sub(48)) / 264).clamp(1, 4);
        let (top, bottom) = self.card_bounds();
        let rows = ((bottom - top).max(200) as u32 / 200).max(1);
        let selected = self.selected as u32 / columns;
        if selected < self.scroll_row {
            self.scroll_row = selected;
        }
        if selected >= self.scroll_row + rows {
            self.scroll_row = selected + 1 - rows;
        }
    }
    /// Logical client sizes of the previews, in item order.
    fn sources(&self) -> Vec<(u32, u32)> {
        self.previews.iter().map(|p| p.source).collect()
    }
    /// Logical work area (origin, size) the switcher is laid out in.
    fn switch_area(&self) -> ((i32, i32), (u32, u32)) {
        (
            (self.origin_x, self.origin_y),
            (self.width, self.height.saturating_sub(48)),
        )
    }
    /// Alt+Tab rows: (first item, end, content width) wrapped to the work
    /// area, and how many rows fit its height.
    fn switch_rows(&self) -> (Vec<(usize, usize, i32)>, usize) {
        let (_, size) = self.switch_area();
        switch_rows(size, &self.sources())
    }
    /// Alt+Tab plate and item rectangles (logical): rows centred on a plate
    /// centred in the work area; items of rows scrolled out of view are None.
    pub fn switch_layout(&self) -> Option<SwitchLayout> {
        let (origin, size) = self.switch_area();
        switch_layout(origin, size, &self.sources(), self.scroll_row)
    }
    /// The close button of an Alt+Tab item (its header's right end).
    pub fn switch_close(item: Rect) -> Rect {
        Rect::new(
            item.right() - switch::CLOSE_W as i32,
            item.y() + 1,
            switch::CLOSE_W,
            switch::CLOSE_H,
        )
        .unwrap_or(item)
    }
    pub fn step(&mut self, backwards: bool) {
        if !self.previews.is_empty() {
            self.selected = (self.selected
                + if backwards {
                    self.previews.len() - 1
                } else {
                    1
                })
                % self.previews.len();
            self.ensure_visible();
        }
    }
    fn card(&self, width: u32, index: usize) -> Rect {
        let columns = ((width.saturating_sub(48)) / 264).clamp(1, 4);
        let col = index as u32 % columns;
        let row = index as u32 / columns;
        let in_row = (self.previews.len() as u32 - row * columns).min(columns);
        let left = (width.saturating_sub(in_row * 264)) / 2 + 8;
        let top = if self.switcher {
            let rows = (self.previews.len() as u32).div_ceil(columns);
            let visible = rows.min(((self.height.saturating_sub(112)) / 200).max(1));
            (self.height.saturating_sub(visible * 200)) as i32 / 2
        } else {
            84
        };
        Rect::new(
            self.origin_x + left as i32 + (col * 264) as i32,
            self.origin_y + top + (row as i32 - self.scroll_row as i32) * 200,
            248,
            184,
        )
        .unwrap()
    }
    pub fn hit(
        &self,
        _width: u32,
        _height: u32,
        desktops: &[WorkspaceId],
        x: i32,
        y: i32,
    ) -> Action {
        if self.switcher {
            let Some(layout) = self.switch_layout() else {
                return Action::Dismiss;
            };
            for (item, p) in layout.items.iter().zip(&self.previews) {
                if let Some(r) = item.filter(|r| r.contains(x, y)) {
                    return if Self::switch_close(r).contains(x, y) {
                        Action::CloseWindow(p.id)
                    } else {
                        Action::Window(p.id)
                    };
                }
            }
            return Action::Dismiss;
        }
        let (width, height) = (self.width, self.height);
        let bottom = self.origin_y + height as i32 - 174;
        if (bottom..bottom + 106).contains(&y) {
            let tile =
                ((width.saturating_sub(48)) / (desktops.len() as u32 + 1)).clamp(40, 132) as i32;
            for (i, id) in desktops.iter().enumerate() {
                let left = self.origin_x + 24 + i as i32 * tile;
                if (left..left + tile - 8).contains(&x) {
                    return if desktops.len() > 1 && x >= left + tile - 30 && y < bottom + 24 {
                        Action::CloseDesktop(*id)
                    } else {
                        Action::Desktop(*id)
                    };
                }
            }
            if (self.origin_x + 24 + desktops.len() as i32 * tile
                ..self.origin_x + 24 + (desktops.len() + 1) as i32 * tile - 8)
                .contains(&x)
                && desktops.len() < 8
            {
                return Action::NewDesktop;
            }
        }
        for (i, p) in self.previews.iter().enumerate() {
            let r = self.card(width, i);
            if r.y() >= self.card_bounds().0
                && r.bottom() <= self.card_bounds().1
                && r.contains(x, y)
            {
                return if x >= r.right() - 30 && y < r.y() + 28 {
                    Action::CloseWindow(p.id)
                } else {
                    Action::Window(p.id)
                };
            }
        }
        Action::Dismiss
    }
    pub fn hover(&mut self, action: Action) -> bool {
        let changed = self.hovered != Some(action);
        self.hovered = Some(action);
        changed
    }
    pub fn draw(
        &self,
        c: &mut Canvas<'_>,
        fonts: &FontAtlas,
        desktops: &[WorkspaceId],
        active: WorkspaceId,
        dark: bool,
        background: u32,
    ) {
        let (width, height) = (self.width, self.height);
        let bottom = self.origin_y + height as i32 - 174;
        // Contrast themes (forced colours): solid Window surfaces, WindowText
        // text and borders, Highlight for the selection; light and dark keep
        // their Windows reference values.
        let hc = crate::paint::contrast().map(crate::paint::Palette::contrast_theme);
        let pick = |themed: [u8; 4], role: fn(&crate::paint::Palette) -> [u8; 4]| {
            hc.as_ref().map_or(themed, role)
        };
        let _ = c.fill_rect(
            Rect::new(self.origin_x, self.origin_y, width, height - 48).unwrap(),
            pick(if dark { [0, 0, 0, 80] } else { [8, 8, 8, 24] }, |t| {
                t.surface
            }),
        );
        let fg = pick(
            if dark {
                [240, 240, 240, 255]
            } else {
                [32, 32, 32, 255]
            },
            |t| t.text,
        );
        if self.switcher {
            self.draw_switcher(c, fonts, dark);
            return;
        }
        label(
            c,
            fonts,
            self.origin_x + 24,
            self.origin_y + 28,
            "Task view",
            fg,
        );
        let _ = c.fill_rect(
            Rect::new(self.origin_x, bottom - 12, width, 126).unwrap(),
            pick(
                if dark {
                    [32, 32, 32, 220]
                } else {
                    [205, 205, 205, 220]
                },
                |t| t.surface,
            ),
        );
        for (i, p) in self.previews.iter().enumerate() {
            let r = self.card(width, i);
            if r.y() < self.card_bounds().0 || r.bottom() > self.card_bounds().1 {
                continue;
            }
            let color = if i == self.selected || self.hovered == Some(Action::Window(p.id)) {
                crate::paint::Palette::new(dark).accent
            } else {
                pick(
                    if dark {
                        [55, 55, 55, 255]
                    } else {
                        [245, 245, 245, 255]
                    },
                    |t| t.tile_border,
                )
            };
            let _ = c.fill_rounded_rect(r, 8, color);
            let _ = c.fill_rounded_rect(
                Rect::new(r.x() + 2, r.y() + 2, r.width() - 4, r.height() - 4).unwrap(),
                6,
                pick(
                    if dark {
                        [55, 55, 55, 255]
                    } else {
                        [245, 245, 245, 255]
                    },
                    |t| t.surface,
                ),
            );
            label_capped(
                c,
                fonts,
                (r.x() + 12, r.y() + 8),
                r.width() - 48,
                &p.title,
                fg,
            );
            close_glyph(c, r.right() - 22, r.y() + 8, fg);
            if !p.pixels.is_empty() {
                let s = c.scale();
                let _ = c.blit_bgra_physical(
                    Rect::new(s.px(r.x() + 4), s.px(r.y() + 34), s.len(240), s.len(144)).unwrap(),
                    &p.pixels,
                );
            }
        }
        let tile = ((width.saturating_sub(48)) / (desktops.len() as u32 + 1)).clamp(40, 132) as i32;
        for (i, id) in desktops.iter().enumerate() {
            let x = self.origin_x + 24 + i as i32 * tile;
            let _ = c.fill_rounded_rect(
                Rect::new(x, bottom, (tile - 8) as u32, 106).unwrap(),
                6,
                pick(
                    if dark {
                        [48, 48, 48, 255]
                    } else {
                        [245, 245, 245, 255]
                    },
                    |t| t.tile,
                ),
            );
            label(
                c,
                fonts,
                x + 8,
                bottom + 6,
                &format!("Desktop {}", i + 1),
                fg,
            );
            let scale = c.scale();
            let (pw, ph, _) = c.physical_dimensions();
            let (ox, oy) = (scale.px(x + 4), scale.px(bottom + 24));
            let clip = Rect::new(ox, oy, scale.len((tile - 16) as u32), scale.len(76)).unwrap();
            c.set_clip(clip);
            // Native preview at the asset density; fractional scales
            // area-filter it to the exact physical size.
            let factor = scale.asset_factor();
            let native = wallpaper_preview(background, factor);
            let (w, h) = (scale.len(116), scale.len(76));
            let resampled;
            let source = if scale.factor().is_some() {
                native
            } else {
                resampled = door_graphics_core::area_resample(
                    native,
                    3,
                    ((116 * factor) as usize, (76 * factor) as usize),
                    (w as usize, h as usize),
                );
                &resampled[..]
            };
            let _ = c.blit_rounded_bgr(Rect::new(ox, oy, w, h).unwrap(), scale.len(4), source);
            c.set_clip(Rect::new(0, 0, pw, ph).unwrap());
            if *id == active {
                let _ = c.fill_rect(
                    Rect::new(x + tile / 2 - 12, bottom + 103, 24, 3).unwrap(),
                    crate::paint::Palette::new(dark).accent,
                );
            }
            if desktops.len() > 1 {
                close_glyph(c, x + tile - 24, bottom + 6, fg);
            }
        }
        if desktops.len() < 8 {
            label(
                c,
                fonts,
                self.origin_x + 24 + desktops.len() as i32 * tile + 8,
                bottom + 38,
                "+ New desktop",
                fg,
            );
        }
    }
}
impl Overview {
    /// Windows 11 26H2 Alt+Tab: a grey acrylic plate (radius 8) holding one
    /// item per window, each a dark header (app icon and title) above its
    /// live thumbnail, with no card plate. The selection is an accent ring
    /// with an inner contrast stroke; Close shows on hover or selection.
    fn draw_switcher(&self, c: &mut Canvas<'_>, fonts: &FontAtlas, dark: bool) {
        let Some(layout) = self.switch_layout() else {
            return;
        };
        let palette = crate::paint::Palette::new(dark);
        let hc = palette.contrast;
        let (plate, header, inner) = switch_colors(&palette);
        let _ = c.fill_rounded_rect(layout.plate, switch::RADIUS, plate);
        if hc {
            ring(c, layout.plate, switch::RADIUS, 1, palette.surface_border);
        }
        let scale = c.scale();
        for (i, (item, p)) in layout.items.iter().zip(&self.previews).enumerate() {
            let Some(r) = *item else {
                continue;
            };
            let selected = i == self.selected;
            let hovered = matches!(
                self.hovered,
                Some(Action::Window(id) | Action::CloseWindow(id)) if id == p.id
            );
            // Header band: the item's rounded top on the header colour.
            let head = Rect::new(r.x(), r.y(), r.width(), switch::HEADER).unwrap();
            if let Some(saved) = c.clip_rect() {
                if let Some(clip) = saved.intersection(crate::paint::physical(head, scale)) {
                    c.set_clip(clip);
                    let _ = c.fill_rounded_rect(r, switch::RADIUS, header);
                }
                c.set_clip(saved);
            }
            crate::shell_view::draw_icon(
                c,
                r.x() + switch::ICON_INSET,
                r.y() + (switch::HEADER as i32 - 16) / 2,
                16,
                p.icon,
            );
            let close = Self::switch_close(r);
            let show_close = selected || hovered;
            let reserved = if show_close {
                switch::CLOSE_W as i32
            } else {
                12
            };
            let title_w = (r.width() as i32 - switch::TITLE_X - reserved).max(1) as u32;
            let title = crate::paint::ellipsize(fonts, &p.title, TextStyle::Regular12, title_w);
            crate::paint::text_row(
                c,
                fonts,
                r.x() + switch::TITLE_X,
                r.y(),
                switch::HEADER,
                &title,
                TextStyle::Regular12,
                palette.text,
                title_w,
            );
            if show_close {
                let hot = self.hovered == Some(Action::CloseWindow(p.id));
                if hot {
                    let plate = crate::paint::inset(close, 4, 4).unwrap_or(close);
                    let _ = c.fill_rounded_rect(plate, 4, crate::paint::rgb(196, 43, 28));
                }
                close_glyph(
                    c,
                    close.x() + (close.width() as i32 - 12) / 2,
                    close.y() + (close.height() as i32 - 12) / 2,
                    if hot {
                        crate::paint::rgb(255, 255, 255)
                    } else {
                        palette.text
                    },
                );
            }
            // Thumbnail: the window's live pixels (aspect-fit, rounded bottom
            // corners) or, without one yet, its app icon on the header colour.
            let thumb = Rect::new(
                r.x(),
                r.y() + switch::HEADER as i32,
                r.width(),
                switch::THUMB_HEIGHT,
            )
            .unwrap();
            let size = (scale.len(p.boxed.0) as usize, scale.len(p.boxed.1) as usize);
            let ready = !p.pixels.is_empty() && p.pixels.len() == size.0 * size.1 * 4;
            if !ready || p.boxed.0 != r.width() {
                rounded_bottom(c, thumb, switch::RADIUS, header, None);
            }
            if ready {
                let area = crate::paint::physical(thumb, scale);
                let x = area.x() + (area.width() as i32 - size.0 as i32) / 2;
                if let Ok(at) = Rect::new(x, area.y(), size.0 as u32, size.1 as u32) {
                    rounded_bottom(c, thumb, switch::RADIUS, header, Some((at, &p.pixels)));
                }
            } else {
                crate::shell_view::draw_icon(
                    c,
                    thumb.x() + (thumb.width() as i32 - 32) / 2,
                    thumb.y() + (thumb.height() as i32 - 32) / 2,
                    32,
                    p.icon,
                );
            }
            if selected && let Some(outer) = switch_ring(r) {
                ring(c, outer, switch::RING_RADIUS, switch::RING, palette.accent);
                if !hc
                    && let Some(stroke) =
                        crate::paint::inset(outer, switch::RING as i32, switch::RING as i32)
                {
                    ring(
                        c,
                        stroke,
                        switch::RING_RADIUS - switch::RING,
                        switch::RING_INNER,
                        inner,
                    );
                }
            }
        }
    }
}
/// Outer bounds of the selection ring around Alt+Tab item `r`.
pub fn switch_ring(r: Rect) -> Option<Rect> {
    let o = switch::RING_OUTSET;
    Rect::new(
        r.x() - o,
        r.y() - o,
        r.width() + 2 * o as u32,
        r.height() + 2 * o as u32,
    )
    .ok()
}
/// Alt+Tab colours: (plate, item header, inner selection stroke). Dark from
/// the reference (plate #3E3E3E over the dimmed scene, header #161616, the
/// WinUI inner focus stroke: black at 70 %); light mirrors them.
pub fn switch_colors(palette: &crate::paint::Palette) -> ([u8; 4], [u8; 4], [u8; 4]) {
    if palette.contrast {
        (palette.surface, palette.surface, palette.surface)
    } else if palette.dark {
        (
            [60, 60, 60, 230],
            crate::paint::rgb(22, 22, 22),
            [0, 0, 0, 179],
        )
    } else {
        (
            [205, 205, 205, 230],
            crate::paint::rgb(249, 249, 249),
            [179, 179, 179, 179],
        )
    }
}
/// Rounded-rectangle sample test in 1/8 px.
fn inside_rounded(rect: Rect, radius: i64, sx: i64, sy: i64) -> bool {
    let (l, t, r, b) = (
        i64::from(rect.x()) * 8,
        i64::from(rect.y()) * 8,
        i64::from(rect.right()) * 8,
        i64::from(rect.bottom()) * 8,
    );
    if sx < l || sx >= r || sy < t || sy >= b {
        return false;
    }
    let rad = radius
        .min(i64::from(rect.width()) / 2)
        .min(i64::from(rect.height()) / 2)
        * 8;
    let cx = sx.clamp(l + rad, r - rad);
    let cy = sy.clamp(t + rad, b - rad);
    (sx - cx).pow(2) + (sy - cy).pow(2) <= rad * rad
}
/// Antialiased rounded ring (logical rect, outer radius, thickness): each
/// physical pixel takes outer-minus-inner contour coverage (4x4 samples).
fn ring(c: &mut Canvas<'_>, r: Rect, radius: u32, thickness: u32, color: [u8; 4]) {
    let scale = c.scale();
    let outer = crate::paint::physical(r, scale);
    let t = scale.len(thickness).max(1) as i32;
    let ro = i64::from(scale.len(radius));
    let Ok(inner) = Rect::new(
        outer.x() + t,
        outer.y() + t,
        outer.width().saturating_sub(2 * t as u32).max(1),
        outer.height().saturating_sub(2 * t as u32).max(1),
    ) else {
        return;
    };
    let ri = (ro - i64::from(t)).max(0);
    let band = (ro as i32 + 1).max(t);
    for y in outer.y()..outer.bottom() {
        let full = y < outer.y() + band || y >= outer.bottom() - band;
        let spans: [(i32, i32); 2] = if full {
            [(outer.x(), outer.right()), (0, 0)]
        } else {
            [
                (outer.x(), outer.x() + t),
                (outer.right() - t, outer.right()),
            ]
        };
        for (x0, x1) in spans {
            for x in x0..x1 {
                let mut n = 0u32;
                for dy in [1, 3, 5, 7] {
                    for dx in [1, 3, 5, 7] {
                        let (sx, sy) = (i64::from(x) * 8 + dx, i64::from(y) * 8 + dy);
                        if inside_rounded(outer, ro, sx, sy) && !inside_rounded(inner, ri, sx, sy) {
                            n += 1;
                        }
                    }
                }
                if n > 0 {
                    let _ = c.pixel(x, y, crate::paint::alpha(color, (n * 255 / 16) as u8));
                }
            }
        }
    }
}
/// Paint `r` (logical) with rounded bottom corners (square top): a solid
/// colour, or a physical premultiplied image placed at `image.0`.
fn rounded_bottom(
    c: &mut Canvas<'_>,
    r: Rect,
    radius: u32,
    fill: [u8; 4],
    image: Option<(Rect, &[u8])>,
) {
    let scale = c.scale();
    let area = crate::paint::physical(r, scale);
    let rad = i64::from(scale.len(radius));
    // The contour is a rounded rect extended upward past the area, so only
    // its bottom corners cut.
    let Ok(contour) = Rect::new(
        area.x(),
        area.y() - rad as i32,
        area.width(),
        area.height() + rad as u32,
    ) else {
        return;
    };
    let Some(clip) = c.clip_rect().and_then(|cl| cl.intersection(area)) else {
        return;
    };
    let corner_top = area.bottom() - rad as i32;
    for y in clip.y()..clip.bottom() {
        for x in clip.x()..clip.right() {
            let color = match image {
                None => fill,
                Some((at, pixels)) => {
                    if !at.contains(x, y) {
                        continue;
                    }
                    let i =
                        ((y - at.y()) as usize * at.width() as usize + (x - at.x()) as usize) * 4;
                    [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
                }
            };
            if color[3] == 0 {
                continue;
            }
            let corner =
                y >= corner_top && (x < area.x() + rad as i32 || x >= area.right() - rad as i32);
            let k = if corner {
                let mut n = 0u32;
                for dy in [1, 3, 5, 7] {
                    for dx in [1, 3, 5, 7] {
                        if inside_rounded(
                            contour,
                            rad,
                            i64::from(x) * 8 + dx,
                            i64::from(y) * 8 + dy,
                        ) {
                            n += 1;
                        }
                    }
                }
                (n * 255 / 16) as u8
            } else {
                255
            };
            if k != 0 {
                let _ = c.pixel(x, y, crate::paint::alpha(color, k));
            }
        }
    }
}
fn label(c: &mut Canvas<'_>, fonts: &FontAtlas, x: i32, y: i32, text: &str, color: [u8; 4]) {
    let (w, _, _) = c.dimensions();
    label_capped(
        c,
        fonts,
        (x, y),
        w.saturating_sub(x.max(0) as u32),
        text,
        color,
    );
}
fn close_glyph(c: &mut Canvas<'_>, x: i32, y: i32, color: [u8; 4]) {
    let mask: &[u8] = if c.scale().asset_factor() == 3 {
        include_bytes!("../../../assets/icons/windows-shell/chrome-close.36.mask")
    } else if c.scale().asset_factor() == 2 {
        include_bytes!("../../../assets/icons/windows-shell/chrome-close.24.mask")
    } else {
        include_bytes!("../../../assets/icons/windows-shell/chrome-close.12.mask")
    };
    crate::tray::glyph(c, x, y, 12, mask, color);
}
fn label_capped(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    (x, y): (i32, i32),
    width: u32,
    text: &str,
    color: [u8; 4],
) {
    // A 20 px row from `y`, cap height centred (shell text rules).
    crate::paint::text_row(c, fonts, x, y, 20, text, TextStyle::Regular12, color, width);
}

/// The native wallpaper preview (116x76 logical) at whole density `scale`.
fn wallpaper_preview(background: u32, scale: u32) -> &'static [u8] {
    macro_rules! image {
        ($name:literal) => {
            if scale == 3 {
                &include_bytes!(concat!(
                    "../../../assets/wallpapers/windows/",
                    $name,
                    "-348x228.bgr"
                ))[..]
            } else if scale == 2 {
                &include_bytes!(concat!(
                    "../../../assets/wallpapers/windows/",
                    $name,
                    "-232x152.bgr"
                ))[..]
            } else {
                &include_bytes!(concat!(
                    "../../../assets/wallpapers/windows/",
                    $name,
                    "-116x76.bgr"
                ))[..]
            }
        };
    }
    match background {
        1 => image!("bloom-dark"),
        2 => image!("glow"),
        3 => image!("sunrise"),
        _ => image!("bloom"),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overview_uses_each_reserved_edge_for_cards_desktop_targets_and_tint() {
        use door_graphics_core::{Limits, Owner, Theme};
        use door_service_protocol::desktop::Controls;
        for bits in 0..16 {
            let layout = crate::taskbar::Layout::with_controls(
                1280,
                720,
                Controls::from_bits(bits | 48).unwrap(),
            );
            let area = layout.work_area();
            let bar = layout.bar();
            let mut engine = Engine::new(1280, 720, Theme::Light, Limits::default()).unwrap();
            engine.set_work_area(area).unwrap();
            let owner = Owner {
                pid: 1,
                session: 1,
                epoch: 1,
            };
            engine
                .create_window(
                    owner,
                    Role::TopLevel,
                    Rect::new(100, 100, 240, 176).unwrap(),
                )
                .unwrap();
            let mut overview = Overview::default();
            overview.refresh(&engine);
            let card = overview.card(overview.width, 0);
            assert_eq!(area.intersection(card), Some(card));
            assert_eq!(
                overview.hit(
                    1280,
                    720,
                    &[engine.active_workspace()],
                    area.x() + 30,
                    area.bottom() - 100
                ),
                Action::Desktop(engine.active_workspace())
            );
            let mut bytes = alloc::vec![0;1280*720*4];
            for p in bytes.chunks_exact_mut(4) {
                p[3] = 255;
            }
            let mut canvas = Canvas::new(&mut bytes, 1280, 720, 5120).unwrap();
            overview.draw(
                &mut canvas,
                &FontAtlas::windows_reference().unwrap(),
                &[engine.active_workspace()],
                engine.active_workspace(),
                false,
                0,
            );
            let at = ((bar.y() + bar.height() as i32 / 2) as usize * 1280
                + (bar.x() + bar.width() as i32 / 2) as usize)
                * 4;
            assert_eq!(&bytes[at..at + 4], &[0, 0, 0, 255]);
        }
    }
    use door_graphics_core::{Limits, Owner, SurfaceDescriptor, Theme};
    #[test]
    fn backdrop_really_composites_in_both_themes_and_switcher_is_mru() {
        let mut e = Engine::new(1280, 720, Theme::Light, Limits::default()).unwrap();
        let owner = Owner {
            pid: 1,
            session: 1,
            epoch: 1,
        };
        let mut ids = Vec::new();
        for _ in 0..3 {
            ids.push(
                e.create_window(owner, Role::TopLevel, Rect::new(20, 20, 240, 176).unwrap())
                    .unwrap(),
            );
        }
        e.activate(ids[0]).unwrap();
        e.activate(ids[2]).unwrap();
        let mut v = Overview::switcher(&e, false);
        assert_eq!(v.selected_window(), Some(ids[0]));
        v.step(false);
        assert_eq!(v.selected_window(), Some(ids[1]));
        let spaces = e.workspace_ids().collect::<Vec<_>>();
        assert_eq!(v.hit(1280, 720, &spaces, 50, 600), Action::Dismiss);
        let fonts = FontAtlas::windows_reference().unwrap();
        for (dark, expected) in [(false, 8), (true, 0)] {
            let mut bytes = alloc::vec![0;1280*720*4];
            for p in bytes.chunks_exact_mut(4) {
                p[3] = 255;
            }
            let mut canvas = Canvas::new(&mut bytes, 1280, 720, 5120).unwrap();
            v.draw(&mut canvas, &fonts, &spaces, e.active_workspace(), dark, 0);
            assert_eq!(
                &bytes[(10 * 1280 + 1000) * 4..(10 * 1280 + 1000) * 4 + 4],
                &[expected, expected, expected, 255]
            );
        }
    }
    #[test]
    fn switcher_from_the_desktop_starts_at_the_most_recent_window() {
        let mut e = Engine::new(1280, 720, Theme::Light, Limits::default()).unwrap();
        let owner = Owner {
            pid: 1,
            session: 1,
            epoch: 1,
        };
        let ids: Vec<_> = (0..3)
            .map(|_| {
                e.create_window(owner, Role::TopLevel, Rect::new(20, 20, 240, 176).unwrap())
                    .unwrap()
            })
            .collect();
        e.activate(ids[2]).unwrap();
        // The active window is minimized: nothing has the focus.
        e.request_state(owner, ids[2], WindowState::Minimized)
            .unwrap();
        assert_eq!(e.focused_window(), None);
        let v = Overview::switcher(&e, false);
        assert_eq!(v.selected_window(), Some(ids[2]));
        let back = Overview::switcher(&e, true);
        assert_eq!(back.selected_window(), Some(ids[0]));
    }
    #[test]
    fn overview_previews_follow_real_commit_revision_reuse_memory_and_keep_aspect() {
        let mut e = Engine::new(1280, 720, Theme::Light, Limits::default()).unwrap();
        let owner = Owner {
            pid: 1,
            session: 1,
            epoch: 1,
        };
        let id = e
            .create_window(owner, Role::TopLevel, Rect::new(50, 50, 240, 176).unwrap())
            .unwrap();
        e.acknowledge_configure(owner, id, 1).unwrap();
        let b = e
            .register_buffer(
                owner,
                SurfaceDescriptor::new(240, 144, 960, 240 * 144 * 4).unwrap(),
            )
            .unwrap();
        let mut v = Overview::default();
        for color in [[10, 20, 30, 255], [50, 60, 70, 255]] {
            e.attach(owner, id, b, 1).unwrap();
            e.commit(owner, id, color.repeat(240 * 144)).unwrap();
            let prior = v.previews.first().map(|p| p.pixels.as_ptr());
            v.refresh(&e);
            assert_eq!(&v.previews[0].pixels[..4], &color);
            if let Some(prior) = prior {
                assert_eq!(prior, v.previews[0].pixels.as_ptr());
            }
            let image = v.previews[0].pixels.clone();
            v.refresh(&e);
            assert_eq!(image, v.previews[0].pixels);
        }
    }
    #[test]
    fn overview_keyboard_scrolls_only_to_drawn_cards_and_hits_generation_ids() {
        let mut e = Engine::new(1280, 720, Theme::Light, Limits::default()).unwrap();
        for i in 1..=12 {
            e.create_window(
                Owner {
                    pid: i,
                    session: 1,
                    epoch: 1,
                },
                Role::TopLevel,
                Rect::new(20, 20, 240, 176).unwrap(),
            )
            .unwrap();
        }
        let mut v = Overview::default();
        v.refresh(&e);
        let spaces = e.workspace_ids().collect::<Vec<_>>();
        for _ in 0..12 {
            let id = v.selected_window().unwrap();
            let r = v.card(1280, v.selected);
            assert_eq!(
                v.hit(1280, 720, &spaces, r.x() + 10, r.y() + 50),
                Action::Window(id)
            );
            v.step(false);
        }
        let second = e.create_workspace().unwrap();
        let all = e.workspace_ids().collect::<Vec<_>>();
        assert_eq!(v.hit(1280, 720, &all, 170, 600), Action::Desktop(second));
    }
}
#[cfg(test)]
mod switcher_tests {
    extern crate std;
    use super::*;
    use door_graphics_core::{Limits, SurfaceDescriptor, Theme};
    /// Windows with client sizes `sizes`, each committed in one colour.
    fn scene(sizes: &[(u32, u32, [u8; 4])]) -> (Engine, Vec<WindowId>) {
        let mut e = Engine::new(1280, 720, Theme::Dark, Limits::default()).unwrap();
        e.set_work_area(crate::taskbar::Layout::new(1280, 720).work_area())
            .unwrap();
        let mut ids = Vec::new();
        for (i, &(w, h, color)) in sizes.iter().enumerate() {
            let owner = Owner {
                pid: i as u64 + 1,
                session: 1,
                epoch: 1,
            };
            let id = e
                .create_window(owner, Role::TopLevel, Rect::new(40, 40, w, h + 32).unwrap())
                .unwrap();
            e.acknowledge_configure(owner, id, 1).unwrap();
            let info = e.window_info(id).unwrap();
            let (cw, ch) = (info.content.width(), info.content.height());
            let b = e
                .register_buffer(
                    owner,
                    SurfaceDescriptor::new(cw, ch, cw as usize * 4, (cw * ch * 4) as usize)
                        .unwrap(),
                )
                .unwrap();
            e.attach(owner, id, b, 1).unwrap();
            e.commit(owner, id, color.repeat((cw * ch) as usize))
                .unwrap();
            // Activated in creation order: the last window is the active one.
            e.activate(id).unwrap();
            ids.push(id);
        }
        (e, ids)
    }
    fn draw(v: &Overview, dark: bool) -> Vec<u8> {
        let mut bytes = alloc::vec![0u8; 1280 * 720 * 4];
        for p in bytes.chunks_exact_mut(4) {
            p.copy_from_slice(&[20, 20, 20, 255]);
        }
        let mut canvas = Canvas::new(&mut bytes, 1280, 720, 5120).unwrap();
        v.draw(
            &mut canvas,
            &FontAtlas::windows_reference().unwrap(),
            &[],
            WorkspaceId(0),
            dark,
            0,
        );
        bytes
    }
    fn at(bytes: &[u8], x: i32, y: i32) -> [u8; 4] {
        let i = (y as usize * 1280 + x as usize) * 4;
        [bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]
    }
    #[test]
    fn alt_tab_items_follow_window_aspect_with_header_ring_and_centred_plate() {
        let (e, ids) = scene(&[
            (600, 300, [200, 0, 0, 255]),
            (400, 400, [0, 200, 0, 255]),
            (300, 500, [0, 0, 200, 255]),
        ]);
        let mut v = Overview::switcher(&e, false);
        v.set_icons(alloc::vec![]);
        // The most recent window (last created) is active: Alt+Tab selects
        // the next one.
        assert_eq!(v.selected_window(), Some(ids[1]));
        let l = v.switch_layout().unwrap();
        let items: Vec<Rect> = l.items.iter().map(|r| r.unwrap()).collect();
        // Thumbnails keep the client aspect at 198 px; items are 238 tall.
        for (r, p) in items.iter().zip(&v.previews) {
            assert_eq!(r.height(), switch::ITEM_HEIGHT);
            assert_eq!(r.width(), switch_width(p.source));
        }
        // 24 px gaps, 56 px plate padding, plate centred on the work area.
        for pair in items.windows(2) {
            assert_eq!(pair[1].x() - pair[0].right(), switch::GAP);
        }
        assert_eq!(items[0].x() - l.plate.x(), switch::PAD);
        assert_eq!(l.plate.right() - items[2].right(), switch::PAD);
        assert_eq!(items[0].y() - l.plate.y(), switch::PAD);
        let work = crate::taskbar::Layout::new(1280, 720).work_area();
        assert!((l.plate.x() - work.x() - (work.right() - l.plate.right())).abs() <= 1);
        // Hit: body selects, the header's right end closes.
        let r = items[1];
        assert_eq!(
            v.hit(1280, 720, &[], r.x() + 20, r.y() + 100),
            Action::Window(ids[1])
        );
        let close = Overview::switch_close(r);
        assert_eq!(
            v.hit(1280, 720, &[], close.x() + 20, close.y() + 20),
            Action::CloseWindow(ids[1])
        );
        assert_eq!(
            v.hit(1280, 720, &[], l.plate.x() + 4, l.plate.y() + 4),
            Action::Dismiss
        );
        let bytes = draw(&v, true);
        let palette = crate::paint::Palette::new(true);
        let (_, header, _) = switch_colors(&palette);
        // Header colour beside the title, live pixels under it.
        assert_eq!(at(&bytes, r.right() - 60, r.y() + 4), header);
        let (sx, sy) = (r.x() + r.width() as i32 / 2, r.y() + 120);
        let thumb = v.previews.iter().find(|p| p.id == ids[1]).unwrap();
        assert_eq!(
            thumb.boxed,
            (switch_width(thumb.source), switch::THUMB_HEIGHT)
        );
        assert_eq!(at(&bytes, sx, sy), [0, 200, 0, 255]);
        // The selection ring is the accent, 10 px outside the item.
        let ring = switch_ring(r).unwrap();
        assert_eq!(at(&bytes, ring.x() + 1, r.y() + 100), palette.accent);
        assert_eq!(
            at(&bytes, r.x() + r.width() as i32 / 2, ring.y() + 1),
            palette.accent
        );
        // Unselected items have no ring.
        let other = switch_ring(items[0]).unwrap();
        assert_ne!(
            at(&bytes, other.x() + 1, items[0].y() + 100),
            palette.accent
        );
    }
    #[test]
    fn alt_tab_wraps_rows_and_scrolls_to_the_selection() {
        // Twelve 2:1 clients (396 px thumbnails) wrap two to a row; 300x150
        // keeps buffers and snapshots within the default 12 MiB quota.
        let sizes: Vec<_> = (0..12).map(|_| (300, 150, [90, 90, 90, 255])).collect();
        let (e, ids) = scene(&sizes);
        let mut v = Overview::switcher(&e, false);
        let l = v.switch_layout().unwrap();
        let shown = l.items.iter().filter(|r| r.is_some()).count();
        assert!((2..12).contains(&shown), "{shown}");
        let work = crate::taskbar::Layout::new(1280, 720).work_area();
        assert_eq!(work.intersection(l.plate), Some(l.plate));
        for _ in 0..12 {
            let id = v.selected_window().expect("selection stays visible");
            let r = v.switch_layout().unwrap().items[v.selected].unwrap();
            assert_eq!(
                v.hit(1280, 720, &[], r.x() + 30, r.y() + 120),
                Action::Window(id)
            );
            assert!(work.contains(r.x(), r.y()) && work.contains(r.right() - 1, r.bottom() - 1));
            v.step(false);
        }
        assert!(ids.len() == 12);
    }
}
