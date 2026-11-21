//! Magnifier (`window::MAGNIFY`) and On-Screen Keyboard
//! (`window::INJECT_KEY`) service sides: only those apps' kernel-authenticated
//! manifests may ask; the magnification ends with the Magnifier process and
//! the keyboard's windows never take focus.
use super::*;
use door_abi::graphics as d;

impl Desktop {
    fn manifest_of(&self, owner: g::Owner) -> Option<u64> {
        self.clients
            .iter()
            .find(|c| c.owner == owner)
            .map(|c| c.manifest)
    }
    /// Whether `window` belongs to the On-Screen Keyboard (never focused).
    pub(super) fn keyboard_window(&self, window: g::WindowId) -> bool {
        self.engine
            .window_info(window)
            .ok()
            .and_then(|w| self.manifest_of(w.owner))
            == Some(door_abi::platform::manifest::ON_SCREEN_KEYBOARD)
    }
    pub(super) fn magnify_request(&mut self, owner: g::Owner, b: &[u8]) -> Result<Response, Code> {
        if b.len() != 12 {
            return Err(Code::InvalidRequest);
        }
        if self.manifest_of(owner) != Some(door_abi::platform::manifest::MAGNIFIER) {
            return Err(Code::Denied);
        }
        let percent = u32::from_le_bytes(b[..4].try_into().unwrap());
        let view = u32::from_le_bytes(b[4..8].try_into().unwrap());
        if !(100..=1600).contains(&percent) || view > 2 {
            return Err(Code::InvalidRequest);
        }
        self.shell.magnifier = door_window::magnify::Magnify::new(percent, view, owner.pid);
        if self.shell.magnifier.is_none() {
            self.shell.magnified = Vec::new();
        }
        door_rt::diagnostic(format_args!(
            "DOOR_P14 MAGNIFY percent={percent} view={view}\n"
        ));
        self.engine.invalidate();
        self.cursor_dirty = true;
        Ok(Response::empty())
    }
    /// The magnification ends with its process.
    pub(super) fn magnifier_revoked(&mut self, owner: g::Owner) {
        if self.shell.magnifier.is_some_and(|m| m.pid == owner.pid) {
            self.shell.magnifier = None;
            self.shell.magnified = Vec::new();
            self.engine.invalidate();
            self.cursor_dirty = true;
        }
    }
    /// Where the cursor plane draws the pointer.
    pub(super) fn displayed_pointer(&self) -> (i32, i32) {
        match self.shell.magnifier {
            Some(m) => {
                let o = self.engine.output_rect();
                m.pointer(o.width(), o.height(), self.x, self.y)
            }
            None => (self.x, self.y),
        }
    }
    pub(super) fn inject_key_request(
        &mut self,
        owner: g::Owner,
        b: &[u8],
    ) -> Result<Response, Code> {
        if b.len() != 8 {
            return Err(Code::InvalidRequest);
        }
        if self.manifest_of(owner) != Some(door_abi::platform::manifest::ON_SCREEN_KEYBOARD) {
            return Err(Code::Denied);
        }
        let scan = u32::from_le_bytes(b[..4].try_into().unwrap());
        let flags = u32::from_le_bytes(b[4..8].try_into().unwrap());
        if scan == 0 || scan > 0x1ff || flags & !0x30f != 0 {
            return Err(Code::InvalidRequest);
        }
        // Nothing types on the secure desktop: a synthesized Alt+Y must
        // never answer User Account Control.
        if self.session.is_none() || self.shell.locked || self.shell.uac.is_some() {
            return Err(Code::Denied);
        }
        // The keyboard's own window may have the focus (it opened last):
        // the key goes to the window the user was using.
        if self
            .engine
            .focused_window()
            .is_none_or(|f| self.keyboard_window(f))
        {
            let target = self
                .engine
                .windows()
                .filter(|w| w.visible && w.state != g::WindowState::Minimized && w.owner != owner)
                .map(|w| w.id)
                .last();
            if let Some(id) = target {
                let _ = self.engine.activate(id);
            }
        }
        let mut mods = 0;
        if flags & 1 != 0 {
            mods |= d::SHIFT;
        }
        if flags & 2 != 0 {
            mods |= d::CONTROL;
        }
        if flags & 4 != 0 {
            mods |= d::ALT;
        }
        let now = door_sys::time().unwrap_or(0);
        // Win chords (and the Win key alone) take the system-key path.
        if flags & 8 != 0 || scan == 0x15b {
            let keys: &[(u32, bool)] = if scan == 0x15b {
                &[(0x15b, true), (0x15b, false)]
            } else {
                &[(0x15b, true), (scan, true), (scan, false), (0x15b, false)]
            };
            let mut commands = Vec::new();
            for &(code, down) in keys {
                if let door_window::policy::SystemEvent::Command(c) =
                    self.system_keys.event(code, down, mods & d::CONTROL != 0)
                {
                    commands.push(c);
                }
            }
            for c in commands {
                self.system_command(c);
            }
            return Ok(Response::empty());
        }
        let press = flags & 0x200 == 0;
        let release = flags & 0x100 == 0;
        for (down, wanted) in [(true, press), (false, release)] {
            if !wanted {
                continue;
            }
            let event = door_abi::graphics::Input {
                kind: d::KEY,
                flags: mods | if down { d::DOWN } else { 0 },
                x: 0,
                y: 0,
                code: scan,
                text: 0,
                timestamp: now,
            };
            if let Some(code) = self.text_input_key(&event) {
                self.route(g::Input::Key { code, down }, now);
            }
        }
        Ok(Response::empty())
    }
}
