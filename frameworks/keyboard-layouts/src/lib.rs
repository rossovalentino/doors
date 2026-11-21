#![no_std]
//! Door keyboard stack (§53): physical keys, Windows virtual keys, layouts
//! with dead keys and AltGr, Num/Caps Lock semantics, installed input
//! methods and the Windows input-method hotkeys. Allocation-free; shared by
//! the window server (translation and switching) and the IME engines.
pub mod layout;
pub mod layouts;
pub mod methods;
pub mod physical;
pub mod vk;
use door_abi::graphics as g;
pub use layout::{Layout, Text, Translation, Translator};
pub use methods::{Ime, Indicator, Method, Preferences};
/// An input-method switch requested by a Windows hotkey.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    /// Win+Space (with Shift: previous).
    Next,
    Previous,
    /// Left Alt+Shift.
    NextLanguage,
    /// Ctrl+Shift.
    NextLayout,
}
/// Windows 11 default input hotkeys. Win+Space switches immediately and is
/// consumed (with its release); Left Alt+Shift and Ctrl+Shift switch when
/// the chord is released with no other key pressed in between, and their
/// key events still reach applications (Windows behaviour).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hotkeys {
    armed: Option<Switch>,
    other: bool,
    space_down: bool,
}
impl Hotkeys {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// One KEY event (flags = state after the event). Returns the switch to
    /// perform and whether the event is consumed by the shell.
    pub fn event(&mut self, code: u32, flags: u32) -> (Option<Switch>, bool) {
        let down = flags & g::DOWN != 0;
        if code == 0x39 {
            if down && flags & g::LOGO != 0 {
                self.space_down = true;
                self.other = true;
                let s = if flags & g::SHIFT != 0 {
                    Switch::Previous
                } else {
                    Switch::Next
                };
                return (Some(s), true);
            }
            if !down && core::mem::take(&mut self.space_down) {
                return (None, true);
            }
        }
        if down && !physical::modifier(code) {
            self.other = true;
        }
        let left_alt = flags & g::ALT != 0 && flags & g::RIGHT_ALT == 0;
        let held = flags & (g::ALT | g::SHIFT | g::CONTROL | g::LOGO);
        if held == 0 {
            let fire = self.armed.take().filter(|_| !self.other);
            self.other = false;
            return (fire, false);
        }
        if !self.other {
            if held == g::ALT | g::SHIFT && left_alt {
                self.armed = Some(Switch::NextLanguage);
            } else if held == g::CONTROL | g::SHIFT {
                self.armed = Some(Switch::NextLayout);
            } else if held & !(g::SHIFT | g::ALT | g::CONTROL) != 0 {
                self.armed = None;
            }
        }
        (None, false)
    }
}
impl Switch {
    pub fn apply(self, p: Preferences) -> Preferences {
        match self {
            Switch::Next => p.next(false),
            Switch::Previous => p.next(true),
            Switch::NextLanguage => p.next_language(),
            Switch::NextLayout => p.next_layout(),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    const LS: u32 = g::SHIFT | g::LEFT_SHIFT;
    const LA: u32 = g::ALT | g::LEFT_ALT;
    const LC: u32 = g::CONTROL | g::LEFT_CONTROL;
    const WIN: u32 = g::LOGO | g::LEFT_LOGO;
    #[test]
    fn win_space_switches_and_swallows_its_release() {
        let mut h = Hotkeys::default();
        assert_eq!(h.event(0x15b, g::DOWN | WIN), (None, false));
        assert_eq!(h.event(0x39, g::DOWN | WIN), (Some(Switch::Next), true));
        assert_eq!(
            h.event(0x39, g::DOWN | WIN | LS),
            (Some(Switch::Previous), true)
        );
        assert_eq!(h.event(0x39, WIN), (None, true));
        assert_eq!(h.event(0x15b, 0), (None, false));
        assert_eq!(h.event(0x39, g::DOWN), (None, false), "plain Space");
    }
    #[test]
    fn alt_shift_and_ctrl_shift_fire_on_clean_release_only() {
        let mut h = Hotkeys::default();
        h.event(0x38, g::DOWN | LA);
        h.event(0x2a, g::DOWN | LA | LS);
        h.event(0x2a, LA);
        assert_eq!(h.event(0x38, 0), (Some(Switch::NextLanguage), false));
        h.event(0x1d, g::DOWN | LC);
        h.event(0x2a, g::DOWN | LC | LS);
        h.event(0x1d, LS);
        assert_eq!(h.event(0x2a, 0), (Some(Switch::NextLayout), false));
        // Ctrl+Shift+N is a shortcut, not a layout switch.
        h.event(0x1d, g::DOWN | LC);
        h.event(0x2a, g::DOWN | LC | LS);
        h.event(0x31, g::DOWN | LC | LS);
        h.event(0x31, LC | LS);
        h.event(0x2a, LC);
        assert_eq!(h.event(0x1d, 0), (None, false));
        // AltGr+Shift is not the language hotkey.
        h.event(0x138, g::DOWN | g::ALT | g::RIGHT_ALT);
        h.event(0x2a, g::DOWN | g::ALT | g::RIGHT_ALT | LS);
        h.event(0x2a, g::ALT | g::RIGHT_ALT);
        assert_eq!(h.event(0x138, 0), (None, false));
        // Win+Shift is neither.
        h.event(0x15b, g::DOWN | WIN);
        h.event(0x2a, g::DOWN | WIN | LS);
        h.event(0x2a, WIN);
        assert_eq!(h.event(0x15b, 0), (None, false));
    }
}
