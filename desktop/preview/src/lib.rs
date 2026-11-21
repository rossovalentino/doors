#![no_std]
//! Standalone bounded native viewer. File bytes are authenticated and checked
//! again after capture; input files are never overwritten by view transforms.
extern crate alloc;
mod editing;
mod formats;
mod player;
mod presentation;
// door.accessibility tree (shared adapter `door_workspace::semantics`).
mod semantics;
use alloc::{format, string::String, vec::Vec};
use door_app_ui::{Application, View, icons::Icon};
use door_graphics_text::TextStyle;
use door_service_client::{Disposition, Error, File, READ, SHARE_ALL, Storage, WRITE};
use door_service_protocol::{Code, files::FileInfo};
use door_workspace::{
    archive_codec::{self as packing, Archive, Fault},
    converter::{self, Target},
    explorer_preview::{self as codec, Image, Kind},
    line_edit::LineEdit,
};
pub use formats::png as export_png;
const IMAGE_BUDGET: usize = 16 * 1024 * 1024;
/// One entry to pack: its path and data (None: a folder).
type Entry = (String, Option<Vec<u8>>);
/// The most one archive entry (or a .tar.gz) may expand to.
const MAX_UNPACKED: usize = 64 * 1024 * 1024;
/// What converting an archive may hold in memory at once (all entries),
/// and what extracting one may write in total: decompression bombs stop.
const MAX_REPACK: usize = 128 * 1024 * 1024;
const MAX_EXTRACT: u64 = 1024 * 1024 * 1024;
/// Why an archive entry could not be read, for the status line.
fn fault(f: Fault) -> String {
    String::from(match f {
        Fault::Password => "The password is missing or wrong.",
        Fault::Damaged => "The archive is damaged.",
        Fault::Method => "An entry uses a compression method Doors can't read.",
        Fault::TooLarge => "An entry is too large to extract.",
    })
}
#[cfg(test)]
fn randomness(bytes: usize) -> Result<Vec<u8>, String> {
    // Host tests have no kernel entropy source: a fixed pattern.
    Ok((0..bytes).map(|i| (i * 37 + 11) as u8).collect())
}
#[cfg(not(test))]
/// `bytes` of fresh randomness from the kernel's entropy source, read
/// before anything is encrypted: a protected archive is never written with
/// a fixed salt or IV when the source fails (spec section 75).
fn randomness(bytes: usize) -> Result<Vec<u8>, String> {
    let mut out = alloc::vec![0u8; bytes];
    for chunk in out.chunks_mut(door_abi::entropy::READ_MAX) {
        if door_sys::entropy_read(chunk) != Ok(chunk.len()) {
            return Err("The system's random source isn't available, so the archive can't be password protected.".into());
        }
    }
    Ok(out)
}
const FG: [u8; 4] = [24, 24, 24, 255];
const MUTED: [u8; 4] = [112, 112, 112, 255];
const ACCENT: [u8; 4] = [196, 103, 0, 255];
struct Tab {
    id: u64,
    zoom: u32,
    fit: bool,
    path: String,
    info: FileInfo,
    image: Option<Image>,
    backup: Option<Image>,
    text: String,
    error: String,
    scroll: usize,
    details: Vec<(String, String)>,
    /// The file as read (packing into archives and archive conversion).
    raw: Vec<u8>,
    /// Decoded audio (WAV, FLAC, MP3): playback and audio conversion.
    audio: Option<door_workspace::audio_codec::Pcm>,
}
struct Exporting {
    path: String,
    staging: String,
    file: File,
    parent: u64,
    bytes: Vec<u8>,
    written: usize,
}
struct Reading {
    path: String,
    file: File,
    info: FileInfo,
    bytes: Vec<u8>,
}
pub struct Preview {
    cropping: bool,
    crop_drag: bool,
    crop: Option<(i32, i32, i32, i32)>,
    uid: u32,
    /// The service registry (door.audio for playback); 0 in tests.
    registry: u64,
    /// The playing (or paused) audio, and the last pointer press.
    player: Option<player::Player>,
    press: (i32, i32),
    width: u32,
    height: u32,
    info_page: usize,
    home: String,
    path: LineEdit,
    /// The export dialog's password box (protected ZIP, encrypted archives)
    /// and whether it has the keyboard instead of the path.
    password: LineEdit,
    password_focus: bool,
    /// The open export needs the password box (a protected ZIP, or an
    /// archive with encrypted entries to extract or repack).
    password_needed: bool,
    opening: bool,
    exporting: bool,
    output: Option<Exporting>,
    tabs: Vec<Tab>,
    active: usize,
    job: Option<Reading>,
    message: String,
    export_uncertainty: Option<String>,
    notice_open: bool,
    notice_page: usize,
    notice_pages: core::cell::Cell<usize>,
    zoom: u32,
    fit: bool,
    section: u32,
    recent: Vec<String>,
    format: u32,
    info_open: bool,
    /// Navigation-pane scroll in logical pixels, kept across destinations.
    nav_scroll: u32,
    /// Thumb grab offset while the navigation-pane thumb is dragged.
    nav_grab: Option<i32>,
    library_open: bool,
    tools_open: bool,
    section_scroll: usize,
    focused: bool,
    caret: bool,
    /// The home tab's close button asks the runtime to close the window.
    close_requested: bool,
}
impl Preview {
    /// Show an in-memory image as the active tab, as a completed read does
    /// (host reviews; the runtime path reads through Storage).
    #[doc(hidden)]
    pub fn open_image_for_review(&mut self, path: &str, bytes: &[u8]) -> bool {
        let Ok(image) = codec::decode_full(bytes) else {
            return false;
        };
        self.tabs.push(Tab {
            id: door_workspace::document_tabs::next_id(),
            zoom: 100,
            fit: true,
            path: path.into(),
            info: FileInfo {
                attributes: 0,
                object: 1,
                version: 1,
                bytes: bytes.len() as u64,
                directory: false,
            },
            image: Some(image),
            backup: None,
            text: String::new(),
            error: String::new(),
            scroll: 0,
            details: Vec::new(),
            raw: Vec::new(),
            audio: None,
        });
        self.active = self.tabs.len() - 1;
        true
    }
    /// Show a navigation destination, as its row does (host reviews).
    #[doc(hidden)]
    pub fn show_section(&mut self, section: u32) {
        self.select_section(section);
    }
    /// Give Preview the session's service registry (playback).
    pub fn with_registry(mut self, registry: u64) -> Self {
        self.registry = registry;
        self
    }
    pub fn new(uid: u32) -> Self {
        let home = format!("/users/{uid}");
        let mut path = LineEdit::default();
        path.set(home.clone(), false);
        Self {
            cropping: false,
            crop_drag: false,
            crop: None,
            uid,
            registry: 0,
            player: None,
            press: (0, 0),
            width: 1280,
            height: 800,
            info_page: 0,
            home,
            path,
            password: LineEdit::default(),
            password_focus: false,
            password_needed: false,
            opening: false,
            exporting: false,
            output: None,
            tabs: Vec::new(),
            active: 0,
            job: None,
            message: String::new(),
            export_uncertainty: None,
            notice_open: false,
            notice_page: 0,
            notice_pages: core::cell::Cell::new(1),
            zoom: 100,
            fit: true,
            section: 0,
            recent: Vec::new(),
            format: 0,
            info_open: true,
            nav_scroll: 0,
            nav_grab: None,
            library_open: true,
            tools_open: true,
            section_scroll: 0,
            focused: true,
            caret: true,
            close_requested: false,
        }
    }
    fn save_view(&mut self) {
        if let Some(t) = self.tabs.get_mut(self.active) {
            t.zoom = self.zoom;
            t.fit = self.fit;
        }
    }
    fn restore_view(&mut self) {
        if let Some(t) = self.tabs.get(self.active) {
            self.zoom = t.zoom;
            self.fit = t.fit;
        } else {
            self.zoom = 100;
            self.fit = true;
        }
    }
    fn activate(&mut self, index: usize) {
        self.cancel_crop();
        self.save_view();
        self.active = index.min(self.tabs.len().saturating_sub(1));
        self.restore_view();
    }
    fn move_tab_to(&mut self, target: &mut Self, index: usize) -> Result<bool, Error> {
        if index >= self.tabs.len()
            || target.tabs.len() >= 8
            || self.busy()
            || target.busy()
            || self.modal()
            || target.modal()
        {
            return Ok(false);
        }
        let bytes = self.tabs[index].image.as_ref().map_or(0, |i| i.bgra.len())
            + self.tabs[index].backup.as_ref().map_or(0, |i| i.bgra.len());
        if target.image_bytes().saturating_add(bytes) > IMAGE_BUDGET {
            return Ok(false);
        }
        target
            .tabs
            .try_reserve(1)
            .map_err(|_| Error::Service(Code::Resources))?;
        self.save_view();
        target.save_view();
        let tab = self.tabs.remove(index);
        if self.active > index {
            self.active -= 1;
        }
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
        target.tabs.push(tab);
        target.active = target.tabs.len() - 1;
        self.restore_view();
        target.restore_view();
        target.section = 0;
        Ok(true)
    }
    fn selected(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }
    fn notice_rect(&self) -> (i32, i32, u32, u32) {
        let w = self
            .width
            .saturating_sub(if self.width < 400 { 16 } else { 48 })
            .min(680);
        let h = self
            .height
            .saturating_sub(if self.height < 240 { 8 } else { 48 })
            .min(400);
        (
            ((self.width - w) / 2) as i32,
            ((self.height - h) / 2) as i32,
            w,
            h,
        )
    }
    fn notice_button(&self, index: u32) -> (i32, i32, u32, u32) {
        let (left, top, width, height) = self.notice_rect();
        let cell = width.saturating_sub(24) / 3;
        let h = (26 + presentation::grow(TextStyle::Regular12)) as u32;
        (
            left + 8 + (index * (cell + 4)) as i32,
            top + height as i32 - 6 - h as i32,
            cell,
            h,
        )
    }
    fn page_notice(&mut self, forward: bool) {
        let last = self.notice_pages.get().saturating_sub(1);
        let page = self.notice_page.min(last);
        self.notice_page = if forward {
            page.saturating_add(1).min(last)
        } else {
            page.saturating_sub(1)
        };
    }
    fn notice_lines<'a>(&'a self, v: &View<'_>, width: u32) -> Vec<&'a str> {
        let mut remaining = self.export_uncertainty.as_deref().unwrap_or("");
        let mut lines = Vec::new();
        while !remaining.is_empty() {
            let mut end = 0;
            for (i, c) in remaining.char_indices() {
                let next = i + c.len_utf8();
                if end != 0 && v.measure_small(&remaining[..next]) > width {
                    break;
                }
                end = next;
            }
            lines.push(&remaining[..end]);
            remaining = &remaining[end..];
        }
        lines
    }
    fn export_failed(&mut self, error: Error, path: &str, staging: &str) {
        self.message = format!(
            "Export not confirmed ({error:?}). Inspect destination {path} and staging {staging}; do not retry automatically."
        );
        self.export_uncertainty = Some(self.message.clone());
        self.notice_open = true;
        self.notice_page = 0;
    }
    fn create_output<T>(
        &mut self,
        path: &str,
        staging: &str,
        create: impl FnOnce() -> Result<T, Error>,
    ) -> Result<T, Error> {
        // Record intent before the first mutating request. A lost CreateNew
        // response may already have published the staging binding.
        self.export_uncertainty = Some(format!(
            "Export creation not confirmed. Inspect destination {path} and staging {staging}; do not retry automatically."
        ));
        match create() {
            Ok(file) => {
                self.export_uncertainty = None;
                self.notice_open = false;
                Ok(file)
            }
            Err(e) => {
                self.export_failed(e, path, staging);
                Err(e)
            }
        }
    }
    fn cancelled_output(&mut self, staging: &str, close: impl FnOnce() -> Result<(), Error>) {
        self.notice_open = true;
        self.notice_page = 0;
        self.message = format!(
            "Export cancelled before publication. Inspect incomplete staging file {staging}; do not retry automatically."
        );
        self.export_uncertainty = Some(self.message.clone());
        if let Err(e) = close() {
            self.message = format!("{} Close not confirmed ({e:?}).", self.message);
            self.export_uncertainty = Some(self.message.clone());
        }
    }
    fn info_rows(&self, tab: &Tab) -> Vec<(String, String)> {
        let mut rows = alloc::vec![
            ("Type".into(), Kind::of(&tab.path, false).label().into()),
            ("Size".into(), codec::size(tab.info.bytes))
        ];
        if let Some(image) = &tab.image {
            rows.push((
                if Kind::of(&tab.path, false) == Kind::Image {
                    "Dimensions"
                } else {
                    "Preview size"
                }
                .into(),
                format!("{} x {}", image.width, image.height),
            ));
        }
        rows.extend(tab.details.iter().cloned());
        rows
    }
    /// Information rows per page: the pane's layout when it is shown.
    fn info_visible(&self) -> usize {
        self.panel_layout_count()
            .unwrap_or_else(|| self.info_threshold())
    }
    fn info_threshold(&self) -> usize {
        if self.height >= 820 {
            4
        } else if self.height >= 650 {
            3
        } else {
            2
        }
    }
    fn info_pages(&self) -> usize {
        self.selected().map_or(1, |tab| {
            self.info_rows(tab)
                .len()
                .div_ceil(self.info_visible())
                .max(1)
        })
    }

    fn cancel(&mut self, storage: &mut Storage) {
        if let Some(job) = self.job.take() {
            let _ = storage.close(job.file);
        }
        if let Some(job) = self.output.take() {
            self.cancelled_output(&job.staging, || storage.close(job.file));
        }
    }
    fn resolve(
        &self,
        storage: &mut Storage,
        input: &str,
        missing_leaf: bool,
    ) -> Result<String, Error> {
        let plan = door_workspace::explorer_path::parse(input, &self.home, &self.home)
            .map_err(|_| Error::Service(Code::InvalidRequest))?;
        let door_workspace::explorer_path::Address::Directory {
            start,
            parts,
            windows,
        } = plan
        else {
            return Err(Error::Service(Code::InvalidRequest));
        };
        let mut path = start;
        let last = parts.len().saturating_sub(1);
        for (index, part) in parts.into_iter().enumerate() {
            if !storage.metadata(&path)?.directory {
                return Err(Error::Service(Code::InvalidRequest));
            }
            if part == "." {
                continue;
            }
            if part == ".." {
                if path == "/" {
                    return Err(Error::Service(Code::InvalidRequest));
                }
                path = door_workspace::explorer_path::parent(&path, windows);
            } else if windows && path == "/compat" && part.eq_ignore_ascii_case("Users") {
                path = "/users".into();
            } else {
                let name = if windows {
                    let mut after = 0;
                    let mut found = None;
                    for _ in 0..512 {
                        let Some(entry) = storage.list(&path, after)? else {
                            break;
                        };
                        if entry.info.object <= after {
                            return Err(Error::Malformed);
                        }
                        after = entry.info.object;
                        if let Ok(name) = String::from_utf16(&entry.name[..entry.name_len])
                            && name.eq_ignore_ascii_case(&part)
                        {
                            found = Some(name);
                            break;
                        }
                    }
                    match found {
                        Some(name) => name,
                        None if missing_leaf && index == last => part,
                        None => return Err(Error::Service(Code::NotFound)),
                    }
                } else {
                    part
                };
                path = if path == "/" {
                    format!("/{name}")
                } else {
                    format!("{path}/{name}")
                };
            }
        }
        Ok(path)
    }
    /// What the selected file converts to (`converter::targets`); images
    /// and text need their decoded content, packing needs the file's bytes.
    pub(crate) fn convert_targets(&self) -> Vec<Target> {
        let Some(tab) = self.selected() else {
            return Vec::new();
        };
        converter::targets(Kind::of(&tab.path, false), &tab.path)
            .into_iter()
            .filter(|t| match t {
                Target::Png
                | Target::Jpeg
                | Target::Bmp
                | Target::Qoi
                | Target::Tga
                | Target::Ppm
                | Target::Ico
                | Target::Gif => tab.image.is_some(),
                Target::Text | Target::TextUtf16 | Target::Markdown | Target::Html => {
                    !tab.text.is_empty()
                }
                Target::Wav | Target::Flac => tab.audio.is_some(),
                _ => !tab.raw.is_empty(),
            })
            .collect()
    }
    /// The target the Convert list shows.
    pub(crate) fn convert_target(&self) -> Option<Target> {
        let targets = self.convert_targets();
        targets
            .get(self.format as usize % targets.len().max(1))
            .copied()
    }
    /// The selected archive opened from its bytes (None for other files).
    fn source_archive(&self) -> Option<Archive> {
        let tab = self.selected()?;
        (Kind::of(&tab.path, false) == Kind::Archive)
            .then(|| {
                let password = Some(self.password.text()).filter(|p| !p.is_empty());
                Archive::open_with(&tab.path, tab.raw.clone(), MAX_UNPACKED, password).ok()
            })
            .flatten()
    }
    fn begin_export(&mut self) {
        if !self.can_export() {
            self.message = "This file has no conversion for the chosen format.".into();
            return;
        }
        let (Some(tab), Some(target)) = (self.selected(), self.convert_target()) else {
            return;
        };
        let lower = tab.path.to_ascii_lowercase();
        let base = [".tar.gz", ".tgz"]
            .iter()
            .find(|e| lower.ends_with(*e))
            .map(|e| &tab.path[..tab.path.len() - e.len()])
            .or_else(|| tab.path.rsplit_once('.').map(|(b, _)| b))
            .unwrap_or(tab.path.as_str());
        let ext = target.extension();
        let path = if target == Target::Folder {
            String::from(base)
        } else if lower.ends_with(&format!(".{ext}")) {
            if target.needs_password() {
                format!("{base}-protected.{ext}")
            } else {
                format!("{base}-converted.{ext}")
            }
        } else {
            format!("{base}.{ext}")
        };
        let encrypted = matches!(
            target,
            Target::Folder
                | Target::Zip
                | Target::ZipProtected
                | Target::SevenZip
                | Target::SevenZipProtected
                | Target::Tar
                | Target::TarGz
        ) && self
            .source_archive()
            .is_some_and(|a| a.items.iter().any(|i| i.encrypted));
        self.password_needed = target.needs_password() || encrypted;
        self.password.set(String::new(), false);
        self.password_focus = false;
        self.path.set(path, true);
        self.opening = true;
        self.exporting = true;
    }
    /// The file's entries for packing: an archive's own entries (decrypted
    /// with the password box), else the file itself.
    fn entries(&self) -> Result<Vec<Entry>, String> {
        let tab = self.selected().ok_or("No file is open")?;
        let password = Some(self.password.text()).filter(|p| !p.is_empty());
        match self.source_archive() {
            Some(archive) => {
                let mut total = 0usize;
                (0..archive.items.len())
                    .map(|i| {
                        let item = &archive.items[i];
                        if item.folder {
                            return Ok((item.path.clone(), None));
                        }
                        let data = archive.read(i, password, MAX_UNPACKED).map_err(fault)?;
                        total += data.len();
                        if total > MAX_REPACK {
                            return Err(format!(
                                "The archive holds more than {} MB, too much to convert at once.",
                                MAX_REPACK >> 20
                            ));
                        }
                        Ok((item.path.clone(), Some(data)))
                    })
                    .collect()
            }
            None => Ok(alloc::vec![(
                String::from(tab.path.rsplit('/').next().unwrap_or("file")),
                Some(tab.raw.clone())
            )]),
        }
    }
    fn pack(&self, target: Target) -> Result<Vec<u8>, String> {
        let entries = self.entries()?;
        match target {
            Target::Zip | Target::ZipProtected => {
                let password = (target == Target::ZipProtected).then(|| self.password.text());
                if password.is_some_and(|p| p.chars().count() < 4) {
                    return Err("Enter a password of at least 4 characters.".into());
                }
                let pool = if password.is_some() {
                    randomness(entries.len() * packing::SALT_BYTES)?
                } else {
                    Vec::new()
                };
                let next = core::cell::Cell::new(0usize);
                let salt = || {
                    let mut s = [0u8; packing::SALT_BYTES];
                    let at = next.get();
                    if let Some(bytes) = pool.get(at..at + packing::SALT_BYTES) {
                        s.copy_from_slice(bytes);
                    }
                    next.set(at + packing::SALT_BYTES);
                    s
                };
                let mut zip = packing::ZipWriter::new(password);
                for (path, data) in &entries {
                    zip.add(path, data.as_deref(), 6, salt);
                }
                Ok(zip.finish())
            }
            Target::SevenZip | Target::SevenZipProtected => {
                let password = (target == Target::SevenZipProtected).then(|| self.password.text());
                if password.is_some_and(|p| p.chars().count() < 4) {
                    return Err("Enter a password of at least 4 characters.".into());
                }
                // The writer asks for at most four values (salt and IV for
                // the data and for the encrypted header).
                let pool = if password.is_some() {
                    randomness(64)?
                } else {
                    alloc::vec![0; 64]
                };
                let next = core::cell::Cell::new(0usize);
                let random = || {
                    let mut b = [0u8; 16];
                    let at = next.get() % 64;
                    b.copy_from_slice(&pool[at..at + 16]);
                    next.set(at + 16);
                    b
                };
                Ok(door_workspace::sevenzip::write(&entries, password, random))
            }
            Target::Tar | Target::TarGz => {
                let mut tar = Vec::new();
                for (path, data) in &entries {
                    let size = data.as_ref().map_or(0, Vec::len) as u64;
                    let header = packing::tar_header(path, size, 0, data.is_none())
                        .ok_or("A name is too long for a TAR archive.")?;
                    tar.extend_from_slice(&header);
                    if let Some(data) = data {
                        tar.extend_from_slice(data);
                        tar.resize(tar.len().next_multiple_of(512), 0);
                    }
                }
                tar.resize(tar.len() + 1024, 0);
                Ok(if target == Target::TarGz {
                    let tab = self.selected().ok_or("No file is open")?;
                    let stem = tab.path.rsplit('/').next().unwrap_or("archive");
                    let stem = stem.rsplit_once('.').map_or(stem, |(s, _)| s);
                    packing::gzip(Some(&format!("{stem}.tar")), &tar, 6)
                } else {
                    tar
                })
            }
            _ => Err("Not an archive format".into()),
        }
    }
    /// Extract every entry into the new folder `folder` (written at once;
    /// archives are bounded by the 6 MB read limit).
    fn extract_to(&mut self, storage: &mut Storage, folder: &str) -> Result<(), Error> {
        let Some(archive) = self.source_archive() else {
            return Err(Error::Service(Code::Unsupported));
        };
        let password = Some(String::from(self.password.text())).filter(|p| !p.is_empty());
        // Check the password on the first encrypted entry before writing.
        if let Some(i) = archive.items.iter().position(|i| i.encrypted && !i.folder)
            && let Err(f) = archive.read(i, password.as_deref(), MAX_UNPACKED)
        {
            self.message = fault(f);
            return Ok(());
        }
        match storage.metadata(folder) {
            Err(Error::Service(Code::NotFound)) => (),
            Ok(_) => return Err(Error::Service(Code::Exists)),
            Err(e) => return Err(e),
        }
        storage.mkdir(folder)?;
        let mut made: Vec<String> = alloc::vec![String::from(folder)];
        let mut written = 0;
        let mut total = 0u64;
        for i in 0..archive.items.len() {
            let item = &archive.items[i];
            let parts: Vec<&str> = item.path.split('/').collect();
            let dirs = if item.folder {
                parts.len()
            } else {
                parts.len() - 1
            };
            let mut path = String::from(folder);
            for part in &parts[..dirs] {
                path = format!("{path}/{part}");
                if !made.contains(&path) {
                    match storage.mkdir(&path) {
                        Ok(()) | Err(Error::Service(Code::Exists)) => (),
                        Err(e) => return Err(e),
                    }
                    made.push(path.clone());
                }
            }
            if item.folder {
                continue;
            }
            let data = match archive.read(i, password.as_deref(), MAX_UNPACKED) {
                Ok(data) => data,
                Err(f) => {
                    self.message = fault(f);
                    return Ok(());
                }
            };
            total += data.len() as u64;
            if total > MAX_EXTRACT {
                let _ = storage.flush();
                self.message = format!(
                    "Extraction stopped: the archive expands to more than {} GB.",
                    MAX_EXTRACT >> 30
                );
                return Ok(());
            }
            let target = format!("{path}/{}", parts[parts.len() - 1]);
            let file = match storage.open(&target, Disposition::CreateNew, READ | WRITE, 0) {
                Ok(f) => f,
                // A repeated (or case-colliding) name: the first one stays.
                Err(Error::Service(Code::Exists)) => continue,
                Err(e) => return Err(e),
            };
            let mut at = 0;
            while at < data.len() {
                let n = (data.len() - at).min(door_service_protocol::CHUNK_BYTES);
                let w = storage.write(&file, at as u64, &data[at..at + n])?;
                if w == 0 {
                    let _ = storage.close(file);
                    return Err(Error::Service(Code::Io));
                }
                at += w;
            }
            storage.close(file)?;
            written += 1;
        }
        storage.flush()?;
        door_rt::diagnostic(format_args!(
            "DOOR_P15 PREVIEW EXTRACT entries={written} skipped={} encrypted={}\n",
            archive.skipped,
            u8::from(password.is_some())
        ));
        self.opening = false;
        self.message = format!(
            "Extracted {written} files to {}",
            folder.rsplit('/').next().unwrap_or(folder)
        );
        Ok(())
    }
    fn export(&mut self, storage: &mut Storage) -> Result<(), Error> {
        self.cancel(storage);
        let path = self.resolve(storage, self.path.text(), true)?;
        let selected = self
            .convert_target()
            .ok_or(Error::Service(Code::Unsupported))?;
        let target = typed_target(&self.convert_targets(), selected, &path);
        if target == Target::Folder {
            return self.extract_to(storage, &path);
        }
        match storage.metadata(&path) {
            Err(Error::Service(Code::NotFound)) => (),
            Ok(_) => return Err(Error::Service(Code::Exists)),
            Err(e) => return Err(e),
        }
        let tab = self
            .selected()
            .ok_or(Error::Service(Code::InvalidRequest))?;
        if !door_workspace::runtime::working_memory(32 * 1024 * 1024) {
            return Err(Error::Service(Code::Resources));
        }
        let name = tab.path.rsplit('/').next().unwrap_or("file");
        let converted = match target {
            Target::Png
            | Target::Jpeg
            | Target::Bmp
            | Target::Qoi
            | Target::Tga
            | Target::Ppm
            | Target::Ico
            | Target::Gif => tab
                .image
                .as_ref()
                .ok_or("No decoded image")
                .and_then(|image| converter::image(image, target, 90))
                .map_err(String::from),
            Target::Text | Target::TextUtf16 | Target::Markdown | Target::Html => {
                converter::text(&tab.text, target, name).map_err(String::from)
            }
            Target::Wav | Target::Flac => match tab.audio.as_ref() {
                None => Err(String::from("No decoded audio")),
                Some(pcm) if target == Target::Wav => {
                    Ok(door_workspace::audio_codec::encode_wav(pcm))
                }
                Some(pcm) => door_workspace::audio_codec::encode_flac(pcm)
                    .map_err(|f| String::from(f.message())),
            },
            _ => self.pack(target),
        };
        let bytes = match converted {
            Ok(bytes) => bytes,
            Err(reason) => {
                self.message = reason;
                return Ok(());
            }
        };
        door_rt::diagnostic(format_args!(
            "DOOR_PREVIEW CONVERT target={:?} bytes={}\n",
            target,
            bytes.len()
        ));
        let (parent, name) = path.rsplit_once('/').ok_or(Error::Malformed)?;
        if name.encode_utf16().count() > 48 || name.is_empty() {
            return Err(Error::Service(Code::InvalidRequest));
        }
        let parent = if parent.is_empty() { "/" } else { parent };
        let info = storage.metadata(parent)?;
        if !info.directory {
            return Err(Error::Service(Code::InvalidRequest));
        }
        let staging = format!(
            "{}/.preview-{}-{}.part",
            parent.trim_end_matches('/'),
            door_sys::time()?,
            tab.info.object
        );
        let file = self.create_output(&path, &staging, || {
            storage.open(&staging, Disposition::CreateNew, READ | WRITE, 5)
        })?;
        self.output = Some(Exporting {
            path,
            staging,
            file,
            parent: info.object,
            bytes,
            written: 0,
        });
        self.opening = false;
        self.message = "Exporting...".into();
        Ok(())
    }
    fn complete_request(&mut self, result: Result<(), Error>, exporting: bool) {
        if let Err(e) = result {
            if exporting {
                door_rt::diagnostic(format_args!("DOOR_PREVIEW EXPORT_REFUSED code={e:?}\n"));
            }
            self.message = if exporting && self.export_uncertainty.is_some() {
                self.export_uncertainty.clone().unwrap()
            } else {
                format!(
                    "{}: {e:?}",
                    if exporting {
                        "Cannot export this file"
                    } else {
                        "Cannot open this file"
                    }
                )
            };
        }
    }
    fn open(&mut self, storage: &mut Storage) -> Result<(), Error> {
        self.cancel(storage);
        let path = self.resolve(storage, self.path.text(), false)?;
        let file = storage.open(&path, Disposition::OpenExisting, READ, SHARE_ALL)?;
        let info = match storage.info(&file) {
            Ok(info) => info,
            Err(e) => {
                let _ = storage.close(file);
                return Err(e);
            }
        };
        if info.directory || info.bytes > codec::MAX_VIEW_INPUT as u64 {
            storage.close(file)?;
            self.message = "Choose a file up to 6 MB.".into();
            return Ok(());
        }
        if self.tabs.len() == 8 {
            storage.close(file)?;
            self.message = "Close a tab before opening another file.".into();
            return Ok(());
        }
        let mut bytes = Vec::new();
        let mut margin = Vec::<u8>::new();
        if margin.try_reserve_exact(8 * 1024 * 1024).is_err()
            || bytes.try_reserve_exact(info.bytes as usize).is_err()
        {
            let _ = storage.close(file);
            return Err(Error::Service(Code::Resources));
        }
        drop(margin);
        // File Explorer Home > Recent.
        door_workspace::recent::note_opened(storage, &path, door_workspace::recent::Kind::File);
        self.job = Some(Reading {
            path,
            file,
            info,
            bytes,
        });
        self.opening = false;
        self.message = "Reading file...".into();
        Ok(())
    }
    /// File Explorer's caption strip from the left edge. With no document
    /// open, slot 0 is the Preview home tab, which carries the app identity.
    fn strip(&self) -> door_workspace::document_tabs::Strip {
        door_workspace::caption_tabs::strip(self.width, self.tabs.len().max(1), self.active)
    }
    fn tab_start(&self) -> usize {
        self.strip().start()
    }
    #[cfg(test)]
    fn tab_rect(&self, index: usize) -> Option<(i32, u32)> {
        self.strip().tab(index).map(|r| (r.x, r.width))
    }
    fn film_rect(&self, index: usize) -> Option<(i32, i32)> {
        let start = self.tab_start();
        if index < start {
            return None;
        }
        let (x, _, w, _) = self.dimensions();
        let tx = x + (index - start) as i32 * 116;
        let ty = self.height as i32 - 112 - presentation::grow(TextStyle::Regular12);
        (tx + 108 <= x + w as i32).then_some((tx, ty))
    }
    fn image_bytes(&self) -> usize {
        self.tabs
            .iter()
            .map(|t| {
                t.image.as_ref().map_or(0, |i| i.bgra.len())
                    + t.backup.as_ref().map_or(0, |i| i.bgra.len())
            })
            .sum()
    }
    fn admit_image(&self) -> bool {
        self.image_bytes() <= IMAGE_BUDGET - codec::MAX_PIXELS * 4
    }
    /// Play, pause or resume the selected audio tab (a finished one starts
    /// over).
    fn play_pause(&mut self) {
        let Some(index) = (self.active < self.tabs.len()).then_some(self.active) else {
            return;
        };
        let id = self.tabs[index].id;
        if let Some(p) = self.player.as_mut()
            && p.tab == id
            && !p.finished
        {
            p.toggle_pause();
            return;
        }
        self.player = None;
        let Some(pcm) = self.tabs[index].audio.as_ref() else {
            return;
        };
        match player::Player::start(self.registry, id, pcm, 0) {
            Ok(p) => self.player = Some(p),
            Err(reason) => self.message = reason.into(),
        }
    }
    /// Seek to where the last press landed on the transport's track.
    fn seek_press(&mut self) {
        let Some((_, track)) = self.transport() else {
            return;
        };
        let Some(index) = (self.active < self.tabs.len()).then_some(self.active) else {
            return;
        };
        let id = self.tabs[index].id;
        let Some(frames) = self.tabs[index].audio.as_ref().map(|p| p.frames()) else {
            return;
        };
        let along = (self.press.0 - track.x).clamp(0, track.w as i32) as u64;
        let frame = (along * frames as u64 / u64::from(track.w.max(1))) as usize;
        let result = match self.player.as_mut() {
            Some(p) if p.tab == id => p.seek(frame),
            _ => {
                self.player = None;
                let pcm = self.tabs[index].audio.as_ref().unwrap();
                player::Player::start(self.registry, id, pcm, frame).map(|p| {
                    self.player = Some(p);
                })
            }
        };
        if let Err(reason) = result {
            self.message = reason.into();
            self.player = None;
        }
    }
    fn tick_rest(&mut self, storage: &mut Storage, now: u64) -> Result<bool, Error> {
        let next = self.focused && (now / 50).is_multiple_of(2);
        let caret_changed = self.opening && self.caret != next;
        self.caret = next;
        if let Some(mut output) = self.output.take() {
            let result = (|| -> Result<bool, Error> {
                for _ in 0..16 {
                    if output.written == output.bytes.len() {
                        break;
                    }
                    let end = (output.written + 192).min(output.bytes.len());
                    let n = storage.write(
                        &output.file,
                        output.written as u64,
                        &output.bytes[output.written..end],
                    )?;
                    if n != end - output.written {
                        return Err(Error::Malformed);
                    }
                    output.written = end;
                }
                if output.written != output.bytes.len() {
                    return Ok(false);
                }
                storage.flush()?;
                let info = storage.info(&output.file)?;
                if info.bytes != output.bytes.len() as u64
                    || info.directory
                    || storage.metadata(&output.staging)? != info
                {
                    return Err(Error::Service(Code::Changed));
                }
                let name: Vec<u16> = output
                    .path
                    .rsplit('/')
                    .next()
                    .ok_or(Error::Malformed)?
                    .encode_utf16()
                    .collect();
                storage.rename_child_checked(output.parent, info.object, info.version, &name)?;
                Ok(true)
            })();
            if result == Ok(false) {
                self.output = Some(output);
                return Ok(now.is_multiple_of(10));
            }
            let closed = storage.close(output.file);
            if let Err(e) = result.and(closed.map(|_| true)) {
                self.export_failed(e, &output.path, &output.staging);
            } else {
                self.export_uncertainty = None;
                self.message = format!("Exported {} bytes to {}", output.bytes.len(), output.path);
                door_rt::diagnostic(format_args!(
                    "DOOR_PREVIEW EXPORT bytes={}\n",
                    output.bytes.len()
                ));
            }
            return Ok(true);
        }
        let Some(mut job) = self.job.take() else {
            return Ok(caret_changed);
        };
        let result = (|| -> Result<bool, Error> {
            for _ in 0..8 {
                let remaining = job.info.bytes as usize - job.bytes.len();
                if remaining == 0 {
                    break;
                }
                let mut part = [0; 192];
                let n = storage.read(
                    &job.file,
                    job.bytes.len() as u64,
                    &mut part[..remaining.min(192)],
                )?;
                if n == 0 {
                    return Err(Error::Malformed);
                }
                job.bytes.extend_from_slice(&part[..n]);
            }
            if job.bytes.len() != job.info.bytes as usize {
                return Ok(false);
            }
            if storage.info(&job.file)? != job.info || storage.metadata(&job.path)? != job.info {
                return Err(Error::Service(Code::Changed));
            }
            Ok(true)
        })();
        match result {
            Ok(false) => {
                self.job = Some(job);
                Ok(now.is_multiple_of(10) || caret_changed)
            }
            result => {
                let closed = storage.close(job.file);
                if let Err(e) = result.and(closed.map(|_| true)) {
                    self.message = format!("File read not confirmed: {e:?}");
                    return Ok(true);
                }
                if !door_workspace::runtime::working_memory(32 * 1024 * 1024) {
                    self.message="Not enough shared app memory to decode this file. Existing tabs and edits are retained; close another window or image and retry.".into();
                    return Ok(true);
                }
                let kind = Kind::of(&job.path, false);
                let raw = job.bytes.clone();
                let mut tab = Tab {
                    id: door_workspace::document_tabs::next_id(),
                    zoom: 100,
                    fit: true,
                    path: job.path,
                    info: job.info,
                    image: None,
                    backup: None,
                    text: String::new(),
                    error: String::new(),
                    scroll: 0,
                    details: Vec::new(),
                    raw: Vec::new(),
                    audio: None,
                };
                if kind == Kind::Image {
                    if !self.admit_image() {
                        self.message="Image memory budget reached. Close an image tab before opening another.".into();
                        return Ok(true);
                    }
                    match codec::decode_full(&job.bytes) {
                        Ok(image) => {
                            if self.image_bytes() + image.bgra.len() > IMAGE_BUDGET {
                                self.message =
                                    "Image memory budget reached. Close an image tab first.".into();
                                return Ok(true);
                            }
                            tab.image = Some(image)
                        }
                        Err(e) => tab.error = e.into(),
                    }
                } else if matches!(kind, Kind::Text | Kind::Code) {
                    match codec::text_document(&job.bytes) {
                        Ok(text) => {
                            if text.is_empty() {
                                tab.error = "This file is empty.".into();
                            }
                            tab.text = text
                        }
                        Err(e) => tab.error = e.into(),
                    }
                } else {
                    if matches!(kind, Kind::Audio | Kind::Model) && !self.admit_image() {
                        self.message = "Close an image tab to free preview memory.".into();
                        return Ok(true);
                    }
                    let decoded = (kind == Kind::Audio)
                        .then(|| door_workspace::audio_codec::decode(&job.bytes));
                    match decoded {
                        Some(Ok(pcm)) => {
                            tab.image = Some(formats::waveform(&pcm));
                            tab.details = formats::audio_details(&pcm, &job.bytes);
                            tab.audio = Some(pcm);
                        }
                        Some(Err(door_workspace::audio_codec::Fault::TooLarge)) => {
                            tab.error = door_workspace::audio_codec::Fault::TooLarge
                                .message()
                                .into();
                        }
                        _ => match formats::inspect(kind, &job.bytes) {
                            Ok(content) => {
                                tab.image = content.image;
                                tab.text = content.text;
                                tab.details = content.details
                            }
                            Err(e) => tab.error = e.into(),
                        },
                    }
                }
                tab.raw = raw;
                let image_decoded = tab.image.is_some();
                self.recent.retain(|p| p != &tab.path);
                self.recent.insert(0, tab.path.clone());
                self.recent.truncate(16);
                self.section = 0;
                self.save_view();
                self.tabs.push(tab);
                self.active = self.tabs.len() - 1;
                self.restore_view();
                self.info_page = 0;
                self.message.clear();
                door_rt::diagnostic(format_args!(
                    "DOOR_PREVIEW OPEN object={} bytes={} tabs={} image={}\n",
                    job.info.object,
                    job.info.bytes,
                    self.tabs.len(),
                    u8::from(image_decoded)
                ));
                Ok(true)
            }
        }
    }
    fn close_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            if self
                .player
                .as_ref()
                .is_some_and(|p| p.tab == self.tabs[index].id)
            {
                self.player = None;
            }
            self.save_view();
            self.tabs.remove(index);
            if index < self.active {
                self.active -= 1;
            }
            self.active = self.active.min(self.tabs.len().saturating_sub(1));
            self.restore_view();
            door_rt::diagnostic(format_args!(
                "DOOR_PREVIEW TABS count={}\n",
                self.tabs.len()
            ));
        }
    }
    fn dimensions(&self) -> (i32, i32, u32, u32) {
        let left = if self.width < 760 {
            80
        } else {
            self.rail() as i32 + 12
        };
        let right = if self.width >= 1100 && self.info_open {
            380
        } else {
            0
        };
        // The toolbar band above and the status line below grow with text.
        let grown =
            presentation::grow(TextStyle::Regular14) + presentation::grow(TextStyle::Regular12);
        (
            left,
            presentation::content_top(),
            self.width.saturating_sub(left as u32 + right + 24),
            self.height.saturating_sub(260 + grown as u32),
        )
    }
}
impl Preview {
    fn paint_notice(&self, v: &mut View<'_>) {
        if self.notice_open {
            let (left, top, width, height) = self.notice_rect();
            v.rounded(
                left - 2,
                top - 2,
                width + 4,
                height + 4,
                10,
                [128, 128, 128, 255],
            );
            v.rounded(left, top, width, height, 8, [252, 252, 252, 255]);
            let (g12, g16) = (
                presentation::grow(TextStyle::Regular12),
                presentation::grow(TextStyle::Semibold16),
            );
            let pitch = (18 + g12) as u32;
            let rows = (height.saturating_sub((78 + g16 + g12) as u32) / pitch).max(1) as usize;
            let lines = self.notice_lines(v, width.saturating_sub(24));
            let pages = lines.len().div_ceil(rows).max(1);
            self.notice_pages.set(pages);
            let page = self.notice_page.min(pages - 1);
            v.text_clip(
                left + 12,
                top + 8,
                width.saturating_sub(24),
                &format!("Export notice ({}/{})", page + 1, pages),
                TextStyle::Semibold16,
                FG,
            );
            for (i, line) in lines.iter().skip(page * rows).take(rows).enumerate() {
                v.small(
                    left + 12,
                    top + 38 + g16 + i as i32 * pitch as i32,
                    line,
                    FG,
                );
            }
            for (index, label) in ["Prev", "Next", "Close"].iter().enumerate() {
                let (bx, by, bw, bh) = self.notice_button(index as u32);
                let hover = v.hover.is_some_and(|(x, y)| {
                    (bx..bx + bw as i32).contains(&x) && (by..by + bh as i32).contains(&y)
                });
                v.rounded(
                    bx,
                    by,
                    bw,
                    bh,
                    4,
                    if hover {
                        [245, 226, 205, 255]
                    } else {
                        [238, 238, 238, 255]
                    },
                );
                // A contrast theme's hover plate is Highlight: HighlightText on it.
                let ink = if hover && v.palette.contrast.is_some() {
                    [255; 4]
                } else {
                    FG
                };
                v.text_in_box(
                    (bx + 4, by, bw.saturating_sub(8), bh),
                    label,
                    TextStyle::Regular12,
                    ink,
                );
            }
        }
    }
}

impl Application for Preview {
    fn retired(&mut self) {
        self.tabs.clear();
    }
    fn title(&self) -> &str {
        "Preview"
    }
    /// Start search opens images here (shell DOCUMENT_OPEN, a Door path).
    fn open_document(
        &mut self,
        payload: &[u8],
        _launched: bool,
        storage: &mut Storage,
    ) -> Result<bool, Error> {
        let path = core::str::from_utf8(payload).map_err(|_| Error::Malformed)?;
        self.path.set(path.into(), true);
        let result = self.open(storage);
        self.complete_request(result, false);
        if self.job.is_some() {
            self.section = 0;
        }
        Ok(true)
    }
    fn new_window(&self, _s: &mut Storage) -> Result<Option<Self>, Error> {
        let mut p = Self::new(self.uid);
        p.info_open = self.info_open;
        p.format = self.format;
        p.library_open = self.library_open;
        p.tools_open = self.tools_open;
        Ok(Some(p))
    }
    fn modal(&self) -> bool {
        self.opening || self.exporting || self.notice_open || self.cropping
    }
    fn close_shortcut(&self) -> bool {
        self.tabs.is_empty()
    }
    fn tab_at(&self, x: i32, y: i32) -> Option<usize> {
        // The home tab is not a document and cannot be dragged or detached.
        if self.busy() || self.modal() || self.tabs.is_empty() {
            None
        } else {
            self.strip().hit(x, y)
        }
    }
    fn tab_identity(&self, index: usize) -> Option<u64> {
        self.tabs.get(index).map(|t| t.id).filter(|id| *id != 0)
    }
    fn tab_index(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id && id != 0)
    }
    fn transfer_tab(
        &mut self,
        target: &mut Self,
        index: usize,
        _s: &mut Storage,
    ) -> Result<bool, Error> {
        self.move_tab_to(target, index)
    }
    fn reorder_tab(&mut self, from: usize, to: usize) -> bool {
        if from >= self.tabs.len() || to >= self.tabs.len() || from == to {
            return false;
        }
        self.save_view();
        let t = self.tabs.remove(from);
        self.tabs.insert(to, t);
        self.active = to;
        self.restore_view();
        true
    }
    fn empty_after_transfer(&self) -> bool {
        self.tabs.is_empty()
    }

    fn busy(&self) -> bool {
        self.job.is_some() || self.output.is_some()
    }
    fn closing(&mut self, storage: &mut Storage) -> Result<bool, Error> {
        if self.output.is_some() {
            self.message="Export is in progress. Wait for it to finish, or press Esc to cancel before closing.".into();
            Ok(false)
        } else {
            if let Some(job) = self.job.take() {
                let _ = storage.close(job.file);
            }
            Ok(true)
        }
    }
    fn caption(&self) -> (u32, u32) {
        door_workspace::caption_tabs::region(self.strip())
    }
    fn options(&self) -> door_workspace::runtime::WindowOptions {
        door_workspace::runtime::WindowOptions {
            width: 1280,
            height: 800,
            caption_height: door_workspace::caption_tabs::HEIGHT,
            caption_left: door_workspace::caption_tabs::LEFT as u32,
            centered: false,
        }
    }
    fn wants_close(&mut self) -> bool {
        core::mem::take(&mut self.close_requested)
    }
    fn wheel(&mut self, x: i32, y: i32, notches: i32, _f: &door_graphics_text::FontAtlas) -> bool {
        self.scroll(x, y, notches)
    }
    fn configure(&mut self, width: u32, height: u32) {
        if width != self.width || height != self.height {
            self.cancel_crop();
        }
        self.width = width;
        self.height = height;
        self.clamp_nav();
    }
    fn semantics(
        &self,
        s: &mut door_workspace::semantics::Semantics,
        _f: &door_graphics_text::FontAtlas,
    ) {
        self.describe(s)
    }
    fn target(&self, x: i32, y: i32) -> Option<u32> {
        self.actions()
            .into_iter()
            .rev()
            .find(|(_, r)| r.hit(x, y))
            .map(|(id, _)| id)
    }
    fn click(&mut self, id: u32, storage: &mut Storage) -> Result<bool, Error> {
        if matches!(id, 700 | 701) && !self.cropping && !self.notice_open && !self.opening {
            if id == 700 {
                self.play_pause();
            } else {
                self.seek_press();
            }
            return Ok(true);
        }
        if self.cropping {
            match id {
                210 => {
                    self.apply_crop();
                }
                211 => self.cancel_crop(),
                _ => (),
            };
            return Ok(true);
        }
        match id {
            920 => {
                self.page_notice(false);
                return Ok(true);
            }
            921 => {
                self.page_notice(true);
                return Ok(true);
            }
            922 => {
                self.notice_open = false;
                return Ok(true);
            }
            923 => {
                self.notice_open = true;
                return Ok(true);
            }
            _ => (),
        }
        match id {
            424 | 425 => self.toggle_group(id == 424),
            // Thumb drags are handled by pointer_down/motion.
            426 => return Ok(false),
            422 => self.section_scroll = self.section_scroll.saturating_sub(1),
            423 => {
                self.section_scroll = (self.section_scroll + 1).min(if self.section == 16 {
                    2
                } else {
                    self.library_paths().len().saturating_sub(1)
                })
            }
            200 => {
                self.activate(self.active.saturating_sub(1));
            }
            201 => {
                self.activate((self.active + 1).min(self.tabs.len().saturating_sub(1)));
            }
            202 => self.info_open = !self.info_open,
            203 => {
                let count = self.convert_targets().len().max(1) as u32;
                self.format = (self.format + 1) % count;
            }
            204 => {
                self.fit = true;
                self.zoom = 100;
            }
            91 => {
                return self.key(28, storage);
            }
            92 => {
                self.opening = false;
                self.password.set(String::new(), false);
            }
            400..=416 => {
                self.select_section(id - 400);
                self.section_scroll = 0;
                door_rt::diagnostic(format_args!("DOOR_PREVIEW SECTION id={}\n", self.section));
            }
            600..=615 => {
                if let Some(path) = self
                    .library_paths()
                    .get(self.section_scroll + (id - 600) as usize)
                    .cloned()
                {
                    self.path.set(path, true);
                    let result = self.open(storage);
                    self.complete_request(result, false);
                    if self.job.is_some() {
                        self.section = 0;
                    }
                }
            }
            1 => {
                self.opening = true;
                self.exporting = false;
                self.path.select_all();
            }
            7 => self.begin_export(),
            90 => self.password_focus = false,
            93 => self.password_focus = true,
            8 => self.info_page = self.info_page.saturating_sub(1),
            9 => self.info_page = (self.info_page + 1) % self.info_pages(),
            2 => {
                self.section = 0;
                self.fit = false;
                self.zoom = self.zoom.saturating_sub(25).max(25);
            }
            3 => {
                self.fit = true;
            }
            4 => {
                self.fit = false;
                self.zoom = (self.zoom + 25).min(400);
            }
            5 | 6 => {
                self.section = 0;
                self.transform_image(id == 5);
            }
            206 => {
                if self.editable_image() {
                    self.section = 0;
                    self.fit = true;
                    self.cropping = true;
                    self.crop = None;
                }
            }
            209 => self.undo_image(),
            // Closing the home tab closes the window, as Explorer's last tab does.
            300 if self.tabs.is_empty() => self.close_requested = true,
            300..=307 => self.close_tab((id - 300) as usize),
            100..=107 => {
                if ((id - 100) as usize) < self.tabs.len() {
                    self.section = 0;
                    self.activate((id - 100) as usize);
                    self.info_page = 0;
                }
            }
            _ => (),
        }
        let _ = storage;
        Ok(true)
    }
    fn key(&mut self, key: u32, storage: &mut Storage) -> Result<bool, Error> {
        let scan = key & 511;
        if scan == 57
            && !self.opening
            && !self.cropping
            && self.selected().is_some_and(|t| t.audio.is_some())
        {
            self.play_pause();
            return Ok(true);
        }
        if self.cropping {
            match scan {
                1 => self.cancel_crop(),
                28 | 0x11c => {
                    self.apply_crop();
                }
                _ => (),
            };
            return Ok(true);
        }
        if self.notice_open {
            match scan {
                1 | 28 => self.notice_open = false,
                0x149 | 0x14b => self.page_notice(false),
                0x151 | 0x14d => self.page_notice(true),
                _ => (),
            }
            return Ok(true);
        }
        let control = (key >> 9) & door_abi::graphics::CONTROL != 0;
        let shift = (key >> 9) & door_abi::graphics::SHIFT != 0;
        if control && shift && scan == 31 {
            self.begin_export();
            return Ok(true);
        }
        if control && scan == 24 {
            self.opening = true;
            self.exporting = false;
            self.path.select_all();
            return Ok(true);
        }
        if self.opening {
            self.caret = true;
            match scan {
                1 => self.opening = false,
                28 | 0x11c => {
                    let result = if self.exporting {
                        self.export(storage)
                    } else {
                        self.open(storage)
                    };
                    self.complete_request(result, self.exporting);
                }
                // Tab moves between the path and the password box.
                15 if self.password_needed => self.password_focus = !self.password_focus,
                _ => {
                    let field = if self.password_focus && self.password_needed {
                        &mut self.password
                    } else {
                        &mut self.path
                    };
                    match scan {
                        14 => field.backspace(),
                        0x153 => field.delete(),
                        30 if control => field.select_all(),
                        0x14b => field.horizontal(false, false, control),
                        0x14d => field.horizontal(true, false, control),
                        _ => {
                            let c = (key >> 16) as u8;
                            if !control && (32..=126).contains(&c) && field.text().len() < 192 {
                                field.insert(c as char);
                            }
                        }
                    }
                }
            }
            return Ok(true);
        }
        if self.image_shortcut(key) {
            return Ok(true);
        }
        if control && scan == 17 && !self.tabs.is_empty() {
            self.close_tab(self.active);
            return Ok(true);
        }
        if control && scan == 15 && !self.tabs.is_empty() {
            self.activate((self.active + 1) % self.tabs.len());
            return Ok(true);
        }
        if matches!(scan, 0x151 | 0x149) && matches!(self.section, 1 | 3..=9 | 16) {
            self.section_scroll = if scan == 0x151 {
                (self.section_scroll + 1).min(if self.section == 16 {
                    2
                } else {
                    self.library_paths().len().saturating_sub(1)
                })
            } else {
                self.section_scroll.saturating_sub(1)
            };
            return Ok(true);
        }
        if matches!(scan, 0x151 | 0x149)
            && let Some(tab) = self.tabs.get_mut(self.active)
        {
            tab.scroll = if scan == 0x151 {
                (tab.scroll + 16).min(tab.text.lines().count().saturating_sub(1))
            } else {
                tab.scroll.saturating_sub(16)
            };
            return Ok(true);
        }
        if scan == 1 {
            self.cancel(storage);
            return Ok(true);
        }
        Ok(false)
    }
    fn tick(&mut self, storage: &mut Storage, now: u64) -> Result<bool, Error> {
        let mut playing = false;
        if let Some(p) = self.player.as_mut() {
            match p.tick() {
                Ok(moved) => playing = moved || !p.paused,
                Err(reason) => {
                    self.message = reason.into();
                    self.player = None;
                    playing = true;
                }
            }
        }
        if playing {
            return self.tick_rest(storage, now).map(|_| true);
        }
        self.tick_rest(storage, now)
    }
    fn focus(&mut self, focused: bool) {
        self.focused = focused;
        self.crop_drag = false;
    }
    fn disconnected(&mut self) {
        self.job = None;
        if let Some(output) = self.output.take() {
            self.export_failed(
                Error::Transport(door_abi::Status::PeerClosed),
                &output.path,
                &output.staging,
            );
        }
        self.message = self
            .export_uncertainty
            .clone()
            .unwrap_or_else(|| "Connection changed. Reopen an unfinished read.".into());
        if self.export_uncertainty.is_some() {
            self.notice_open = true;
        }
    }
    fn pointer_down(&mut self, x: i32, y: i32, _f: &door_graphics_text::FontAtlas) -> bool {
        self.press = (x, y);
        if !self.cropping && self.target(x, y) == Some(426) {
            return self.grab_nav(y);
        }
        self.crop_pointer(x, y, true)
    }
    fn pointer_motion(&mut self, x: i32, y: i32, _f: &door_graphics_text::FontAtlas) -> bool {
        if self.nav_grab.is_some() {
            return self.drag_nav(y);
        }
        self.crop_pointer(x, y, false)
    }
    fn pointer_up(&mut self) {
        self.crop_drag = false;
        self.nav_grab = None;
    }
    fn draw(&self, v: &mut View<'_>) {
        self.paint(v);
        self.paint_crop(v);
        self.paint_notice(v);
    }
}

/// The format an export writes: a name typed with the extension of another
/// format this file converts to picks that format, as Windows' save dialog
/// does for a typed extension, so the bytes always match the name; any other
/// name keeps the Convert list's choice.
fn typed_target(targets: &[Target], selected: Target, path: &str) -> Target {
    let lower = path.to_ascii_lowercase();
    let typed = |t: &Target| lower.ends_with(&format!(".{}", t.extension()));
    if selected == Target::Folder || typed(&selected) {
        return selected;
    }
    targets
        .iter()
        .copied()
        .find(|t| *t != Target::Folder && typed(t))
        .unwrap_or(selected)
}
#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    #[test]
    fn a_typed_extension_picks_its_export_format() {
        let images = [Target::Png, Target::Jpeg, Target::Qoi, Target::Bmp];
        let pick = |path: &str| typed_target(&images, Target::Png, path);
        assert_eq!(pick("/users/1001/notes/a.qoi"), Target::Qoi);
        assert_eq!(pick("/users/1001/notes/a.QOI"), Target::Qoi);
        assert_eq!(pick("/users/1001/notes/a.jpg"), Target::Jpeg);
        assert_eq!(pick("/users/1001/notes/a.png"), Target::Png);
        // An unknown or missing extension keeps the Convert list's choice.
        assert_eq!(pick("/users/1001/notes/a.webp"), Target::Png);
        assert_eq!(pick("/users/1001/notes/a"), Target::Png);
        // A format this file cannot convert to is never chosen by name.
        assert_eq!(pick("/users/1001/notes/a.tga"), Target::Png);
    }
    #[test]
    fn archives_list_extract_first_and_repack_with_their_password() {
        let mut app = Preview::new(1001);
        let mut zip = packing::ZipWriter::new(Some("hunter22"));
        zip.add("notes/a.txt", Some(b"alpha alpha alpha"), 6, || {
            [9; packing::SALT_BYTES]
        });
        let mut tab = document("/users/1001/locked.zip");
        tab.raw = zip.finish();
        app.tabs.push(tab);
        let targets = app.convert_targets();
        assert_eq!(targets[0], Target::Folder);
        assert!(targets.contains(&Target::TarGz) && !targets.contains(&Target::Zip));
        // Repacking needs the archive's password.
        assert!(app.pack(Target::Tar).is_err());
        app.password.set("hunter22".into(), false);
        let tar = app.pack(Target::Tar).unwrap();
        let back = Archive::open("x.tar", tar, 1 << 20).unwrap();
        assert_eq!(back.items[0].path, "notes/a.txt");
        assert_eq!(back.read(0, None, 1 << 20).unwrap(), b"alpha alpha alpha");
        // A plain file packs as one entry; a protected ZIP wants 4+ characters.
        let mut text = document("/users/1001/readme.txt");
        text.raw = b"hello".to_vec();
        text.text = "hello".into();
        app.tabs = alloc::vec![text];
        app.active = 0;
        assert!(app.convert_targets().contains(&Target::Html));
        app.password.set("abc".into(), false);
        assert!(app.pack(Target::ZipProtected).is_err());
        app.password.set("abcd".into(), false);
        let z = app.pack(Target::ZipProtected).unwrap();
        let a = Archive::open("x.zip", z, 64).unwrap();
        assert_eq!(a.read(0, Some("abcd"), 64).unwrap(), b"hello");
    }
    fn document(path: &str) -> Tab {
        Tab {
            id: door_workspace::document_tabs::next_id(),
            zoom: 100,
            fit: true,
            path: path.into(),
            info: FileInfo {
                attributes: 0,
                object: 7,
                version: 9,
                bytes: 4,
                directory: false,
            },
            image: None,
            backup: None,
            text: "Keep".into(),
            error: String::new(),
            scroll: 3,
            details: alloc::vec![("Author".into(), "Actual".into())],
            raw: Vec::new(),
            audio: None,
        }
    }
    #[test]
    fn tab_transfer_moves_owned_content_and_inactive_view_without_losing_source_state() {
        let mut a = Preview::new(1001);
        let mut b = Preview::new(1001);
        a.tabs.push(document("first.txt"));
        a.tabs.push(document("second.txt"));
        a.tabs[1].zoom = 250;
        a.tabs[1].fit = false;
        a.zoom = 75;
        a.fit = false;
        let id = a.tabs[1].id;
        assert!(a.move_tab_to(&mut b, 1).unwrap());
        assert_eq!(a.tabs.len(), 1);
        assert_eq!(a.zoom, 75);
        assert_eq!(b.tab_identity(0), Some(id));
        assert_eq!(b.tabs[0].text, "Keep");
        assert_eq!(b.tabs[0].info.version, 9);
        assert_eq!(b.tabs[0].scroll, 3);
        assert_eq!(b.zoom, 250);
        assert!(!b.fit);
        assert_eq!(b.tab_index(id), Some(0));
        b.opening = true;
        assert!(!a.move_tab_to(&mut b, 0).unwrap());
        assert_eq!(a.tabs.len(), 1);
        assert_eq!(b.tabs.len(), 1);
        b.opening = false;
        assert!(b.move_tab_to(&mut a, 0).unwrap());
        assert!(b.tabs.is_empty());
        assert_eq!(a.tabs[1].id, id);
        a.activate(0);
        assert_eq!(a.zoom, 75);
        a.activate(1);
        assert_eq!(a.zoom, 250);
    }
    #[test]
    fn text_conversion_choice_matches_proposed_extension_and_rejects_missing_provider() {
        let mut app = Preview::new(1001);
        app.format = 1;
        app.section = 11;
        app.begin_export();
        assert!(!app.opening);
        app.tabs.push(Tab {
            id: door_workspace::document_tabs::next_id(),
            zoom: 100,
            fit: true,
            path: "/users/1001/readme.txt".into(),
            info: FileInfo {
                attributes: 0,
                object: 8,
                version: 1,
                bytes: 4,
                directory: false,
            },
            image: None,
            backup: None,
            text: "Text".into(),
            error: String::new(),
            scroll: 0,
            details: Vec::new(),
            raw: Vec::new(),
            audio: None,
        });
        // Text converts to UTF-16, Markdown and HTML (packing needs the
        // file's bytes, which this fixture lacks).
        assert_eq!(
            app.convert_targets(),
            [Target::TextUtf16, Target::Markdown, Target::Html]
        );
        assert_eq!(app.export_label(), "Markdown");
        assert!(app.actions().iter().any(|(id, _)| *id == 203));
        app.begin_export();
        assert!(app.opening && app.exporting && !app.password_needed);
        assert_eq!(app.path.text(), "/users/1001/readme.md");
    }
    #[test]
    fn refused_recent_open_is_nonfatal_and_preserves_existing_selection() {
        let mut app = Preview::new(1001);
        app.tabs.push(Tab {
            id: door_workspace::document_tabs::next_id(),
            zoom: 100,
            fit: true,
            path: "/users/1001/keep.txt".into(),
            info: FileInfo {
                attributes: 0,
                object: 7,
                version: 1,
                bytes: 4,
                directory: false,
            },
            image: None,
            backup: None,
            text: "Keep".into(),
            error: String::new(),
            scroll: 0,
            details: Vec::new(),
            raw: Vec::new(),
            audio: None,
        });
        for code in [Code::NotFound, Code::Denied] {
            app.complete_request(Err(Error::Service(code)), false);
            assert_eq!(app.tabs.len(), 1);
            assert_eq!(app.selected().unwrap().path, "/users/1001/keep.txt");
            assert!(app.message.starts_with("Cannot open this file:"));
        }
    }
    #[test]
    fn uncertain_export_paths_survive_storage_reconnect() {
        let mut app = Preview::new(1001);
        app.export_failed(
            Error::Transport(door_abi::Status::PeerClosed),
            "/users/1001/image-edited.png",
            "/users/1001/.preview-42-17.part",
        );
        let expected = app.message.clone();
        app.disconnected();
        app.disconnected();
        assert_eq!(app.message, expected);
        assert!(app.message.contains("/users/1001/image-edited.png"));
        assert!(app.message.contains("/users/1001/.preview-42-17.part"));
        assert!(app.message.contains("do not retry automatically"));
    }
    #[test]
    fn staging_creation_and_cancel_reply_loss_keep_recovery_names() {
        let mut app = Preview::new(1001);
        let failed = Error::Transport(door_abi::Status::PeerClosed);
        let result: Result<(), Error> = app.create_output(
            "/users/1001/export.png",
            "/users/1001/.preview-7-9.part",
            || Err(failed),
        );
        assert_eq!(result, Err(failed));
        app.disconnected();
        assert!(app.message.contains("/users/1001/export.png"));
        assert!(app.message.contains("/users/1001/.preview-7-9.part"));
        app.cancelled_output("/users/1001/.preview-8-9.part", || Err(failed));
        app.disconnected();
        app.disconnected();
        assert!(app.message.contains("/users/1001/.preview-8-9.part"));
        assert!(app.message.contains("before publication"));
        assert!(app.message.contains("do not retry automatically"));
    }
    #[test]
    fn export_notice_wraps_complete_paths_and_paging_stays_reachable() {
        for scale in 1..=3 {
            let fonts = door_graphics_text::FontAtlas::windows_reference_scaled(scale).unwrap();
            for (width, height) in [(240, 120), (320, 180), (490, 330)] {
                let mut pixels = alloc::vec![0;(width*height*4*scale*scale) as usize];
                let mut view =
                    View::new_scaled(&mut pixels, width * scale, height * scale, &fonts, scale);
                let mut app = Preview::new(1001);
                app.configure(width, height);
                let parent = format!("/users/1001/{}", "long name ".repeat(15));
                app.export_failed(
                    Error::Transport(door_abi::Status::PeerClosed),
                    &format!("{parent}/edited-image.png"),
                    &format!("{parent}/.preview-18446744073709551615-42.part"),
                );
                let lines = app.notice_lines(&view, app.notice_rect().2 - 40);
                assert_eq!(lines.concat(), app.export_uncertainty.as_deref().unwrap());
                assert!(
                    lines
                        .iter()
                        .all(|line| view.measure_small(line) <= app.notice_rect().2 - 40)
                );
                for dark in [false, true] {
                    view.palette.dark = dark;
                    app.draw(&mut view);
                    if width == 240 {
                        assert!(app.notice_pages.get() > 1);
                    }
                    for _ in 0..100 {
                        app.page_notice(true);
                    }
                    assert_eq!(app.notice_page, app.notice_pages.get() - 1);
                    app.page_notice(false);
                    assert_eq!(app.notice_page, app.notice_pages.get().saturating_sub(2));
                    for index in 0..3 {
                        let (x, y, w, h) = app.notice_button(index);
                        assert_eq!(
                            app.target(x + w as i32 / 2, y + h as i32 / 2),
                            Some(920 + index)
                        );
                        assert!(
                            x >= 0
                                && y >= 0
                                && x + w as i32 <= width as i32
                                && y + h as i32 <= height as i32
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn tabs_only_hit_visible_rectangles_and_image_budget_preserves_existing_tabs() {
        let mut app = Preview::new(1001);
        for i in 0..8 {
            app.tabs.push(Tab {
                id: door_workspace::document_tabs::next_id(),
                zoom: 100,
                fit: true,
                path: format!("/users/1001/{i}.ppm"),
                info: FileInfo {
                    attributes: 0,
                    object: i + 1,
                    version: 1,
                    bytes: 1,
                    directory: false,
                },
                image: None,
                backup: None,
                text: String::new(),
                error: String::new(),
                scroll: 0,
                details: Vec::new(),
                raw: Vec::new(),
                audio: None,
            });
        }
        for width in [490, 520, 700, 900, 1280] {
            app.configure(width, 600);
            for active in 0..8 {
                app.active = active;
                for x in 204..width as i32 {
                    let painted = app.tabs.iter().enumerate().any(|(i, _)| {
                        app.tab_rect(i)
                            .is_some_and(|(left, w)| (left..left + w as i32).contains(&x))
                    });
                    assert_eq!(
                        matches!(app.target(x, 24), Some(100..=107 | 300..=307)),
                        painted,
                        "width={width} active={active} x={x}"
                    );
                }
                assert!(app.tab_rect(active).is_some());
            }
        }
        for i in 0..4 {
            app.tabs[i].image = Some(Image {
                width: 1024,
                height: 1024,
                source_width: 1024,
                source_height: 1024,
                bgra: alloc::vec![255;codec::MAX_PIXELS*4],
            });
        }
        assert_eq!(app.image_bytes(), IMAGE_BUDGET);
        assert!(!app.admit_image());
        assert_eq!(app.tabs.len(), 8);
        app.tabs[0].image = None;
        assert!(app.admit_image());
    }
    #[test]
    fn zoom_magnifies_source_detail_and_clips_to_canvas() {
        let fonts = door_graphics_text::FontAtlas::windows_reference_scaled(1).unwrap();
        let mut image = alloc::vec![0;1000*4];
        for x in 0..1000 {
            image[x * 4..x * 4 + 4].copy_from_slice(&[(x / 4) as u8, 0, 0, 255]);
        }
        let paint = |percent| {
            let mut pixels = alloc::vec![255;200*40*4];
            let mut v = View::new_scaled(&mut pixels, 200, 40, &fonts, 1);
            v.image_zoom((20, 10, 160, 20), 1000, 1, &image, percent);
            pixels
        };
        let one = paint(100);
        let two = paint(200);
        assert_ne!(one, two);
        let at = (19 * 200 + 25) * 4;
        assert_eq!(one[at], (425 / 4) as u8);
        assert_eq!(two[at], (462 / 4) as u8);
        assert_eq!(&two[(19 * 200 + 19) * 4..][..4], &[255; 4]);
    }
    #[test]
    fn actual_viewer_paints_opaque_themes_and_keeps_caption_controls_out_of_targets() {
        let mut app = Preview::new(1001);
        app.tabs.push(Tab {
            id: door_workspace::document_tabs::next_id(),
            zoom: 100,
            fit: true,
            path: "/users/1001/sunrise.qoi".into(),
            info: FileInfo {
                attributes: 0,
                object: 42,
                version: 1,
                bytes: 42,
                directory: false,
            },
            backup: None,
            image: Some(
                codec::decode_full(include_bytes!(
                    "../../../assets/wallpapers/windows/sunrise-960x600.qoi"
                ))
                .unwrap(),
            ),
            text: String::new(),
            error: String::new(),
            scroll: 0,
            details: Vec::new(),
            raw: Vec::new(),
            audio: None,
        });
        app.tabs.push(Tab {
            id: door_workspace::document_tabs::next_id(),
            zoom: 100,
            fit: true,
            path: "/users/1001/readme.txt".into(),
            info: FileInfo {
                attributes: 0,
                object: 43,
                version: 1,
                bytes: 28,
                directory: false,
            },
            image: None,
            backup: None,
            text: "Preview reference review".into(),
            error: String::new(),
            scroll: 0,
            details: Vec::new(),
            raw: Vec::new(),
            audio: None,
        });
        app.recent = app.tabs.iter().map(|t| t.path.clone()).collect();
        for scale in 1..=3 {
            let fonts = door_graphics_text::FontAtlas::windows_reference_scaled(scale).unwrap();
            for (width, height) in [(490, 358), (900, 600), (1472, 952)] {
                for dark in [false, true] {
                    app.configure(width, height);
                    for section in 0..17 {
                        app.section = section;
                        let cache = door_app_ui::ImageCache::default();
                        let mut pixels = alloc::vec![0;(width*height*scale*scale*4) as usize];
                        let mut v = View::new_scaled(
                            &mut pixels,
                            width * scale,
                            height * scale,
                            &fonts,
                            scale,
                        )
                        .with_images(&cache);
                        v.palette.dark = dark;
                        app.draw(&mut v);
                        assert!(pixels.chunks_exact(4).all(|p| p[3] == 255));
                        if width == 1472 && scale == 1 {
                            let directory = std::path::Path::new("../../target/preview-review");
                            std::fs::create_dir_all(directory).unwrap();
                            let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
                            for pixel in pixels.chunks_exact(4) {
                                ppm.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
                            }
                            std::fs::write(
                                directory.join(format!(
                                    "page-{}-{}.ppm",
                                    app.section,
                                    if dark { "dark" } else { "light" }
                                )),
                                ppm,
                            )
                            .unwrap();
                        }
                        assert_eq!(app.target(width as i32 - 5, 20), None);
                        assert_eq!(app.target(24, 80), Some(400));
                    }
                }
            }
        }
    }
    /// Review captures of the navigation pane: overflowing, scrolled, collapsed
    /// with a hidden selection, hovered header, and the compact rail.
    #[test]
    fn navigation_pane_review_captures_use_the_actual_painter() {
        /// (name, width, height, scale, library open, tools open, section, hover)
        type NavCase = (
            &'static str,
            u32,
            u32,
            u32,
            bool,
            bool,
            u32,
            Option<(i32, i32)>,
        );
        let cases: [NavCase; 5] = [
            ("nav-full", 1472, 952, 1, true, true, 4, None),
            (
                "nav-short-scrolled",
                1100,
                560,
                1,
                true,
                true,
                12,
                Some((100, 300)),
            ),
            (
                "nav-collapsed-hidden-selection",
                1100,
                700,
                1,
                false,
                true,
                5,
                None,
            ),
            ("nav-compact", 700, 520, 1, true, true, 2, None),
            ("nav-short-2x", 1100, 560, 2, true, false, 0, None),
        ];
        for (name, width, height, scale, library, tools, section, hover) in cases {
            let fonts = door_graphics_text::FontAtlas::windows_reference_scaled(scale).unwrap();
            let mut app = Preview::new(1001);
            app.library_open = library;
            app.tools_open = tools;
            app.configure(width, height);
            app.select_section(section);
            if name == "nav-short-scrolled" {
                app.nav_scroll = app.nav_pane().max_scroll(&app.nav_rows()) / 2;
            }
            let cache = door_app_ui::ImageCache::default();
            let mut pixels = alloc::vec![0; (width * height * scale * scale * 4) as usize];
            let mut v = View::new_scaled(&mut pixels, width * scale, height * scale, &fonts, scale)
                .with_images(&cache);
            v.hover = hover;
            app.draw(&mut v);
            assert!(pixels.chunks_exact(4).all(|p| p[3] == 255));
            let directory = std::path::Path::new("../../target/preview-review");
            std::fs::create_dir_all(directory).unwrap();
            let (w, h) = (width * scale, height * scale);
            let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
            for pixel in pixels.chunks_exact(4) {
                ppm.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
            }
            std::fs::write(directory.join(format!("{name}.ppm")), ppm).unwrap();
        }
    }
    #[test]
    fn explorer_caption_tabs_start_at_the_left_edge_and_report_their_region() {
        let mut app = Preview::new(1001);
        for width in [490, 900, 1280, 1920] {
            app.configure(width, 700);
            // Home tab with the app identity when nothing is open.
            let (left, w) = app.caption();
            assert_eq!(left, 8);
            let home = app.strip().tab(0).unwrap();
            assert_eq!((home.x, home.y), (8, 3));
            assert_eq!(app.target(home.x + 40, 20), Some(100));
            let plus = app.plus().unwrap();
            assert_eq!(app.target(plus.x + 16, plus.y + 16), Some(1));
            assert_eq!(left + w, (plus.x + plus.w as i32) as u32);
            // Empty caption between "+" and the window controls is draggable.
            for x in (plus.x + plus.w as i32 + 2..width as i32).step_by(5) {
                assert_eq!(app.target(x, 20), None, "x={x} width={width}");
            }
            assert_eq!(app.tab_at(home.x + 40, 20), None);
        }
        let close = app.strip().close(0).unwrap();
        assert_eq!(app.target(close.x + 4, close.y + 4), Some(300));
        assert!(!app.wants_close());
        app.click_close_for_test();
        assert!(app.wants_close());
        assert!(!app.wants_close());
    }
    #[test]
    fn wheel_scrolls_the_navigation_pane_lists_and_text_documents() {
        let mut app = Preview::new(1001);
        app.configure(1100, 520);
        let max = app.nav_pane().max_scroll(&app.nav_rows());
        assert!(max > 0);
        assert!(app.scroll(40, 200, -1));
        assert_eq!(app.nav_scroll, 48.min(max));
        for _ in 0..40 {
            app.scroll(40, 200, -2);
        }
        assert_eq!(app.nav_scroll, max);
        assert!(!app.scroll(40, 200, -1));
        while app.scroll(40, 200, 1) {}
        assert_eq!(app.nav_scroll, 0);
        let mut doc = document("/users/1001/notes.txt");
        doc.text = (0..100).map(|i| format!("line {i}\n")).collect();
        doc.scroll = 0;
        app.tabs.push(doc);
        let body = app.body_for_test();
        assert!(app.scroll(body.0 + 40, body.1 + 40, -2));
        assert_eq!(app.tabs[0].scroll, 6);
        assert!(app.scroll(body.0 + 40, body.1 + 40, 5));
        assert_eq!(app.tabs[0].scroll, 0);
        app.opening = true;
        assert!(!app.scroll(body.0 + 40, body.1 + 40, -2));
    }
    #[test]
    fn caption_review_captures_use_the_actual_painter() {
        for dark in [false, true] {
            for (name, open) in [("caption-home", false), ("caption-documents", true)] {
                let fonts = door_graphics_text::FontAtlas::windows_reference_scaled(1).unwrap();
                let mut app = Preview::new(1001);
                if open {
                    app.tabs.push(document("/users/1001/sunrise.png"));
                    app.tabs.push(document("/users/1001/readme.txt"));
                    app.active = 0;
                }
                app.configure(1280, 400);
                let cache = door_app_ui::ImageCache::default();
                let mut pixels = alloc::vec![0; 1280 * 400 * 4];
                let mut v = View::new_scaled(&mut pixels, 1280, 400, &fonts, 1).with_images(&cache);
                v.palette.dark = dark;
                v.hover = Some((app.plus().unwrap().x + 10, 20));
                app.draw(&mut v);
                assert!(pixels.chunks_exact(4).all(|p| p[3] == 255));
                let directory = std::path::Path::new("../../target/preview-review");
                std::fs::create_dir_all(directory).unwrap();
                let mut ppm = b"P6\n1280 400\n255\n".to_vec();
                for pixel in pixels.chunks_exact(4) {
                    ppm.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
                }
                let theme = if dark { "dark" } else { "light" };
                std::fs::write(directory.join(format!("{name}-{theme}.ppm")), ppm).unwrap();
            }
        }
    }
    /// New windows open at 70% x 78% of the work area
    /// (`door_graphics_core::initial_window`): Preview's 1280x800 request
    /// becomes 896x524 at 1280x720, 717x562 at 1024x768 and 896x587 at
    /// 1280x800. The empty window and an open text document lay out there:
    /// the caption strip leaves the trusted buttons alone and the document
    /// body keeps its place beside the navigation pane.
    #[test]
    fn preview_lays_out_at_the_default_window_sizes() {
        let fonts = door_graphics_text::FontAtlas::windows_reference_scaled(1).unwrap();
        for (width, height) in [(896u32, 524u32), (717, 562), (896, 587)] {
            for open in [false, true] {
                let mut app = Preview::new(1001);
                if open {
                    let mut doc = document("/users/1001/readme.txt");
                    doc.text = (0..40)
                        .map(|i| format!("line {i} of a text document\n"))
                        .collect();
                    doc.scroll = 0;
                    app.tabs.push(doc);
                }
                app.configure(width, height);
                let (left, strip) = app.caption();
                assert!(left + strip <= width - 138, "{width}x{height}");
                let (x, y, w, h) = app.dimensions();
                assert!(x >= 0 && y >= 0 && w >= 240 && h >= 160, "{width}x{height}");
                assert!(x as u32 + w <= width && y as u32 + h <= height);
                let cache = door_app_ui::ImageCache::default();
                let mut pixels = alloc::vec![0; (width * height * 4) as usize];
                let mut v =
                    View::new_scaled(&mut pixels, width, height, &fonts, 1).with_images(&cache);
                app.draw(&mut v);
                assert!(pixels.chunks_exact(4).all(|p| p[3] == 255));
                let directory = std::path::Path::new("../../target/preview-review");
                std::fs::create_dir_all(directory).unwrap();
                let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
                for pixel in pixels.chunks_exact(4) {
                    ppm.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
                }
                let name = if open { "document" } else { "home" };
                std::fs::write(
                    directory.join(format!("default-{width}x{height}-{name}.ppm")),
                    ppm,
                )
                .unwrap();
            }
        }
    }
    impl Preview {
        fn click_close_for_test(&mut self) {
            // Mirrors click(300) without a storage connection.
            if self.tabs.is_empty() {
                self.close_requested = true;
            }
        }
        fn body_for_test(&self) -> (i32, i32) {
            let (x, y, _, _) = self.dimensions();
            (x, y)
        }
    }
    #[test]
    fn home_and_documents_paint_snapped_lines_at_150_percent() {
        use door_workspace::raster_check::{blurred_lines, crisp_lines};
        let zoom = door_workspace::Scale::from_percent(150).unwrap();
        let atlas = door_workspace::assets::get("fonts/segoe-ui/hinted-150.dfa");
        let fonts = door_graphics_text::FontAtlas::from_packed_percent(atlas, 150).unwrap();
        for dark in [false, true] {
            for open in [false, true] {
                let mut app = Preview::new(1001);
                if open {
                    app.tabs.push(document("/users/1001/sunrise.png"));
                    app.tabs.push(document("/users/1001/readme.txt"));
                    app.active = 0;
                }
                app.configure(1000, 640);
                let (w, h) = (zoom.len(1000), zoom.len(640));
                let cache = door_app_ui::ImageCache::default();
                let mut pixels = alloc::vec![0; (w * h * 4) as usize];
                let mut v = View::new_zoomed(&mut pixels, w, h, &fonts, zoom).with_images(&cache);
                assert_eq!((v.width, v.height), (1000, 640));
                v.palette.dark = dark;
                app.draw(&mut v);
                assert!(pixels.chunks_exact(4).all(|p| p[3] == 255), "opaque frame");
                let (w, h) = (w as usize, h as usize);
                assert_eq!(blurred_lines(&pixels, w, h), 0, "open {open} dark {dark}");
                assert!(crisp_lines(&pixels, w, h) > 0, "open {open} dark {dark}");
            }
        }
    }
}
