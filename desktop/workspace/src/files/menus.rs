//! Explorer context menus and dropdown flyouts on the shared Windows 11 menu
//! model (`door_workspace::context_menu`): what was clicked decides the
//! commands, every enabled command performs its real operation, and the
//! painter reproduces the 25H2 compact menu (rounded panel, icon row,
//! separators, hover plates, shortcuts, submenus, disabled reasons).
use super::menu_paint::ink;
use super::*;
use door_workspace::context_menu::{
    self as cm, Action, Entry, Focus, Gates, Glyph, Hit, MenuState, Outcome, Panel, Source, Style,
    Subject,
};
use door_workspace::explorer_tree::Node as TreeNode;

/// What the open menu acts on.
pub(super) enum MenuFor {
    None,
    Context {
        subject: Subject,
        /// Target paths (THIS_PC for This PC); the first is the clicked item.
        paths: Vec<String>,
        tree_row: Option<usize>,
        keyboard: bool,
    },
    /// Location flyout: `Pick(i)` opens `paths[i]`.
    Locations(Vec<String>),
    /// Back/Forward history list: `Pick(i)` jumps to history entry i.
    History,
    /// Command-bar dropdown (New, Sort, View, See more).
    Command,
}

/// Hit-test identities: one per row so the runtime repaints hover changes.
pub(super) const MENU_ROW: u32 = 2000;
pub(super) const MENU_ICON: u32 = 2100;
pub(super) const MENU_SUB: u32 = 2200;
pub(super) const MENU_CHROME: u32 = 1998;
pub(super) const MENU_OUTSIDE: u32 = 1999;

impl Files {
    pub fn menu_open(&self) -> bool {
        self.xmenu.is_some()
    }
    pub fn hover_menu(&mut self, x: i32, y: i32) -> bool {
        !self.modal() && self.xmenu.as_mut().is_some_and(|m| m.hover(x, y))
    }
    pub fn dismiss_menu(&mut self) {
        self.xmenu = None;
        self.crumb_flyout = None;
        self.menu_for = MenuFor::None;
    }
    pub fn context(&mut self, x: i32, y: i32, w: u32, h: u32) {
        self.open_context(x, y, w, h, false);
    }
    /// Shift+F10 / the menu key: the menu of the selection, else of the folder.
    pub fn keyboard_context(&mut self, w: u32, h: u32) {
        let cell = if self.overview && self.search.is_empty() {
            self.overview_selected
                .checked_sub(self.offset)
                .filter(|_| self.overview_selected < self.drives().len())
                .and_then(|row| self.item_cell(row))
        } else if self.home_view {
            // Home lays its sections out itself (tiles and lists).
            self.selected
                .and_then(|i| self.home_layout().cells.get(i).copied().flatten())
        } else {
            self.shown()
                .skip(self.offset)
                .take(self.content_layout().capacity(self.view))
                .position(|(index, _)| Some(index) == self.selected)
                .and_then(|row| self.item_cell(row))
        };
        match cell {
            Some(r) => self.open_context(r.x + 8, r.y + 8, w, h, true),
            // Nothing selected: the folder (or This PC) background menu.
            None => {
                self.background_menu = true;
                self.open_context(self.layout.sidebar + 24, content_top() + 48, w, h, true);
                self.background_menu = false;
            }
        }
    }
    fn subject_at(&mut self, x: i32, y: i32) -> Option<(Subject, Vec<String>, Option<usize>)> {
        if let Some(r) = self.pane().filter(|r| r.contains(x, y)) {
            // Tag chips as painted (their widths are measured text). Rows of
            // the Tags tab catalog (ids 920+) are tags themselves.
            let chip = self
                .chip_hits
                .borrow()
                .iter()
                .find(|(b, _, _)| b.contains(x, y) && b.y >= r.y + super::details::body())
                .map(|(_, tag, id)| (*tag, *id));
            return chip.map(|(tag, id)| {
                let subject = if (920..928).contains(&id) {
                    Subject::Tag(tag as u8)
                } else {
                    Subject::TagChip(tag as u8)
                };
                (subject, Vec::new(), None)
            });
        }
        if y < content_top() || y >= self.layout.height as i32 - xl::status_height() as i32 || x < 0
        {
            return None;
        }
        if x < self.layout.sidebar {
            if self
                .rail_scrollbar()
                .is_some_and(|(track, _)| track.contains(x, y))
            {
                return None;
            }
            let ry = y + self.rail_offset;
            if let Some(id) = self.rail_target(x, ry) {
                let index = ((id - 1100) % 300) as usize;
                let rows = self.rail_rows();
                let row = rows.get(index)?;
                let (pins, _) = self.rail_pins();
                let drives = self.rail_drives();
                let path = row.path(&pins, &drives).map(String::from);
                let home = row.node == TreeNode::Home;
                let subject = match row.node {
                    // Home behaves like This PC here (open, new tab, new window).
                    TreeNode::Home | TreeNode::ThisPc => Subject::ThisPc,
                    TreeNode::Drive { .. } => Subject::Drive,
                    TreeNode::Pin(_) => Subject::TreePin,
                    TreeNode::Folder { .. } | TreeNode::NoneDrive { .. } => Subject::TreeFolder,
                    TreeNode::RecycleBin => {
                        return Some((
                            Subject::RecycleBin,
                            alloc::vec![RECYCLE_BIN.into()],
                            Some(index),
                        ));
                    }
                    TreeNode::Network | TreeNode::Linux | TreeNode::Separator => return None,
                };
                let fallback = if home { HOME_PAGE } else { THIS_PC };
                let paths = alloc::vec![path.unwrap_or_else(|| fallback.into())];
                return Some((subject, paths, Some(index)));
            }
            let tag_top = self.rail_tag_top();
            for i in 0..self.tags.names.len() {
                let top = tag_top + modern::tag_head() + i as i32 * modern::tag_pitch();
                if (top..top + modern::tag_pitch()).contains(&ry) {
                    return Some((Subject::Tag(i as u8), Vec::new(), None));
                }
            }
            return None;
        }
        let l = self.content_layout();
        if x >= l.width as i32 || self.column_target(x, y).is_some() {
            return None;
        }
        if self.overview && self.search.is_empty() {
            for (row, slot) in (self.offset..self.drives().len()).enumerate() {
                if self.item_cell(row).is_some_and(|r| r.contains(x, y)) {
                    self.overview_selected = slot;
                    return Some((
                        Subject::Drive,
                        alloc::vec![self.drives()[slot].path.into()],
                        None,
                    ));
                }
            }
            return Some((Subject::Background, alloc::vec![THIS_PC.into()], None));
        }
        let hit = if self.home_view {
            if self.home_layout().headers.iter().any(|h| h.contains(x, y)) {
                return None;
            }
            self.home_item_at(x, y)
                .and_then(|i| self.entries.get(i).map(|e| (i, *e)))
        } else {
            self.shown()
                .skip(self.offset)
                .take(l.capacity(self.view))
                .enumerate()
                .find(|(row, _)| self.item_cell(*row).is_some_and(|r| r.contains(x, y)))
                .map(|(_, (index, entry))| (index, *entry))
        };
        let Some((index, entry)) = hit else {
            let key = if self.home_view {
                HOME_PAGE.into()
            } else {
                self.cwd.clone()
            };
            return Some((Subject::Background, alloc::vec![key], None));
        };
        if self.recycle_view() {
            if !(self.marked.len() > 1 && self.marked.contains(&entry.info.object)) {
                self.marked.clear();
            }
            self.selected = Some(index);
            return Some((Subject::RecycleItem, alloc::vec![RECYCLE_BIN.into()], None));
        }
        if self.marked.len() > 1 && self.marked.contains(&entry.info.object) {
            let paths = self
                .entries
                .iter()
                .filter(|e| self.marked.contains(&e.info.object))
                .filter_map(|e| self.path_for(e))
                .collect();
            self.selected = Some(index);
            return Some((Subject::Multiple, paths, None));
        }
        self.marked.clear();
        self.selected = Some(index);
        let path = self.path_for(&entry)?;
        Some((
            if entry.info.directory {
                Subject::Folder
            } else {
                Subject::File
            },
            alloc::vec![path],
            None,
        ))
    }
    fn menu_gates(&self, subject: Subject, paths: &[String], tree_row: Option<usize>) -> Gates {
        let single = matches!(subject, Subject::File | Subject::Folder);
        let rename = if !single {
            Some(match subject {
                Subject::Multiple => cm::MULTI_RENAME,
                Subject::Drive => cm::RENAME_DRIVE,
                _ => cm::RENAME_TREE,
            })
        } else {
            self.rename_block()
        };
        let delete = match subject {
            Subject::RecycleItem => None,
            Subject::File | Subject::Folder | Subject::Multiple => self.delete_block(),
            _ => Some("Select the folder in the file list to delete it."),
        };
        let (cut, copy) = match subject {
            Subject::File | Subject::Folder | Subject::Multiple => {
                (self.delete_block(), self.copy_block())
            }
            Subject::RecycleItem => (Some("Restore the item to move it."), None),
            _ => (Some(cm::SELECT_FIRST), Some(cm::SELECT_FIRST)),
        };
        let folder = match subject {
            Subject::Background => Some(self.cwd.as_str()),
            Subject::Folder | Subject::TreeFolder | Subject::TreePin | Subject::Drive => {
                paths.first().map(String::as_str)
            }
            _ => None,
        };
        let terminal = if folder == Some(self.home.as_str()) && !self.overview {
            None
        } else {
            Some(cm::TERMINAL_HOME)
        };
        let compress = if self.archive.is_some() {
            Some("Another archive is being created. Wait for it to finish.")
        } else {
            None
        };
        let expanded = tree_row.and_then(|i| {
            let rows = self.rail_rows();
            rows.get(i).filter(|r| r.expandable).map(|r| r.expanded)
        });
        let first = paths.first().map(String::as_str).unwrap_or("");
        let (pinned, pin_reason) = match subject {
            Subject::Folder | Subject::TreeFolder | Subject::TreePin => match self.pinned(first) {
                Some(true) => (Some(true), None),
                Some(false) => (Some(true), Some(cm::PIN_KNOWN)),
                None => (Some(false), None),
            },
            Subject::Drive | Subject::ThisPc => (
                Some(false),
                Some("Drives and This PC are always in the navigation pane."),
            ),
            _ => (None, Some(cm::PIN_FOLDERS)),
        };
        let favorite = (subject == Subject::File).then(|| self.favorite(first));
        let section = self.selected.and_then(|i| self.home_section(i));
        let located = single && (self.home_view || !self.result_paths.is_empty());
        let extract = (subject == Subject::File && super::extract::extractable(first)).then(|| {
            self.extract
                .is_some()
                .then_some("Another archive is being extracted. Wait for it to finish.")
        });
        let empty_bin = if self.recycle_view() {
            (self.entries.is_empty()).then_some(cm::BIN_EMPTY)
        } else {
            (self.ops.bin_items == Some(0)).then_some(cm::BIN_EMPTY)
        };
        Gates {
            cut,
            copy,
            rename,
            delete,
            terminal,
            compress,
            new_window: None,
            paste: self.paste_block(),
            recycle: self.recycle_view(),
            empty_bin,
            expanded,
            view: self.view_mode().index() as u8,
            sort: self.sort_key as u8,
            descending: self.descending,
            overview: (self.overview || self.home_view) && self.search.is_empty(),
            file_actions: !self.controls.has(Controls::CONTEXT_HIDE_FILE_ACTIONS),
            pinned,
            pin_reason,
            favorite,
            recent: single && section == Some(super::home::RECENT),
            located,
            extract,
            convert: subject == Subject::File
                && !door_workspace::converter::targets(Kind::of(first, false), first).is_empty(),
            elevate: subject == Subject::File
                && (super::is_program(first) || super::is_script(first)),
            installer: subject == Subject::File && super::is_package(first),
            tags: matches!(subject, Subject::File | Subject::Folder) && self.tags_error.is_empty(),
        }
    }
    pub(super) fn open_context(&mut self, x: i32, y: i32, w: u32, h: u32, keyboard: bool) {
        if self.modal() {
            return;
        }
        self.dismiss_menu();
        if self.editor == 3 {
            self.rename = None;
        }
        self.editor = 0;
        // Right-clicking Back or Forward lists the recent locations.
        for i in 0..2 {
            if let Some(r) = ExplorerLayout::navigation(i).filter(|r| r.contains(x, y)) {
                if self.history.len() > 1 {
                    self.open_history_menu(r);
                }
                return;
            }
        }
        let target = if self.background_menu {
            let key = if self.overview && self.search.is_empty() {
                THIS_PC.into()
            } else {
                self.cwd.clone()
            };
            Some((Subject::Background, alloc::vec![key], None))
        } else {
            self.subject_at(x, y)
        };
        let Some((subject, paths, tree_row)) = target else {
            return;
        };
        let gates = self.menu_gates(subject, &paths, tree_row);
        let mut menu = if self.controls.classic_context_menu() {
            // The classic-menu setting is Windows 10's menu.
            let mut menu = cm::classic(subject, gates);
            menu.look.win10 = true;
            menu
        } else {
            cm::modern(subject, gates, self.controls)
        };
        self.add_open_with(&mut menu, subject, &paths);
        // The user's layout for this context (Settings > Context menu).
        self.menu_profile.apply(subject, &mut menu);
        self.xmenu = Some(MenuState::open(menu, (x, y), (w, h), keyboard));
        self.menu_for = MenuFor::Context {
            subject,
            paths,
            tree_row,
            keyboard,
        };
        door_rt::diagnostic(format_args!("DOOR_P12 FILES MENU subject={subject:?}\n"));
    }
    /// Opens a command-bar or history dropdown under `anchor`.
    pub(super) fn open_dropdown(
        &mut self,
        entries: Vec<Entry>,
        anchor: Bounds,
        width: u32,
        kind: MenuFor,
    ) {
        self.dismiss_menu();
        let menu = cm::Menu {
            style: Style::Modern,
            width,
            icons: Vec::new(),
            entries,
            look: cm::Look::default(),
        };
        self.xmenu = Some(MenuState::open(
            menu,
            (anchor.x, anchor.y + anchor.height as i32 + 2),
            (self.layout.width, self.layout.height),
            false,
        ));
        self.menu_for = kind;
    }
    pub(super) fn menu_target(&self, x: i32, y: i32) -> Option<u32> {
        let m = self.xmenu.as_ref()?;
        Some(match m.hit(x, y) {
            Hit::Outside => MENU_OUTSIDE,
            Hit::Chrome => MENU_CHROME,
            Hit::Icon(i) => MENU_ICON + i as u32,
            Hit::Row(i) => MENU_ROW + i as u32,
            Hit::SubRow(i) => MENU_SUB + i as u32,
        })
    }
    pub(super) fn menu_click(
        &mut self,
        x: i32,
        y: i32,
        storage: &mut Storage,
    ) -> Result<(), Error> {
        let Some(m) = self.xmenu.as_mut() else {
            return Ok(());
        };
        let outcome = m.click(x, y);
        self.run_menu(outcome, storage)
    }
    pub(super) fn menu_key(&mut self, scan: u32, storage: &mut Storage) -> Result<(), Error> {
        let Some(m) = self.xmenu.as_mut() else {
            return Ok(());
        };
        let outcome = m.key(scan);
        self.run_menu(outcome, storage)
    }
    fn run_menu(&mut self, outcome: Outcome, storage: &mut Storage) -> Result<(), Error> {
        match outcome {
            Outcome::Stay => Ok(()),
            Outcome::Dismiss => {
                self.dismiss_menu();
                Ok(())
            }
            Outcome::Invoke(action, source, index) => {
                let anchor = self.xmenu.as_ref().map(|m| m.anchor);
                let kind = core::mem::replace(&mut self.menu_for, MenuFor::None);
                self.xmenu = None;
                self.crumb_flyout = None;
                self.invoke(action, source, index, kind, anchor, storage)
            }
        }
    }
    fn invoke(
        &mut self,
        action: Action,
        _source: Source,
        _index: usize,
        kind: MenuFor,
        anchor: Option<(i32, i32)>,
        storage: &mut Storage,
    ) -> Result<(), Error> {
        let (subject, paths, tree_row, keyboard) = match kind {
            MenuFor::Locations(paths) => {
                if let Action::Pick(i) = action
                    && let Some(path) = paths.get(i as usize).filter(|p| !p.is_empty()).cloned()
                {
                    if path == HOME_PAGE {
                        self.go_home(storage)?;
                    } else if path == THIS_PC {
                        self.go_this_pc(storage)?;
                    } else {
                        self.navigate(storage, path)?;
                    }
                }
                return Ok(());
            }
            MenuFor::History => {
                if let Action::Pick(i) = action {
                    self.jump_history(storage, i as usize)?;
                }
                return Ok(());
            }
            MenuFor::Command | MenuFor::None => (
                Subject::Background,
                alloc::vec![self.cwd.clone()],
                None,
                false,
            ),
            MenuFor::Context {
                subject,
                paths,
                tree_row,
                keyboard,
            } => (subject, paths, tree_row, keyboard),
        };
        let first = paths.first().cloned().unwrap_or_default();
        match action {
            Action::Open => match subject {
                Subject::File => {
                    self.activate(storage)?;
                }
                Subject::ThisPc if first == HOME_PAGE => self.go_home(storage)?,
                Subject::ThisPc => self.go_this_pc(storage)?,
                _ => {
                    if let Some(row) = tree_row {
                        self.open_rail_row(storage, row)?;
                    } else {
                        self.navigate(storage, first)?;
                    }
                }
            },
            Action::OpenNewTab => {
                if first == HOME_PAGE {
                    self.new_tab(storage, HOME_PAGE.into())?;
                } else if first == THIS_PC {
                    self.new_tab(storage, self.home.clone())?;
                    let entries = Self::read_entries(storage, &self.home)?;
                    self.show_this_pc(entries);
                    self.load_volume(storage);
                } else {
                    self.new_tab(storage, first)?;
                }
            }
            Action::OpenNewWindow => {
                self.request_window(door_workspace::explorer_search::Open::Location {
                    folder: first,
                    select: None,
                });
            }
            Action::Properties
                if matches!(subject, Subject::RecycleItem | Subject::RecycleBin)
                    || self.recycle_view() =>
            {
                self.recycle_properties(subject == Subject::RecycleItem, storage);
            }
            // This PC > Properties opens Settings > System > About (device
            // name and specifications), as Windows 11 does.
            Action::Properties
                if first == THIS_PC && matches!(subject, Subject::ThisPc | Subject::Background) =>
            {
                door_rt::diagnostic(format_args!("DOOR_P14 FILES PROPERTIES this_pc=1\n"));
                self.pending_settings = Some(door_settings_map::Page::About.index());
            }
            Action::Properties => {
                door_rt::diagnostic(format_args!(
                    "DOOR_P12 FILES PROPERTIES subject={subject:?}\n"
                ));
                self.details_visible = true;
                self.detail_tab = 0;
                if first == THIS_PC || first == HOME_PAGE {
                    self.cancel_preview(storage);
                    self.detail = None;
                } else if subject == Subject::Multiple {
                    self.inspect_selection(storage)?;
                } else {
                    self.show_details(storage, first)?;
                }
                // Shown until the selection changes.
                self.pane_hold = Some((self.nav_generation, self.selected, self.marked.len()));
            }
            Action::ShowMore => {
                let gates = self.menu_gates(subject, &paths, tree_row);
                let mut menu = cm::classic(subject, gates);
                self.add_open_with(&mut menu, subject, &paths);
                let (x, y) = anchor.unwrap_or((self.layout.sidebar + 24, content_top() + 48));
                self.xmenu = Some(MenuState::open(
                    menu,
                    (x, y),
                    (self.layout.width, self.layout.height),
                    keyboard,
                ));
                self.menu_for = MenuFor::Context {
                    subject,
                    paths,
                    tree_row,
                    keyboard,
                };
            }
            Action::Customize => {
                // Settings > Personalization > Context menu, through the real
                // Settings singleton (LAUNCH_APP destination page 16).
                door_rt::diagnostic(format_args!(
                    "DOOR_P12 FILES CUSTOMIZE_MENU page=16
"
                ));
                self.pending_settings = Some(16);
            }
            Action::OpenTerminal => {
                // Terminal starts in the home folder; the command is offered
                // only there, so the new window opens exactly where asked.
                self.pending_launch = Some(9);
            }
            Action::Refresh => self.user_refresh(storage)?,
            Action::NewFolder => self.new_folder(storage)?,
            Action::NewTextDocument => self.new_text_document(storage)?,
            Action::View(i) => self.set_view(ViewMode::from_index(i as usize).unwrap_or_default()),
            Action::Sort(i) => {
                if let Some(key) = SortKey::ALL.get(i as usize) {
                    self.sort_key = *key;
                }
                self.resort();
            }
            Action::Ascending | Action::Descending => {
                self.descending = action == Action::Descending;
                self.resort();
            }
            Action::Expand | Action::Collapse => {
                if let Some(row) = tree_row {
                    self.toggle_rail_row(storage, row);
                }
            }
            Action::Rename => self.begin_rename(storage),
            Action::PinQuickAccess => {
                let path = match (subject, tree_row) {
                    (Subject::TreePin, Some(row)) => {
                        let (pins, _) = self.rail_pins();
                        let rows = self.rail_rows();
                        match rows.get(row).map(|r| r.node.clone()) {
                            Some(TreeNode::Pin(i)) => pins.get(i).cloned().unwrap_or(first.clone()),
                            _ => first.clone(),
                        }
                    }
                    _ => first.clone(),
                };
                self.toggle_pin(&path, storage);
            }
            Action::Favorite => self.toggle_favorite(&first, storage),
            Action::RunAsAdministrator => self.run_as_administrator(&first, storage),
            Action::Install => self.run_checked(storage, &first)?,
            Action::Repair => self.run_installer(&first, "/f")?,
            Action::Uninstall => self.run_installer(&first, "/x")?,
            Action::AddTag => self.begin_tag(),
            Action::RemoveRecent => {
                self.recent.remove(&first);
                if self.home_view {
                    self.show_home(storage)?;
                }
            }
            Action::OpenLocation => {
                let folder = match first.rsplit_once('/') {
                    Some(("", _)) | None => String::from("/"),
                    Some((p, _)) => String::from(p),
                };
                let name = String::from(first.rsplit('/').next().unwrap_or(""));
                self.navigate(storage, folder)?;
                self.select_named(storage, &name)?;
            }
            Action::SelectAll => {
                self.select_all();
                self.inspect_selection(storage)?;
            }
            Action::SelectNone => {
                self.marked.clear();
                self.anchor = None;
                self.selected = None;
                self.inspect_selection(storage)?;
            }
            Action::InvertSelection => {
                self.invert_selection();
                self.inspect_selection(storage)?;
            }
            Action::Delete => self.delete_selection(false, storage),
            Action::Compress => self.begin_archive(storage, paths),
            Action::ExtractAll => {
                if let Some(first) = paths.first() {
                    self.begin_extract(storage, first);
                }
            }
            Action::Convert => {
                if let Some(first) = paths.first() {
                    self.open_with_key("native:10", first);
                }
            }
            Action::FilterTag(i) => self.filter_tag(i as usize),
            Action::RenameTag(i) => self.begin_tag_rename(i as usize),
            Action::DeleteTag(i) => self.delete_tag(i as usize, storage)?,
            Action::RemoveTag(i) => self.remove_tag_from_detail(i as usize, storage)?,
            Action::Cut => self.copy_selection(true),
            Action::Copy => self.copy_selection(false),
            Action::Paste => self.paste(),
            Action::CopyPath => self.copy_as_path(),
            Action::Restore => self.restore(false, storage),
            Action::RestoreAll => self.restore(true, storage),
            Action::FolderSizes => {
                self.prefs.hide_folder_sizes = !self.prefs.hide_folder_sizes;
                self.prefs_dirty = true;
            }
            Action::EmptyRecycleBin => self.ask_empty_bin(storage),
            Action::OpenWith(row) => self.open_with_row(row, &first),
            Action::ChooseApp => self.choose_app(storage, &first),
            // Provider-less commands are disabled in the model; never reached.
            Action::Share | Action::Pick(_) | Action::Submenu => (),
        }
        Ok(())
    }
    pub(super) fn resort(&mut self) {
        if !self.overview {
            self.sort();
        }
        self.remember_view();
        if let Some(position) = self.selected_position() {
            self.reveal_position(position);
        }
    }
    /// Command-bar New / Sort / View / See more dropdowns (ids 8, 5, 6, 7).
    pub(super) fn open_command_menu(&mut self, id: u32) {
        let Some((anchor, ..)) = self.command_controls().into_iter().find(|c| c.1 == id) else {
            return;
        };
        let gates = self.menu_gates(Subject::Background, &[], None);
        let (entries, width) = match id {
            8 => (cm::new_entries(), 240),
            5 => (cm::sort_entries(gates.sort, gates.descending), 200),
            6 => (cm::view_menu(gates.view, self.prefs.folder_sizes()), 300),
            _ => {
                let listing = !(self.overview && self.search.is_empty());
                let none =
                    (!listing).then_some("This PC lists drives; select one to see its properties.");
                (
                    alloc::vec![
                        {
                            let state = (!self.overview && !self.home_view)
                                .then(|| self.pinned(&self.cwd))
                                .flatten();
                            Entry::item(
                                if state.is_some() {
                                    "Unpin from Quick access"
                                } else {
                                    "Pin to Quick access"
                                },
                                Action::PinQuickAccess,
                            )
                            .gate(if self.overview || self.home_view {
                                Some(cm::PIN_FOLDERS)
                            } else if state == Some(false) {
                                Some(cm::PIN_KNOWN)
                            } else {
                                None
                            })
                        },
                        Entry::separator(),
                        Entry::item("Select all", Action::SelectAll)
                            .shortcut("Ctrl+A")
                            .glyph(Glyph::None)
                            .gate(none),
                        Entry::item("Select none", Action::SelectNone)
                            .glyph(Glyph::None)
                            .gate(none),
                        Entry::item("Invert selection", Action::InvertSelection)
                            .glyph(Glyph::None)
                            .gate(none),
                        Entry::separator(),
                        Entry::item("Refresh", Action::Refresh).shortcut("F5"),
                        Entry::item("Properties", Action::Properties).shortcut("Alt+Enter"),
                    ],
                    260,
                )
            }
        };
        self.open_dropdown(entries, anchor, width, MenuFor::Command);
    }
    pub(super) fn open_history_menu(&mut self, anchor: Bounds) {
        let entries = self
            .history
            .iter()
            .enumerate()
            .rev()
            .map(|(i, path)| {
                Entry::item(&Self::history_label(path), Action::Pick(i as u16))
                    .glyph(if path == THIS_PC {
                        Glyph::ThisPc
                    } else {
                        Glyph::Folder
                    })
                    .checked(i == self.history_index)
            })
            .collect();
        self.dismiss_menu();
        self.xmenu = Some(MenuState::open(
            cm::flyout(entries, 280),
            (anchor.x, anchor.y + anchor.height as i32 + 2),
            (self.layout.width, self.layout.height),
            false,
        ));
        self.menu_for = MenuFor::History;
    }
}

impl Files {
    pub(super) fn draw_menus(&self, v: &mut ui::View<'_>) {
        let Some(m) = self.xmenu.as_ref() else {
            return;
        };
        let root = m.root();
        let focus_row = match m.focus {
            Focus::Row(i) => Some(i),
            _ => None,
        };
        let focus_icon = match m.focus {
            Focus::Icon(i) => Some(i),
            _ => None,
        };
        self.draw_panel(
            v,
            m,
            &root,
            &m.menu.entries,
            focus_row,
            focus_icon,
            m.sub.map(|s| s.0),
        );
        if let (Some(sub), Some((parent, focus, _))) = (m.submenu(), m.sub) {
            self.draw_panel(
                v,
                m,
                &sub,
                &m.menu.entries[parent].children,
                focus,
                None,
                None,
            );
        }
        // Disabled commands explain themselves under the pointer.
        if let Some((x, y)) = v.hover
            && let Some((entry, r)) = m.entry_at(x, y)
            && !entry.enabled
            && !entry.reason.is_empty()
        {
            self.draw_tooltip(v, r, entry.reason);
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn draw_panel(
        &self,
        v: &mut ui::View<'_>,
        m: &MenuState,
        p: &Panel,
        entries: &[Entry],
        focus: Option<usize>,
        focus_icon: Option<usize>,
        open_sub: Option<usize>,
    ) {
        // Known folders, tags and application icons come from Explorer.
        let glyph = |v: &mut ui::View<'_>, glyph: Glyph, (x, y): (i32, i32), color: [u8; 4]| {
            match glyph {
                Glyph::Known(k) => {
                    v.compact_icon(x, y, FOLDERS[(k as usize).min(FOLDERS.len() - 1)].1, color)
                }
                Glyph::Tag(t) => v.rounded_asset(
                    x + 4,
                    y + 4,
                    8,
                    8,
                    4,
                    super::modern::paint::tag_color(v, t as usize),
                ),
                Glyph::App(id) => Self::app_icon(v, x, y, 16, id),
                _ => return false,
            }
            true
        };
        super::menu_paint::draw_panel(v, m, p, entries, focus, focus_icon, open_sub, &glyph);
    }
    /// Windows tooltip under (or above) the hovered command.
    pub(super) fn draw_tooltip(&self, v: &mut ui::View<'_>, anchor: Bounds, text: &str) {
        let ink = ink(v.palette);
        let width = (v.measure_small(text) + 18).min(v.width.saturating_sub(16).max(40));
        // One 12 px line: the tip grows with its line box.
        let height = door_workspace::palette::text_grow_i(28, TextStyle::Regular12);
        let x = (anchor.x + 12)
            .min(v.width as i32 - width as i32 - 8)
            .max(4);
        let below = anchor.y + anchor.height as i32 + 6;
        let y = if below + height <= v.height as i32 - 4 {
            below
        } else {
            (anchor.y - height - 6).max(4)
        };
        v.elevation(x, y + 2, width, height as u32, 50);
        v.rounded_asset(x, y, width, height as u32, 4, ink.border);
        v.rounded_asset(x + 1, y + 1, width - 2, height as u32 - 2, 3, ink.panel);
        v.text_in_box(
            (x + 9, y, width - 18, height as u32),
            text,
            TextStyle::Regular12,
            ink.text,
        );
    }
}
impl Files {
    /// Why a command-bar button is unavailable, if it is.
    pub(super) fn command_reason(&self, id: u32) -> Option<&'static str> {
        match id {
            44 if self.recycle_view() => Some("Restore the item to move it."),
            44 => self.delete_block(),
            45 => self.copy_block(),
            46 => self.paste_block(),
            48 if self.recycle_view() => (self.selection_count() == 0).then_some(cm::SELECT_FIRST),
            48 => self.delete_block(),
            49 | 50 => (self.entries.is_empty()).then_some(cm::BIN_EMPTY),
            5 if self.home_view => Some("Home keeps each section's own order."),
            47 => Some(cm::NO_SHARE),
            33 if self.selection_count() > 1 => Some(cm::MULTI_RENAME),
            33 if self.selected_position().is_none()
                || (self.overview && self.search.is_empty()) =>
            {
                Some(cm::SELECT_FIRST)
            }
            33 => self.rename_block(),
            _ => None,
        }
    }
    /// Tooltip for a hovered, disabled command-bar button.
    pub(super) fn draw_command_tooltip(&self, v: &mut ui::View<'_>) {
        if self.xmenu.is_some() || self.modal() {
            return;
        }
        let Some((x, y)) = v.hover else {
            return;
        };
        for (r, id, _, _, enabled) in self.command_controls() {
            if !enabled
                && r.contains(x, y)
                && let Some(reason) = self.command_reason(id)
            {
                self.draw_tooltip(v, r, reason);
            }
        }
    }
}
