//! Win+V clipboard history panel (Windows 11). Up to 25 recent items (text,
//! HTML and bitmaps) newest first, each with "See more" (Pin/Unpin, Delete),
//! "Clear all" in the header, and the "Turn on" prompt while history is off.
//! Choosing an item makes it the clipboard and pastes it into the app that
//! had focus. The emoji, GIF, kaomoji and symbol tabs of the Windows panel
//! are not shown: Doors has no emoji font or GIF service.
//!
//! This module is the panel's model (state, geometry, keys, hit testing,
//! previews); `clipboard_view` paints it.
extern crate alloc;
use crate::clipboard::{Content, HistoryItem};
use alloc::{string::String, vec::Vec};
use door_graphics_core::Rect;
use door_service_protocol::clipboard as cb;
pub const WIDTH: u32 = 360;
pub const HEIGHT: u32 = 460;
const HEADER: i32 = 52;
pub const CARD: u32 = 80;
const GAP: i32 = 8;
const MARGIN: i32 = 12;
pub const MORE: u32 = 32;
const MENU_W: u32 = 140;
pub const MENU_ROW: u32 = 36;
// Accessibility > Text size (identity at 100%): text bands grow by their
// style's line growth, text-only buttons widen with their text; the panel
// width, "See more" button and glyphs keep their size.
use crate::paint::{text_grow, text_len};
use door_graphics_text::TextStyle;
fn header() -> i32 {
    HEADER + text_grow(0, TextStyle::Semibold14) as i32
}
/// An item card: two Regular14 preview lines.
pub fn card_height() -> u32 {
    CARD + 2 * text_grow(0, TextStyle::Regular14)
}
/// Pitch of the card's preview lines.
pub fn card_line() -> u32 {
    text_grow(22, TextStyle::Regular14)
}
pub fn menu_row_height() -> u32 {
    text_grow(MENU_ROW, TextStyle::Regular14)
}
/// Lines of the off/empty explanation shown (two at 100%; room for four
/// once larger text wraps it further) and their pitch.
pub fn body_lines() -> u32 {
    if text_len(100) > 100 { 4 } else { 2 }
}
pub fn body_pitch() -> u32 {
    text_grow(20, TextStyle::Regular12)
}
/// Off/empty-state title band at `top` (panel-relative 140 or 150) and the
/// explanation's first line under it.
pub fn state_title(panel: Rect, top: i32) -> (Rect, i32) {
    let h = text_grow(24, TextStyle::Semibold14);
    (
        Rect::new(panel.x() + 24, panel.y() + top, WIDTH - 48, h).unwrap(),
        panel.y() + top + h as i32 + 4,
    )
}
/// Strings (Windows 11 wording where known).
pub const TITLE: &str = "Clipboard history";
pub const CLEAR_ALL: &str = "Clear all";
pub const OFF_TITLE: &str = "Clipboard history is off";
pub const OFF_BODY: &str =
    "Turn it on to see the last 25 items you copied and pin the ones you use often.";
pub const TURN_ON: &str = "Turn on";
pub const EMPTY_TITLE: &str = "Nothing here";
pub const EMPTY_BODY: &str = "You'll see your clipboard history here once you've copied something.";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Panel,
    TurnOn,
    ClearAll,
    /// Paste the item with this id.
    Item(u32),
    /// "See more" on the item with this id.
    More(u32),
    Pin,
    Delete,
}
/// What a key does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Close,
    /// Item ids, never positions: a copy made while the panel is open
    /// reorders the list under the user.
    Paste(u32),
    Delete(u32),
    TurnOn,
    Moved,
    Ignored,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Panel {
    pub open: bool,
    /// Keyboard selection: an item id (0, or an id no longer listed: the
    /// first item).
    pub selected: u32,
    /// First visible pixel row of the item list.
    pub scroll: i32,
    /// "See more" menu open for the item with this id.
    pub menu: Option<u32>,
    /// The focused app's text caret (logical) when the panel opened: the
    /// panel opens there, as in Windows.
    pub anchor: Option<Rect>,
}
pub struct Geometry {
    pub panel: Rect,
    pub clear: Rect,
    pub list: Rect,
}
impl Geometry {
    /// Layout inside the panel bounds (`clipboard_view::geometry` places
    /// the panel above the taskbar).
    pub fn at(panel: Rect) -> Self {
        let clear_w = text_len(72);
        let clear = Rect::new(
            panel.right() - 16 - clear_w as i32,
            panel.y() + 12,
            clear_w,
            text_grow(28, TextStyle::Regular14),
        )
        .unwrap();
        let list_h = (panel.height() as i32 - header() - MARGIN).max(1) as u32;
        let list = Rect::new(
            panel.x() + MARGIN,
            panel.y() + header(),
            panel.width() - 2 * MARGIN as u32,
            list_h,
        )
        .unwrap();
        Self { panel, clear, list }
    }
    /// Item `i`'s card (before clipping to the list).
    pub fn card(&self, i: usize, scroll: i32) -> Rect {
        let h = card_height();
        Rect::new(
            self.list.x(),
            self.list.y() + i as i32 * (h as i32 + GAP) - scroll,
            self.list.width(),
            h,
        )
        .unwrap()
    }
    pub fn more(&self, i: usize, scroll: i32) -> Rect {
        let card = self.card(i, scroll);
        Rect::new(card.right() - 4 - MORE as i32, card.y() + 4, MORE, MORE).unwrap()
    }
    /// The "See more" menu (Pin/Unpin, Delete) under item `i`'s button.
    pub fn menu(&self, i: usize, scroll: i32) -> Rect {
        let more = self.more(i, scroll);
        let h = 2 * menu_row_height() + 8;
        let w = MENU_W - 92 + text_len(92);
        let below = more.bottom() + 4;
        let y = if below + h as i32 <= self.panel.bottom() - 8 {
            below
        } else {
            more.y() - 4 - h as i32
        };
        Rect::new(more.right() - w as i32, y, w, h).unwrap()
    }
    pub fn menu_row(&self, i: usize, scroll: i32, row: i32) -> Rect {
        let m = self.menu(i, scroll);
        let h = menu_row_height();
        Rect::new(m.x() + 4, m.y() + 4 + row * h as i32, m.width() - 8, h).unwrap()
    }
    /// "Turn on" (history off).
    pub fn turn_on(&self) -> Rect {
        // Under the off-state title and its explanation (both grown).
        let (_, body) = state_title(self.panel, 140);
        let y = body + (body_lines() * body_pitch()) as i32 + 12;
        let w = 48 + text_len(72);
        Rect::new(
            self.panel.x() + (WIDTH as i32 - w as i32) / 2,
            y,
            w,
            text_grow(32, TextStyle::Regular14),
        )
        .unwrap()
    }
    fn content_height(items: usize) -> i32 {
        (items as i32 * (card_height() as i32 + GAP) - GAP).max(0)
    }
    pub fn max_scroll(&self, items: usize) -> i32 {
        (Self::content_height(items) - self.list.height() as i32).max(0)
    }
}
impl Panel {
    pub fn show(&mut self) {
        self.show_at(None);
    }
    /// Open at the text caret (`None`: above the taskbar's tray end).
    pub fn show_at(&mut self, anchor: Option<Rect>) {
        *self = Self {
            open: true,
            anchor,
            ..Self::default()
        };
    }
    pub fn hit(
        &self,
        g: &Geometry,
        history_on: bool,
        items: &[HistoryItem],
        x: i32,
        y: i32,
    ) -> Option<Target> {
        if !self.open || !g.panel.contains(x, y) {
            return None;
        }
        if let Some(i) = self.menu_index(items)
            && g.menu(i, self.scroll).contains(x, y)
        {
            return Some(if g.menu_row(i, self.scroll, 0).contains(x, y) {
                Target::Pin
            } else if g.menu_row(i, self.scroll, 1).contains(x, y) {
                Target::Delete
            } else {
                Target::Panel
            });
        }
        if !history_on {
            return Some(if g.turn_on().contains(x, y) {
                Target::TurnOn
            } else {
                Target::Panel
            });
        }
        if !items.is_empty() && g.clear.contains(x, y) {
            return Some(Target::ClearAll);
        }
        if g.list.contains(x, y) {
            for (i, item) in items.iter().enumerate() {
                if g.more(i, self.scroll).contains(x, y) {
                    return Some(Target::More(item.id));
                }
                if g.card(i, self.scroll).contains(x, y) {
                    return Some(Target::Item(item.id));
                }
            }
        }
        Some(Target::Panel)
    }
    /// Position of the selected item (the first when it is gone).
    pub fn selected_index(&self, items: &[HistoryItem]) -> usize {
        items
            .iter()
            .position(|h| h.id == self.selected)
            .unwrap_or(0)
    }
    /// Position of the item whose "See more" menu is open, while it exists.
    pub fn menu_index(&self, items: &[HistoryItem]) -> Option<usize> {
        let id = self.menu?;
        items.iter().position(|h| h.id == id)
    }
    fn ensure_visible(&mut self, g: &Geometry, index: usize, items: usize) {
        let card = g.card(index, 0);
        let top = card.y() - g.list.y();
        let bottom = top + card_height() as i32;
        if top < self.scroll {
            self.scroll = top;
        } else if bottom > self.scroll + g.list.height() as i32 {
            self.scroll = bottom - g.list.height() as i32;
        }
        self.scroll = self.scroll.clamp(0, g.max_scroll(items));
    }
    /// Keyboard (set 1 scan codes): Esc closes, Up/Down select, Enter pastes,
    /// Delete removes the selected item (the selection moves to its
    /// neighbour, as in Windows).
    pub fn key(&mut self, g: &Geometry, scan: u32, history_on: bool, items: &[HistoryItem]) -> Key {
        if scan == 1 {
            if self.menu.take().is_some() {
                return Key::Moved;
            }
            return Key::Close;
        }
        if !history_on {
            return if matches!(scan, 28 | 0x11c) {
                Key::TurnOn
            } else {
                Key::Ignored
            };
        }
        if items.is_empty() {
            return Key::Ignored;
        }
        let mut i = self.selected_index(items);
        match scan {
            0x148 => i = i.saturating_sub(1),
            0x150 => i = (i + 1).min(items.len() - 1),
            28 | 0x11c => return Key::Paste(items[i].id),
            0x153 => {
                let next = items.get(i + 1).or(i.checked_sub(1).map(|j| &items[j]));
                self.selected = next.map_or(0, |h| h.id);
                return Key::Delete(items[i].id);
            }
            _ => return Key::Ignored,
        }
        self.selected = items[i].id;
        self.menu = None;
        self.ensure_visible(g, i, items.len());
        Key::Moved
    }
    pub fn wheel(&mut self, g: &Geometry, items: usize, notches: i32) -> bool {
        let old = self.scroll;
        self.scroll = (self.scroll - notches * 48).clamp(0, g.max_scroll(items));
        old != self.scroll
    }
}
/// Source bytes a preview reads: enough for two lines after tags are
/// stripped, never the whole (up to 4 MB) item.
const PREVIEW_SCAN: usize = 4096;
/// Longest preview line kept (the card shows well under this).
pub const PREVIEW_BYTES: usize = 256;
/// The valid UTF-8 prefix of at most `max` bytes.
fn utf8_prefix(bytes: &[u8], max: usize) -> &str {
    let bytes = &bytes[..bytes.len().min(max)];
    match core::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => core::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap_or(""),
    }
}
/// Up to two preview lines of an item's text (HTML stripped of tags), each
/// cut to `PREVIEW_BYTES` at a grapheme boundary. Computed once per item
/// (`HistoryItem::new`).
pub fn preview_of(content: &Content) -> Vec<String> {
    let text: String = match (content.get(cb::TEXT), content.get(cb::HTML)) {
        (Some(t), _) => String::from(utf8_prefix(t, PREVIEW_SCAN)),
        (None, Some(h)) => {
            let mut out = String::new();
            let mut tag = false;
            for ch in utf8_prefix(h, PREVIEW_SCAN).chars() {
                match ch {
                    '<' => tag = true,
                    '>' => tag = false,
                    c if !tag => out.push(c),
                    _ => (),
                }
            }
            out
        }
        _ => String::new(),
    };
    text.lines()
        .map(|l| l.trim_end_matches('\r').replace('\t', "    "))
        .filter(|l| !l.trim().is_empty())
        .take(2)
        .map(|l| {
            let l = l.trim_start();
            String::from(&l[..door_graphics_text::grapheme_floor(l, PREVIEW_BYTES)])
        })
        .collect()
}
/// The item's cached preview lines.
pub fn preview(item: &HistoryItem) -> &[String] {
    &item.preview
}
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::clipboard::{Broker, Data};
    use alloc::rc::Rc;
    pub(crate) fn broker_with(texts: &[&str]) -> Broker {
        let mut b = Broker::new();
        b.apply_settings(cb::SETTING_HISTORY).unwrap();
        for t in texts.iter().rev() {
            b.remember_for_tests(Content {
                formats: alloc::vec![Data {
                    format: cb::TEXT,
                    bytes: Rc::new(t.as_bytes().to_vec())
                }],
            });
        }
        b
    }
    /// The panel where `clipboard_view::geometry` puts it on a 1280x720 output.
    pub(crate) fn sample() -> Geometry {
        Geometry::at(
            Rect::new(
                1280 - 12 - WIDTH as i32,
                720 - 48 - 12 - HEIGHT as i32,
                WIDTH,
                HEIGHT,
            )
            .unwrap(),
        )
    }
    #[test]
    fn cards_stack_without_overlap_inside_the_panel() {
        let g = sample();
        assert!(g.panel.contains(g.clear.x(), g.clear.y()));
        assert_eq!(g.panel.intersection(g.list), Some(g.list));
        for i in 0..4 {
            let (a, b) = (g.card(i, 0), g.card(i + 1, 0));
            assert!(a.intersection(b).is_none());
            assert_eq!(b.y() - a.bottom(), GAP);
            assert!(a.contains(g.more(i, 0).x(), g.more(i, 0).y()));
        }
        assert!(g.max_scroll(25) > 0);
        assert_eq!(g.max_scroll(1), 0);
    }
    #[test]
    fn keys_select_paste_delete_and_close() {
        let g = sample();
        let texts: Vec<String> = (0..25).map(|i| alloc::format!("item {i}")).collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let b = broker_with(&refs);
        let items = b.history();
        let mut p = Panel::default();
        p.show();
        assert_eq!(p.key(&g, 0x150, true, items), Key::Moved);
        assert_eq!(p.selected_index(items), 1);
        for _ in 0..30 {
            p.key(&g, 0x150, true, items);
        }
        assert_eq!(p.selected_index(items), 24);
        let last = g.card(24, p.scroll);
        assert!(
            last.bottom() <= g.list.bottom() && last.y() >= g.list.y(),
            "selection scrolled into view"
        );
        assert_eq!(p.key(&g, 28, true, items), Key::Paste(items[24].id));
        assert_eq!(p.key(&g, 0x153, true, items), Key::Delete(items[24].id));
        assert_eq!(p.selected, items[23].id, "selection moves to the neighbour");
        p.menu = Some(items[2].id);
        assert_eq!(
            p.key(&g, 1, true, items),
            Key::Moved,
            "Esc closes See more first"
        );
        assert_eq!(p.key(&g, 1, true, items), Key::Close);
        assert_eq!(p.key(&g, 28, false, &[]), Key::TurnOn);
        assert_eq!(p.key(&g, 0x150, true, &[]), Key::Ignored);
    }
    #[test]
    fn pointer_targets_follow_the_painted_geometry() {
        let g = sample();
        let b = broker_with(&["a", "b", "c"]);
        let items = b.history();
        let mut p = Panel::default();
        p.show();
        let at = |r: Rect| (r.x() + r.width() as i32 / 2, r.y() + r.height() as i32 / 2);
        let (x, y) = at(g.card(1, 0));
        assert_eq!(
            p.hit(&g, true, items, x, y),
            Some(Target::Item(items[1].id))
        );
        let (x, y) = at(g.more(1, 0));
        assert_eq!(
            p.hit(&g, true, items, x, y),
            Some(Target::More(items[1].id))
        );
        let (x, y) = at(g.clear);
        assert_eq!(p.hit(&g, true, items, x, y), Some(Target::ClearAll));
        assert_eq!(
            p.hit(&g, true, &[], x, y),
            Some(Target::Panel),
            "nothing to clear"
        );
        p.menu = Some(items[1].id);
        let (x, y) = at(g.menu_row(1, 0, 0));
        assert_eq!(p.hit(&g, true, items, x, y), Some(Target::Pin));
        let (x, y) = at(g.menu_row(1, 0, 1));
        assert_eq!(p.hit(&g, true, items, x, y), Some(Target::Delete));
        let (x, y) = at(g.turn_on());
        assert_eq!(p.hit(&g, false, &[], x, y), Some(Target::TurnOn));
        assert_eq!(p.hit(&g, true, items, g.panel.x() - 5, y), None);
    }
    /// H4: a copy committed by a background app while the panel is open
    /// shifts every item down one place; keys and the See more menu still
    /// act on the item the user chose, and a vanished item is a no-op.
    #[test]
    fn a_background_commit_does_not_redirect_panel_actions() {
        use crate::clipboard::tests::{A, TestEnv, broker, copy};
        let (mut c, mut env) = (broker(), TestEnv::default());
        c.apply_settings(cb::SETTING_HISTORY).unwrap();
        for t in ["one", "two", "three"] {
            copy(&mut c, &mut env, A, 1, &[(cb::TEXT, t.as_bytes())]).unwrap();
        }
        let g = sample();
        let mut p = Panel::default();
        p.show();
        p.key(&g, 0x150, true, c.history());
        let chosen = c.history()[1].id;
        assert_eq!(p.selected, chosen);
        p.menu = Some(c.history()[2].id);
        let menu_item = c.history()[2].id;
        // The background copy lands on top.
        copy(&mut c, &mut env, A, 1, &[(cb::TEXT, b"background")]).unwrap();
        assert_ne!(c.history()[1].id, chosen, "the list moved under the panel");
        assert_eq!(p.selected_index(c.history()), 2);
        assert_eq!(p.menu_index(c.history()), Some(3));
        assert_eq!(p.key(&g, 28, true, c.history()), Key::Paste(chosen));
        assert!(c.paste_history(chosen));
        assert_eq!(c.content().get(cb::TEXT), Some(&b"two"[..]));
        // See more > Pin hits the menu's own row and pins that item.
        let at = |r: Rect| (r.x() + 4, r.y() + 4);
        let row = g.menu_row(p.menu_index(c.history()).unwrap(), p.scroll, 0);
        let (x, y) = at(row);
        assert_eq!(p.hit(&g, true, c.history(), x, y), Some(Target::Pin));
        assert!(c.toggle_pin(p.menu.take().unwrap()));
        assert!(c.history().iter().any(|h| h.id == menu_item && h.pinned));
        assert!(c.history().iter().filter(|h| h.pinned).count() == 1);
        // A deleted item is a no-op for the actions still holding its id.
        assert!(c.delete(chosen));
        assert!(!c.delete(chosen));
        assert!(!c.paste_history(chosen));
        assert!(!c.toggle_pin(chosen));
        assert_eq!(p.selected_index(c.history()), 0, "falls back to the top");
    }
    #[test]
    fn previews_show_two_lines_and_strip_html() {
        let b = broker_with(&["first line\r\n\r\n  second line\nthird"]);
        assert_eq!(preview(&b.history()[0]), ["first line", "second line"]);
        let item = HistoryItem::new(
            1,
            Content {
                formats: alloc::vec![Data {
                    format: cb::HTML,
                    bytes: Rc::new(b"<p>Hi <b>there</b></p>".to_vec())
                }],
            },
        );
        assert_eq!(preview(&item), ["Hi there"]);
    }
    /// H3: a 4 MiB single-line item keeps a short cached preview, and fitting
    /// it (or any label) into a card is a bounded binary search.
    #[test]
    fn a_4_mib_item_previews_with_bounded_measurements() {
        let mut line = String::from("e\u{301}");
        while line.len() + 32 <= cb::HISTORY_ITEM_BYTES {
            line.push_str("wide text \u{1F469}\u{200D}\u{1F4BB} ");
        }
        let b = broker_with(&[line.as_str()]);
        let cached = preview(&b.history()[0]);
        assert_eq!(cached.len(), 1);
        assert!(cached[0].len() <= PREVIEW_BYTES && cached[0].len() > 200);
        assert!(line.starts_with(cached[0].as_str()));
        // 7 px per char, 300 px of room.
        let width = |s: &str| 7 * s.chars().count() as u32;
        for label in [cached[0].as_str(), line.as_str()] {
            let (mut calls, mut longest) = (0u32, 0usize);
            let fitted = crate::paint::ellipsize_by(label, 300, |s| {
                calls += 1;
                longest = longest.max(s.len());
                width(s)
            });
            assert!(fitted.ends_with("...") && width(&fitted) <= 300, "{fitted}");
            assert!(calls <= 20, "{calls} measurements");
            assert!(longest <= 300 * 32 + 3, "measured {longest} bytes");
            // Grapheme boundaries only: the ZWJ sequence is never split.
            let kept = fitted.trim_end_matches("...");
            assert!(!kept.ends_with('\u{200D}') && !kept.ends_with('\u{1F469}'));
        }
        // A frame of the panel with the real fonts paints the cached line.
        let layout = crate::taskbar::Layout::new(1280, 720);
        let fonts = door_graphics_text::FontAtlas::windows_reference_scaled(1).unwrap();
        let mut bytes = alloc::vec![0u8; 1280 * 720 * 4];
        let mut c = door_graphics_core::Canvas::new(&mut bytes, 1280, 720, 1280 * 4).unwrap();
        let mut p = Panel::default();
        p.show();
        crate::clipboard_view::draw(&mut c, &fonts, &p, &b, layout, false, (0, 0));
    }
}
