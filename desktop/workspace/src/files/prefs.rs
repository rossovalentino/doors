//! Windows 11 26H2 folder memory: each folder reopens with the view and sort
//! it was left in, and Explorer remembers which sections were collapsed
//! (This PC's "Devices and drives" group, navigation-tree branches). Kept per
//! session in memory and saved, coalesced on the next tick, to the profile
//! file `.door-explorer-views` (bounded; a damaged file is ignored, never
//! overwritten with less information than it had).
use super::*;
use door_workspace::explorer_layout::ViewMode;

pub(super) const FILE: &str = ".door-explorer-views";
const MAGIC: &str = "DOORVIEW1";
const MAX_FOLDERS: usize = 64;
const MAX_TREE: usize = 64;
pub(super) const MAX_PINS: usize = 16;
pub(super) const MAX_FAVORITES: usize = 64;
const MAX_BYTES: usize = 24 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FolderView {
    pub(super) view: ViewMode,
    pub(super) sort: SortKey,
    pub(super) descending: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Prefs {
    /// Most recently changed last.
    pub(super) folders: Vec<(String, FolderView)>,
    pub(super) drives_collapsed: bool,
    /// Expanded navigation-tree row keys.
    pub(super) tree: Vec<String>,
    /// Home > Quick access folders pinned by the user (known folders are
    /// always pinned).
    pub(super) pins: Vec<String>,
    /// Home > Favorites.
    pub(super) favorites: Vec<String>,
    /// Collapsed Home sections (bit per section).
    pub(super) home_collapsed: u8,
    /// View > Show > Folder sizes switched off. Doors totals folders in
    /// Details' Size column by default (Windows leaves it blank).
    pub(super) hide_folder_sizes: bool,
    /// Navigation pane width the splitter was left at (None: `SIDEBAR`).
    pub(super) sidebar: Option<i32>,
}
impl Prefs {
    /// Details' Size column totals folders (View > Show > Folder sizes).
    pub(super) fn folder_sizes(&self) -> bool {
        !self.hide_folder_sizes
    }
    pub(super) fn get(&self, path: &str) -> Option<FolderView> {
        self.folders
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, v)| *v)
    }
    pub(super) fn set(&mut self, path: &str, view: FolderView) {
        self.folders.retain(|(p, _)| p != path);
        self.folders.push((path.into(), view));
        if self.folders.len() > MAX_FOLDERS {
            self.folders.remove(0);
        }
    }
    pub(super) fn encode(&self) -> Vec<u8> {
        let mut out = format!("{MAGIC}\n");
        out.push_str(&format!("drives {}\n", u8::from(self.drives_collapsed)));
        for key in self.tree.iter().take(MAX_TREE) {
            out.push_str(&format!("tree {key}\n"));
        }
        out.push_str(&format!("home {}\n", self.home_collapsed));
        if self.hide_folder_sizes {
            out.push_str("foldersizes 0\n");
        }
        if let Some(width) = self.sidebar {
            out.push_str(&format!("sidebar {width}\n"));
        }
        for path in self.pins.iter().take(MAX_PINS) {
            out.push_str(&format!("pin {path}\n"));
        }
        for path in self.favorites.iter().take(MAX_FAVORITES) {
            out.push_str(&format!("fav {path}\n"));
        }
        for (path, v) in &self.folders {
            out.push_str(&format!(
                "view {} {} {} {path}\n",
                v.view.index(),
                v.sort.persisted(),
                u8::from(v.descending)
            ));
        }
        out.into_bytes()
    }
    pub(super) fn decode(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.is_empty() {
            return Ok(Self::default());
        }
        if bytes.len() > MAX_BYTES {
            return Err("view preferences exceed their limit");
        }
        let text = core::str::from_utf8(bytes).map_err(|_| "view preferences are not text")?;
        let mut lines = text.lines();
        if lines.next() != Some(MAGIC) {
            return Err("unknown view preferences format");
        }
        let mut prefs = Self::default();
        for line in lines {
            let (kind, rest) = line.split_once(' ').ok_or("malformed view preference")?;
            match kind {
                "drives" => prefs.drives_collapsed = rest == "1",
                "foldersizes" => prefs.hide_folder_sizes = rest == "0",
                "sidebar" => {
                    prefs.sidebar = Some(
                        rest.parse::<i32>()
                            .map_err(|_| "malformed navigation pane width")?
                            .clamp(door_workspace::explorer_layout::SIDEBAR_MIN, 600),
                    )
                }
                "tree" if !rest.is_empty() && prefs.tree.len() < MAX_TREE => {
                    prefs.tree.push(rest.into())
                }
                "home" => {
                    prefs.home_collapsed =
                        rest.parse::<u8>().map_err(|_| "malformed home sections")? & 7
                }
                "pin" if rest.starts_with('/') => {
                    if prefs.pins.len() < MAX_PINS && !prefs.pins.iter().any(|p| p == rest) {
                        prefs.pins.push(rest.into());
                    }
                }
                "fav" if rest.starts_with('/') => {
                    if prefs.favorites.len() < MAX_FAVORITES
                        && !prefs.favorites.iter().any(|p| p == rest)
                    {
                        prefs.favorites.push(rest.into());
                    }
                }
                "view" => {
                    let mut f = rest.splitn(4, ' ');
                    let view = f
                        .next()
                        .and_then(|v| v.parse().ok())
                        .and_then(ViewMode::from_index);
                    let sort = f
                        .next()
                        .and_then(|v| v.parse::<usize>().ok())
                        .and_then(SortKey::from_persisted);
                    let descending = f.next().map(|v| v == "1");
                    let path = f
                        .next()
                        .filter(|p| p.starts_with('/') || *p == THIS_PC || *p == HOME_PAGE);
                    match (view, sort, descending, path) {
                        (Some(view), Some(sort), Some(descending), Some(path)) => prefs.set(
                            path,
                            FolderView {
                                view,
                                sort,
                                descending,
                            },
                        ),
                        _ => return Err("malformed folder view"),
                    }
                }
                _ => return Err("unknown view preference"),
            }
        }
        Ok(prefs)
    }
}

impl Files {
    /// Location key of the current view (This PC has its own).
    fn view_key(&self) -> String {
        if self.overview && self.search.is_empty() {
            THIS_PC.into()
        } else if self.home_view {
            HOME_PAGE.into()
        } else {
            self.cwd.clone()
        }
    }
    /// Records the current view and sort for this folder.
    pub(super) fn remember_view(&mut self) {
        if !self.search.is_empty() && !self.overview && !self.home_view {
            return;
        }
        let key = self.view_key();
        let v = FolderView {
            view: self.view_mode(),
            sort: self.sort_key,
            descending: self.descending,
        };
        if self.prefs.get(&key) != Some(v) {
            self.prefs.set(&key, v);
            self.prefs_dirty = true;
        }
    }
    /// Applies a folder's remembered view and sort, if it has one; folders
    /// never changed keep the tab's current choice.
    pub(super) fn recall_view(&mut self, key: &str) {
        if let Some(v) = self.prefs.get(key) {
            if key == THIS_PC {
                self.overview_view = v.view;
            } else {
                self.view = v.view;
            }
            self.sort_key = v.sort;
            self.descending = v.descending;
        }
    }
    pub(super) fn remember_sections(&mut self) {
        let tree = self.tree.expanded_keys();
        if self.prefs.tree != tree || self.prefs.drives_collapsed == self.drives_expanded {
            self.prefs.tree = tree;
            self.prefs.drives_collapsed = !self.drives_expanded;
            self.prefs_dirty = true;
        }
    }
    /// Reads the context-menu profile when its file appeared, changed or
    /// went away (the customizer saves it while Explorer runs). Returns
    /// whether it was re-read.
    pub(super) fn load_menu_profile(&mut self, storage: &mut Storage) -> bool {
        use door_workspace::context_menu_profile::{FILE as PROFILE, MAX_BYTES as LIMIT, Profile};
        let path = format!("{}/{PROFILE}", self.home);
        let info = match storage.metadata(&path) {
            Ok(info) => Some(info),
            Err(Error::Service(Code::NotFound)) => None,
            Err(_) => return false,
        };
        if self.menu_profile_info == Some(info) {
            return false;
        }
        self.menu_profile_info = Some(info);
        self.menu_profile = info
            .and_then(|_| {
                let file = storage
                    .open(&path, Disposition::OpenExisting, READ, SHARE_ALL)
                    .ok()?;
                let bytes = Self::read_small(storage, &file, LIMIT);
                let _ = storage.close(file);
                Profile::decode(&bytes.ok()?).ok()
            })
            .unwrap_or_default();
        true
    }
    pub(super) fn load_prefs(&mut self, storage: &mut Storage) {
        let path = format!("{}/{FILE}", self.home);
        let loaded = (|| {
            let file = match storage.open(&path, Disposition::OpenExisting, READ, SHARE_ALL) {
                Ok(file) => file,
                Err(Error::Service(Code::NotFound)) => return Ok(Prefs::default()),
                Err(e) => return Err(e),
            };
            let bytes = Self::read_small(storage, &file, MAX_BYTES);
            let closed = storage.close(file);
            let prefs = Prefs::decode(&bytes?).map_err(|_| Error::Malformed)?;
            closed?;
            Ok(prefs)
        })();
        match loaded {
            Ok(prefs) => {
                self.tree.set_expanded(&prefs.tree);
                self.drives_expanded = !prefs.drives_collapsed;
                if let Some(width) = prefs.sidebar {
                    self.layout.sidebar = width;
                }
                self.prefs = prefs;
                self.prefs_writable = true;
            }
            // A damaged or unreadable file stays as it is; changes are kept
            // for this session only.
            Err(_) => self.prefs_writable = false,
        }
    }
    fn read_small(
        storage: &mut Storage,
        file: &door_service_client::File,
        limit: usize,
    ) -> Result<Vec<u8>, Error> {
        let info = storage.info(file)?;
        if info.bytes > limit as u64 {
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
        Ok(bytes)
    }
    /// Saves changed preferences (coalesced per tick). Another window's saved
    /// folders are merged in; this window's changes win.
    pub(super) fn save_prefs(&mut self, storage: &mut Storage) -> bool {
        if !self.prefs_dirty || !self.prefs_writable || self.modal() {
            return false;
        }
        self.prefs_dirty = false;
        let path = format!("{}/{FILE}", self.home);
        let result = (|| {
            let file = storage.open(
                &path,
                Disposition::OpenAlways,
                READ | door_service_client::WRITE,
                SHARE_ALL,
            )?;
            let saved = (|| {
                let info = storage.info(&file)?;
                let bytes = Self::read_small(storage, &file, MAX_BYTES)?;
                let mut merged = Prefs::decode(&bytes).map_err(|_| Error::Malformed)?;
                for (p, v) in &self.prefs.folders {
                    merged.set(p, *v);
                }
                merged.tree = self.prefs.tree.clone();
                merged.drives_collapsed = self.prefs.drives_collapsed;
                merged.pins = self.prefs.pins.clone();
                merged.favorites = self.prefs.favorites.clone();
                merged.home_collapsed = self.prefs.home_collapsed;
                storage.replace_checked(&file, info, &merged.encode())?;
                Ok::<_, Error>(merged)
            })();
            let closed = storage.close(file);
            let merged = saved?;
            closed?;
            Ok::<_, Error>(merged)
        })();
        match result {
            Ok(merged) => {
                self.prefs.folders = merged.folders;
                door_rt::diagnostic(format_args!(
                    "DOOR_P12 FILES VIEWS SAVED folders={}\n",
                    self.prefs.folders.len()
                ));
            }
            Err(error) => {
                self.prefs_writable = false;
                door_rt::diagnostic(format_args!(
                    "DOOR_P12 FILES VIEWS UNSAVED error={error:?}\n"
                ));
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folder_views_and_sections_roundtrip_and_stay_bounded() {
        let mut p = Prefs {
            drives_collapsed: true,
            hide_folder_sizes: true,
            sidebar: Some(212),
            tree: alloc::vec!["pc".into(), "pc/d:0/Users".into()],
            ..Prefs::default()
        };
        p.set(
            "/users/1001/Pictures",
            FolderView {
                view: ViewMode::Large,
                sort: SortKey::Size,
                descending: true,
            },
        );
        p.set(
            THIS_PC,
            FolderView {
                view: ViewMode::Tiles,
                sort: SortKey::Name,
                descending: false,
            },
        );
        p.set(
            "/users/1001/My folder",
            FolderView {
                view: ViewMode::List,
                sort: SortKey::Type,
                descending: false,
            },
        );
        let back = Prefs::decode(&p.encode()).unwrap();
        assert_eq!(back, p);
        assert_eq!(
            back.get("/users/1001/My folder").unwrap().view,
            ViewMode::List
        );
        for i in 0..100 {
            p.set(
                &format!("/f{i}"),
                FolderView {
                    view: ViewMode::Details,
                    sort: SortKey::Name,
                    descending: false,
                },
            );
        }
        assert_eq!(p.folders.len(), MAX_FOLDERS);
        assert!(p.get("/f99").is_some() && p.get("/f0").is_none());
        assert!(p.encode().len() <= MAX_BYTES);
        for bad in [
            &b"DOORVIEW2\n"[..],
            b"DOORVIEW1\nview 9 0 0 /x\n",
            b"DOORVIEW1\nview 1 0 0 relative\n",
            b"DOORVIEW1\nnope\n",
        ] {
            assert!(Prefs::decode(bad).is_err());
        }
        assert_eq!(Prefs::decode(b"").unwrap(), Prefs::default());
    }
}
