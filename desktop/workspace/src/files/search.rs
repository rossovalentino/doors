//! 26H2 This PC search is index-based. The index itself is the shared
//! `door_workspace::explorer_search::Index` (names on every mounted volume,
//! built in the background); this window advances it per tick while a This
//! PC search needs it, shows the matches as items and states the coverage.
use super::*;
use door_workspace::explorer_search::{BinAction, Index, Open, State};

const CALLS_PER_TICK: usize = 96;
/// Rows the item view can address (ids 100..=227).
pub(super) const MAX_RESULTS: usize = 128;

impl Files {
    /// Whether the item view shows volume search results (This PC search).
    pub(super) fn volume_search(&self) -> bool {
        self.overview && !self.search.is_empty()
    }
    /// Starts indexing when the search box opens on This PC.
    pub(super) fn warm_index(&mut self) {
        if self.overview && self.index.is_none() {
            self.index = Some(Index::new());
        }
    }
    /// Shows the index's matches for `self.search` as the item list.
    pub(super) fn apply_results(&mut self) {
        if self.index.is_none() {
            self.index = Some(Index::new());
        }
        let Some(idx) = self.index.as_ref() else {
            return;
        };
        self.index_applied = idx.len();
        let keep = self
            .selected
            .and_then(|i| self.entries.get(i))
            .map(|e| e.info.object);
        let mut found: Vec<(String, DirEntry)> = idx
            .matches(&self.search)
            .take(MAX_RESULTS)
            .cloned()
            .collect();
        found.sort_by(|a, b| {
            explorer_sort::natural(&a.1.name[..a.1.name_len], &b.1.name[..b.1.name_len])
        });
        // `sort` keeps `result_paths` aligned with the entries.
        self.result_paths = found.iter().map(|(p, _)| p.clone()).collect();
        self.entries = found.into_iter().map(|(_, e)| e).collect();
        self.selected = keep.and_then(|o| self.entries.iter().position(|e| e.info.object == o));
        self.offset = self.offset.min(self.max_offset());
        self.thumbnail_scanned = false;
        self.folders_scanned = false;
    }
    /// Leaves volume search: This PC lists the home folder again.
    pub(super) fn end_volume_search(&mut self, storage: &mut Storage) {
        if self.result_paths.is_empty() && self.entries.is_empty() {
            return;
        }
        self.result_paths.clear();
        if let Ok(mut entries) = Self::read_entries(storage, &self.home) {
            let home = self.home.clone();
            self.hide_tag_metadata(&home, &mut entries);
            entries.retain(Self::visible);
            self.entries = entries;
            self.sort();
        }
        self.selected = None;
        self.offset = 0;
    }
    /// What the search covered, for the status bar.
    pub(super) fn search_scope(&self) -> String {
        let Some(idx) = self.index.as_ref() else {
            return String::new();
        };
        let count = self.shown().count();
        let names = format!("{} names on {}", idx.len(), DRIVES[0].label);
        let more = if count >= MAX_RESULTS {
            " (first 128 shown)"
        } else {
            ""
        };
        match idx.state() {
            State::Building => format!("{count} results so far{more} - indexing... {names}"),
            State::Complete => {
                format!("{count} results{more} - file and folder names, {names} indexed")
            }
            State::Partial => format!(
                "{count} results{more} - names only; index stopped at {names}{}",
                if idx.unreadable() > 0 {
                    format!(", {} folders unreadable", idx.unreadable())
                } else {
                    String::new()
                }
            ),
        }
    }
    /// Builds the index while the search box is open on This PC or a search
    /// is shown; refreshes results as names arrive.
    pub(super) fn index_step(&mut self, storage: &mut Storage) -> bool {
        if !(self.overview && (self.editor == 2 || !self.search.is_empty())) {
            return false;
        }
        self.warm_index();
        let Some(idx) = self.index.as_mut() else {
            return false;
        };
        if idx.state() != State::Building {
            return false;
        }
        idx.step(storage, CALLS_PER_TICK);
        let (len, state) = (idx.len(), idx.state());
        if self.volume_search() && (len != self.index_applied || state != State::Building) {
            self.apply_results();
            if state != State::Building {
                door_rt::diagnostic(format_args!(
                    "DOOR_P12 FILES INDEX state={state:?} items={len}\n"
                ));
            }
            return true;
        }
        false
    }
    /// Selects and reveals the listed item called `name`.
    pub(super) fn select_named(&mut self, storage: &mut Storage, name: &str) -> Result<(), Error> {
        let found = self.entries.iter().position(|e| Self::name(e) == name);
        if let Some(i) = found {
            self.selected = Some(i);
            self.marked.clear();
            let position = self.shown().position(|(j, _)| j == i);
            if let Some(p) = position {
                self.reveal_position(p);
            }
            self.inspect_selection(storage)?;
        }
        Ok(())
    }
    /// Opens a location (selecting an item) or runs a This PC search: the
    /// receiving end of `explorer_search::Open` (Start search, new windows).
    pub fn open_request(&mut self, storage: &mut Storage, request: &Open) -> Result<(), Error> {
        match request {
            Open::Location { folder, select } => {
                self.open_location(storage, folder.clone())?;
                if let Some(name) = select {
                    self.select_named(storage, name)?;
                }
            }
            // Run's folders and paths resolve as typed in the address bar;
            // one that can't be opened is reported the same way.
            Open::Typed(text) => {
                // Run resolves relative names against the profile folder,
                // not against the folder this window last showed.
                let rooted = text.starts_with(['~', '%', '\\', '/', '.'])
                    || text.as_bytes().get(1) == Some(&b':')
                    || text
                        .get(..6)
                        .is_some_and(|s| s.eq_ignore_ascii_case("shell:"))
                    || text.eq_ignore_ascii_case("Home")
                    || text.eq_ignore_ascii_case("This PC");
                let text = if rooted {
                    text.clone()
                } else {
                    alloc::format!("~\\{text}")
                };
                self.edit.set(text, false);
                let result = self.address(storage);
                self.edit.set(String::new(), false);
                if let Err(error) = result {
                    self.message = Self::location_error(&error).into();
                }
            }
            Open::Search(text) => {
                self.go_this_pc(storage)?;
                self.search = text.trim().into();
                self.apply_results();
            }
            // The desktop Recycle Bin's context menu.
            Open::RecycleBin(action) => {
                self.open_location(storage, RECYCLE_BIN.into())?;
                match action {
                    BinAction::Empty => self.ask_empty_bin(storage),
                    BinAction::Properties => self.recycle_properties(false, storage),
                }
            }
            // A Windows program's ShellExecute of a file Windows has no
            // association for (OpenWith.exe on Windows).
            Open::OpenWith { path, choose } => {
                let (folder, name) = match path.rsplit_once('/') {
                    Some(("", name)) => ("/", name),
                    Some((folder, name)) => (folder, name),
                    None => return Err(Error::Service(Code::InvalidRequest)),
                };
                self.open_location(storage, folder.into())?;
                self.select_named(storage, name)?;
                if *choose {
                    self.choose_app(storage, path);
                } else {
                    self.open_file(storage, path);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn this_pc_results_come_from_the_shared_index_and_are_bounded() {
        let mut f = Files::empty("/users/owner".into(), Controls::default());
        f.overview = true;
        f.search = "a".into();
        let mut idx = Index::for_roots(&["/compat"]);
        for i in 0..300u64 {
            let name = format!("a{i}.txt");
            let mut units = [0u16; door_service_protocol::files::NAME_UNITS];
            for (o, u) in units.iter_mut().zip(name.encode_utf16()) {
                *o = u;
            }
            idx.visit(
                "/users/owner/x",
                1,
                DirEntry {
                    times: Default::default(),
                    cookie: i,
                    name: units,
                    name_len: name.len(),
                    info: door_service_client::FileInfo {
                        attributes: 0,
                        bytes: 1,
                        object: i + 100,
                        version: 1,
                        directory: false,
                    },
                },
            );
        }
        f.index = Some(idx);
        f.apply_results();
        assert_eq!(f.entries.len(), MAX_RESULTS);
        assert_eq!(f.result_paths.len(), MAX_RESULTS);
        let first = f.entries[0];
        assert_eq!(f.path_for(&first).unwrap(), f.result_paths[0]);
        assert!(f.result_paths[0].starts_with("/users/owner/x/a"));
        assert!(f.search_scope().contains("first 128 shown"));
        assert!(
            f.rename_block().is_some(),
            "results cannot be renamed in place"
        );
        // Sorting keeps every result with its own path.
        f.descending = true;
        f.sort();
        for (e, p) in f.entries.iter().zip(&f.result_paths) {
            assert!(p.ends_with(&Files::name(e)));
        }
    }
}
