//! Physical Win-key chords. Bare Win activates on release; a chord never also
//! opens Start, and its key-up cannot leak into an application after focus changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemCommand {
    Start,
    Search,
    QuickSettings,
    Notifications,
    Files,
    Settings,
    Desktop,
    TaskView,
    NewDesktop,
    CloseDesktop,
    PreviousDesktop,
    NextDesktop,
    Left,
    Right,
    Up,
    Down,
    /// Win+X quick-link menu.
    QuickLink,
    /// Win+L lock.
    Lock,
    /// Win+M minimize all windows.
    MinimizeAll,
    /// Win+Z snap layouts for the focused window.
    SnapLayouts,
    /// Win+1..9: the Nth taskbar app (0-based).
    Taskbar(u8),
    /// Win+R: the Run dialog.
    Run,
    /// Win+V: clipboard history.
    ClipboardHistory,
    /// Win+K: Quick settings' Cast page.
    Cast,
    /// Win+P: Quick settings' Project page.
    Project,
    /// Win+Shift+S or Print Screen: the snip overlay.
    Snip,
    /// Left Alt + left Shift + Print Screen: contrast themes on/off.
    ToggleContrast,
    /// Win+Ctrl+left Shift+F12: DISPLAY-01 presentation fault injection.
    /// Acted on only when Init enabled the test-image hook; else ignored.
    PresentFault,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemEvent {
    Pass,
    Consumed,
    Command(SystemCommand),
}
#[derive(Default)]
pub struct SystemKeys {
    held: u8,
    used: bool,
    swallowed: [u64; 8],
    /// Left Alt and left Shift (the contrast shortcut wants the left keys).
    left_alt: bool,
    left_shift: bool,
    right_shift: bool,
    alt: bool,
}
impl SystemKeys {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn event(&mut self, scan: u32, down: bool, control: bool) -> SystemEvent {
        use SystemCommand as C;
        match scan {
            56 => {
                self.left_alt = down;
                self.alt = down;
            }
            0x138 => self.alt = down,
            42 => self.left_shift = down,
            54 => self.right_shift = down,
            _ => (),
        }
        // Print Screen (SysRq while Alt is held) with left Alt and left Shift.
        if down && matches!(scan, 0x137 | 0x54) && self.left_alt && self.left_shift {
            if let Some(bits) = self.swallowed.get_mut(scan as usize / 64) {
                *bits |= 1u64 << (scan % 64);
            }
            return SystemEvent::Command(C::ToggleContrast);
        }
        // Print Screen alone opens the snip overlay (Windows 11's default).
        if down && matches!(scan, 0x137 | 0x54) && !self.alt && self.held == 0 {
            if let Some(bits) = self.swallowed.get_mut(scan as usize / 64) {
                *bits |= 1u64 << (scan % 64);
            }
            return SystemEvent::Command(C::Snip);
        }
        let win = match scan {
            0x15b => 1,
            0x15c => 2,
            _ => 0,
        };
        if win != 0 {
            if down {
                if self.held == 0 {
                    self.used = false;
                }
                self.held |= win;
                return SystemEvent::Consumed;
            }
            let was = self.held & win != 0;
            self.held &= !win;
            return if was && self.held == 0 && !self.used {
                SystemEvent::Command(C::Start)
            } else {
                SystemEvent::Consumed
            };
        }
        let Some(bits) = self.swallowed.get_mut(scan as usize / 64) else {
            return SystemEvent::Consumed;
        };
        let mask = 1u64 << (scan % 64);
        if !down && *bits & mask != 0 {
            *bits &= !mask;
            return SystemEvent::Consumed;
        }
        if self.held == 0 {
            return SystemEvent::Pass;
        }
        if !down {
            return SystemEvent::Consumed;
        }
        self.used = true;
        *bits |= mask;
        let command = match (control, scan) {
            (true, 0x58) if self.left_shift => C::PresentFault,
            (true, 32) => C::NewDesktop,
            (true, 62) => C::CloseDesktop,
            (true, 0x14b) => C::PreviousDesktop,
            (true, 0x14d) => C::NextDesktop,
            (false, 15) => C::TaskView,
            (false, 18) => C::Files,
            (false, 23) => C::Settings,
            (false, 31) if self.left_shift || self.right_shift => C::Snip,
            (false, 31 | 16) => C::Search,
            (false, 45) => C::QuickLink,
            (false, 19) => C::Run,
            (false, 47) => C::ClipboardHistory,
            (false, 37) => C::Cast,
            (false, 25) => C::Project,
            (false, 38) => C::Lock,
            (false, 50) => C::MinimizeAll,
            (false, 44) => C::SnapLayouts,
            (false, 2..=10) => C::Taskbar((scan - 2) as u8),
            (false, 30) => C::QuickSettings,
            (false, 49) => C::Notifications,
            (false, 32) => C::Desktop,
            (false, 0x14b) => C::Left,
            (false, 0x14d) => C::Right,
            (false, 0x148) => C::Up,
            (false, 0x150) => C::Down,
            _ => return SystemEvent::Consumed,
        };
        SystemEvent::Command(command)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bare_win_chord_releases_repetition_and_reset_are_distinct() {
        let mut s = SystemKeys::default();
        assert_eq!(s.event(0x15b, true, false), SystemEvent::Consumed);
        assert_eq!(
            s.event(0x15b, false, false),
            SystemEvent::Command(SystemCommand::Start)
        );
        s.event(0x15b, true, false);
        assert_eq!(
            s.event(0x14b, true, false),
            SystemEvent::Command(SystemCommand::Left)
        );
        assert_eq!(s.event(0x15b, false, false), SystemEvent::Consumed);
        assert_eq!(s.event(0x14b, false, false), SystemEvent::Consumed);
        assert_eq!(s.event(30, true, false), SystemEvent::Pass);
        s.event(0x15c, true, false);
        s.reset();
        assert_eq!(s.event(0x15c, false, false), SystemEvent::Consumed);
        s.event(0x15b, true, false);
        assert_eq!(
            s.event(32, true, true),
            SystemEvent::Command(SystemCommand::NewDesktop)
        );
        s.event(0x15c, true, false);
        assert_eq!(s.event(0x15b, false, false), SystemEvent::Consumed);
        assert_eq!(s.event(0x15c, false, false), SystemEvent::Consumed);
    }
    #[test]
    fn a_win_l_whose_release_the_lock_gate_took_is_cleared_by_reset() {
        // signin.rs `gate_unlock`: the releases of Win+L reach the gate, not
        // this tracker, so Win would stay held and "l" would lock again.
        let mut s = SystemKeys::default();
        s.event(0x15b, true, false);
        assert_eq!(
            s.event(38, true, false),
            SystemEvent::Command(SystemCommand::Lock)
        );
        assert_eq!(
            s.event(38, true, false),
            SystemEvent::Command(SystemCommand::Lock)
        );
        s.reset();
        assert_eq!(s.event(38, true, false), SystemEvent::Pass);
    }
    #[test]
    fn windows_chords_map_to_shell_commands() {
        for (scan, command) in [
            (18, SystemCommand::Files),
            (23, SystemCommand::Settings),
            (32, SystemCommand::Desktop),
            (50, SystemCommand::MinimizeAll),
            (15, SystemCommand::TaskView),
            (45, SystemCommand::QuickLink),
            (38, SystemCommand::Lock),
            (31, SystemCommand::Search),
            (16, SystemCommand::Search),
            (44, SystemCommand::SnapLayouts),
            (19, SystemCommand::Run),
            (47, SystemCommand::ClipboardHistory),
            (2, SystemCommand::Taskbar(0)),
            (10, SystemCommand::Taskbar(8)),
            (0x148, SystemCommand::Up),
        ] {
            let mut keys = SystemKeys::default();
            keys.event(0x15b, true, false);
            assert_eq!(keys.event(scan, true, false), SystemEvent::Command(command));
            assert_eq!(keys.event(scan, false, false), SystemEvent::Consumed);
            assert_eq!(
                keys.event(0x15b, false, false),
                SystemEvent::Consumed,
                "no Start after a chord"
            );
        }
        // Digits without Win reach applications.
        assert_eq!(
            SystemKeys::default().event(2, true, false),
            SystemEvent::Pass
        );
    }
    #[test]
    fn win_ctrl_left_shift_f12_is_the_presentation_fault_chord() {
        let mut keys = SystemKeys::default();
        keys.event(0x15b, true, false);
        keys.event(42, true, true);
        assert_eq!(
            keys.event(0x58, true, true),
            SystemEvent::Command(SystemCommand::PresentFault)
        );
        assert_eq!(keys.event(0x58, false, true), SystemEvent::Consumed);
        // Without Shift or Ctrl it is no command.
        let mut keys = SystemKeys::default();
        keys.event(0x15b, true, false);
        assert_eq!(keys.event(0x58, true, true), SystemEvent::Consumed);
        assert_eq!(keys.event(0x58, true, false), SystemEvent::Consumed);
    }
    #[test]
    fn left_alt_left_shift_print_screen_toggles_contrast() {
        for print in [0x137, 0x54] {
            let mut keys = SystemKeys::default();
            assert_eq!(keys.event(56, true, false), SystemEvent::Pass);
            assert_eq!(keys.event(42, true, false), SystemEvent::Pass);
            assert_eq!(
                keys.event(print, true, false),
                SystemEvent::Command(SystemCommand::ToggleContrast)
            );
            assert_eq!(keys.event(print, false, false), SystemEvent::Consumed);
            assert_eq!(keys.event(42, false, false), SystemEvent::Pass);
            assert_eq!(keys.event(56, false, false), SystemEvent::Pass);
        }
        // Right Alt or right Shift, or Print Screen alone, do not toggle.
        let mut keys = SystemKeys::default();
        keys.event(0x138, true, false);
        keys.event(42, true, false);
        assert_eq!(keys.event(0x137, true, false), SystemEvent::Pass);
        let mut keys = SystemKeys::default();
        keys.event(56, true, false);
        keys.event(54, true, false);
        assert_eq!(keys.event(0x137, true, false), SystemEvent::Pass);
        // Print Screen alone opens the snip overlay; Win+Shift+S too.
        let mut keys = SystemKeys::default();
        assert_eq!(
            keys.event(0x137, true, false),
            SystemEvent::Command(SystemCommand::Snip)
        );
        assert_eq!(keys.event(0x137, false, false), SystemEvent::Consumed);
        let mut keys = SystemKeys::default();
        keys.event(0x15b, true, false);
        keys.event(42, true, false);
        assert_eq!(
            keys.event(31, true, false),
            SystemEvent::Command(SystemCommand::Snip)
        );
    }
    #[test]
    fn tray_chords_consume_both_releases_without_opening_start() {
        for (scan, command) in [
            (30, SystemCommand::QuickSettings),
            (49, SystemCommand::Notifications),
        ] {
            let mut keys = SystemKeys::default();
            keys.event(0x15b, true, false);
            assert_eq!(keys.event(scan, true, false), SystemEvent::Command(command));
            assert_eq!(keys.event(0x15b, false, false), SystemEvent::Consumed);
            assert_eq!(keys.event(scan, false, false), SystemEvent::Consumed);
            assert_eq!(keys.event(scan, true, false), SystemEvent::Pass);
        }
    }
}
