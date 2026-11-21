//! Shell presentation of door.shell state (spec §60): icons of extra
//! taskbar buttons (registered and Windows apps), attention and progress on
//! taskbar buttons, notification toasts (bottom right, Windows 11 cards) and
//! the notification centre's cards, plus the hover thumbnail preview.
//! Geometry is shared by painting and hit testing; nothing here owns state.
extern crate alloc;
use crate::apps::{self, App};
use crate::paint::{self, Palette};
use crate::shell_link::{Notice, Notices};
use crate::taskbar::Layout;
use alloc::{string::String, vec::Vec};
use door_graphics_core::{Canvas, Rect};
use door_graphics_text::{FontAtlas, TextStyle};
use door_service_protocol::{desktop::Edge, shell as s};

/// The catalog app whose bundled artwork an icon reference names.
pub fn icon_app(icon: u16) -> Option<App> {
    match icon {
        s::icon::FILES => Some(App::Files),
        s::icon::NOTES => Some(App::Notes),
        s::icon::SETTINGS => Some(App::Settings),
        s::icon::TERMINAL => Some(App::Terminal),
        s::icon::PREVIEW => Some(App::Preview),
        s::icon::TASK_MANAGER => Some(App::Monitor),
        _ => None,
    }
}
/// The icon reference of a catalog app.
pub const fn app_icon(app: App) -> u16 {
    match app {
        App::Files => s::icon::FILES,
        App::Notes => s::icon::NOTES,
        App::Settings => s::icon::SETTINGS,
        App::Terminal => s::icon::TERMINAL,
        App::Preview => s::icon::PREVIEW,
        App::Monitor => s::icon::TASK_MANAGER,
    }
}
/// Draw icon reference `icon` at a logical position and size.
pub fn draw_icon(c: &mut Canvas<'_>, x: i32, y: i32, size: u32, icon: u16) {
    match icon {
        _ if icon_app(icon).is_some() => apps::draw_icon(c, x, y, icon_app(icon).unwrap(), size),
        s::icon::GALLERY | s::icon::SYSTEM => apps::draw_logo(c, x, y, size),
        s::icon::CAPTIONS => apps::draw_tool_icon(c, x, y, apps::Tool::Captions, size),
        s::icon::CALCULATOR => apps::draw_tool_icon(c, x, y, apps::Tool::Calculator, size),
        s::icon::CLOCK => apps::draw_tool_icon(c, x, y, apps::Tool::Clock, size),
        s::icon::PAINT => apps::draw_tool_icon(c, x, y, apps::Tool::Paint, size),
        s::icon::SNIPPING_TOOL => apps::draw_tool_icon(c, x, y, apps::Tool::SnippingTool, size),
        s::icon::WEATHER => apps::draw_tool_icon(c, x, y, apps::Tool::Weather, size),
        s::icon::MAGNIFIER => apps::draw_tool_icon(c, x, y, apps::Tool::Magnifier, size),
        s::icon::NARRATOR => apps::draw_tool_icon(c, x, y, apps::Tool::Narrator, size),
        s::icon::ON_SCREEN_KEYBOARD => {
            apps::draw_tool_icon(c, x, y, apps::Tool::OnScreenKeyboard, size)
        }
        s::icon::VOICE_ACCESS => apps::draw_tool_icon(c, x, y, apps::Tool::VoiceAccess, size),
        _ => draw_window_app(c, x, y, size),
    }
}
/// Original artwork for a program without bundled art: a small window
/// (accent caption band over a light client area) on the icon grid.
fn draw_window_app(c: &mut Canvas<'_>, x: i32, y: i32, size: u32) {
    let inset = (size / 8).max(1) as i32;
    let side = size.saturating_sub(2 * inset as u32).max(4);
    let Ok(frame) = Rect::new(x + inset, y + inset + inset / 2, side, side - side / 8) else {
        return;
    };
    let radius = (size / 10).max(1);
    paint::rounded(c, frame, radius, paint::rgb(0, 95, 184));
    if let Some(client) = paint::inset(frame, 1, 1) {
        let band = (client.height() / 4).max(2);
        if let Ok(body) = Rect::new(
            client.x(),
            client.y() + band as i32,
            client.width(),
            client.height() - band,
        ) {
            paint::rounded(c, body, radius.saturating_sub(1), paint::rgb(250, 250, 250));
        }
    }
}
/// Windows 11 attention request: the button turns a light orange plate.
pub fn attention_plate(c: &mut Canvas<'_>, r: Rect, palette: &Palette) {
    if palette.contrast {
        paint::outline(c, r, palette.accent, 2);
        return;
    }
    let fill = if palette.dark {
        paint::rgb(92, 58, 38)
    } else {
        paint::rgb(252, 225, 208)
    };
    paint::rounded(c, r, 4, fill);
}
pub const ATTENTION_PILL: [u8; 4] = [12, 99, 247, 255];
/// Progress colors (normal, paused, error) per theme, as BGRA.
fn progress_color(state: u8, dark: bool) -> [u8; 4] {
    match (state, dark) {
        (s::progress::PAUSED, false) => paint::rgb(157, 93, 0),
        (s::progress::PAUSED, true) => paint::rgb(252, 225, 0),
        (s::progress::ERROR, false) => paint::rgb(196, 43, 28),
        (s::progress::ERROR, true) => paint::rgb(255, 153, 164),
        (_, false) => paint::rgb(15, 123, 15),
        (_, true) => paint::rgb(108, 203, 95),
    }
}
/// Taskbar progress in place of the running indicator: a track across the
/// bottom of the button filled to `value` (0..=1000); indeterminate shows a
/// centered segment.
pub fn progress_bar(
    c: &mut Canvas<'_>,
    r: Rect,
    edge: Edge,
    state: u8,
    value: u16,
    palette: &Palette,
) {
    let vertical = matches!(edge, Edge::Left | Edge::Right);
    let length = if vertical { r.height() } else { r.width() }.saturating_sub(12);
    let (x0, y0) = match edge {
        Edge::Left => (r.x() + 1, r.y() + 6),
        Edge::Right => (r.right() - 4, r.y() + 6),
        Edge::Top => (r.x() + 6, r.y() + 1),
        Edge::Bottom => (r.x() + 6, r.bottom() - 4),
    };
    let bar = |c: &mut Canvas<'_>, from: u32, len: u32, color: [u8; 4]| {
        let rect = if vertical {
            Rect::new(x0, y0 + from as i32, 3, len.max(1))
        } else {
            Rect::new(x0 + from as i32, y0, len.max(1), 3)
        };
        if let Ok(rect) = rect {
            paint::rounded(c, rect, 1, color);
        }
    };
    let track = if palette.contrast {
        palette.text_secondary
    } else if palette.dark {
        paint::white(40)
    } else {
        paint::black(40)
    };
    bar(c, 0, length, track);
    let color = if palette.contrast {
        palette.accent
    } else {
        progress_color(state, palette.dark)
    };
    if state == s::progress::INDETERMINATE {
        bar(c, length / 3, length / 3, color);
    } else {
        let filled = (u64::from(length) * u64::from(value.min(s::MAX_PROGRESS)) / 1000) as u32;
        if filled > 0 {
            bar(c, 0, filled, color);
        }
    }
}
/// Unread count badge on the notification bell (Windows 11: a small accent
/// circle with the number).
pub fn draw_badge(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    cx: i32,
    cy: i32,
    count: u8,
    palette: &Palette,
) {
    paint::circle(c, cx, cy, 8, palette.accent);
    let mut text = String::new();
    if count > 9 {
        text.push_str("9+");
    } else {
        text.push(char::from(b'0' + count));
    }
    paint::text_center(
        c,
        fonts,
        cx,
        cy - 8,
        16,
        &text,
        TextStyle::Regular12,
        palette.accent_text,
        16,
    );
}

// --- Notification cards -------------------------------------------------

pub const CARD_GAP: i32 = 8;
const PAD: i32 = 12;
const HEADER: u32 = 16;
const TITLE: u32 = 20;
const BODY_LINE: u32 = 20;
const BUTTON: u32 = 32;
/// Fixed card height: header, title, two body lines and, when the
/// notification has actions, a button row.
pub fn card_height(n: &Notice) -> u32 {
    let base = PAD as u32 * 2 + HEADER + 8 + TITLE + 2 * BODY_LINE;
    if n.actions.is_empty() {
        base
    } else {
        base + 8 + BUTTON
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub card: Rect,
    pub icon: Rect,
    pub close: Rect,
    pub title: Rect,
    pub body: Rect,
    pub buttons: Vec<Rect>,
}
pub fn card(r: Rect, n: &Notice) -> Card {
    let x = r.x() + PAD;
    let w = r.width().saturating_sub(2 * PAD as u32).max(1);
    let y = r.y() + PAD;
    let icon = Rect::new(x, y, HEADER, HEADER).unwrap();
    let close = Rect::new(r.right() - PAD - 24, y - 4, 24, 24).unwrap();
    let title = Rect::new(x, y + HEADER as i32 + 8, w, TITLE).unwrap();
    let body = Rect::new(x, title.bottom(), w, 2 * BODY_LINE).unwrap();
    let mut buttons = Vec::new();
    let count = n.actions.len() as u32;
    if count > 0 {
        let each = (w - 8 * (count - 1)) / count;
        for i in 0..count {
            buttons.push(
                Rect::new(
                    x + (i * (each + 8)) as i32,
                    body.bottom() + 8,
                    each.max(1),
                    BUTTON,
                )
                .unwrap(),
            );
        }
    }
    Card {
        card: r,
        icon,
        close,
        title,
        body,
        buttons,
    }
}
/// Greedy word wrap into at most two lines; the second is ellipsized.
pub fn wrap2(fonts: &FontAtlas, text: &str, style: TextStyle, width: u32) -> (String, String) {
    if paint::measure(fonts, text, style) <= width {
        return (text.into(), String::new());
    }
    let mut first = String::new();
    let mut rest = text;
    for (i, _) in text.match_indices(' ') {
        if paint::measure(fonts, &text[..i], style) > width {
            break;
        }
        first = text[..i].into();
        rest = text[i + 1..].trim_start();
    }
    if first.is_empty() {
        return (paint::ellipsize(fonts, text, style, width), String::new());
    }
    (first, paint::ellipsize(fonts, rest, style, width))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeHit {
    /// The card body: open what it is about.
    Body(u32),
    /// Action button `n` (0-based).
    Button(u32, u8),
    Close(u32),
    ClearAll,
    Dnd,
    /// Inside the surface but on no control.
    Inside,
}
fn hit_card(k: &Card, id: u32, x: i32, y: i32) -> Option<NoticeHit> {
    if !k.card.contains(x, y) {
        return None;
    }
    if k.close.contains(x, y) {
        return Some(NoticeHit::Close(id));
    }
    if let Some(i) = k.buttons.iter().position(|b| b.contains(x, y)) {
        return Some(NoticeHit::Button(id, i as u8));
    }
    Some(NoticeHit::Body(id))
}
#[allow(clippy::too_many_arguments)]
fn draw_card_content(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    k: &Card,
    n: &Notice,
    palette: &Palette,
    pointer: (i32, i32),
    show_time: bool,
) {
    draw_icon(c, k.icon.x(), k.icon.y(), HEADER, n.icon);
    let header_w = k.close.x() - k.icon.right() - 16;
    paint::text_row(
        c,
        fonts,
        k.icon.right() + 8,
        k.icon.y(),
        HEADER,
        &n.source,
        TextStyle::Regular12,
        palette.text_secondary,
        header_w.max(1) as u32,
    );
    if show_time && !n.time.is_empty() {
        paint::text_right(
            c,
            fonts,
            k.close.x() - 4,
            k.icon.y(),
            HEADER,
            &n.time,
            TextStyle::Regular12,
            palette.text_secondary,
        );
    }
    if k.close.contains(pointer.0, pointer.1) {
        paint::rounded(c, k.close, 4, palette.subtle_hover);
    }
    paint::glyph_in(
        c,
        k.close,
        12,
        crate::shell_mask!("chrome-close", 12),
        palette.text,
    );
    paint::text_row(
        c,
        fonts,
        k.title.x(),
        k.title.y(),
        k.title.height(),
        &paint::ellipsize(fonts, &n.title, TextStyle::Semibold14, k.title.width()),
        TextStyle::Semibold14,
        palette.text,
        k.title.width(),
    );
    let (one, two) = wrap2(fonts, &n.body, TextStyle::Regular14, k.body.width());
    for (i, line) in [one, two].iter().enumerate() {
        if !line.is_empty() {
            paint::text_row(
                c,
                fonts,
                k.body.x(),
                k.body.y() + (i as u32 * BODY_LINE) as i32,
                BODY_LINE,
                line,
                TextStyle::Regular14,
                palette.text_secondary,
                k.body.width(),
            );
        }
    }
    for (b, label) in k.buttons.iter().zip(&n.actions) {
        let hovered = b.contains(pointer.0, pointer.1);
        let fill = if hovered {
            palette.subtle_hover
        } else {
            palette.card
        };
        paint::framed(c, *b, 4, palette.surface_border, fill);
        paint::text_in(c, fonts, *b, label, TextStyle::Regular14, palette.text);
    }
}

// --- Toasts -------------------------------------------------------------

pub const TOAST_WIDTH: u32 = 364;
/// Toasts visible at once (newest at the bottom).
pub const MAX_TOASTS: usize = 3;
/// Toast cards bottom-right above the taskbar, newest lowest.
pub fn toasts(layout: Layout, notices: &Notices) -> Vec<(usize, Rect)> {
    let work = layout.work_area();
    let x = work.right() - 12 - TOAST_WIDTH as i32;
    let mut bottom = work.bottom() - 12;
    let mut out = Vec::new();
    for (i, n) in notices.list.iter().enumerate().filter(|(_, n)| n.toast) {
        if out.len() == MAX_TOASTS {
            break;
        }
        let h = card_height(n);
        let top = bottom - h as i32;
        if top < work.y() + 12 {
            break;
        }
        out.push((i, Rect::new(x, top, TOAST_WIDTH, h).unwrap()));
        bottom = top - CARD_GAP;
    }
    out
}
pub fn toast_bounds(layout: Layout, notices: &Notices) -> Option<Rect> {
    toasts(layout, notices)
        .into_iter()
        .map(|(_, r)| r)
        .reduce(paint::union)
}
pub fn hit_toasts(layout: Layout, notices: &Notices, x: i32, y: i32) -> Option<NoticeHit> {
    toasts(layout, notices).into_iter().find_map(|(i, r)| {
        let n = &notices.list[i];
        hit_card(&card(r, n), n.id, x, y)
    })
}
fn surface(c: &mut Canvas<'_>, r: Rect, palette: &Palette) {
    paint::shadow(c, r, 8, palette.dark);
    paint::framed(c, r, 8, palette.surface_border, palette.surface);
}
pub fn draw_toasts(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    layout: Layout,
    notices: &Notices,
    dark: bool,
    pointer: (i32, i32),
) {
    let palette = Palette::new(dark);
    for (i, r) in toasts(layout, notices) {
        let n = &notices.list[i];
        surface(c, r, &palette);
        draw_card_content(c, fonts, &card(r, n), n, &palette, pointer, false);
    }
}

// --- Notification centre ------------------------------------------------

const CENTRE_HEADER: u32 = 48;
/// Centre surface (above the calendar) with the cards that fit, newest
/// first; `None` when the output is too short for it.
pub struct Centre {
    pub rect: Rect,
    pub heading: Rect,
    pub clear_all: Option<Rect>,
    pub dnd: Rect,
    pub cards: Vec<(usize, Card)>,
}
pub fn centre(layout: Layout, notices: &Notices) -> Option<Centre> {
    let empty = crate::tray::notifications_rect(layout)?;
    let c = crate::tray::calendar_rect(layout);
    let top = layout.work_area().y() + 12;
    let rect = if notices.list.is_empty() {
        empty
    } else {
        let room = (c.y() - 12 - top).max(empty.height() as i32) as u32;
        let mut h = CENTRE_HEADER + 4;
        for n in &notices.list {
            let next = h + card_height(n) + CARD_GAP as u32;
            if next > room {
                break;
            }
            h = next;
        }
        let h = h.max(empty.height());
        Rect::new(empty.x(), c.y() - 12 - h as i32, empty.width(), h).ok()?
    };
    // 26H2 header (`reports/artifacts/reference-26h2/shell-notifications.uia.tsv`):
    // "Notifications" at x 17, the 40 x 40 Do not disturb toggle, then
    // "Clear all" 62 x 24 ending 17 px from the right edge.
    let heading = Rect::new(rect.x() + 17, rect.y() + 12, 200, 24).unwrap();
    let clear_all = (!notices.list.is_empty())
        .then(|| Rect::new(rect.right() - 17 - 62, rect.y() + 13, 62, 24).unwrap());
    let dnd = Rect::new(
        clear_all.map_or(rect.right() - 17, |r| r.x()) - 40,
        rect.y() + 5,
        40,
        40,
    )
    .unwrap();
    let mut cards = Vec::new();
    let mut y = rect.y() + CENTRE_HEADER as i32;
    for (i, n) in notices.list.iter().enumerate() {
        let h = card_height(n);
        if y + h as i32 > rect.bottom() - 4 {
            break;
        }
        let r = Rect::new(rect.x() + 8, y, rect.width() - 16, h).unwrap();
        cards.push((i, card(r, n)));
        y += h as i32 + CARD_GAP;
    }
    Some(Centre {
        rect,
        heading,
        clear_all,
        dnd,
        cards,
    })
}
/// `dnd_available`: a desktop shell is connected to carry out the toggle.
pub fn hit_centre(
    layout: Layout,
    notices: &Notices,
    dnd_available: bool,
    x: i32,
    y: i32,
) -> Option<NoticeHit> {
    let k = centre(layout, notices)?;
    if !k.rect.contains(x, y) {
        return None;
    }
    if dnd_available && k.dnd.contains(x, y) {
        return Some(NoticeHit::Dnd);
    }
    if k.clear_all.is_some_and(|r| r.contains(x, y)) {
        return Some(NoticeHit::ClearAll);
    }
    k.cards
        .iter()
        .find_map(|(i, card)| hit_card(card, notices.list[*i].id, x, y))
        .or(Some(NoticeHit::Inside))
}
/// Centre content over the surface `tray` already painted (heading, Clear
/// all, Do not disturb, cards or the empty-state line).
#[allow(clippy::too_many_arguments)]
pub fn draw_centre_content(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    k: &Centre,
    notices: &Notices,
    dnd_available: bool,
    palette: &Palette,
    pointer: (i32, i32),
    dy: i32,
) {
    let shift = |r: Rect| Rect::new(r.x(), r.y() + dy, r.width(), r.height()).unwrap();
    if dnd_available {
        let dnd = shift(k.dnd);
        if notices.dnd {
            paint::rounded(c, dnd, 4, palette.accent);
        } else if k.dnd.contains(pointer.0, pointer.1) {
            paint::rounded(c, dnd, 4, palette.subtle_hover);
        }
        paint::glyph_in(
            c,
            dnd,
            if notices.dnd { 20 } else { 16 },
            if notices.dnd {
                crate::shell_mask!("setting-quiet", 20)
            } else {
                crate::shell_mask!("notification", 16)
            },
            if notices.dnd {
                palette.accent_text
            } else {
                palette.text
            },
        );
    }
    if let Some(clear) = k.clear_all.map(shift) {
        if k.clear_all
            .is_some_and(|r| r.contains(pointer.0, pointer.1))
        {
            paint::rounded(c, clear, 4, palette.subtle_hover);
        }
        paint::text_in(
            c,
            fonts,
            clear,
            "Clear all",
            TextStyle::Regular14,
            palette.text,
        );
    }
    for (i, card) in &k.cards {
        let n = &notices.list[*i];
        let r = shift(card.card);
        paint::framed(c, r, 6, palette.surface_border, palette.card);
        let moved = self::card(r, n);
        let pointer = (pointer.0, pointer.1 + dy);
        draw_card_content(c, fonts, &moved, n, palette, pointer, true);
    }
}

// --- Taskbar hover preview ----------------------------------------------

/// Thumbnail box (logical) of one window in the hover preview.
pub const THUMB_W: u32 = 200;
pub const THUMB_H: u32 = 120;
const PREVIEW_TITLE: u32 = 32;
const PREVIEW_PAD: i32 = 8;
/// Windows shown side by side in one preview.
pub const MAX_PREVIEW: usize = 4;
/// One window's live thumbnail (physical premultiplied BGRA) and title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewItem {
    pub id: u64,
    pub title: String,
    pub icon: u16,
    /// Physical thumbnail extent and pixels (letterboxed inside the box).
    pub size: (u32, u32),
    pub pixels: Vec<u8>,
}
/// The Windows 11 taskbar hover preview above one button.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    pub rect: Rect,
    pub items: Vec<PreviewItem>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewHit {
    Window(u64),
    Close(u64),
    Inside,
}
/// Preview surface centred above `anchor` (the hovered button), inside the
/// output.
pub fn preview_rect(layout: Layout, anchor: Rect, count: usize) -> Rect {
    let n = count.clamp(1, MAX_PREVIEW) as u32;
    let w = n * (THUMB_W + PREVIEW_PAD as u32) + PREVIEW_PAD as u32;
    let h = PREVIEW_TITLE + THUMB_H + 2 * PREVIEW_PAD as u32;
    layout.beside_bar(anchor, w, h, 8)
}
/// (card, close button, thumbnail box) per window.
pub fn preview_cards(rect: Rect, count: usize) -> Vec<(Rect, Rect, Rect)> {
    (0..count.min(MAX_PREVIEW))
        .map(|i| {
            let x = rect.x() + PREVIEW_PAD / 2 + i as i32 * (THUMB_W as i32 + PREVIEW_PAD);
            let card = Rect::new(
                x,
                rect.y() + PREVIEW_PAD / 2,
                THUMB_W + PREVIEW_PAD as u32,
                rect.height() - PREVIEW_PAD as u32,
            )
            .unwrap();
            let close = Rect::new(card.right() - 4 - 24, card.y() + 4, 24, 24).unwrap();
            let thumb = Rect::new(
                card.x() + PREVIEW_PAD / 2,
                card.y() + PREVIEW_TITLE as i32,
                THUMB_W,
                THUMB_H,
            )
            .unwrap();
            (card, close, thumb)
        })
        .collect()
}
pub fn hit_preview(p: &Preview, x: i32, y: i32) -> Option<PreviewHit> {
    if !p.rect.contains(x, y) {
        return None;
    }
    for ((card, close, _), item) in preview_cards(p.rect, p.items.len()).iter().zip(&p.items) {
        if close.contains(x, y) {
            return Some(PreviewHit::Close(item.id));
        }
        if card.contains(x, y) {
            return Some(PreviewHit::Window(item.id));
        }
    }
    Some(PreviewHit::Inside)
}
/// Bilinear thumbnail of a premultiplied BGRA surface into a `tw`×`th`
/// box (aspect kept, transparent letterbox). Returns (size, pixels).
pub fn thumbnail(
    bytes: &[u8],
    stride: usize,
    sw: usize,
    sh: usize,
    tw: usize,
    th: usize,
) -> ((u32, u32), Vec<u8>) {
    if sw == 0 || sh == 0 || tw == 0 || th == 0 || bytes.len() < stride * (sh - 1) + sw * 4 {
        return ((0, 0), Vec::new());
    }
    let dw = tw.min(th * sw / sh).max(1);
    let dh = th.min(tw * sh / sw).max(1);
    let mut out = alloc::vec![0u8; dw * dh * 4];
    for y in 0..dh {
        for x in 0..dw {
            let sx = (((x as u64 * 2 + 1) * sw as u64 * 32768 / dw as u64).saturating_sub(32768))
                .min(((sw - 1) as u64) << 16);
            let sy = (((y as u64 * 2 + 1) * sh as u64 * 32768 / dh as u64).saturating_sub(32768))
                .min(((sh - 1) as u64) << 16);
            let (ix, iy, fx, fy) = (sx as usize >> 16, sy as usize >> 16, sx & 65535, sy & 65535);
            for channel in 0..4 {
                let v = |dx: usize, dy: usize| u64::from(bytes[dy * stride + dx * 4 + channel]);
                let a = v(ix, iy) * (65536 - fx) + v((ix + 1).min(sw - 1), iy) * fx;
                let b = v(ix, (iy + 1).min(sh - 1)) * (65536 - fx)
                    + v((ix + 1).min(sw - 1), (iy + 1).min(sh - 1)) * fx;
                out[(y * dw + x) * 4 + channel] =
                    ((a * (65536 - fy) + b * fy + (1 << 31)) >> 32) as u8;
            }
        }
    }
    ((dw as u32, dh as u32), out)
}
/// A taskbar tooltip: the button's name in a WinUI ToolTip (12 px text, 8 px
/// side padding, 4 px corners), off the bar beside its button.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tooltip {
    pub rect: Rect,
    pub text: String,
}
/// Tooltip height at 100% text size; it grows with Accessibility > Text size.
pub const TOOLTIP_HEIGHT: u32 = 30;
impl Tooltip {
    pub fn new(fonts: &FontAtlas, layout: Layout, anchor: Rect, text: &str) -> Self {
        let w = paint::measure(fonts, text, TextStyle::Regular12) + 18;
        let h = paint::text_grow(TOOLTIP_HEIGHT, TextStyle::Regular12);
        Self {
            rect: layout.beside_bar(anchor, w, h, 10),
            text: String::from(text),
        }
    }
}
pub fn draw_tooltip(c: &mut Canvas<'_>, fonts: &FontAtlas, t: &Tooltip, dark: bool) {
    let palette = Palette::new(dark);
    let (fill, border) = if dark {
        (paint::rgb(43, 43, 43), paint::rgb(60, 60, 60))
    } else {
        (paint::rgb(249, 249, 249), paint::rgb(224, 224, 224))
    };
    paint::shadow(c, t.rect, 4, dark);
    paint::framed(c, t.rect, 4, border, fill);
    paint::text_span(
        c,
        fonts,
        t.rect.x(),
        t.rect.width(),
        t.rect.y(),
        t.rect.height(),
        &t.text,
        TextStyle::Regular12,
        palette.text,
    );
}
pub fn draw_preview(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    p: &Preview,
    dark: bool,
    pointer: (i32, i32),
) {
    let palette = Palette::new(dark);
    surface(c, p.rect, &palette);
    let scale = c.scale();
    for ((card, close, thumb), item) in preview_cards(p.rect, p.items.len()).iter().zip(&p.items) {
        if card.contains(pointer.0, pointer.1) {
            paint::rounded(c, *card, 6, palette.subtle_hover);
        }
        draw_icon(c, card.x() + 8, card.y() + 8, 16, item.icon);
        paint::text_row(
            c,
            fonts,
            card.x() + 32,
            card.y() + 4,
            24,
            &paint::ellipsize(fonts, &item.title, TextStyle::Regular12, THUMB_W - 60),
            TextStyle::Regular12,
            palette.text,
            THUMB_W - 60,
        );
        if card.contains(pointer.0, pointer.1) {
            if close.contains(pointer.0, pointer.1) {
                paint::rounded(c, *close, 4, paint::rgb(196, 43, 28));
            }
            paint::glyph_in(
                c,
                *close,
                12,
                crate::shell_mask!("chrome-close", 12),
                if close.contains(pointer.0, pointer.1) {
                    paint::rgb(255, 255, 255)
                } else {
                    palette.text
                },
            );
        }
        let (w, h) = item.size;
        if w == 0 || item.pixels.len() != (w * h * 4) as usize {
            continue;
        }
        let box_px = paint::physical(*thumb, scale);
        let x = box_px.x() + (box_px.width() as i32 - w as i32) / 2;
        let y = box_px.y() + (box_px.height() as i32 - h as i32) / 2;
        if let Ok(r) = Rect::new(x, y, w, h) {
            let _ = c.blit_bgra_physical(r, &item.pixels);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn previews_and_tooltips_open_off_the_bar_on_every_edge() {
        use door_service_protocol::desktop::Controls;
        for edge in 0..4u32 {
            let controls = Controls::from_bits(edge).unwrap();
            let layout = Layout::with_controls(1920, 1080, controls);
            let (bar, work) = (layout.bar(), layout.work_area());
            let anchor = layout.slot(crate::taskbar::Slot::Start, 6);
            for r in [
                preview_rect(layout, anchor, 2),
                layout.beside_bar(anchor, 60, 30, 10),
            ] {
                assert!(r.intersection(bar).is_none(), "edge {edge}: covers the bar");
                assert!(
                    r.x() >= work.x() && r.right() <= work.right(),
                    "edge {edge}: off the work area"
                );
                assert!(r.y() >= work.y() && r.bottom() <= work.bottom());
                // Centred on the button along the bar.
                if layout.vertical() {
                    let mid = anchor.y() + anchor.height() as i32 / 2;
                    assert!(r.y() <= mid && r.bottom() >= mid);
                } else {
                    let mid = anchor.x() + anchor.width() as i32 / 2;
                    assert!(r.x() <= mid && r.right() >= mid);
                }
            }
        }
    }
    #[test]
    fn hover_preview_sits_above_its_button_and_hits_close_and_cards() {
        let layout = Layout::new(1920, 1080);
        let anchor = Rect::new(900, 1036, 40, 40).unwrap();
        let r = preview_rect(layout, anchor, 2);
        assert!(r.bottom() <= layout.work_area().bottom());
        assert_eq!(r.x() + r.width() as i32 / 2, 920);
        let edge = preview_rect(layout, Rect::new(0, 1036, 40, 40).unwrap(), 4);
        assert!(edge.x() >= 8);
        let item = |id| PreviewItem {
            id,
            title: "Editor".into(),
            icon: s::icon::WINDOWS_APP,
            size: (0, 0),
            pixels: Vec::new(),
        };
        let p = Preview {
            rect: r,
            items: alloc::vec![item(5), item(6)],
        };
        let cards = preview_cards(r, 2);
        let (card, close, _) = cards[1];
        assert_eq!(
            hit_preview(&p, close.x() + 2, close.y() + 2),
            Some(PreviewHit::Close(6))
        );
        assert_eq!(
            hit_preview(&p, card.x() + 2, card.bottom() - 2),
            Some(PreviewHit::Window(6))
        );
        assert_eq!(hit_preview(&p, 0, 0), None);
        // A 4x2 opaque source letterboxes into a 4x4 box as 4x2.
        let src = [255u8; 4 * 2 * 4];
        let ((w, h), px) = thumbnail(&src, 16, 4, 2, 4, 4);
        assert_eq!((w, h, px.len()), (4, 2, 32));
        assert!(px.iter().all(|b| *b == 255));
    }
}
#[cfg(test)]
mod notice_tests {
    use super::*;
    fn notice(id: u32, toast: bool, actions: usize) -> Notice {
        Notice {
            id,
            icon: s::icon::FILES,
            toast,
            source: "File Explorer".into(),
            time: "Now".into(),
            title: "Copy complete".into(),
            body: "3 items".into(),
            actions: (0..actions).map(|i| alloc::format!("A{i}")).collect(),
        }
    }
    #[test]
    fn toasts_stack_bottom_right_above_the_taskbar_newest_lowest() {
        let layout = Layout::new(1920, 1080);
        let notices = Notices {
            list: (1..=5)
                .map(|i| notice(i, i != 2, (i % 2) as usize))
                .collect(),
            dnd: false,
            badge: 5,
        };
        let t = toasts(layout, &notices);
        assert_eq!(t.len(), MAX_TOASTS);
        let work = layout.work_area();
        assert_eq!(t[0].1.bottom(), work.bottom() - 12);
        assert_eq!(t[0].1.right(), work.right() - 12);
        assert_eq!(notices.list[t[0].0].id, 1);
        assert_eq!(notices.list[t[1].0].id, 3, "non-toast id 2 is skipped");
        assert!(t[1].1.bottom() < t[0].1.y());
        let k = card(t[0].1, &notices.list[t[0].0]);
        let (x, y) = (k.close.x() + 2, k.close.y() + 2);
        assert_eq!(
            hit_toasts(layout, &notices, x, y),
            Some(NoticeHit::Close(1))
        );
        let b = k.buttons[0];
        assert_eq!(
            hit_toasts(layout, &notices, b.x() + 1, b.y() + 1),
            Some(NoticeHit::Button(1, 0))
        );
        assert_eq!(hit_toasts(layout, &notices, 5, 5), None);
    }
    #[test]
    fn centre_keeps_its_empty_geometry_and_grows_for_cards() {
        let layout = Layout::new(1920, 1080);
        let empty = Notices::default();
        let k = centre(layout, &empty).unwrap();
        assert_eq!(Some(k.rect), crate::tray::notifications_rect(layout));
        assert!(k.clear_all.is_none() && k.cards.is_empty());
        let full = Notices {
            list: (1..=12).map(|i| notice(i, false, 1)).collect(),
            dnd: true,
            badge: 0,
        };
        let k = centre(layout, &full).unwrap();
        assert!(k.rect.height() > crate::tray::notifications_rect(layout).unwrap().height());
        assert!(k.rect.y() >= layout.work_area().y() + 12);
        assert!(!k.cards.is_empty() && k.cards.len() < 12);
        for (_, card) in &k.cards {
            assert!(k.rect.contains(card.card.x(), card.card.bottom() - 1));
        }
        let clear = k.clear_all.unwrap();
        assert_eq!(
            hit_centre(layout, &full, true, clear.x() + 1, clear.y() + 1),
            Some(NoticeHit::ClearAll)
        );
        assert_eq!(
            hit_centre(layout, &full, true, k.dnd.x() + 1, k.dnd.y() + 1),
            Some(NoticeHit::Dnd)
        );
        assert_eq!(
            hit_centre(layout, &full, false, k.dnd.x() + 1, k.dnd.y() + 1),
            Some(NoticeHit::Inside)
        );
    }
    #[test]
    fn icons_map_to_bundled_artwork() {
        for app in App::ALL {
            assert_eq!(icon_app(app_icon(app)), Some(app));
        }
        assert_eq!(icon_app(s::icon::WINDOWS_APP), None);
    }
}
