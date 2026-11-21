//! Window-server glue for door.text-input (`text-input-v1.md`): the
//! authenticated environment, the key hooks used by `input.rs` (hotkeys
//! before the shell's Win chords, layout translation and routing at
//! application dispatch, device reset), the shell's own text fields as a
//! server-side context, the taskbar input switcher and the candidate window
//! overlay. Policy lives in the host-tested `door_window::text_input`.
use super::*;
use door_service_protocol::text_input as ti;
use door_window::text_input::{KeyRoute, Peer, Reply, candidate};
/// The shell's text fields (Start search, Run) form one text-input context
/// owned by the server itself: no process has pid 0 and no engine window
/// has this id, so no client can name or reach it.
const SHELL: g::Owner = g::Owner {
    pid: 0,
    session: 0,
    epoch: u64::MAX,
};
const SHELL_WINDOW: u64 = u64::MAX;
/// Requests see the server's trusted state, never wire claims.
struct Env<'a> {
    engine: &'a g::Engine,
    locked: bool,
    now: u64,
    /// A shell text field has keyboard focus (its context is focused).
    shell: bool,
}
impl door_window::text_input::Env for Env<'_> {
    fn focused(&self) -> Option<(g::Owner, u64)> {
        if self.shell {
            return Some((SHELL, SHELL_WINDOW));
        }
        let id = self.engine.focused_window()?;
        self.engine.window_info(id).ok().map(|w| (w.owner, id.0))
    }
    fn owns(&self, owner: g::Owner, window: u64) -> bool {
        if owner == SHELL || window == SHELL_WINDOW {
            return owner == SHELL && window == SHELL_WINDOW;
        }
        self.engine
            .window_info(g::WindowId(window))
            .is_ok_and(|info| info.owner == owner && info.state != g::WindowState::Closing)
    }
    fn content_origin(&self, window: u64) -> Option<(i32, i32)> {
        if window == SHELL_WINDOW {
            // The shell reports its caret in output pixels.
            return Some((0, 0));
        }
        self.engine
            .window_info(g::WindowId(window))
            .ok()
            .map(|w| (w.content.x(), w.content.y()))
    }
    fn scale_percent(&self) -> u32 {
        self.engine.scale().percent()
    }
    fn locked(&self) -> bool {
        self.locked
    }
    fn now(&self) -> u64 {
        self.now
    }
}
fn env(engine: &g::Engine, locked: bool, shell: bool) -> Env<'_> {
    Env {
        engine,
        locked,
        now: door_sys::time().unwrap_or(0),
        shell,
    }
}
fn reply(r: Reply) -> Response {
    match r {
        Reply::Empty => Response::empty(),
        Reply::Scalar(v) => Response::scalar(v),
        Reply::Bytes(b) => Response::bytes(&b),
    }
}
impl Desktop {
    fn text_input_changed(&mut self) {
        self.shell_text_drain();
        if let Some(m) = self.shell.text_input.take_method_change() {
            let _ = door_sys::write(
                format!(
                    "DOOR_P12 TEXT_INPUT METHOD id={} ime={}\n",
                    m.id,
                    u8::from(m.ime != door_keyboard_layouts::Ime::None)
                )
                .as_bytes(),
            );
        }
        if self.shell.text_input.take_dirty() {
            // Taskbar input indicator; the candidate overlay repaints with it.
            self.bar_dirty = true;
            self.engine.invalidate();
        }
    }
    /// Win+Space / Left Alt+Shift / Ctrl+Shift. True: the event is consumed
    /// (the shell's Win-chord state still sees it, so Start stays closed).
    pub(super) fn text_input_hotkey(&mut self, event: &door_abi::graphics::Input) -> bool {
        self.shell_text_sync();
        let e = env(&self.engine, self.shell.locked, self.shell_text_focus());
        let consumed = self.shell.text_input.hotkey(&e, event);
        if consumed {
            let down = event.flags & door_abi::graphics::DOWN != 0;
            let _ = self.system_keys.event(
                event.code,
                down,
                event.flags & door_abi::graphics::CONTROL != 0,
            );
        }
        self.text_input_changed();
        consumed
    }
    /// Layout translation and routing for a key that reached applications:
    /// a legacy window key code, or None when text input delivered it.
    pub(super) fn text_input_key(&mut self, event: &door_abi::graphics::Input) -> Option<u32> {
        let e = env(&self.engine, self.shell.locked, false);
        let route = self.shell.text_input.key(&e, event);
        self.text_input_changed();
        match route {
            KeyRoute::Legacy(code) => Some(code),
            KeyRoute::Consumed => None,
        }
    }
    /// HELLO: CAP_IME only for the IME manifest; both start PENDING events.
    pub(super) fn text_input_hello(
        &mut self,
        owner: g::Owner,
        peer: Identity,
        features: u32,
    ) -> u32 {
        let features = if peer.manifest == ti::IME_MANIFEST {
            features
        } else {
            features & !ti::CAP_IME
        };
        if features & (ti::CAP_TEXT_INPUT | ti::CAP_IME) != 0 {
            self.shell.text_input.listen(owner);
        }
        features
    }
    /// door.text-input request (opcodes `ti::FIRST..=ti::LAST`).
    pub(super) fn text_input_request(
        &mut self,
        peer: Identity,
        owner: g::Owner,
        op: u32,
        b: &[u8],
    ) -> Result<Response, Code> {
        let role = if peer.manifest == ti::IME_MANIFEST {
            Peer::Ime
        } else if peer.manifest == door_abi::platform::manifest::WORKSPACE {
            Peer::Settings
        } else {
            Peer::App
        };
        self.shell_text_sync();
        let e = env(&self.engine, self.shell.locked, self.shell_text_focus());
        let r = self.shell.text_input.request(&e, role, owner, op, b);
        self.text_input_changed();
        r.map(reply)
    }
    /// Init (session 0): SET_PREFERENCES with the user's input methods.
    pub(super) fn text_input_init(
        &mut self,
        peer: Identity,
        op: u32,
        b: &[u8],
        t: &[door_abi::Transfer],
    ) -> Result<Response, Code> {
        if op != ti::SET_PREFERENCES || !t.is_empty() {
            return Err(Code::Denied);
        }
        let owner = g::Owner {
            pid: peer.pid,
            session: 0,
            epoch: peer.instance,
        };
        let e = env(&self.engine, self.shell.locked, self.shell_text_focus());
        let r = self.shell.text_input.request(&e, Peer::Init, owner, op, b);
        self.text_input_changed();
        r.map(reply)
    }
    /// The focused app's text caret in logical output pixels (Win+V opens
    /// there, as in Windows); None without an enabled text context.
    pub(super) fn text_input_caret(&self) -> Option<g::Rect> {
        let e = env(&self.engine, self.shell.locked, false);
        let c = self.shell.text_input.caret(&e)?;
        let s = self.engine.scale();
        g::Rect::new(
            s.logical(c.x),
            s.logical(c.y),
            s.logical_len(c.width).max(1),
            s.logical_len(c.height).max(1),
        )
        .ok()
    }
    /// Taskbar input switcher: make installed method `index` current (the
    /// shell is trusted like Settings for SELECT).
    pub(super) fn select_input_method(&mut self, index: usize) {
        let e = env(&self.engine, self.shell.locked, self.shell_text_focus());
        let _ = self.shell.text_input.request(
            &e,
            Peer::Settings,
            SHELL,
            ti::SELECT,
            &(index as u32).to_le_bytes(),
        );
        self.text_input_changed();
    }
    /// A shell text field has keyboard focus: the Run input, or Start (typing
    /// always edits its search box, as in Windows).
    pub(super) fn shell_text_focus(&self) -> bool {
        self.session.is_some()
            && !self.shell.locked
            && self.shell.menu.is_none()
            && self.overview.is_none()
            && (self.shell.run.open || self.launcher)
    }
    /// The focused field's caret at the composition start, in output
    /// physical pixels (the IME's candidate window anchors under it).
    fn shell_caret(&self) -> Option<w::Rect> {
        let layout = self.taskbar_layout();
        let (field, area, top, height) = if self.shell.run.open {
            let input = self.shell.run.input(layout);
            let r = &self.shell.run;
            let area = g::Rect::new(
                input.x() + 12,
                input.y(),
                input.width().saturating_sub(52).max(1),
                input.height(),
            )
            .ok()?;
            (
                door_window::text_edit::Field {
                    text: &r.text,
                    caret: r.caret,
                    preedit: &r.preedit,
                },
                area,
                input.y() + 9,
                18,
            )
        } else if self.launcher {
            let search = self.shell.start.geometry(layout).search_box();
            let s = &self.shell.start;
            let area = g::Rect::new(
                search.x() + 44,
                search.y(),
                search.width().saturating_sub(60).max(1),
                32,
            )
            .ok()?;
            (
                door_window::text_edit::Field {
                    text: &s.query,
                    caret: s.query_caret,
                    preedit: &s.preedit,
                },
                area,
                search.y() + 8,
                16,
            )
        } else {
            return None;
        };
        let x = door_window::text_edit::caret_x(
            &self.fonts,
            area,
            &field,
            door_graphics_text::TextStyle::Regular14,
        );
        let r = self
            .engine
            .scale()
            .rect(g::Rect::new(x, top, 1, height).ok()?)
            .ok()?;
        Some(w::Rect {
            x: r.x(),
            y: r.y(),
            width: r.width(),
            height: r.height(),
        })
    }
    /// Keep the shell context enabled and its caret current before the
    /// broker sees a key or request.
    pub(super) fn shell_text_sync(&mut self) {
        if self.session.is_none() {
            return;
        }
        let focus = self.shell_text_focus();
        let caret = if focus { self.shell_caret() } else { None };
        if !focus {
            // Blur commits or cancels at the IME; no composition stays shown.
            self.shell.run.preedit = Default::default();
            self.shell.start.preedit = Default::default();
        }
        let e = env(&self.engine, self.shell.locked, focus);
        let broker = &mut self.shell.text_input;
        match self.shell.text_serial {
            None => {
                let body = ti::Enable {
                    window: SHELL_WINDOW,
                    content: ti::NORMAL,
                    hints: 0,
                    caret: caret.unwrap_or_default(),
                }
                .encode();
                if let Ok(Reply::Scalar(serial)) =
                    broker.request(&e, Peer::App, SHELL, ti::ENABLE, &body)
                {
                    self.shell.text_serial = Some(serial as u32);
                    self.shell.text_caret = caret;
                }
            }
            Some(serial) => {
                if let Some(c) = caret.filter(|c| Some(*c) != self.shell.text_caret) {
                    let body = ti::encode_cursor(SHELL_WINDOW, serial, c);
                    if broker
                        .request(&e, Peer::App, SHELL, ti::CURSOR, &body)
                        .is_ok()
                    {
                        self.shell.text_caret = Some(c);
                    }
                }
            }
        }
    }
    /// A key while a shell field has focus: the session layout translates it
    /// and an active IME sees it first; the shell then gets key records
    /// (Unicode key text) and commits from `shell_text_drain`.
    pub(super) fn shell_text_key(&mut self, event: &door_abi::graphics::Input) {
        self.shell_text_sync();
        let e = env(&self.engine, self.shell.locked, true);
        let route = self.shell.text_input.key(&e, event);
        if let KeyRoute::Legacy(code) = route
            && event.flags & door_abi::graphics::DOWN != 0
        {
            // No shell context (queue overflow): layout ASCII, as before.
            let ascii = (code >> 16) as u8;
            let mut b = [0u8; 4];
            let text = if (0x20..0x7f).contains(&ascii) {
                char::from(ascii).encode_utf8(&mut b)
            } else {
                ""
            };
            if !self.desktop_search(event.code, text) {
                self.desktop_key(event.code);
            }
        }
        self.text_input_changed();
    }
    /// Apply the shell context's queued records to the focused field.
    fn shell_text_drain(&mut self) {
        if self.shell.text_serial.is_none() {
            return;
        }
        for _ in 0..8 {
            let e = env(&self.engine, self.shell.locked, self.shell_text_focus());
            let Ok(Reply::Bytes(bytes)) =
                self.shell
                    .text_input
                    .request(&e, Peer::App, SHELL, ti::POLL, &[])
            else {
                return;
            };
            for record in ti::records(&bytes) {
                let Ok(record) = record else { break };
                self.shell_text_record(record);
            }
        }
    }
    fn shell_text_record(&mut self, record: ti::Record<'_>) {
        match record {
            ti::Record::Key { key, .. } if key.down() => {
                if !self.desktop_search(key.code, key.text()) {
                    self.desktop_key(key.code);
                }
            }
            ti::Record::Commit { text, .. } => {
                // The commit replaces the composition (also on blur, when a
                // Start menu took focus).
                if self.shell.run.open {
                    self.shell.run.preedit = Default::default();
                    self.shell.run.type_text(text);
                } else if self.launcher {
                    let mode = self.shell.start.mode;
                    self.shell.start.preedit = Default::default();
                    self.shell.start.type_text(text);
                    self.panel_mode_changed(mode);
                }
                self.bar_dirty = true;
            }
            ti::Record::Preedit { preedit, .. } => {
                let p = door_window::text_edit::Preedit {
                    text: alloc::string::String::from(preedit.text),
                    cursor: (preedit.cursor != ti::HIDDEN_CURSOR)
                        .then_some(preedit.cursor as usize),
                };
                if self.shell.run.open {
                    self.shell.run.preedit = p;
                } else if self.launcher {
                    self.shell.start.preedit = p;
                }
                self.bar_dirty = true;
            }
            ti::Record::Delete { before, after, .. } => {
                let (before, after) = (before as usize, after as usize);
                if self.shell.run.open {
                    self.shell.run.ime_delete(before, after);
                } else if self.launcher {
                    self.shell.start.ime_delete(before, after);
                }
                self.bar_dirty = true;
            }
            ti::Record::Disabled { .. } => {
                self.shell.text_serial = None;
                self.shell.text_caret = None;
            }
            _ => (),
        }
    }
    /// The IME candidate window over the output (Windows 11 style: under the
    /// composition caret, flipped above it at the work area's end), drawn
    /// with the shell's Unicode TextSystem in the current palette (light,
    /// dark or contrast). Returns the painted frame.
    pub(super) fn paint_candidates(
        broker: &door_window::text_input::Broker,
        text: &door_graphics_text::TextSystem,
        work: g::Rect,
        dark: bool,
        target: &mut candidate::Target<'_>,
    ) -> Option<w::Rect> {
        let window = broker.candidates()?;
        let work = w::Rect {
            x: work.x(),
            y: work.y(),
            width: work.width(),
            height: work.height(),
        };
        let palette = candidate::Palette::current(dark);
        let frame =
            candidate::render(window, work, &palette, text, broker.method().tag, target).frame;
        if broker.announce_candidates() {
            let _ = door_sys::write(
                format!(
                    "DOOR_P12 CANDIDATES SHOWN x={} y={} width={} height={} items={} vertical={} caret={},{},{}\n",
                    frame.x,
                    frame.y,
                    frame.width,
                    frame.height,
                    window.items.len(),
                    u8::from(window.vertical),
                    window.anchor.x,
                    window.anchor.y,
                    window.anchor.height
                )
                .as_bytes(),
            );
        }
        Some(frame)
    }
}
