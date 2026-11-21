//! Window relations for Windows GUI clients (CAP_WIN32, native-window-v1
//! "Win32 window relations"): owned windows, disabled owners under a modal
//! window, client-initiated move/resize bound to the current press, and
//! minimum/maximum size limits. The server applies them in input routing
//! and interactive geometry; it never calls back into the client, so a hung
//! client cannot stall these decisions.
use alloc::vec::Vec;
use door_service_protocol::Code;
use door_service_protocol::window::{EDGE_BOTTOM, EDGE_LEFT, EDGE_RIGHT, EDGE_TOP};

/// Bounded relation records (one per window that uses any relation).
pub const MAX_ENTRIES: usize = 64;

/// HELLO: CAP_WIN32 (window relations, frameless windows, the Win32 close
/// and not-responding semantics) is granted only to Windows-personality
/// processes, decided from the kernel-authenticated manifest. Every other
/// client keeps the native behaviour and is refused the Win32 requests.
pub const fn grant(manifest: u64, features: u32) -> u32 {
    use door_abi::platform::manifest::{WINDOWS_APPLICATION, WINDOWS_GUI};
    if manifest == WINDOWS_GUI || manifest == WINDOWS_APPLICATION {
        features
    } else {
        features & !door_service_protocol::window::CAP_WIN32
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Limits {
    pub min_width: u32,
    pub min_height: u32,
    /// 0 = unbounded.
    pub max_width: u32,
    pub max_height: u32,
}
impl Limits {
    pub fn validate(self) -> Result<Self, Code> {
        let bad = |min: u32, max: u32| max != 0 && min > max;
        if bad(self.min_width, self.max_width)
            || bad(self.min_height, self.max_height)
            || self.min_width > 16_384
            || self.min_height > 16_384
        {
            return Err(Code::InvalidRequest);
        }
        Ok(self)
    }
    /// Clamp an outer size.
    pub fn clamp(self, width: u32, height: u32) -> (u32, u32) {
        let c = |v: u32, min: u32, max: u32| {
            let v = v.max(min);
            if max != 0 { v.min(max) } else { v }
        };
        (
            c(width, self.min_width, self.max_width),
            c(height, self.min_height, self.max_height),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    window: u64,
    owner: Option<u64>,
    enabled: bool,
    limits: Limits,
    /// Order of the last enable/owner change, for "topmost owned window".
    serial: u64,
}

#[derive(Default)]
pub struct Relations {
    entries: Vec<Entry>,
    serial: u64,
}

impl Relations {
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
            serial: 0,
        }
    }
    fn entry(&mut self, window: u64) -> Result<&mut Entry, Code> {
        if let Some(i) = self.entries.iter().position(|e| e.window == window) {
            return Ok(&mut self.entries[i]);
        }
        if self.entries.len() >= MAX_ENTRIES {
            return Err(Code::Resources);
        }
        self.entries.push(Entry {
            window,
            owner: None,
            enabled: true,
            limits: Limits::default(),
            serial: 0,
        });
        Ok(self.entries.last_mut().unwrap())
    }
    fn get(&self, window: u64) -> Option<&Entry> {
        self.entries.iter().find(|e| e.window == window)
    }
    pub fn owner(&self, window: u64) -> Option<u64> {
        self.get(window).and_then(|e| e.owner)
    }
    /// SET_OWNER: the caller already proved it owns both windows. Self and
    /// cyclic ownership are refused.
    pub fn set_owner(&mut self, window: u64, owner: Option<u64>) -> Result<(), Code> {
        if let Some(o) = owner {
            let mut cur = Some(o);
            let mut steps = 0;
            while let Some(c) = cur {
                if c == window || steps > MAX_ENTRIES {
                    return Err(Code::InvalidRequest);
                }
                cur = self.owner(c);
                steps += 1;
            }
        }
        self.serial += 1;
        let serial = self.serial;
        let e = self.entry(window)?;
        e.owner = owner;
        e.serial = serial;
        Ok(())
    }
    pub fn set_enabled(&mut self, window: u64, enabled: bool) -> Result<(), Code> {
        self.serial += 1;
        let serial = self.serial;
        let e = self.entry(window)?;
        e.enabled = enabled;
        e.serial = serial;
        Ok(())
    }
    pub fn enabled(&self, window: u64) -> bool {
        self.get(window).is_none_or(|e| e.enabled)
    }
    pub fn set_limits(&mut self, window: u64, limits: Limits) -> Result<(), Code> {
        let limits = limits.validate()?;
        self.entry(window)?.limits = limits;
        Ok(())
    }
    pub fn limits(&self, window: u64) -> Limits {
        self.get(window).map(|e| e.limits).unwrap_or_default()
    }
    /// Windows directly owned by `window`.
    pub fn owned(&self, window: u64) -> Vec<u64> {
        self.entries
            .iter()
            .filter(|e| e.owner == Some(window))
            .map(|e| e.window)
            .collect()
    }
    /// Where input aimed at a disabled window goes: its most recently
    /// related enabled owned window, followed down the ownership chain
    /// (a modal dialog, or a message box above it). None when enabled.
    pub fn modal_target(&self, window: u64) -> Option<u64> {
        if self.enabled(window) {
            return None;
        }
        let mut cur = window;
        for _ in 0..MAX_ENTRIES {
            let next = self
                .entries
                .iter()
                .filter(|e| e.owner == Some(cur))
                .max_by_key(|e| e.serial)?;
            if next.enabled {
                // A deeper enabled window owned by this one wins.
                match self.modal_target_enabled(next.window) {
                    Some(deeper) => return Some(deeper),
                    None => return Some(next.window),
                }
            }
            cur = next.window;
        }
        None
    }
    /// A key while `focus` holds the keyboard focus: the window to activate
    /// before the key is delivered (a disabled owner's modal, on a press).
    /// The key itself is always delivered to the focused client afterwards,
    /// as Windows queues every key on the thread and its focus decides:
    /// dropping keys aimed at a disabled owner lost the Ctrl and S releases
    /// of the Ctrl+S that opened a Save As dialog whenever they arrived
    /// between the owner's disable and the dialog's activation, so the
    /// client kept Ctrl down and the typed file name became Ctrl chords.
    pub fn key_activation(&self, focus: u64, down: bool) -> Option<u64> {
        down.then(|| self.modal_target(focus)).flatten()
    }
    fn modal_target_enabled(&self, window: u64) -> Option<u64> {
        self.entries
            .iter()
            .filter(|e| e.owner == Some(window) && e.enabled)
            .max_by_key(|e| e.serial)
            .map(|e| self.modal_target_enabled(e.window).unwrap_or(e.window))
    }
    /// A destroyed window: drop its record; windows it owned become unowned.
    pub fn forget(&mut self, window: u64) {
        self.entries.retain(|e| e.window != window);
        for e in &mut self.entries {
            if e.owner == Some(window) {
                e.owner = None;
            }
        }
        self.entries
            .retain(|e| e.owner.is_some() || !e.enabled || e.limits != Limits::default());
    }
}

/// The left-button press a MOVE_RESIZE may continue: the window that got
/// the press and the INPUT event timestamp the client saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Press {
    pub window: u64,
    pub serial: u64,
}

/// MOVE_RESIZE admission: the primary button is still held, the press went
/// to this window with this serial, no drag is running and the edge set is
/// a valid move (0) or border combination.
pub fn admit_move_resize(
    press: Option<Press>,
    held: bool,
    dragging: bool,
    window: u64,
    serial: u64,
    edges: u32,
) -> Result<(), Code> {
    let valid_edges = edges & !(EDGE_LEFT | EDGE_TOP | EDGE_RIGHT | EDGE_BOTTOM) == 0
        && edges & (EDGE_LEFT | EDGE_RIGHT) != EDGE_LEFT | EDGE_RIGHT
        && edges & (EDGE_TOP | EDGE_BOTTOM) != EDGE_TOP | EDGE_BOTTOM;
    if !valid_edges {
        return Err(Code::InvalidRequest);
    }
    match press {
        Some(p) if held && !dragging && p.window == window && p.serial == serial => Ok(()),
        _ => Err(Code::StaleInstance),
    }
}

/// Protocol edge bits to the server drag bits (1 left, 2 right, 4 top,
/// 8 bottom; `CursorIcon::resize_edges`).
pub const fn server_edges(edges: u32) -> u8 {
    (edges & EDGE_LEFT != 0) as u8
        | (((edges & EDGE_RIGHT != 0) as u8) << 1)
        | (((edges & EDGE_TOP != 0) as u8) << 2)
        | (((edges & EDGE_BOTTOM != 0) as u8) << 3)
}
pub const fn protocol_edges(edges: u8) -> u32 {
    (if edges & 1 != 0 { EDGE_LEFT } else { 0 })
        | (if edges & 2 != 0 { EDGE_RIGHT } else { 0 })
        | (if edges & 4 != 0 { EDGE_TOP } else { 0 })
        | (if edges & 8 != 0 { EDGE_BOTTOM } else { 0 })
}

/// Clamp an interactively resized rectangle, keeping the edges opposite
/// the dragged ones anchored. `(x, y, w, h)` in, clamped out.
pub fn clamp_resize(
    limits: Limits,
    edges: u32,
    rect: (i32, i32, u32, u32),
) -> (i32, i32, u32, u32) {
    let (x, y, w, h) = rect;
    let (cw, ch) = limits.clamp(w, h);
    let nx = if edges & EDGE_LEFT != 0 {
        x + w as i32 - cw as i32
    } else {
        x
    };
    let ny = if edges & EDGE_TOP != 0 {
        y + h as i32 - ch as i32
    } else {
        y
    };
    (nx, ny, cw, ch)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owners_refuse_cycles_and_forget_on_destroy() {
        let mut r = Relations::new();
        r.set_owner(2, Some(1)).unwrap();
        r.set_owner(3, Some(2)).unwrap();
        assert_eq!(r.set_owner(1, Some(3)), Err(Code::InvalidRequest), "cycle");
        assert_eq!(r.set_owner(4, Some(4)), Err(Code::InvalidRequest), "self");
        assert_eq!(r.owned(1), [2]);
        r.forget(2);
        assert_eq!(r.owner(3), None, "orphaned owned window");
        assert!(r.owned(1).is_empty());
        r.set_owner(5, None).unwrap();
        r.forget(9);
        assert!(
            r.entries.iter().all(|e| e.window != 5),
            "empty records pruned"
        );
    }
    #[test]
    fn disabled_owner_redirects_to_the_modal_chain() {
        let mut r = Relations::new();
        // main(1) -> dialog(2) -> message box(3) over the dialog.
        r.set_owner(2, Some(1)).unwrap();
        r.set_enabled(1, false).unwrap();
        assert_eq!(r.modal_target(1), Some(2));
        assert_eq!(r.modal_target(2), None, "enabled windows take input");
        r.set_owner(3, Some(2)).unwrap();
        r.set_enabled(2, false).unwrap();
        assert_eq!(r.modal_target(1), Some(3));
        assert_eq!(r.modal_target(2), Some(3));
        r.forget(3);
        r.set_enabled(2, true).unwrap();
        assert_eq!(r.modal_target(1), Some(2));
        r.set_enabled(1, true).unwrap();
        assert_eq!(r.modal_target(1), None);
        let mut lone = Relations::new();
        lone.set_enabled(7, false).unwrap();
        assert_eq!(lone.modal_target(7), None, "disabled, nothing to activate");
        assert!(!lone.enabled(7));
    }
    #[test]
    fn keys_at_a_disabled_owner_activate_its_modal_only_on_a_press() {
        let mut r = Relations::new();
        // Ctrl+S: the owner is disabled before its Save As dialog exists...
        r.set_enabled(1, false).unwrap();
        assert_eq!(r.key_activation(1, true), None, "no modal yet");
        assert_eq!(r.key_activation(1, false), None);
        // ...and the dialog appears: a press raises it, a release never
        // activates anything (it is delivered to the focused client).
        r.set_owner(2, Some(1)).unwrap();
        assert_eq!(r.key_activation(1, true), Some(2));
        assert_eq!(r.key_activation(1, false), None);
        assert_eq!(r.key_activation(2, true), None, "the dialog takes keys");
        r.set_enabled(1, true).unwrap();
        assert_eq!(r.key_activation(1, true), None, "enabled owners take keys");
    }
    #[test]
    fn limits_validate_and_clamp_with_anchors() {
        let mut r = Relations::new();
        let l = Limits {
            min_width: 200,
            min_height: 100,
            max_width: 800,
            max_height: 0,
        };
        r.set_limits(1, l).unwrap();
        assert_eq!(r.limits(1).clamp(50, 5000), (200, 5000));
        assert_eq!(r.limits(1).clamp(900, 50), (800, 100));
        assert_eq!(
            r.set_limits(
                1,
                Limits {
                    min_width: 9,
                    max_width: 8,
                    ..l
                }
            ),
            Err(Code::InvalidRequest)
        );
        // Dragging the left edge keeps the right edge at 300.
        assert_eq!(
            clamp_resize(l, EDGE_LEFT, (250, 0, 50, 300)),
            (100, 0, 200, 300)
        );
        assert_eq!(
            clamp_resize(l, EDGE_RIGHT | EDGE_BOTTOM, (0, 0, 50, 50)),
            (0, 0, 200, 100)
        );
        assert_eq!(
            clamp_resize(l, EDGE_TOP, (0, 100, 300, 40)),
            (0, 40, 300, 100)
        );
    }
    #[test]
    fn move_resize_needs_the_live_press() {
        let p = Some(Press {
            window: 4,
            serial: 77,
        });
        assert_eq!(admit_move_resize(p, true, false, 4, 77, 0), Ok(()));
        assert_eq!(
            admit_move_resize(p, true, false, 4, 77, EDGE_LEFT | EDGE_TOP),
            Ok(())
        );
        assert_eq!(
            admit_move_resize(p, false, false, 4, 77, 0),
            Err(Code::StaleInstance),
            "released"
        );
        assert_eq!(
            admit_move_resize(p, true, true, 4, 77, 0),
            Err(Code::StaleInstance),
            "dragging"
        );
        assert_eq!(
            admit_move_resize(p, true, false, 5, 77, 0),
            Err(Code::StaleInstance),
            "other window"
        );
        assert_eq!(
            admit_move_resize(p, true, false, 4, 76, 0),
            Err(Code::StaleInstance),
            "old press"
        );
        assert_eq!(
            admit_move_resize(None, true, false, 4, 77, 0),
            Err(Code::StaleInstance)
        );
        assert_eq!(
            admit_move_resize(p, true, false, 4, 77, EDGE_LEFT | EDGE_RIGHT),
            Err(Code::InvalidRequest)
        );
        assert_eq!(
            admit_move_resize(p, true, false, 4, 77, 16),
            Err(Code::InvalidRequest)
        );
    }
    #[test]
    fn edge_encodings_roundtrip() {
        for e in 0..16u32 {
            assert_eq!(protocol_edges(server_edges(e)), e);
        }
        assert_eq!(server_edges(EDGE_TOP | EDGE_RIGHT), 2 | 4);
    }
    #[test]
    fn cap_win32_is_granted_only_to_windows_personality_manifests() {
        use door_abi::platform::manifest as m;
        use door_service_protocol::window::{CAP_MULTI_WINDOW, CAP_WIN32};
        let asked = CAP_WIN32 | CAP_MULTI_WINDOW;
        assert_eq!(grant(m::WINDOWS_GUI, asked), asked);
        assert_eq!(grant(m::WINDOWS_APPLICATION, asked), asked);
        for other in [
            m::WORKSPACE,
            m::SHELL,
            m::APPLICATION,
            m::PREVIEW,
            m::GALLERY,
            0,
        ] {
            assert_eq!(grant(other, asked), CAP_MULTI_WINDOW, "manifest {other}");
        }
    }
    #[test]
    fn records_are_bounded() {
        let mut r = Relations::new();
        for w in 0..MAX_ENTRIES as u64 {
            r.set_enabled(w + 1, false).unwrap();
        }
        assert_eq!(r.set_enabled(1000, false), Err(Code::Resources));
    }
}
