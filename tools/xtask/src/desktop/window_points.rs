//! Window input points derived from the real window authority
//! (`door_window::graphics`, i.e. door-graphics-core) and taskbar layout,
//! never from remembered screen coordinates. App windows open at
//! `graphics::initial_window`, the function the native app runtime uses; the
//! requested sizes and caption kinds mirror `WindowOptions` in
//! `desktop/workspace/src/main.rs`. All points are logical (multiply by the
//! density for QHD/4K profiles).
use door_window::graphics as g;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum App {
    Files,
    Notes,
    Settings,
}
impl App {
    const fn request(self) -> (u32, u32) {
        match self {
            Self::Files => (1000, 640),
            Self::Notes => (900, 640),
            Self::Settings => (1129, 787),
        }
    }
    const fn caption(self) -> g::CaptionLayout {
        match self {
            // Tabbed captions open with the full strip; the app then narrows
            // it to its tabs (`set_tabbed_caption`).
            Self::Files => g::CaptionLayout {
                tabbed: true,
                height: 40,
                max_width: 2048,
                side_margin: 8,
            },
            // Notepad 26H2: a 41 px tab row whose first tab starts at x 44
            // (desktop/workspace/src/document_format.rs `CAPTION`, `TAB_LEFT`).
            Self::Notes => g::CaptionLayout {
                tabbed: true,
                height: 41,
                max_width: 2048,
                side_margin: 44,
            },
            // `create_client_caption` with the Settings title-bar search
            // policy (desktop/workspace/src/runtime.rs): the centred pill.
            Self::Settings => g::CaptionLayout {
                tabbed: false,
                height: 32,
                max_width: door_workspace::settings_layout::Layout::CAPTION_CONTROL,
                side_margin: door_workspace::settings_layout::Layout::CAPTION_MARGIN,
            },
        }
    }
}
/// The real authority at a logical output with the default (bottom) taskbar.
pub(super) struct Desk {
    engine: g::Engine,
    owner: g::Owner,
    output: (u32, u32),
}
impl Desk {
    pub(super) fn new(width: u32, height: u32) -> Result<Self, String> {
        let mut engine = g::Engine::new(width, height, g::Theme::Light, g::Limits::default())
            .map_err(|e| format!("window authority: {e:?}"))?;
        let work = door_window::taskbar::Layout::new(width, height).work_area();
        engine
            .set_work_area(work)
            .map_err(|e| format!("work area: {e:?}"))?;
        Ok(Self {
            engine,
            owner: g::Owner {
                pid: 7,
                session: 1,
                epoch: 1,
            },
            output: (width, height),
        })
    }
    #[cfg(test)]
    pub(super) fn work_area(&self) -> g::Rect {
        door_window::taskbar::Layout::new(self.output.0, self.output.1).work_area()
    }
    /// Open `app` as its process's first window; later windows stack on top.
    pub(super) fn open(&mut self, app: App) -> Result<g::WindowId, String> {
        let (x, y, w, h) = g::initial_window(self.output, app.request(), 0, None);
        let outer = g::Rect::new(x, y, w, h).map_err(|e| format!("{e:?}"))?;
        self.engine
            .create_client_caption(self.owner, outer, app.caption())
            .map_err(|e| format!("open {app:?}: {e:?}"))
    }
    pub(super) fn outer(&self, id: g::WindowId) -> Result<g::Rect, String> {
        Ok(self
            .engine
            .window_info(id)
            .map_err(|e| format!("{e:?}"))?
            .outer)
    }
    /// The app's tab strip (tabbed captions), as the app reports it.
    pub(super) fn tabs(&mut self, id: g::WindowId, left: u32, width: u32) -> Result<(), String> {
        self.engine
            .set_tabbed_caption(self.owner, id, left, width)
            .map_err(|e| format!("{e:?}"))
    }
    pub(super) fn maximize(&mut self, id: g::WindowId) -> Result<(), String> {
        self.engine
            .request_state(self.owner, id, g::WindowState::Maximized)
            .map_err(|e| format!("{e:?}"))
    }
    pub(super) fn restore(&mut self, id: g::WindowId) -> Result<(), String> {
        self.engine
            .restore_window(self.owner, id)
            .map_err(|e| format!("{e:?}"))
    }
    /// A held edge drag from `from` to `to`, resized with the window server's
    /// arithmetic (graphics/window-server/src/input.rs: edges from
    /// `CursorIcon::resize_edges`, minimum 240 x 120), then configured.
    pub(super) fn resize_drag(
        &mut self,
        id: g::WindowId,
        from: (i32, i32),
        to: (i32, i32),
    ) -> Result<g::Rect, String> {
        if self.hit(from.0, from.1) != Some((id, g::HitRegion::Resize)) {
            return Err(format!("{from:?} is not on the resize band"));
        }
        let r = self.outer(id)?;
        let edges = g::CursorIcon::resize_edges(r, from.0, from.1);
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let (ow, oh) = (self.output.0 as i32, self.output.1 as i32);
        let (mut left, mut top, mut right, mut bottom) = (r.x(), r.y(), r.right(), r.bottom());
        if edges & 1 != 0 {
            left = (left + dx).clamp(right - ow, right - 240);
        }
        if edges & 2 != 0 {
            right = (right + dx).clamp(left + 240, left + ow);
        }
        if edges & 4 != 0 {
            top = (top + dy).clamp(bottom - oh, bottom - 120);
        }
        if edges & 8 != 0 {
            bottom = (bottom + dy).clamp(top + 120, top + oh);
        }
        let rect = g::Rect::new(left, top, (right - left) as u32, (bottom - top) as u32)
            .map_err(|e| format!("{e:?}"))?;
        let epoch = self
            .engine
            .configure(self.owner, id, rect)
            .map_err(|e| format!("{e:?}"))?;
        self.engine
            .acknowledge_configure(self.owner, id, epoch)
            .map_err(|e| format!("{e:?}"))?;
        Ok(rect)
    }
    /// Move `id` (a caption drag), as the window server does on motion.
    pub(super) fn move_to(&mut self, id: g::WindowId, x: i32, y: i32) -> Result<(), String> {
        self.engine
            .move_window(self.owner, id, x, y)
            .map_err(|e| format!("{e:?}"))
    }
    /// A title drag dropped at `(x, y)`: the window server's edge snap
    /// (`policy::edge_snap`), applied when the drop lands on an edge.
    pub(super) fn drop_at(&mut self, id: g::WindowId, x: i32, y: i32) -> Result<(), String> {
        let area = self.engine.work_area();
        let snap = door_window::policy::edge_snap(area, self.engine.scale(), x, y)
            .ok_or_else(|| format!("({x}, {y}) is not on a snap edge"))?;
        let rect = self.outer(id)?;
        self.engine
            .snap_window(self.owner, id, snap, Some(rect))
            .map_err(|e| format!("{e:?}"))
    }
    /// A title drag of a maximized or snapped window from `pressed` to
    /// `pointer`: restored to its normal size under the pointer.
    pub(super) fn restore_title_drag(
        &mut self,
        id: g::WindowId,
        pressed: (i32, i32),
        pointer: (i32, i32),
    ) -> Result<g::Rect, String> {
        self.engine
            .restore_title_drag(self.owner, id, pressed, pointer)
            .map_err(|e| format!("{e:?}"))
    }
    /// A server-decorated top-level (32 px caption) of another process.
    pub(super) fn open_foreign(&mut self, (x, y, w, h): Bounds) -> Result<g::WindowId, String> {
        let owner = g::Owner {
            pid: 8,
            ..self.owner
        };
        let outer = g::Rect::new(x, y, w, h).map_err(|e| format!("{e:?}"))?;
        self.engine
            .create_window(owner, g::Role::TopLevel, outer)
            .map_err(|e| format!("open foreign window: {e:?}"))
    }
    pub(super) fn activate(&mut self, id: g::WindowId) -> Result<(), String> {
        self.engine.activate(id).map_err(|e| format!("{e:?}"))
    }
    pub(super) fn minimize(&mut self, id: g::WindowId) -> Result<(), String> {
        self.engine
            .request_state(self.owner, id, g::WindowState::Minimized)
            .map_err(|e| format!("{e:?}"))
    }
    pub(super) fn hit(&self, x: i32, y: i32) -> Option<(g::WindowId, g::HitRegion)> {
        self.engine.hit_test(x, y)
    }
    /// `rgb` at frame pixel `at` (a pixel of `below` or of the desktop that
    /// no window body covers) darkened by the shadows of every window
    /// stacked above `below` (every window when None), as composed.
    pub(super) fn shadowed(
        &self,
        (x, y): (usize, usize),
        rgb: [u8; 3],
        below: Option<g::WindowId>,
    ) -> [u8; 3] {
        let c = self
            .engine
            .shadowed(x as i32, y as i32, [rgb[2], rgb[1], rgb[0], 255], below);
        [c[2], c[1], c[0]]
    }
    /// Centre of caption region `region` of `id`, found by hit testing the
    /// authority across the caption's middle row.
    pub(super) fn button(
        &self,
        id: g::WindowId,
        region: g::HitRegion,
    ) -> Result<(i32, i32), String> {
        let outer = self.outer(id)?;
        let height = self
            .engine
            .window_info(id)
            .map_err(|e| format!("{e:?}"))?
            .caption_height;
        // Server-decorated windows report 0: the authority draws its title bar.
        let height = if height == 0 { g::TITLE_HEIGHT } else { height };
        let y = outer.y() + height as i32 / 2;
        let xs: Vec<_> = (outer.x()..outer.right())
            .filter(|x| self.hit(*x, y) == Some((id, region)))
            .collect();
        match (xs.first(), xs.last()) {
            (Some(a), Some(b)) => Ok(((a + b) / 2, y)),
            _ => Err(format!("{region:?} not found on window caption")),
        }
    }
    /// A caption point that drags the window: 24 px left of Minimize, at
    /// window y 26 (screen y >= 40 keeps the snap-layouts bar hidden).
    pub(super) fn drag_point(&self, id: g::WindowId) -> Result<(i32, i32), String> {
        let (min_x, _) = self.button(id, g::HitRegion::Minimize)?;
        let outer = self.outer(id)?;
        Ok((min_x - 23 - 24, outer.y() + 26))
    }
}
/// Light window border token: the outer column of every normal window.
pub(super) fn border() -> [u8; 3] {
    let b = g::Theme::Light.tokens().border;
    [b[2], b[1], b[0]]
}
/// The logical desktop of the wide (1x) and accelerated 3840x2160 (3x)
/// profiles.
pub(super) const WIDE: (u32, u32) = (1280, 720);
/// `app`'s initial placement on a logical `output`.
pub(super) fn placement(app: App, output: (u32, u32)) -> Result<g::Rect, String> {
    let mut desk = Desk::new(output.0, output.1)?;
    let id = desk.open(app)?;
    desk.outer(id)
}
/// Origin of `app`'s initial placement on the 1280x720 desktop. Native apps
/// draw their own captions, so window (client) points plus this origin are
/// screen points.
pub(super) fn origin(app: App) -> Result<(i32, i32), String> {
    let r = placement(app, WIDE)?;
    Ok((r.x(), r.y()))
}
/// `origin(app)` plus a window point.
pub(super) fn screen(app: App, (x, y): (i32, i32)) -> Result<(i32, i32), String> {
    let (ox, oy) = origin(app)?;
    Ok((ox + x, oy + y))
}
/// Maximize button centre of `app` at its initial placement on `output`.
pub(super) fn maximize_button(app: App, output: (u32, u32)) -> Result<(i32, i32), String> {
    let mut desk = Desk::new(output.0, output.1)?;
    let id = desk.open(app)?;
    desk.button(id, g::HitRegion::Maximize)
}
/// lifecycle.rs `wide-max`: per pinned app, the maximize button of its
/// initial placement and the restore button once maximized; both on a 1x
/// 1280x720 desktop.
/// Two physical pointer points.
type PointPair = ((i32, i32), (i32, i32));
/// Client-space rectangle (x, y, width, height).
type Bounds = (i32, i32, u32, u32);
pub(super) fn wide_max(app: App) -> Result<PointPair, String> {
    let mut desk = Desk::new(WIDE.0, WIDE.1)?;
    let id = desk.open(app)?;
    let maximize = desk.button(id, g::HitRegion::Maximize)?;
    desk.maximize(id)?;
    let restore = desk.button(id, g::HitRegion::Maximize)?;
    Ok((maximize, restore))
}
/// resolution.rs desktop-motion: Settings' maximize button at its initial
/// placement and its minimize button once maximized (1280x720 logical).
pub(super) fn settings_max_min() -> Result<PointPair, String> {
    let mut desk = Desk::new(WIDE.0, WIDE.1)?;
    let id = desk.open(App::Settings)?;
    let maximize = desk.button(id, g::HitRegion::Maximize)?;
    desk.maximize(id)?;
    Ok((maximize, desk.button(id, g::HitRegion::Minimize)?))
}
/// shell.rs caption visuals: Settings' Minimize, Maximize and Close centres
/// at its initial placement (1280x720 logical).
pub(super) fn settings_caption() -> Result<[(i32, i32); 3], String> {
    let mut desk = Desk::new(WIDE.0, WIDE.1)?;
    let id = desk.open(App::Settings)?;
    Ok([
        desk.button(id, g::HitRegion::Minimize)?,
        desk.button(id, g::HitRegion::Maximize)?,
        desk.button(id, g::HitRegion::Close)?,
    ])
}
/// Settings window point between the account header and the category rail:
/// its light window background at every window size (asserted by
/// `window_management_probes_hit_the_settings_background` in
/// desktop/workspace/tests/settings_reference_painter.rs).
pub(super) const SETTINGS_BACKGROUND_POINT: (i32, i32) = (75, 130);
/// The leftmost window x on `SETTINGS_BACKGROUND_POINT`'s row that is still
/// the background below the account picture (also asserted by
/// `window_management_probes_hit_the_settings_background`).
pub(super) const SETTINGS_EXPOSED_MIN_X: i32 = 60;
/// `SETTINGS_BACKGROUND_POINT` of Settings at its initial placement.
pub(super) fn settings_background() -> Result<(i32, i32), String> {
    screen(App::Settings, SETTINGS_BACKGROUND_POINT)
}
/// A Settings caption point that drags the window: right of the centred
/// search box, left of Minimize, below the snap-layouts reveal band.
pub(super) fn settings_drag() -> Result<(i32, i32), String> {
    let mut desk = Desk::new(WIDE.0, WIDE.1)?;
    let id = desk.open(App::Settings)?;
    desk.drag_point(id)
}
/// lifecycle.rs held drag of File Explorer at its initial placement: the
/// caption drag point and the border column before the drag.
pub(super) fn explorer_drag() -> Result<((i32, i32), i32), String> {
    let mut desk = Desk::new(WIDE.0, WIDE.1)?;
    let id = desk.open(App::Files)?;
    // A representative single-tab strip; the point stays a drag region for
    // any strip that leaves 24 px before Minimize (see the host test).
    desk.tabs(id, 8, 300)?;
    Ok((desk.drag_point(id)?, desk.outer(id)?.x()))
}
/// Points for the desktop-motion cached drag (resolution.rs, owned by the
/// shell workstream), logical: hover Close then the drag point, drag by
/// (+120, +20), then eight pairs to (+80, +20) and back. `chrome` is a
/// caption pixel of Explorer before the drag and its moved position after;
/// `exposed` is uncovered by the drag and shows Settings' window background.
pub(super) struct Motion {
    pub hover_close: (i32, i32),
    pub start: (i32, i32),
    pub end: (i32, i32),
    pub alternate: (i32, i32),
    pub chrome_before: (i32, i32),
    pub chrome_after: (i32, i32),
    pub exposed: (i32, i32),
}
/// The scenario activates Explorer, Notes and Settings, then Explorer again,
/// so Settings is directly below Explorer (all three share one placement).
fn motion_desk() -> Result<(Desk, [g::WindowId; 3]), String> {
    let mut desk = Desk::new(WIDE.0, WIDE.1)?;
    let notes = desk.open(App::Notes)?;
    let settings = desk.open(App::Settings)?;
    let files = desk.open(App::Files)?;
    desk.tabs(files, 8, 300)?;
    Ok((desk, [files, notes, settings]))
}
pub(super) fn motion() -> Result<Motion, String> {
    let (desk, [files, _, settings]) = motion_desk()?;
    let start = desk.drag_point(files)?;
    let outer = desk.outer(files)?;
    let chrome_before = (outer.x() + 700, outer.y() + 8);
    let s = desk.outer(settings)?;
    // Settings' background where Explorer was, clear of Explorer's shadow
    // once it has moved 120 px right (`g::WINDOW_SHADOW` reaches 52 px left
    // of a window; inside it a 3x pixel can round one level darker).
    let moved_left = outer.x() + 120;
    let exposed = (
        (s.x() + SETTINGS_BACKGROUND_POINT.0).min(moved_left - g::WINDOW_SHADOW.left as i32 - 4),
        s.y() + SETTINGS_BACKGROUND_POINT.1,
    );
    if exposed.0 - s.x() < SETTINGS_EXPOSED_MIN_X {
        return Err("motion probe left Settings' background".into());
    }
    Ok(Motion {
        hover_close: desk.button(files, g::HitRegion::Close)?,
        start,
        end: (start.0 + 120, start.1 + 20),
        alternate: (start.0 + 200, start.1 + 40),
        chrome_before,
        chrome_after: (chrome_before.0 + 120, chrome_before.1 + 20),
        exposed,
    })
}
/// Notes' window at its initial placement (logical). Notes draws its own
/// 40 px tabbed caption, so its client coordinates start at this origin.
pub(super) fn notes_window() -> Result<(i32, i32, u32, u32), String> {
    let o = placement(App::Notes, WIDE)?;
    Ok((o.x(), o.y(), o.width(), o.height()))
}
/// Notes' three-button modal (`door_workspace::dialog::Layout::new`) for a
/// client of `width` x `height`: (panel x, y, w, h) and the button rects.
/// The workspace host test `notes_scenario_dialog_geometry_matches_the_app`
/// asserts these values against the real layout function.
pub(super) fn notes_dialog(width: u32, height: u32) -> (Bounds, [Bounds; 3]) {
    let w = width.saturating_sub(16).min(448);
    let compact = w < 336;
    let h = height.saturating_sub(16).min(232);
    let panel = (((width - w) / 2) as i32, ((height - h) / 2) as i32, w, h);
    let pad = (if compact { 8 } else { 24 }).min(w / 4).min(h / 8);
    let inner = w.saturating_sub(pad * 2);
    let gap = (if compact { 4 } else { 8 }).min(inner / 8);
    let bh = 32.min(h.saturating_sub(pad * 2));
    let bw = inner.saturating_sub(gap * 2) / 3;
    let y = panel.1 + h.saturating_sub(bh + pad) as i32;
    let button = |i: i32| (panel.0 + pad as i32 + i * (bw + gap) as i32, y, bw, bh);
    (panel, [button(0), button(1), button(2)])
}
/// The keyboard-focused Save button of Notes' save prompt at its initial
/// placement: 5 px right, 9 px down of its origin, painted in the accent
/// `[0, 103, 196]` (notes.rs `save_focus`).
pub(super) fn notes_save_focus() -> Result<(i32, i32), String> {
    let (x, y, w, h) = notes_window()?;
    let (bx, by, _, _) = notes_dialog(w, h).1[0];
    Ok((x + bx + 5, y + by + 9))
}
/// shell.rs focus probe: a Notes body point above its status bar.
pub(super) const NOTES_BODY_POINT: (i32, i32) = (648, 466);
/// scene.rs resize-cursor probe on the 1024x768 desktop: a point inside the
/// right edge of Notes' initial (proportional) placement that the authority
/// hit-tests as Resize, at the window's vertical middle.
pub(super) fn scene_resize_probe() -> Result<(i32, i32), String> {
    let mut desk = Desk::new(1024, 768)?;
    let id = desk.open(App::Notes)?;
    let o = desk.outer(id)?;
    let y = o.y() + o.height() as i32 / 2;
    // Innermost such point: off the 1 px frame, on the white client.
    (o.right() - 4..o.right())
        .find(|&x| desk.hit(x, y) == Some((id, g::HitRegion::Resize)))
        .map(|x| (x, y))
        .ok_or_else(|| "Notes right edge has no inner resize border".into())
}
/// `scene_resize_probe` as frame-buffer pixel coordinates.
pub(super) fn scene_resize_probe_pixels() -> Result<(usize, usize), String> {
    let (x, y) = scene_resize_probe()?;
    Ok((
        usize::try_from(x).map_err(|e| e.to_string())?,
        usize::try_from(y).map_err(|e| e.to_string())?,
    ))
}
/// The scene.rs desktop (1024x768, 1x, bottom taskbar).
pub(super) const SCENE: (u32, u32) = (1024, 768);
/// Light content token: Notes' body and every white client.
pub(super) fn content() -> [u8; 3] {
    let c = g::Theme::Light.tokens().content;
    [c[2], c[1], c[0]]
}
/// Notes' light command bar below its caption
/// (`door_workspace::document_view::Theme::resolve(light).bar`, BGRA
/// [250, 247, 243]).
pub(super) const NOTES_BAR: [u8; 3] = [243, 247, 250];
/// File Explorer's light 24 px status bar (files/modern/paint.rs, BGRA
/// [253, 251, 248]).
pub(super) const FILES_STATUS: [u8; 3] = [248, 251, 253];
/// The bundled default wallpaper (background 0, Fill) at `Scene::outside` on
/// the 1024x768 desktop, as `door_window::wallpaper::load` decodes it (pinned
/// by `scene_wallpaper_is_the_decoded_default_background`).
pub(super) const SCENE_WALLPAPER: [u8; 3] = [17, 73, 192];
/// A Notes command-bar window point, 15 px below the caption and 8 px in:
/// the blank margin before the File menu (the 26H2 Notepad layout starts
/// File's label 13 px in, so the old 16 px point landed on it). It must lie
/// in the strip the 50 px drag exposes, outside the initial placement.
const NOTES_BAR_POINT: (i32, i32) = (8, 55);
/// Light modern menu row fill under focus (files/menus.rs `MenuInk`, light
/// `hover`).
pub(super) const MENU_FOCUS: [u8; 3] = [239, 239, 239];
/// scene.rs File Explorer menu point, a window point of Explorer at its
/// initial 1024x768 placement (717 x 562): blank list space of /users/1001
/// in Details (30 px rows from window y 189, so ten folders end at y 489),
/// above the 24 px status bar (window y 538) and left of the details pane
/// (window x 437 when shown). In Home it lies in the empty Favorites band.
const FILES_MENU_POINT: (i32, i32) = (327, 511);
/// The folder background menu opened there does not fit below the pointer,
/// so it opens above it (bottom edge 3 px below). Offsets from the pointer:
/// a flat point of its New row (between the rules at -196 and -155) and of
/// its last row, Customize menu (blank list space once dismissed).
const MENU_NEW_ROW: (i32, i32) = (200, -175);
const MENU_LAST_ROW: (i32, i32) = (28, -10);
/// scene.rs window management of Notes on the 1024x768 desktop, replayed on
/// the real authority. Files then Notes open at the same initial placement
/// (Notes on top); after Alt+Tab to Files, the Notes taskbar button brings
/// Notes back; then a caption drag by (-50, -46), a bottom-right resize by
/// (+120, +60), Maximize, Restore, left and top edge drags by 20 px and back,
/// and Minimize; after Notes closes, a right-click on File Explorer. Points
/// are logical pointer points; probes are frame pixels.
pub(super) struct Scene {
    pub activate: (i32, i32),
    pub moves: [(i32, i32); 2],
    pub resize: [(i32, i32); 2],
    pub maximize: (i32, i32),
    pub restore: (i32, i32),
    pub left: [(i32, i32); 2],
    pub top: [(i32, i32); 2],
    pub minimize: (i32, i32),
    /// `NOTES_BAR_POINT` of the moved (and later restored) window: outside
    /// the initial placement, so only a moved Notes paints `NOTES_BAR` there.
    pub bar: (usize, usize),
    /// `bar` after the left edge moved out by 20 px.
    pub bar_left: (usize, usize),
    /// `bar` after the top edge moved up by 20 px (a caption pixel before).
    pub bar_top: (usize, usize),
    /// `NOTES_BAR_POINT` and a body point of the maximized window, both
    /// outside the resized window.
    pub bar_max: (usize, usize),
    pub body_max: (usize, usize),
    /// Inside the initial placement, below the moved window: File Explorer's
    /// status bar, still at the initial placement, once Notes moved away.
    pub exposed: (usize, usize),
    /// `FILES_STATUS` at `exposed` under the moved (active) Notes' shadow,
    /// which reaches 89 px below a window (`g::WINDOW_SHADOW`).
    pub exposed_color: [u8; 3],
    /// A body point of the resized window outside both the initial placement
    /// (clear of its shadow: `g::WINDOW_SHADOW` past its right edge) and the
    /// moved window: wallpaper before the resize and while Notes is
    /// minimized or closed.
    pub outside: (usize, usize),
    /// File Explorer's right-click point (see `FILES_MENU_POINT`) and two
    /// probes of the menu it opens: the focused New row, and a point of its
    /// last row that shows the blank list once the menu is dismissed.
    pub files_menu: (i32, i32),
    pub menu_new: (usize, usize),
    pub menu_last: (usize, usize),
}
fn pixel((x, y): (i32, i32)) -> Result<(usize, usize), String> {
    Ok((
        usize::try_from(x).map_err(|e| e.to_string())?,
        usize::try_from(y).map_err(|e| e.to_string())?,
    ))
}
/// The innermost Resize point of `id` walking `step` in from `outer`.
fn inner_resize(
    desk: &Desk,
    id: g::WindowId,
    outer: (i32, i32),
    step: (i32, i32),
) -> Result<(i32, i32), String> {
    (0..4)
        .map(|k| (outer.0 + k * step.0, outer.1 + k * step.1))
        .rfind(|&(x, y)| desk.hit(x, y) == Some((id, g::HitRegion::Resize)))
        .ok_or_else(|| format!("no resize border inward of {outer:?}"))
}
fn scene_desk() -> Result<(Desk, g::WindowId, g::WindowId), String> {
    let mut desk = Desk::new(SCENE.0, SCENE.1)?;
    let files = desk.open(App::Files)?;
    let notes = desk.open(App::Notes)?;
    // A representative single-tab strip ("desktop.txt" and New tab).
    desk.tabs(notes, 48, 380)?;
    Ok((desk, files, notes))
}
pub(super) fn scene() -> Result<Scene, String> {
    let (mut desk, files, notes) = scene_desk()?;
    let f = desk.outer(files)?;
    let files_menu = (f.x() + FILES_MENU_POINT.0, f.y() + FILES_MENU_POINT.1);
    let menu = |(dx, dy): (i32, i32)| pixel((files_menu.0 + dx, files_menu.1 + dy));
    let bar = |o: g::Rect| pixel((o.x() + NOTES_BAR_POINT.0, o.y() + NOTES_BAR_POINT.1));
    let initial = desk.outer(notes)?;
    let taskbar = door_window::taskbar::Layout::new(SCENE.0, SCENE.1)
        .slot(door_window::taskbar::Slot::App(1), door_window::apps::COUNT);
    let activate = (
        taskbar.x() + taskbar.width() as i32 / 2,
        taskbar.y() + taskbar.height() as i32 / 2,
    );
    let from = desk.drag_point(notes)?;
    let moves = [from, (from.0 - 50, from.1 - 46)];
    desk.move_to(notes, initial.x() - 50, initial.y() - 46)?;
    let moved = desk.outer(notes)?;
    // Notes is the active window when it is dragged (its taskbar button
    // brought it back first).
    desk.activate(notes)?;
    let exposed = pixel((initial.right() - 70, initial.bottom() - 12))?;
    let exposed_color = desk.shadowed(exposed, FILES_STATUS, Some(files));
    let corner = inner_resize(
        &desk,
        notes,
        (moved.right() - 1, moved.bottom() - 1),
        (-1, -1),
    )?;
    let resize = [corner, (corner.0 + 120, corner.1 + 60)];
    desk.resize_drag(notes, resize[0], resize[1])?;
    let resized = desk.outer(notes)?;
    let maximize = desk.button(notes, g::HitRegion::Maximize)?;
    desk.maximize(notes)?;
    let max = desk.outer(notes)?;
    let restore = desk.button(notes, g::HitRegion::Maximize)?;
    desk.restore(notes)?;
    let mid = resized.y() + resized.height() as i32 / 2;
    let edge = inner_resize(&desk, notes, (resized.x(), mid), (1, 0))?;
    let left = [edge, (edge.0 - 20, edge.1)];
    desk.resize_drag(notes, left[0], left[1])?;
    let bar_left = bar(desk.outer(notes)?)?;
    desk.resize_drag(notes, left[1], left[0])?;
    let centre = resized.x() + resized.width() as i32 / 2;
    let edge = inner_resize(&desk, notes, (centre, resized.y()), (0, 1))?;
    let top = [edge, (edge.0, edge.1 - 20)];
    desk.resize_drag(notes, top[0], top[1])?;
    let bar_top = bar(desk.outer(notes)?)?;
    desk.resize_drag(notes, top[1], top[0])?;
    if desk.outer(notes)? != resized {
        return Err("Notes edge drags did not return to the resized window".into());
    }
    Ok(Scene {
        activate,
        moves,
        resize,
        maximize,
        restore,
        left,
        top,
        minimize: desk.button(notes, g::HitRegion::Minimize)?,
        bar: bar(moved)?,
        bar_left,
        bar_top,
        bar_max: bar(max)?,
        body_max: pixel((max.right() - 14, max.bottom() - 40))?,
        exposed,
        exposed_color,
        outside: pixel((
            initial.right() + g::WINDOW_SHADOW.right as i32 + 4,
            moved.bottom() - 15,
        ))?,
        files_menu,
        menu_new: menu(MENU_NEW_ROW)?,
        menu_last: menu(MENU_LAST_ROW)?,
    })
}
/// The adversary probe's top-levels and the hung producer, all created at
/// this rect with server decoration (`create` in
/// tests/fixtures/native-workload/src/window.rs).
pub(super) const PROBE_WINDOW: Bounds = (760, 32, 220, 150);
/// recovery.rs on the 1024x768 accelerated profile: File Explorer and Notes
/// at their shared initial placement under the probe windows, replayed on the
/// real authority. Explorer is activated, dragged by (+80, +60) and back,
/// maximized through its caption button (each gesture held across a denied
/// foreign close), then minimized; Notes is activated, typed into and
/// minimized, exposing the probe; after the restart the hung producer's
/// caption Close button ends it.
pub(super) struct Recovery {
    pub drag: [(i32, i32); 2],
    pub drag_back: [(i32, i32); 2],
    /// The moved Explorer's right border column, outside its placement and
    /// the probe windows: the `border()` token only once it moved.
    pub moved_border: (usize, usize),
    pub maximize: (i32, i32),
    /// Explorer's status bar when maximized, outside its placement and the
    /// probe windows (`FILES_STATUS`).
    pub max_status: (usize, usize),
    pub minimize: (i32, i32),
    pub notes_body: (i32, i32),
    /// The topmost probe window's client (`abandoned`, which takes Escape).
    pub probe: (i32, i32),
    pub hung_close: (i32, i32),
}
pub(super) fn recovery() -> Result<Recovery, String> {
    let mut desk = Desk::new(SCENE.0, SCENE.1)?;
    let files = desk.open(App::Files)?;
    // A representative single-tab strip.
    desk.tabs(files, 8, 300)?;
    let notes = desk.open(App::Notes)?;
    let probe = desk.open_foreign(PROBE_WINDOW)?;
    desk.activate(files)?;
    let initial = desk.outer(files)?;
    let from = desk.drag_point(files)?;
    let to = (from.0 + 80, from.1 + 60);
    desk.move_to(files, initial.x() + 80, initial.y() + 60)?;
    let moved = desk.outer(files)?;
    if desk.hit(to.0, to.1) != Some((files, g::HitRegion::TitleBar)) {
        return Err("moved Explorer drag point left its caption".into());
    }
    let moved_border = pixel((moved.right() - 1, moved.y() + moved.height() as i32 / 2))?;
    desk.move_to(files, initial.x(), initial.y())?;
    let maximize = desk.button(files, g::HitRegion::Maximize)?;
    desk.maximize(files)?;
    let max = desk.outer(files)?;
    let minimize = desk.button(files, g::HitRegion::Minimize)?;
    desk.minimize(files)?;
    desk.activate(notes)?;
    let n = desk.outer(notes)?;
    let notes_body = (n.x() + n.width() as i32 / 2, n.y() + n.height() as i32 / 2);
    desk.minimize(notes)?;
    let p = desk.outer(probe)?;
    let probe_point = (p.x() + 60, p.y() + 78);
    if desk.hit(probe_point.0, probe_point.1) != Some((probe, g::HitRegion::Client)) {
        return Err("probe point is not on the probe client".into());
    }
    Ok(Recovery {
        drag: [from, to],
        drag_back: [to, from],
        moved_border,
        maximize,
        max_status: pixel((max.right() - 70, max.bottom() - 12))?,
        minimize,
        notes_body,
        probe: probe_point,
        hung_close: desk.button(probe, g::HitRegion::Close)?,
    })
}
/// native_default.rs title gestures on the automatic mode's logical desktop
/// (`output` at the guest's automatic `scale`), replayed on the authority in
/// logical pixels and returned as physical points (`Scale::px`). File
/// Explorer opens at its initial placement, is dragged to the top edge
/// (maximized), pulled down from the maximized title (restored under the
/// pointer), dropped on the left edge (left half) and pulled out again.
pub(super) struct NativeDefault {
    pub logical: (u32, u32),
    /// A caption (TitleBar) point of each stage: placement, maximized,
    /// restored, left half, restored again.
    pub caption: [(usize, usize); 5],
    pub to_top: [(i32, i32); 2],
    pub pull_down: [(i32, i32); 2],
    pub to_left: [(i32, i32); 2],
    pub pull_out: [(i32, i32); 2],
    /// Explorer's white content beside its right edge, and the wallpaper
    /// just outside it, for the restored, half and re-restored windows.
    pub body: [(usize, usize); 3],
    pub outside: [(usize, usize); 3],
}
pub(super) fn native_default(output: (u32, u32), scale: g::Scale) -> Result<NativeDefault, String> {
    let logical = (scale.logical_len(output.0), scale.logical_len(output.1));
    let (lw, lh) = (logical.0 as i32, logical.1 as i32);
    let mut desk = Desk::new(logical.0, logical.1)?;
    let files = desk.open(App::Files)?;
    // A representative single-tab strip ("Home" and New tab).
    desk.tabs(files, 8, 300)?;
    let px = |(x, y): (i32, i32)| (scale.px(x), scale.px(y));
    let pixel = |p: (i32, i32)| -> Result<(usize, usize), String> { pixel(px(p)) };
    let caption = |desk: &Desk| -> Result<(i32, i32), String> {
        let p = desk.drag_point(files)?;
        match desk.hit(p.0, p.1) {
            Some((id, g::HitRegion::TitleBar)) if id == files => Ok(p),
            hit => Err(format!("Explorer caption point {p:?} hits {hit:?}")),
        }
    };
    // The wallpaper probe lies past Explorer's shadow (`g::WINDOW_SHADOW`).
    let clear = g::WINDOW_SHADOW.right as i32 + 4;
    let beside = |r: g::Rect| {
        (
            (r.right() - 20, r.y() + 400),
            (r.right() + clear, r.y() + 100),
        )
    };
    let placed = caption(&desk)?;
    // Drag to the top edge, then drop: maximized.
    let top = (lw / 2, 0);
    desk.drop_at(files, top.0, top.1)?;
    if desk.outer(files)? != desk.engine.work_area() {
        return Err("top-edge drop did not maximize Explorer".into());
    }
    let maximized = caption(&desk)?;
    // Pull down from the maximized title.
    let (pressed, released) = ((lw / 2, 16), (lw / 2, 160));
    let restored = desk.restore_title_drag(files, pressed, released)?;
    let restored_caption = caption(&desk)?;
    // Grab the restored title at its centre and drop on the left edge.
    let grab = (
        restored.x() + restored.width() as i32 / 2,
        restored.y() + 16,
    );
    if desk.hit(grab.0, grab.1) != Some((files, g::HitRegion::TitleBar)) {
        return Err("restored Explorer centre is not caption".into());
    }
    let left = (0, 400);
    desk.drop_at(files, left.0, left.1)?;
    let half = desk.outer(files)?;
    let half_caption = caption(&desk)?;
    // Pull the half window out by its caption to the centre.
    let out = (lw / 2, 180);
    let again = desk.restore_title_drag(files, half_caption, out)?;
    let again_caption = caption(&desk)?;
    let points = [restored, half, again].map(beside);
    for ((body, outside), r) in points.iter().zip([restored, half, again]) {
        if !r.contains(body.0, body.1)
            || r.contains(outside.0, outside.1)
            || outside.0 >= lw
            || outside.1 >= lh
        {
            return Err(format!("probes beside {r:?} leave the frame or the window"));
        }
    }
    Ok(NativeDefault {
        logical,
        caption: [
            pixel(placed)?,
            pixel(maximized)?,
            pixel(restored_caption)?,
            pixel(half_caption)?,
            pixel(again_caption)?,
        ],
        to_top: [px(placed), (output.0 as i32 / 2, 0)],
        pull_down: [px(pressed), px(released)],
        to_left: [px(grab), px(left)],
        pull_out: [px(half_caption), px(out)],
        body: [
            pixel(points[0].0)?,
            pixel(points[1].0)?,
            pixel(points[2].0)?,
        ],
        outside: [
            pixel(points[0].1)?,
            pixel(points[1].1)?,
            pixel(points[2].1)?,
        ],
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scene_wallpaper_is_the_decoded_default_background() {
        let blob = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../assets/wallpapers/windows/desktop-assets.blob"),
        )
        .unwrap();
        let pack = door_window::wallpaper::Pack::parse(&blob).unwrap();
        let mut cache = Vec::new();
        door_window::wallpaper::load(pack.background(0).unwrap(), SCENE.0, SCENE.1, &mut cache)
            .unwrap();
        let (x, y) = scene().unwrap().outside;
        let at = (y * SCENE.0 as usize + x) * 4;
        // The cache is BGRA; probes are RGB.
        assert_eq!([cache[at + 2], cache[at + 1], cache[at]], SCENE_WALLPAPER);
    }
    #[test]
    fn scene_points_replay_the_notes_window_on_the_authority() {
        let s = scene().unwrap();
        // Files and Notes share the initial placement; Notes' taskbar button
        // is the second app slot.
        let initial = placement(App::Notes, SCENE).unwrap();
        assert_eq!(initial, placement(App::Files, SCENE).unwrap());
        assert_eq!(initial, g::Rect::new(153, 79, 717, 562).unwrap());
        let bar = door_window::taskbar::Layout::new(SCENE.0, SCENE.1);
        assert_eq!(
            bar.hit_slot(s.activate.0, s.activate.1, door_window::apps::COUNT),
            Some(door_window::taskbar::Slot::App(1))
        );
        assert_eq!(s.activate, (600, 744));
        // Caption drag by (-50, -46), then the bottom-right corner by
        // (+120, +60): moved (103, 33, 717, 562), resized (103, 33, 837, 622).
        assert_eq!(s.moves, [(707, 105), (657, 59)]);
        assert_eq!(s.resize, [(816, 591), (936, 651)]);
        let moved = g::Rect::new(103, 33, 717, 562).unwrap();
        let resized = g::Rect::new(103, 33, 837, 622).unwrap();
        let mut desk = Desk::new(SCENE.0, SCENE.1).unwrap();
        desk.open(App::Files).unwrap();
        let notes = desk.open(App::Notes).unwrap();
        desk.tabs(notes, 48, 380).unwrap();
        assert_eq!(
            desk.hit(s.moves[0].0, s.moves[0].1),
            Some((notes, g::HitRegion::TitleBar))
        );
        desk.move_to(notes, 103, 33).unwrap();
        assert_eq!(desk.outer(notes).unwrap(), moved);
        assert_eq!(
            desk.resize_drag(notes, s.resize[0], s.resize[1]).unwrap(),
            resized
        );
        assert_eq!(
            (s.maximize, s.restore, s.minimize),
            ((870, 53), (954, 20), (824, 53))
        );
        assert_eq!(desk.hit(870, 53), Some((notes, g::HitRegion::Maximize)));
        assert_eq!(desk.hit(824, 53), Some((notes, g::HitRegion::Minimize)));
        assert_eq!(s.left, [(106, 344), (86, 344)]);
        assert_eq!(s.top, [(521, 36), (521, 16)]);
        // Probes: each changes colour only if the gesture took effect.
        let has = |r: g::Rect, (x, y): (usize, usize)| r.contains(x as i32, y as i32);
        assert_eq!(s.bar, (111, 88));
        assert!(has(moved, s.bar) && !has(initial, s.bar));
        assert_eq!(s.exposed, (800, 629));
        assert!(has(initial, s.exposed) && !has(moved, s.exposed));
        assert!(
            s.exposed.1 as i32 >= initial.bottom() - 24,
            "Files status bar"
        );
        assert_eq!(s.outside, (926, 580));
        assert!(has(resized, s.outside) && !has(moved, s.outside));
        assert!(
            s.outside.0 as i32 >= initial.right() + g::WINDOW_SHADOW.right as i32,
            "clear of the shadow"
        );
        // The moved Notes darkens Explorer's status bar below it.
        assert!(
            (0..3).all(|i| s.exposed_color[i] < FILES_STATUS[i]),
            "{:?}",
            s.exposed_color
        );
        assert_eq!(
            desk.hit(926, 580),
            Some((notes, g::HitRegion::Client)),
            "Notes body (status bar from window y {})",
            resized.height() - 32
        );
        assert_eq!((s.bar_max, s.body_max), ((8, 55), (1010, 680)));
        assert!(!has(resized, s.bar_max) && !has(resized, s.body_max));
        assert_eq!((s.bar_left, s.bar_top), ((91, 88), (111, 68)));
        assert!(!has(resized, s.bar_left));
        // Before the top drag this is caption (window y 35 of 40).
        assert!(has(resized, s.bar_top) && (s.bar_top.1 as i32) < resized.y() + 40);
        // File Explorer, alone after Notes closes, right-clicked on blank
        // list space; its menu probes sit inside the 1024x768 frame.
        assert_eq!(s.files_menu, (480, 590));
        assert_eq!((s.menu_new, s.menu_last), ((680, 415), (508, 580)));
        let mut desk = Desk::new(SCENE.0, SCENE.1).unwrap();
        let files = desk.open(App::Files).unwrap();
        assert_eq!(desk.hit(480, 590), Some((files, g::HitRegion::Client)));
        assert!(has(initial, s.menu_new) && has(initial, s.menu_last));
    }
    #[test]
    fn recovery_points_avoid_the_probe_windows_and_hit_their_targets() {
        let r = recovery().unwrap();
        let initial = placement(App::Files, SCENE).unwrap();
        let (x, y, w, h) = PROBE_WINDOW;
        let probe = g::Rect::new(x, y, w, h).unwrap();
        let has = |rect: g::Rect, (x, y): (usize, usize)| rect.contains(x as i32, y as i32);
        // Explorer's caption drag by (+80, +60) and back.
        assert_eq!(r.drag, [(707, 105), (787, 165)]);
        assert_eq!(r.drag_back, [(787, 165), (707, 105)]);
        // The moved right border column and the maximized status bar lie
        // outside both the placement and the probe windows.
        assert_eq!((r.moved_border, r.max_status), ((949, 420), (954, 708)));
        for p in [r.moved_border, r.max_status] {
            assert!(!has(initial, p) && !has(probe, p), "{p:?}");
        }
        assert!(
            r.max_status.1 as i32 >= 720 - 24,
            "status bar of the work area"
        );
        // Explorer's Maximize sits over the probe rect, so it must be raised.
        assert_eq!((r.maximize, r.minimize), ((800, 99), (908, 20)));
        assert!(probe.contains(r.maximize.0, r.maximize.1));
        assert_eq!(r.notes_body, (511, 360));
        assert!(initial.contains(r.notes_body.0, r.notes_body.1));
        // The probe client and the hung producer's server-drawn Close.
        assert_eq!((r.probe, r.hung_close), ((820, 110), (954, 48)));
        let mut desk = Desk::new(SCENE.0, SCENE.1).unwrap();
        let hung = desk.open_foreign(PROBE_WINDOW).unwrap();
        assert_eq!(desk.hit(954, 48), Some((hung, g::HitRegion::Close)));
    }
    #[test]
    fn native_default_points_follow_the_automatic_scale() {
        for (percent, logical) in [(150, (1707, 1067)), (200, (1280, 800))] {
            let scale = g::Scale::from_percent(percent).unwrap();
            let n = native_default((2560, 1600), scale).unwrap();
            assert_eq!(n.logical, logical);
            // Explorer opens at its placement on the logical desktop.
            let o = placement(App::Files, logical).unwrap();
            let (cx, cy) = n.caption[0];
            assert!(scale.rect(o).unwrap().contains(cx as i32, cy as i32));
            assert_eq!(n.to_top[1], (1280, 0));
            // Each beside probe pair straddles the window's right edge.
            for (b, o) in n.body.iter().zip(n.outside) {
                assert!(o.0 > b.0 && o.0 < 2560 && o.1 < 1600);
            }
        }
        let n = native_default((2560, 1600), g::Scale::from_percent(150).unwrap()).unwrap();
        assert_eq!(n.caption[1], (2316, 39), "maximized caption");
        assert_eq!(n.pull_down, [(1279, 24), (1279, 240)]);
        assert_eq!(n.to_left, [(1281, 240), (0, 600)]);
        assert_eq!(n.body[1], (1249, 600), "left half: 1707 / 2 - 20 DIP");
        assert_eq!(n.outside[1], (1363, 150));
    }
    #[test]
    fn scene_resize_probe_is_inside_notes_right_edge() {
        let (x, y) = scene_resize_probe().unwrap();
        let o = placement(App::Notes, (1024, 768)).unwrap();
        assert!(x >= o.right() - 4 && x < o.right(), "{x} vs {}", o.right());
        assert_eq!(y, o.y() + o.height() as i32 / 2);
    }
    #[test]
    fn wide_max_points_come_from_placement_and_caption_hit_testing() {
        // Every first window opens at 70% x 78% of the 1280x672 work area,
        // centred above the taskbar band.
        let mut desk = Desk::new(1280, 720).unwrap();
        assert_eq!(desk.work_area(), g::Rect::new(0, 0, 1280, 672).unwrap());
        for app in [App::Files, App::Notes, App::Settings] {
            let id = desk.open(app).unwrap();
            let o = desk.outer(id).unwrap();
            assert_eq!((o.x(), o.y(), o.width(), o.height()), (192, 74, 896, 524));
            assert_eq!(origin(app).unwrap(), (192, 74));
        }
        // Maximize buttons: 46 px wide, second from the right, mid caption
        // (normal windows keep a 4 px resize border inside Close).
        assert_eq!(wide_max(App::Files).unwrap(), ((1018, 94), (1210, 20)));
        assert_eq!(wide_max(App::Notes).unwrap(), ((1018, 94), (1210, 20)));
        assert_eq!(wide_max(App::Settings).unwrap(), ((1018, 90), (1210, 16)));
        // desktop-motion: Settings Maximize, then Minimize left of Restore.
        let (maximize, minimize) = settings_max_min().unwrap();
        assert_eq!(maximize, (1018, 90));
        assert!(
            minimize.1 == 16 && minimize.0 < 1210 && minimize.0 > 1100,
            "{minimize:?}"
        );
        // shell.rs: the three caption buttons, 46 px apart, left to right.
        let [min, max, close] = settings_caption().unwrap();
        assert_eq!(max, (1018, 90));
        // Its close-cancel probe, 14 px left of Close, is asserted on the
        // Settings painter (`window_management_probes_hit_the_settings_background`).
        assert_eq!(close, (1062, 90));
        assert!(min.1 == 90 && close.1 == 90 && min.0 < max.0 && max.0 < close.0);
        assert!(
            close.0 < 192 + 896 && max.0 - min.0 == 46,
            "{min:?} {max:?} {close:?}"
        );
        // Maximized windows fill the work area; restore returns the placement.
        let mut desk = Desk::new(1280, 720).unwrap();
        let id = desk.open(App::Settings).unwrap();
        desk.maximize(id).unwrap();
        assert_eq!(desk.outer(id).unwrap(), desk.work_area());
        // The colour probe (1270, 630) lies inside every maximized window.
        assert!(desk.outer(id).unwrap().contains(1270, 630));
        desk.restore(id).unwrap();
        assert_eq!(
            desk.outer(id).unwrap(),
            g::Rect::new(192, 74, 896, 524).unwrap()
        );
    }
    #[test]
    fn resolution_modes_maximize_buttons_hit_the_real_caption() {
        // desktop-resolution 1920x1080 (1x) and 2560x1600 (2x: 1280x800).
        for output in [(1920, 1080), (1280, 800)] {
            for app in [App::Files, App::Notes, App::Settings] {
                let mut desk = Desk::new(output.0, output.1).unwrap();
                let id = desk.open(app).unwrap();
                let (x, y) = maximize_button(app, output).unwrap();
                assert_eq!(
                    desk.hit(x, y),
                    Some((id, g::HitRegion::Maximize)),
                    "{app:?}"
                );
            }
        }
        // 1920x1080 leaves the requested sizes; 1280x800 caps them.
        assert_eq!(
            placement(App::Files, (1920, 1080)).unwrap(),
            g::Rect::new(460, 196, 1000, 640).unwrap()
        );
        assert_eq!(
            placement(App::Settings, (1920, 1080)).unwrap(),
            g::Rect::new(395, 122, 1129, 787).unwrap()
        );
        assert_eq!(
            placement(App::Notes, (1280, 800)).unwrap(),
            g::Rect::new(192, 82, 896, 587).unwrap()
        );
        // multiwindow.rs tears tabs off maximized windows at 1920x1080 and
        // drops them at (1000, 400): the extra window (index 1, at the
        // pointer less (80, 20)) is 960x600 for Explorer and Preview, so the
        // dragged tab stays under the drop point.
        for requested in [(1000, 640), (1280, 800)] {
            assert_eq!(
                g::initial_window((1920, 1080), requested, 1, Some((920, 380))),
                (920, 380, 960, 600)
            );
        }
    }
    #[test]
    fn explorer_drag_point_is_a_caption_drag_region_below_the_snap_bar() {
        let ((x, y), left) = explorer_drag().unwrap();
        assert_eq!(((x, y), left), ((925, 100), 192));
        let mut desk = Desk::new(1280, 720).unwrap();
        let id = desk.open(App::Files).unwrap();
        // Any tab strip ending 24 px before Minimize keeps the point draggable.
        for strip in [1, 120, 300, 600, 720] {
            desk.tabs(id, 8, strip).unwrap();
            assert_eq!(
                desk.hit(x, y),
                Some((id, g::HitRegion::TitleBar)),
                "{strip}"
            );
        }
        // Horizontal steps never reach the snap-layouts reveal band.
        let area = desk.work_area();
        for dx in [0, 12, 24, 36] {
            assert!(!door_window::snap::reveals(area, x + dx, y));
        }
        // lifecycle.rs probes the left border column at y 500, inside the
        // window, before and after the 36 px drag.
        let o = desk.outer(id).unwrap();
        assert!(o.y() < 500 && 500 < o.bottom());
        assert_eq!(border(), [180, 180, 180]);
    }
    #[test]
    fn settings_points_hit_the_settings_window() {
        let mut desk = Desk::new(1280, 720).unwrap();
        let id = desk.open(App::Settings).unwrap();
        let (x, y) = settings_background().unwrap();
        assert_eq!((x, y), (267, 204));
        assert_eq!(desk.hit(x, y), Some((id, g::HitRegion::Client)));
        // multitask.rs drags this caption point to the top edge to maximize.
        let (x, y) = settings_drag().unwrap();
        assert_eq!(desk.hit(x, y), Some((id, g::HitRegion::TitleBar)));
        assert!(!door_window::snap::reveals(desk.work_area(), x, y));
        assert!(y > 74 && y < 74 + 32, "{y}");
        // Its drop at the top edge (x, 0) maximizes: it is outside the
        // top-centred snap-layouts flyout (which therefore never opens) and on
        // the top edge snap.
        let area = desk.work_area();
        assert!(!door_window::snap::reveals(area, x, 0));
        assert_eq!(
            door_window::snap::hit(door_window::snap::drag_flyout(area), x, 0),
            None
        );
        assert_eq!(
            door_window::policy::edge_snap(area, g::Scale::ONE, x, 0),
            Some(g::Snap::Maximize)
        );
        // shell.rs releases a cancelled Close press on this client point.
        assert_eq!(desk.hit(1000, 110), Some((id, g::HitRegion::Client)));
    }
    #[test]
    fn motion_points_keep_drags_on_the_caption_and_expose_settings() {
        let m = motion().unwrap();
        assert_eq!(m.hover_close, (1062, 94));
        assert_eq!(m.start, (925, 100));
        assert_eq!(m.exposed, (256, 204));
        let (mut desk, [files, _notes, settings]) = motion_desk().unwrap();
        let area = desk.work_area();
        for p in [m.start, m.end, m.alternate] {
            assert!(!door_window::snap::reveals(area, p.0, p.1));
        }
        assert_eq!(
            desk.hit(m.hover_close.0, m.hover_close.1),
            Some((files, g::HitRegion::Close))
        );
        // Before the drag the exposed point is Explorer content, the chrome
        // point Explorer's caption.
        assert_eq!(desk.hit(m.exposed.0, m.exposed.1).map(|h| h.0), Some(files));
        assert_eq!(
            desk.hit(m.chrome_before.0, m.chrome_before.1),
            Some((files, g::HitRegion::TitleBar))
        );
        // The first (+120, +20) drag keeps Explorer inside the work area
        // (translated-pixel reuse); Settings shows at the exposed point and
        // Explorer's caption at the moved chrome point.
        let o = desk.outer(files).unwrap();
        desk.move_to(files, o.x() + 120, o.y() + 20).unwrap();
        let moved = desk.outer(files).unwrap();
        assert!(moved.x() >= 0 && moved.right() <= 1280 && moved.bottom() <= area.bottom());
        assert_eq!(
            desk.hit(m.exposed.0, m.exposed.1),
            Some((settings, g::HitRegion::Client))
        );
        // ... outside the moved window's shadow, which can darken a 3x pixel.
        assert!(m.exposed.0 < moved.x() - g::WINDOW_SHADOW.left as i32);
        assert_eq!(
            desk.hit(m.chrome_after.0, m.chrome_after.1),
            Some((files, g::HitRegion::TitleBar))
        );
    }
    #[test]
    fn notes_points_follow_the_initial_placement() {
        assert_eq!(notes_window().unwrap(), (192, 74, 896, 524));
        assert_eq!(
            notes_dialog(896, 524).1[0],
            (248, 322, 128, 32),
            "Save button of the full-size prompt"
        );
        // lifecycle.rs shutdown prompt probe.
        assert_eq!(notes_save_focus().unwrap(), (445, 405));
        // shell.rs focus probe: inside the body (x 20..876, rows 86..486),
        // above the 32 px status bar at window y 492.
        let (x, y) = NOTES_BODY_POINT;
        // Inside the body (16..880, 89..492) of the 896 x 524 client.
        assert!((16..880).contains(&x) && (89..492).contains(&y));
        assert_eq!(screen(App::Notes, NOTES_BODY_POINT).unwrap(), (840, 540));
    }
}
