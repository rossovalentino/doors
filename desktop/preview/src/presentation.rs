//! Reference composition: integrated tabs, command strip, canvas and information pane.
use super::*;
use door_app_ui::fluent::{self as f, Glyph as G, Rect as R, RowState};
use door_workspace::nav_pane::{Pane, Row};
pub(super) const NAV: [(&str, G); 17] = [
    ("Home", G::Home),
    ("Recents", G::History),
    ("Shared", G::People),
    ("Images", G::Image),
    ("Videos", G::Video),
    ("Audio", G::Audio),
    ("Documents", G::Document),
    ("3D", G::Model),
    ("Archives", G::Archive),
    ("Other", G::More),
    ("Edit", G::Edit),
    ("Convert", G::Convert),
    ("Batch", G::Batch),
    ("Screenshot", G::Camera),
    ("Record", G::Record),
    ("OCR", G::Scan),
    ("Settings", G::Settings),
];
/// Collapsible navigation groups: (header label, first and last NAV index).
pub(super) const GROUPS: [(&str, usize, usize); 2] = [("Library", 3, 9), ("Tools", 10, 15)];
const SETTINGS: usize = 16;
/// Header ids are 424 + group; 426 is the pane's thumb drag band.
const HEADER_ID: u32 = 424;
const THUMB_ID: u32 = 426;
/// Accessibility > Text size: a band that holds text grows by the growth of
/// its style's line box (0 at 100%); icons and chrome keep their size. Paint,
/// `actions` (hit testing) and semantics read the same rectangles.
pub(super) fn grow(style: TextStyle) -> i32 {
    door_workspace::palette::text_grow_i(0, style)
}
/// A logical text-holding width at the applied text size.
fn text_w(width: u32) -> u32 {
    door_workspace::palette::text_len(width)
}
/// Toolbar buttons (36 px at 100%): the band grows with its zoom value.
pub(super) fn tool_h() -> u32 {
    (36 + grow(TextStyle::Regular14)) as u32
}
/// Zoom value box (66 px at 100%: 42 px of text and its chevron).
fn zoom_w() -> u32 {
    66 + text_w(42) - 42
}
/// Top of the canvas, the sections and the information pane (128 at 100%).
pub(super) fn content_top() -> i32 {
    128 + grow(TextStyle::Regular14)
}
/// Information pane geometry, shared by its painter and `actions`.
pub(super) struct Info {
    pub(super) name: R,
    pub(super) kind: R,
    pub(super) size: R,
    pub(super) divider: i32,
    pub(super) heading: R,
    pub(super) more: R,
    pub(super) rows_top: i32,
    pub(super) pitch: i32,
    pub(super) row_h: u32,
    pub(super) name_w: u32,
    pub(super) count: usize,
    pub(super) quick: Option<Quick>,
    pub(super) export: R,
}
/// Quick actions and Convert, shown when the pane is tall enough.
pub(super) struct Quick {
    pub(super) dividers: [i32; 2],
    pub(super) heading: R,
    pub(super) actions: [R; 6],
    pub(super) convert_heading: R,
    pub(super) format: R,
    pub(super) convert: R,
}
impl Preview {
    /// Navigation pane width: labels follow 55 px of icon and padding in a
    /// 196 px row, so the open pane widens with the text size.
    pub(super) fn rail(&self) -> u32 {
        if self.width < 760 {
            64
        } else {
            220 + text_w(141) - 141
        }
    }
    fn group_open(&self, group: usize) -> bool {
        if group == 0 {
            self.library_open
        } else {
            self.tools_open
        }
    }
    /// Destinations above the pinned Settings row. Compact (icon-only) panes
    /// cannot show disclosure controls, so every destination stays listed.
    pub(super) fn nav_rows(&self) -> Vec<Row<usize>> {
        let compact = self.rail() <= 64;
        let mut rows: Vec<Row<usize>> = (0..3).map(Row::Item).collect();
        for (group, (_, first, last)) in GROUPS.iter().enumerate() {
            rows.push(Row::Header(group as u8));
            if compact || self.group_open(group) {
                rows.extend((*first..=*last).map(Row::Item));
            }
        }
        rows
    }
    fn settings_rect(&self) -> R {
        let h = door_workspace::nav_pane::row_height();
        R::new(
            12,
            self.height as i32 - 24 - h as i32,
            self.rail().saturating_sub(24),
            h,
        )
    }
    pub(super) fn nav_pane(&self) -> Pane {
        Pane {
            x: 0,
            width: self.rail(),
            top: 64,
            bottom: (self.settings_rect().y - 12).max(64),
            scroll: self.nav_scroll,
            compact: self.rail() <= 64,
        }
    }
    pub(super) fn clamp_nav(&mut self) {
        self.nav_scroll = self.nav_pane().clamped(&self.nav_rows()).scroll;
    }
    /// Destination clicks change only the route and selection; pane expansion
    /// and scroll stay where the user left them.
    pub(super) fn select_section(&mut self, section: u32) {
        self.section = section;
        let rows = self.nav_rows();
        if let Some(index) = rows.iter().position(|r| *r == Row::Item(section as usize)) {
            self.nav_scroll = self.nav_pane().reveal(&rows, index);
        }
    }
    /// A disclosure toggles only its own children and never navigates.
    pub(super) fn toggle_group(&mut self, library: bool) {
        let group = usize::from(!library);
        let open = !self.group_open(group);
        if library {
            self.library_open = open;
        } else {
            self.tools_open = open;
        }
        let rows = self.nav_rows();
        self.clamp_nav();
        if open {
            // Show the expanded children while keeping their header in view.
            let header = rows.iter().position(|r| *r == Row::Header(group as u8));
            let last = rows.iter().rposition(|r| *r == Row::Item(GROUPS[group].2));
            for index in [last, header].into_iter().flatten() {
                self.nav_scroll = self.nav_pane().reveal(&rows, index);
            }
        }
        door_rt::diagnostic(format_args!(
            "DOOR_PREVIEW NAV group={group} open={}\n",
            u8::from(open)
        ));
    }
    pub(super) fn grab_nav(&mut self, y: i32) -> bool {
        let rows = self.nav_rows();
        self.nav_grab = self.nav_pane().grab(&rows, y);
        // Repaint even without movement: the grabbed thumb widens.
        self.drag_nav(y);
        self.nav_grab.is_some()
    }
    pub(super) fn drag_nav(&mut self, y: i32) -> bool {
        let Some(grab) = self.nav_grab else {
            return false;
        };
        let next = self.nav_pane().drag(&self.nav_rows(), y, grab);
        let changed = next != self.nav_scroll;
        self.nav_scroll = next;
        changed
    }
    fn body(&self) -> R {
        let (x, y, w, h) = self.dimensions();
        R::new(
            x,
            y,
            w,
            if self.section == 0 || self.section == 10 {
                h
            } else {
                self.height.saturating_sub(y as u32 + 56)
            },
        )
    }
    pub(super) fn panel(&self) -> Option<R> {
        (self.width >= 1100 && self.info_open).then(|| {
            R::new(
                self.width as i32 - 388,
                content_top(),
                388,
                self.height.saturating_sub(content_top() as u32),
            )
        })
    }
    /// The information pane's rows and actions; at 100% the reference
    /// positions (header at 152, rows from 291, quick actions from 534 and
    /// Convert at 788, shown from an 850 px tall window).
    pub(super) fn info_layout(&self, p: R) -> Info {
        let (g14, g16) = (grow(TextStyle::Regular14), grow(TextStyle::Semibold16));
        let column_x = p.x + 120;
        let column_w = p.w.saturating_sub(140);
        let name = R::new(column_x, p.y + 24, column_w, (30 + g16) as u32);
        let kind = R::new(column_x, p.y + 58 + g16, column_w, (22 + g14) as u32);
        let size = R::new(
            column_x,
            kind.y + kind.h as i32 + 2,
            column_w,
            (22 + g14) as u32,
        );
        let header = g16 + 2 * g14;
        let divider = p.y + 114 + header;
        let more_w = text_w(88);
        let more = R::new(
            p.x + p.w as i32 - 20 - more_w as i32,
            divider + 8,
            more_w,
            (32 + g14) as u32,
        );
        let heading_w = text_w(200).min((more.x - 8 - (p.x + 20)).max(0) as u32);
        let heading = R::new(p.x + 20, divider + 8, heading_w, (34 + g16) as u32);
        let rows_top = divider + 49 + g16;
        let pitch = 34 + g14;
        let row_h = (28 + g14) as u32;
        let name_w = text_w(148).min(p.w.saturating_sub(40) / 2);
        let export_h = (38 + g14) as u32;
        let export = R::new(
            p.x + 16,
            self.height as i32 - 42 - export_h as i32,
            328,
            export_h,
        );
        // Quick actions follow four rows of information.
        let d1 = rows_top + 4 * pitch + 39;
        let top = d1 + 68 + g16;
        let action = |i: usize| {
            R::new(
                p.x + 19 + (i % 3) as i32 * 112,
                top + (i / 3) as i32 * (95 + g14),
                112,
                (86 + g14) as u32,
            )
        };
        let d2 = top + 194 + 2 * g14;
        let button_y = d2 + 60 + g16;
        let quick = Quick {
            dividers: [d1, d2],
            heading: R::new(p.x + 20, d1 + 24, p.w.saturating_sub(40), (34 + g16) as u32),
            actions: core::array::from_fn(action),
            convert_heading: R::new(p.x + 20, d2 + 17, p.w.saturating_sub(40), (32 + g16) as u32),
            format: R::new(p.x + 20, button_y, 192, export_h),
            convert: R::new(p.x + 232, button_y, 136, export_h),
        };
        let quick = (button_y + export_h as i32 + 24 <= self.height as i32).then_some(quick);
        // Larger text keeps only the rows that fit above the export button
        // when there are no quick actions (100% keeps the reference count).
        let count = if quick.is_some() {
            4
        } else if g14 == 0 {
            self.info_threshold()
        } else {
            let room = ((export.y - 8 - rows_top) / pitch).max(1) as usize;
            self.info_threshold().min(room)
        };
        Info {
            name,
            kind,
            size,
            divider,
            heading,
            more,
            rows_top,
            pitch,
            row_h,
            name_w,
            count,
            quick,
            export,
        }
    }
    #[cfg(test)]
    fn quick_action_rect(&self, index: usize) -> R {
        let p = self.panel().expect("information pane");
        self.info_layout(p).quick.expect("quick actions").actions[index]
    }
    /// Rows per page of the shown information pane.
    pub(super) fn panel_layout_count(&self) -> Option<usize> {
        self.panel().map(|p| self.info_layout(p).count)
    }
    /// Library rows: (first row top, pitch, height) for a section body.
    fn library_rows(&self, r: R) -> (i32, i32, u32) {
        let g = grow(TextStyle::Regular16) + grow(TextStyle::Regular12);
        (
            r.y + 80 + grow(TextStyle::Semibold28),
            64 + g,
            (56 + g) as u32,
        )
    }
    /// Library rows that fit in the section body.
    fn library_capacity(&self, r: R) -> usize {
        let (top, pitch, _) = self.library_rows(r);
        ((r.y + r.h as i32 - 20 - top).max(0) / pitch) as usize
    }
    fn library_row(&self, r: R, i: usize) -> R {
        let (top, pitch, h) = self.library_rows(r);
        R::new(r.x + 20, top + i as i32 * pitch, r.w.saturating_sub(40), h)
    }
    /// Settings section rows and their value buttons.
    fn settings_row(&self, r: R, i: usize) -> R {
        let g = grow(TextStyle::Regular14);
        R::new(
            r.x + 20,
            r.y + 70 + grow(TextStyle::Semibold28) + i as i32 * (76 + g),
            r.w.saturating_sub(40),
            (64 + g) as u32,
        )
    }
    fn settings_button(row: R) -> R {
        let (w, h) = (text_w(104), tool_h());
        R::new(
            row.x + row.w as i32 - 12 - w as i32,
            row.y + (row.h as i32 - h as i32) / 2,
            w,
            h,
        )
    }
    /// The status-bar "View export notice" button.
    fn notice_link(&self) -> R {
        let (w, h) = (
            224 + text_w(160) - 160,
            (32 + grow(TextStyle::Regular14)) as u32,
        );
        R::new(
            self.width as i32 - 16 - w as i32,
            self.height as i32 - 4 - h as i32,
            w,
            h,
        )
    }
    pub(super) fn plus(&self) -> Option<R> {
        self.strip()
            .plus()
            .map(|r| R::new(r.x, r.y, r.width, r.height))
    }
    fn toolbar(&self) -> Vec<(u32, R, G, bool)> {
        if self.height < 240 && !matches!(self.section, 0 | 10) {
            return Vec::new();
        }
        let (left, _, _, _) = self.dimensions();
        let image = self.selected().is_some_and(|t| t.image.is_some());
        let editable = self.editable_image();
        let active = self
            .selected()
            .is_some_and(|t| t.image.is_some() || !t.text.is_empty());
        [
            (200, 0, G::Back, self.active > 0),
            (201, 42, G::Forward, self.active + 1 < self.tabs.len()),
            (2, 94, G::ZoomOut, image),
            (3, 136, G::Fit, image),
            (4, 212, G::ZoomIn, image),
            (3, 254, G::Fit, image),
            (5, 296, G::Rotate, editable),
            (205, 338, G::Hand, false),
            (206, 380, G::Crop, editable),
            (6, 422, G::Convert, editable),
            (207, 464, G::Edit, false),
            (7, 506, G::Save, active),
            (1, 548, G::Folder, true),
            (
                209,
                590,
                G::History,
                self.selected().is_some_and(|t| t.backup.is_some()),
            ),
        ]
        .into_iter()
        .map(|(id, x, g, e)| {
            // Buttons after the zoom value shift by its growth.
            let shift = if x > 136 { (zoom_w() - 66) as i32 } else { 0 };
            (
                id,
                R::new(
                    left + x + shift,
                    72,
                    if x == 136 { zoom_w() } else { 36 },
                    tool_h(),
                ),
                g,
                e,
            )
        })
        .filter(|(_, r, _, _)| r.x + r.w as i32 <= self.width as i32 - 48)
        .collect()
    }
    pub(super) fn can_export(&self) -> bool {
        self.convert_target().is_some()
    }
    pub(super) fn export_label(&self) -> &'static str {
        self.convert_target().map_or("No conversion", |t| t.label())
    }
    pub(super) fn actions(&self) -> Vec<(u32, R)> {
        let mut a = Vec::new();
        if self.cropping {
            let [apply, cancel] = self.crop_controls();
            a.push((210, apply));
            a.push((211, cancel));
            return a;
        }
        if self.notice_open {
            for i in 0..3 {
                let (x, y, w, h) = self.notice_button(i);
                a.push((920 + i, R::new(x, y, w, h)));
            }
            return a;
        }
        // Caption tabs; with no document, slot 0 is the home tab.
        let strip = self.strip();
        for i in 0..self.tabs.len().max(1) {
            if let (Some(t), Some(c)) = (strip.tab(i), strip.close(i)) {
                a.push((100 + i as u32, R::new(t.x, t.y, t.width, t.height)));
                a.push((300 + i as u32, R::new(c.x, c.y, c.width, c.height)));
            }
        }
        if let Some(r) = self.plus() {
            a.push((1, r));
        }
        // Navigation pane: only the visible part of each row answers.
        let pane = self.nav_pane();
        let rows = self.nav_rows();
        for p in pane.layout(&rows) {
            match p.row {
                Row::Item(i) => a.push((400 + i as u32, p.visible)),
                Row::Header(g) if !pane.compact => a.push((HEADER_ID + g as u32, p.visible)),
                Row::Header(_) => (),
            }
        }
        if let Some(track) = pane.track(&rows) {
            a.push((THUMB_ID, track));
        }
        if self.settings_rect().y >= pane.top {
            a.push((400 + SETTINGS as u32, self.settings_rect()));
        }
        for (id, r, _, enabled) in self.toolbar() {
            if enabled {
                a.push((id, r));
            }
        }
        if self.height >= 240 || matches!(self.section, 0 | 10) {
            a.push((202, R::new(self.width as i32 - 44, 72, 32, tool_h())));
        }
        let body = self.body();
        let (x, y, w, h) = (body.x, body.y, body.w, body.h);
        if matches!(self.section, 1 | 3..=9) {
            for (i, _) in self
                .library_paths()
                .iter()
                .skip(self.section_scroll)
                .take(self.library_capacity(body))
                .enumerate()
            {
                a.push((600 + i as u32, self.library_row(body, i)));
            }
        }
        if self.section == 16 {
            for (i, id) in [202, 204, 203].iter().skip(self.section_scroll).enumerate() {
                a.push((*id, self.settings_row(body, i)));
            }
        }
        let _ = (x, y, w, h);
        if matches!(self.section, 1 | 3..=9 | 16) {
            a.push((
                422,
                R::new(x + w as i32 - 88, self.height as i32 - 52, 36, 28),
            ));
            a.push((
                423,
                R::new(x + w as i32 - 44, self.height as i32 - 52, 36, 28),
            ));
        }
        if let Some(p) = self.panel() {
            let info = self.info_layout(p);
            a.push((9, info.more));
            if let Some(q) = &info.quick {
                for (i, id) in [Some(206), Some(5), None, None, Some(6), Some(7)]
                    .iter()
                    .enumerate()
                {
                    if let Some(id) = id {
                        if !self.editable_image() && *id != 7 {
                            continue;
                        }
                        if *id == 7 && !self.can_export() {
                            continue;
                        }
                        a.push((*id, q.actions[i]));
                    }
                }
                if self.can_export() {
                    if self.convert_targets().len() > 1 {
                        a.push((203, q.format));
                    }
                    a.push((7, q.convert));
                }
            } else if self.can_export() {
                a.push((7, info.export));
            }
        }
        if self.section == 0 || self.section == 10 {
            for i in 0..self.tabs.len() {
                if let Some((tx, ty)) = self.film_rect(i) {
                    a.push((100 + i as u32, R::new(tx, ty, 108, 88)));
                }
            }
        }
        if self.section == 11 && self.can_export() {
            let [format, convert] = self.conversion_controls();
            if self.convert_targets().len() > 1 {
                a.push((203, format));
            }
            a.push((7, convert));
        }
        if let Some((button, track)) = self.transport() {
            a.push((700, button));
            a.push((
                701,
                R::new(track.x - 10, track.y - 8, track.w + 20, track.h + 16),
            ));
        }
        if self.export_uncertainty.is_some() {
            a.push((923, self.notice_link()));
        }
        if self.opening {
            a.clear();
            for (i, r) in self.open_controls().into_iter().enumerate() {
                a.push((90 + i as u32, r));
            }
            if self.password_needed {
                a.push((93, self.password_box()));
            }
        }
        a.retain(|(_, r)| {
            r.w > 0
                && r.h > 0
                && r.x >= 0
                && r.y >= 0
                && r.x + r.w as i32 <= self.width as i32
                && r.y + r.h as i32 <= self.height as i32
        });
        a
    }
    pub(super) fn library_paths(&self) -> Vec<String> {
        if self.section == 1 {
            return self.recent.clone();
        }
        self.tabs
            .iter()
            .filter(|t| match self.section {
                3 => Kind::of(&t.path, false) == Kind::Image,
                4 => Kind::of(&t.path, false) == Kind::Video,
                5 => Kind::of(&t.path, false) == Kind::Audio,
                6 => matches!(
                    Kind::of(&t.path, false),
                    Kind::Text
                        | Kind::Document
                        | Kind::Pdf
                        | Kind::Spreadsheet
                        | Kind::Presentation
                ),
                7 => Kind::of(&t.path, false) == Kind::Model,
                8 => Kind::of(&t.path, false) == Kind::Archive,
                9 => !matches!(
                    Kind::of(&t.path, false),
                    Kind::Image
                        | Kind::Video
                        | Kind::Audio
                        | Kind::Model
                        | Kind::Archive
                        | Kind::Text
                        | Kind::Document
                        | Kind::Pdf
                        | Kind::Spreadsheet
                        | Kind::Presentation
                ),
                _ => true,
            })
            .map(|t| t.path.clone())
            .collect()
    }
    /// Open/Export dialog growth: title, field, helper line and buttons.
    fn open_grow() -> i32 {
        grow(TextStyle::Semibold20) + 2 * grow(TextStyle::Regular14) + grow(TextStyle::Regular12)
    }
    /// Whether the dialog uses its compact (short window) arrangement.
    fn open_compact(r: R) -> bool {
        (r.h as i32) < 160 + Self::open_grow()
    }
    fn open_dialog(&self) -> R {
        let w = self.width.saturating_sub(48).min(620);
        let extra = if self.password_needed { 76 } else { 0 };
        let h = (180 + extra + Self::open_grow() as u32).min(self.height.saturating_sub(16));
        R::new(
            ((self.width - w) / 2) as i32,
            ((self.height - h) / 2) as i32,
            w,
            h,
        )
    }
    /// The password box under the path (and its help line), when the
    /// export needs one.
    pub(super) fn password_box(&self) -> R {
        let [field, ..] = self.open_controls();
        R::new(field.x, field.y + field.h as i32 + 54, field.w, field.h)
    }
    fn open_controls(&self) -> [R; 3] {
        let r = self.open_dialog();
        let compact = Self::open_compact(r);
        let (title, g) = (grow(TextStyle::Semibold20), grow(TextStyle::Regular14));
        let inset = if compact { 8 } else { 20 };
        let field = R::new(
            r.x + inset,
            r.y + title + if compact { 34 } else { 56 },
            r.w.saturating_sub(inset as u32 * 2),
            (g + if compact { 30 } else { 42 }) as u32,
        );
        let button_w = text_w(96).min(r.w.saturating_sub(inset as u32 * 2 + 8) / 2);
        let button_h = (g + if compact { 26 } else { 36 }) as u32;
        let by = r.y + r.h as i32 - button_h as i32 - inset;
        [
            field,
            R::new(
                r.x + r.w as i32 - inset - 2 * button_w as i32 - 8,
                by,
                button_w,
                button_h,
            ),
            R::new(
                r.x + r.w as i32 - inset - button_w as i32,
                by,
                button_w,
                button_h,
            ),
        ]
    }
    fn conversion_controls(&self) -> [R; 2] {
        let r = self.body();
        let (title, g) = (grow(TextStyle::Semibold28), grow(TextStyle::Regular14));
        if r.w >= 420 {
            let (first, second) = (text_w(180), text_w(160));
            let y = r.y + 124 + title + grow(TextStyle::Regular16);
            let h = (38 + g) as u32;
            return [
                R::new(r.x + 24, y, first, h),
                R::new(r.x + 40 + first as i32, y, second, h),
            ];
        }
        let h = (g + if self.height < 240 { 28 } else { 38 }) as u32;
        let y = if self.height < 240 {
            48
        } else {
            r.y + 76 + title
        };
        [
            R::new(r.x + 8, y, r.w.saturating_sub(16), h),
            R::new(r.x + 8, y + h as i32 + 8, r.w.saturating_sub(16), h),
        ]
    }
    pub(super) fn paint(&self, v: &mut View<'_>) {
        // Explorer caption band with document tabs; the app surface below
        // continues the selected tab, and a hairline separates the pane.
        let surface = door_workspace::caption_tabs::surface_color(v.palette.dark);
        v.rect(0, 0, v.width, v.height, surface);
        door_workspace::caption_tabs::paint_band(v);
        v.rect(
            self.rail() as i32,
            64,
            1,
            v.height.saturating_sub(64),
            if v.palette.dark {
                [52, 47, 42, 255]
            } else {
                [233, 229, 225, 255]
            },
        );
        if matches!(self.section, 1 | 3..=9 | 16) {
            f::tool(
                v,
                R::new(
                    self.body().x + self.body().w as i32 - 88,
                    self.height as i32 - 52,
                    36,
                    28,
                ),
                G::Back,
                self.section_scroll > 0,
                false,
            );
            f::tool(
                v,
                R::new(
                    self.body().x + self.body().w as i32 - 44,
                    self.height as i32 - 52,
                    36,
                    28,
                ),
                G::Forward,
                true,
                false,
            );
        }
        self.paint_nav(v);
        self.paint_caption_tabs(v);
        for (id, r, g, enabled) in self.toolbar() {
            if id == 3 && r.w == zoom_w() {
                let label = if self.fit {
                    "Fit".into()
                } else {
                    format!("{}%", self.zoom)
                };
                // Zoom value centred in the space left of its dropdown chevron.
                f::boxed_label(
                    v,
                    R::new(r.x, r.y, r.w - 24, r.h),
                    &label,
                    TextStyle::Regular14,
                    FG,
                );
                f::glyph(
                    v,
                    r.x + r.w as i32 - 22,
                    r.y + (r.h as i32 - 20) / 2,
                    G::Down,
                    MUTED,
                );
            } else {
                f::tool(v, r, g, enabled, false);
            }
        }
        if self.height >= 240 || matches!(self.section, 0 | 10) {
            f::tool(
                v,
                R::new(self.width as i32 - 44, 72, 32, tool_h()),
                G::Info,
                true,
                self.info_open,
            );
        }
        let (x, y, w, h) = self.dimensions();
        if self.section == 0 || self.section == 10 {
            self.paint_viewer(v, (x, y, w, h));
            self.paint_transport(v);
        } else {
            self.paint_section(v, self.body());
        }
        if let Some(p) = self.panel() {
            self.paint_information(v, p);
        }
        if self.section == 0 || self.section == 10 {
            for (i, t) in self.tabs.iter().enumerate() {
                let Some((tx, ty)) = self.film_rect(i) else {
                    continue;
                };
                v.rounded(
                    tx,
                    ty,
                    108,
                    88,
                    7,
                    if i == self.active {
                        [220, 143, 42, 255]
                    } else {
                        [230, 230, 230, 255]
                    },
                );
                v.rounded(
                    tx + 2,
                    ty + 2,
                    104,
                    84,
                    6,
                    if v.palette.dark {
                        [44, 40, 37, 255]
                    } else {
                        [255, 253, 250, 255]
                    },
                );
                if let Some(image) = &t.image {
                    v.image_fit(
                        (tx + 5, ty + 5, 98, 78),
                        image.width,
                        image.height,
                        &image.bgra,
                    );
                } else {
                    v.large_icon(tx + 34, ty + 23, Icon::for_file_name(&t.path));
                }
            }
        }
        let message = if let Some(j) = &self.job {
            format!("Opening file... {} / {} bytes", j.bytes.len(), j.info.bytes)
        } else if let Some(j) = &self.output {
            format!("Exporting... {} / {} bytes", j.written, j.bytes.len())
        } else {
            self.message.clone()
        };
        v.small_clip(
            x + 4,
            self.height as i32 - 24 - grow(TextStyle::Regular12),
            self.width.saturating_sub(x as u32 + 32),
            &message,
            MUTED,
        );
        if self.export_uncertainty.is_some() {
            f::button(
                v,
                self.notice_link(),
                "View export notice",
                Some(G::Info),
                false,
                true,
            );
        }
        if self.opening {
            let r = self.open_dialog();
            let compact = Self::open_compact(r);
            let [field, confirm, cancel] = self.open_controls();
            f::card(v, r);
            f::label(
                v,
                R::new(
                    r.x + 8,
                    r.y + if compact { 2 } else { 12 },
                    r.w.saturating_sub(16),
                    (30 + grow(TextStyle::Semibold20)) as u32,
                ),
                match (self.exporting, self.convert_target()) {
                    (true, Some(converter::Target::Folder)) => "Extract all files",
                    (true, _) => "Convert a copy",
                    (false, _) => "Open file",
                },
                TextStyle::Semibold20,
                FG,
            );
            v.rounded(field.x, field.y, field.w, field.h, 4, [255; 4]);
            v.line_editor(
                field.x + 8,
                field.y + (field.h as i32 - 16) / 2,
                field.w.saturating_sub(16),
                &self.path,
                self.caret && !self.password_focus,
            );
            if !compact {
                f::label(
                    v,
                    R::new(
                        r.x + 20,
                        field.y + field.h as i32 + 2,
                        r.w.saturating_sub(40),
                        (24 + grow(TextStyle::Regular12)) as u32,
                    ),
                    match (self.exporting, self.convert_target()) {
                        (true, Some(converter::Target::Folder)) => {
                            "A new folder for the files. The archive stays unchanged."
                        }
                        (true, Some(t)) if t.needs_password() => {
                            "Every entry is encrypted with AES-256 using the password below."
                        }
                        (true, _) => "Choose a new file name. The original stays unchanged.",
                        (false, _) => "Enter a file path, then choose Open.",
                    },
                    TextStyle::Regular12,
                    MUTED,
                );
            }
            if self.password_needed {
                let b = self.password_box();
                f::label(
                    v,
                    R::new(b.x, b.y - 26, b.w, 22),
                    "Password",
                    TextStyle::Regular14,
                    FG,
                );
                v.rounded(b.x, b.y, b.w, b.h, 4, [255; 4]);
                // The focused box has the WinUI accent underline.
                if self.password_focus {
                    v.rect(
                        b.x + 1,
                        b.y + b.h as i32 - 2,
                        b.w.saturating_sub(2),
                        2,
                        ACCENT,
                    );
                }
                let masked: String =
                    core::iter::repeat_n('*', self.password.text().chars().count()).collect();
                let mut shown = LineEdit::default();
                shown.set(masked, false);
                v.line_editor(
                    b.x + 8,
                    b.y + (b.h as i32 - 16) / 2,
                    b.w.saturating_sub(16),
                    &shown,
                    self.caret && self.password_focus,
                );
            }
            f::button(
                v,
                confirm,
                match (self.exporting, self.convert_target()) {
                    (true, Some(converter::Target::Folder)) => "Extract",
                    (true, _) => "Convert",
                    (false, _) => "Open",
                },
                None,
                true,
                true,
            );
            f::button(v, cancel, "Cancel", None, false, true);
        }
    }
    /// Explorer caption tabs: documents carry their file-type icon; with no
    /// document open, the home tab carries the Preview identity.
    fn paint_caption_tabs(&self, v: &mut View<'_>) {
        use door_workspace::caption_tabs::{TabIcon, paint_plus, paint_tab};
        let strip = self.strip();
        if self.tabs.is_empty()
            && let Some(t) = strip.tab(0)
        {
            let logo = match v.scale {
                3 => door_workspace::asset!("../../../assets/branding/preview.48.bgra"),
                2 => door_workspace::asset!("../../../assets/branding/preview.32.bgra"),
                _ => door_workspace::asset!("../../../assets/branding/preview.16.bgra"),
            };
            paint_tab(v, t, strip.close(0), TabIcon::Color(logo), "Preview", true);
        }
        for (i, t) in self.tabs.iter().enumerate() {
            if let Some(r) = strip.tab(i) {
                paint_tab(
                    v,
                    r,
                    strip.close(i),
                    TabIcon::Shell(Icon::for_file_name(&t.path)),
                    t.path.rsplit('/').next().unwrap_or("File"),
                    i == self.active,
                );
            }
        }
        if let Some(r) = strip.plus() {
            paint_plus(v, r);
        }
    }
    /// Mouse wheel: the navigation pane scrolls by pixels; library lists and
    /// text documents scroll by rows. Images have no scroll (zoom uses the
    /// toolbar; the wheel carries no modifier state).
    pub(super) fn scroll(&mut self, x: i32, y: i32, notches: i32) -> bool {
        if notches == 0 || self.modal() {
            return false;
        }
        let pane = self.nav_pane();
        if x < pane.width as i32 && y >= pane.top && y < pane.bottom {
            let before = self.nav_scroll;
            let next = self.nav_scroll as i64 - i64::from(notches) * 48;
            self.nav_scroll = next.clamp(0, u32::MAX as i64) as u32;
            self.clamp_nav();
            return self.nav_scroll != before;
        }
        let body = self.body();
        if !body.hit(x, y) {
            return false;
        }
        if matches!(self.section, 1 | 3..=9 | 16) {
            let last = if self.section == 16 {
                2
            } else {
                self.library_paths().len().saturating_sub(1)
            };
            let before = self.section_scroll;
            self.section_scroll =
                (self.section_scroll as i64 - i64::from(notches)).clamp(0, last as i64) as usize;
            return self.section_scroll != before;
        }
        if matches!(self.section, 0 | 10)
            && let Some(tab) = self.tabs.get_mut(self.active)
            && tab.image.is_none()
            && !tab.text.is_empty()
        {
            let last = tab.text.lines().count().saturating_sub(1) as i64;
            let before = tab.scroll;
            tab.scroll = (tab.scroll as i64 - i64::from(notches) * 3).clamp(0, last) as usize;
            return tab.scroll != before;
        }
        false
    }
    /// Paints the navigation pane from the same layout `actions` hit-tests.
    fn paint_nav(&self, v: &mut View<'_>) {
        let hovered = v.hover.and_then(|(x, y)| self.target(x, y));
        let pressed = v.pressed.and_then(|(x, y)| self.target(x, y));
        let state = |id: u32, selected: bool| RowState {
            selected,
            enabled: true,
            hovered: hovered == Some(id),
            pressed: pressed == Some(id),
        };
        let compact = self.rail() <= 64;
        let spacing = if compact { (10, 52) } else { (14, 55) };
        let item = |v: &mut View<'_>, i: usize, r: R| {
            let (label, g) = NAV[i];
            f::nav_row(
                v,
                r,
                if compact { "" } else { label },
                g,
                state(400 + i as u32, self.section == i as u32),
                match i {
                    3 | 7 => [204, 112, 0, 255],
                    4 => [184, 94, 142, 255],
                    5 => [157, 94, 220, 255],
                    8 => [18, 166, 212, 255],
                    14 => [60, 60, 210, 255],
                    _ => FG,
                },
                spacing,
            );
        };
        let pane = self.nav_pane();
        let rows = self.nav_rows();
        if pane.clip(v) {
            for p in pane.layout(&rows) {
                match p.row {
                    Row::Item(i) => item(v, i, p.rect),
                    Row::Header(_) if compact => f::nav_separator(v, p.rect),
                    Row::Header(g) => {
                        let (label, first, last) = GROUPS[g as usize];
                        let open = self.group_open(g as usize);
                        // A collapsed group carries its hidden child's selection.
                        let hidden = !open && (first..=last).contains(&(self.section as usize));
                        f::nav_header(v, p.rect, label, open, state(HEADER_ID + g as u32, hidden));
                    }
                }
            }
            v.reset_viewport();
        }
        if let Some(t) = pane.thumb(&rows) {
            let active = self.nav_grab.is_some() || hovered == Some(THUMB_ID);
            f::pane_thumb(v, pane.width as i32 - 7, t.top, t.height, active);
            // Mark the boundary with the pinned row when destinations scroll under it.
            v.rect(
                20,
                pane.bottom + 5,
                pane.width.saturating_sub(40),
                1,
                [222, 222, 222, 255],
            );
        }
        let settings = self.settings_rect();
        if settings.y >= pane.top {
            item(v, SETTINGS, settings);
        }
    }
    /// The audio transport under the waveform: (play/pause button, seek
    /// track), when the selected tab holds decoded audio.
    pub(super) fn transport(&self) -> Option<(R, R)> {
        if !matches!(self.section, 0 | 10) || self.selected().is_none_or(|t| t.audio.is_none()) {
            return None;
        }
        let (x, y, w, h) = self.dimensions();
        if w < 280 || h < 160 {
            return None;
        }
        let bottom = y + h as i32;
        Some((
            R::new(x + 16, bottom - 60, 44, 44),
            R::new(x + 128, bottom - 44, w - 256, 12),
        ))
    }
    /// The playing position of the selected tab (0..=1000 of its length) and
    /// the (elapsed, total) times.
    pub(super) fn transport_state(&self) -> (u32, String, String, bool) {
        use door_workspace::audio_codec::duration_text;
        let Some(tab) = self.selected() else {
            return (0, String::new(), String::new(), false);
        };
        let Some(pcm) = tab.audio.as_ref() else {
            return (0, String::new(), String::new(), false);
        };
        let frames = pcm.frames().max(1);
        let (position, playing) = match self.player.as_ref() {
            Some(p) if p.tab == tab.id => (p.position(), !p.paused && !p.finished),
            _ => (0, false),
        };
        let permille = (position as u64 * 1000 / frames as u64) as u32;
        let rate = u64::from(pcm.rate.max(1));
        (
            permille,
            duration_text(position as u64 * 1000 / rate),
            duration_text(pcm.millis()),
            playing,
        )
    }
    fn paint_transport(&self, v: &mut View<'_>) {
        let Some((button, track)) = self.transport() else {
            return;
        };
        let (permille, elapsed, total, playing) = self.transport_state();
        let dark = v.palette.dark;
        let text = if dark { [255, 255, 255, 255] } else { FG };
        // Play/pause: an accent circle with the glyph in white.
        v.rounded(button.x, button.y, button.w, button.h, button.w / 2, ACCENT);
        let (cx, cy) = (
            button.x + button.w as i32 / 2,
            button.y + button.h as i32 / 2,
        );
        if playing {
            v.rect(cx - 6, cy - 8, 4, 16, [255; 4]);
            v.rect(cx + 2, cy - 8, 4, 16, [255; 4]);
        } else {
            for row in 0..16 {
                let span = if row < 8 { row + 1 } else { 16 - row };
                v.rect(
                    cx - 4,
                    cy - 8 + row,
                    (span as u32 * 12 / 8).max(1),
                    1,
                    [255; 4],
                );
            }
        }
        v.text_clip(
            button.x + 56,
            track.y - 5,
            60,
            &elapsed,
            TextStyle::Regular12,
            text,
        );
        let rail = if dark {
            [158, 158, 158, 255]
        } else {
            [134, 134, 134, 255]
        };
        let cy = track.y + track.h as i32 / 2;
        v.rounded(track.x, cy - 2, track.w, 4, 2, rail);
        let fill = track.w * permille / 1000;
        if fill > 0 {
            v.rounded(track.x, cy - 2, fill, 4, 2, ACCENT);
        }
        let thumb_x = track.x + fill as i32 - 10;
        let ring = if dark {
            [69, 69, 69, 255]
        } else {
            [255, 255, 255, 255]
        };
        v.rounded(
            thumb_x,
            cy - 10,
            20,
            20,
            10,
            if dark {
                [80, 80, 80, 255]
            } else {
                [210, 210, 210, 255]
            },
        );
        v.rounded(thumb_x + 1, cy - 9, 18, 18, 9, ring);
        v.rounded(thumb_x + 4, cy - 6, 12, 12, 6, ACCENT);
        v.text_clip(
            track.x + track.w as i32 + 12,
            track.y - 5,
            100,
            &total,
            TextStyle::Regular12,
            text,
        );
    }
    fn paint_viewer(&self, v: &mut View<'_>, r: (i32, i32, u32, u32)) {
        let (x, y, w, h) = r;
        v.rounded(
            x,
            y,
            w,
            h,
            4,
            if v.palette.dark {
                [29, 27, 25, 255]
            } else {
                [242, 239, 234, 255]
            },
        );
        let h = if self.transport().is_some() {
            h.saturating_sub(72)
        } else {
            h
        };
        if let Some(t) = self.selected() {
            if let Some(image) = &t.image {
                if self.fit {
                    v.image_fit(
                        (x + 8, y + 8, w.saturating_sub(16), h.saturating_sub(16)),
                        image.width,
                        image.height,
                        &image.bgra,
                    );
                } else {
                    v.image_zoom(
                        (x + 8, y + 8, w.saturating_sub(16), h.saturating_sub(16)),
                        image.width,
                        image.height,
                        &image.bgra,
                        self.zoom,
                    );
                }
            } else if !t.text.is_empty() {
                v.rounded(
                    x + 8,
                    y + 8,
                    w.saturating_sub(16),
                    h.saturating_sub(16),
                    3,
                    [255; 4],
                );
                for (i, line) in t
                    .text
                    .lines()
                    .skip(t.scroll)
                    .take(h.saturating_sub(28) as usize / 22)
                    .enumerate()
                {
                    v.text_clip(
                        x + 24,
                        y + 16 + i as i32 * 22,
                        w.saturating_sub(48),
                        &door_app_ui::ui::display_ascii(line),
                        TextStyle::Regular14,
                        FG,
                    );
                }
            } else {
                f::glyph(v, x + 28, y + 36, G::Document, MUTED);
                f::label(
                    v,
                    R::new(x + 24, y + 80, w.saturating_sub(48), 64),
                    &t.error,
                    TextStyle::Regular14,
                    MUTED,
                );
            }
        } else {
            f::glyph(v, x + 28, y + 36, G::Image, ACCENT);
            f::label(
                v,
                R::new(x + 24, y + 80, w.saturating_sub(48), 42),
                "Open a file to preview it",
                TextStyle::Semibold20,
                FG,
            );
            f::label(
                v,
                R::new(x + 24, y + 126, w.saturating_sub(48), 40),
                "Images, text, archive listings, audio waveforms and 3D wireframes",
                TextStyle::Regular14,
                MUTED,
            );
        }
    }
    fn paint_information(&self, v: &mut View<'_>, p: R) {
        v.rect(
            p.x,
            p.y,
            p.w,
            p.h,
            if v.palette.dark {
                [39, 37, 34, 255]
            } else {
                [252, 249, 245, 255]
            },
        );
        v.rect(
            p.x,
            p.y,
            1,
            p.h,
            if v.palette.dark {
                [57, 54, 50, 255]
            } else {
                [243, 236, 228, 255]
            },
        );
        let Some(t) = self.selected() else {
            let title = grow(TextStyle::Semibold20);
            f::label(
                v,
                R::new(p.x + 20, p.y + 26, p.w - 40, (40 + title) as u32),
                "File information",
                TextStyle::Semibold20,
                FG,
            );
            f::paragraph(
                v,
                R::new(p.x + 20, p.y + 80 + title, p.w - 40, text_w(80)),
                "Open a file to inspect its information and available actions.",
                MUTED,
            );
            return;
        };
        let info = self.info_layout(p);
        v.rect(p.x + 16, info.divider, p.w - 32, 1, [238, 238, 238, 255]);
        if let Some(q) = &info.quick {
            for y in q.dividers {
                v.rect(p.x + 16, y, p.w - 32, 1, [238, 238, 238, 255]);
            }
        }
        if let Some(image) = &t.image {
            v.image_fit(
                (p.x + 20, p.y + 26, 80, 80),
                image.width,
                image.height,
                &image.bgra,
            );
        } else {
            v.large_icon(p.x + 36, p.y + 36, Icon::for_file_name(&t.path));
        }
        f::label(
            v,
            info.name,
            t.path.rsplit('/').next().unwrap_or("File"),
            TextStyle::Semibold16,
            FG,
        );
        f::label(
            v,
            info.kind,
            Kind::of(&t.path, false).label(),
            TextStyle::Regular14,
            MUTED,
        );
        f::label(
            v,
            info.size,
            &codec::size(t.info.bytes),
            TextStyle::Regular14,
            MUTED,
        );
        f::label(v, info.heading, "Information", TextStyle::Semibold16, FG);
        f::label(v, info.more, "Show more", TextStyle::Regular14, ACCENT);
        let rows = self.info_rows(t);
        let count = info.count;
        let page = self.info_page.min(self.info_pages() - 1);
        let value_x = p.x + 20 + info.name_w as i32 + 16;
        let value_w = (p.x + p.w as i32 - 20 - value_x).max(0) as u32;
        for (i, (name, value)) in rows.iter().skip(page * count).take(count).enumerate() {
            let y = info.rows_top + i as i32 * info.pitch;
            f::label(
                v,
                R::new(p.x + 20, y, info.name_w, info.row_h),
                name,
                TextStyle::Regular14,
                MUTED,
            );
            f::label(
                v,
                R::new(value_x, y, value_w, info.row_h),
                value,
                TextStyle::Regular14,
                FG,
            );
        }
        if let Some(q) = &info.quick {
            f::label(v, q.heading, "Quick actions", TextStyle::Semibold16, FG);
            for (i, (label, g, enabled)) in [
                ("Crop", G::Crop, self.editable_image()),
                ("Rotate", G::Rotate, self.editable_image()),
                ("Adjust", G::Adjust, false),
                ("Markup", G::Edit, false),
                ("Flip", G::Convert, self.editable_image()),
                ("Export", G::Save, self.can_export()),
            ]
            .iter()
            .enumerate()
            {
                let r = q.actions[i];
                if *enabled && v.hover.is_some_and(|(x, y)| r.hit(x, y)) {
                    v.rounded(r.x, r.y, r.w, r.h, 5, [234, 234, 234, 255]);
                }
                f::glyph(v, r.x + 46, r.y + 10, *g, if *enabled { FG } else { MUTED });
                f::centered_label14(
                    v,
                    R::new(r.x + 4, r.y + 42, r.w - 8, r.h.saturating_sub(58)),
                    label,
                    if *enabled { FG } else { MUTED },
                );
            }
            f::label(v, q.convert_heading, "Convert", TextStyle::Semibold16, FG);
            f::button(
                v,
                q.format,
                self.export_label(),
                Some(G::Down),
                false,
                self.convert_targets().len() > 1,
            );
            f::button(v, q.convert, "Convert...", None, true, self.can_export());
        } else {
            f::button(
                v,
                info.export,
                "Export a copy",
                Some(G::Save),
                true,
                self.can_export(),
            );
        }
    }
    fn paint_section(&self, v: &mut View<'_>, r: R) {
        f::card(v, r);
        let title = NAV.get(self.section as usize).map_or("Home", |n| n.0);
        let title_g = grow(TextStyle::Semibold28);
        f::label(
            v,
            R::new(
                r.x + 24,
                r.y + 16,
                r.w.saturating_sub(48),
                (42 + title_g) as u32,
            ),
            title,
            TextStyle::Semibold28,
            FG,
        );
        if matches!(self.section, 1 | 3..=9) {
            let paths = self.library_paths();
            let (g16, g12) = (grow(TextStyle::Regular16), grow(TextStyle::Regular12));
            for (i, path) in paths
                .iter()
                .skip(self.section_scroll)
                .take(self.library_capacity(r))
                .enumerate()
            {
                let rr = self.library_row(r, i);
                if v.hover.is_some_and(|(x, y)| rr.hit(x, y)) {
                    v.rounded(rr.x, rr.y, rr.w, rr.h, 4, [236, 236, 236, 255]);
                }
                v.icon(
                    rr.x + 12,
                    rr.y + (rr.h as i32 - 20) / 2,
                    Icon::for_file_name(path),
                    FG,
                );
                f::label(
                    v,
                    R::new(rr.x + 48, rr.y, rr.w.saturating_sub(60), (30 + g16) as u32),
                    path.rsplit('/').next().unwrap_or("File"),
                    TextStyle::Regular16,
                    FG,
                );
                f::label(
                    v,
                    R::new(
                        rr.x + 48,
                        rr.y + 30 + g16,
                        rr.w.saturating_sub(60),
                        (22 + g12) as u32,
                    ),
                    path,
                    TextStyle::Regular12,
                    MUTED,
                );
            }
            if paths.is_empty() {
                f::label(
                    v,
                    R::new(
                        r.x + 24,
                        r.y + 92 + title_g,
                        r.w.saturating_sub(48),
                        text_w(60),
                    ),
                    "No matching files opened in this session.",
                    TextStyle::Regular16,
                    MUTED,
                );
            }
            return;
        }
        if self.section == 11 {
            let [format, convert] = self.conversion_controls();
            if r.w >= 420 {
                f::label(
                    v,
                    R::new(
                        r.x + 24,
                        r.y + 76 + title_g,
                        r.w.saturating_sub(48),
                        (34 + grow(TextStyle::Regular16)) as u32,
                    ),
                    "Create a converted copy of the selected file",
                    TextStyle::Regular16,
                    FG,
                );
            }
            f::button(
                v,
                format,
                self.export_label(),
                Some(G::Down),
                false,
                self.convert_targets().len() > 1,
            );
            f::button(
                v,
                convert,
                "Convert...",
                Some(G::Save),
                true,
                self.can_export(),
            );
            return;
        }
        if self.section == 16 {
            for (i, (name, val)) in [
                (
                    "Information pane",
                    if self.info_open { "Shown" } else { "Hidden" },
                ),
                ("Reset zoom and fit", "Reset"),
                ("Convert to", self.export_label()),
            ]
            .iter()
            .skip(self.section_scroll)
            .enumerate()
            {
                let row = self.settings_row(r, i);
                let button = Self::settings_button(row);
                f::label(
                    v,
                    R::new(
                        row.x + 12,
                        row.y,
                        (button.x - 4 - row.x - 12).max(0) as u32,
                        row.h,
                    ),
                    name,
                    TextStyle::Regular14,
                    FG,
                );
                f::button(v, button, val, None, false, true);
            }
            return;
        }
        let message = match self.section {
            2 => "No sharing provider is connected.",
            12 => {
                "Batch conversion is not available. Convert individual files from the information pane."
            }
            13 => "A screen-capture provider is not connected.",
            14 => "A recording provider is not connected.",
            15 => "An OCR provider is not connected.",
            _ => "Select a file to use the available tools.",
        };
        f::paragraph(
            v,
            R::new(
                r.x + 24,
                r.y + 88 + title_g,
                r.w.saturating_sub(48),
                text_w(96),
            ),
            message,
            MUTED,
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use door_workspace::nav_pane;
    #[test]
    fn quick_action_label_ink_is_centered_beneath_its_icon_at_each_density() {
        let mut app = Preview::new(1001);
        app.configure(1472, 952);
        app.tabs.push(Tab {
            id: door_workspace::document_tabs::next_id(),
            zoom: 100,
            fit: true,
            path: "/users/1001/image.ppm".into(),
            info: FileInfo {
                attributes: 0,
                object: 8,
                version: 1,
                bytes: 21,
                directory: false,
            },
            backup: None,
            image: Some(codec::decode_full(b"P3 1 1 255 255 0 0").unwrap()),
            text: String::new(),
            error: String::new(),
            scroll: 0,
            details: Vec::new(),
            raw: Vec::new(),
            audio: None,
        });
        for scale in 1..=3 {
            let fonts = door_graphics_text::FontAtlas::windows_reference_scaled(scale).unwrap();
            let width = 1472 * scale;
            let mut pixels = alloc::vec![255;(width*952*scale*4) as usize];
            let mut v = View::new_scaled(&mut pixels, width, 952 * scale, &fonts, scale);
            app.draw(&mut v);
            for i in 0..6 {
                let r = app.quick_action_rect(i);
                let mut left = i32::MAX;
                let mut right = i32::MIN;
                for y in (r.y + 42) * scale as i32..(r.y + 70) * scale as i32 {
                    for x in r.x * scale as i32..(r.x + r.w as i32) * scale as i32 {
                        let p = &pixels[((y as u32 * width + x as u32) * 4) as usize..][..3];
                        if p.iter().all(|c| *c < 160) {
                            left = left.min(x);
                            right = right.max(x);
                        }
                    }
                }
                assert!(left <= right, "label{i} missing");
                assert!(
                    (left + right - (r.x * 2 + r.w as i32) * scale as i32).abs()
                        <= 4 * scale as i32,
                    "label{i} off center"
                );
            }
        }
    }
    #[test]
    fn compact_image_conversion_uses_the_same_reachable_painted_bounds() {
        let mut app = Preview::new(1001);
        app.section = 11;
        app.tabs.push(Tab {
            id: door_workspace::document_tabs::next_id(),
            zoom: 100,
            fit: true,
            path: "/users/1001/image.ppm".into(),
            info: FileInfo {
                attributes: 0,
                object: 8,
                version: 1,
                bytes: 21,
                directory: false,
            },
            backup: None,
            image: Some(codec::decode_full(b"P3 1 1 255 255 0 0").unwrap()),
            text: String::new(),
            error: String::new(),
            scroll: 0,
            details: Vec::new(),
            raw: Vec::new(),
            audio: None,
        });
        for (w, h) in [(240, 120), (240, 600), (490, 358), (1472, 952)] {
            app.configure(w, h);
            for (id, r) in [203, 7].into_iter().zip(app.conversion_controls()) {
                assert_eq!(
                    app.target(r.x + r.w as i32 / 2, r.y + r.h as i32 / 2),
                    Some(id)
                );
                assert!(
                    r.x >= 0
                        && r.y >= 0
                        && r.x + r.w as i32 <= w as i32
                        && r.y + r.h as i32 <= h as i32
                );
            }
        }
    }
    fn nav_actions(app: &Preview) -> Vec<(u32, R)> {
        app.actions()
            .into_iter()
            .filter(|(id, _)| (400..=426).contains(id))
            .collect()
    }
    #[test]
    fn navigation_pane_reaches_every_destination_at_every_size_and_expansion() {
        for (w, h) in [(490, 358), (900, 300), (900, 600), (1472, 760), (1472, 952)] {
            for (library, tools) in [(true, true), (false, true), (true, false), (false, false)] {
                let mut app = Preview::new(1001);
                app.library_open = library;
                app.tools_open = tools;
                app.configure(w, h);
                let rows = app.nav_rows();
                let max = app.nav_pane().max_scroll(&rows);
                let mut reached = [false; 17];
                let mut scroll = 0;
                loop {
                    app.nav_scroll = scroll;
                    for (id, r) in nav_actions(&app) {
                        // No nav target may cover the canvas or another target.
                        assert!(r.x + r.w as i32 <= app.rail() as i32, "{id} at {w}x{h}");
                        assert_eq!(
                            app.target(r.x + r.w as i32 / 2, r.y + r.h as i32 / 2),
                            Some(id),
                            "{id} at {w}x{h} scroll {scroll}"
                        );
                        assert!(!matches!(id, 420 | 421), "stray scroll chevron");
                        if (400..=416).contains(&id) && r.h == nav_pane::row_height() {
                            reached[(id - 400) as usize] = true;
                        }
                    }
                    // Settings stays pinned and clickable at every scroll position.
                    assert!(nav_actions(&app).iter().any(|(id, _)| *id == 416));
                    if scroll == max {
                        break;
                    }
                    scroll = (scroll + 7).min(max);
                }
                let compact = app.rail() <= 64;
                for (i, hit) in reached.iter().enumerate() {
                    let hidden = !compact
                        && ((3..=9).contains(&i) && !library || (10..=15).contains(&i) && !tools);
                    assert_eq!(*hit, !hidden, "destination {i} at {w}x{h}");
                }
            }
        }
    }
    #[test]
    fn disclosure_toggles_only_its_own_children_and_never_navigates() {
        let mut app = Preview::new(1001);
        app.configure(1472, 952);
        app.select_section(5);
        let rows = app.nav_rows().len();
        for round in 0..40 {
            let library = round % 2 == 0;
            let (before_library, before_tools) = (app.library_open, app.tools_open);
            app.toggle_group(library);
            assert_eq!(app.section, 5, "a disclosure click navigated");
            assert_eq!(app.library_open, before_library ^ library);
            assert_eq!(app.tools_open, before_tools ^ !library);
            let header = app
                .nav_rows()
                .iter()
                .position(|r| *r == Row::Header(u8::from(!library)))
                .unwrap();
            // The toggled header stays visible and clickable.
            let target = 424 + u32::from(!library);
            let placed = app
                .nav_pane()
                .layout(&app.nav_rows())
                .into_iter()
                .find(|p| p.index == header)
                .expect("toggled header visible");
            assert_eq!(
                app.target(placed.visible.x + 8, placed.visible.y + 8),
                Some(target)
            );
        }
        // Even toggles restore the original rows exactly: nothing multiplies.
        assert_eq!(app.nav_rows().len(), rows);
        assert!(app.library_open && app.tools_open);
    }
    #[test]
    fn destination_clicks_keep_pane_state_and_reveal_the_selection() {
        let mut app = Preview::new(1001);
        app.configure(1472, 520);
        app.tools_open = false;
        let max = app.nav_pane().max_scroll(&app.nav_rows());
        assert!(max > 0);
        app.nav_scroll = max;
        app.select_section(15);
        assert!(
            !app.tools_open,
            "selection must not expand a collapsed group"
        );
        app.tools_open = true;
        app.clamp_nav();
        app.select_section(15);
        let rows = app.nav_rows();
        let index = rows.iter().position(|r| *r == Row::Item(15)).unwrap();
        assert!(
            app.nav_pane()
                .layout(&rows)
                .iter()
                .any(|p| p.index == index && p.visible == p.rect)
        );
        let scroll = app.nav_scroll;
        app.select_section(0);
        app.select_section(15);
        assert_eq!(app.nav_scroll, scroll);
        // Resizing clamps instead of leaving a blank pane.
        app.configure(1472, 2000);
        assert_eq!(app.nav_scroll, 0);
        assert!(nav_actions(&app).iter().any(|(id, _)| *id == 400));
    }
    #[test]
    fn minimum_open_and_export_dialogs_have_reachable_nonoverlapping_controls() {
        let mut app = Preview::new(1001);
        for (w, h) in [(240, 120), (490, 358), (900, 600), (1472, 952)] {
            app.configure(w, h);
            for exporting in [false, true] {
                app.exporting = exporting;
                app.opening = true;
                let controls = app.open_controls();
                for (i, r) in controls.iter().enumerate() {
                    assert_eq!(
                        app.target(r.x + r.w as i32 / 2, r.y + r.h as i32 / 2),
                        Some(90 + i as u32)
                    );
                    assert!(
                        r.x >= 0
                            && r.y >= 0
                            && r.x + r.w as i32 <= w as i32
                            && r.y + r.h as i32 <= h as i32
                    );
                }
                assert!(controls[0].y + controls[0].h as i32 <= controls[1].y);
                assert!(controls[1].x + controls[1].w as i32 <= controls[2].x);
                assert_eq!(app.actions().len(), 3);
            }
        }
    }
}
