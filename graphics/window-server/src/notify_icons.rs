//! Windows programs' notification-area icons (Shell_NotifyIcon). Windows
//! 11 puts a new program's icon in the "Show hidden icons" overflow until
//! the user promotes it, so that is where they live here: the chevron at
//! the left of the notification area opens a small flyout with the icons
//! in a grid, and a click there is sent back to the owner, which posts its
//! callback message.
//!
//! Requests are untrusted: the tip is bounded UTF-8, each owner holds at
//! most `notify::MAX_ICONS`, and the pixels are exactly one 32 px BGRA
//! premultiplied image.
extern crate alloc;
use crate::taskbar::Layout;
use alloc::string::String;
use alloc::vec::Vec;
use door_graphics_core::Owner;
pub use door_graphics_core::Rect;
use door_service_protocol::Code;
use door_service_protocol::window::{self as w, notify};

/// What the user did to an icon (`Event::b`).
pub const LEFT: u32 = 1;
pub const RIGHT: u32 = 2;
pub const DOUBLE: u32 = 3;
pub const KEYBOARD: u32 = 4;

/// Flyout grid: 40 px cells, at most five a row, 6 px padding.
pub const CELL: u32 = 40;
pub const COLUMNS: usize = 5;
pub const PAD: u32 = 6;
pub const ICON: u32 = 16;

pub struct Icon {
    pub owner: Owner,
    pub window: u64,
    pub id: u32,
    pub version: u8,
    pub tip: String,
    /// 32 x 32 premultiplied BGRA, or empty until the owner gives one.
    pub bgra: Vec<u8>,
}
pub struct Request<'a> {
    pub op: u8,
    pub version: u8,
    pub flags: u16,
    pub window: u64,
    pub id: u32,
    pub tip: &'a str,
}
/// `[op][version][flags u16][window u64][id u32][callback u32][tip len][tip]`.
pub fn decode(b: &[u8]) -> Result<Request<'_>, Code> {
    if b.len() < 21 {
        return Err(Code::InvalidRequest);
    }
    let tip_len = usize::from(b[20]);
    if tip_len > notify::MAX_TIP || b.len() != 21 + tip_len {
        return Err(Code::InvalidRequest);
    }
    let tip = core::str::from_utf8(&b[21..]).map_err(|_| Code::InvalidRequest)?;
    if tip.chars().any(|c| c.is_control() && c != '\n') {
        return Err(Code::InvalidRequest);
    }
    let op = b[0];
    if op > notify::RECT || op == notify::SET_FOCUS && b[1] > 4 {
        return Err(Code::InvalidRequest);
    }
    Ok(Request {
        op,
        version: b[1],
        flags: u16::from_le_bytes([b[2], b[3]]),
        window: u64::from_le_bytes(b[4..12].try_into().unwrap()),
        id: u32::from_le_bytes(b[12..16].try_into().unwrap()),
        tip,
    })
}

#[derive(Default)]
pub struct Icons {
    pub icons: Vec<Icon>,
    events: Vec<(Owner, w::Event)>,
    /// The last primary click (icon, milliseconds) for double clicks.
    last: Option<(usize, u64)>,
}
impl Icons {
    pub fn find(&self, owner: Owner, window: u64, id: u32) -> Option<usize> {
        self.icons
            .iter()
            .position(|i| i.owner == owner && i.window == window && i.id == id)
    }
    /// One request with, for `notify::ICON`, the pixels the section held.
    pub fn apply(
        &mut self,
        owner: Owner,
        r: &Request<'_>,
        bgra: Option<Vec<u8>>,
    ) -> Result<(), Code> {
        if bgra.as_ref().is_some_and(|b| b.len() != notify::ICON_BYTES) {
            return Err(Code::InvalidRequest);
        }
        let at = self.find(owner, r.window, r.id);
        match r.op {
            notify::ADD => {
                if at.is_some() {
                    return Err(Code::Exists);
                }
                if self.icons.iter().filter(|i| i.owner == owner).count() >= notify::MAX_ICONS {
                    return Err(Code::Resources);
                }
                self.icons.push(Icon {
                    owner,
                    window: r.window,
                    id: r.id,
                    version: 0,
                    tip: String::new(),
                    bgra: Vec::new(),
                });
                self.update(self.icons.len() - 1, r, bgra);
                Ok(())
            }
            notify::MODIFY => {
                let at = at.ok_or(Code::NotFound)?;
                self.update(at, r, bgra);
                Ok(())
            }
            notify::DELETE => {
                self.icons.remove(at.ok_or(Code::NotFound)?);
                Ok(())
            }
            notify::SET_VERSION => {
                self.icons[at.ok_or(Code::NotFound)?].version = r.version;
                Ok(())
            }
            // NIM_SETFOCUS: the icon is in the flyout; nothing to move.
            _ => at.map(|_| ()).ok_or(Code::NotFound),
        }
    }
    fn update(&mut self, at: usize, r: &Request<'_>, bgra: Option<Vec<u8>>) {
        let icon = &mut self.icons[at];
        if r.flags & notify::TIP != 0 {
            icon.tip = String::from(r.tip);
        }
        if r.flags & notify::ICON != 0 {
            icon.bgra = bgra.unwrap_or_default();
        }
    }
    /// The owner's connection ended: its icons go (Explorer drops a dead
    /// program's icon the next time the pointer passes over it).
    pub fn exited(&mut self, owner: Owner) {
        self.icons.retain(|i| i.owner != owner);
        self.events.retain(|(o, _)| *o != owner);
    }
    pub fn take_event(&mut self, owner: Owner) -> Option<w::Event> {
        let at = self.events.iter().position(|(o, _)| *o == owner)?;
        Some(self.events.remove(at).1)
    }
    /// A click on icon `index` (in flyout order) at its screen `rect`.
    pub fn click(&mut self, index: usize, kind: u32, rect: Rect) -> bool {
        let Some(icon) = self.icons.get(index) else {
            return false;
        };
        if self.events.iter().filter(|(o, _)| *o == icon.owner).count() >= 16 {
            return false;
        }
        self.events.push((
            icon.owner,
            w::Event {
                kind: w::NOTIFY_ICON_EVENT,
                detail: u32::from(icon.version),
                window: 0,
                value: icon.window,
                rect: w::Rect {
                    x: rect.x(),
                    y: rect.y(),
                    width: rect.width(),
                    height: rect.height(),
                },
                a: icon.id,
                b: kind,
                timestamp: 0,
            },
        ));
        true
    }
    /// What a primary click on icon `index` at `now` (ms) is: a double
    /// click when it follows one on the same icon within 500 ms.
    pub fn primary(&mut self, index: usize, now: u64) -> u32 {
        let double = self
            .last
            .is_some_and(|(i, at)| i == index && now.saturating_sub(at) <= 500);
        self.last = (!double).then_some((index, now));
        if double { DOUBLE } else { LEFT }
    }
    pub fn len(&self) -> usize {
        self.icons.len()
    }
    pub fn is_empty(&self) -> bool {
        self.icons.is_empty()
    }
}

/// The "Show hidden icons" chevron: left of (above, on a vertical bar) the
/// microphone slot, or in it when nothing records.
pub fn chevron(layout: Layout, microphone: bool) -> Rect {
    let mic = layout.microphone();
    if !microphone {
        return mic;
    }
    if layout.vertical() {
        Rect::new(
            mic.x(),
            mic.y() - mic.height() as i32 - 4,
            mic.width(),
            mic.height(),
        )
        .unwrap()
    } else {
        Rect::new(
            mic.x() - mic.width() as i32,
            mic.y(),
            mic.width(),
            mic.height(),
        )
        .unwrap()
    }
}
/// The overflow flyout for `count` icons, beside the bar at the chevron.
pub fn flyout(layout: Layout, microphone: bool, count: usize) -> Rect {
    let count = count.max(1);
    let columns = count.min(COLUMNS) as u32;
    let rows = count.div_ceil(COLUMNS) as u32;
    layout.beside_bar(
        chevron(layout, microphone),
        columns * CELL + 2 * PAD,
        rows * CELL + 2 * PAD,
        12,
    )
}
pub fn cell(flyout: Rect, index: usize) -> Rect {
    let (col, row) = ((index % COLUMNS) as i32, (index / COLUMNS) as i32);
    Rect::new(
        flyout.x() + PAD as i32 + col * CELL as i32,
        flyout.y() + PAD as i32 + row * CELL as i32,
        CELL,
        CELL,
    )
    .unwrap()
}
pub fn hit(flyout: Rect, count: usize, x: i32, y: i32) -> Option<usize> {
    (0..count).find(|i| cell(flyout, *i).contains(x, y))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(op: u8, flags: u16, id: u32, tip: &str) -> Vec<u8> {
        let mut b = alloc::vec![op, 4];
        b.extend_from_slice(&flags.to_le_bytes());
        b.extend_from_slice(&7u64.to_le_bytes());
        b.extend_from_slice(&id.to_le_bytes());
        b.extend_from_slice(&0x8001u32.to_le_bytes());
        b.push(tip.len() as u8);
        b.extend_from_slice(tip.as_bytes());
        b
    }
    #[test]
    fn icons_are_added_changed_clicked_and_removed_per_owner() {
        let owner = |pid| Owner {
            pid,
            session: 1,
            epoch: 1,
        };
        let (a, b) = (owner(1), owner(2));
        let mut s = Icons::default();
        let add = request(notify::ADD, notify::TIP | notify::ICON, 1, "Sync");
        let pixels = alloc::vec![0u8; notify::ICON_BYTES];
        s.apply(a, &decode(&add).unwrap(), Some(pixels.clone()))
            .unwrap();
        assert_eq!(s.apply(a, &decode(&add).unwrap(), None), Err(Code::Exists));
        // Another owner's icon with the same window and id is its own.
        s.apply(b, &decode(&add).unwrap(), None).unwrap();
        assert_eq!(s.len(), 2);
        let tip = request(notify::MODIFY, notify::TIP, 1, "Up to date");
        s.apply(a, &decode(&tip).unwrap(), None).unwrap();
        assert_eq!(s.icons[0].tip, "Up to date");
        assert_eq!(s.icons[0].bgra.len(), notify::ICON_BYTES);
        let version = request(notify::SET_VERSION, 0, 1, "");
        s.apply(a, &decode(&version).unwrap(), None).unwrap();
        assert_eq!(s.icons[0].version, 4);
        // Wrong pixel size, bad tip, unknown icon.
        assert!(
            s.apply(a, &decode(&tip).unwrap(), Some(alloc::vec![0; 3]))
                .is_err()
        );
        assert!(decode(&request(notify::ADD, 0, 1, "a\u{7}b")).is_err());
        let gone = request(notify::DELETE, 0, 9, "");
        assert_eq!(
            s.apply(a, &decode(&gone).unwrap(), None),
            Err(Code::NotFound)
        );
        // A click reaches only the owner.
        let r = Rect::new(10, 20, 40, 40).unwrap();
        assert!(s.click(0, LEFT, r));
        assert!(s.take_event(b).is_none());
        let e = s.take_event(a).unwrap();
        assert_eq!(
            (e.kind, e.a, e.b, e.value, e.detail),
            (w::NOTIFY_ICON_EVENT, 1, LEFT, 7, 4)
        );
        s.exited(a);
        assert_eq!(s.len(), 1);
        for i in 0..notify::MAX_ICONS as u32 {
            let _ = s.apply(
                a,
                &decode(&request(notify::ADD, 0, 100 + i, "")).unwrap(),
                None,
            );
        }
        assert_eq!(
            s.apply(a, &decode(&request(notify::ADD, 0, 999, "")).unwrap(), None),
            Err(Code::Resources)
        );
    }
    #[test]
    fn the_flyout_grid_wraps_at_five_and_hits_cells() {
        let layout = Layout::new(1280, 720);
        let f = flyout(layout, false, 7);
        assert_eq!(f.width(), 5 * CELL + 2 * PAD);
        assert_eq!(f.height(), 2 * CELL + 2 * PAD);
        // Above the bar, near the chevron.
        assert!(f.bottom() <= layout.work_area().bottom());
        let c = cell(f, 6);
        assert_eq!(hit(f, 7, c.x() + 1, c.y() + 1), Some(6));
        assert_eq!(hit(f, 7, f.x(), f.y()), None);
        assert!(chevron(layout, true).x() < chevron(layout, false).x());
        let mut s = Icons::default();
        assert_eq!(s.primary(0, 1000), LEFT);
        assert_eq!(s.primary(0, 1400), DOUBLE);
        assert_eq!(s.primary(0, 1500), LEFT);
        assert_eq!(s.primary(1, 1600), LEFT);
        assert_eq!(s.primary(1, 2500), LEFT);
    }
}
