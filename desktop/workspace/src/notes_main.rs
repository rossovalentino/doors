//! `door-notes`: Notes (Workspace manifest, session mode).
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
mod notes;
fn main(startup: StartupV2) -> i64 {
    native::run::<notes::Notes>(&HEAP, startup)
}
impl Hosted for notes::Notes {
    fn make(startup: &StartupV2, storage: &mut Storage, fresh: bool) -> Result<Self, Error> {
        let home = format!("/users/{}", startup.identity.uid);
        if fresh {
            let path = door_workspace::document_tabs::untitled_path(
                &format!("{home}/notes"),
                door_sys::time()?,
            )
            .ok_or(Error::Service(Code::Resources))?;
            Ok(notes::Notes::empty(path))
        } else {
            notes::Notes::new(format!("{home}/notes/desktop.txt"), storage)
        }
    }
    fn title(&self) -> &str {
        self.file_name()
    }
    fn options(&self) -> WindowOptions {
        WindowOptions {
            width: 900,
            height: 640,
            caption_height: door_workspace::document_format::CAPTION,
            caption_left: door_workspace::document_format::TAB_LEFT as u32,
            centered: false,
        }
    }
    fn caption(&self) -> (u32, u32) {
        (
            door_workspace::document_format::TAB_LEFT as u32,
            self.caption_width(),
        )
    }
    fn configure(&mut self, width: u32, height: u32, fonts: &FontAtlas) {
        notes::Notes::configure(self, width, height, fonts)
    }
    fn draw(&self, view: &mut ui::View<'_>) {
        notes::Notes::draw(self, view)
    }
    fn semantics(&self, s: &mut door_workspace::semantics::Semantics, _fonts: &FontAtlas) {
        notes::Notes::semantics(self, s)
    }
    fn target(&self, x: i32, y: i32, _fonts: &FontAtlas) -> Option<u32> {
        notes::Notes::target(self, x, y)
    }
    fn click(
        &mut self,
        x: i32,
        y: i32,
        size: (u32, u32),
        storage: &mut Storage,
        fonts: &FontAtlas,
    ) -> Result<bool, Error> {
        notes::Notes::click(self, x, y, size.0, size.1, storage, fonts)
    }
    fn key(&mut self, code: u32, storage: &mut Storage, fonts: &FontAtlas) -> Result<bool, Error> {
        notes::Notes::key(self, code, storage, fonts)
    }
    fn modal(&self) -> bool {
        notes::Notes::modal(self)
    }
    fn menu_open(&self) -> bool {
        notes::Notes::menu_open(self)
    }
    fn dismiss_menu(&mut self) {
        notes::Notes::dismiss_menu(self)
    }
    fn hover_menu(&mut self, x: i32, y: i32) -> bool {
        notes::Notes::hover_menu(self, x, y)
    }
    fn text_hit(&self, x: i32, y: i32) -> bool {
        notes::Notes::text_hit(self, x, y)
    }
    fn pointer_down(&mut self, x: i32, y: i32, fonts: &FontAtlas) -> bool {
        notes::Notes::pointer_down(self, x, y, fonts)
    }
    fn pointer_motion(&mut self, x: i32, y: i32, fonts: &FontAtlas) -> bool {
        notes::Notes::pointer_motion(self, x, y, fonts)
    }
    fn pointer_up(&mut self) {
        notes::Notes::pointer_up(self)
    }
    fn wheel(&mut self, x: i32, y: i32, notches: i32, fonts: &FontAtlas) -> bool {
        notes::Notes::wheel(self, x, y, notches, fonts)
    }
    fn context(&mut self, x: i32, y: i32, width: u32, height: u32) {
        notes::Notes::context(self, x, y, width, height)
    }
    fn recreated(&mut self) {
        self.window_recreated()
    }
    fn close_shortcut(&self) -> bool {
        self.tab_count() <= 1
    }
    fn closing(&mut self, _storage: &mut Storage) -> Result<bool, Error> {
        Ok(self.close_requested())
    }
    fn take_new_window(&mut self) -> bool {
        notes::Notes::take_new_window(self)
    }
    fn close_failed(&mut self) {
        notes::Notes::close_failed(self)
    }
    fn take_close_declined(&mut self) -> bool {
        notes::Notes::take_close_declined(self)
    }
    fn wants_close(&mut self) -> bool {
        self.take_close()
    }
    fn open_document(
        &mut self,
        payload: &[u8],
        _launched: bool,
        storage: &mut Storage,
    ) -> Result<bool, Error> {
        let path = core::str::from_utf8(payload).map_err(|_| Error::Malformed)?;
        notes::Notes::open_document(self, path.into(), storage)?;
        Ok(true)
    }
    fn capabilities(&self) -> u32 {
        door_service_protocol::text_input::CAP_TEXT_INPUT
            | door_service_protocol::clipboard::CAP_CLIPBOARD
    }
    fn focus(&mut self, focused: bool) {
        notes::Notes::focus(self, focused)
    }
    fn text_input(
        &mut self,
        event: door_workspace::text_entry::TextEvent,
        _storage: &mut Storage,
        fonts: &FontAtlas,
    ) -> Result<bool, Error> {
        Ok(notes::Notes::text_input(self, event, fonts))
    }
    fn window_update(
        &mut self,
        _storage: &mut Storage,
        client: &mut WindowClient,
        window: u64,
    ) -> Result<bool, Error> {
        notes::Notes::window_update(self, client, window)
    }
    fn monospace_layout(&self) -> bool {
        true
    }
    fn monospace(&self) -> bool {
        true
    }
}
