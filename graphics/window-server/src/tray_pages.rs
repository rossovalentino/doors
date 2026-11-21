//! Quick settings' second tile page and its sub-pages (Windows 11 26H2):
//! the Live captions, Night light, Mobile hotspot, Nearby sharing, Cast and
//! Project tiles; the Accessibility, VPN, Cast and Project pages; the shared
//! slider and toggle switch; and the battery in the footer.
//!
//! Geometry follows the 26H2 flyout captured on the owner's PC (px / 1.5):
//! a 64 px page header (back button, title, key hint chips), 56 px list
//! rows with a 16 px glyph column, and the 48 px "More ... settings" footer.
use super::{Action, QuickPage, State, grow, quick_rect, shifted};
use crate::paint::{self, Palette};
use crate::taskbar::Layout;
use door_graphics_core::{Canvas, Rect};
use door_graphics_text::{FontAtlas, TextStyle};

/// Second tile page in Windows order; `None` reason = available.
pub const TILES_SECOND: [(&str, Option<&str>); 6] = [
    ("Live captions", Some("No speech recognition")),
    ("Night light", None),
    ("Mobile hotspot", Some("No Wi-Fi adapter")),
    ("Nearby sharing", Some("No Bluetooth or Wi-Fi")),
    ("Cast", None),
    ("Project", None),
];
/// Vertical distance between the two tile pages (two tile rows).
pub fn tile_page_height() -> i32 {
    2 * (96 + grow(TextStyle::Regular12))
}
/// The band the tiles scroll in (above the divider, left of the page
/// indicator).
pub fn tiles_band(layout: Layout) -> Rect {
    let r = quick_rect(layout);
    Rect::new(
        r.x() + 1,
        r.y() + 1,
        r.width() - 21,
        (212 + 2 * grow(TextStyle::Regular12)) as u32,
    )
    .unwrap()
}
/// The page indicator's arrows: up (shown on the second page) and down
/// (shown on the first).
pub fn page_up(layout: Layout) -> Rect {
    let r = quick_rect(layout);
    Rect::new(r.right() - 21, r.y() + 80, 16, 14).unwrap()
}
pub fn page_down(layout: Layout) -> Rect {
    let r = quick_rect(layout);
    Rect::new(r.right() - 21, r.y() + 120, 16, 14).unwrap()
}

/// A sub-page's frame: back button, title, the list area and the footer.
pub struct PageFrame {
    pub back: Rect,
    pub title: Rect,
    pub list: Rect,
    pub more: Rect,
}
pub fn page_frame(layout: Layout) -> PageFrame {
    let r = quick_rect(layout);
    let more = Rect::new(r.x() + 1, r.bottom() - 49, r.width() - 2, 48).unwrap();
    PageFrame {
        back: Rect::new(r.x() + 16, r.y() + 16, 32, 32).unwrap(),
        title: Rect::new(r.x() + 56, r.y() + 16, r.width() - 72, 32).unwrap(),
        list: Rect::new(
            r.x() + 1,
            r.y() + 60,
            r.width() - 2,
            (more.y() - r.y() - 60) as u32,
        )
        .unwrap(),
        more,
    }
}
/// The footer link of each page.
pub fn more_label(page: QuickPage) -> &'static str {
    match page {
        QuickPage::Main => "",
        QuickPage::SoundOutput => "More volume settings",
        QuickPage::Accessibility => "More Accessibility settings",
        QuickPage::Vpn => "More VPN settings",
        QuickPage::Cast => "More cast settings",
        QuickPage::Project => "More Display settings",
    }
}

/// Accessibility page rows (26H2 order, Vision then Hearing).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessRow {
    Magnifier,
    Narrator,
    ColorFilters,
    LiveCaptions,
    MonoAudio,
}
pub enum AccessItem {
    Heading(&'static str),
    Row(AccessRow, &'static str, &'static str),
}
pub const ACCESS_ITEMS: [AccessItem; 7] = [
    AccessItem::Heading("Vision"),
    AccessItem::Row(
        AccessRow::Magnifier,
        "Magnifier",
        "See words and images better",
    ),
    AccessItem::Row(
        AccessRow::Narrator,
        "Narrator",
        "Your built-in screen reader",
    ),
    AccessItem::Row(
        AccessRow::ColorFilters,
        "Color filters",
        "Distinguish among colors easily",
    ),
    AccessItem::Heading("Hearing"),
    AccessItem::Row(
        AccessRow::LiveCaptions,
        "Live captions",
        "Real time audio transcription",
    ),
    AccessItem::Row(
        AccessRow::MonoAudio,
        "Mono audio",
        "Combine left and right channels",
    ),
];
fn heading_height() -> i32 {
    40 + grow(TextStyle::Semibold14)
}
fn row_height() -> i32 {
    56 + grow(TextStyle::Regular14) + grow(TextStyle::Regular12)
}
/// Total height of the Accessibility list.
pub fn access_content() -> i32 {
    ACCESS_ITEMS
        .iter()
        .map(|i| match i {
            AccessItem::Heading(_) => heading_height(),
            AccessItem::Row(..) => row_height(),
        })
        .sum()
}
/// How far the Accessibility list can scroll.
pub fn access_scroll_max(layout: Layout) -> i32 {
    (access_content() - page_frame(layout).list.height() as i32 + 8).max(0)
}
/// Each Accessibility row's rect at the list's scroll offset (unclipped).
pub fn access_rows(layout: Layout, scroll: i32) -> alloc::vec::Vec<(AccessRow, Rect)> {
    let list = page_frame(layout).list;
    let mut y = list.y() - scroll;
    let mut out = alloc::vec::Vec::new();
    for item in &ACCESS_ITEMS {
        match item {
            AccessItem::Heading(_) => y += heading_height(),
            AccessItem::Row(row, ..) => {
                let h = row_height();
                out.push((
                    *row,
                    Rect::new(list.x() + 8, y, list.width() - 16, h as u32).unwrap(),
                ));
                y += h;
            }
        }
    }
    out
}
/// The toggle switch inside a list row (40 x 20, right-aligned).
pub fn row_toggle(row: Rect) -> Rect {
    Rect::new(
        row.right() - 56,
        row.y() + row.height() as i32 / 2 - 10,
        40,
        20,
    )
    .unwrap()
}

/// Project page rows (Win+P).
pub const PROJECT_MODES: [&str; 4] = [
    "PC screen only",
    "Duplicate",
    "Extend",
    "Second screen only",
];
pub fn project_row(layout: Layout, index: usize) -> Rect {
    let list = page_frame(layout).list;
    let h = 52 + grow(TextStyle::Regular14);
    Rect::new(
        list.x() + 8,
        list.y() + 6 + index as i32 * (h + 4),
        list.width() - 16,
        h as u32,
    )
    .unwrap()
}
/// Cast page: "Available displays" heading and the help link.
pub fn cast_rows(layout: Layout) -> (Rect, Rect) {
    let list = page_frame(layout).list;
    let g = grow(TextStyle::Semibold14);
    let heading = Rect::new(list.x() + 24, list.y() + 12, 300, (28 + g) as u32).unwrap();
    let link = Rect::new(
        list.x() + 24,
        heading.bottom() + 8,
        200,
        paint::text_grow(28, TextStyle::Regular14),
    )
    .unwrap();
    (heading, link)
}
/// VPN page rows: each profile, the first expanded with its button.
pub fn vpn_row(layout: Layout, index: usize) -> Rect {
    let list = page_frame(layout).list;
    let g = grow(TextStyle::Regular14);
    let expanded = 104 + g;
    let collapsed = 52 + g;
    let y = if index == 0 {
        list.y() + 4
    } else {
        list.y() + 4 + expanded + 4 + (index as i32 - 1) * (collapsed + 4)
    };
    Rect::new(
        list.x() + 8,
        y,
        list.width() - 16,
        if index == 0 { expanded } else { collapsed } as u32,
    )
    .unwrap()
}
pub fn vpn_button(layout: Layout) -> Rect {
    let r = vpn_row(layout, 0);
    Rect::new(r.right() - 236, r.bottom() - 48, 220, 32).unwrap()
}

/// Hit-test a sub-page (not the main page or Sound output).
pub fn hit(state: &State, layout: Layout, x: i32, y: i32) -> Option<Action> {
    let f = page_frame(layout);
    if f.back.contains(x, y) {
        return Some(Action::Back);
    }
    if f.more.contains(x, y) {
        return Some(Action::MoreSettings);
    }
    if !f.list.contains(x, y) {
        return None;
    }
    match state.quick_page {
        QuickPage::Accessibility => access_rows(layout, state.list_scroll)
            .into_iter()
            .find(|(_, r)| r.contains(x, y))
            .filter(|(row, _)| access_enabled(state, *row))
            .map(|(row, _)| Action::AccessToggle(row)),
        QuickPage::Project => (0..PROJECT_MODES.len())
            .find(|i| project_row(layout, *i).contains(x, y))
            .map(|i| Action::ProjectMode(i as u8)),
        QuickPage::Cast => cast_rows(layout)
            .1
            .contains(x, y)
            .then_some(Action::CastHelp),
        QuickPage::Vpn => {
            if !state.vpns.is_empty() && vpn_button(layout).contains(x, y) {
                return Some(Action::VpnConnect(0));
            }
            (0..state.vpns.len())
                .find(|i| vpn_row(layout, *i).contains(x, y))
                .map(|i| Action::VpnSelect(i as u8))
        }
        QuickPage::Main | QuickPage::SoundOutput => None,
    }
}
/// Rows this profile can act on: Narrator and Live captions need a speech
/// engine Doors does not have; Mono audio needs an output device.
pub fn access_enabled(state: &State, row: AccessRow) -> bool {
    match row {
        AccessRow::Magnifier | AccessRow::ColorFilters => true,
        AccessRow::MonoAudio => state.audio.is_some_and(|a| a.present),
        AccessRow::Narrator | AccessRow::LiveCaptions => false,
    }
}
pub fn access_on(state: &State, row: AccessRow) -> bool {
    use door_service_protocol::preferences::vision as v;
    match row {
        AccessRow::Magnifier => state.magnifier,
        AccessRow::ColorFilters => state.vision & v::FILTER != 0,
        AccessRow::MonoAudio => state.audio.is_some_and(|a| a.output.mono),
        AccessRow::Narrator | AccessRow::LiveCaptions => false,
    }
}

/// The WinUI toggle switch: an outlined pill with a knob when off, an
/// accent pill with a contrasting knob when on.
pub fn draw_toggle(
    c: &mut Canvas<'_>,
    r: Rect,
    on: bool,
    enabled: bool,
    hovered: bool,
    palette: &Palette,
) {
    let cy = r.y() + r.height() as i32 / 2;
    let knob = if hovered && enabled { 7 } else { 6 };
    if on {
        let fill = if enabled {
            palette.accent
        } else {
            palette.text_disabled
        };
        paint::rounded(c, r, r.height() / 2, fill);
        paint::circle(c, r.right() - 10, cy, knob, palette.accent_text);
    } else {
        let edge = if enabled {
            palette.text_secondary
        } else {
            palette.text_disabled
        };
        let body = if hovered && enabled {
            palette.subtle_hover
        } else {
            paint::black(0)
        };
        if body[3] != 0 {
            paint::rounded(c, r, r.height() / 2, body);
        }
        paint::ring(c, r, r.height() / 2, edge);
        paint::circle(c, r.x() + 10, cy, knob, edge);
    }
}
/// The WinUI slider: rail, accent fill to `value`, and the ringed thumb
/// whose inner dot grows on hover and shrinks while dragged.
#[allow(clippy::too_many_arguments)]
pub fn draw_slider(
    c: &mut Canvas<'_>,
    palette: &Palette,
    track: Rect,
    value: u8,
    fill: [u8; 4],
    hovered: bool,
    dragging: bool,
) {
    let cy = track.y() + track.height() as i32 / 2;
    let rail = if palette.contrast {
        palette.text_disabled
    } else if palette.dark {
        paint::rgb(158, 158, 158)
    } else {
        paint::rgb(134, 134, 134)
    };
    let w = track.width() as i32 * i32::from(value.min(100)) / 100;
    paint::rounded(
        c,
        Rect::new(track.x(), cy - 2, track.width(), 4).unwrap(),
        2,
        rail,
    );
    if w > 0 {
        paint::rounded(
            c,
            Rect::new(track.x(), cy - 2, w as u32, 4).unwrap(),
            2,
            fill,
        );
    }
    let thumb = Rect::new(track.x() + w - 10, cy - 10, 20, 20).unwrap();
    let ring = if palette.dark {
        paint::rgb(69, 69, 69)
    } else {
        paint::rgb(255, 255, 255)
    };
    paint::framed(c, thumb, 10, palette.surface_border, ring);
    let inner = if dragging {
        10
    } else if hovered {
        14
    } else {
        12
    };
    let o = (20 - inner) / 2;
    paint::rounded(
        c,
        Rect::new(thumb.x() + o, thumb.y() + o, inner as u32, inner as u32).unwrap(),
        inner as u32 / 2,
        fill,
    );
}

/// Battery state for the footer and the taskbar (None on desktops).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
    pub saver: bool,
}
/// The 26H2 coloured battery: green while charging, yellow with Energy
/// saver, red at 20% and below, otherwise the text colour.
pub fn battery_fill(b: Battery, palette: &Palette) -> [u8; 4] {
    if b.charging {
        paint::rgb(108, 203, 95)
    } else if b.saver {
        paint::rgb(252, 225, 0)
    } else if b.percent <= 20 {
        paint::rgb(255, 99, 97)
    } else {
        palette.text
    }
}
/// Draw the battery glyph at (x, y) (16 px box): outline, fill by charge
/// and, while charging, a bolt cut out of the fill.
pub fn draw_battery(c: &mut Canvas<'_>, x: i32, y: i32, b: Battery, palette: &Palette) {
    paint::glyph(
        c,
        x,
        y,
        16,
        crate::shell_mask!("shell-battery-outline", 16),
        palette.text,
    );
    // Inside of the Fluent outline (EBA0 at 16 px): x 2..13, y 5..11.
    let inner_w = 10 * u32::from(b.percent.min(100)) / 100;
    if inner_w > 0 {
        paint::rounded(
            c,
            Rect::new(x + 2, y + 6, inner_w, 5).unwrap(),
            1,
            battery_fill(b, palette),
        );
    }
    if b.charging {
        let cut = if palette.dark {
            paint::rgb(32, 32, 32)
        } else {
            paint::rgb(243, 243, 243)
        };
        for (dx, dy, w) in [(7, 5, 2), (6, 6, 2), (5, 7, 4), (7, 8, 2), (6, 9, 2)] {
            paint::rect(c, x + dx, y + dy, w, 1, cut);
        }
    }
}

/// The second tile page's glyphs.
pub fn second_masks() -> [paint::Mask; 6] {
    [
        crate::shell_mask!("shell-live-captions", 16),
        crate::shell_mask!("shell-night-light", 16),
        crate::shell_mask!("shell-hotspot", 16),
        crate::shell_mask!("shell-nearby", 16),
        crate::shell_mask!("shell-cast", 16),
        crate::shell_mask!("shell-project", 16),
    ]
}

/// Draw the page header: back button, title and optional key chips
/// ("Win" + letter, as the Cast and Project pages show their shortcut).
#[allow(clippy::too_many_arguments)]
fn draw_header(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    layout: Layout,
    palette: &Palette,
    pointer: (i32, i32),
    dy: i32,
    title: &str,
    key: Option<&str>,
) {
    let f = page_frame(layout);
    if f.back.contains(pointer.0, pointer.1) {
        paint::rounded(c, shifted(f.back, dy), 4, palette.subtle_hover);
    }
    paint::glyph_in(
        c,
        shifted(f.back, dy),
        16,
        crate::shell_mask!("shell-back", 16),
        palette.text,
    );
    let t = shifted(f.title, dy);
    paint::text_row(
        c,
        fonts,
        t.x(),
        t.y(),
        t.height(),
        title,
        TextStyle::Regular14,
        palette.text,
        t.width(),
    );
    if let Some(key) = key {
        let mut x = t.x() + paint::measure(fonts, title, TextStyle::Regular14) as i32 + 10;
        let chip = |c: &mut Canvas<'_>, x: i32| {
            let r = Rect::new(x, t.y() + 6, 20, 20).unwrap();
            paint::rounded(c, r, 3, palette.subtle_hover);
            paint::ring(c, r, 3, palette.surface_border);
            r
        };
        let win = chip(c, x);
        // The Windows key: four small squares.
        for (ox, oy) in [(5, 5), (10, 5), (5, 10), (10, 10)] {
            paint::rect(c, win.x() + ox, win.y() + oy, 4, 4, palette.text);
        }
        x += 24;
        let k = chip(c, x);
        paint::text_span(
            c,
            fonts,
            k.x(),
            k.width(),
            k.y(),
            k.height(),
            key,
            TextStyle::Regular12,
            palette.text,
        );
    }
}
fn draw_more(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    layout: Layout,
    palette: &Palette,
    pointer: (i32, i32),
    dy: i32,
    label: &str,
) {
    let f = page_frame(layout);
    let more = shifted(f.more, dy);
    paint::rect(
        c,
        more.x(),
        more.y(),
        more.width(),
        1,
        palette.surface_divider,
    );
    if f.more.contains(pointer.0, pointer.1) {
        paint::rect(
            c,
            more.x(),
            more.y() + 1,
            more.width(),
            more.height() - 1,
            palette.subtle_hover,
        );
    }
    paint::text_row(
        c,
        fonts,
        more.x() + 23,
        more.y(),
        more.height(),
        label,
        TextStyle::Regular14,
        palette.text,
        more.width() - 46,
    );
}
/// A selectable list row: subtle plate when selected or hovered, and the
/// accent pill at the left of the selected one.
fn select_plate(c: &mut Canvas<'_>, r: Rect, selected: bool, hovered: bool, palette: &Palette) {
    if selected || hovered {
        paint::rounded(
            c,
            r,
            4,
            if hovered {
                palette.subtle_hover
            } else {
                palette.subtle_pressed
            },
        );
    }
    if selected {
        paint::rounded(
            c,
            Rect::new(r.x(), r.y() + r.height() as i32 / 2 - 8, 3, 16).unwrap(),
            2,
            palette.accent,
        );
    }
}

/// Draw the current sub-page (Accessibility, VPN, Cast, Project).
#[allow(clippy::too_many_arguments)]
pub fn draw(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    state: &State,
    layout: Layout,
    palette: &Palette,
    pointer: (i32, i32),
    dy: i32,
) {
    let page = state.quick_page;
    let (title, key) = match page {
        QuickPage::Accessibility => ("Accessibility", None),
        QuickPage::Vpn => ("VPN", None),
        QuickPage::Cast => ("Cast", Some("K")),
        QuickPage::Project => ("Project", Some("P")),
        QuickPage::Main | QuickPage::SoundOutput => return,
    };
    draw_header(c, fonts, layout, palette, pointer, dy, title, key);
    let f = page_frame(layout);
    let list = shifted(f.list, dy);
    let saved = c.clip_rect();
    let scale = c.scale();
    if let Some(clip) = saved.and_then(|cl| cl.intersection(paint::physical(list, scale))) {
        c.set_clip(clip);
    }
    let hover = |r: Rect| f.list.contains(pointer.0, pointer.1) && r.contains(pointer.0, pointer.1);
    match page {
        QuickPage::Accessibility => {
            let mut y = f.list.y() - state.list_scroll;
            let rows = access_rows(layout, state.list_scroll);
            let mut row_index = 0;
            for item in &ACCESS_ITEMS {
                match item {
                    AccessItem::Heading(text) => {
                        let h = heading_height();
                        paint::text_row(
                            c,
                            fonts,
                            list.x() + 24,
                            y + dy + 8,
                            (h - 8) as u32,
                            text,
                            TextStyle::Semibold14,
                            palette.text,
                            list.width() - 48,
                        );
                        y += h;
                    }
                    AccessItem::Row(row, name, detail) => {
                        let (_, r0) = rows[row_index];
                        row_index += 1;
                        y += row_height();
                        let enabled = access_enabled(state, *row);
                        let on = access_on(state, *row);
                        let r = shifted(r0, dy);
                        if enabled && hover(r0) {
                            paint::rounded(c, r, 4, palette.subtle_hover);
                        }
                        let fg = if enabled {
                            palette.text
                        } else {
                            palette.text_disabled
                        };
                        let mask = match row {
                            AccessRow::Magnifier => crate::shell_mask!("shell-magnifier", 16),
                            AccessRow::Narrator => crate::shell_mask!("shell-narrator", 16),
                            AccessRow::ColorFilters => {
                                crate::shell_mask!("shell-color-filters", 16)
                            }
                            AccessRow::LiveCaptions => {
                                crate::shell_mask!("shell-live-captions", 16)
                            }
                            AccessRow::MonoAudio => crate::shell_mask!("shell-volume", 16),
                        };
                        paint::glyph(c, r.x() + 16, r.y() + 12, 16, mask, fg);
                        let g14 = grow(TextStyle::Regular14);
                        let text_w = (r.width() as i32 - 160).max(1) as u32;
                        paint::text_row(
                            c,
                            fonts,
                            r.x() + 48,
                            r.y() + 6,
                            paint::text_grow(22, TextStyle::Regular14),
                            name,
                            TextStyle::Regular14,
                            fg,
                            text_w,
                        );
                        let detail = paint::ellipsize(fonts, detail, TextStyle::Regular12, text_w);
                        paint::text_row(
                            c,
                            fonts,
                            r.x() + 48,
                            r.y() + 28 + g14,
                            paint::text_grow(18, TextStyle::Regular12),
                            &detail,
                            TextStyle::Regular12,
                            if enabled {
                                palette.text_secondary
                            } else {
                                palette.text_disabled
                            },
                            text_w,
                        );
                        let t = row_toggle(r);
                        paint::text_row(
                            c,
                            fonts,
                            t.x() - 36,
                            t.y() - 2,
                            24,
                            if on { "On" } else { "Off" },
                            TextStyle::Regular14,
                            fg,
                            30,
                        );
                        draw_toggle(c, t, on, enabled, hover(r0), palette);
                    }
                }
            }
            // A thin scroll indicator while the list overflows.
            let max = access_scroll_max(layout);
            if max > 0 {
                let track = list.height() as i32 - 16;
                let thumb = (track * list.height() as i32 / access_content()).max(24);
                let top = list.y() + 8 + (track - thumb) * state.list_scroll / max;
                paint::rounded(
                    c,
                    Rect::new(list.right() - 6, top, 2, thumb as u32).unwrap(),
                    1,
                    palette.text_secondary,
                );
            }
        }
        QuickPage::Project => {
            for (i, name) in PROJECT_MODES.iter().enumerate() {
                let r0 = project_row(layout, i);
                let r = shifted(r0, dy);
                select_plate(c, r, i == usize::from(state.projection), hover(r0), palette);
                // Laptop and second-screen pair; lit parts follow the mode.
                let lit = |on: bool| {
                    if on {
                        palette.text
                    } else {
                        palette.text_secondary
                    }
                };
                let (laptop_on, screen_on) = match i {
                    0 => (true, false),
                    3 => (false, true),
                    _ => (true, true),
                };
                let cy = r.y() + r.height() as i32 / 2;
                paint::glyph(
                    c,
                    r.x() + 16,
                    cy - 8,
                    16,
                    crate::shell_mask!("shell-laptop", 16),
                    lit(laptop_on),
                );
                let screen = Rect::new(r.x() + 34, cy - 8, 14, 12).unwrap();
                paint::ring(c, screen, 2, lit(screen_on));
                if i == 1 || i == 2 {
                    // Duplicate mirrors, Extend continues: a corner mark.
                    let mark = if i == 1 { 0 } else { 6 };
                    paint::rect(c, screen.x() + 3 + mark, screen.y() + 3, 4, 1, lit(true));
                }
                paint::text_row(
                    c,
                    fonts,
                    r.x() + 64,
                    r.y(),
                    r.height(),
                    name,
                    TextStyle::Regular14,
                    palette.text,
                    r.width() - 72,
                );
            }
        }
        QuickPage::Cast => {
            // Searching: the indeterminate progress bar under the header.
            let span = 120;
            let travel = list.width() as i32 + span;
            let t = (state.now / 1000 % 1600) as i32;
            let x = list.x() - span + travel * t / 1600;
            let bar = Rect::new(list.x(), list.y(), list.width(), 3).unwrap();
            if let Some(seg) = Rect::new(x, list.y(), span as u32, 3)
                .ok()
                .and_then(|s| s.intersection(bar))
            {
                paint::rounded(c, seg, 1, palette.accent);
            }
            let (heading, link) = cast_rows(layout);
            let h = shifted(heading, dy);
            paint::text_row(
                c,
                fonts,
                h.x(),
                h.y(),
                h.height(),
                "Available displays",
                TextStyle::Semibold14,
                palette.text,
                h.width(),
            );
            let l = shifted(link, dy);
            paint::text_row(
                c,
                fonts,
                l.x(),
                l.y(),
                l.height(),
                "Can\u{2019}t find a device?",
                TextStyle::Regular14,
                palette.accent,
                l.width(),
            );
            if hover(link) {
                let w = paint::measure(fonts, "Can\u{2019}t find a device?", TextStyle::Regular14);
                paint::rect(c, l.x(), l.bottom() - 5, w, 1, palette.accent);
            }
        }
        QuickPage::Vpn => {
            for (i, vpn) in state.vpns.iter().enumerate() {
                let r0 = vpn_row(layout, i);
                let r = shifted(r0, dy);
                select_plate(c, r, i == 0, hover(r0), palette);
                paint::glyph(
                    c,
                    r.x() + 16,
                    r.y() + 18,
                    16,
                    crate::shell_mask!("shell-vpn", 16),
                    palette.text,
                );
                paint::text_row(
                    c,
                    fonts,
                    r.x() + 52,
                    r.y() + 10,
                    paint::text_grow(32, TextStyle::Regular14),
                    &vpn.name,
                    TextStyle::Regular14,
                    palette.text,
                    r.width() - 60,
                );
                let status = if vpn.connected {
                    "Connected"
                } else if vpn.connecting {
                    "Connecting…"
                } else {
                    ""
                };
                if !status.is_empty() {
                    paint::text_row(
                        c,
                        fonts,
                        r.x() + 52,
                        r.y() + 10 + paint::text_grow(20, TextStyle::Regular14) as i32,
                        paint::text_grow(16, TextStyle::Regular12),
                        status,
                        TextStyle::Regular12,
                        palette.text_secondary,
                        r.width() - 60,
                    );
                }
                if i == 0 {
                    let b0 = vpn_button(layout);
                    let b = shifted(b0, dy);
                    let fill = if b0.contains(pointer.0, pointer.1) {
                        paint::alpha(palette.accent, 230)
                    } else {
                        palette.accent
                    };
                    paint::rounded(c, b, 4, fill);
                    paint::text_span(
                        c,
                        fonts,
                        b.x(),
                        b.width(),
                        b.y(),
                        b.height(),
                        if vpn.connected {
                            "Disconnect"
                        } else if vpn.connecting {
                            "Cancel"
                        } else {
                            "Connect"
                        },
                        TextStyle::Regular14,
                        palette.accent_text,
                    );
                }
            }
        }
        QuickPage::Main | QuickPage::SoundOutput => (),
    }
    if let Some(saved) = saved {
        c.set_clip(saved);
    }
    draw_more(c, fonts, layout, palette, pointer, dy, more_label(page));
}

/// One VPN profile the network service reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VpnEntry {
    pub name: alloc::string::String,
    pub connected: bool,
    /// Resolving the server or waiting for the first handshake.
    pub connecting: bool,
}
