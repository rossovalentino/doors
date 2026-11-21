//! Win32 close and "not responding" (Windows 11; native-window-v1 "Win32
//! close and not responding").
//!
//! * Close on a CAP_WIN32 window queues one CLOSE event for its client. The
//!   window keeps its state and input: no close transition and no deadline.
//!   user32 acknowledges with CANCEL_CLOSE the moment it retrieves the event
//!   and then runs WM_CLOSE for as long as the application needs (a save
//!   prompt may stay open indefinitely).
//! * A client is hung when events have waited for it for `HUNG_TICKS` (five
//!   seconds) without a POLL, or when its user32 reported the thread owning a
//!   window hung (SET_RESPONDING) while other threads still poll. Hung windows
//!   are ghosts: the frozen last frame, whitened, with " (Not Responding)"
//!   after the title.
//! * Close on a hung window opens the prompt. End task terminates the client
//!   (the existing Init termination); Wait only dismisses it. The prompt also
//!   goes away by itself when the window responds again.
//!
//! Shutdown and sign-out keep the native close path and its bounded
//! containment; nothing here ever terminates a client by itself.
use crate::paint::{self, Palette};
use alloc::string::String;
use alloc::vec::Vec;
use door_graphics_core::{Canvas, Owner, Rect};
use door_graphics_text::{FontAtlas, TextStyle};
use door_service_protocol::Code;

/// Windows' hung-application threshold (IsHungAppWindow, DWM ghosting).
pub const HUNG_TICKS: u64 = 5000 / door_abi::platform::MONOTONIC_TICK_MILLIS;
/// Appended to a ghost window's caption.
pub const SUFFIX: &str = " (Not Responding)";

#[derive(Clone, Debug)]
struct Client {
    owner: Owner,
    /// Tick since which events have waited without a POLL.
    waiting_since: Option<u64>,
    /// The whole client stopped polling (process-level hang).
    hung: bool,
    /// Windows whose owning thread the client reported hung.
    reported: Vec<u64>,
    /// CLOSE events not yet delivered (at most one per window).
    closes: Vec<u64>,
}

/// A client-level transition `tick` observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Hung,
    Responding,
}

/// What a close request on a Win32 window did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Close {
    /// One CLOSE event waits for the client (repeated requests add none).
    Queued,
    /// The window is not responding: the prompt is open for it.
    Prompt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    EndTask,
    Wait,
}

/// The open "is not responding" prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub owner: Owner,
    pub window: u64,
    pub title: String,
    /// The focused button (Enter and Space activate it; Wait at first).
    pub focus: Answer,
    /// The button the primary press went down on.
    pub pressed: Option<Answer>,
}

#[derive(Clone, Debug, Default)]
pub struct Responsiveness {
    clients: Vec<Client>,
    prompt: Option<Prompt>,
}

/// A ghost window's caption text, kept within the 128-byte title bound.
pub fn ghost_title(title: &str) -> String {
    let room = 128 - SUFFIX.len();
    let mut end = title.len().min(room);
    while !title.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = String::from(&title[..end]);
    out.push_str(SUFFIX);
    out
}

impl Responsiveness {
    pub const fn new() -> Self {
        Self {
            clients: Vec::new(),
            prompt: None,
        }
    }
    fn client(&self, owner: Owner) -> Option<&Client> {
        self.clients.iter().find(|c| c.owner == owner)
    }
    fn client_mut(&mut self, owner: Owner) -> Option<&mut Client> {
        self.clients.iter_mut().find(|c| c.owner == owner)
    }
    /// HELLO granted CAP_WIN32.
    pub fn register(&mut self, owner: Owner) -> Result<(), Code> {
        if self.client(owner).is_some() {
            return Ok(());
        }
        self.clients.push(Client {
            owner,
            waiting_since: None,
            hung: false,
            reported: Vec::new(),
            closes: Vec::new(),
        });
        Ok(())
    }
    pub fn is_client(&self, owner: Owner) -> bool {
        self.client(owner).is_some()
    }
    pub fn owners(&self) -> impl Iterator<Item = Owner> + '_ {
        self.clients.iter().map(|c| c.owner)
    }
    /// The client is gone (exit, revocation, End task).
    pub fn forget(&mut self, owner: Owner) {
        self.clients.retain(|c| c.owner != owner);
        if self.prompt.as_ref().is_some_and(|p| p.owner == owner) {
            self.prompt = None;
        }
    }
    /// One of its windows was destroyed.
    pub fn forget_window(&mut self, owner: Owner, window: u64) {
        if let Some(c) = self.client_mut(owner) {
            c.closes.retain(|w| *w != window);
            c.reported.retain(|w| *w != window);
        }
        if self.prompt.as_ref().is_some_and(|p| p.window == window) {
            self.prompt = None;
        }
    }
    /// The client retrieved its events (POLL or POLL_BATCH).
    pub fn polled(&mut self, owner: Owner) {
        if let Some(c) = self.client_mut(owner) {
            c.waiting_since = None;
        }
    }
    /// Once per server iteration: `pending` = events wait in the client's
    /// queue. Returns the client-level transition, if any.
    pub fn tick(&mut self, owner: Owner, pending: bool, now: u64) -> Option<Change> {
        let c = self.client_mut(owner)?;
        let pending = pending || !c.closes.is_empty();
        c.waiting_since = if pending {
            Some(c.waiting_since.unwrap_or(now))
        } else {
            None
        };
        let hung = c
            .waiting_since
            .is_some_and(|since| now.saturating_sub(since) >= HUNG_TICKS);
        if hung == c.hung {
            return None;
        }
        c.hung = hung;
        if !hung {
            self.dismiss_if_responding();
        }
        Some(if hung {
            Change::Hung
        } else {
            Change::Responding
        })
    }
    /// SET_RESPONDING. Returns whether the window's state changed.
    pub fn report(&mut self, owner: Owner, window: u64, responding: bool) -> Result<bool, Code> {
        let c = self.client_mut(owner).ok_or(Code::Unsupported)?;
        let known = c.reported.contains(&window);
        if responding != known {
            return Ok(false);
        }
        if responding {
            c.reported.retain(|w| *w != window);
            self.dismiss_if_responding();
        } else {
            c.reported.push(window);
        }
        Ok(true)
    }
    pub fn client_hung(&self, owner: Owner) -> bool {
        self.client(owner).is_some_and(|c| c.hung)
    }
    /// A ghost: its client or the thread owning it stopped responding.
    pub fn window_hung(&self, owner: Owner, window: u64) -> bool {
        self.client(owner)
            .is_some_and(|c| c.hung || c.reported.contains(&window))
    }
    /// The user asked to close a Win32 window (caption Close, Alt+F4, Task
    /// View, taskbar). Never starts a deadline.
    pub fn request_close(&mut self, owner: Owner, window: u64, title: &str) -> Close {
        if self.window_hung(owner, window) {
            if self.prompt.is_none() {
                self.prompt = Some(Prompt {
                    owner,
                    window,
                    title: String::from(title),
                    focus: Answer::Wait,
                    pressed: None,
                });
            }
            return Close::Prompt;
        }
        if let Some(c) = self.client_mut(owner)
            && !c.closes.contains(&window)
        {
            c.closes.push(window);
        }
        Close::Queued
    }
    /// The next CLOSE event for the client's POLL.
    pub fn take_close(&mut self, owner: Owner) -> Option<u64> {
        let c = self.client_mut(owner)?;
        (!c.closes.is_empty()).then(|| c.closes.remove(0))
    }
    pub fn has_close(&self, owner: Owner) -> bool {
        self.client(owner).is_some_and(|c| !c.closes.is_empty())
    }
    /// CANCEL_CLOSE from a Win32 client: its WM_CLOSE is running.
    pub fn acknowledge(&mut self, owner: Owner, window: u64) {
        if let Some(c) = self.client_mut(owner) {
            c.closes.retain(|w| *w != window);
        }
    }
    pub fn prompt(&self) -> Option<&Prompt> {
        self.prompt.as_ref()
    }
    fn dismiss_if_responding(&mut self) {
        if let Some(p) = &self.prompt
            && !self.window_hung(p.owner, p.window)
        {
            self.prompt = None;
        }
    }
    fn answer(&mut self, answer: Answer) -> Option<(Owner, u64, Answer)> {
        let p = self.prompt.take()?;
        Some((p.owner, p.window, answer))
    }
    /// A key while the prompt is open (`scan` set-1, extended 0x1xx). Esc
    /// is Wait, Enter/Space the focused button, Tab and the arrows move the
    /// focus. Returns the answer when the prompt closed.
    pub fn key(&mut self, scan: u32) -> Option<(Owner, u64, Answer)> {
        let focus = self.prompt.as_ref()?.focus;
        match scan {
            1 => self.answer(Answer::Wait),
            28 | 0x11c | 57 => self.answer(focus),
            15 | 0x14b | 0x14d => {
                if let Some(p) = self.prompt.as_mut() {
                    p.focus = match focus {
                        Answer::Wait => Answer::EndTask,
                        Answer::EndTask => Answer::Wait,
                    };
                }
                None
            }
            _ => None,
        }
    }
    /// A primary button edge at logical (x, y) with the prompt laid out in
    /// `area`. Returns (consumed, answer): presses inside the prompt are
    /// consumed; a release on the button it went down on answers.
    pub fn button(
        &mut self,
        area: Rect,
        x: i32,
        y: i32,
        down: bool,
    ) -> (bool, Option<(Owner, u64, Answer)>) {
        let Some(p) = self.prompt.as_mut() else {
            return (false, None);
        };
        let hit = hit(area, x, y);
        if down {
            p.pressed = match hit {
                Some(Target::Button(a)) => Some(a),
                _ => None,
            };
            return (hit.is_some(), None);
        }
        let pressed = p.pressed.take();
        match (hit, pressed) {
            (Some(Target::Button(a)), Some(b)) if a == b => (true, self.answer(a)),
            (h, _) => (h.is_some(), None),
        }
    }
}

/// Logical layout of the prompt, centred in the work area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub panel: Rect,
    pub end_task: Rect,
    pub wait: Rect,
}
const WIDTH: u32 = 456;
const HEIGHT: u32 = 196;
const BUTTON: (u32, u32) = (120, 32);
const FOOTER: u32 = 80;
pub fn geometry(area: Rect) -> Geometry {
    let w = WIDTH.min(area.width().max(1));
    let h = HEIGHT.min(area.height().max(1));
    let x = area.x() + (area.width().saturating_sub(w) / 2) as i32;
    let y = area.y() + (area.height().saturating_sub(h) / 2) as i32;
    let panel = Rect::new(x, y, w, h).unwrap_or(area);
    let by = panel.bottom() - (FOOTER as i32 + BUTTON.1 as i32) / 2;
    let wait_x = panel.right() - 24 - BUTTON.0 as i32;
    let end_x = wait_x - 8 - BUTTON.0 as i32;
    let button = |bx: i32| Rect::new(bx, by, BUTTON.0, BUTTON.1).unwrap_or(panel);
    Geometry {
        panel,
        end_task: button(end_x),
        wait: button(wait_x),
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Button(Answer),
    Panel,
}
pub fn hit(area: Rect, x: i32, y: i32) -> Option<Target> {
    let g = geometry(area);
    if g.end_task.contains(x, y) {
        Some(Target::Button(Answer::EndTask))
    } else if g.wait.contains(x, y) {
        Some(Target::Button(Answer::Wait))
    } else if g.panel.contains(x, y) {
        Some(Target::Panel)
    } else {
        None
    }
}

/// The prompt (WinUI content dialog over the desktop), logical coordinates.
pub fn draw(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    prompt: &Prompt,
    area: Rect,
    dark: bool,
    pointer: (i32, i32),
) {
    let palette = Palette::new(dark);
    let g = geometry(area);
    let p = g.panel;
    paint::shadow(c, p, 8, dark);
    let base = if palette.contrast {
        palette.surface
    } else if dark {
        paint::rgb(43, 43, 43)
    } else {
        paint::rgb(255, 255, 255)
    };
    paint::framed(c, p, 8, palette.surface_border, base);
    if let Ok(footer) = Rect::new(
        p.x() + 1,
        p.bottom() - FOOTER as i32,
        p.width().saturating_sub(2).max(1),
        FOOTER - 1,
    ) {
        paint::fill(c, footer, palette.surface_header);
    }
    let width = p.width().saturating_sub(48).max(1);
    let header = alloc::format!("{} is not responding", prompt.title);
    let header = paint::ellipsize(fonts, &header, TextStyle::Semibold20, width);
    paint::lcd::text_row(
        c,
        fonts,
        p.x() + 24,
        p.y() + 20,
        32,
        &header,
        TextStyle::Semibold20,
        palette.text,
        width,
    );
    paint::lcd::text_row(
        c,
        fonts,
        p.x() + 24,
        p.y() + 60,
        24,
        "If you end the task, you might lose information.",
        TextStyle::Regular14,
        palette.text,
        width,
    );
    let hover = hit(area, pointer.0, pointer.1);
    for (answer, r, label) in [
        (Answer::EndTask, g.end_task, "End task"),
        (Answer::Wait, g.wait, "Wait"),
    ] {
        let focused = prompt.focus == answer;
        let hot = hover == Some(Target::Button(answer));
        if focused {
            paint::rounded(c, r, 4, palette.accent);
            paint::focus_ring(c, r, &palette);
        } else {
            let fill = if hot { palette.tile } else { palette.field };
            paint::framed(c, r, 4, palette.field_border, fill);
        }
        let fg = if focused {
            palette.accent_text
        } else {
            palette.text
        };
        paint::lcd::text_in(c, fonts, r, label, TextStyle::Regular14, fg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const A: Owner = Owner {
        pid: 7,
        session: 1,
        epoch: 1,
    };
    const W: u64 = 41;
    fn client() -> Responsiveness {
        let mut r = Responsiveness::new();
        r.register(A).unwrap();
        r
    }
    #[test]
    fn slow_wm_close_is_acknowledged_and_never_timed_out() {
        let mut r = client();
        assert_eq!(r.request_close(A, W, "Editor"), Close::Queued);
        assert_eq!(r.request_close(A, W, "Editor"), Close::Queued);
        assert!(r.tick(A, false, 100).is_none(), "the close is pending");
        // user32 retrieves it and acknowledges before WM_CLOSE runs.
        r.polled(A);
        assert_eq!(r.take_close(A), Some(W));
        assert_eq!(r.take_close(A), None, "repeated requests queue one close");
        r.acknowledge(A, W);
        // WM_CLOSE then blocks for a second without polling, input waiting.
        for t in 101..=200 {
            assert_eq!(r.tick(A, true, t), None);
        }
        assert!(!r.window_hung(A, W) && r.prompt().is_none());
        assert!(r.is_client(A), "nothing evicts a slow close");
    }
    #[test]
    fn dirty_close_message_box_survives_indefinitely() {
        let mut r = client();
        r.request_close(A, W, "Editor");
        r.polled(A);
        assert_eq!(r.take_close(A), Some(W));
        r.acknowledge(A, W);
        // The save prompt's modal loop keeps polling for ten minutes while
        // the user decides; input arrives in between.
        for t in 0..60_000u64 {
            let pending = t % 7 == 0;
            assert_eq!(r.tick(A, pending, t), None);
            if t % 3 == 0 {
                r.polled(A);
            }
        }
        assert!(!r.client_hung(A));
        // Clicking Close again while it asks delivers another WM_CLOSE.
        assert_eq!(r.request_close(A, W, "Editor"), Close::Queued);
        assert!(r.prompt().is_none());
    }
    #[test]
    fn a_real_hang_becomes_a_ghost_after_five_seconds() {
        let mut r = client();
        assert_eq!(HUNG_TICKS, 500);
        // Idle without pending events is never hung, however long.
        assert_eq!(r.tick(A, false, 10_000), None);
        assert_eq!(r.tick(A, true, 1000), None);
        assert_eq!(r.tick(A, true, 1000 + HUNG_TICKS - 1), None);
        assert!(!r.window_hung(A, W));
        assert_eq!(r.tick(A, true, 1000 + HUNG_TICKS), Some(Change::Hung));
        assert!(r.window_hung(A, W) && r.client_hung(A));
        assert_eq!(
            ghost_title("Untitled - wingui-hang"),
            "Untitled - wingui-hang (Not Responding)"
        );
        let long = "x".repeat(200);
        assert_eq!(ghost_title(&long).len(), 128);
        // Retrieval ends the ghost.
        r.polled(A);
        assert_eq!(r.tick(A, false, 2000), Some(Change::Responding));
        assert!(!r.window_hung(A, W));
    }
    #[test]
    fn a_hung_thread_reported_by_user32_is_a_ghost_while_others_poll() {
        let mut r = client();
        assert_eq!(r.report(A, W, false), Ok(true));
        assert_eq!(r.report(A, W, false), Ok(false));
        assert!(r.window_hung(A, W) && !r.window_hung(A, W + 1));
        assert_eq!(r.request_close(A, W, "Worker"), Close::Prompt);
        assert_eq!(r.report(A, W, true), Ok(true));
        assert!(r.prompt().is_none(), "recovery dismisses the prompt");
        let other = Owner { pid: 8, ..A };
        assert_eq!(r.report(other, W, false), Err(Code::Unsupported));
    }
    #[test]
    fn close_on_a_ghost_offers_end_task_or_wait_and_only_end_task_terminates() {
        let mut r = client();
        r.tick(A, true, 0);
        r.tick(A, true, HUNG_TICKS);
        assert_eq!(r.request_close(A, W, "Hang"), Close::Prompt);
        assert_eq!(r.request_close(A, W, "Hang"), Close::Prompt);
        assert!(!r.has_close(A), "a ghost gets no CLOSE event");
        let p = r.prompt().unwrap();
        assert_eq!(
            (p.window, p.focus, p.title.as_str()),
            (W, Answer::Wait, "Hang")
        );
        // Wait (Esc) only dismisses it; the client stays.
        assert_eq!(r.key(1), Some((A, W, Answer::Wait)));
        assert!(r.prompt().is_none() && r.is_client(A));
        // Close again, Tab to End task, Enter.
        r.request_close(A, W, "Hang");
        assert_eq!(r.key(15), None);
        assert_eq!(r.prompt().unwrap().focus, Answer::EndTask);
        assert_eq!(r.key(28), Some((A, W, Answer::EndTask)));
        // Pointer: press and release on End task.
        r.request_close(A, W, "Hang");
        let area = Rect::new(0, 0, 1920, 1032).unwrap();
        let g = geometry(area);
        assert!(g.panel.contains(g.end_task.x(), g.end_task.y()));
        assert!(g.end_task.right() <= g.wait.x());
        let (x, y) = (g.end_task.x() + 4, g.end_task.y() + 4);
        assert_eq!(r.button(area, x, y, true), (true, None));
        assert_eq!(
            r.button(area, x, y, false),
            (true, Some((A, W, Answer::EndTask)))
        );
        // A release elsewhere never answers; outside presses pass through.
        r.request_close(A, W, "Hang");
        assert_eq!(r.button(area, x, y, true), (true, None));
        assert_eq!(
            r.button(area, g.wait.x() + 2, g.wait.y() + 2, false),
            (true, None)
        );
        assert_eq!(r.button(area, 2, 2, true), (false, None));
        assert!(r.prompt().is_some());
        r.forget(A);
        assert!(r.prompt().is_none() && !r.is_client(A));
    }
}
