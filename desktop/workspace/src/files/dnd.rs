//! File Explorer as a drag source and drop target (door.dnd), as in
//! Windows 11: drag items out of the list (the pressed item, or the whole
//! selection when it is selected) into another folder, another Explorer
//! window or onto a folder item; on the same volume a drop moves, Ctrl
//! copies, Shift moves, and the server labels the drag "Move to X" /
//! "Copy to X" from the folder this window names in its STATUS. Decisions
//! live in `door_workspace::explorer_dnd`; this module does the storage
//! work with the window's own identity (paths only cross the broker).
use super::*;
use door_service_client::window::WindowClient;
use door_service_protocol::{clipboard as cb, dnd as d, window as w};
use door_workspace::explorer_dnd as x;

#[derive(Default)]
pub(crate) struct State {
    /// Pressed entry and where, until it becomes a drag or is released.
    press: Option<(usize, i32, i32)>,
    /// Offer waiting for `update` to start (paths, FILES payload).
    start: Option<(Vec<String>, Vec<u8>)>,
    /// Paths this window is dragging, until FINISHED.
    own: Vec<String>,
    /// The window id this window registered as a drop target.
    registered: Option<u64>,
    /// STATUS owed for the drag over this window: (serial, effects,
    /// default, insert).
    status: Option<(u32, u32, u32, String)>,
    /// DROP to perform: (serial, effect, destination folder).
    drop: Option<(u32, u32, String)>,
    /// A drag from here moved items away: re-list the folder.
    moved_away: bool,
    /// Display scale percent (drag events carry physical coordinates).
    scale: u32,
}

impl Files {
    pub(crate) fn dnd_display(&mut self, scale: u32) {
        self.dnd.scale = scale;
    }
    /// The connection was re-created: register again.
    pub(crate) fn dnd_recreated(&mut self) {
        self.dnd = State {
            scale: self.dnd.scale,
            ..State::default()
        };
    }
    /// The listed entry under logical (`x`, `y`), in a plain folder listing
    /// (`listing`: also volume search results, which can be dragged from).
    pub(super) fn entry_at(&self, x: i32, y: i32, listing: bool) -> Option<usize> {
        if self.modal()
            || self.xmenu.is_some()
            || self.editor != 0
            || self.home_view
            || (self.overview && self.search.is_empty())
            || (!listing && !self.result_paths.is_empty())
            || y < content_top()
            || x < self.layout.sidebar
            || y >= self.layout.height as i32 - xl::status_height() as i32
            || self.pane().is_some_and(|r| r.contains(x, y))
            || self.progress_target(x, y).is_some()
            || self.recycle_view()
        {
            return None;
        }
        let l = self.content_layout();
        self.shown()
            .skip(self.offset)
            .take(l.capacity(self.view))
            .enumerate()
            .find(|(row, _)| self.item_cell(*row).is_some_and(|r| r.contains(x, y)))
            .map(|(_, (index, _))| index)
    }
    /// Primary press: an item may become a drag source.
    pub(crate) fn drag_press(&mut self, x: i32, y: i32) {
        self.dnd.press = self.entry_at(x, y, true).map(|i| (i, x, y));
    }
    pub(crate) fn drag_release(&mut self) {
        self.dnd.press = None;
    }
    /// Pointer motion with the button held: past the drag rectangle the
    /// pressed item (or the selection it belongs to) is offered.
    pub(crate) fn drag_motion(&mut self, x: i32, y: i32) -> bool {
        let Some((index, px, py)) = self.dnd.press else {
            return false;
        };
        if !x::drag_started((px, py), (x, y)) {
            return false;
        }
        self.dnd.press = None;
        let indices: Vec<usize> = if self.is_selected(index) {
            (0..self.entries.len())
                .filter(|i| self.is_selected(*i))
                .collect()
        } else {
            alloc::vec![index]
        };
        let paths: Vec<String> = indices
            .iter()
            .filter_map(|i| self.entries.get(*i))
            .filter_map(|e| self.path_for(e))
            .collect();
        match x::payload(&paths) {
            Ok(bytes) if !paths.is_empty() => self.dnd.start = Some((paths, bytes)),
            _ => self.message = "These items can't be dragged.".into(),
        }
        true
    }
    /// The folder a drop at physical content-local (`px`, `py`) lands in: a
    /// folder item under the pointer, else the folder shown.
    pub(super) fn drop_folder(&self, px: i32, py: i32) -> Option<String> {
        let scale = door_graphics_core::Scale::from_percent(self.dnd.scale.max(100))?;
        let (x, y) = (scale.logical(px), scale.logical(py));
        if let Some(entry) = self.entry_at(x, y, false).and_then(|i| self.entries.get(i))
            && entry.info.directory
        {
            return self.path_for(entry);
        }
        // Over the list (not on an item): the folder shown, when it is one.
        (y >= content_top()
            && x >= self.layout.sidebar
            && !self.home_view
            && !self.overview
            && self.result_paths.is_empty()
            && !self.modal())
        .then(|| self.cwd.clone())
    }
    /// door.dnd events for this window (target ENTER/OVER/LEAVE/DROP and
    /// source FINISHED).
    pub(crate) fn dnd_event(&mut self, e: w::Event) -> bool {
        match e.kind {
            d::ENTER | d::OVER => {
                let serial = e.value as u32;
                let folder = self.drop_folder(e.rect.x, e.rect.y);
                let (effects, default) = match &folder {
                    // Only this window knows what it is dragging itself.
                    Some(f) if e.a & cb::mask(cb::FILES) != 0 => {
                        if self.dnd.own.is_empty() {
                            (d::COPY | d::MOVE, d::MOVE)
                        } else {
                            x::status(&self.dnd.own, f)
                        }
                    }
                    _ => (d::NONE, d::NONE),
                };
                let insert = folder.as_deref().map(x::insert).unwrap_or_default();
                self.dnd.status = Some((serial, effects, default, insert));
                false
            }
            d::LEAVE => {
                self.dnd.status = None;
                false
            }
            d::DROP => {
                self.dnd.status = None;
                match self.drop_folder(e.rect.x, e.rect.y) {
                    Some(folder) => self.dnd.drop = Some((e.value as u32, e.b, folder)),
                    None => self.dnd.drop = Some((e.value as u32, d::NONE, String::new())),
                }
                false
            }
            d::FINISHED => {
                self.dnd.moved_away = e.a == d::MOVE && e.b == d::OUTCOME_DROPPED;
                self.dnd.own.clear();
                false
            }
            _ => false,
        }
    }
    /// Requests owed to the server: registration, a pending drag start,
    /// STATUS answers and dropped items.
    pub(crate) fn dnd_update(
        &mut self,
        storage: &mut Storage,
        client: &mut WindowClient,
        window: u64,
    ) -> Result<bool, Error> {
        if !client.supports_drag_drop() {
            return Ok(false);
        }
        let mut dirty = false;
        if self.dnd.registered != Some(window) {
            client.register_drop_target(window, cb::mask(cb::FILES), d::COPY | d::MOVE, d::MOVE)?;
            self.dnd.registered = Some(window);
        }
        if let Some((paths, bytes)) = self.dnd.start.take() {
            match client.start_drag(window, d::COPY | d::MOVE, &[(cb::FILES, &bytes)]) {
                Ok(_) => {
                    let _ = door_sys::write(
                        format!("DOOR_P12 FILES DRAG count={}\n", paths.len()).as_bytes(),
                    );
                    self.dnd.own = paths;
                }
                // Released before the drag could start: an ordinary click.
                Err(Error::Service(Code::Denied | Code::InvalidRequest)) => (),
                Err(e) => return Err(e),
            }
        }
        if let Some((serial, effects, default, insert)) = self.dnd.status.take() {
            match client.drag_status_described(serial, window, effects, default, &insert) {
                Ok(()) | Err(Error::Service(Code::NotFound | Code::Changed)) => (),
                Err(e) => return Err(e),
            }
        }
        if let Some((serial, effect, folder)) = self.dnd.drop.take() {
            let performed = self.perform_drop(storage, client, serial, effect, &folder);
            match client.drop_finish(serial, performed) {
                Ok(()) | Err(Error::Service(Code::NotFound | Code::Changed)) => (),
                Err(e) => return Err(e),
            }
            dirty = true;
        }
        if core::mem::take(&mut self.dnd.moved_away) {
            self.refresh(storage)?;
            dirty = true;
        }
        Ok(dirty)
    }
    /// Read the dropped paths and move or copy them into `folder`; returns
    /// the effect performed (NONE when nothing landed).
    fn perform_drop(
        &mut self,
        storage: &mut Storage,
        client: &mut WindowClient,
        serial: u32,
        effect: u32,
        folder: &str,
    ) -> u32 {
        if !matches!(effect, d::MOVE | d::COPY) || folder.is_empty() {
            return d::NONE;
        }
        let mut payload = Vec::new();
        let mut chunk = [0u8; cb::READ_BYTES];
        loop {
            match client.drop_read(serial, cb::FILES, payload.len() as u32, &mut chunk) {
                Ok(0) => break,
                Ok(n) if payload.len() + n <= cb::MAX_FILES_BYTES => {
                    payload.extend_from_slice(&chunk[..n])
                }
                _ => return d::NONE,
            }
        }
        let Ok(sources) = x::paths(&payload) else {
            return d::NONE;
        };
        // Into the Recycle Bin: the items are recycled.
        if folder == RECYCLE_BIN {
            let mut recycled = 0;
            for source in &sources {
                if let Ok(door_service_protocol::files::RecycleOutcome::Recycled(id)) =
                    storage.recycle(source)
                {
                    recycled += 1;
                    door_rt::diagnostic(format_args!("DOOR_P14 RECYCLE id={id} path={source}\n"));
                }
            }
            let _ = self.relist(storage);
            return if recycled == 0 { d::NONE } else { d::MOVE };
        }
        // Same checks as the STATUS this window could not make for another
        // window's drag: nothing into itself or where it already is.
        if x::status(&sources, folder).0 == d::NONE {
            self.message = "The items are already in this folder.".into();
            return d::NONE;
        }
        // The storage engine moves (one rename per item on this volume) or
        // copies, with the progress and conflict dialogs; Ctrl copies and
        // Shift moves (the server resolved the keys into `effect`).
        let mode = if effect == d::COPY {
            door_service_protocol::files::FileOpMode::Copy
        } else {
            door_service_protocol::files::FileOpMode::Move
        };
        let plans = door_workspace::file_operations::plans(&sources, folder, mode);
        let started = self.start_plans(storage, plans);
        let _ = door_sys::write(
            format!(
                "DOOR_P12 FILES DROP effect={} done={started} skipped=0 folder={folder}\n",
                if effect == d::COPY { "copy" } else { "move" }
            )
            .as_bytes(),
        );
        if started == 0 { d::NONE } else { effect }
    }
}
