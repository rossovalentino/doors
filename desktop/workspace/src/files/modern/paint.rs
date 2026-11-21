use super::*;
pub(in crate::files) const TEXT: [u8; 4] = [24, 24, 24, 255];
pub(in crate::files) const MUTED: [u8; 4] = [112, 112, 112, 255];
pub(in crate::files) const ACCENT: [u8; 4] = [196, 103, 0, 255];
pub(in crate::files) const SELECTED: [u8; 4] = [250, 233, 213, 255];
pub(in crate::files) const HOVER: [u8; 4] = [249, 243, 235, 255];
/// A tag's colour dot: its own hue in light and dark; a contrast theme
/// paints it as coloured ink (Hotlight), as forced colours do.
pub(in crate::files) fn tag_color(v: &ui::View<'_>, tag: usize) -> [u8; 4] {
    let c = door_workspace::explorer_tags::color(tag, v.palette.dark);
    if v.palette.contrast.is_some() {
        v.palette.foreground(c)
    } else {
        c
    }
}
/// A literal light/dark fill painted with `rounded_asset`: exact in light
/// and dark; a contrast theme maps its light role to the system colours.
pub(in crate::files) fn fill(v: &ui::View<'_>, light: [u8; 4], dark: [u8; 4]) -> [u8; 4] {
    if v.palette.contrast.is_some() {
        v.palette.surface(light)
    } else if v.palette.dark {
        dark
    } else {
        light
    }
}
/// A hover or pressed plate behind ink: its literal light/dark fill; a
/// contrast theme outlines it in Highlight instead, so the ink keeps
/// WindowText-on-Window contrast.
pub(in crate::files) fn plate(
    v: &mut ui::View<'_>,
    r: Bounds,
    radius: u32,
    light: [u8; 4],
    dark: [u8; 4],
) {
    match v.palette.system() {
        Some(s) => {
            let rgb = |c: [u8; 3]| [c[2], c[1], c[0], 255];
            v.rounded_asset(r.x, r.y, r.width, r.height, radius, rgb(s.highlight));
            v.rounded_asset(
                r.x + 1,
                r.y + 1,
                r.width.saturating_sub(2),
                r.height.saturating_sub(2),
                radius.saturating_sub(1),
                rgb(s.window),
            );
        }
        None => v.rounded_asset(
            r.x,
            r.y,
            r.width,
            r.height,
            radius,
            if v.palette.dark { dark } else { light },
        ),
    }
}
pub(in crate::files) fn line(v: &mut ui::View<'_>, x: i32, y: i32, w: u32) {
    v.rect(x, y, w, 1, [238, 235, 231, 255]);
}
/// Windows' filled pin (Segoe Fluent E840 Pinned + E841 PinnedFill) in its
/// 11 px box, as the navigation pane and Quick access tiles draw it.
pub(in crate::files) fn pinned(v: &mut ui::View<'_>, x: i32, y: i32, color: [u8; 4]) {
    let mask = Icon::Pinned.mask_scaled(11, v.scale);
    v.mask_pixels(x, y, 11, mask, color, None);
}
/// One line of text in `w` px; text that does not fit ends with "..." at
/// the edge, as Windows truncates, instead of a glyph cut in half.
pub(in crate::files) fn label(
    v: &mut ui::View<'_>,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    text: &str,
    color: [u8; 4],
) {
    let text = fit(v, text, w);
    v.text_clip_centered(x, (y, h), w, &text, TextStyle::Regular14, color);
}
/// The text `label` paints in `w` px.
pub(in crate::files) fn fit<'t>(
    v: &ui::View<'_>,
    text: &'t str,
    w: u32,
) -> alloc::borrow::Cow<'t, str> {
    if v.measure(text) <= w {
        alloc::borrow::Cow::Borrowed(text)
    } else {
        alloc::borrow::Cow::Owned(door_workspace::explorer_path::ellipsize(text, w, |s| {
            v.measure(s)
        }))
    }
}
pub(in crate::files) fn wrap(
    v: &mut ui::View<'_>,
    x: i32,
    y: i32,
    w: u32,
    text: &str,
    limit: usize,
    color: [u8; 4],
) {
    let text = ui::display_ascii(text);
    for (i, range) in door_workspace::dialog::lines(&text, w, |s| v.measure(s))
        .into_iter()
        .take(limit)
        .enumerate()
    {
        label(v, x, y + i as i32 * 18, w, 18, &text[range], color);
    }
}
pub(in crate::files) fn control(
    v: &mut ui::View<'_>,
    r: Bounds,
    icon: Icon,
    text: &str,
    selected: bool,
) {
    let hover = v.hover.is_some_and(|(x, y)| r.contains(x, y));
    if hover || selected {
        v.rounded(
            r.x,
            r.y,
            r.width,
            r.height,
            5,
            if selected { SELECTED } else { HOVER },
        );
    }
    // Icon-only controls centre their glyph; labelled action rows lead with it.
    let icon_x = if text.is_empty() {
        r.x + (r.width as i32 - 16) / 2
    } else {
        r.x + 10
    };
    v.compact_icon(
        icon_x,
        r.y + (r.height as i32 - 16) / 2,
        icon,
        if selected { ACCENT } else { TEXT },
    );
    if !text.is_empty() {
        label(
            v,
            r.x + 34,
            r.y,
            r.width.saturating_sub(40),
            r.height,
            text,
            TEXT,
        );
    }
}
fn command_glyph(v: &mut ui::View<'_>, x: i32, y: i32, id: u32, icon: Icon, enabled: bool) {
    let foreground = if v.palette.dark {
        if enabled {
            [204, 204, 204, 255]
        } else {
            [85, 84, 83, 255]
        }
    } else if enabled {
        TEXT
    } else {
        [165, 165, 165, 255]
    };
    let accent = if v.palette.dark {
        if enabled {
            [194, 153, 108, 255]
        } else {
            [93, 65, 40, 255]
        }
    } else if enabled {
        [177, 119, 0, 255]
    } else {
        [175, 155, 131, 255]
    };
    if id == 910 {
        // Literal glyph colours; a contrast theme maps their light roles.
        let contrast = v.palette.contrast.is_some();
        let stroke = if contrast {
            v.palette.foreground(foreground)
        } else {
            foreground
        };
        let face = fill(v, [250, 252, 253, 255], [25, 25, 25, 255]);
        let lines = if contrast {
            v.palette.foreground([220, 140, 0, 255])
        } else if v.palette.dark {
            [235, 195, 55, 255]
        } else {
            [220, 140, 0, 255]
        };
        v.rounded_asset(x, y, 16, 14, 3, stroke);
        v.rounded_asset(x + 1, y + 1, 14, 12, 2, face);
        v.rounded_asset(x + 10, y + 1, 1, 12, 0, stroke);
        for top in [3, 6, 9] {
            v.rounded_asset(x + 12, y + top, 2, 1, 0, lines);
        }
        return;
    }
    let mask = icon.mask_scaled(16, v.scale);
    v.mask_pixels(x, y, 16, mask, foreground, None);
    let region = match id {
        8 => Some((5, 5, 6, 6)),
        44 => Some((0, 8, 16, 8)),
        45 => Some((7, 0, 9, 11)),
        46 => Some((7, 4, 7, 12)),
        33 => Some((0, 0, 7, 16)),
        47 => Some((7, 0, 9, 9)),
        5 => Some((0, 0, 7, 16)),

        _ => None,
    };
    if let Some((left, top, w, h)) = region
        && let Ok(clip) = door_graphics_core::Rect::new(x + left, y + top, w, h)
    {
        v.viewport(clip, 0);
        v.mask_pixels(x, y, 16, mask, accent, None);
        v.reset_viewport();
    }
}
impl Files {
    pub(in crate::files) fn draw_modern(&self, v: &mut ui::View<'_>) {
        v.explorer_style = true;
        v.rect(0, 0, v.width, v.height, [255; 4]);
        v.rect(
            0,
            0,
            v.width,
            xl::command_top() as u32,
            [250, 247, 243, 255],
        );
        v.rect(0, 0, v.width, 40, [243, 235, 225, 255]);
        for i in self.visible_tabs() {
            let r = self.tab_bounds(i);
            v.document_tab(
                r.x,
                r.y,
                r.width,
                r.height,
                self.tab_icon(i),
                self.tab_label(i),
                i == self.active_tab,
            );
        }
        for (r, id) in self.caption_controls() {
            control(
                v,
                r,
                match id {
                    897 => Icon::Back,
                    898 => Icon::Forward,
                    _ => Icon::Add,
                },
                "",
                false,
            );
        }
        for (i, (icon, enabled)) in [
            (Icon::Back, self.history_index > 0),
            (Icon::Forward, self.history_index + 1 < self.history.len()),
            (
                Icon::Up,
                !self.overview && !self.home_view && !matches!(self.cwd.as_str(), "/"),
            ),
            (Icon::Refresh, true),
        ]
        .into_iter()
        .enumerate()
        {
            let r = ExplorerLayout::navigation(i).unwrap();
            if enabled && v.hover.is_some_and(|(x, y)| r.contains(x, y)) {
                let down = v.pressed.is_some_and(|(x, y)| r.contains(x, y));
                v.rounded(
                    r.x,
                    r.y,
                    r.width,
                    r.height,
                    4,
                    if down { SELECTED } else { HOVER },
                );
            }
            let ink = if v.palette.dark {
                [255; 4]
            } else {
                [26, 26, 26, 255]
            };
            v.mask_pixels(
                r.x + (r.width as i32 - 16) / 2,
                r.y + (r.height as i32 - 16) / 2,
                16,
                icon.mask_scaled(16, v.scale),
                if enabled {
                    ink
                } else if v.palette.dark {
                    [110, 110, 110, 255]
                } else {
                    [180, 180, 180, 255]
                },
                None,
            );
        }
        let address = self.layout.address();
        let search = self.layout.search();
        for r in [address, search] {
            v.rounded(r.x, r.y, r.width, r.height, 5, [230, 226, 221, 255]);
            v.rounded(
                r.x + 1,
                r.y + 1,
                r.width.saturating_sub(2),
                r.height.saturating_sub(2),
                4,
                [250, 249, 247, 255],
            );
        }
        self.draw_address(v);
        v.compact_icon(
            search.x + search.width as i32 - 28,
            search.y + (search.height as i32 - 16) / 2,
            Icon::Search,
            MUTED,
        );
        if self.editor == 2 {
            self.draw_editor(
                v,
                search.x + 12,
                search.y + (search.height as i32 - 20) / 2,
                search.width.saturating_sub(42),
            );
        } else {
            let text = if self.search.is_empty() {
                format!("Search {}", self.title())
            } else {
                self.search.clone()
            };
            label(
                v,
                search.x + 12,
                search.y,
                search.width.saturating_sub(42),
                search.height,
                &text,
                MUTED,
            );
        }
        if matches!(self.editor, 1 | 2) {
            let r = if self.editor == 1 { address } else { search };
            v.rounded_asset(
                r.x + 1,
                r.y + r.height as i32 - 2,
                r.width.saturating_sub(2),
                2,
                1,
                fill(v, ACCENT, [255, 183, 96, 255]),
            );
        }
        line(v, 0, xl::command_top() - 1, v.width);
        let end = self.content_layout().width;
        v.rect(
            0,
            xl::command_top(),
            v.width,
            xl::command_height(),
            [253, 252, 250, 255],
        );
        for (r, id, icon, text, enabled) in self.command_controls() {
            let hover = enabled && v.hover.is_some_and(|(x, y)| r.contains(x, y));
            if hover {
                v.rounded(r.x, r.y, r.width, r.height, 4, HOVER);
            }
            let foreground = if enabled { TEXT } else { [155, 155, 155, 255] };
            // Icon, label and dropdown chevron are one centred content group.
            let chevron = matches!(id, 8 | 5 | 6);
            let (icon_x, text_x, chevron_x) = v.content_group(
                r.x,
                r.width,
                (16, 10),
                text,
                TextStyle::Regular14,
                (8, if chevron { 12 } else { 0 }),
                8,
            );
            command_glyph(
                v,
                icon_x,
                r.y + (r.height as i32 - 16) / 2,
                id,
                icon,
                enabled,
            );
            if !text.is_empty() {
                label(
                    v,
                    text_x,
                    r.y,
                    (r.x + r.width as i32 - text_x).max(0) as u32,
                    r.height,
                    text,
                    foreground,
                );
            }
            if chevron {
                v.tiny_icon(
                    chevron_x,
                    r.y + (r.height as i32 - 12) / 2,
                    Icon::Chevron,
                    foreground,
                );
            }
        }
        for x in self.command_separators() {
            if x < v.width as i32 - 116 {
                v.rect(
                    x,
                    xl::command_top() + 7 + xl::grow() / 2,
                    1,
                    30,
                    [238, 235, 231, 255],
                );
            }
        }
        line(v, 0, content_top() - 1, v.width);
        self.draw_rail(v);
        if let Ok(clip) = door_graphics_core::Rect::new(
            self.layout.sidebar,
            content_top(),
            end.saturating_sub(self.layout.sidebar as u32),
            v.height
                .saturating_sub(content_top() as u32 + xl::status_height()),
        ) {
            v.viewport(clip, 0);
            self.draw_items(v);
            v.reset_viewport();
        }
        if self.editor == 3
            && let Some(r) = self.rename_bounds()
        {
            v.rounded(
                r.x - 4,
                r.y - 2,
                r.width + 8,
                r.height + 4,
                5,
                [160, 160, 160, 255],
            );
            v.rounded(r.x - 3, r.y - 1, r.width + 6, r.height + 2, 4, [255; 4]);
            self.draw_editor(v, r.x, r.y, r.width);
        }
        if let Some(r) = self.pane() {
            self.draw_pane(v, r);
        }
        let bottom = v.height as i32 - xl::status_height() as i32;
        let bar = xl::status_height();
        v.rect(0, bottom, v.width, bar, [253, 251, 248, 255]);
        line(v, 0, bottom, v.width);
        let count = self.item_count();
        let status = if !self.message.is_empty() && self.search.is_empty() {
            self.message.clone()
        } else if self.volume_search() {
            self.search_scope()
        } else {
            format!("{count} {}", if count == 1 { "item" } else { "items" })
        };
        label(
            v,
            14,
            bottom,
            v.width.saturating_sub(76),
            bar,
            &status,
            MUTED,
        );
        let x = 30 + v.measure(&status) as i32;
        let picked = self.selection_count();
        if picked > 0 {
            label(
                v,
                x,
                bottom,
                (v.width as i32 - 60 - x).max(0) as u32,
                bar,
                &if picked == 1 {
                    String::from("1 item selected")
                } else {
                    format!("{picked} items selected")
                },
                MUTED,
            );
        }
        for (icons, icon, mode) in [
            (false, Icon::Details, ViewMode::Details),
            (true, Icon::LargeIcons, ViewMode::Large),
        ] {
            let r = self.layout.status_view(icons);
            if self.view_mode() == mode {
                v.rounded(r.x, r.y, r.width, r.height, 3, SELECTED);
            }
            v.compact_icon(
                r.x + 2,
                r.y + 3,
                icon,
                if self.view_mode() == mode {
                    ACCENT
                } else {
                    MUTED
                },
            );
        }
    }
    pub(in crate::files) fn close_glyph(
        &self,
        v: &mut ui::View<'_>,
        x: i32,
        y: i32,
        color: [u8; 4],
    ) {
        v.tiny_icon(x - 6, y - 6, Icon::Close, color);
    }
    fn rail_tree_row(&self, v: &mut ui::View<'_>, row: &TreeRow, r: Bounds, active: bool) {
        if row.node == TreeNode::Separator {
            // Windows' group rule runs 11..149 in its 157 px pane (E4).
            line(
                v,
                11,
                r.y + r.height as i32 / 2,
                (self.layout.sidebar - 19) as u32,
            );
            return;
        }
        let hovered = v
            .hover
            .is_some_and(|(x, y)| x < self.layout.sidebar && r.contains(x, y));
        if active || hovered {
            // TreeViewItem plates use the 4 px control corner radius.
            v.rounded(
                r.x,
                r.y,
                r.width,
                r.height,
                4,
                if active { SELECTED } else { HOVER },
            );
        }
        if row.expandable {
            let c = Self::rail_chevron(row, r);
            let (x, y) = (c.x + 5, r.y + (r.height as i32 - 12) / 2);
            v.tiny_icon(
                x,
                y,
                if row.expanded {
                    Icon::Chevron
                } else {
                    Icon::ChevronRight
                },
                MUTED,
            );
        }
        let icon = match &row.node {
            TreeNode::Pin(i) => PINNED_ORDER.get(*i).map_or(Icon::Folder, |k| FOLDERS[*k].1),
            TreeNode::Home => Icon::Home,
            TreeNode::ThisPc => Icon::Computer,
            TreeNode::Drive { .. } => Icon::Drive,
            TreeNode::Network => Icon::Network,
            TreeNode::NoneDrive { .. } => Icon::NoneDrive,
            TreeNode::Linux => Icon::Linux,
            TreeNode::RecycleBin => self.bin_icon(),
            _ => Self::folder_icon(&row.label),
        };
        // Windows 11 26H2 at 150 %: 16 px icons 51 px from the window edge,
        // labels 30 px after the icon's left edge (a 6 px gap), 12 px per
        // level (34, 20 and 8 logical).
        let x = 34 + i32::from(row.depth) * explorer_tree::INDENT;
        // Icons keep their size, centred in the (text-grown) 30 px row like
        // the label's cap band, so both share one centre line.
        v.compact_icon(x, r.y + (r.height as i32 - 16) / 2, icon, TEXT);
        let is_pin = matches!(row.node, TreeNode::Pin(_));
        // The filled pin is right-aligned in an 11 px box 14 px from the
        // pane edge (Windows 214 of 231 px at 150 %); the label ends 4 px
        // before it.
        let pin_x = self.layout.sidebar - 14;
        let end = if is_pin {
            pin_x - 4
        } else {
            self.layout.sidebar - 8
        };
        label(
            v,
            x + 20,
            r.y,
            (end - x - 20).max(0) as u32,
            r.height,
            &row.label,
            TEXT,
        );
        if is_pin {
            pinned(v, pin_x, r.y + (r.height as i32 - 11) / 2, MUTED);
        }
    }
    fn draw_rail(&self, v: &mut ui::View<'_>) {
        v.rect(
            0,
            content_top(),
            self.layout.sidebar as u32,
            v.height
                .saturating_sub(content_top() as u32 + xl::status_height()),
            [246, 240, 233, 255],
        );
        v.rect(
            self.layout.sidebar - 1,
            content_top(),
            1,
            v.height
                .saturating_sub(content_top() as u32 + xl::status_height()),
            [233, 228, 222, 255],
        );
        let hover = v.hover;
        v.hover = hover.map(|(x, y)| {
            (
                x,
                if x < self.layout.sidebar {
                    y + self.rail_offset
                } else {
                    y
                },
            )
        });
        if let Ok(clip) = door_graphics_core::Rect::new(
            0,
            content_top(),
            self.layout.sidebar as u32,
            v.height
                .saturating_sub(content_top() as u32 + xl::status_height()),
        ) {
            v.viewport(clip, self.rail_offset);
        }
        let rows = self.rail_rows();
        let selected = self.rail_selected(&rows);
        for (i, (row, r)) in rows.iter().zip(self.rail_row_bounds(&rows)).enumerate() {
            self.rail_tree_row(v, row, r, selected == Some(i));
        }
        // "Your tags" always shows, with Create tag even before the first.
        {
            let tag_top = self.rail_tag_top();
            line(v, 12, tag_top, (self.layout.sidebar - 19).max(0) as u32);
            label(
                v,
                20,
                tag_top + 12,
                (self.layout.sidebar - 28).max(0) as u32,
                (24 + xl::grow()) as u32,
                "Your tags",
                MUTED,
            );
            for (i, name) in self.tags.names.iter().enumerate() {
                let r = Bounds {
                    x: 2,
                    y: tag_top + tag_head() + i as i32 * tag_pitch(),
                    width: self.layout.sidebar as u32 - 6,
                    height: tag_pitch() as u32,
                };
                if self.editor == 5 && self.tag_rename == Some(i) {
                    v.rounded(
                        r.x + 6,
                        r.y + 1,
                        r.width - 10,
                        r.height - 2,
                        4,
                        [214, 210, 204, 255],
                    );
                    v.rounded(r.x + 7, r.y + 2, r.width - 12, r.height - 4, 3, [255; 4]);
                    self.draw_editor(v, r.x + 14, r.y + (r.height as i32 - 20) / 2, r.width - 26);
                    v.rect(r.x + 9, r.y + r.height as i32 - 3, r.width - 16, 2, ACCENT);
                    continue;
                }
                let hovered = v
                    .hover
                    .is_some_and(|(x, y)| x < self.layout.sidebar && r.contains(x, y));
                if self.tag_filter == Some(i) || hovered {
                    v.rect(
                        r.x,
                        r.y,
                        r.width,
                        r.height,
                        if self.tag_filter == Some(i) {
                            SELECTED
                        } else {
                            HOVER
                        },
                    );
                }
                v.rounded_asset(
                    24,
                    r.y + (r.height as i32 - 8) / 2,
                    8,
                    8,
                    4,
                    tag_color(v, i),
                );
                label(
                    v,
                    42,
                    r.y,
                    (self.layout.sidebar - 48).max(0) as u32,
                    r.height,
                    &ui::display_ascii(name),
                    TEXT,
                );
            }
            let y = tag_top + tag_head() + self.tags.names.len() as i32 * tag_pitch();
            if self.tags_error.is_empty() {
                control(
                    v,
                    Bounds {
                        x: 2,
                        y,
                        width: self.layout.sidebar as u32 - 6,
                        height: (32 + xl::grow()) as u32,
                    },
                    Icon::Add,
                    "Create tag",
                    false,
                );
            } else {
                label(
                    v,
                    20,
                    y,
                    (self.layout.sidebar - 26).max(0) as u32,
                    tag_pitch() as u32,
                    "Tags unavailable",
                    MUTED,
                );
            }
        }
        v.reset_viewport();
        v.hover = hover;
        // Like Windows' navigation pane, the scroll bar shows only while the
        // pointer is over the pane (or dragging it), so it never covers the
        // pins otherwise.
        let in_pane = hover.is_some_and(|(x, y)| {
            x < self.layout.sidebar
                && y >= content_top()
                && y < v.height as i32 - xl::status_height() as i32
        });
        if let Some((track, thumb)) = self
            .rail_scrollbar()
            .filter(|_| in_pane || self.rail_drag.is_some())
        {
            let over = hover.is_some_and(|(x, y)| track.contains(x, y)) || self.rail_drag.is_some();
            v.rounded(
                thumb.x + if over { 2 } else { 4 },
                thumb.y,
                if over { 6 } else { 3 },
                thumb.height,
                2,
                [172, 172, 172, 255],
            );
        }
    }
    fn draw_items(&self, v: &mut ui::View<'_>) {
        if self.home_view {
            self.draw_home(v);
            return;
        }
        let layout = self.content_layout();
        let mode = self.view_mode();
        if self.overview && self.search.is_empty() {
            self.draw_drives(v, layout, mode);
        } else {
            if mode == ViewMode::Details {
                self.draw_columns(v, layout);
            }
            for (slot, (index, entry)) in self
                .shown()
                .skip(self.offset)
                .take(layout.capacity(mode))
                .enumerate()
            {
                let Some(cell) = self.item_cell(slot) else {
                    break;
                };
                self.draw_entry(v, layout, mode, cell, entry, self.is_selected(index));
            }
            if self.shown().next().is_none() {
                let text = if self.search.is_empty() {
                    "This folder is empty."
                } else {
                    "No items match your search."
                };
                // Windows centres it across the item view, under the header.
                let span = layout.width.saturating_sub(self.layout.sidebar as u32);
                let tw = v.measure(text).min(span);
                label(
                    v,
                    self.layout.sidebar + (span - tw) as i32 / 2,
                    layout.view_top(mode) + 4,
                    tw + 4,
                    (32 + xl::grow()) as u32,
                    text,
                    TEXT,
                );
            }
        }
        let count = self.item_count();
        let capacity = layout.capacity(mode);
        if count > capacity && capacity > 0 {
            let track = v.height.saturating_sub(content_top() as u32 + 36);
            let thumb = (track as usize * capacity / count)
                .max(20)
                .min(track as usize) as u32;
            let pos = self.offset.min(count - capacity) as u32 * (track - thumb)
                / (count - capacity) as u32;
            v.rounded(
                layout.width as i32 - 6,
                content_top() + 6 + pos as i32,
                3,
                thumb,
                1,
                [185, 185, 185, 255],
            );
        }
    }
    /// Column header strip for Details: sortable labels with the active
    /// column's caret and hairline dividers on the resize handles.
    fn draw_columns(&self, v: &mut ui::View<'_>, layout: ExplorerLayout) {
        let hover = v.hover;
        for (key, r) in self.detail_columns(layout) {
            if hover.is_some_and(|(x, y)| r.contains(x, y)) {
                v.rect(r.x, r.y, r.width, r.height, HOVER);
            }
            let (text_x, width) = match key {
                // Size is right-aligned, like its values.
                SortKey::Size => {
                    let w = v.measure(key.label()).min(r.width.saturating_sub(20));
                    (r.x + r.width as i32 - 12 - w as i32, w)
                }
                SortKey::Name => (r.x + 30, r.width.saturating_sub(38)),
                _ => (r.x + 8, r.width.saturating_sub(16)),
            };
            label(v, text_x, r.y, width, r.height, key.label(), MUTED);
            if self.sort_key == key {
                let x = r.x + r.width as i32 / 2 - 6;
                if self.descending {
                    v.tiny_icon(x, r.y - 1, Icon::Chevron, MUTED);
                } else {
                    v.tiny_icon_flipped(x, r.y - 1, Icon::Chevron, MUTED);
                }
            }
            v.rect(
                r.x + r.width as i32 - 1,
                r.y + 6,
                1,
                r.height - 12,
                [229, 229, 229, 255],
            );
        }
        // No rule under the header (E8): Windows separates it from the
        // first row by a 6 px gap only.
    }
    pub(in crate::files) fn item_fill(&self, v: &mut ui::View<'_>, r: Bounds, selected: bool) {
        if selected {
            v.rounded(r.x, r.y, r.width, r.height, 4, [240, 220, 197, 255]);
            v.rounded(
                r.x + 1,
                r.y + 1,
                r.width.saturating_sub(2),
                r.height.saturating_sub(2),
                3,
                SELECTED,
            );
        } else if v.hover.is_some_and(|(x, y)| r.contains(x, y)) {
            v.rounded(r.x, r.y, r.width, r.height, 4, HOVER);
        }
    }
    /// Type icon or, at 48 px and above, the cached thumbnail or the
    /// folder's content-aware icon (`CachedFolder`): plain when empty or not
    /// yet listed, paper sheets when it holds items, and in Medium and Large
    /// icons the first image's thumbnail in a white frame between the
    /// folder's back and front plates.
    fn draw_item_icon(
        &self,
        v: &mut ui::View<'_>,
        r: Bounds,
        icon: Icon,
        mode: ViewMode,
        entry: Option<&DirEntry>,
    ) {
        // A program draws its own icon, as in Windows.
        let program = entry.map(|e| (Self::name(e), e.info));
        let size = if r.width <= 16 {
            16
        } else {
            r.width.min(r.height)
        };
        if self.draw_program_icon(
            v,
            r.x,
            r.y,
            size,
            program.as_ref().map(|(n, i)| (n.as_str(), *i)),
        ) {
            return;
        }
        if r.width <= 16 {
            v.compact_icon(r.x, r.y, icon, TEXT);
            return;
        }
        if mode.thumbnails()
            && let Some(entry) = entry
        {
            if let Some(image) = self.cached(entry.info).and_then(|p| p.image.as_ref()) {
                v.image_fit(
                    (r.x, r.y, r.width, r.height),
                    image.width,
                    image.height,
                    &image.bgra,
                );
                return;
            }
            let look = self.folder_look(entry, mode);
            if look != super::FolderLook::Plain && matches!(icon, Icon::Folder) {
                let thumbnail = match look {
                    super::FolderLook::Thumbnail(info) => {
                        self.cached(info).and_then(|p| p.image.as_ref())
                    }
                    _ => None,
                };
                if let Some(image) = thumbnail {
                    let size = r.width;
                    let layers = Icon::folder_layers(size);
                    let (cx, cy, cw, ch) = layers.content;
                    let frame = (size / 48).clamp(1, 2);
                    v.hero_icon(r.x, r.y, size, Icon::FolderBack);
                    v.rounded_asset(r.x + cx, r.y + cy, cw, ch, 1, [255; 4]);
                    v.image_cover(
                        (
                            r.x + cx + frame as i32,
                            r.y + cy + frame as i32,
                            cw.saturating_sub(2 * frame),
                            ch.saturating_sub(2 * frame),
                        ),
                        image.width,
                        image.height,
                        &image.bgra,
                    );
                    v.hero_icon(r.x, r.y, size, Icon::FolderFront);
                } else {
                    v.sized_icon(r.x, r.y, r.width, Icon::FolderFilled);
                }
                return;
            }
        }
        v.sized_icon(r.x, r.y, r.width, icon);
    }
    fn draw_entry(
        &self,
        v: &mut ui::View<'_>,
        layout: ExplorerLayout,
        mode: ViewMode,
        cell: Bounds,
        entry: &DirEntry,
        selected: bool,
    ) {
        let name = ui::display_ascii(&Self::name(entry));
        let directory = entry.info.directory;
        let icon = if directory {
            Self::folder_icon(&name)
        } else {
            Icon::for_file_name(&name)
        };
        let kind = previewing::type_name(&name, directory);
        let kind = kind.as_str();
        let size = if !directory {
            Some(previewing::size(entry.info.bytes))
        } else if mode == ViewMode::Details {
            self.folder_size_text(entry)
        } else {
            None
        };
        self.item_fill(v, cell, selected);
        let icon_box = layout.icon_bounds(cell, mode);
        self.draw_item_icon(v, icon_box, icon, mode, Some(entry));
        let text = layout.label_bounds(cell, mode);
        // Label lines grow with the text size (`ExplorerLayout` cells do too).
        let g = xl::grow();
        match mode {
            ViewMode::ExtraLarge | ViewMode::Large | ViewMode::Medium => {
                let lines =
                    door_workspace::dialog::lines(&name, text.width, |line| v.measure(line));
                for (line, range) in lines.into_iter().take(mode.label_lines()).enumerate() {
                    let part = &name[range];
                    let width = v.measure(part).min(text.width);
                    label(
                        v,
                        text.x + (text.width - width) as i32 / 2,
                        text.y + line as i32 * (18 + g),
                        width,
                        (20 + g) as u32,
                        part,
                        TEXT,
                    );
                }
            }
            ViewMode::Tiles | ViewMode::Content => {
                let width = if mode == ViewMode::Content {
                    text.width.saturating_sub(120)
                } else {
                    text.width
                };
                label(v, text.x, text.y, width, (20 + g) as u32, &name, TEXT);
                label(
                    v,
                    text.x,
                    text.y + 20 + g,
                    width,
                    (18 + g) as u32,
                    kind,
                    MUTED,
                );
                if let Some(size) = &size {
                    if mode == ViewMode::Tiles {
                        label(
                            v,
                            text.x,
                            text.y + 38 + 2 * g,
                            width,
                            (18 + g) as u32,
                            size,
                            MUTED,
                        );
                    } else {
                        let w = v.measure(size);
                        label(
                            v,
                            cell.x + cell.width as i32 - 16 - w as i32,
                            text.y,
                            w,
                            (20 + g) as u32,
                            size,
                            MUTED,
                        );
                    }
                }
            }
            ViewMode::Details => {
                let columns = self.detail_columns(layout);
                let name_end = columns[0].1.x + columns[0].1.width as i32 - 8;
                label(
                    v,
                    text.x,
                    cell.y,
                    (name_end - text.x).max(0) as u32,
                    cell.height,
                    &name,
                    TEXT,
                );
                // Date modified: blank when the volume recorded no time.
                let date = columns[1].1;
                let when = self.date_text(entry.times.modified);
                label(
                    v,
                    date.x + 8,
                    cell.y,
                    date.width.saturating_sub(16),
                    cell.height,
                    &when,
                    MUTED,
                );
                let ty = columns[2].1;
                label(
                    v,
                    ty.x + 8,
                    cell.y,
                    ty.width.saturating_sub(16),
                    cell.height,
                    kind,
                    MUTED,
                );
                if let Some(size) = &size {
                    let sz = columns[3].1;
                    let w = v.measure(size).min(sz.width.saturating_sub(16));
                    label(
                        v,
                        sz.x + sz.width as i32 - 12 - w as i32,
                        cell.y,
                        w,
                        cell.height,
                        size,
                        MUTED,
                    );
                }
            }
            ViewMode::Small | ViewMode::List => {
                label(v, text.x, cell.y, text.width, cell.height, &name, TEXT);
            }
        }
    }
    /// This PC: "Devices and drives" from the mounted-volume model, with the
    /// filesystem's own capacity accounting or an honest unavailable state.
    fn draw_drives(&self, v: &mut ui::View<'_>, layout: ExplorerLayout, mode: ViewMode) {
        let header = Bounds {
            x: self.layout.sidebar + 16,
            y: content_top() + 4,
            width: layout.width.saturating_sub(self.layout.sidebar as u32 + 32),
            height: (24 + xl::grow()) as u32,
        };
        let plate = self.drives_header();
        if v.hover.is_some_and(|(x, y)| plate.contains(x, y)) {
            v.rounded(plate.x, plate.y, plate.width, plate.height, 4, HOVER);
        }
        if self.drives_expanded {
            v.tiny_icon(
                header.x,
                header.y + 6 + xl::grow() / 2,
                Icon::Chevron,
                MUTED,
            );
        } else {
            v.tiny_icon(
                header.x,
                header.y + 6 + xl::grow() / 2,
                Icon::ChevronRight,
                MUTED,
            );
        }
        label(
            v,
            header.x + 20,
            header.y,
            header.width.saturating_sub(20),
            header.height,
            &if self.drives_expanded {
                String::from("Devices and drives")
            } else {
                format!("Devices and drives ({})", self.drives().len())
            },
            TEXT,
        );
        for (slot, index) in (self.offset..self.drives().len())
            .take(layout.capacity(mode))
            .enumerate()
        {
            let drive = &self.drives()[index];
            let Some(cell) = self.item_cell(slot) else {
                break;
            };
            self.item_fill(
                v,
                cell,
                self.detail.is_some() && self.overview_selected == index,
            );
            let icon_box = layout.icon_bounds(cell, mode);
            self.draw_item_icon(v, icon_box, Icon::Drive, mode, None);
            let text = layout.label_bounds(cell, mode);
            let (usage, line) = match self.volume {
                VolumeState::Known(u) => (
                    Some(u),
                    format!(
                        "{} free of {}",
                        previewing::size(u.free_bytes),
                        previewing::size(u.total_bytes)
                    ),
                ),
                VolumeState::Unavailable => (None, "Capacity unavailable".into()),
                VolumeState::Unknown => (None, String::new()),
            };
            let stacked = mode.stacked();
            let (x, y, width) = if stacked {
                (text.x, text.y, text.width)
            } else {
                (
                    text.x,
                    if mode.icon_size() > 16 {
                        text.y
                    } else {
                        cell.y + 3
                    },
                    text.width,
                )
            };
            let g = xl::grow();
            if stacked {
                // Icon views: the name centred under the icon, wrapped onto
                // as many lines as the view allows ("Local Disk" / "(C:)").
                let lines =
                    door_workspace::dialog::lines(drive.label, width, |line| v.measure(line));
                for (line, range) in lines.into_iter().take(mode.label_lines()).enumerate() {
                    let part = &drive.label[range];
                    let w = v.measure(part).min(width);
                    label(
                        v,
                        x + (width - w) as i32 / 2,
                        y + line as i32 * (18 + g),
                        w,
                        (20 + g) as u32,
                        part,
                        TEXT,
                    );
                }
            } else {
                label(v, x, y, width, (20 + g) as u32, drive.label, TEXT);
            }
            if matches!(mode, ViewMode::Tiles | ViewMode::Content) {
                // Capacity bar: Windows' accent fill, red when nearly full.
                let bar = Bounds {
                    x,
                    y: y + 24 + g,
                    width: width.min(240),
                    height: 6,
                };
                if let Some(u) = usage {
                    v.rect(bar.x, bar.y, bar.width, bar.height, [230, 230, 230, 255]);
                    let used = (u128::from(u.used_bytes()) * u128::from(bar.width)
                        / u128::from(u.total_bytes.max(1))) as u32;
                    let full = u.free_bytes * 10 < u.total_bytes;
                    v.rect(
                        bar.x,
                        bar.y,
                        used,
                        bar.height,
                        if full {
                            [196, 43, 28, 255]
                        } else {
                            [0, 120, 212, 255]
                        },
                    );
                }
                label(v, x, y + 34 + g, width, (18 + g) as u32, &line, MUTED);
            } else if mode == ViewMode::Details {
                let columns = self.detail_columns(layout);
                let ty = columns[2].1;
                label(
                    v,
                    ty.x + 8,
                    cell.y,
                    ty.width.saturating_sub(16),
                    cell.height,
                    "Local Disk",
                    MUTED,
                );
                let sz = columns[3].1;
                if let Some(u) = usage {
                    let free = previewing::size(u.free_bytes);
                    let w = v.measure(&free).min(sz.width.saturating_sub(16));
                    label(
                        v,
                        sz.x + sz.width as i32 - 12 - w as i32,
                        cell.y,
                        w,
                        cell.height,
                        &free,
                        MUTED,
                    );
                }
            }
        }
        if self.drives().is_empty() {
            label(
                v,
                header.x,
                header.y + 40 + xl::grow(),
                header.width,
                (24 + xl::grow()) as u32,
                "No drives are mounted.",
                MUTED,
            );
        }
    }
    /// Audio, video and archive previews: an archive's top level as a
    /// compressed folder shows it; for media the cover or poster frame (else
    /// the file type's icon), the title and artist or the picture size and
    /// codec, a WAV waveform, and the playback row.
    fn draw_media(
        &self,
        v: &mut ui::View<'_>,
        r: Bounds,
        (x, y, width): (i32, i32, u32),
        d: &Detail,
        p: &CachedPreview,
        m: &MediaPreview,
    ) {
        let shown = |v: &ui::View<'_>, top: i32, h: i32| {
            top + h >= r.y + details::body() && top <= v.height as i32 - xl::status_height() as i32
        };
        let preview = &m.preview;
        if !preview.format.is_empty() {
            let root = preview.root();
            let items = match root.len() {
                1 => String::from("1 item"),
                n if preview.partial => format!("{n}+ items"),
                n => format!("{n} items"),
            };
            label(
                v,
                x,
                y,
                width,
                20,
                &format!("{} - {items}", preview.format),
                MUTED,
            );
            let mut top = y + 28;
            if !preview.note.is_empty() {
                wrap(v, x, top, width, preview.note, 3, MUTED);
                top += 58;
            }
            for e in &root {
                if shown(v, top, 34) {
                    let name = ui::display_ascii(&e.name);
                    v.compact_icon(
                        x,
                        top + 9,
                        if e.folder {
                            Icon::Folder
                        } else {
                            Icon::for_file_name(&name)
                        },
                        TEXT,
                    );
                    let size = if e.folder {
                        String::new()
                    } else {
                        door_workspace::explorer_preview::size(e.size)
                    };
                    let size_w = v.measure(&size);
                    label(
                        v,
                        x + 28,
                        top,
                        width.saturating_sub(36 + size_w),
                        34,
                        &name,
                        TEXT,
                    );
                    label(
                        v,
                        x + width as i32 - size_w as i32,
                        top,
                        size_w,
                        34,
                        &size,
                        MUTED,
                    );
                }
                top += 34;
            }
            return;
        }
        let video = d.kind == Kind::Video;
        let art = if video {
            width * 9 / 16
        } else {
            width.min(240)
        };
        v.rounded(
            x,
            y,
            width,
            art,
            5,
            if video {
                [32, 32, 32, 255]
            } else {
                [241, 239, 236, 255]
            },
        );
        match p.image.as_ref() {
            Some(image) => v.image_fit(
                (x + 4, y + 4, width.saturating_sub(8), art.saturating_sub(8)),
                image.width,
                image.height,
                &image.bgra,
            ),
            None => v.sized_icon(
                x + (width as i32 - 64) / 2,
                y + (art as i32 - 64) / 2,
                64,
                Icon::for_file_name(&d.name),
            ),
        }
        let facts = &m.report;
        let mut top = y + art as i32 + 10;
        let mut line = |v: &mut ui::View<'_>, text: &str, color: [u8; 4]| {
            if !text.is_empty() {
                label(v, x, top, width, 22, &ui::display_ascii(text), color);
                top += 22;
            }
        };
        line(v, facts.get("Title").unwrap_or(&d.name), TEXT);
        let artist = facts
            .get("Contributing artists")
            .or_else(|| facts.get("Album artist"))
            .unwrap_or("");
        let album = facts.get("Album").unwrap_or("");
        line(
            v,
            &match (artist.is_empty(), album.is_empty()) {
                (false, false) => format!("{artist} - {album}"),
                (false, true) => String::from(artist),
                (true, _) => String::from(album),
            },
            MUTED,
        );
        let length = facts.get("Length").unwrap_or("");
        let detail = if video {
            let size = match (facts.get("Frame width"), facts.get("Frame height")) {
                (Some(w), Some(h)) => format!("{w} x {h}"),
                _ => String::new(),
            };
            [size.as_str(), facts.get("Video codec").unwrap_or("")]
                .iter()
                .filter(|s| !s.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join(" - ")
        } else {
            String::from(facts.get("Bit rate").unwrap_or(""))
        };
        line(v, &detail, MUTED);
        let mut top = top + 6;
        if !preview.peaks.is_empty() && shown(v, top, 40) {
            let step = (width / preview.peaks.len() as u32).max(1);
            for (i, peak) in preview.peaks.iter().enumerate() {
                let h = (u32::from(*peak) * 40 / 255).max(1);
                v.rect(
                    x + (i as u32 * step) as i32,
                    top + 20 - h as i32 / 2,
                    step.saturating_sub(1).max(1),
                    h,
                    ACCENT,
                );
            }
            top += 48;
        }
        // Playback row: a disabled play button, the position and the seek
        // track, until the audio service plays media.
        if shown(v, top, 32) {
            v.rounded(x, top, 32, 32, 16, [243, 243, 243, 255]);
            let (cx, cy) = (x as f32 + 13.0, top as f32 + 10.0);
            v.stroke((cx, cy), (cx, cy + 12.0), 1.5, MUTED);
            v.stroke((cx, cy), (cx + 10.0, cy + 6.0), 1.5, MUTED);
            v.stroke((cx, cy + 12.0), (cx + 10.0, cy + 6.0), 1.5, MUTED);
            let time = if length.is_empty() {
                String::from("0:00")
            } else {
                format!("0:00 / {length}")
            };
            let time_w = v.measure(&time);
            label(v, x + 42, top, time_w, 32, &time, MUTED);
            let track = x + 52 + time_w as i32;
            v.rounded(
                track,
                top + 15,
                (width as i32 - (track - x)).max(0) as u32,
                3,
                1,
                [214, 214, 214, 255],
            );
        }
        let reason = if preview.note == door_workspace::media_preview::DAMAGED {
            preview.note
        } else if video && p.image.is_none() {
            "Doors doesn't decode this video format (H.264, HEVC and similar codecs need patent licences)."
        } else {
            "Playback arrives with the Doors audio service."
        };
        wrap(v, x, top + 42, width, reason, 3, MUTED);
    }
    /// A program's Preview: what it is and who signed it (its version
    /// resource and Authenticode signature, from the details read).
    fn draw_program(&self, v: &mut ui::View<'_>, (x, y, width): (i32, i32, u32), d: &Detail) {
        let meta = self.meta_for(d.info);
        let Some(m) = meta.filter(|m| !m.failed) else {
            let note = if meta.is_some() {
                "This program's details couldn't be read."
            } else {
                "Reading program details..."
            };
            wrap(v, x, y, width, note, 3, MUTED);
            return;
        };
        let get = |label: &str| m.report.get(label);
        if !self.draw_program_icon(v, x, y, 48, Some((&d.name, d.info))) {
            v.hero_icon(x, y, 48, Icon::for_file_name(&d.name));
        }
        let title = ui::display_ascii(get("File description").unwrap_or(&d.name));
        label(v, x + 60, y + 2, width.saturating_sub(60), 24, &title, TEXT);
        if let Some(company) = get("Company") {
            label(
                v,
                x + 60,
                y + 26,
                width.saturating_sub(60),
                22,
                &ui::display_ascii(company),
                MUTED,
            );
        }
        let publisher = match get("Signed by") {
            Some(name) => String::from(name),
            None => String::from("Unknown publisher"),
        };
        let size = previewing::size(d.info.bytes);
        let rows = [
            ("Publisher", Some(publisher.as_str())),
            ("Signature", get("Status")),
            ("Version", get("File version")),
            ("Runs as", get("Interface")),
            ("Architecture", get("Architecture")),
            ("Size", Some(size.as_str())),
        ];
        let half = width / 2;
        let mut top = y + 68;
        for (name, value) in rows {
            let Some(value) = value else { continue };
            label(v, x, top, half.saturating_sub(8), 24, name, MUTED);
            label(
                v,
                x + half as i32,
                top,
                width.saturating_sub(half),
                24,
                &ui::display_ascii(value),
                TEXT,
            );
            top += 28;
        }
    }
    pub(in crate::files) fn draw_preview(&self, v: &mut ui::View<'_>, r: Bounds, shift: i32) {
        let d = self.detail.as_ref().unwrap();
        let x = r.x + 20;
        let y = r.y + details::body() + 10 - shift;
        let width = r.width.saturating_sub(40);
        if !d.status.is_empty() {
            wrap(v, x, y, width, &d.status, 5, MUTED);
            return;
        }
        if d.info.directory {
            if d.children.is_empty() {
                wrap(
                    v,
                    x,
                    y,
                    width,
                    if d.children_done {
                        "This folder is empty."
                    } else {
                        "Reading folder contents..."
                    },
                    3,
                    MUTED,
                );
            }
            for (i, child) in d.children.iter().enumerate() {
                let y = y + i as i32 * 34;
                if y + 34 < r.y + details::body()
                    || y > v.height as i32 - xl::status_height() as i32
                {
                    continue;
                }
                let name = ui::display_ascii(&Self::name(child));
                v.compact_icon(
                    x,
                    y + 9,
                    if child.info.directory {
                        Self::folder_icon(&name)
                    } else {
                        Icon::for_file_name(&name)
                    },
                    TEXT,
                );
                label(v, x + 28, y, width.saturating_sub(28), 34, &name, TEXT);
            }
            return;
        }
        if door_workspace::file_metadata::signable(&d.name) {
            self.draw_program(v, (x, y, width), d);
            return;
        }
        if !d.kind.previewable() {
            wrap(v, x, y, width, d.kind.unavailable(), 5, MUTED);
            return;
        }
        let Some(p) = self.cached(d.info) else {
            wrap(v, x, y, width, "Loading preview...", 2, MUTED);
            return;
        };
        if let Some(m) = p.media.as_deref() {
            self.draw_media(v, r, (x, y, width), d, p, m);
            return;
        }
        if let Some(image) = p.image.as_ref() {
            v.rounded(x, y, width, 182, 5, [241, 239, 236, 255]);
            v.image_fit(
                (x + 4, y + 4, width.saturating_sub(8), 174),
                image.width,
                image.height,
                &image.bgra,
            );
            label(
                v,
                x,
                y + 190,
                width,
                24,
                &format!("{} x {} pixels", image.source_width, image.source_height),
                MUTED,
            );
            label(v, x, y + 220, width, 24, "Read-only image preview", MUTED);
        } else if !p.text.is_empty() {
            v.rounded(x, y, width, 18, 3, [255; 4]);
            let lines =
                door_workspace::dialog::lines(&p.text, width.saturating_sub(16), |s| v.measure(s));
            for (i, range) in lines.into_iter().enumerate() {
                let top = y + i as i32 * 19;
                if top + 19 < r.y + details::body()
                    || top > v.height as i32 - xl::status_height() as i32
                {
                    continue;
                }
                label(
                    v,
                    x + 8,
                    top,
                    width.saturating_sub(16),
                    19,
                    &p.text[range],
                    TEXT,
                );
            }
        } else if p.status.is_empty() {
            wrap(v, x, y, width, "This file is empty.", 3, MUTED);
        }
        if !p.status.is_empty() {
            wrap(v, x, y, width, &p.status, 5, MUTED);
        }
    }
}
