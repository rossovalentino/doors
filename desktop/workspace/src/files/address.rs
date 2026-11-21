//! Windows 11 address bar: location glyph, breadcrumbs with constant chevron
//! spacing (measured from the 25H2 reference), chevron flyouts that list the
//! subfolders of a segment, an overflow list for segments that do not fit,
//! and path editing (click the empty area, Ctrl+L or Alt+D; Enter/Escape).
use super::*;
use door_workspace::context_menu::{self as cm, Entry, Glyph};
use door_workspace::explorer_path::{CrumbLayout, crumb, crumb_layout, ellipsize};

pub(super) const CRUMB_ICON: u32 = 700;
pub(super) const CRUMB_OVERFLOW: u32 = 701;
pub(super) const CRUMB_ROOT: u32 = 702;
/// Segment text plates, by visible position.
pub(super) const CRUMB_TEXT: u32 = 710;
/// Segment chevrons, by visible position.
pub(super) const CRUMB_CHEVRON: u32 = 740;
const MAX_VISIBLE: usize = 29;

/// Newton square root (no_std has no float sqrt without libm).
fn sqrt(v: f32) -> f32 {
    if v <= 0.0 {
        return 0.0;
    }
    let mut x = if v > 1.0 { v } else { 1.0 };
    for _ in 0..16 {
        x = 0.5 * (x + v / x);
    }
    x
}
/// A 12 px chevron stroked at the device density (1 logical px wide, 4x8
/// ink like the reference), pointing right or, while its flyout is open, down.
pub(super) fn chevron_mask(scale: u32, down: bool) -> Vec<u8> {
    let size = 12 * scale as usize;
    let s = scale as f32;
    let points: [(f32, f32); 3] = if down {
        [(2.5, 4.5), (6.0, 8.0), (9.5, 4.5)]
    } else {
        [(4.5, 2.5), (8.0, 6.0), (4.5, 9.5)]
    };
    let mut mask = alloc::vec![0u8; size * size];
    let half = 0.55 * s;
    for py in 0..size {
        for px in 0..size {
            let (x, y) = (px as f32 + 0.5, py as f32 + 0.5);
            let mut d = f32::MAX;
            for pair in points.windows(2) {
                let (ax, ay) = (pair[0].0 * s, pair[0].1 * s);
                let (bx, by) = (pair[1].0 * s, pair[1].1 * s);
                let (dx, dy) = (bx - ax, by - ay);
                let t = (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
                let (cx, cy) = (ax + t * dx - x, ay + t * dy - y);
                let dist = sqrt(cx * cx + cy * cy);
                if dist < d {
                    d = dist;
                }
            }
            let coverage = (half + 0.5 - d).clamp(0.0, 1.0);
            mask[py * size + px] = (coverage * 255.0 + 0.5) as u8;
        }
    }
    mask
}

impl Files {
    /// Chevron masks are rasterized once per density and direction.
    pub(super) fn chevron(&self, scale: u32, down: bool) -> Vec<u8> {
        let mut cache = self.chevron_cache.borrow_mut();
        if let Some((_, _, mask)) = cache.iter().find(|(s, d, _)| *s == scale && *d == down) {
            return mask.clone();
        }
        let mask = chevron_mask(scale, down);
        cache.push((scale, down, mask.clone()));
        mask
    }
    /// Location glyph and (label, path) chain from the breadcrumb root. Known
    /// folders root their own chain ("Desktop > Doors OS"), like Windows.
    pub(super) fn crumb_chain(&self) -> (Icon, Vec<(String, String)>) {
        if self.overview {
            return (Icon::PC, alloc::vec![("This PC".into(), THIS_PC.into())]);
        }
        if self.home_view {
            return (Icon::Home, alloc::vec![("Home".into(), HOME_PAGE.into())]);
        }
        if self.recycle_view() {
            return (
                self.bin_icon(),
                alloc::vec![("Recycle Bin".into(), RECYCLE_BIN.into())],
            );
        }
        for (i, (name, _)) in FOLDERS.iter().enumerate() {
            let root = format!("{}/{name}", self.home);
            if self.cwd == root || self.cwd.starts_with(&format!("{root}/")) {
                let mut parts = alloc::vec![(String::from(*name), root.clone())];
                let tail = self.cwd[root.len()..].trim_start_matches('/');
                for part in tail.split('/').filter(|s| !s.is_empty()) {
                    let path = format!("{}/{part}", parts.last().unwrap().1);
                    parts.push((part.into(), path));
                }
                return (Self::known_glyph(i), parts);
            }
        }
        let mut parts: Vec<(String, String)> = alloc::vec![("This PC".into(), THIS_PC.into())];
        let suffix = if self.cwd == "/compat" || self.cwd.starts_with("/compat/") {
            parts.push((DRIVES[0].label.into(), "/compat".into()));
            self.cwd.strip_prefix("/compat").unwrap_or("")
        } else if self.cwd == "/users" || self.cwd.starts_with("/users/") {
            parts.push((DRIVES[0].label.into(), "/compat".into()));
            parts.push(("Users".into(), "/users".into()));
            self.cwd.strip_prefix("/users").unwrap_or("")
        } else {
            parts.push(("Doors".into(), "/".into()));
            self.cwd.as_str()
        };
        for name in suffix.split('/').filter(|s| !s.is_empty()) {
            let parent = &parts.last().unwrap().1;
            let path = if parent == "/" {
                format!("/{name}")
            } else {
                format!("{parent}/{name}")
            };
            parts.push((name.into(), path));
        }
        (Icon::PC, parts)
    }
    /// Monochrome location glyph of a pinned known folder.
    pub(super) fn known_glyph(index: usize) -> Icon {
        match FOLDERS.get(index).map(|f| f.0) {
            Some("Desktop") => Icon::PC,
            Some("Documents") => Icon::GlyphDocument,
            Some("Downloads") => Icon::SidebarDownloads,
            Some("Pictures") => Icon::LargeIcons,
            Some("Music") => Icon::GlyphMusic,
            Some("Videos") => Icon::GlyphVideo,
            _ => Icon::GlyphFolder,
        }
    }
    pub(super) fn crumb_geometry(
        &self,
        measure: &dyn Fn(&str) -> u32,
    ) -> (Vec<(String, String)>, CrumbLayout) {
        let (_, chain) = self.crumb_chain();
        let widths: Vec<u32> = chain
            .iter()
            .map(|(label, _)| measure(&ui::display_ascii(label)))
            .collect();
        let mut layout = crumb_layout(self.layout.address(), &widths, measure("..."));
        layout.items.truncate(MAX_VISIBLE);
        (chain, layout)
    }
    pub(super) fn address_target(&self, x: i32, y: i32, fonts: &FontAtlas) -> Option<u32> {
        let address = self.layout.address();
        if !address.contains(x, y) {
            return None;
        }
        if self.editor == 1 {
            return Some(30);
        }
        let measure = |s: &str| {
            fonts
                .measure(s, TextStyle::Regular14)
                .map_or(0, |m| m.advance_width)
        };
        let (_, l) = self.crumb_geometry(&measure);
        if l.icon.contains(x, y) {
            return Some(CRUMB_ICON);
        }
        if l.root.contains(x, y) {
            return Some(CRUMB_ROOT);
        }
        if l.overflow.is_some_and(|r| r.contains(x, y)) {
            return Some(CRUMB_OVERFLOW);
        }
        for (i, item) in l.items.iter().enumerate() {
            if item.text.contains(x, y) {
                return Some(CRUMB_TEXT + i as u32);
            }
            if item.chevron.contains(x, y) {
                return Some(CRUMB_CHEVRON + i as u32);
            }
        }
        Some(30)
    }
    /// Starts editing the location as text with the whole path selected.
    pub(super) fn edit_address(&mut self) {
        self.dismiss_menu();
        self.editor = 1;
        let text = if self.overview {
            "This PC".into()
        } else if self.home_view {
            "Home".into()
        } else if self.recycle_view() {
            "Recycle Bin".into()
        } else {
            explorer_path::display(&self.cwd)
        };
        self.edit.set(text, true);
    }
    pub(super) fn address_click(
        &mut self,
        id: u32,
        storage: &mut Storage,
        fonts: &FontAtlas,
    ) -> Result<(), Error> {
        let measure = |s: &str| {
            fonts
                .measure(s, TextStyle::Regular14)
                .map_or(0, |m| m.advance_width)
        };
        let (chain, l) = self.crumb_geometry(&measure);
        let open = self.crumb_flyout;
        match id {
            CRUMB_ICON | 30 => self.edit_address(),
            CRUMB_ROOT => {
                if open == Some(CRUMB_ROOT) {
                    self.dismiss_menu();
                } else {
                    let mut paths = alloc::vec![String::from(HOME_PAGE)];
                    let mut entries =
                        alloc::vec![Entry::item("Home", cm::Action::Pick(0)).glyph(Glyph::Home)];
                    for i in PINNED_ORDER {
                        paths.push(format!("{}/{}", self.home, FOLDERS[i].0));
                        entries.push(
                            Entry::item(FOLDERS[i].0, cm::Action::Pick(entries.len() as u16))
                                .glyph(Glyph::Known(i as u8)),
                        );
                    }
                    entries.push(Entry::separator());
                    paths.push(String::new());
                    paths.push(THIS_PC.into());
                    entries.push(
                        Entry::item("This PC", cm::Action::Pick(entries.len() as u16))
                            .glyph(Glyph::ThisPc),
                    );
                    for d in self.drives() {
                        paths.push(d.path.into());
                        entries.push(
                            Entry::item(d.label, cm::Action::Pick(entries.len() as u16))
                                .glyph(Glyph::Drive),
                        );
                    }
                    paths.push(String::new());
                    entries.push(
                        Entry::item("Network", cm::Action::Pick(entries.len() as u16))
                            .glyph(Glyph::Network)
                            .gate(Some("No network provider is installed.")),
                    );
                    self.open_flyout(entries, paths, CRUMB_ROOT, l.root, &measure);
                }
            }
            CRUMB_OVERFLOW => {
                if open == Some(CRUMB_OVERFLOW) {
                    self.dismiss_menu();
                } else if let Some(r) = l.overflow {
                    // Hidden ancestors, nearest first (as Windows lists them).
                    let hidden: Vec<_> = chain[..l.hidden].iter().rev().cloned().collect();
                    let entries = hidden
                        .iter()
                        .enumerate()
                        .map(|(i, (label, path))| {
                            Entry::item(label, cm::Action::Pick(i as u16))
                                .glyph(self.path_glyph(path))
                        })
                        .collect();
                    let paths = hidden.into_iter().map(|(_, p)| p).collect();
                    self.open_flyout(entries, paths, CRUMB_OVERFLOW, r, &measure);
                }
            }
            _ if (CRUMB_TEXT..CRUMB_CHEVRON).contains(&id) => {
                let Some(item) = l.items.get((id - CRUMB_TEXT) as usize) else {
                    return Ok(());
                };
                let path = chain[item.index].1.clone();
                self.dismiss_menu();
                if path == THIS_PC {
                    self.go_this_pc(storage)?;
                } else if path == HOME_PAGE {
                    self.go_home(storage)?;
                } else {
                    self.navigate(storage, path)?;
                }
                let _ = door_sys::write(
                    format!(
                        "DOOR_P12 FILES BREADCRUMB path={}\n",
                        if self.overview { THIS_PC } else { &self.cwd }
                    )
                    .as_bytes(),
                );
            }
            _ => {
                let Some(item) = l.items.get((id - CRUMB_CHEVRON) as usize) else {
                    return Ok(());
                };
                if open == Some(id) {
                    self.dismiss_menu();
                    return Ok(());
                }
                let parent = chain[item.index].1.clone();
                let next = chain.get(item.index + 1).map(|(_, p)| p.clone());
                let (mut entries, mut paths) = (Vec::new(), Vec::new());
                if parent == THIS_PC {
                    for d in self.drives() {
                        entries.push(
                            Entry::item(d.label, cm::Action::Pick(paths.len() as u16))
                                .glyph(Glyph::Drive),
                        );
                        paths.push(String::from(d.path));
                    }
                } else if parent == HOME_PAGE {
                    // Home's chevron lists Quick access.
                    for (path, name, _) in self.quick_access() {
                        entries.push(
                            Entry::item(&name, cm::Action::Pick(paths.len() as u16))
                                .glyph(Glyph::Folder),
                        );
                        paths.push(path);
                    }
                } else {
                    match Self::read_entries(storage, &parent) {
                        Ok(mut listed) => {
                            self.hide_tag_metadata(&parent, &mut listed);
                            listed.retain(|e| e.info.directory);
                            listed.sort_by(|a, b| {
                                explorer_sort::natural(&a.name[..a.name_len], &b.name[..b.name_len])
                            });
                            for e in listed {
                                let name = Self::name(&e);
                                let path = if parent == "/compat" && name == "Users" {
                                    "/users".into()
                                } else if parent == "/" {
                                    format!("/{name}")
                                } else {
                                    format!("{parent}/{name}")
                                };
                                let current = next.as_deref() == Some(path.as_str());
                                entries.push(
                                    Entry::item(&name, cm::Action::Pick(paths.len() as u16))
                                        .glyph(Glyph::Folder)
                                        .checked(current),
                                );
                                paths.push(path);
                            }
                        }
                        Err(error) => {
                            self.message = Self::location_error(&error).into();
                            return Ok(());
                        }
                    }
                }
                if entries.is_empty() {
                    entries.push(
                        Entry::item("No subfolders", cm::Action::Pick(0))
                            .glyph(Glyph::None)
                            .gate(Some("This folder has no subfolders.")),
                    );
                    paths.push(String::new());
                }
                self.open_flyout(entries, paths, id, item.chevron, &measure);
            }
        }
        Ok(())
    }
    fn path_glyph(&self, path: &str) -> Glyph {
        if path == THIS_PC {
            Glyph::ThisPc
        } else if self.drives().iter().any(|d| d.path == path) {
            Glyph::Drive
        } else {
            Glyph::Folder
        }
    }
    /// Opens a location flyout under `anchor` (a chevron or button plate).
    pub(super) fn open_flyout(
        &mut self,
        entries: Vec<Entry>,
        paths: Vec<String>,
        owner: u32,
        anchor: Bounds,
        measure: &dyn Fn(&str) -> u32,
    ) {
        let widest = entries
            .iter()
            .map(|e| {
                measure(&ui::display_ascii(&e.label))
                    + if e.shortcut.is_empty() {
                        0
                    } else {
                        measure(e.shortcut) + 32
                    }
            })
            .max()
            .unwrap_or(0);
        let menu = cm::flyout(entries, widest + 80);
        self.xmenu = Some(cm::MenuState::open(
            menu,
            (anchor.x, anchor.y + anchor.height as i32 + 4),
            (self.layout.width, self.layout.height),
            false,
        ));
        self.menu_for = MenuFor::Locations(paths);
        self.crumb_flyout = Some(owner);
        self.editor = 0;
    }
    pub(super) fn draw_address(&self, v: &mut ui::View<'_>) {
        let address = self.layout.address();
        let dark = v.palette.dark;
        let ink = if dark { [255; 4] } else { [26, 26, 26, 255] };
        let (glyph, _) = self.crumb_chain();
        let icon_y = address.y + (address.height as i32 - 16) / 2;
        if self.editor == 1 {
            v.mask_pixels(
                address.x + crumb::INSET + 8,
                icon_y,
                16,
                glyph.mask_scaled(16, v.scale),
                ink,
                None,
            );
            self.draw_editor(
                v,
                address.x + crumb::INSET + crumb::ICON as i32,
                address.y + (address.height as i32 - 20) / 2,
                address.width.saturating_sub(crumb::ICON + 16),
            );
            return;
        }
        let (_, l, labels) = {
            let measure = |s: &str| v.measure(s);
            let (chain, l) = self.crumb_geometry(&measure);
            let labels: Vec<String> = l
                .items
                .iter()
                .map(|item| {
                    ellipsize(
                        &ui::display_ascii(&chain[item.index].0),
                        item.label_width,
                        measure,
                    )
                })
                .collect();
            (chain, l, labels)
        };
        let (hover, press) = (v.hover, v.pressed);
        let hovered = move |r: Bounds| hover.is_some_and(|(x, y)| r.contains(x, y));
        let pressed = move |r: Bounds| press.is_some_and(|(x, y)| r.contains(x, y));
        let plate = |v: &mut ui::View<'_>, r: Bounds, open: bool| {
            let down = pressed(r) || open;
            if down || hovered(r) {
                let (light, dark) = if down {
                    ([228, 227, 225, 255], [64, 56, 50, 255])
                } else {
                    ([234, 233, 231, 255], [72, 63, 56, 255])
                };
                super::modern::paint::plate(v, r, 4, light, dark);
            }
        };
        let chevron = |v: &mut ui::View<'_>, r: Bounds, open: bool| {
            let mask = self.chevron(v.scale, open);
            v.mask_pixels(
                r.x + (r.width as i32 - 12) / 2,
                r.y + (r.height as i32 - 12) / 2,
                12,
                &mask,
                ink,
                None,
            );
        };
        plate(v, l.icon, false);
        v.mask_pixels(
            l.icon.x + 8,
            icon_y,
            16,
            glyph.mask_scaled(16, v.scale),
            ink,
            None,
        );
        let open = self.crumb_flyout.filter(|_| self.xmenu.is_some());
        plate(v, l.root, open == Some(CRUMB_ROOT));
        chevron(v, l.root, open == Some(CRUMB_ROOT));
        // Caps centred on the glyph row (`centered_text_top` centres cap
        // height); labels start PAD in, ending PAD_END before the chevron.
        let text_row = (address.y, address.height);
        if let Some(r) = l.overflow {
            plate(v, r, open == Some(CRUMB_OVERFLOW));
            v.text_clip_centered(
                r.x + crumb::PAD as i32,
                text_row,
                r.width,
                "...",
                TextStyle::Regular14,
                ink,
            );
            let c = Bounds {
                x: r.x + r.width as i32,
                width: crumb::CHEVRON,
                ..r
            };
            chevron(v, c, false);
        }
        for (i, item) in l.items.iter().enumerate() {
            let label = &labels[i];
            plate(v, item.text, false);
            v.text_clip_centered(
                item.text.x + crumb::PAD as i32,
                text_row,
                item.label_width + 2,
                label,
                TextStyle::Regular14,
                ink,
            );
            let id = CRUMB_CHEVRON + i as u32;
            plate(v, item.chevron, open == Some(id));
            chevron(v, item.chevron, open == Some(id));
        }
    }
}
