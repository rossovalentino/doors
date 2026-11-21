//! Durable user tag catalog. All I/O is explicit and checked against the opened version.
use super::*;
use door_workspace::explorer_tags::Catalog;
impl Files {
    pub(super) fn reload_tags(&mut self, storage: &mut Storage) {
        match self.load_tags(storage) {
            Ok(()) => self.tags_error.clear(),
            Err(error) => {
                self.tags = Catalog::default();
                self.tags_error = format!(
                    "Tags unavailable: {error:?}. Your saved catalog was preserved. Press F5 to retry."
                );
            }
        }
        self.normalize_tag_view();
    }
    pub(super) fn normalize_tag_view(&mut self) {
        if self.tag_filter.is_some_and(|i| i >= self.tags.names.len()) {
            self.tag_filter = None;
        }
        self.rail_offset = self.rail_offset.clamp(0, self.rail_scroll_max() as i32);
    }
    pub(super) fn hide_tag_metadata(&self, path: &str, entries: &mut Vec<DirEntry>) {
        if path == self.home && self.tags_error.is_empty() {
            entries.retain(|entry| Self::name(entry) != ".door-explorer-tags");
        }
        if path == self.home {
            entries.retain(|entry| Self::name(entry) != super::prefs::FILE);
            entries.retain(|entry| Self::name(entry) != door_workspace::recent::FILE);
        }
    }
    pub(super) fn load_tags(&mut self, storage: &mut Storage) -> Result<(), Error> {
        let path = format!("{}/.door-explorer-tags", self.home);
        let file = match storage.open(&path, Disposition::OpenExisting, READ, SHARE_ALL) {
            Ok(file) => file,
            Err(Error::Service(Code::NotFound)) => {
                self.tags = Catalog::default();
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        let result = Self::read_tag_catalog(storage, &file);
        let closed = storage.close(file);
        self.tags = result?.0;
        closed?;
        Ok(())
    }
    fn read_tag_catalog(
        storage: &mut Storage,
        file: &door_service_client::File,
    ) -> Result<(Catalog, door_service_client::FileInfo), Error> {
        let info = storage.info(file)?;
        if info.bytes > door_workspace::explorer_tags::MAX_BYTES as u64 {
            return Err(Error::Malformed);
        }
        let mut bytes = Vec::new();
        let mut chunk = [0; door_service_protocol::CHUNK_BYTES];
        while bytes.len() < info.bytes as usize {
            let count = (info.bytes as usize - bytes.len()).min(chunk.len());
            let n = storage.read(file, bytes.len() as u64, &mut chunk[..count])?;
            if n == 0 {
                return Err(Error::Malformed);
            }
            bytes.extend_from_slice(&chunk[..n]);
        }
        if storage.info(file)? != info {
            return Err(Error::Service(Code::StaleInstance));
        }
        let catalog = Catalog::decode(&bytes).map_err(|_| Error::Malformed)?;
        Ok((catalog, info))
    }
    fn update_tags(
        &mut self,
        storage: &mut Storage,
        change: impl FnOnce(&mut Catalog) -> Result<(), &'static str>,
    ) -> Result<(), Error> {
        let path = format!("{}/.door-explorer-tags", self.home);
        let file = storage.open(
            &path,
            Disposition::OpenAlways,
            READ | door_service_client::WRITE,
            SHARE_ALL,
        )?;
        let result = (|| {
            let (mut catalog, info) = Self::read_tag_catalog(storage, &file)?;
            if let Err(reason) = change(&mut catalog) {
                self.message = reason.into();
                return Err(Error::Service(Code::InvalidRequest));
            }
            storage.replace_checked(&file, info, &catalog.encode())?;
            Ok(catalog)
        })();
        let closed = storage.close(file);
        let catalog = result?;
        closed?;
        self.tags = catalog;
        if let Some(d) = self.detail.as_mut() {
            d.tags = u32::from(self.tags.tags(d.info.object));
        }
        let _ = door_sys::write(
            format!(
                "DOOR_P12 FILES TAGS SAVED names={} bindings={}\n",
                self.tags.names.len(),
                self.tags.bindings.len()
            )
            .as_bytes(),
        );
        Ok(())
    }
    pub(super) fn toggle_user_tag(
        &mut self,
        index: usize,
        storage: &mut Storage,
    ) -> Result<(), Error> {
        let Some(d) = self.detail.as_ref() else {
            return Ok(());
        };
        if storage.metadata(&d.path)?.object != d.info.object {
            return Err(Error::Service(Code::StaleInstance));
        }
        let object = d.info.object;
        self.update_tags(storage, |catalog| catalog.toggle(object, index))
    }
    pub(super) fn begin_tag(&mut self) {
        self.details_visible = true;
        self.detail_tab = 2;
        let Some(pane) = self.pane() else {
            self.message = "Make the window wider to manage tags.".into();
            return;
        };
        let visible = self
            .layout
            .height
            .saturating_sub(pane.y as u32 + details::body() as u32 + xl::status_height());
        if visible < 34 {
            self.message = "Make the window taller to manage tags.".into();
            return;
        }
        let grid = door_workspace::explorer_tags::TagGrid::new(pane.width, self.tags.names.len());
        let create = grid.create();
        let minimum = (create.y as u32 + create.height)
            .saturating_sub(visible)
            .div_ceil(24) as usize;
        let maximum = create.y as usize / 24;
        if minimum > maximum {
            self.message = "Make the window taller to show the complete tag editor.".into();
            return;
        }
        self.detail_scroll = self.detail_scroll.max(minimum).min(maximum);
        self.editor = 4;
        self.edit.set(String::new(), false);
    }
    /// "Add tag" chip in the Details tab: an inline editor (editor 6) whose
    /// Enter creates the tag if needed and assigns it to the inspected item.
    pub(super) fn begin_add_tag(&mut self) {
        if self.detail.is_none() || !self.tags_error.is_empty() {
            return;
        }
        self.editor = 6;
        self.edit.set(String::new(), false);
    }
    /// Shows only the items with tag `index` (again: shows everything).
    pub(super) fn filter_tag(&mut self, tag: usize) {
        if tag >= self.tags.names.len() {
            return;
        }
        self.tag_filter = if self.tag_filter == Some(tag) {
            None
        } else {
            Some(tag)
        };
        self.overview = false;
        self.offset = 0;
        self.detail = None;
        self.marked.clear();
        self.thumbnail_scanned = false;
        self.folders_scanned = false;
        let first = self.shown().next().map(|(i, _)| i);
        self.selected = first;
        self.message = if self.tag_filter.is_some() {
            format!(
                "{} - {} items in this folder",
                self.tags.names.get(tag).map_or("Tag", String::as_str),
                self.shown().count()
            )
        } else {
            String::new()
        };
    }
    pub(super) fn remove_tag_from_detail(
        &mut self,
        tag: usize,
        storage: &mut Storage,
    ) -> Result<(), Error> {
        if self
            .detail
            .as_ref()
            .is_some_and(|d| d.tags & (1 << tag) != 0)
        {
            self.toggle_user_tag(tag, storage)?;
        }
        Ok(())
    }
    /// Rename a tag in place in the navigation pane (editor 5).
    pub(super) fn begin_tag_rename(&mut self, tag: usize) {
        let Some(name) = self.tags.names.get(tag).cloned() else {
            return;
        };
        self.tag_rename = Some(tag);
        self.editor = 5;
        self.edit.set(name, true);
        // Keep the row visible: rows are 28 px below the section header.
        let top = self.rail_tag_top() + modern::tag_head() + tag as i32 * modern::tag_pitch();
        let view = self.layout.height as i32 - xl::status_height() as i32 - content_top();
        if top - self.rail_offset < content_top()
            || top + modern::tag_pitch() - self.rail_offset > content_top() + view
        {
            self.rail_offset =
                (top - content_top() - view / 2).clamp(0, self.rail_scroll_max() as i32);
        }
    }
    pub(super) fn finish_tag_rename(&mut self, storage: &mut Storage) {
        let Some(tag) = self.tag_rename.take() else {
            self.editor = 0;
            return;
        };
        self.editor = 0;
        let name: String = self.edit.text().into();
        if self.tags.names.get(tag).is_some_and(|n| *n == name.trim()) {
            return;
        }
        if let Err(error) = self.update_tags(storage, |c| c.rename(tag, &name))
            && !matches!(error, Error::Service(Code::InvalidRequest))
        {
            self.message = format!("Tags could not be saved: {error:?}");
        }
    }
    pub(super) fn tag_rename_key(
        &mut self,
        code: u32,
        storage: &mut Storage,
    ) -> Result<bool, Error> {
        match code & 511 {
            1 => {
                self.tag_rename = None;
                self.editor = 0;
                self.edit.set(String::new(), false);
            }
            28 | 0x11c => self.finish_tag_rename(storage),
            _ => self.edit_key(code),
        }
        Ok(true)
    }
    /// Deleting a tag only unassigns it; files are untouched.
    pub(super) fn delete_tag(&mut self, tag: usize, storage: &mut Storage) -> Result<(), Error> {
        let Some(name) = self.tags.names.get(tag).cloned() else {
            return Ok(());
        };
        let mut removed = 0;
        self.update_tags(storage, |c| {
            removed = c.remove(tag)?;
            Ok(())
        })?;
        self.tag_filter = match self.tag_filter {
            Some(f) if f == tag => None,
            Some(f) if f > tag => Some(f - 1),
            other => other,
        };
        self.normalize_tag_view();
        self.message = format!(
            "Deleted tag \"{name}\"; it was removed from {removed} {}.",
            if removed == 1 { "item" } else { "items" }
        );
        Ok(())
    }
    /// Shared single-line editing keys for the tag editors.
    fn edit_key(&mut self, code: u32) {
        let scan = code & 511;
        let control = (code >> 9) & door_abi::graphics::CONTROL != 0;
        let shift = (code >> 9) & door_abi::graphics::SHIFT != 0;
        match scan {
            14 => self.edit.backspace(),
            0x153 => self.edit.delete(),
            0x14b => self.edit.horizontal(false, shift, control),
            0x14d => self.edit.horizontal(true, shift, control),
            0x147 => self.edit.place(0, shift),
            0x14f => self.edit.place(self.edit.text().len(), shift),
            30 if control => self.edit.select_all(),
            _ => {
                let ch = (code >> 16) as u8;
                if !control
                    && (32..=126).contains(&ch)
                    && self
                        .edit
                        .text()
                        .len()
                        .saturating_sub(self.edit.selection().len())
                        < 24
                {
                    self.edit.insert(ch as char);
                }
            }
        }
    }
    pub(super) fn tag_key(&mut self, code: u32, storage: &mut Storage) -> Result<bool, Error> {
        match code & 511 {
            1 => {
                self.editor = 0;
                self.edit.set(String::new(), false);
            }
            28 | 0x11c => {
                let name: String = self.edit.text().into();
                let object = self.detail.as_ref().map(|d| d.info.object);
                match self.update_tags(storage, |catalog| {
                    let index = catalog.create(&name)?;
                    if let Some(id) = object
                        && catalog.tags(id) & (1 << index) == 0
                    {
                        catalog.toggle(id, index)?;
                    }
                    Ok(())
                }) {
                    // The reason is already in the status bar; keep editing.
                    Err(Error::Service(Code::InvalidRequest)) => return Ok(true),
                    other => other?,
                }
                self.editor = 0;
                self.edit.set(String::new(), false);
            }
            _ => self.edit_key(code),
        }
        Ok(true)
    }
}
