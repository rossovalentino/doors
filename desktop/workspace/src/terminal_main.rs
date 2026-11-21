//! `door-terminal`: Terminal (Workspace manifest, terminal mode).
#![no_std]
#![no_main]
extern crate alloc;
mod native;
// Crate-root names the application modules import with `use super::*`.
#[allow(unused_imports)]
use alloc::{boxed::Box, format, string::String};
#[allow(unused_imports)]
use door_abi::platform::{StartupV2, inherited, mode};
#[allow(unused_imports)]
use door_graphics_text::FontAtlas;
#[allow(unused_imports)]
use door_service_client::{Error, Storage, window::WindowClient};
#[allow(unused_imports)]
use door_service_protocol::{Code, window as w};
use door_workspace::runtime::WindowOptions;
#[allow(unused_imports)]
use door_workspace::{icons, menu, ui};
use native::Hosted;
#[global_allocator]
static HEAP: door_rt::HeapAllocator = door_rt::HeapAllocator::new();
door_rt::entry_v2!(main);
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    door_rt::panic()
}
mod terminal;
fn main(startup: StartupV2) -> i64 {
    native::run::<terminal::Terminal>(&HEAP, startup)
}
impl Hosted for terminal::Terminal {
    fn make(startup: &StartupV2, storage: &mut Storage, _fresh: bool) -> Result<Self, Error> {
        Ok(terminal::Terminal::new(
            format!("/users/{}", startup.identity.uid),
            startup,
            storage,
        ))
    }
    fn title(&self) -> &str {
        self.window_title()
    }
    fn options(&self) -> WindowOptions {
        WindowOptions {
            width: 1128,
            height: 632,
            caption_height: door_workspace::caption_tabs::HEIGHT,
            caption_left: door_workspace::caption_tabs::LEFT as u32,
            centered: false,
        }
    }
    fn caption(&self) -> (u32, u32) {
        self.caption_region()
    }
    fn configure(&mut self, width: u32, height: u32, _fonts: &FontAtlas) {
        terminal::Terminal::configure(self, width, height)
    }
    fn draw(&self, view: &mut ui::View<'_>) {
        terminal::Terminal::draw(self, view)
    }
    fn semantics(&self, s: &mut door_workspace::semantics::Semantics, _fonts: &FontAtlas) {
        terminal::Terminal::semantics(self, s)
    }
    fn target(&self, x: i32, y: i32, _fonts: &FontAtlas) -> Option<u32> {
        terminal::Terminal::target(self, x, y)
    }
    fn click(
        &mut self,
        x: i32,
        y: i32,
        _size: (u32, u32),
        storage: &mut Storage,
        _fonts: &FontAtlas,
    ) -> Result<bool, Error> {
        terminal::Terminal::click(self, x, y, storage)
    }
    fn key(&mut self, code: u32, storage: &mut Storage, _fonts: &FontAtlas) -> Result<bool, Error> {
        terminal::Terminal::key(self, code, storage)
    }
    fn menu_open(&self) -> bool {
        terminal::Terminal::menu_open(self)
    }
    fn text_hit(&self, x: i32, y: i32) -> bool {
        terminal::Terminal::text_hit(self, x, y)
    }
    fn pointer_down(&mut self, x: i32, y: i32, _fonts: &FontAtlas) -> bool {
        terminal::Terminal::pointer_down(self, x, y)
    }
    fn pointer_motion(&mut self, x: i32, y: i32, _fonts: &FontAtlas) -> bool {
        terminal::Terminal::pointer_motion(self, x, y)
    }
    fn pointer_up(&mut self) {
        terminal::Terminal::pointer_up(self)
    }
    /// Right click: copy the selection, or else paste (Windows Terminal).
    fn context(&mut self, x: i32, y: i32, _width: u32, _height: u32) {
        terminal::Terminal::context(self, x, y);
    }
    /// Shift+F10 / the Menu key open no menu (and must not paste).
    fn keyboard_context(&mut self, _width: u32, _height: u32) {}
    fn wheel(&mut self, x: i32, y: i32, notches: i32, _fonts: &FontAtlas) -> bool {
        terminal::Terminal::wheel(self, x, y, notches)
    }
    fn acrylic(&self) -> bool {
        terminal::Terminal::acrylic(self)
    }
    /// Explorer's Open on a .bat/.cmd file runs it in a CMD tab; a folder
    /// opens a Doorsh tab there.
    fn open_document(
        &mut self,
        payload: &[u8],
        _launched: bool,
        storage: &mut Storage,
    ) -> Result<bool, Error> {
        let Ok(path) = core::str::from_utf8(payload) else {
            return Ok(false);
        };
        terminal::Terminal::open_path(self, path, storage);
        Ok(true)
    }
    fn tick(&mut self, storage: &mut Storage, now: u64) -> bool {
        let output = self.poll_program(now, storage);
        let engines = self.poll_engines(storage, now);
        self.track_recent(storage);
        self.autoscroll(now) || output || engines
    }
    fn close_shortcut(&self) -> bool {
        false
    }
    /// Tabs drag along the strip, to another Terminal window or out into a
    /// new one, as Windows Terminal's do.
    fn tab_at(&self, x: i32, y: i32) -> Option<usize> {
        self.dragged_tab(x, y)
    }
    fn tab_identity(&self, index: usize) -> Option<u64> {
        terminal::Terminal::tab_identity(self, index)
    }
    fn tab_index(&self, id: u64) -> Option<usize> {
        self.tab_position(id)
    }
    fn transfer_tab(
        &mut self,
        target: &mut Self,
        index: usize,
        target_pristine: bool,
        _storage: &mut Storage,
    ) -> Result<bool, Error> {
        Ok(self.transfer_tab_to(target, index, target_pristine))
    }
    fn reorder_tab(&mut self, from: usize, to: usize) -> bool {
        self.move_tab(from, to)
    }
    fn empty_after_transfer(&self) -> bool {
        self.emptied()
    }
    /// The window closes: the programs its tabs run end with it, as
    /// closing a Windows Terminal window ends its tabs' programs.
    fn closing(&mut self, _storage: &mut Storage) -> Result<bool, Error> {
        self.end_programs();
        Ok(true)
    }
    fn wants_close(&mut self) -> bool {
        terminal::Terminal::take_close(self)
    }
    fn capabilities(&self) -> u32 {
        door_service_protocol::clipboard::CAP_CLIPBOARD
    }
    fn focus(&mut self, focused: bool) {
        terminal::Terminal::focus(self, focused)
    }
    fn window_update(
        &mut self,
        _storage: &mut Storage,
        client: &mut WindowClient,
        window: u64,
    ) -> Result<bool, Error> {
        terminal::Terminal::window_update(self, client, window)
    }
    fn event(&mut self, e: w::Event) -> Result<bool, Error> {
        // Key releases: Shift/Alt state for Shift+click and Alt+drag.
        if e.kind == w::INPUT && e.detail == 1 && e.b == 0 {
            self.modifier(e.a & 511, false);
            return Ok(false);
        }
        if e.kind != w::CONTROLS_CHANGED {
            return Ok(false);
        }
        let c = door_service_protocol::desktop::Controls::from_bits(e.a).ok_or(Error::Malformed)?;
        self.controls_changed(c);
        Ok(true)
    }
    fn monospace(&self) -> bool {
        true
    }
}
