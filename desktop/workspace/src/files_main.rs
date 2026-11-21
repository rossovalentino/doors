//! `door-files`: File Explorer (Workspace manifest, desktop mode).
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
mod files;
fn main(startup: StartupV2) -> i64 {
    native::run::<files::Files>(&HEAP, startup)
}
impl Hosted for files::Files {
    fn make(startup: &StartupV2, storage: &mut Storage, _fresh: bool) -> Result<Self, Error> {
        files::Files::new(
            format!("/users/{}", startup.identity.uid),
            startup.base.inherited[inherited::REGISTRY].handle,
            startup.base.inherited[inherited::DIRECTORY].handle,
            storage,
        )
    }
    fn title(&self) -> &str {
        files::Files::title(self)
    }
    fn options(&self) -> WindowOptions {
        WindowOptions {
            width: 1000,
            height: 640,
            caption_height: 40,
            caption_left: 8,
            centered: false,
        }
    }
    fn caption(&self) -> (u32, u32) {
        (8, self.caption_width())
    }
    fn configure(&mut self, width: u32, height: u32, _fonts: &FontAtlas) {
        files::Files::configure(self, width, height)
    }
    fn draw(&self, view: &mut ui::View<'_>) {
        files::Files::draw(self, view)
    }
    fn semantics(&self, s: &mut door_workspace::semantics::Semantics, fonts: &FontAtlas) {
        files::Files::semantics(self, s, fonts)
    }
    fn target(&self, x: i32, y: i32, fonts: &FontAtlas) -> Option<u32> {
        files::Files::target(self, x, y, fonts)
    }
    fn click(
        &mut self,
        x: i32,
        y: i32,
        _size: (u32, u32),
        storage: &mut Storage,
        fonts: &FontAtlas,
    ) -> Result<bool, Error> {
        files::Files::click(self, x, y, storage, fonts)
    }
    fn key(&mut self, code: u32, storage: &mut Storage, _fonts: &FontAtlas) -> Result<bool, Error> {
        files::Files::key(self, code, storage)
    }
    fn modal(&self) -> bool {
        files::Files::modal(self)
    }
    fn menu_open(&self) -> bool {
        files::Files::menu_open(self)
    }
    fn dismiss_menu(&mut self) {
        files::Files::dismiss_menu(self)
    }
    fn hover_menu(&mut self, x: i32, y: i32) -> bool {
        files::Files::hover_menu(self, x, y)
    }
    fn pointer_down(&mut self, x: i32, y: i32, _fonts: &FontAtlas) -> bool {
        let handled = files::Files::pointer_down(self, x, y);
        if !handled {
            // A press on an item may become a drag (door.dnd).
            self.drag_press(x, y);
        }
        handled
    }
    fn pointer_motion(&mut self, x: i32, y: i32, _fonts: &FontAtlas) -> bool {
        self.drag_motion(x, y) | files::Files::pointer_motion(self, x, y)
    }
    fn cursor_at(&self, x: i32, y: i32) -> u32 {
        if self.splitter_cursor(x, y) {
            door_service_protocol::window::CURSOR_SIZE_WE
        } else {
            door_service_protocol::window::CURSOR_ARROW
        }
    }
    fn pointer_up(&mut self) {
        self.drag_release();
        files::Files::pointer_up(self)
    }
    fn display(&mut self, _width: u32, _height: u32, scale: u32) {
        self.dnd_display(scale);
    }
    fn recreated(&mut self) {
        self.dnd_recreated();
    }
    /// Drag and drop (door.dnd): Explorer drags items and takes drops.
    fn capabilities(&self) -> u32 {
        door_service_protocol::dnd::CAP_DRAG_DROP | door_service_protocol::clipboard::CAP_CLIPBOARD
    }
    fn wheel(&mut self, x: i32, y: i32, notches: i32, _fonts: &FontAtlas) -> bool {
        files::Files::wheel(self, x, y, notches)
    }
    fn context(&mut self, x: i32, y: i32, width: u32, height: u32) {
        files::Files::context(self, x, y, width, height)
    }
    fn keyboard_context(&mut self, width: u32, height: u32) {
        files::Files::keyboard_context(self, width, height)
    }
    fn tab_at(&self, x: i32, y: i32) -> Option<usize> {
        self.dragged_tab(x, y)
    }
    fn tab_identity(&self, index: usize) -> Option<u64> {
        files::Files::tab_identity(self, index)
    }
    fn tab_index(&self, id: u64) -> Option<usize> {
        files::Files::tab_index(self, id)
    }
    fn transfer_tab(
        &mut self,
        target: &mut Self,
        index: usize,
        target_pristine: bool,
        storage: &mut Storage,
    ) -> Result<bool, Error> {
        self.transfer_to(target, index, target_pristine, storage)
    }
    fn reorder_tab(&mut self, from: usize, to: usize) -> bool {
        self.reorder(from, to)
    }
    fn empty_after_transfer(&self) -> bool {
        self.window_empty()
    }
    /// Explorer opens a new window with Ctrl+N (Notepad keeps Ctrl+Shift+N).
    fn new_window_key(&self, key: u32) -> bool {
        let m = key >> 9;
        key & 511 == 49
            && m & door_abi::graphics::CONTROL != 0
            && m & door_abi::graphics::SHIFT == 0
    }
    fn prepare_window(&self, window: &mut Self, storage: &mut Storage) -> Result<(), Error> {
        // "Open in new window": the new window opens that location.
        if let Some(open) = self.take_window_request() {
            window.open_request(storage, &open)?;
        }
        Ok(())
    }
    fn close_shortcut(&self) -> bool {
        self.tab_count() <= 1
    }
    fn closing(&mut self, storage: &mut Storage) -> Result<bool, Error> {
        if self.has_pending_request() {
            // `update` sends it; the close follows through `wants_close`.
            self.pending_close = true;
            return Ok(false);
        }
        self.release_previews(storage);
        self.release_watch(storage);
        Ok(true)
    }
    fn take_new_window(&mut self) -> bool {
        self.window_requested()
    }
    fn wants_close(&mut self) -> bool {
        core::mem::take(&mut self.pending_close)
    }
    fn tick(&mut self, storage: &mut Storage, now: u64) -> bool {
        self.poll_preview(storage) | self.operations_tick(storage, now)
    }
    // Start search results (shell_search host -> shell -> DOCUMENT_OPEN).
    fn open_document(
        &mut self,
        payload: &[u8],
        launched: bool,
        storage: &mut Storage,
    ) -> Result<bool, Error> {
        let open = door_workspace::explorer_search::Open::decode(payload)
            .ok_or(Error::Service(Code::InvalidRequest))?;
        if launched {
            // Started for this request: the first window shows it.
            self.open_request(storage, &open)?;
        } else {
            self.request_window(open);
        }
        Ok(true)
    }
    fn disconnected(&mut self) {
        self.storage_retired()
    }
    fn event(&mut self, e: w::Event) -> Result<bool, Error> {
        if (door_service_protocol::dnd::ENTER..=door_service_protocol::dnd::FINISHED)
            .contains(&e.kind)
        {
            return Ok(self.dnd_event(e));
        }
        if e.kind == door_service_protocol::clipboard::CHANGED {
            // Another owner's clipboard ends this window's cut (no fading).
            self.clipboard_changed(e.a, e.b);
            return Ok(true);
        }
        if e.kind != w::CONTROLS_CHANGED {
            return Ok(false);
        }
        let c = door_service_protocol::desktop::Controls::from_bits(e.a).ok_or(Error::Malformed)?;
        self.controls_changed(c);
        Ok(true)
    }
    fn window_update(
        &mut self,
        storage: &mut Storage,
        client: &mut WindowClient,
        window: u64,
    ) -> Result<bool, Error> {
        let dragged = self.dnd_update(storage, client, window)?
            | self.clipboard_update(storage, client, window);
        Ok(<Self as Hosted>::update(self, storage, client)? | dragged)
    }
    fn update(&mut self, _storage: &mut Storage, c: &mut WindowClient) -> Result<bool, Error> {
        // The desktop Recycle Bin icon shows the full or empty bin.
        if let Some(full) = self.take_bin_report() {
            match c.set_recycle_bin(full) {
                Ok(()) | Err(Error::Service(Code::Denied | Code::Unsupported)) => (),
                Err(e) => return Err(e),
            }
        }
        if let Some(page) = self.pending_settings.take() {
            c.open_settings_page(page)?;
            return Ok(true);
        }
        if self.send_elevation(c)? {
            return Ok(true);
        }
        // Open / Open with: the file goes to its app through the window
        // server, which launches or activates Notes or Preview.
        if let Some((app, payload)) = self.take_document() {
            match c.open_document_in_app(app, &payload) {
                Ok(()) => (),
                Err(Error::Service(code)) => self.document_failed(code),
                Err(e) => return Err(e),
            }
            return Ok(true);
        }
        // "Open in Terminal" (offered for the home folder, where a new
        // Terminal window starts).
        if let Some(app) = self.pending_launch.take() {
            match c.launch_window(app) {
                Err(Error::Service(Code::Unsupported)) => c.launch_app(app)?,
                other => other?,
            }
            return Ok(true);
        }
        Ok(false)
    }
}
