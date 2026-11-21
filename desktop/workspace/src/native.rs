//! One desktop application per executable (`door-files`, `door-notes`,
//! `door-settings`, `door-terminal`). Each binary root implements `Hosted`
//! for its single model; `Native` adapts it to the shared runtime, so an
//! executable links only its own application and the shared library.
//! Identity is unchanged: every binary still runs under the Workspace
//! manifest with the launch mode Init chose, so the shell, Task Manager and
//! shutdown accounting see the same applications as before the split.
use alloc::{boxed::Box, string::String};
use door_abi::platform::{StartupV2, inherited};
use door_graphics_text::FontAtlas;
use door_service_client::{Error, Storage, window::WindowClient};
use door_service_protocol::window as w;
use door_workspace::runtime::WindowOptions;
use door_workspace::ui;

/// The per-application surface `Native` dispatches to. Defaults describe an
/// application without that capability.
#[allow(unused_variables)]
pub trait Hosted: Sized {
    /// `fresh`: a new window/tab rather than the startup document.
    fn make(startup: &StartupV2, storage: &mut Storage, fresh: bool) -> Result<Self, Error>;
    fn title(&self) -> &str;
    fn options(&self) -> WindowOptions;
    /// Client-owned caption region `(left, width)`.
    fn caption(&self) -> (u32, u32);
    fn configure(&mut self, width: u32, height: u32, fonts: &FontAtlas);
    fn draw(&self, view: &mut ui::View<'_>);
    fn target(&self, x: i32, y: i32, fonts: &FontAtlas) -> Option<u32>;
    fn click(
        &mut self,
        x: i32,
        y: i32,
        size: (u32, u32),
        storage: &mut Storage,
        fonts: &FontAtlas,
    ) -> Result<bool, Error>;
    fn key(&mut self, code: u32, storage: &mut Storage, fonts: &FontAtlas) -> Result<bool, Error>;
    fn modal(&self) -> bool {
        false
    }
    fn menu_open(&self) -> bool {
        false
    }
    fn dismiss_menu(&mut self) {}
    fn hover_menu(&mut self, x: i32, y: i32) -> bool {
        false
    }
    fn text_hit(&self, x: i32, y: i32) -> bool {
        false
    }
    /// `runtime::App::cursor_at`.
    fn cursor_at(&self, x: i32, y: i32) -> u32 {
        if self.text_hit(x, y) {
            door_service_protocol::window::CURSOR_TEXT
        } else {
            door_service_protocol::window::CURSOR_ARROW
        }
    }
    fn pointer_down(&mut self, x: i32, y: i32, fonts: &FontAtlas) -> bool {
        false
    }
    fn pointer_motion(&mut self, x: i32, y: i32, fonts: &FontAtlas) -> bool {
        false
    }
    fn pointer_up(&mut self) {}
    fn wheel(&mut self, x: i32, y: i32, notches: i32, fonts: &FontAtlas) -> bool {
        false
    }
    fn acrylic(&self) -> bool {
        false
    }
    fn mica(&self) -> bool {
        false
    }
    fn observes_settings(&self) -> bool {
        true
    }
    fn context(&mut self, x: i32, y: i32, width: u32, height: u32) {}
    fn keyboard_context(&mut self, width: u32, height: u32) {
        self.context(140, 112, width, height)
    }
    fn tab_at(&self, x: i32, y: i32) -> Option<usize> {
        None
    }
    fn tab_identity(&self, index: usize) -> Option<u64> {
        None
    }
    fn tab_index(&self, id: u64) -> Option<usize> {
        None
    }
    /// Same-application tab tear-off; `target_pristine` is the destination
    /// window's untouched startup state.
    fn transfer_tab(
        &mut self,
        target: &mut Self,
        index: usize,
        target_pristine: bool,
        storage: &mut Storage,
    ) -> Result<bool, Error> {
        Ok(false)
    }
    fn reorder_tab(&mut self, from: usize, to: usize) -> bool {
        false
    }
    fn empty_after_transfer(&self) -> bool {
        false
    }
    /// Ctrl+Shift+N opens a new window unless an application says otherwise.
    fn new_window_key(&self, key: u32) -> bool {
        let m = key >> 9;
        key & 511 == 49
            && m & door_abi::graphics::CONTROL != 0
            && m & door_abi::graphics::SHIFT != 0
    }
    /// Prepare a freshly made window (e.g. Explorer "Open in new window").
    fn prepare_window(&self, window: &mut Self, storage: &mut Storage) -> Result<(), Error> {
        Ok(())
    }
    fn recreated(&mut self) {}
    fn close_shortcut(&self) -> bool {
        true
    }
    fn closing(&mut self, storage: &mut Storage) -> Result<bool, Error> {
        Ok(true)
    }
    fn take_new_window(&mut self) -> bool {
        false
    }
    fn close_failed(&mut self) {}
    fn take_close_declined(&mut self) -> bool {
        false
    }
    fn wants_close(&mut self) -> bool {
        false
    }
    fn tick(&mut self, storage: &mut Storage, now: u64) -> bool {
        false
    }
    fn display(&mut self, width: u32, height: u32, scale: u32) {}
    fn open_document(
        &mut self,
        payload: &[u8],
        launched: bool,
        storage: &mut Storage,
    ) -> Result<bool, Error> {
        Ok(false)
    }
    fn disconnected(&mut self) {}
    fn event(&mut self, event: w::Event) -> Result<bool, Error> {
        Ok(false)
    }
    fn update(&mut self, storage: &mut Storage, client: &mut WindowClient) -> Result<bool, Error> {
        Ok(false)
    }
    /// `update` with this window's identity (text-input and clipboard calls).
    fn window_update(
        &mut self,
        storage: &mut Storage,
        client: &mut WindowClient,
        window: u64,
    ) -> Result<bool, Error> {
        self.update(storage, client)
    }
    fn focus(&mut self, focused: bool) {}
    /// Extra HELLO capabilities (`runtime::Application::capabilities`).
    fn capabilities(&self) -> u32 {
        0
    }
    fn text_input(
        &mut self,
        event: door_workspace::text_entry::TextEvent,
        storage: &mut Storage,
        fonts: &FontAtlas,
    ) -> Result<bool, Error> {
        Ok(false)
    }
    fn monospace_layout(&self) -> bool {
        false
    }
    fn monospace(&self) -> bool {
        false
    }
    /// door.accessibility: describe the controls from the layout `target`
    /// hit-tests (`door_workspace::semantics`).
    fn semantics(&self, s: &mut door_workspace::semantics::Semantics, fonts: &FontAtlas) {}
}

pub struct Native<A: Hosted> {
    app: Box<A>,
    startup: StartupV2,
    title: String,
    size: (u32, u32),
    pristine: bool,
}
impl<A: Hosted> Native<A> {
    fn make(startup: StartupV2, storage: &mut Storage, fresh: bool) -> Result<Self, Error> {
        let app = Box::new(A::make(&startup, storage, fresh)?);
        let title = Self::title_of(&app);
        Ok(Self {
            app,
            startup,
            title,
            size: (720, 480),
            pristine: fresh,
        })
    }
    fn title_of(app: &A) -> String {
        ui::display_ascii(app.title()).chars().take(128).collect()
    }
    fn refresh(&mut self, changed: bool) -> bool {
        if changed {
            self.title = Self::title_of(&self.app);
        }
        changed
    }
}
impl<A: Hosted> door_workspace::runtime::Application for Native<A> {
    fn title(&self) -> &str {
        &self.title
    }
    fn options(&self) -> WindowOptions {
        self.app.options()
    }
    fn caption(&self) -> (u32, u32) {
        self.app.caption()
    }
    fn tab_at(&self, x: i32, y: i32) -> Option<usize> {
        self.app.tab_at(x, y)
    }
    fn tab_identity(&self, index: usize) -> Option<u64> {
        self.app.tab_identity(index)
    }
    fn tab_index(&self, id: u64) -> Option<usize> {
        self.app.tab_index(id)
    }
    fn transfer_tab(
        &mut self,
        target: &mut Self,
        index: usize,
        s: &mut Storage,
    ) -> Result<bool, Error> {
        let moved = self
            .app
            .transfer_tab(&mut target.app, index, target.pristine, s)?;
        if moved {
            target.pristine = false;
            self.title = Self::title_of(&self.app);
            target.title = Self::title_of(&target.app);
        }
        Ok(moved)
    }
    fn reorder_tab(&mut self, from: usize, to: usize) -> bool {
        self.app.reorder_tab(from, to)
    }
    fn empty_after_transfer(&self) -> bool {
        self.app.empty_after_transfer()
    }
    fn new_window_key(&self, key: u32) -> bool {
        self.app.new_window_key(key)
    }
    fn keyboard_context(&mut self, w: u32, h: u32) {
        self.app.keyboard_context(w, h)
    }
    fn new_window(&self, s: &mut Storage) -> Result<Option<Self>, Error> {
        let mut window = Self::make(self.startup, s, true)?;
        self.app.prepare_window(&mut window.app, s)?;
        window.title = Self::title_of(&window.app);
        Ok(Some(window))
    }
    fn configure(&mut self, w: u32, h: u32) {
        self.size = (w, h)
    }
    fn configure_fonts(&mut self, w: u32, h: u32, f: &FontAtlas) {
        self.size = (w, h);
        self.app.configure(w, h, f)
    }
    fn draw(&self, v: &mut ui::View<'_>) {
        self.app.draw(v)
    }
    fn target(&self, _x: i32, _y: i32) -> Option<u32> {
        None
    }
    fn target_fonts(&self, x: i32, y: i32, f: &FontAtlas) -> Option<u32> {
        self.app.target(x, y, f)
    }
    fn click(&mut self, _id: u32, _s: &mut Storage) -> Result<bool, Error> {
        Ok(false)
    }
    fn click_at(
        &mut self,
        _id: u32,
        x: i32,
        y: i32,
        s: &mut Storage,
        f: &FontAtlas,
    ) -> Result<bool, Error> {
        self.pristine = false;
        let d = self.app.click(x, y, self.size, s, f)?;
        Ok(self.refresh(d))
    }
    fn key(&mut self, _key: u32, _s: &mut Storage) -> Result<bool, Error> {
        Ok(false)
    }
    fn key_fonts(&mut self, key: u32, s: &mut Storage, f: &FontAtlas) -> Result<bool, Error> {
        self.pristine = false;
        let d = self.app.key(key, s, f)?;
        Ok(self.refresh(d))
    }
    fn menu_open(&self) -> bool {
        self.app.menu_open()
    }
    fn modal(&self) -> bool {
        self.app.modal()
    }
    fn dismiss_menu(&mut self) {
        self.app.dismiss_menu()
    }
    fn text_hit(&self, x: i32, y: i32) -> bool {
        self.app.text_hit(x, y)
    }
    fn cursor_at(&self, x: i32, y: i32) -> u32 {
        self.app.cursor_at(x, y)
    }
    fn pointer_down(&mut self, x: i32, y: i32, f: &FontAtlas) -> bool {
        self.app.pointer_down(x, y, f)
    }
    fn pointer_motion(&mut self, x: i32, y: i32, f: &FontAtlas) -> bool {
        let d = self.app.pointer_motion(x, y, f);
        self.app.hover_menu(x, y) | d
    }
    fn pointer_up(&mut self) {
        self.app.pointer_up()
    }
    fn wheel(&mut self, x: i32, y: i32, notches: i32, f: &FontAtlas) -> bool {
        self.app.wheel(x, y, notches, f)
    }
    fn acrylic(&self) -> bool {
        self.app.acrylic()
    }
    fn mica(&self) -> bool {
        self.app.mica()
    }
    fn observes_settings(&self) -> bool {
        self.app.observes_settings()
    }
    fn context(&mut self, x: i32, y: i32, w: u32, h: u32) {
        self.app.context(x, y, w, h)
    }
    fn recreated(&mut self) {
        self.app.recreated()
    }
    fn close_shortcut(&self) -> bool {
        self.app.close_shortcut()
    }
    fn closing(&mut self, s: &mut Storage) -> Result<bool, Error> {
        self.app.closing(s)
    }
    fn take_new_window(&mut self) -> bool {
        self.app.take_new_window()
    }
    fn close_failed(&mut self) {
        self.app.close_failed()
    }
    fn take_close_declined(&mut self) -> bool {
        self.app.take_close_declined()
    }
    fn wants_close(&mut self) -> bool {
        self.app.wants_close()
    }
    fn tick(&mut self, s: &mut Storage, now: u64) -> Result<bool, Error> {
        let d = self.app.tick(s, now);
        Ok(self.refresh(d))
    }
    fn display(&mut self, width: u32, height: u32, scale: u32) {
        self.app.display(width, height, scale)
    }
    fn open_document(
        &mut self,
        payload: &[u8],
        launched: bool,
        s: &mut Storage,
    ) -> Result<bool, Error> {
        let d = self.app.open_document(payload, launched, s)?;
        Ok(self.refresh(d))
    }
    fn disconnected(&mut self) {
        self.app.disconnected()
    }
    fn event(&mut self, e: w::Event) -> Result<bool, Error> {
        self.app.event(e)
    }
    fn update(
        &mut self,
        s: &mut Storage,
        c: &mut WindowClient,
        window: u64,
    ) -> Result<bool, Error> {
        let d = self.app.window_update(s, c, window)?;
        Ok(self.refresh(d))
    }
    fn focus(&mut self, focused: bool) {
        self.app.focus(focused)
    }
    fn capabilities(&self) -> u32 {
        self.app.capabilities()
    }
    fn text_input(
        &mut self,
        event: door_workspace::text_entry::TextEvent,
        s: &mut Storage,
        f: &FontAtlas,
    ) -> Result<bool, Error> {
        self.pristine = false;
        let d = self.app.text_input(event, s, f)?;
        Ok(self.refresh(d))
    }
    fn monospace_layout(&self) -> bool {
        self.app.monospace_layout()
    }
    fn monospace(&self) -> bool {
        self.app.monospace()
    }
    fn semantics(&self, s: &mut door_workspace::semantics::Semantics, f: &FontAtlas) {
        self.app.semantics(s, f)
    }
}
/// Shared executable entry: heap, startup document, runtime event loop.
pub fn run<A: Hosted>(heap: &door_rt::HeapAllocator, startup: StartupV2) -> i64 {
    if let Err(error) = heap.initialize_with_bytes(96 * 1024 * 1024) {
        door_rt::diagnostic(format_args!("DOOR_P12 HEAP FAILED {error:?}\n"));
        return -240;
    }
    // Which startup step failed, for the serial record.
    let mut stage = "storage";
    let result = (|| {
        let registry = startup.base.inherited[inherited::REGISTRY].handle;
        let mut s = Storage::connect(registry)?;
        stage = "make";
        let mut app = Native::<A>::make(startup, &mut s, false)?;
        drop(s);
        stage = "run";
        door_workspace::runtime::run(&startup, &mut app)
    })();
    match result {
        Ok(()) => 0,
        Err(e) => {
            door_rt::diagnostic(format_args!("DOOR_P11 APP FAILED {e:?} stage={stage}\n"));
            -241
        }
    }
}
