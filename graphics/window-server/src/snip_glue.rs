//! The snip overlay's service side (`door_window::snip` is the model): it
//! starts from Win+Shift+S, Print Screen or Snipping Tool's New
//! (`window::SNIP_BEGIN`), owns the pointer and keyboard while open, puts the
//! result on the clipboard and opens Snipping Tool with it (SNIP_READY).
use super::*;
use door_window::snip::{Hit, Mode, Snip};

impl Desktop {
    /// Open the overlay; `requester` (Snipping Tool) has its windows hidden
    /// until the snip is done.
    pub(super) fn start_snip(&mut self, mode: Mode, delay_ms: u64, requester: Option<g::Owner>) {
        if self.shell.snip.is_some() || self.session.is_none() || self.shell.locked {
            return;
        }
        let now = door_sys::time().unwrap_or(0);
        self.close_start();
        self.close_menu();
        self.tray.open = None;
        self.overview = None;
        let mut wait = delay_ms;
        if let Some(owner) = requester {
            let windows: Vec<g::WindowId> = self
                .engine
                .windows()
                .filter(|w| w.owner == owner)
                .map(|w| w.id)
                .collect();
            for id in windows {
                let _ = self
                    .engine
                    .request_state(owner, id, g::WindowState::Minimized);
            }
            // Let the minimize animation leave the screen first.
            wait = wait.max(400);
        }
        let freeze_at = now + wait / door_abi::platform::MONOTONIC_TICK_MILLIS;
        self.shell.snip = Some(Snip::new(mode, freeze_at, requester.map(|o| o.pid)));
        door_rt::diagnostic(format_args!(
            "DOOR_SNIP OPEN mode={mode:?} delay_ms={delay_ms}\n"
        ));
        self.engine.invalidate();
    }
    /// Main loop: the frame that freezes the screen once the delay is over.
    pub(super) fn snip_tick(&mut self, now: u64) {
        if let Some(s) = &self.shell.snip
            && !s.frozen()
            && now >= s.freeze_at
        {
            self.engine.invalidate();
        }
    }
    /// The windows the Window mode can take, front to back (physical).
    pub(super) fn snip_windows(&self) -> Vec<g::Rect> {
        let mut v: Vec<g::Rect> = self
            .engine
            .windows()
            .filter(|w| w.visible && w.state != g::WindowState::Minimized)
            .map(|w| w.outer)
            .collect();
        v.reverse();
        v
    }
    fn restore_requester(&mut self, pid: Option<u64>) {
        let Some(pid) = pid else {
            return;
        };
        let windows: Vec<(g::Owner, g::WindowId)> = self
            .engine
            .windows()
            .filter(|w| w.owner.pid == pid)
            .map(|w| (w.owner, w.id))
            .collect();
        for (owner, id) in windows {
            let _ = self.engine.restore_window(owner, id);
            let _ = self.engine.activate(id);
        }
    }
    pub(super) fn cancel_snip(&mut self) {
        if let Some(s) = self.shell.snip.take() {
            door_rt::diagnostic(format_args!("DOOR_SNIP CANCELLED\n"));
            self.restore_requester(s.requester);
            self.engine.invalidate();
        }
    }
    fn finish_snip(&mut self) {
        let Some(s) = self.shell.snip.take() else {
            return;
        };
        let Some(image) = s.take() else {
            self.restore_requester(s.requester);
            return;
        };
        let content = door_window::clipboard::Content {
            formats: alloc::vec![door_window::clipboard::Data {
                format: door_service_protocol::clipboard::IMAGE,
                bytes: alloc::rc::Rc::new(image.clipboard()),
            }],
        };
        let stored = self.shell.clipboard.set_system(content).is_ok();
        door_rt::diagnostic(format_args!(
            "DOOR_SNIP TAKEN mode={:?} size={}x{} clipboard={}\n",
            s.mode,
            image.width,
            image.height,
            u8::from(stored)
        ));
        self.engine.invalidate();
        if !stored {
            self.restore_requester(s.requester);
            return;
        }
        self.shell.snip_ready = Some((image.width, image.height));
        // Snipping Tool opens with the snip: its running window comes back
        // to the front, or it starts.
        let tool = door_window::apps::Tool::SnippingTool;
        let running = self
            .clients
            .iter()
            .find(|c| c.manifest == tool.manifest())
            .map(|c| c.owner);
        match running {
            Some(owner) => {
                let windows: Vec<g::WindowId> = self
                    .engine
                    .windows()
                    .filter(|w| w.owner == owner)
                    .map(|w| w.id)
                    .collect();
                for id in windows {
                    let _ = self.engine.restore_window(owner, id);
                    let _ = self.engine.activate(id);
                }
            }
            None => self.launch_tool(tool),
        }
    }
    /// A pointer event while the overlay is open (physical position in
    /// `self.x`/`self.y`; `flags` the new button state).
    pub(super) fn snip_pointer(&mut self, flags: u32) {
        let scale = self.engine.scale();
        let width = scale.logical_len(self.engine.output_rect().width());
        let (lx, ly) = (scale.logical(self.x), scale.logical(self.y));
        let (x, y) = (self.x, self.y);
        let pressed = flags & 1 != 0 && self.buttons & 1 == 0;
        let released = flags & 1 == 0 && self.buttons & 1 != 0;
        let Some(s) = self.shell.snip.as_mut() else {
            return;
        };
        if !s.frozen() {
            return;
        }
        s.motion(x, y);
        if pressed {
            match s.hit(width, lx, ly) {
                Hit::Close => {
                    self.cancel_snip();
                    return;
                }
                Hit::Mode(Mode::FullScreen) => {
                    s.mode = Mode::FullScreen;
                    self.finish_snip();
                    return;
                }
                Hit::Mode(m) => {
                    s.mode = m;
                    s.start = None;
                    s.lasso.clear();
                }
                Hit::Screen => match s.mode {
                    Mode::Window | Mode::FullScreen => {
                        self.finish_snip();
                        return;
                    }
                    _ => s.press(x, y),
                },
            }
        } else if released && s.start.is_some() {
            if s.selection().is_some() {
                self.finish_snip();
                return;
            }
            s.start = None;
            s.lasso.clear();
        }
        self.engine.invalidate();
    }
    /// Keys while the overlay is open: Esc cancels, Enter takes the full
    /// screen in that mode; everything else is swallowed.
    pub(super) fn snip_key(&mut self, scan: u32, down: bool) {
        if !down {
            return;
        }
        match scan {
            1 => self.cancel_snip(),
            28 | 0x11c
                if self
                    .shell
                    .snip
                    .as_ref()
                    .is_some_and(|s| s.mode == Mode::FullScreen && s.frozen()) =>
            {
                self.finish_snip()
            }
            _ => {}
        }
    }
    /// SNIP_READY for Snipping Tool once its window has focus (the clipboard
    /// READ needs it).
    pub(super) fn take_snip_event(&mut self, owner: g::Owner) -> Option<w::Event> {
        let (width, height) = self.shell.snip_ready?;
        let manifest = self.clients.iter().find(|c| c.owner == owner)?.manifest;
        if manifest != door_abi::platform::manifest::SNIPPING_TOOL {
            return None;
        }
        let focused = self.engine.focused_window()?;
        if self.engine.window_info(focused).ok()?.owner != owner {
            return None;
        }
        self.shell.snip_ready = None;
        Some(w::Event {
            kind: w::SNIP_READY,
            window: focused.0,
            a: width,
            b: height,
            ..w::Event::default()
        })
    }
    /// SNIP_BEGIN from Snipping Tool.
    pub(super) fn snip_request(&mut self, owner: g::Owner, b: &[u8]) -> Result<Response, Code> {
        if b.len() != 8 {
            return Err(Code::InvalidRequest);
        }
        let manifest = self
            .clients
            .iter()
            .find(|c| c.owner == owner)
            .map(|c| c.manifest);
        if manifest != Some(door_abi::platform::manifest::SNIPPING_TOOL) {
            return Err(Code::Denied);
        }
        let mode = Mode::from_word(u32::from_le_bytes(b[..4].try_into().unwrap()))
            .ok_or(Code::InvalidRequest)?;
        let delay = u32::from_le_bytes(b[4..8].try_into().unwrap());
        if !matches!(delay, 0 | 3 | 5 | 10) {
            return Err(Code::InvalidRequest);
        }
        self.start_snip(mode, u64::from(delay) * 1000, Some(owner));
        Ok(Response::empty())
    }
}
