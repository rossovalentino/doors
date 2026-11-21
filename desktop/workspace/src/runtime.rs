//! Shared native multiwindow lifecycle. Model state survives display reconnects.
use crate::document_tabs::Drag;
use crate::{
    surface::Surface,
    ui::{ImageCache, View},
};
use alloc::{format, string::String};
use door_abi::{
    Status,
    platform::{StartupV2, inherited},
};
use door_graphics_core::Scale;
use door_graphics_text::FontAtlas;
use door_service_client::{
    Error, Storage, StorageConnect,
    window::{Rect, WindowClient},
};
use door_service_protocol::{Code, window as w};
/// Minimum ticks between two semantic-tree publications of one window.
const SEMANTIC_TICKS: u64 = 3;
/// A window-loop failure that ends the app names its source line
/// (`CLIENT_FAILED`), since `APP FAILED` alone cannot say which call it was.
fn failed(at: u32, e: Error) -> Error {
    door_rt::diagnostic(format_args!(
        "DOOR_P12 CLIENT_FAILED at={at} error={e:?}
"
    ));
    e
}
/// Three seconds in scheduler ticks (`PRESENT_STALL`), read once.
fn stall_ticks() -> u64 {
    use core::sync::atomic::{AtomicU64, Ordering};
    static TICKS: AtomicU64 = AtomicU64::new(0);
    let known = TICKS.load(Ordering::Relaxed);
    if known != 0 {
        return known;
    }
    let ticks = door_sys::query().map_or(3000, |q| 3 * q.tick_hz.max(1));
    TICKS.store(ticks, Ordering::Relaxed);
    ticks
}
trait CloseAuthority {
    fn cancel(&mut self, window: u64) -> Result<(), Error>;
    fn ack(&mut self, window: u64, epoch: u64) -> Result<(), Error>;
}
impl CloseAuthority for WindowClient {
    fn cancel(&mut self, window: u64) -> Result<(), Error> {
        self.cancel_close(window)
    }
    fn ack(&mut self, window: u64, epoch: u64) -> Result<(), Error> {
        self.acknowledge(window, epoch)
    }
}
// An ACK interrupted by Closing did not grant paint authority. Cancellation
// restores the live window, then idempotently restores the current epoch ACK.
// A newer configure or repeated close is drained before painting.
fn resume_after_close(
    client: &mut impl CloseAuthority,
    window: u64,
    epoch: u64,
) -> Result<bool, Error> {
    client.cancel(window)?;
    if epoch == 0 {
        return Ok(false);
    }
    match client.ack(window, epoch) {
        Ok(()) | Err(Error::Service(Code::StaleInstance)) => Ok(false),
        Err(Error::Service(Code::Closing)) => Ok(true),
        Err(e) => Err(e),
    }
}
#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    struct Window {
        closing: bool,
        epoch: u64,
        acknowledged: bool,
        repeat_close: bool,
    }
    impl CloseAuthority for Window {
        fn cancel(&mut self, _: u64) -> Result<(), Error> {
            self.closing = self.repeat_close;
            Ok(())
        }
        fn ack(&mut self, _: u64, epoch: u64) -> Result<(), Error> {
            if self.closing {
                return Err(Error::Service(Code::Closing));
            }
            if epoch != self.epoch {
                return Err(Error::Service(Code::StaleInstance));
            }
            self.acknowledged = true;
            Ok(())
        }
    }
    #[test]
    fn cancelled_export_close_recovers_interrupted_configure_authority() {
        let mut w = Window {
            closing: true,
            epoch: 7,
            acknowledged: false,
            repeat_close: false,
        };
        assert_eq!(w.ack(1, 7), Err(Error::Service(Code::Closing)));
        assert!(!resume_after_close(&mut w, 1, 7).unwrap());
        assert!(w.acknowledged);
        // A configure already queued after the locally known epoch must drain.
        w.epoch = 8;
        w.acknowledged = false;
        assert!(!resume_after_close(&mut w, 1, 7).unwrap());
        assert!(!w.acknowledged);
        w.ack(1, 8).unwrap();
        w.repeat_close = true;
        assert!(resume_after_close(&mut w, 1, 8).unwrap());
    }
}

#[derive(Clone, Copy)]
pub struct WindowOptions {
    pub width: u32,
    pub height: u32,
    pub caption_height: u32,
    pub caption_left: u32,
    pub centered: bool,
}
impl Default for WindowOptions {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 800,
            caption_height: 48,
            caption_left: 220,
            centered: false,
        }
    }
}
pub trait Application: Sized {
    fn title(&self) -> &str;
    fn busy(&self) -> bool {
        false
    }
    fn closing(&mut self, _s: &mut Storage) -> Result<bool, Error> {
        Ok(true)
    }
    fn caption(&self) -> (u32, u32) {
        (220, 1)
    }
    fn options(&self) -> WindowOptions {
        WindowOptions::default()
    }
    fn configure(&mut self, width: u32, height: u32);
    fn configure_fonts(&mut self, w: u32, h: u32, _f: &FontAtlas) {
        self.configure(w, h)
    }
    fn draw(&self, v: &mut View<'_>);
    fn target(&self, x: i32, y: i32) -> Option<u32>;
    fn target_fonts(&self, x: i32, y: i32, _f: &FontAtlas) -> Option<u32> {
        self.target(x, y)
    }
    fn click(&mut self, id: u32, s: &mut Storage) -> Result<bool, Error>;
    fn click_at(
        &mut self,
        id: u32,
        _x: i32,
        _y: i32,
        s: &mut Storage,
        _f: &FontAtlas,
    ) -> Result<bool, Error> {
        self.click(id, s)
    }
    fn key(&mut self, key: u32, s: &mut Storage) -> Result<bool, Error>;
    fn key_fonts(&mut self, key: u32, s: &mut Storage, _f: &FontAtlas) -> Result<bool, Error> {
        self.key(key, s)
    }
    fn tick(&mut self, _s: &mut Storage, _now: u64) -> Result<bool, Error> {
        Ok(false)
    }
    fn update(
        &mut self,
        _s: &mut Storage,
        _c: &mut WindowClient,
        _window: u64,
    ) -> Result<bool, Error> {
        Ok(false)
    }
    fn close_failed(&mut self) {}
    fn wants_close(&mut self) -> bool {
        false
    }
    /// The user chose Cancel in this window's close prompt (taken once). The
    /// runtime reports it so a shutdown waiting on the prompt is cancelled.
    fn take_close_declined(&mut self) -> bool {
        false
    }
    fn close_shortcut(&self) -> bool {
        true
    }
    fn new_window_key(&self, key: u32) -> bool {
        let m = key >> 9;
        key & 511 == 49
            && m & door_abi::graphics::CONTROL != 0
            && m & door_abi::graphics::SHIFT != 0
    }
    fn keyboard_context(&mut self, w: u32, h: u32) {
        self.context(140, 112, w, h)
    }
    fn modal(&self) -> bool {
        false
    }
    fn menu_open(&self) -> bool {
        false
    }
    fn dismiss_menu(&mut self) {}
    fn pointer_down(&mut self, _x: i32, _y: i32, _f: &FontAtlas) -> bool {
        false
    }
    fn pointer_motion(&mut self, _x: i32, _y: i32, _f: &FontAtlas) -> bool {
        false
    }
    fn pointer_up(&mut self) {}
    /// Mouse wheel over (x, y) in logical pixels. `notches` > 0 scrolls up /
    /// away from the user (one detent = 1). Return true to repaint.
    fn wheel(&mut self, _x: i32, _y: i32, _notches: i32, _f: &FontAtlas) -> bool {
        false
    }
    /// Whether the window's translucent pixels should show a live blurred
    /// backdrop of the windows behind it (acrylic) rather than the wallpaper
    /// material. Read every frame; the runtime forwards changes.
    fn acrylic(&self) -> bool {
        false
    }
    /// Whether the window is drawn over Mica (Windows 11 Settings): the
    /// wallpaper under the theme tint behind its translucent pixels.
    fn mica(&self) -> bool {
        false
    }
    /// Whether the process observes the typed settings registry
    /// (`settings_watch`); Settings commits through its own.
    fn observes_settings(&self) -> bool {
        true
    }
    /// Display mode from the window server greeting: physical width and
    /// height and the integer density. Called on every (re)connection.
    /// Physical output size and display scale in percent (100, 125, ...).
    fn display(&mut self, _width: u32, _height: u32, _percent: u32) {}
    fn context(&mut self, _x: i32, _y: i32, _w: u32, _h: u32) {}
    fn text_hit(&self, _x: i32, _y: i32) -> bool {
        false
    }
    /// Pointer shape at (x, y) (`window::CURSOR_*`): the I-beam over text,
    /// else the arrow; apps with splitters add the resize shape.
    fn cursor_at(&self, x: i32, y: i32) -> u32 {
        if self.text_hit(x, y) {
            w::CURSOR_TEXT
        } else {
            w::CURSOR_ARROW
        }
    }
    fn event(&mut self, _e: w::Event) -> Result<bool, Error> {
        Ok(false)
    }
    /// DOCUMENT_OPEN from the shell (Start search): open `payload`, an
    /// encoded Explorer request for File Explorer or a Door path for Notes
    /// and Preview. `launched`: the app was started for this document, so
    /// its first window may show it in place. Returns true to repaint.
    fn open_document(
        &mut self,
        _payload: &[u8],
        _launched: bool,
        _s: &mut Storage,
    ) -> Result<bool, Error> {
        Ok(false)
    }
    /// Construct a fresh model without moving source documents or writing
    /// existing data. Configure/paint may precede admission; input and mutating
    /// tick/update work begin only after its first successful frame commit.
    fn new_window(&self, _s: &mut Storage) -> Result<Option<Self>, Error> {
        Ok(None)
    }
    fn take_new_window(&mut self) -> bool {
        false
    }
    fn tab_at(&self, _x: i32, _y: i32) -> Option<usize> {
        None
    }
    fn tab_identity(&self, _index: usize) -> Option<u64> {
        None
    }
    fn tab_index(&self, _id: u64) -> Option<usize> {
        None
    }
    fn transfer_tab(
        &mut self,
        _target: &mut Self,
        _index: usize,
        _s: &mut Storage,
    ) -> Result<bool, Error> {
        Ok(false)
    }
    fn reorder_tab(&mut self, _from: usize, _to: usize) -> bool {
        false
    }
    fn empty_after_transfer(&self) -> bool {
        false
    }
    fn retired(&mut self) {}
    fn disconnected(&mut self) {}
    fn focus(&mut self, _focused: bool) {}
    /// HELLO capabilities beyond the runtime's own (an editor asks for
    /// `text_input::CAP_TEXT_INPUT` and `clipboard::CAP_CLIPBOARD`).
    fn capabilities(&self) -> u32 {
        0
    }
    /// The application publishes its own semantic tree (the door-ui
    /// adapter); the runtime's shared adapter then stays out of the way.
    fn publishes_semantics(&self) -> bool {
        false
    }
    /// Describe this window's controls for door.accessibility from the same
    /// layout hit testing uses (`crate::semantics`). The runtime publishes
    /// the tree (the window root is named by `title`) and clicks a node's
    /// target for assistive Invoke.
    fn semantics(&self, _s: &mut crate::semantics::Semantics, _f: &FontAtlas) {}
    /// Ordered door.text-input text for this window (`crate::text_entry`);
    /// keys of an enabled context arrive as ordinary key input instead.
    fn text_input(
        &mut self,
        _event: crate::text_entry::TextEvent,
        _s: &mut Storage,
        _f: &FontAtlas,
    ) -> Result<bool, Error> {
        Ok(false)
    }
    fn recreated(&mut self) {}
    fn monospace_layout(&self) -> bool {
        false
    }
    fn monospace(&self) -> bool {
        false
    }
}
/// Drain server backlog across the three-event wire batches before repainting.
/// Lifecycle effects still run for every event; only tick/update/paint are batched.
/// Work runs after a genuine empty poll or at the next local batch boundary after
/// 64 events (at most 66). Suppressed provisional input counts toward this bound.
#[derive(Default)]
struct InputDrain {
    count: usize,
}
impl InputDrain {
    fn work_due(&mut self, polled: bool, local_pending: bool) -> bool {
        if polled {
            self.count += 1;
        }
        if !local_pending && (!polled || self.count >= 64) {
            self.count = 0;
            true
        } else {
            false
        }
    }
}
#[derive(Clone, Copy)]
struct PendingAdmission {
    source: u64,
    target: u64,
    tab: Option<u64>,
    deadline: u64,
}
impl PendingAdmission {
    fn blocks_input(self, e: w::Event) -> bool {
        e.kind == w::INPUT && e.window == self.target
    }
}

enum Model<'a, A> {
    Root(&'a mut A),
    Owned(A),
}
impl<A> core::ops::Deref for Model<'_, A> {
    type Target = A;
    fn deref(&self) -> &A {
        match self {
            Self::Root(a) => a,
            Self::Owned(a) => a,
        }
    }
}
impl<A> core::ops::DerefMut for Model<'_, A> {
    fn deref_mut(&mut self) -> &mut A {
        match self {
            Self::Root(a) => a,
            Self::Owned(a) => a,
        }
    }
}
struct Instance<'a, A> {
    app: Model<'a, A>,
    window: u64,
    epoch: u64,
    surface: Option<Surface>,
    dirty: bool,
    closing: bool,
    ready: bool,
    hover: Option<(i32, i32)>,
    pressed: Option<(i32, i32)>,
    /// A key cancelled a press held inside a modal; its release is ignored.
    modal_press_cancelled: bool,
    drag: Option<Drag>,
    title: String,
    status: String,
    rect: Rect,
    cursor: u32,
    /// Last material the server accepted for this window (None: not sent).
    material: Option<u32>,
    /// door.accessibility publication of this window's semantic tree.
    semantics: crate::semantics::Publisher,
    /// Since when the window has needed a frame it has not presented, and
    /// whether that stall was reported (`PRESENT_STALL`, once per window).
    dirty_since: Option<u64>,
    stall_reported: bool,
    /// The last refusal on the frame path (`PRESENT_STALL` reason).
    present_refusal: &'static str,
}
impl<'a, A: Application> Instance<'a, A> {
    fn new(app: Model<'a, A>) -> Self {
        Self {
            title: app.title().into(),
            app,
            window: 0,
            epoch: 0,
            surface: None,
            dirty: true,
            closing: false,
            ready: false,
            hover: None,
            pressed: None,
            modal_press_cancelled: false,
            drag: None,
            status: String::new(),
            rect: Rect::default(),
            cursor: w::CURSOR_ARROW,
            material: None,
            semantics: crate::semantics::Publisher::default(),
            dirty_since: None,
            stall_reported: false,
            present_refusal: "none",
        }
    }
    /// Build this window's semantic tree from the app's description and
    /// publish what changed (full tree first, then deltas). At most one
    /// publication per `SEMANTIC_TICKS`, so animations cannot flood the
    /// broker; the latest state is published once the slot comes.
    fn publish_semantics(&mut self, c: &mut WindowClient, zoom: Scale, f: &FontAtlas, now: u64) {
        if !self.semantics.stale
            || now < self.semantics.next_at
            || self.closing
            || !self.ready
            || self.window == 0
            || !c.supports_accessibility()
            || self.app.publishes_semantics()
        {
            return;
        }
        let Some((width, height)) = self.client_size(zoom) else {
            return;
        };
        let mut s = crate::semantics::Semantics::new(width, height);
        self.app.semantics(&mut s, f);
        let mut built = s.finish(self.window, 1, self.app.title());
        self.semantics.next_at = now.saturating_add(SEMANTIC_TICKS);
        let sent = match self.semantics.prepare(&mut built) {
            None => {
                self.semantics.stale = false;
                return;
            }
            Some(crate::semantics::Message::Full(bytes)) => {
                c.accessibility_publish(self.window, &bytes)
            }
            Some(crate::semantics::Message::Delta(bytes)) => {
                match c.accessibility_publish(self.window, &bytes) {
                    // The broker holds another version: send the whole tree.
                    Err(Error::Service(Code::Changed)) => {
                        match crate::semantics::Publisher::full(&built) {
                            Some(full) => c.accessibility_publish(self.window, &full),
                            None => return,
                        }
                    }
                    other => other,
                }
            }
        };
        match sent {
            Ok(_) => self.semantics.committed(built),
            // Retried at the next slot from a fresh full tree.
            Err(_) => self.semantics.reset(),
        }
    }
    /// An assistive action on one of this window's nodes: Invoke, Select,
    /// Toggle, Expand and Collapse click the node's target, exactly as the
    /// pointer would; anything else is refused.
    fn semantic_action(
        &mut self,
        c: &mut WindowClient,
        serial: u32,
        storage: &mut Storage,
        f: &FontAtlas,
    ) -> Result<bool, Error> {
        use door_service_protocol::accessibility as a;
        let mut arg = [0u8; a::ARG_BYTES];
        let request = match c.accessibility_action(serial, &mut arg) {
            Ok(r) => r,
            Err(Error::Service(_)) => return Ok(false),
            Err(e) => return Err(failed(line!(), e)),
        };
        let point = self
            .semantics
            .published()
            .and_then(|b| Some((b.target(request.node)?, b.center(request.node)?)));
        let clicks = matches!(
            request.action,
            a::ACTION_INVOKE
                | a::ACTION_SELECT
                | a::ACTION_TOGGLE
                | a::ACTION_EXPAND
                | a::ACTION_COLLAPSE
        );
        let (result, dirty) = match point {
            Some((target, (x, y))) if clicks && request.window == self.window && !self.closing => {
                match self.app.click_at(target, x, y, storage, f) {
                    Ok(d) => (a::RESULT_DONE, d),
                    Err(_) => (a::RESULT_REJECTED, true),
                }
            }
            Some(_) => (a::RESULT_REJECTED, false),
            None => (a::RESULT_GONE, false),
        };
        match c.accessibility_action_done(serial, result) {
            Ok(()) | Err(Error::Service(_)) => Ok(dirty),
            Err(e) => Err(e),
        }
    }
    fn reset_input(&mut self) {
        self.app.pointer_up();
        self.app.dismiss_menu();
        self.pressed = None;
        self.hover = None;
        self.drag = None;
        self.dirty = true;
    }
    /// Painted client size in logical pixels, once a surface exists.
    fn client_size(&self, zoom: Scale) -> Option<(u32, u32)> {
        self.surface
            .as_ref()
            .map(|p| (zoom.logical_len(p.width), zoom.logical_len(p.height)))
    }
    /// Pointer motion; returns whether the hovered target changed. The server
    /// routes pointer input only to the window under the pointer (or the
    /// capture holder) and sends no leave event, so any point outside the
    /// client (this window's frame, or captured motion beyond it) clears hover.
    fn move_pointer(&mut self, x: i32, y: i32, size: Option<(u32, u32)>, f: &FontAtlas) -> bool {
        let next = inside(x, y, size).then_some((x, y));
        let target = |p: Option<(i32, i32)>| p.and_then(|(a, b)| self.app.target_fonts(a, b, f));
        let changed = target(next) != target(self.hover);
        self.hover = next;
        changed
    }
    /// A key press ends a tab drag, and a held press inside a modal (the
    /// key may dismiss it). Returns whether pointer input was reset, so the
    /// caller releases capture.
    fn key_cancels_pointer(&mut self) -> bool {
        let modal_press = self.app.modal() && self.pressed.is_some();
        let cancel = self.drag.is_some() || modal_press;
        if cancel {
            self.reset_input();
        }
        self.modal_press_cancelled |= modal_press;
        cancel
    }
    /// Button down: the press, and hover when it lands inside the client.
    fn press_pointer(&mut self, x: i32, y: i32, size: Option<(u32, u32)>) {
        self.hover = inside(x, y, size).then_some((x, y));
        self.pressed = Some((x, y));
        self.modal_press_cancelled = false;
    }
    /// Button up: completes and returns the matching press. A release whose
    /// press was cancelled (taken by a modal, capture lost, focus change) sets
    /// neither hover nor pressed, so the control underneath shows no plate.
    fn release_pointer(&mut self, x: i32, y: i32, size: Option<(u32, u32)>) -> Option<(i32, i32)> {
        let press = self.pressed.take();
        if press.is_some() {
            self.hover = inside(x, y, size).then_some((x, y));
        }
        press
    }
    /// Whether this release ends a modal press a key cancelled (once).
    fn release_was_cancelled(&mut self) -> bool {
        core::mem::take(&mut self.modal_press_cancelled)
    }
    fn create(
        &mut self,
        c: &mut WindowClient,
        ow: u32,
        oh: u32,
        zoom: Scale,
        index: usize,
        position: Option<(i32, i32)>,
    ) -> Result<(), Error> {
        let o = self.app.options();
        // Shared with host tools that derive input points from placement.
        let (x, y, width, height) =
            door_graphics_core::initial_window((ow, oh), (o.width, o.height), index, position);
        self.rect = Rect {
            x: zoom.px(x),
            y: zoom.px(y),
            width: zoom.len(width),
            height: zoom.len(height),
        };
        let surface = Surface::new(c, self.rect.width, self.rect.height)?;
        self.window = match if o.centered {
            // Settings' centred title-bar search (`settings_layout::Layout`).
            c.create_client_caption(
                self.rect,
                &self.title,
                zoom.len(crate::settings_layout::Layout::CAPTION_CONTROL),
                zoom.len(crate::settings_layout::Layout::CAPTION_MARGIN),
            )
        } else {
            c.create_tabbed_caption_with_height(
                self.rect,
                &self.title,
                zoom.len(o.caption_left),
                o.caption_height,
            )
        } {
            Ok(id) => id,
            Err(e) => {
                surface.unregister(c);
                return Err(e);
            }
        };
        self.surface = Some(surface);
        door_rt::diagnostic(format_args!(
            "DOOR_P12 WINDOW_CREATED id={} title={}\n",
            self.window, self.title
        ));
        self.epoch = 0;
        self.ready = false;
        self.closing = false;
        self.reset_input();
        self.material = None;
        self.app.recreated();
        Ok(())
    }
}
/// Whether a client-local logical point lies on the painted client.
fn inside(x: i32, y: i32, size: Option<(u32, u32)>) -> bool {
    size.is_none_or(|(w, h)| x >= 0 && y >= 0 && (x as i64) < w as i64 && (y as i64) < h as i64)
}
/// Discard only a pending factory destination, never a borrowed source model.
fn take_admission_target<'a, A>(
    windows: &mut alloc::vec::Vec<Instance<'a, A>>,
    target: u64,
) -> Option<Instance<'a, A>> {
    let i = windows.iter().position(|s| s.window == target)?;
    matches!(&windows[i].app, Model::Owned(_)).then(|| windows.remove(i))
}
/// Restore keyboard focus only to an admitted, live sibling. Never activate a
/// provisional destination that has not finished the admission transaction.
fn focus_survivor<A>(
    windows: &[Instance<'_, A>],
    preferred: Option<u64>,
    excluded: Option<u64>,
) -> Option<u64> {
    let eligible =
        |s: &&Instance<'_, A>| s.ready && !s.closing && s.window != 0 && Some(s.window) != excluded;
    windows
        .iter()
        .filter(eligible)
        .find(|s| Some(s.window) == preferred)
        .or_else(|| windows.iter().rev().find(eligible))
        .map(|s| s.window)
}
/// A single native UI thread admits bounded codec/factory working sets before
/// invoking libraries with infallible internal allocations. No other UI model
/// allocates between this check and the synchronous operation it guards.
pub fn working_memory(bytes: usize) -> bool {
    let mut scratch = alloc::vec::Vec::<u8>::new();
    scratch.try_reserve_exact(bytes).is_ok()
}
fn finish_admission<A: Application>(
    s: &mut Instance<'_, A>,
    result: Result<Option<A>, Error>,
) -> Option<A> {
    match result {
        Ok(a) => a,
        Err(e) => {
            s.status =
                format!("New window could not be opened; current documents are retained: {e:?}");
            s.dirty = true;
            None
        }
    }
}
fn new_model<A: Application>(s: &mut Instance<'_, A>, storage: &mut Storage) -> Option<A> {
    if !working_memory(24 * 1024 * 1024) {
        return finish_admission(s, Err(Error::Service(Code::Resources)));
    }
    let result = s.app.new_window(storage);
    finish_admission(s, result)
}
fn begin_close<A: Application>(
    s: &mut Instance<'_, A>,
    client: &mut WindowClient,
) -> Result<(), Error> {
    match client.request_close(s.window) {
        Ok(()) | Err(Error::Service(Code::Closing)) => s.closing = true,
        Err(e) => {
            s.app.close_failed();
            s.dirty = true;
            s.status = format!("Window could not close; documents are retained: {e:?}");
            if matches!(
                e,
                Error::Transport(Status::PeerClosed | Status::Timeout | Status::NotFound)
            ) {
                return Err(e);
            }
        }
    }
    Ok(())
}
/// One authenticated Window connection routes all windows by their actual IDs.
/// Failed admission never removes a tab. Each accepted move transfers its owned
/// model exactly once; the source is closed only after the target owns the tab.
/// UI text atlas at the display density. Native processes read the hinted
/// Segoe UI atlas for a display scale, from the shared application asset
/// pack (no executable embeds it): the hinted 1x/2x/3x atlases or the
/// exported 125/150/175% ones. Host builds use the pinned reference atlas
/// at integer scales and the same asset files at fractional ones.
/// Accessibility > Text size multiplies the display scale for glyphs only:
/// the hinted atlas for display x text when one exists (for example the
/// 150% atlas at 100% display and 150% text), otherwise Segoe UI outlines
/// from the asset pack rasterized at that density.
fn ui_fonts(zoom: Scale, text: u32) -> Result<FontAtlas, Error> {
    let atlas = if text == 100 {
        ui_atlas(zoom)?
    } else {
        text_atlas(zoom, text)?
    };
    unicode(atlas, door_graphics_text::Family::Ui)
}
fn text_atlas(zoom: Scale, text: u32) -> Result<FontAtlas, Error> {
    let display = zoom.percent();
    let atlas = match FontAtlas::hinted_percent(display, text) {
        Some(p) => {
            let key = match p {
                100 => "fonts/segoe-ui/hinted-1x.dfa",
                125 => "fonts/segoe-ui/hinted-125.dfa",
                150 => "fonts/segoe-ui/hinted-150.dfa",
                175 => "fonts/segoe-ui/hinted-175.dfa",
                200 => "fonts/segoe-ui/hinted-2x.dfa",
                _ => "fonts/segoe-ui/hinted-3x.dfa",
            };
            FontAtlas::from_packed_text(crate::assets::get(key), display, text)
        }
        None => FontAtlas::outline_text(
            crate::assets::get("fonts/segoe-ui/segoeui.ttf"),
            crate::assets::get("fonts/segoe-ui/seguisb.ttf"),
            display,
            text,
        ),
    };
    atlas.map_err(|_| Error::Transport(Status::IoFailure))
}
/// The UI atlas for the session's requested text size, or the 100% one if
/// that cannot be built; the applied size is recorded for layout.
fn ui_fonts_requested(zoom: Scale) -> Result<FontAtlas, Error> {
    let requested = crate::palette::requested_text_percent();
    let fonts = ui_fonts(zoom, requested).or_else(|_| ui_fonts(zoom, 100))?;
    crate::palette::set_text_percent(fonts.text_percent());
    Ok(fonts)
}
/// Text outside ASCII (any script, emoji) goes through the process's
/// Unicode engine; ASCII keeps the hinted atlas pixels.
fn unicode(mut atlas: FontAtlas, family: door_graphics_text::Family) -> Result<FontAtlas, Error> {
    if let Some(system) = crate::assets::text_system() {
        atlas.attach_unicode(system, family);
    }
    Ok(atlas)
}
fn ui_atlas(zoom: Scale) -> Result<FontAtlas, Error> {
    let key = match zoom.percent() {
        100 => "fonts/segoe-ui/hinted-1x.dfa",
        125 => "fonts/segoe-ui/hinted-125.dfa",
        150 => "fonts/segoe-ui/hinted-150.dfa",
        175 => "fonts/segoe-ui/hinted-175.dfa",
        200 => "fonts/segoe-ui/hinted-2x.dfa",
        300 => "fonts/segoe-ui/hinted-3x.dfa",
        // 225/250% have no exported atlas; the window server never offers them.
        _ => return Err(Error::Malformed),
    };
    #[cfg(not(target_os = "none"))]
    if let Some(factor) = zoom.factor() {
        let _ = key;
        return FontAtlas::windows_reference_scaled(factor)
            .map_err(|_| Error::Transport(Status::IoFailure));
    }
    FontAtlas::from_packed_percent(crate::assets::get(key), zoom.percent())
        .map_err(|_| Error::Transport(Status::IoFailure))
}
/// Fixed-cell atlas for editors and consoles (Cascadia Mono) at a display
/// scale, likewise from the asset pack on native targets.
fn mono_fonts(zoom: Scale) -> Result<FontAtlas, Error> {
    #[cfg(target_os = "none")]
    let atlas = FontAtlas::monospace_from_percent(
        crate::assets::get("fonts/cascadia-mono/CascadiaMono.ttf"),
        zoom.percent(),
    );
    #[cfg(not(target_os = "none"))]
    let atlas = FontAtlas::monospace_reference_percent(zoom.percent());
    unicode(
        atlas.map_err(|_| Error::Transport(Status::IoFailure))?,
        door_graphics_text::Family::Monospace,
    )
}
/// Display scale from a HELLO DPI (96 x percent / 100).
fn hello_zoom(dpi: u32) -> Option<Scale> {
    if dpi == 0 || !(dpi * 100).is_multiple_of(96) {
        return None;
    }
    Scale::from_percent(dpi * 100 / 96)
}
pub fn run(startup: &StartupV2, app: &mut impl Application) -> Result<(), Error> {
    // Fonts and icons come from the kernel-shared read-only asset section.
    #[cfg(target_os = "none")]
    crate::assets::install_from_startup(startup).map_err(Error::Transport)?;
    let registry = startup.base.inherited[inherited::REGISTRY].handle;
    let mut storage = loop {
        match Storage::connect(registry) {
            Ok(s) => break s,
            Err(Error::Transport(
                Status::PeerClosed | Status::Timeout | Status::NotFound | Status::WouldBlock,
            ))
            | Err(Error::Service(Code::WouldBlock)) => door_sys::sleep(door_sys::time()? + 10)?,
            Err(e) => return Err(failed(line!(), e)),
        }
    };
    // Every desktop process reads its settings through the typed registry
    // and observes changes made by others (P14-REGISTRY).
    let mut settings_watch = app
        .observes_settings()
        .then(|| crate::settings_watch::Watch::load(&mut storage, registry));
    let mut windows = alloc::vec![Instance::new(Model::Root(app))];
    let mut reconnect: Option<StorageConnect> = None;
    let mut retry_at = 0;
    let mut storage_retired = false;
    let mut pending: Option<PendingAdmission> = None;
    // DOCUMENT_OPEN payload waiting for an admitted window.
    let mut document: Option<(alloc::vec::Vec<u8>, bool)> = None;
    loop {
        let outcome = (|| -> Result<(), Error> {
            // Fractional display scales: the HELLO DPI is 96 x percent / 100
            // and SCALE_CHANGED reports every later change.
            let extra = windows.iter().fold(0, |m, s| m | s.app.capabilities());
            // Every window publishes a semantic tree (door.accessibility).
            let extra = extra | door_service_protocol::accessibility::CAP_ACCESSIBILITY;
            let mut client = WindowClient::connect_with(registry, w::CAP_DISPLAY_SCALE | extra)?;
            if !client.supports_epoch_commit() {
                return Err(Error::Service(Code::Unsupported));
            }
            let hello = client.hello()?;
            let number = |at| u32::from_le_bytes(hello[at..at + 4].try_into().unwrap());
            let mut zoom = hello_zoom(number(16)).ok_or(Error::Malformed)?;
            let mut output = (number(4), number(8));
            let (mut ow, mut oh) = (zoom.logical_len(output.0), zoom.logical_len(output.1));
            let mut dark = false;
            let mut fonts = ui_fonts_requested(zoom)?;
            let mut mono = if windows.iter().any(|s| s.app.monospace()) {
                Some(mono_fonts(zoom)?)
            } else {
                None
            };
            let mono_layout = windows.first().is_some_and(|s| s.app.monospace_layout());
            let mut rescale: Option<Scale> = None;
            // CONTROLS_CHANGED with a new text size: new atlas, relayout.
            let mut retext = false;
            let images = ImageCache::default();
            for (i, s) in windows.iter_mut().enumerate() {
                s.app.display(output.0, output.1, zoom.percent());
                // A new connection holds no tree: publish it whole.
                s.semantics.reset();
                s.create(&mut client, ow, oh, zoom, i, None)?;
            }
            let mut drain = InputDrain::default();
            // Text-input records waiting for dispatch, in server order.
            let mut replay = alloc::collections::VecDeque::<crate::text_entry::Routed>::new();
            loop {
                // A scale change applies before the CONFIGURED events that
                // follow it: new atlases, logical output, full repaint.
                if let Some(next) = rescale.take() {
                    zoom = next;
                    fonts = ui_fonts_requested(zoom)?;
                    retext = false;
                    if mono.is_some() {
                        mono = Some(mono_fonts(zoom)?);
                    }
                    (ow, oh) = (zoom.logical_len(output.0), zoom.logical_len(output.1));
                    for s in &mut windows {
                        s.reset_input();
                    }
                    door_rt::diagnostic(format_args!(
                        "DOOR_P12 CLIENT_SCALE percent={}\n",
                        zoom.percent()
                    ));
                }
                // Text size applies live: windows keep their size, every
                // text-holding layout is recomputed with the new atlas.
                if core::mem::take(&mut retext) {
                    let applied = fonts.text_percent();
                    fonts = ui_fonts_requested(zoom)?;
                    let layout_fonts = if mono_layout {
                        mono.as_ref().unwrap_or(&fonts)
                    } else {
                        &fonts
                    };
                    for s in &mut windows {
                        if let Some(p) = s.surface.as_ref() {
                            s.app.configure_fonts(
                                zoom.logical_len(p.width),
                                zoom.logical_len(p.height),
                                layout_fonts,
                            );
                        }
                        s.dirty = true;
                    }
                    door_rt::diagnostic(format_args!(
                        "DOOR_P12 CLIENT_TEXT_SIZE from={} percent={} requested={}\n",
                        applied,
                        fonts.text_percent(),
                        crate::palette::requested_text_percent()
                    ));
                }
                let layout_fonts = if mono_layout {
                    mono.as_ref().unwrap_or(&fonts)
                } else {
                    &fonts
                };
                let now = door_sys::time()?;
                if storage.needs_reconnect() {
                    if !storage_retired {
                        for s in &mut windows {
                            s.app.disconnected();
                            s.reset_input();
                            s.status = "Storage disconnected; unsaved state retained.".into();
                        }
                        storage_retired = true;
                        if let Some(watch) = settings_watch.as_mut() {
                            watch.disconnected();
                        }
                    }
                    if reconnect.is_none() && now >= retry_at {
                        retry_at = now.saturating_add(100);
                        reconnect = StorageConnect::begin(registry, now).ok();
                    }
                    if let Some(attempt) = reconnect.as_mut() {
                        match attempt.poll(now) {
                            Ok(Some(s)) => {
                                storage = s;
                                reconnect = None;
                                storage_retired = false;
                                for s in &mut windows {
                                    s.dirty = true;
                                    s.status = "Storage connected. Refresh before retrying.".into();
                                }
                                door_rt::diagnostic(format_args!("DOOR_P12 STORAGE RECONNECTED\n"));
                            }
                            Ok(None) => (),
                            Err(_) => {
                                reconnect = None;
                                retry_at = now.saturating_add(100);
                            }
                        }
                    }
                }
                if !storage_retired && let Some(watch) = settings_watch.as_mut() {
                    watch.poll(&mut storage, now);
                }
                let (polled, replayed, mut text_event) = match replay.pop_front() {
                    Some((e, text)) => (Some(e), true, text),
                    None => (client.poll()?, false, None),
                };
                let work_due = drain.work_due(
                    polled.is_some(),
                    client.pending_events() || !replay.is_empty(),
                );
                // Filter only after counting, so held provisional input cannot
                // starve painting. Admission deadlines run on every iteration.
                let event = polled.filter(|e| !pending.is_some_and(|p| p.blocks_input(*e)));
                let mut spawn = None;
                let mut tab_drop = None;
                let mut remove = None;
                if let Some(e) = event {
                    if e.kind == w::NEW_WINDOW {
                        if pending.is_none()
                            && let Some(s) = windows.iter_mut().find(|s| s.ready && !s.closing)
                            && let Some(a) = new_model(s, &mut storage)
                        {
                            spawn = Some((s.window, a, None));
                        }
                    } else if matches!(e.kind, w::CONTROLS_CHANGED | w::SETTINGS_NAVIGATE) {
                        if e.kind == w::CONTROLS_CHANGED {
                            retext |=
                                crate::palette::requested_text_percent() != fonts.text_percent();
                        }
                        for s in &mut windows {
                            s.dirty |= s.app.event(e).map_err(|e| failed(line!(), e))?;
                        }
                    } else if e.kind == w::DOCUMENT_OPEN {
                        let mut payload = [0u8; w::MAX_DOCUMENT];
                        match client.document_get(&mut payload) {
                            Ok(n) => document = Some((payload[..n].to_vec(), e.b != 0)),
                            // Expired or taken: nothing to open.
                            Err(Error::Service(_)) => (),
                            Err(error) => return Err(error),
                        }
                    } else if e.kind == w::RELEASED {
                        for s in &mut windows {
                            if let Some(p) = s.surface.as_mut()
                                && p.buffer == e.value
                            {
                                p.available = true;
                            }
                        }
                    } else if e.kind == w::SCALE_CHANGED {
                        let next = Scale::from_percent(e.a).ok_or(Error::Malformed)?;
                        output = (e.rect.width, e.rect.height);
                        for s in &mut windows {
                            s.app.display(output.0, output.1, next.percent());
                            s.dirty = true;
                        }
                        if next != zoom {
                            rescale = Some(next);
                        }
                    } else if e.kind == w::APPEARANCE_CHANGED {
                        // Mode bit plus the desktop picture (18 pictures:
                        // window::APPEARANCE_VALUES words).
                        if e.value >= u64::from(w::APPEARANCE_VALUES) {
                            return Err(Error::Malformed);
                        }
                        dark = e.value & 1 != 0;
                        for s in &mut windows {
                            s.app.event(e).map_err(|e| failed(line!(), e))?;
                            s.dirty = true;
                        }
                    } else if e.kind == door_service_protocol::text_input::PENDING && !replayed {
                        // Drain the records; each becomes a key event or a
                        // text event for its window, dispatched in order.
                        let mut records = [0u8; door_service_protocol::BODY_BYTES];
                        for _ in 0..8 {
                            let n = match client.text_input_poll(&mut records) {
                                Ok(n) => n,
                                Err(Error::Service(_) | Error::Malformed) => 0,
                                Err(error) => return Err(error),
                            };
                            if n == 0 {
                                break;
                            }
                            for record in
                                door_service_protocol::text_input::records(&records[..n]).flatten()
                            {
                                replay.extend(crate::text_entry::route(&record));
                            }
                        }
                    } else if (door_service_protocol::accessibility::EVENT_FIRST
                        ..=door_service_protocol::accessibility::EVENT_LAST)
                        .contains(&e.kind)
                        && e.kind != door_service_protocol::accessibility::ACTION
                        && !windows.iter().any(|s| s.window == e.window)
                    {
                        // Assistive events name other clients' windows (or
                        // none): they belong to the session, not a window.
                        if let Some(s) = windows.first_mut() {
                            s.dirty |= s.app.event(e).map_err(|e| failed(line!(), e))?;
                        }
                    } else if let Some(index) = windows.iter().position(|s| s.window == e.window) {
                        let capacity_available = pending.is_none();
                        let s = &mut windows[index];
                        let was_modal = s.app.modal();
                        match e.kind {
                            w::CONFIGURED => {
                                s.reset_input();
                                let resized = if let Some(p) = s.surface.as_mut() {
                                    if p.width == e.a && p.height == e.b {
                                        Ok(())
                                    } else {
                                        p.reconfigure(&mut client, e.a, e.b)
                                    }
                                } else {
                                    Surface::new(&mut client, e.a, e.b).map(|p| s.surface = Some(p))
                                };
                                match resized {
                                    Ok(()) => (),
                                    Err(Error::Service(Code::Resources))
                                    | Err(Error::Transport(Status::ResourceExhausted)) => {
                                        if let Some(p) = s.surface.as_ref() {
                                            let mut restore = e.rect;
                                            restore.width = p.width;
                                            restore.height = p.height;
                                            let _ = client.configure(s.window, restore);
                                            s.status="Window size exceeds available rendering memory. Documents are retained.".into();
                                        }
                                        continue;
                                    }
                                    Err(e) => return Err(failed(line!(), e)),
                                }
                                s.rect = e.rect;
                                s.epoch = e.value;
                                s.app.configure_fonts(
                                    zoom.logical_len(e.a),
                                    zoom.logical_len(e.b),
                                    layout_fonts,
                                );
                                match client.acknowledge(s.window, s.epoch) {
                                    Ok(()) => s.dirty = true,
                                    Err(Error::Service(Code::Closing)) => s.closing = true,
                                    Err(Error::Service(Code::StaleInstance)) => {
                                        if let Some(p) = s.surface.take() {
                                            p.unregister(&mut client);
                                        }
                                    }
                                    Err(e) => return Err(failed(line!(), e)),
                                }
                            }
                            w::CLOSE => {
                                if let Some(p) = s.surface.as_mut() {
                                    p.close_requested();
                                }
                                if !s.app.closing(&mut storage)? {
                                    s.closing = resume_after_close(&mut client, s.window, s.epoch)
                                        .map_err(|e| failed(line!(), e))?;
                                    s.reset_input();
                                    if let Some(p) = s.surface.as_ref() {
                                        s.app.configure_fonts(
                                            zoom.logical_len(p.width),
                                            zoom.logical_len(p.height),
                                            layout_fonts,
                                        );
                                    }
                                } else {
                                    client.destroy(s.window).map_err(|e| failed(line!(), e))?;
                                    if let Some(p) = s.surface.take() {
                                        p.unregister(&mut client);
                                    }
                                    s.app.retired();
                                    door_rt::diagnostic(format_args!(
                                        "DOOR_P12 WINDOW_RETIRED id={}\n",
                                        s.window
                                    ));
                                    remove = Some(index);
                                }
                            }
                            w::FOCUS => {
                                s.app.focus(e.a != 0);
                                s.reset_input();
                            }
                            // Assistive actions on the runtime-published tree
                            // (a self-publishing app gets them as events).
                            door_service_protocol::accessibility::ACTION
                                if !s.app.publishes_semantics() =>
                            {
                                s.dirty |= s.semantic_action(
                                    &mut client,
                                    e.b,
                                    &mut storage,
                                    layout_fonts,
                                )?;
                            }
                            w::CAPTURE_LOST | w::DISMISS_TRANSIENTS => s.reset_input(),
                            door_service_protocol::text_input::PENDING => {
                                if let Some(text) = text_event.take() {
                                    match s.app.text_input(text, &mut storage, layout_fonts) {
                                        Ok(d) => s.dirty |= d,
                                        Err(e) => {
                                            s.status = format!("Operation failed: {e:?}");
                                            s.dirty = true;
                                        }
                                    }
                                }
                            }
                            w::INPUT if e.detail == 1 && e.b == 1 => {
                                let scan = e.a & 511;
                                let mods = (e.a >> 9) & 127;
                                let ctrl = mods & door_abi::graphics::CONTROL != 0;
                                let shift = mods & door_abi::graphics::SHIFT != 0;
                                if s.key_cancels_pointer() {
                                    let _ = client.release_capture(s.window);
                                    if s.modal_press_cancelled {
                                        door_rt::diagnostic(format_args!(
                                            "DOOR_P12 MODAL_KEY_CAPTURE_CANCELED\n"
                                        ));
                                    }
                                }
                                if s.app.new_window_key(e.a) && !s.app.modal() {
                                    if !capacity_available {
                                        s.status =
                                            "Another window is opening. Try again in a moment."
                                                .into();
                                        s.dirty = true;
                                    } else if let Some(a) = new_model(s, &mut storage) {
                                        spawn = Some((s.window, a, None));
                                    }
                                } else if ctrl
                                    && scan == 17
                                    && !s.app.modal()
                                    && s.app.close_shortcut()
                                {
                                    begin_close(s, &mut client).map_err(|e| failed(line!(), e))?;
                                } else if shift && scan == 68 || scan == 0x15d {
                                    if let Some(p) = s.surface.as_ref() {
                                        s.app.keyboard_context(
                                            zoom.logical_len(p.width),
                                            zoom.logical_len(p.height),
                                        );
                                        s.dirty = true;
                                    }
                                } else {
                                    match s.app.key_fonts(e.a, &mut storage, layout_fonts) {
                                        Ok(d) => s.dirty |= d,
                                        Err(e) => {
                                            s.status = format!("Operation failed: {e:?}");
                                            s.dirty = true;
                                        }
                                    }
                                }
                            }
                            w::INPUT if e.detail == w::INPUT_WHEEL => {
                                let (x, y) = (zoom.logical(e.rect.x), zoom.logical(e.rect.y));
                                s.hover = inside(x, y, s.client_size(zoom)).then_some((x, y));
                                if s.drag.is_none() {
                                    s.dirty |= s.app.wheel(x, y, e.a as i32, layout_fonts);
                                }
                            }
                            w::INPUT if e.detail == 2 => {
                                let (x, y) = (zoom.logical(e.rect.x), zoom.logical(e.rect.y));
                                let size = s.client_size(zoom);
                                s.dirty |= s.move_pointer(x, y, size, layout_fonts);
                                if let Some(d) = s.drag.as_mut() {
                                    d.motion(x, y);
                                } else {
                                    s.dirty |= s.app.pointer_motion(x, y, layout_fonts);
                                }
                            }
                            w::INPUT if e.detail == 3 && e.a == 1 && e.b == 0 => {
                                if let Some(p) = s.surface.as_ref() {
                                    s.app.context(
                                        zoom.logical(e.rect.x),
                                        zoom.logical(e.rect.y),
                                        zoom.logical_len(p.width),
                                        zoom.logical_len(p.height),
                                    );
                                    s.dirty = true;
                                }
                            }
                            w::INPUT if e.detail == 3 && e.a == 0 => {
                                let (x, y) = (zoom.logical(e.rect.x), zoom.logical(e.rect.y));
                                let size = s.client_size(zoom);
                                s.dirty = true;
                                if e.b == 1 {
                                    s.press_pointer(x, y, size);
                                    if !s.app.modal() {
                                        s.drag = s.app.tab_at(x, y).map(|index| Drag {
                                            index,
                                            x,
                                            y,
                                            moved: false,
                                        });
                                    }
                                    if s.drag.is_none() {
                                        s.dirty |= s.app.pointer_down(x, y, layout_fonts);
                                    }
                                    match client.capture(s.window) {
                                        Ok(()) => (),
                                        // A close (Alt+F4) queued behind this press marks the
                                        // window Closing first; the accepted press keeps its
                                        // state so the queued release still clicks before
                                        // the CLOSE event is handled.
                                        Err(Error::Service(Code::Closing)) => s.closing = true,
                                        Err(Error::Service(
                                            Code::StaleInstance | Code::InvalidRequest,
                                        )) => s.reset_input(),
                                        Err(e) => return Err(failed(line!(), e)),
                                    }
                                } else {
                                    let press = s.release_pointer(x, y, size);
                                    if press.is_none() && s.release_was_cancelled() {
                                        door_rt::diagnostic(format_args!(
                                            "DOOR_P12 MODAL_POINTER_RELEASE_IGNORED\n"
                                        ));
                                    }
                                    let drag = s.drag.take();
                                    s.app.pointer_up();
                                    let moved = drag.is_some_and(|mut d| {
                                        d.motion(x, y);
                                        d.moved
                                    });
                                    if let Some(mut d) = drag {
                                        d.motion(x, y);
                                        if d.moved && !s.closing {
                                            match client.tab_target(s.window) {
                                                Ok(target) => {
                                                    tab_drop = Some((index, d, target, x, y))
                                                }
                                                Err(Error::Service(
                                                    Code::Closing
                                                    | Code::StaleInstance
                                                    | Code::InvalidRequest
                                                    | Code::Unsupported,
                                                )) => {
                                                    s.status="Tab move cancelled; the document remains here.".into();
                                                }
                                                Err(e) => return Err(failed(line!(), e)),
                                            }
                                        }
                                    }
                                    let _ = client.release_capture(s.window);
                                    if !moved
                                        && tab_drop.is_none()
                                        && press.is_some()
                                        && press.and_then(|(a, b)| {
                                            s.app.target_fonts(a, b, layout_fonts)
                                        }) == s.app.target_fonts(x, y, layout_fonts)
                                        && let Some(id) = s.app.target_fonts(x, y, layout_fonts)
                                    {
                                        match s.app.click_at(id, x, y, &mut storage, layout_fonts) {
                                            Ok(d) => s.dirty |= d,
                                            Err(e) => s.status = format!("Operation failed: {e:?}"),
                                        }
                                    }
                                }
                            }
                            _ => s.dirty |= s.app.event(e)?,
                        }
                        if was_modal != s.app.modal() {
                            s.reset_input();
                            s.status.clear();
                            let _ = client.release_capture(s.window);
                        }
                    }
                }
                if let Some(i) = remove {
                    windows.remove(i);
                    if let Some(id) = focus_survivor(&windows, None, pending.map(|p| p.target)) {
                        let _ = client.activate(id);
                    }
                    if windows.is_empty() {
                        return Ok(());
                    }
                }
                if let Some((source, drag, target, x, y)) = tab_drop {
                    if pending.is_some() {
                        windows[source].status =
                            "Wait for the new window to finish opening before moving another tab."
                                .into();
                        windows[source].dirty = true;
                    } else if target.0 == windows[source].window {
                        if let Some(to) = windows[source].app.tab_at(target.1, target.2) {
                            windows[source].dirty |=
                                windows[source].app.reorder_tab(drag.index, to);
                        }
                    } else if let Some(dest) = windows
                        .iter()
                        .position(|s| s.window == target.0 && !s.closing && !s.app.modal())
                    {
                        let (a, b) = if source < dest {
                            let (l, r) = windows.split_at_mut(dest);
                            (&mut l[source], &mut r[0])
                        } else {
                            let (l, r) = windows.split_at_mut(source);
                            (&mut r[0], &mut l[dest])
                        };
                        let moved = match a.app.transfer_tab(&mut b.app, drag.index, &mut storage) {
                            Ok(m) => m,
                            Err(e) => {
                                a.status =
                                    format!("Tab move was not completed; document retained: {e:?}");
                                a.dirty = true;
                                false
                            }
                        };
                        if moved {
                            door_rt::diagnostic(format_args!(
                                "DOOR_P12 TAB_MOVED from={} to={}\n",
                                a.window, b.window
                            ));
                            a.dirty = true;
                            b.dirty = true;
                            if let Err(e) = client.activate(b.window) {
                                b.status = format!(
                                    "Tab moved; window activation was not confirmed: {e:?}"
                                );
                                b.dirty = true;
                            }
                            if a.app.empty_after_transfer() {
                                begin_close(a, &mut client).map_err(|e| failed(line!(), e))?;
                            }
                        }
                    } else if drag.detached(
                        x,
                        y,
                        0,
                        zoom.logical_len(windows[source].rect.width) as i32 - 144,
                    ) && let Some(a) = new_model(&mut windows[source], &mut storage)
                    {
                        let pos = (
                            (zoom.logical(windows[source].rect.x) + x - 80)
                                .clamp(0, ow as i32 - 240),
                            (zoom.logical(windows[source].rect.y) + y - 20)
                                .clamp(0, oh as i32 - 168),
                        );
                        let mut next = Instance::new(Model::Owned(a));
                        match next.create(&mut client, ow, oh, zoom, windows.len(), Some(pos)) {
                            Ok(()) => {
                                if let Some(tab) = windows[source].app.tab_identity(drag.index) {
                                    pending = Some(PendingAdmission {
                                        source: windows[source].window,
                                        target: next.window,
                                        tab: Some(tab),
                                        deadline: now.saturating_add(500),
                                    });
                                    windows.push(next);
                                } else {
                                    client
                                        .destroy(next.window)
                                        .map_err(|e| failed(line!(), e))?;
                                    if let Some(p) = next.surface.take() {
                                        p.unregister(&mut client);
                                    }
                                }
                            }
                            Err(e) => {
                                windows[source].status =
                                    format!("New window could not be opened: {e:?}");
                                windows[source].dirty = true;
                            }
                        }
                    }
                }
                for s in &mut windows {
                    let unadmitted = pending.is_some_and(|p| p.target == s.window);
                    if !s.closing && work_due && !unadmitted {
                        if !storage_retired {
                            match s.app.tick(&mut storage, now) {
                                Ok(d) => s.dirty |= d,
                                Err(e) => {
                                    s.status = format!("Operation failed: {e:?}");
                                    s.dirty = true;
                                }
                            }
                        }
                        match s.app.update(&mut storage, &mut client, s.window) {
                            Ok(d) => s.dirty |= d,
                            Err(e) => {
                                s.status = format!("Operation failed: {e:?}");
                                s.dirty = true;
                            }
                        }
                    }
                    // Consume lifecycle requests before dispatching the next
                    // input, even when ordinary app work is being coalesced.
                    if !s.closing && !unadmitted {
                        if s.app.take_new_window() {
                            if pending.is_some() || spawn.is_some() {
                                s.status =
                                    "Another window is opening. Try again in a moment.".into();
                                s.dirty = true;
                            } else if let Some(a) = new_model(s, &mut storage) {
                                spawn = Some((s.window, a, None));
                            }
                        }
                        if s.app.wants_close() {
                            begin_close(s, &mut client).map_err(|e| failed(line!(), e))?;
                        }
                        if s.app.take_close_declined() {
                            match client.close_declined(s.window) {
                                Ok(()) | Err(Error::Service(_)) => (),
                                Err(e) => return Err(failed(line!(), e)),
                            }
                        }
                    }
                    if !s.closing && s.app.title() != s.title {
                        let title = s.app.title();
                        // A close racing this update (shutdown, Alt+F4) is not
                        // a failure; a replaced window takes the title later.
                        match client.set_title(s.window, title) {
                            Ok(()) => s.title = title.into(),
                            Err(Error::Service(Code::Closing)) => s.closing = true,
                            Err(Error::Service(Code::StaleInstance | Code::WouldBlock)) => (),
                            Err(e) => return Err(failed(line!(), e)),
                        }
                    }
                    let desired = match s.hover {
                        Some((x, y)) if !s.app.modal() => s.app.cursor_at(x, y),
                        _ => w::CURSOR_ARROW,
                    };
                    let material = if s.app.acrylic() {
                        w::MATERIAL_ACRYLIC
                    } else if s.app.mica() {
                        w::MATERIAL_MICA
                    } else {
                        w::MATERIAL_DEFAULT
                    };
                    if s.material != Some(material) && !s.closing && client.supports_acrylic() {
                        match client.set_material(s.window, material) {
                            Ok(()) => s.material = Some(material),
                            Err(Error::Service(Code::Closing)) => s.closing = true,
                            // A replaced window is reconfigured; retry later.
                            Err(Error::Service(Code::StaleInstance | Code::WouldBlock)) => (),
                            Err(Error::Service(_)) => s.material = Some(material),
                            Err(e) => return Err(failed(line!(), e)),
                        }
                    }
                    if desired != s.cursor && !s.closing {
                        match client.set_cursor(s.window, desired) {
                            Ok(()) => s.cursor = desired,
                            Err(Error::Service(Code::Closing)) => s.closing = true,
                            Err(Error::Service(Code::StaleInstance | Code::WouldBlock)) => (),
                            Err(e) => return Err(failed(line!(), e)),
                        }
                    }
                    // A window that needs a frame and has not presented one for
                    // three seconds says why, once (diagnosis of frozen windows).
                    if s.dirty && !s.closing {
                        let since = *s.dirty_since.get_or_insert(now);
                        if !s.stall_reported && now.saturating_sub(since) > stall_ticks() {
                            s.stall_reported = true;
                            door_rt::diagnostic(format_args!(
                                "DOOR_P12 PRESENT_STALL window={} epoch={} refusal={} surface={} available={} work_due={} ready={}
",
                                s.window,
                                s.epoch,
                                s.present_refusal,
                                u8::from(s.surface.is_some()),
                                u8::from(s.surface.as_ref().is_some_and(|p| p.available)),
                                u8::from(work_due),
                                u8::from(s.ready),
                            ));
                        }
                    } else {
                        s.dirty_since = None;
                    }
                    if s.dirty
                        && !s.closing
                        && work_due
                        && let Some(p) = s.surface.as_mut()
                        && p.available
                    {
                        let (left, width) = s.app.caption();
                        match if s.app.options().centered {
                            Ok(())
                        } else {
                            client.set_tabbed_caption(
                                s.window,
                                zoom.len(left),
                                zoom.len(left + width) - zoom.len(left),
                            )
                        } {
                            Ok(()) => (),
                            Err(Error::Service(Code::Closing)) => {
                                s.closing = true;
                                continue;
                            }
                            Err(Error::Service(Code::StaleInstance)) => {
                                s.present_refusal = "caption-stale";
                                continue;
                            }
                            Err(e) => return Err(failed(line!(), e)),
                        }
                        p.pixels.fill(0);
                        let mut v =
                            View::new_zoomed(&mut p.pixels, p.width, p.height, &fonts, zoom)
                                .with_images(&images)
                                .with_monospace(mono.as_ref());
                        v.palette.dark = dark;
                        v.materials = !s.app.options().centered || s.app.mica();
                        v.hover = s.hover;
                        v.pressed = s.pressed.filter(|(x, y)| {
                            s.app.target_fonts(*x, *y, layout_fonts)
                                == s.hover
                                    .and_then(|(a, b)| s.app.target_fonts(a, b, layout_fonts))
                        });
                        s.app.draw(&mut v);
                        if !s.status.is_empty() && !s.app.modal() {
                            v.rect(0, v.height as i32 - 25, v.width, 25, [249, 249, 249, 255]);
                            v.text_clip_centered(
                                12,
                                (v.height as i32 - 25, 25),
                                v.width.saturating_sub(24),
                                &s.status,
                                door_graphics_text::TextStyle::Regular12,
                                [32, 32, 32, 255],
                            );
                        }
                        match p.present(&mut client, s.window, s.epoch) {
                            Ok(fresh) => {
                                s.dirty = !fresh;
                                s.dirty_since = None;
                                if !s.ready {
                                    door_rt::diagnostic(format_args!(
                                        "DOOR_P12 WINDOW_COMMITTED id={}\n",
                                        s.window
                                    ));
                                }
                                s.ready = true;
                                // The semantic tree follows what was drawn.
                                s.semantics.stale = true;
                            }
                            Err(Error::Service(Code::Closing)) => {
                                s.closing = true;
                                s.dirty = false;
                            }
                            Err(Error::Service(Code::Resources))
                            | Err(Error::Transport(Status::ResourceExhausted)) => {
                                // A first-frame quota refusal must not
                                // leave an editable invisible window.
                                s.present_refusal = "present-resources";
                                if let Some(admission) =
                                    pending.as_mut().filter(|p| p.target == s.window)
                                {
                                    admission.deadline = now;
                                }
                            }
                            Err(Error::Service(Code::StaleInstance)) => {
                                s.present_refusal = "present-stale";
                            }
                            Err(Error::Service(Code::WouldBlock)) => {
                                s.present_refusal = "present-busy";
                            }
                            Err(e) => {
                                door_rt::diagnostic(format_args!(
                                    "DOOR_P12 PRESENT_REFUSED window={} epoch={} error={e:?}
",
                                    s.window, s.epoch
                                ));
                                return Err(e);
                            }
                        }
                    }
                    s.publish_semantics(&mut client, zoom, layout_fonts, now);
                }
                if let Some(move_) = pending {
                    let source = windows
                        .iter()
                        .position(|s| s.window == move_.source && !s.closing);
                    let target = windows
                        .iter()
                        .position(|s| s.window == move_.target && !s.closing);
                    let index = source.and_then(|i| match move_.tab {
                        Some(tab) => windows[i].app.tab_index(tab),
                        None => Some(0),
                    });
                    let ready = target.is_some_and(|i| windows[i].ready);
                    let expired = now >= move_.deadline;
                    if source.is_none() || target.is_none() || index.is_none() || expired || ready {
                        pending = None;
                        let mut moved =
                            ready && !expired && source.is_some() && move_.tab.is_none();
                        if ready
                            && !expired
                            && let (Some(src), Some(dst), Some(index), Some(tab)) =
                                (source, target, index, move_.tab)
                        {
                            let (a, b) = if src < dst {
                                let (l, r) = windows.split_at_mut(dst);
                                (&mut l[src], &mut r[0])
                            } else {
                                let (l, r) = windows.split_at_mut(src);
                                (&mut r[0], &mut l[dst])
                            };
                            match a.app.transfer_tab(&mut b.app, index, &mut storage) {
                                Ok(true) => {
                                    moved = true;
                                    a.dirty = true;
                                    b.dirty = true;
                                    door_rt::diagnostic(format_args!(
                                        "DOOR_P12 TAB_DETACHED from={} to={} tab={}\n",
                                        a.window, b.window, tab
                                    ));
                                    if a.app.empty_after_transfer() {
                                        begin_close(a, &mut client)
                                            .map_err(|e| failed(line!(), e))?;
                                    }
                                }
                                Ok(false) => (),
                                Err(e) => {
                                    a.status = format!("Tab move was not completed: {e:?}");
                                    a.dirty = true;
                                }
                            }
                        }
                        if !moved {
                            if let Some(i) = source {
                                windows[i].status =
                                    "New window could not open. Existing documents were retained."
                                        .into();
                                windows[i].dirty = true;
                            }
                            if windows.iter().any(|s| s.window == move_.target) {
                                match client.destroy(move_.target) {
                                    Ok(()) | Err(Error::Service(Code::InvalidHandle)) => {
                                        if let Some(mut retired) =
                                            take_admission_target(&mut windows, move_.target)
                                        {
                                            if let Some(p) = retired.surface.take() {
                                                p.unregister(&mut client);
                                            }
                                            retired.app.retired();
                                            if let Some(id) =
                                                focus_survivor(&windows, Some(move_.source), None)
                                            {
                                                let _ = client.activate(id);
                                            }
                                            door_rt::diagnostic(format_args!(
                                                "DOOR_P12 WINDOW_ADMISSION_REFUSED from={} to={}\n",
                                                move_.source, move_.target
                                            ));
                                        }
                                    }
                                    Err(error) => {
                                        pending = Some(PendingAdmission {
                                            deadline: now.saturating_add(100),
                                            ..move_
                                        });
                                        if matches!(
                                            error,
                                            Error::Transport(
                                                Status::PeerClosed
                                                    | Status::Timeout
                                                    | Status::NotFound
                                            )
                                        ) {
                                            return Err(error);
                                        }
                                    }
                                }
                            }
                        }
                        if windows.is_empty() {
                            return Ok(());
                        }
                    }
                }
                if let Some((source, a, pos)) = spawn.take()
                    && pending.is_none()
                    && windows.iter().any(|s| s.window == source && !s.closing)
                {
                    let mut s = Instance::new(Model::Owned(a));
                    s.app.display(output.0, output.1, zoom.percent());
                    match s.create(&mut client, ow, oh, zoom, windows.len(), pos) {
                        Ok(()) => {
                            pending = Some(PendingAdmission {
                                source,
                                target: s.window,
                                tab: None,
                                deadline: now.saturating_add(500),
                            });
                            windows.push(s);
                        }
                        Err(error) => {
                            if let Some(s) = windows.iter_mut().find(|s| s.window == source) {
                                s.status = format!(
                                    "New window could not open; documents retained: {error:?}"
                                );
                                s.dirty = true;
                            }
                        }
                    }
                }
                if !storage_retired
                    && document.is_some()
                    && let Some(s) = windows.iter_mut().find(|s| s.ready && !s.closing)
                {
                    let (payload, launched) = document.take().unwrap();
                    match s.app.open_document(&payload, launched, &mut storage) {
                        Ok(d) => s.dirty |= d,
                        Err(e) => {
                            s.status = format!("The file could not be opened: {e:?}");
                            s.dirty = true;
                        }
                    }
                }
                if polled.is_none() && !windows.iter().any(|s| s.app.busy()) {
                    door_sys::sleep(now + 1)?;
                }
            }
        })();
        match outcome {
            Ok(()) => return Ok(()),
            Err(Error::Transport(Status::PeerClosed | Status::Timeout | Status::NotFound))
            | Err(Error::Service(Code::WouldBlock)) => {
                // An unadmitted blank destination never owns the source document.
                if let Some(move_) = pending.take() {
                    let _ = take_admission_target(&mut windows, move_.target);
                }
                if windows.is_empty() {
                    return Ok(());
                }
                door_rt::diagnostic(format_args!("DOOR_P11 APP RECONNECT\n"));
                for s in &mut windows {
                    s.app.focus(false);
                    s.reset_input();
                    if let Some(p) = s.surface.take() {
                        drop(p);
                    }
                    s.window = 0;
                }
                door_sys::sleep(door_sys::time()? + 10)?;
            }
            Err(e) => return Err(failed(line!(), e)),
        }
    }
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    #[test]
    fn bounded_drain_preserves_editor_and_modal_window_boundaries() {
        // One trace is dispatched identically with serial or batched painting.
        // Lifecycle changes execute on dispatch, never on deferred work.
        fn trace(batched: bool) -> (alloc::vec::Vec<u8>, alloc::vec::Vec<usize>, usize) {
            let mut drain = InputDrain::default();
            let mut bytes = alloc::vec::Vec::new();
            let mut boundaries = alloc::vec::Vec::new();
            let mut modal = false;
            let mut closing = false;
            let mut paints = 0;
            for i in 0..240 {
                match i {
                    4 => {
                        modal = true;
                        boundaries.push(bytes.len());
                    }
                    9 => {
                        modal = false;
                        boundaries.push(bytes.len());
                    }
                    13 => boundaries.push(bytes.len()), // new-window request
                    201 => {
                        closing = true;
                        boundaries.push(bytes.len());
                    }
                    _ if !modal && !closing => bytes.push(b'a' + (i % 26) as u8),
                    _ => (),
                }
                let due = drain.work_due(true, i % 3 != 2);
                if !batched || due {
                    paints += 1;
                }
            }
            assert!(drain.work_due(false, false));
            (bytes, boundaries, paints + 1)
        }
        let serial = trace(false);
        let batched = trace(true);
        assert_eq!((&serial.0, &serial.1), (&batched.0, &batched.1));
        assert_eq!(batched.1, [4, 4, 7, 194]);
        assert!(batched.2 < serial.2 / 10);
    }
    #[test]
    fn continuous_and_filtered_input_cannot_starve_work_or_local_batch_order() {
        let mut drain = InputDrain::default();
        let mut last = 0;
        let mut slices = 0;
        let admission = PendingAdmission {
            source: 100,
            target: 200,
            tab: None,
            deadline: 500,
        };
        for n in 1..=660 {
            let e = w::Event {
                kind: w::INPUT,
                window: 200,
                ..w::Event::default()
            };
            assert!(admission.blocks_input(e));
            if drain.work_due(true, n % 3 != 0) {
                assert_eq!(n - last, 66);
                last = n;
                slices += 1;
            }
        }
        assert_eq!(slices, 10);
        assert!(drain.work_due(false, false));
    }
    struct Editor {
        bytes: alloc::vec::Vec<u8>,
    }
    impl Application for Editor {
        fn title(&self) -> &str {
            "Unsaved"
        }
        fn configure(&mut self, _: u32, _: u32) {}
        fn draw(&self, _: &mut View<'_>) {}
        fn target(&self, _: i32, _: i32) -> Option<u32> {
            None
        }
        fn click(&mut self, _: u32, _: &mut Storage) -> Result<bool, Error> {
            Ok(false)
        }
        fn key(&mut self, _: u32, _: &mut Storage) -> Result<bool, Error> {
            Ok(false)
        }
    }
    #[test]
    fn focus_restoration_excludes_pending_closing_and_uncommitted_models() {
        let mut windows: alloc::vec::Vec<_> = (0..4)
            .map(|_| {
                Instance::new(Model::Owned(Editor {
                    bytes: alloc::vec::Vec::new(),
                }))
            })
            .collect();
        for (i, s) in windows.iter_mut().enumerate() {
            s.window = 100 + i as u64;
            s.ready = true;
        }
        windows[3].ready = false;
        windows[2].closing = true;
        assert_eq!(focus_survivor(&windows, Some(100), Some(101)), Some(100));
        assert_eq!(focus_survivor(&windows, Some(103), None), Some(101));
        assert_eq!(focus_survivor(&windows, None, Some(101)), Some(100));
        windows[0].closing = true;
        assert_eq!(focus_survivor(&windows, None, Some(101)), None);
    }
    #[test]
    fn denied_offline_or_exhausted_admission_preserves_every_sibling_document() {
        let mut windows = alloc::vec![
            Instance::new(Model::Owned(Editor {
                bytes: b"first unsaved edit".to_vec()
            })),
            Instance::new(Model::Owned(Editor {
                bytes: b"second unsaved edit".to_vec()
            }))
        ];
        for error in [
            Error::Service(Code::Denied),
            Error::Service(Code::Resources),
            Error::Transport(Status::PeerClosed),
        ] {
            assert!(finish_admission(&mut windows[0], Err(error)).is_none());
            assert_eq!(windows.len(), 2);
            assert_eq!(windows[0].app.bytes, b"first unsaved edit");
            assert_eq!(windows[1].app.bytes, b"second unsaved edit");
            assert!(!windows[0].status.is_empty());
            assert!(!windows[0].closing && !windows[1].closing);
        }
    }
    #[test]
    fn failed_admission_discards_only_the_factory_target_and_last_removal_exits() {
        let mut editor = Editor {
            bytes: b"unsaved source".to_vec(),
        };
        let mut windows = alloc::vec![
            Instance::new(Model::Root(&mut editor)),
            Instance::new(Model::Owned(Editor {
                bytes: b"fresh template".to_vec()
            })),
        ];
        windows[0].window = 100;
        windows[1].window = 200;
        assert!(take_admission_target(&mut windows, 100).is_none());
        let blank = take_admission_target(&mut windows, 200).unwrap();
        assert_eq!(blank.app.bytes, b"fresh template");
        assert_eq!(windows[0].app.bytes, b"unsaved source");
        windows.push(blank);
        windows.remove(0); // Source explicitly closed while destination was pending.
        assert!(take_admission_target(&mut windows, 200).is_some());
        assert!(
            windows.is_empty(),
            "runtime must return normally after rollback"
        );
        drop(windows);
        assert_eq!(editor.bytes, b"unsaved source");
    }
    #[test]
    fn pending_normal_windows_and_detached_tabs_block_edits_but_not_close_events() {
        for tab in [None, Some(99)] {
            let admission = PendingAdmission {
                source: 100,
                target: 200,
                tab,
                deadline: 500,
            };
            let event = w::Event {
                kind: w::INPUT,
                window: 200,
                ..w::Event::default()
            };
            assert!(admission.blocks_input(event));
            assert!(!admission.blocks_input(w::Event {
                window: 100,
                ..event
            }));
            assert!(!admission.blocks_input(w::Event {
                kind: w::CLOSE,
                ..event
            }));
        }
    }
}
#[cfg(test)]
mod pointer_tests {
    use super::*;
    /// Notes at its 240x178 minimum client: a File button under the area the
    /// Close prompt covers, and the prompt's own buttons while it is shown.
    struct Prompted {
        modal: bool,
        clicks: alloc::vec::Vec<u32>,
        downs: usize,
    }
    const FILE: u32 = 1;
    const CANCEL: u32 = 2;
    impl Application for Prompted {
        fn title(&self) -> &str {
            "Notes"
        }
        fn configure(&mut self, _: u32, _: u32) {}
        fn draw(&self, _: &mut View<'_>) {}
        fn target(&self, x: i32, y: i32) -> Option<u32> {
            if self.modal {
                ((120..200).contains(&x) && (130..162).contains(&y)).then_some(CANCEL)
            } else {
                ((8..64).contains(&x) && (40..72).contains(&y)).then_some(FILE)
            }
        }
        fn click(&mut self, id: u32, _: &mut Storage) -> Result<bool, Error> {
            self.clicks.push(id);
            Ok(true)
        }
        fn key(&mut self, _: u32, _: &mut Storage) -> Result<bool, Error> {
            Ok(false)
        }
        fn modal(&self) -> bool {
            self.modal
        }
        fn pointer_down(&mut self, _: i32, _: i32, _: &FontAtlas) -> bool {
            self.downs += 1;
            false
        }
    }
    /// What the paint pass shows, exactly as `run` derives it for `View`:
    /// the hovered target and the pressed target that is still under it.
    fn visible(s: &Instance<'_, Prompted>) -> (Option<u32>, Option<u32>) {
        let target = |p: Option<(i32, i32)>| p.and_then(|(x, y)| s.app.target(x, y));
        let hover = target(s.hover);
        (hover, target(s.pressed).filter(|t| Some(*t) == hover))
    }
    /// The run loop's modal-transition rule after any event.
    fn after_event(s: &mut Instance<'_, Prompted>, was_modal: bool) {
        if was_modal != s.app.modal() {
            s.reset_input();
        }
    }
    #[test]
    fn modal_dismissed_while_held_leaves_no_hover_or_pressed_plate_underneath() {
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        let size = Some((240, 178));
        let mut s = Instance::new(Model::Owned(Prompted {
            modal: false,
            clicks: alloc::vec::Vec::new(),
            downs: 0,
        }));
        // Pointer parked outside the window: the server delivers nothing.
        let before = visible(&s);
        assert_eq!(before, (None, None));
        // Ctrl+W shows the Close prompt.
        s.app.modal = true;
        after_event(&mut s, false);
        // Motion to client (20, 44): over File, under the dialog's body.
        s.move_pointer(20, 44, size, &fonts);
        assert_eq!(visible(&s), (None, None));
        // Left down on a non-button dialog point; capture keeps the press.
        s.press_pointer(20, 44, size);
        s.app.pointer_down(20, 44, &fonts);
        assert_eq!(s.app.downs, 1);
        // Escape: the held modal press is cancelled, then the prompt closes.
        assert!(s.key_cancels_pointer());
        assert!(
            s.modal_press_cancelled,
            "DOOR_P12 MODAL_KEY_CAPTURE_CANCELED"
        );
        s.app.modal = false;
        after_event(&mut s, true);
        assert_eq!(visible(&s), (None, None));
        // Left up over File: no matching press, so no click and no state.
        let press = s.release_pointer(20, 44, size);
        s.app.pointer_up();
        assert_eq!(press, None);
        assert!(s.app.clicks.is_empty(), "stale press activated File");
        // The cancelled release is reported once (DOOR_P12
        // MODAL_POINTER_RELEASE_IGNORED); a fresh press clears the mark.
        assert!(s.release_was_cancelled());
        assert!(!s.release_was_cancelled());
        s.app.modal = true;
        s.press_pointer(20, 44, size);
        assert!(s.key_cancels_pointer() && s.modal_press_cancelled);
        s.press_pointer(20, 44, size);
        assert!(!s.modal_press_cancelled);
        s.app.modal = false;
        s.reset_input();
        assert_eq!((s.hover, s.pressed), (None, None));
        // The pointer leaves for (0, 0): routed elsewhere, no event here.
        assert_eq!(visible(&s), before, "File keeps a hover/pressed plate");
        // Sensitivity: the same point does show File's plate once it is
        // genuinely hovered, so the assertion above is meaningful.
        assert!(s.move_pointer(20, 44, size, &fonts));
        assert_eq!(visible(&s), (Some(FILE), None));
    }
    #[test]
    fn leaving_the_client_clears_hover_and_unpresses_a_dragged_off_press() {
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        let size = Some((240, 178));
        let mut s = Instance::new(Model::Owned(Prompted {
            modal: false,
            clicks: alloc::vec::Vec::new(),
            downs: 0,
        }));
        assert!(s.move_pointer(20, 44, size, &fonts));
        assert_eq!(visible(&s), (Some(FILE), None));
        // Motion over this window's own frame arrives outside the client.
        assert!(s.move_pointer(-2, 44, size, &fonts));
        assert_eq!((s.hover, visible(&s)), (None, (None, None)));
        // A held press dragged beyond the client (captured motion) shows
        // neither hover nor pressed, and releasing out there clicks nothing.
        s.move_pointer(20, 44, size, &fonts);
        s.press_pointer(20, 44, size);
        assert_eq!(visible(&s), (Some(FILE), Some(FILE)));
        s.move_pointer(300, 44, size, &fonts);
        assert_eq!(visible(&s), (None, None));
        assert_eq!(s.release_pointer(300, 44, size), Some((20, 44)));
        assert_eq!((s.hover, s.pressed), (None, None));
        // A normal click keeps hover where the release happened.
        s.press_pointer(20, 44, size);
        assert_eq!(s.release_pointer(21, 45, size), Some((20, 44)));
        assert_eq!(visible(&s), (Some(FILE), None));
    }
}
