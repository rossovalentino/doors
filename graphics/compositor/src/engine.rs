use crate::*;
use alloc::{
    collections::VecDeque,
    string::{String, ToString},
    vec,
    vec::Vec,
};

/// The service MUST construct this from authenticated IPC metadata, never wire fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Owner {
    pub pid: u64,
    pub session: u64,
    pub epoch: u64,
}
/// The caption offers Minimize (`set_caption_buttons`).
pub const CAPTION_MINIMIZE: u8 = 1;
/// The caption offers Maximize.
pub const CAPTION_MAXIMIZE: u8 = 2;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    TopLevel,
    OwnedPopup(WindowId),
    ShellPanel,
    LockScreen,
    TrustedPrompt,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowState {
    Normal,
    Minimized,
    Maximized,
    Closing,
}
pub(crate) mod acrylic;
pub(crate) mod animation;
mod arrangement;
pub(crate) mod damage;
pub use damage::MAX_DAMAGE_RECTS;
pub(crate) mod material;
/// Mica as the compositor composes it behind a focused Mica window's
/// transparent pixels: an opaque BGRA wallpaper at the output size, reduced,
/// blurred and tinted for `theme` exactly as rendering does. Host checks of
/// guest captures sample it at screen points.
pub struct MicaField(material::Backdrop);
impl MicaField {
    pub fn new(wallpaper: &[u8], width: u32, height: u32, theme: Theme) -> Result<Self> {
        Ok(Self(
            material::Backdrop::new(wallpaper, width, height)?.mica(theme),
        ))
    }
    /// The Mica pixel (BGRA) at screen point (`x`, `y`).
    pub fn pixel(&self, x: i32, y: i32) -> [u8; 4] {
        self.0.pixel(x, y)
    }
}
mod move_reuse;
pub use animation::{
    TRANSITION_CLOSE, TRANSITION_MINIMIZE, TRANSITION_OPEN, TRANSITION_RELOCATE,
    TRANSITION_RESTORE, TransitionActivity,
};
pub use arrangement::{Snap, WorkspaceId};
/// Material behind a window's translucent client pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Material {
    /// Theme content, or the cached wallpaper frost for tabbed captions.
    #[default]
    Default,
    /// Live blur of the scene composed beneath the window in each frame.
    Acrylic,
    /// Mica: the wallpaper, heavily blurred under the theme's luminosity
    /// tint (computed once per wallpaper and theme), behind the client's
    /// translucent pixels and the trusted title bar while the window is
    /// active; the solid Mica fallback colour while it is inactive or
    /// transparency is reduced (Windows 11 window backdrop of Settings,
    /// File Explorer and Notepad).
    Mica,
}
/// Native client title-bar content with trusted system buttons and drag policy.
/// The centered control shrinks below `max_width` to preserve both side margins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptionLayout {
    /// Left-aligned app tabs; trusted system buttons always retain priority.
    pub tabbed: bool,
    pub height: u32,
    pub max_width: u32,
    pub side_margin: u32,
}
impl CaptionLayout {
    pub fn validate(self) -> Result<()> {
        if !(32..=72).contains(&self.height)
            || self.max_width == 0
            || self.max_width > if self.tabbed { 6144 } else { 2048 }
            || !(if self.tabbed { 0..=2048 } else { 138..=2048 }).contains(&self.side_margin)
        {
            return Err(Error::InvalidGeometry);
        }
        Ok(())
    }
    pub fn control(self, outer: Rect) -> Option<Rect> {
        self.control_scaled(outer, Scale::ONE)
    }
    fn control_scaled(self, outer: Rect, scale: Scale) -> Option<Rect> {
        if self.tabbed {
            let width = outer
                .width()
                .saturating_sub(self.side_margin + scale.len(138))
                .min(self.max_width);
            return (width > 0).then(|| {
                Rect::new(
                    outer.x() + self.side_margin as i32,
                    outer.y(),
                    width,
                    scale.len(self.height),
                )
                .unwrap()
            });
        }
        let width = self.max_width.min(
            outer
                .width()
                .saturating_sub(self.side_margin.saturating_mul(2)),
        );
        (width > 0).then(|| {
            Rect::new(
                outer.x() + ((outer.width() - width) / 2) as i32,
                outer.y(),
                width,
                scale.len(TITLE_HEIGHT),
            )
            .unwrap()
        })
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub windows_per_owner: usize,
    pub buffers_per_owner: usize,
    pub bytes_per_owner: usize,
    pub total_bytes: usize,
    pub events_per_owner: usize,
    pub total_windows: usize,
    pub total_buffers: usize,
    pub total_owners: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            windows_per_owner: 8,
            buffers_per_owner: 8,
            bytes_per_owner: 8 * 1024 * 1024,
            total_bytes: 12 * 1024 * 1024,
            events_per_owner: 64,
            total_windows: 32,
            total_buffers: 32,
            total_owners: 16,
        }
    }
}
struct Slot<T> {
    generation: u32,
    value: Option<T>,
}
struct Client {
    owner: Owner,
    events: VecDeque<Event>,
    dropped_notifications: u64,
}
struct Buffer {
    owner: Owner,
    descriptor: SurfaceDescriptor,
    attached: Option<WindowId>,
}
pub(crate) struct Snapshot {
    pub revision: u64,
    pub epoch: u64,
    pub descriptor: SurfaceDescriptor,
    pub bytes: Vec<u8>,
    pub opaque: bool,
    /// Buffer the snapshot was captured from; band commits must match it.
    pub source: BufferId,
    /// Rows that may hold non-opaque pixels. A full capture marks every row
    /// when any pixel is translucent; band commits refine their own rows.
    pub translucent: Vec<bool>,
}
pub(crate) struct Window {
    pub workspace: WorkspaceId,
    pub snapped: bool,
    pub scale: Scale,
    pub id: WindowId,
    pub owner: Owner,
    pub role: Role,
    pub outer: Rect,
    pub restore: Rect,
    pub state: WindowState,
    pre_minimize: WindowState,
    pre_close: WindowState,
    pub title: String,
    pub cursor: CursorIcon,
    pub epoch: u64,
    acknowledged: bool,
    pending: Option<(BufferId, u64)>,
    pub snapshot: Option<Snapshot>,
    pub caption: Option<CaptionLayout>,
    pub material: Material,
    frame_requested: bool,
    /// No trusted caption buttons, border or title bar (a Win32 window
    /// without WS_CAPTION): every pixel is client area.
    pub frameless: bool,
    /// The caption's Minimize/Maximize (`CAPTION_*` bits; both by default).
    pub caption_buttons: u8,
    /// Full screen (F11 in Terminal, Win32 borderless full screen): the
    /// whole output, frameless and square, drawn over the taskbar while
    /// focused. Its state is Maximized so every maximized rule applies.
    pub fullscreen: bool,
    /// Placement to return to when full screen ends.
    pre_fullscreen: (Rect, WindowState, bool, bool),
}
impl Window {
    /// The caption shows this button (Close always; Minimize and Maximize
    /// unless the window offers neither).
    pub(crate) fn caption_shows(&self, region: HitRegion) -> bool {
        match region {
            HitRegion::Close => true,
            HitRegion::Minimize | HitRegion::Maximize => self.caption_buttons != 0,
            _ => false,
        }
    }
    /// The caption button acts (a shown but disabled one is greyed).
    pub(crate) fn caption_enabled(&self, region: HitRegion) -> bool {
        match region {
            HitRegion::Close => true,
            HitRegion::Minimize => self.caption_buttons & CAPTION_MINIMIZE != 0,
            HitRegion::Maximize => self.caption_buttons & CAPTION_MAXIMIZE != 0,
            _ => false,
        }
    }
    pub(crate) fn title_height(&self) -> u32 {
        self.caption.map_or(TITLE_HEIGHT, |c| c.height)
    }
    pub(crate) fn content(&self) -> Rect {
        if self.caption.is_some() {
            self.outer
        } else {
            Engine::content_rect_scaled(self.outer, self.scale).expect("validated window")
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct WindowInfo<'a> {
    pub workspace: WorkspaceId,
    pub snapped: bool,
    pub client_caption: bool,
    pub caption_height: u32,
    pub id: WindowId,
    pub owner: Owner,
    pub role: Role,
    pub outer: Rect,
    pub content: Rect,
    pub epoch: u64,
    pub state: WindowState,
    pub visible: bool,
    pub active: bool,
    pub title: &'a str,
    pub cursor: CursorIcon,
}
pub struct SnapshotView<'a> {
    pub revision: u64,
    pub descriptor: SurfaceDescriptor,
    pub bytes: &'a [u8],
}
/// One CPU frame, in premultiplied BGRA. The caller may add text/chrome through
/// this private frame before transfer. Drop the borrow before complete_transfer.
pub struct Frame<'a> {
    pub pixels: &'a mut [u8],
    pub width: u32,
    pub height: u32,
    pub stride_bytes: usize,
    pub serial: u64,
    /// Bounding box of every composed region.
    pub damage: Option<Rect>,
    /// The composed regions (disjoint unless the caller opted into a single
    /// box); transfer these rather than the bounding box.
    pub regions: [Option<Rect>; MAX_DAMAGE_RECTS],
    /// Opaque client pixels translated from the preceding composition.
    pub reused_pixels: u64,
}
/// Completed private composition without an output cursor overlay. Borrowing
/// this does not expose producer memory and cannot overlap an in-flight render.
pub struct OutputView<'a> {
    pub pixels: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub stride_bytes: usize,
}
pub struct Engine {
    workspace: WorkspaceId,
    workspaces: [(u32, bool); 8],
    pub(crate) scale: Scale,
    pub(crate) output: Rect,
    work_area: Rect,
    pub(crate) theme: Theme,
    limits: Limits,
    windows: Vec<Slot<Window>>,
    buffers: Vec<Slot<Buffer>>,
    clients: Vec<Client>,
    pub(crate) z_order: Vec<WindowId>,
    pub(crate) focused: Option<WindowId>,
    capture: Option<WindowId>,
    /// Window that received the latest routed pointer input (pointer leave).
    pointer_target: Option<WindowId>,
    pub(crate) wallpaper_material: Option<material::Backdrop>,
    /// Mica grid of the wallpaper in the current theme.
    pub(crate) mica_material: Option<material::Backdrop>,
    /// User preference: translucent materials render as a solid fallback.
    pub(crate) reduce_transparency: bool,
    /// User preference: the active window's trusted title bar and border
    /// paint with this accent (opaque BGRA); None keeps the theme chrome.
    pub(crate) title_accent: Option<[u8; 4]>,
    /// Doors extra (MicaForEveryone): top-level windows that chose no
    /// material get Mica (`set_mica_everywhere`).
    pub(crate) mica_everywhere: bool,
    pub(crate) motion: animation::Motion,
    /// Window shadow masks per size, scale and active state.
    pub(crate) shadows: core::cell::RefCell<crate::shadow::ShadowCache>,
    /// Test oracle: compose every window pixel through the scalar contour
    /// path instead of the interior row spans.
    #[cfg(test)]
    pub(crate) scalar_reference: bool,
    caption_pointer: Option<(i32, i32)>,
    caption_hover: Option<(WindowId, HitRegion)>,
    caption_press: Option<(WindowId, HitRegion)>,
    caption_left_down: bool,
    caption_damage: Option<Rect>,
    /// Bounding box of pending damage (None: nothing to compose).
    damage: Option<Rect>,
    /// The same damage as a short list of rectangles (see `damage`).
    damage_list: damage::DamageList,
    damage_limit: usize,
    split_allowed: bool,
    move_reuse: Option<move_reuse::MoveReuse>,
    pub(crate) pixels: Vec<u8>,
    next_frame: u64,
    in_flight: Option<(u64, Vec<(Owner, WindowId)>)>,
}
fn raw(slot: usize, generation: u32) -> u64 {
    ((generation as u64) << 32) | (slot as u64 + 1)
}
fn parts(id: u64) -> Result<(usize, u32)> {
    let low = id as u32;
    let generation = (id >> 32) as u32;
    if low == 0 || generation == 0 {
        return Err(Error::UnknownId);
    }
    Ok(((low - 1) as usize, generation))
}
fn lookup<T>(slots: &[Slot<T>], id: u64) -> Result<&T> {
    let (slot, generation) = parts(id)?;
    slots
        .get(slot)
        .filter(|s| s.generation == generation)
        .and_then(|s| s.value.as_ref())
        .ok_or(Error::UnknownId)
}
fn lookup_mut<T>(slots: &mut [Slot<T>], id: u64) -> Result<&mut T> {
    let (slot, generation) = parts(id)?;
    slots
        .get_mut(slot)
        .filter(|s| s.generation == generation)
        .and_then(|s| s.value.as_mut())
        .ok_or(Error::UnknownId)
}
fn insert<T>(slots: &mut Vec<Slot<T>>, value: T) -> u64 {
    if let Some((i, slot)) = slots
        .iter_mut()
        .enumerate()
        .find(|(_, s)| s.value.is_none() && s.generation < u32::MAX)
    {
        slot.generation += 1;
        slot.value = Some(value);
        return raw(i, slot.generation);
    }
    let id = raw(slots.len(), 1);
    slots.push(Slot {
        generation: 1,
        value: Some(value),
    });
    id
}
impl Engine {
    pub fn new(width: u32, height: u32, theme: Theme, limits: Limits) -> Result<Self> {
        if width > MAX_WIDTH
            || height > MAX_HEIGHT
            || width < 160
            || height < 96
            || limits.events_per_owner < 8
            || limits.total_windows > 4096
            || limits.total_buffers > 4096
            || limits.total_owners > 1024
        {
            return Err(Error::InvalidGeometry);
        }
        let output = Rect::new(0, 0, width, height)?;
        Ok(Self {
            workspace: WorkspaceId(raw(0, 1)),
            workspaces: [
                (1, true),
                (0, false),
                (0, false),
                (0, false),
                (0, false),
                (0, false),
                (0, false),
                (0, false),
            ],
            scale: Scale::ONE,
            output,
            work_area: output,
            theme,
            limits,
            windows: Vec::new(),
            buffers: Vec::new(),
            clients: Vec::new(),
            z_order: Vec::new(),
            focused: None,
            capture: None,
            pointer_target: None,
            wallpaper_material: None,
            mica_material: None,
            reduce_transparency: false,
            title_accent: None,
            mica_everywhere: false,
            motion: animation::Motion::default(),
            shadows: core::cell::RefCell::default(),
            #[cfg(test)]
            scalar_reference: false,
            caption_pointer: None,
            caption_hover: None,
            caption_press: None,
            caption_left_down: false,
            caption_damage: None,
            damage: Some(output),
            damage_list: damage::DamageList::single(output),
            damage_limit: 1,
            split_allowed: true,
            move_reuse: None,
            pixels: vec![0; width as usize * height as usize * 4],
            next_frame: 1,
            in_flight: None,
        })
    }
    /// Trusted server supplies the selected wallpaper, never client pixels.
    /// Cache is reduced and blurred once, reused during moves and hover frames.
    pub fn set_wallpaper_material(&mut self, pixels: &[u8]) -> Result<()> {
        if self.in_flight.is_some() {
            return Err(Error::Busy);
        }
        let material = material::Backdrop::new(pixels, self.output.width(), self.output.height())?;
        self.mica_material = Some(material.mica(self.theme));
        self.wallpaper_material = Some(material);
        self.invalidate();
        Ok(())
    }
    pub fn set_tabbed_caption(
        &mut self,
        owner: Owner,
        id: WindowId,
        left: u32,
        width: u32,
    ) -> Result<()> {
        let previous = self.owned(owner, id)?.caption.ok_or(Error::InvalidState)?;
        let layout = CaptionLayout {
            tabbed: true,
            height: previous.height,
            side_margin: left,
            max_width: width,
        };
        layout.validate()?;
        let window = lookup_mut(&mut self.windows, id.0)?;
        if !window.caption.is_some_and(|c| c.tabbed) {
            return Err(Error::InvalidState);
        }
        window.caption = Some(layout);
        Ok(())
    }
    fn client_index(&self, owner: Owner) -> Option<usize> {
        self.clients.iter().position(|c| c.owner == owner)
    }
    fn ensure_client(&mut self, owner: Owner) -> Result<()> {
        if self.client_index(owner).is_none() {
            if self.clients.len() >= self.limits.total_owners {
                return Err(Error::Quota);
            }
            self.clients.push(Client {
                owner,
                events: VecDeque::new(),
                dropped_notifications: 0,
            });
        }
        Ok(())
    }
    fn room(&self, owner: Owner, count: usize) -> Result<()> {
        let n = self
            .client_index(owner)
            .map(|i| self.clients[i].events.len())
            .unwrap_or(0);
        if n.saturating_add(count) > self.limits.events_per_owner {
            Err(Error::QueueFull)
        } else {
            Ok(())
        }
    }
    fn emit(&mut self, owner: Owner, event: Event) {
        if let Some(i) = self.client_index(owner) {
            self.clients[i].events.push_back(event);
        }
    }
    pub fn poll_event(&mut self, owner: Owner) -> Option<Event> {
        let i = self.client_index(owner)?;
        let event = self.clients[i].events.pop_front();
        if event.is_some() {
            // A backpressured frame callback becomes eligible when its owner
            // drains events. Merely retaining the request never spins rendering.
            let pending: Vec<_> = self
                .windows
                .iter()
                .filter_map(|s| s.value.as_ref())
                .filter(|w| w.owner == owner && w.frame_requested && self.visible(w.id))
                .map(|w| w.outer)
                .collect();
            for rect in pending {
                self.dirty(rect);
            }
        }
        event
    }
    pub fn pending_events(&self, owner: Owner) -> usize {
        self.client_index(owner)
            .map(|i| self.clients[i].events.len())
            .unwrap_or(0)
    }
    /// Advisory focus/capture/presentation events may be coalesced or dropped for
    /// an unresponsive owner. Releases and accepted key/button input are reliable.
    /// A consumer observing a changed count must reconcile window_info state.
    pub fn dropped_notifications(&self, owner: Owner) -> u64 {
        self.client_index(owner)
            .map(|i| self.clients[i].dropped_notifications)
            .unwrap_or(0)
    }
    fn advisory(&mut self, owner: Owner, event: Event) {
        if self.room(owner, 1).is_ok() {
            self.emit(owner, event);
        } else if let Some(i) = self.client_index(owner) {
            self.clients[i].dropped_notifications =
                self.clients[i].dropped_notifications.saturating_add(1);
        }
    }
    fn owned(&self, owner: Owner, id: WindowId) -> Result<&Window> {
        let w = lookup(&self.windows, id.0)?;
        if w.owner != owner {
            return Err(Error::WrongOwner);
        }
        if w.state == WindowState::Closing {
            return Err(Error::Closing);
        }
        Ok(w)
    }
    fn buffer(&self, owner: Owner, id: BufferId) -> Result<&Buffer> {
        let b = lookup(&self.buffers, id.0)?;
        if b.owner != owner {
            return Err(Error::WrongOwner);
        }
        Ok(b)
    }
    fn validate_outer(&self, outer: Rect) -> Result<()> {
        if outer.width() < self.scale.len(160)
            || outer.height() < self.scale.len(64)
            || outer.width() > MAX_WIDTH
            || outer.height() > MAX_HEIGHT
            || outer.x() < -(MAX_WIDTH as i32)
            || outer.y() < -(MAX_HEIGHT as i32)
            || outer.x() > MAX_WIDTH as i32
            || outer.y() > MAX_HEIGHT as i32
        {
            return Err(Error::InvalidGeometry);
        }
        Ok(())
    }
    /// Integer device scale (1..=3) of an empty scene.
    pub fn set_scale(&mut self, factor: u32) -> Result<()> {
        self.set_scale_percent(Scale::from_factor(factor).ok_or(Error::InvalidState)?)
    }
    /// Output scale of an empty scene (before any window exists). A live
    /// scene changes scale with `rescale`.
    pub fn set_scale_percent(&mut self, scale: Scale) -> Result<()> {
        if self.windows().next().is_some() || self.in_flight.is_some() {
            return Err(Error::InvalidState);
        }
        self.scale = scale;
        self.invalidate();
        Ok(())
    }
    pub fn scale(&self) -> Scale {
        self.scale
    }
    pub fn content_rect(outer: Rect) -> Result<Rect> {
        Self::content_rect_scaled(outer, Scale::ONE)
    }
    fn content_rect_scaled(outer: Rect, scale: Scale) -> Result<Rect> {
        // All engine-owned windows have a 32 DIP nonclient title bar.
        let title = scale.len(TITLE_HEIGHT);
        let height = outer
            .height()
            .checked_sub(title)
            .ok_or(Error::InvalidGeometry)?;
        let y = outer
            .y()
            .checked_add(title as i32)
            .ok_or(Error::InvalidGeometry)?;
        Rect::new(outer.x(), y, outer.width(), height)
    }
    /// Text-only title area with 12 DIP side padding, 4 DIP vertical
    /// gutters, and all three 46 DIP caption controls excluded.
    pub fn title_clip(outer: Rect) -> Option<Rect> {
        Self::title_clip_scaled(outer, Scale::ONE)
    }
    pub fn title_clip_scaled(outer: Rect, scale: Scale) -> Option<Rect> {
        let left = outer.x().checked_add(scale.len(12) as i32)?;
        let right = outer.right().checked_sub(scale.len(12 + 138) as i32)?;
        let top = outer.y().checked_add(scale.len(4) as i32)?;
        let bottom = outer.y().checked_add(scale.len(TITLE_HEIGHT - 4) as i32)?;
        Rect::new(
            left,
            top,
            u32::try_from(right - left).ok()?,
            u32::try_from(bottom - top).ok()?,
        )
        .ok()
    }
    /// Damage from a window's geometry or state (`rect` is its outer
    /// rectangle): the window plus its whole shadow.
    fn dirty(&mut self, rect: Rect) {
        self.damage_area(crate::shadow::window_shadow_bounds(rect, self.scale));
    }
    /// Damage from pixels changing inside `rect` (client content, trusted
    /// overlays): a small margin covers antialiased edges and the blur of
    /// live acrylic above, without the window shadow's reach.
    fn dirty_content(&mut self, rect: Rect) {
        self.damage_area(rect.expanded(self.scale.len(render::ELEVATION_MARGIN as u32) as i32));
    }
    fn damage_area(&mut self, area: Option<Rect>) {
        self.move_reuse = None;
        let clipped = area.unwrap_or(self.output).intersection(self.output);
        if let Some(r) = clipped {
            self.damage = Some(self.damage.and_then(|d| d.union(r)).unwrap_or(r));
            self.damage_list.add(r, self.damage_limit);
        }
    }
    /// Trusted opt-in: compose up to `limit` separate damage rectangles per
    /// frame (`Frame::regions`) instead of their bounding box. The overlay
    /// callback then runs once, clipped to the bounding box, after all
    /// regions are composed; pixels between regions are retained from the
    /// previous frame, so an overlay must restore its own underlay there.
    pub fn set_damage_regions(&mut self, limit: usize) {
        self.damage_limit = limit.clamp(1, damage::MAX_DAMAGE_RECTS);
        self.invalidate();
    }
    /// Per-frame veto of split regions (for example while a floating shell
    /// overlay that does not restore its own underlay is visible): the next
    /// composition uses the bounding box. Pending damage is unchanged.
    pub fn allow_split_regions(&mut self, allow: bool) {
        self.split_allowed = allow;
    }
    fn dirty_tree(&mut self, id: WindowId) {
        let bounds: Vec<_> = self
            .z_order
            .iter()
            .copied()
            .filter(|child| self.is_descendant(*child, id))
            .filter_map(|child| self.window(child).map(|w| w.outer))
            .collect();
        for bounds in bounds {
            self.dirty(bounds);
        }
    }
    pub fn has_damage(&self) -> bool {
        self.damage.is_some()
    }
    pub fn output_rect(&self) -> Rect {
        self.output
    }
    pub fn has_snapshot(&self, id: WindowId) -> bool {
        self.window(id)
            .is_some_and(|window| window.snapshot.is_some())
    }
    /// Keep late background applications below the current foreground family.
    pub fn lower_top_level(&mut self, owner: Owner, id: WindowId) -> Result<()> {
        if self.owned(owner, id)?.role != Role::TopLevel {
            return Err(Error::InvalidState);
        }
        let family: Vec<_> = self
            .z_order
            .iter()
            .copied()
            .filter(|child| self.is_descendant(*child, id))
            .collect();
        self.z_order.retain(|window| !family.contains(window));
        self.z_order.splice(0..0, family);
        self.dirty_tree(id);
        Ok(())
    }
    pub fn set_work_area(&mut self, rect: Rect) -> Result<()> {
        if self.windows().next().is_some() {
            return Err(Error::Busy);
        }
        if self.output.intersection(rect) != Some(rect) {
            return Err(Error::InvalidGeometry);
        }
        self.work_area = rect;
        Ok(())
    }
    /// Reserve a different shell edge without publishing a partially reflowed
    /// desktop. Preflight geometry, epochs and all owner queues before mutation.
    pub fn reflow_work_area(&mut self, rect: Rect) -> Result<()> {
        if self.output.intersection(rect) != Some(rect)
            || rect.width() < self.scale.len(160)
            || rect.height() < self.scale.len(64)
        {
            return Err(Error::InvalidGeometry);
        }
        if rect == self.work_area {
            return Ok(());
        }
        let old = self.work_area;
        let restore_plans: Vec<_> = self
            .windows()
            .filter(|w| w.role == Role::TopLevel && w.state != WindowState::Closing)
            .map(|w| {
                let saved = self.window(w.id).unwrap().restore;
                let x = saved.x().clamp(
                    rect.x() + self.scale.len(80) as i32 - saved.width() as i32,
                    rect.right() - self.scale.len(80) as i32,
                );
                let y = saved
                    .y()
                    .clamp(rect.y(), rect.bottom() - self.scale.len(32) as i32);
                (
                    w.id,
                    Rect::new(x, y, saved.width(), saved.height()).unwrap(),
                )
            })
            .collect();
        let plans: Vec<_> = self
            .windows()
            .filter(|w| w.role == Role::TopLevel && w.state != WindowState::Closing)
            .filter_map(|w| {
                // Full screen covers the output, not the work area.
                if self.is_fullscreen(w.id) {
                    return None;
                }
                let maximized = w.state == WindowState::Maximized
                    || self.window(w.id).is_some_and(|v| {
                        v.state == WindowState::Minimized
                            && v.pre_minimize == WindowState::Maximized
                    });
                let target = if maximized {
                    rect
                } else if w.snapped {
                    let x = rect.x()
                        + ((i64::from(w.outer.x() - old.x()) * i64::from(rect.width()))
                            / i64::from(old.width())) as i32;
                    let y = rect.y()
                        + ((i64::from(w.outer.y() - old.y()) * i64::from(rect.height()))
                            / i64::from(old.height())) as i32;
                    let right = rect.x()
                        + ((i64::from(w.outer.right() - old.x()) * i64::from(rect.width()))
                            / i64::from(old.width())) as i32;
                    let bottom = rect.y()
                        + ((i64::from(w.outer.bottom() - old.y()) * i64::from(rect.height()))
                            / i64::from(old.height())) as i32;
                    Rect::new(x, y, (right - x) as u32, (bottom - y) as u32).ok()?
                } else {
                    let x = w.outer.x().clamp(
                        rect.x() + self.scale.len(80) as i32 - w.outer.width() as i32,
                        rect.right() - self.scale.len(80) as i32,
                    );
                    let y = w
                        .outer
                        .y()
                        .clamp(rect.y(), rect.bottom() - self.scale.len(32) as i32);
                    Rect::new(x, y, w.outer.width(), w.outer.height()).ok()?
                };
                (target != w.outer).then_some((w.id, w.owner, target, w.snapped))
            })
            .collect();
        for (id, owner, target, _) in &plans {
            self.validate_outer(*target)?;
            self.owned(*owner, *id)?
                .epoch
                .checked_add(1)
                .ok_or(Error::Quota)?;
            self.room(*owner, plans.iter().filter(|p| p.1 == *owner).count())?;
        }
        for (_, saved) in &restore_plans {
            self.validate_outer(*saved)?;
        }
        for (id, owner, target, snapped) in plans {
            self.configure(owner, id, target)?;
            lookup_mut(&mut self.windows, id.0)?.snapped = snapped;
        }
        for (id, saved) in restore_plans {
            lookup_mut(&mut self.windows, id.0)?.restore = saved;
        }
        self.work_area = rect;
        self.invalidate();
        Ok(())
    }
    pub fn invalidate(&mut self) {
        self.move_reuse = None;
        self.damage = Some(self.output);
        self.damage_list = damage::DamageList::single(self.output);
    }
    /// Trusted shell damage uses the same clipped union as window mutations.
    pub fn invalidate_region(&mut self, region: Rect) {
        self.dirty_content(region);
    }
    pub fn windows(&self) -> impl Iterator<Item = WindowInfo<'_>> {
        self.z_order
            .iter()
            .filter_map(|id| lookup(&self.windows, id.0).ok())
            .map(|w| WindowInfo {
                workspace: w.workspace,
                snapped: w.snapped,
                id: w.id,
                owner: w.owner,
                role: w.role,
                outer: w.outer,
                content: w.content(),
                client_caption: w.caption.is_some(),
                caption_height: w.caption.map_or(0, |c| c.height),
                epoch: w.epoch,
                state: w.state,
                visible: self.visible(w.id),
                active: self.focused == Some(w.id),
                title: &w.title,
                cursor: w.cursor,
            })
    }
    pub fn window_info(&self, id: WindowId) -> Result<WindowInfo<'_>> {
        self.windows().find(|w| w.id == id).ok_or(Error::UnknownId)
    }
    pub fn snapshot(&self, id: WindowId) -> Option<SnapshotView<'_>> {
        lookup(&self.windows, id.0)
            .ok()?
            .snapshot
            .as_ref()
            .map(|s| SnapshotView {
                revision: s.revision,
                descriptor: s.descriptor,
                bytes: &s.bytes,
            })
    }
    pub(crate) fn window(&self, id: WindowId) -> Option<&Window> {
        lookup(&self.windows, id.0).ok()
    }
    pub(crate) fn visible(&self, id: WindowId) -> bool {
        let mut current = id;
        loop {
            let Some(w) = self.window(current) else {
                return false;
            };
            if matches!(w.state, WindowState::Minimized | WindowState::Closing) {
                return false;
            }
            if let Role::OwnedPopup(parent) = w.role {
                current = parent;
            } else {
                return w.workspace == self.workspace;
            }
        }
    }
    pub fn create_window(&mut self, owner: Owner, role: Role, outer: Rect) -> Result<WindowId> {
        self.create_with_caption(owner, role, outer, None)
    }
    pub fn create_client_caption(
        &mut self,
        owner: Owner,
        outer: Rect,
        caption: CaptionLayout,
    ) -> Result<WindowId> {
        caption.validate()?;
        self.create_with_caption(owner, Role::TopLevel, outer, Some(caption))
    }
    fn create_with_caption(
        &mut self,
        owner: Owner,
        role: Role,
        outer: Rect,
        caption: Option<CaptionLayout>,
    ) -> Result<WindowId> {
        self.validate_outer(outer)?;
        match role {
            Role::TopLevel => {}
            Role::OwnedPopup(parent) => {
                self.owned(owner, parent)?;
            }
            _ => return Err(Error::UnsupportedRole),
        }
        if self.z_order.len() >= self.limits.total_windows
            || self
                .windows
                .iter()
                .filter_map(|s| s.value.as_ref())
                .filter(|w| w.owner == owner)
                .count()
                >= self.limits.windows_per_owner
        {
            return Err(Error::Quota);
        }
        self.ensure_client(owner)?;
        self.room(owner, 1)?;
        let workspace = if let Role::OwnedPopup(parent) = role {
            self.window(parent).expect("validated parent").workspace
        } else {
            self.workspace
        };
        let id = WindowId(insert(
            &mut self.windows,
            Window {
                workspace,
                snapped: false,
                scale: self.scale,
                id: WindowId(0),
                owner,
                role,
                outer,
                restore: outer,
                state: WindowState::Normal,
                pre_minimize: WindowState::Normal,
                pre_close: WindowState::Normal,
                title: String::new(),
                cursor: CursorIcon::Arrow,
                epoch: 1,
                acknowledged: false,
                pending: None,
                snapshot: None,
                caption,
                material: Material::Default,
                frame_requested: false,
                frameless: false,
                caption_buttons: CAPTION_MINIMIZE | CAPTION_MAXIMIZE,
                fullscreen: false,
                pre_fullscreen: (outer, WindowState::Normal, false, false),
            },
        ));
        lookup_mut(&mut self.windows, id.0)?.id = id;
        self.z_order.push(id);
        self.dirty(outer);
        self.emit_configure(id);
        if role == Role::TopLevel {
            self.motion_created(id);
        }
        Ok(id)
    }
    /// Owner-selected material for translucent client pixels. Changing it
    /// recomposes the window; no client repaint or epoch change is required.
    pub fn set_material(&mut self, owner: Owner, id: WindowId, material: Material) -> Result<()> {
        let outer = self.owned(owner, id)?.outer;
        let window = lookup_mut(&mut self.windows, id.0)?;
        if window.material != material {
            window.material = material;
            self.dirty(outer);
        }
        Ok(())
    }
    pub fn material(&self, id: WindowId) -> Option<Material> {
        self.window(id).map(|w| w.material)
    }
    /// Trusted user preference (Settings > Accessibility > Transparency
    /// effects off): every translucent material composes over a solid color.
    pub fn set_reduce_transparency(&mut self, reduce: bool) {
        if self.reduce_transparency != reduce {
            self.reduce_transparency = reduce;
            self.invalidate();
        }
    }
    /// Trusted user preference (Settings > Personalization > Colors "Show
    /// accent colour on title bars and window borders"): the active window's
    /// trusted title bar and 1 px border paint with `accent` (opaque BGRA);
    /// inactive windows keep the neutral theme chrome. None turns it off.
    /// Window style (Doors extra): square corners instead of the Windows 11
    /// 8 px radius, for frames, clips and shadows alike.
    /// Doors extra: Mica behind every top-level window that chose no material.
    pub fn set_mica_everywhere(&mut self, on: bool) {
        if self.mica_everywhere != on {
            self.mica_everywhere = on;
            self.invalidate();
        }
    }
    pub fn set_square_corners(&mut self, square: bool) {
        let radius = if square {
            0
        } else {
            crate::render::WINDOW_RADIUS
        };
        if crate::render::window_radius() != radius {
            crate::render::set_window_radius(radius);
            self.invalidate();
        }
    }
    pub fn set_title_accent(&mut self, accent: Option<[u8; 4]>) {
        let accent = accent.map(|c| [c[0], c[1], c[2], 255]);
        if self.title_accent != accent {
            self.title_accent = accent;
            self.invalidate();
        }
    }
    pub fn title_accent(&self) -> Option<[u8; 4]> {
        self.title_accent
    }
    fn emit_configure(&mut self, id: WindowId) {
        let w = self.window(id).expect("live");
        let owner = w.owner;
        let outer = w.outer;
        let epoch = w.epoch;
        self.emit(
            owner,
            Event::Configure {
                window: id,
                epoch,
                outer,
                content_width: outer.width(),
                content_height: w.content().height(),
            },
        );
    }
    pub fn set_cursor(&mut self, owner: Owner, id: WindowId, icon: CursorIcon) -> Result<()> {
        self.owned(owner, id)?;
        // A client asks for the shapes `window::CURSOR_*` offers: the arrow,
        // the I-beam and the horizontal resize of an in-app splitter
        // (Explorer's navigation pane edge). The frame's other resize shapes
        // stay the server's own.
        if !matches!(
            icon,
            CursorIcon::Arrow | CursorIcon::Text | CursorIcon::EastWest
        ) {
            return Err(Error::InvalidState);
        }
        lookup_mut(&mut self.windows, id.0)?.cursor = icon;
        Ok(())
    }
    pub fn set_title(&mut self, owner: Owner, id: WindowId, title: &str) -> Result<()> {
        self.owned(owner, id)?;
        if title.len() > 128 || title.chars().any(|c| c.is_control()) {
            return Err(Error::InvalidState);
        }
        let outer = self.window(id).unwrap().outer;
        lookup_mut(&mut self.windows, id.0)?.title = title.to_string();
        self.dirty(outer);
        Ok(())
    }
    /// Which of Minimize and Maximize the caption offers
    /// (`CAPTION_MINIMIZE | CAPTION_MAXIMIZE`, a Win32 window's
    /// WS_MINIMIZEBOX/WS_MAXIMIZEBOX). With neither, only Close shows; with
    /// one, both show and the other is disabled, as Windows draws them.
    pub fn set_caption_buttons(&mut self, owner: Owner, id: WindowId, buttons: u8) -> Result<()> {
        if buttons & !(CAPTION_MINIMIZE | CAPTION_MAXIMIZE) != 0 {
            return Err(Error::InvalidState);
        }
        let w = self.owned(owner, id)?;
        if w.role != Role::TopLevel {
            return Err(Error::InvalidState);
        }
        let outer = w.outer;
        lookup_mut(&mut self.windows, id.0)?.caption_buttons = buttons;
        if self.caption_press.is_some_and(|(window, _)| window == id) {
            self.caption_press = None;
        }
        self.dirty(outer);
        Ok(())
    }
    /// A top-level window without trusted caption buttons, border or title
    /// bar (Win32 `WS_POPUP` chrome); its client answers its own hit test.
    pub fn set_frameless(&mut self, owner: Owner, id: WindowId, frameless: bool) -> Result<()> {
        let w = self.owned(owner, id)?;
        if w.role != Role::TopLevel {
            return Err(Error::InvalidState);
        }
        let outer = w.outer;
        lookup_mut(&mut self.windows, id.0)?.frameless = frameless;
        if self.caption_press.is_some_and(|(window, _)| window == id) {
            self.caption_press = None;
        }
        self.dirty(outer);
        Ok(())
    }
    pub fn configure(&mut self, owner: Owner, id: WindowId, outer: Rect) -> Result<u64> {
        self.validate_outer(outer)?;
        let w = self.owned(owner, id)?;
        let old = w.outer;
        let epoch = w.epoch.checked_add(1).ok_or(Error::Quota)?;
        // Coalesce superseded configures; clients cannot ack a superseded epoch.
        if let Some(i) = self.client_index(owner) {
            self.clients[i]
                .events
                .retain(|e| !matches!(e,Event::Configure{window,..} if *window==id));
        }
        self.room(owner, 1)?;
        self.cancel_attachment(id);
        let output = self.output;
        let w = lookup_mut(&mut self.windows, id.0)?;
        w.outer = outer;
        w.snapped = false;
        w.epoch = epoch;
        w.acknowledged = false;
        // Any other placement (snap, a client resize) ends full screen and
        // brings the frame back.
        if w.fullscreen && outer != output {
            w.fullscreen = false;
            w.frameless = w.pre_fullscreen.2;
        }
        self.dirty(old);
        self.dirty(outer);
        self.emit_configure(id);
        Ok(epoch)
    }
    /// Enter or leave full screen. Entering keeps the placement (normal
    /// rectangle, maximized or snapped) that leaving restores; minimizing
    /// and restoring keeps full screen, as Windows does.
    pub fn set_fullscreen(&mut self, owner: Owner, id: WindowId, on: bool) -> Result<()> {
        let w = self.owned(owner, id)?;
        if w.role != Role::TopLevel || matches!(w.state, WindowState::Closing) {
            return Err(Error::InvalidState);
        }
        if w.fullscreen == on {
            return Ok(());
        }
        let (outer, state, frameless, snapped) = (w.outer, w.state, w.frameless, w.snapped);
        if on {
            let prior = if state == WindowState::Minimized {
                w.pre_minimize
            } else {
                state
            };
            let output = self.output;
            self.configure(owner, id, output)?;
            let w = lookup_mut(&mut self.windows, id.0)?;
            if prior == WindowState::Normal && !snapped {
                w.restore = outer;
            }
            w.pre_fullscreen = (outer, prior, frameless, snapped);
            w.fullscreen = true;
            w.frameless = true;
            w.state = WindowState::Maximized;
        } else {
            let (saved, prior, was_frameless, was_snapped) = w.pre_fullscreen;
            let w = lookup_mut(&mut self.windows, id.0)?;
            w.fullscreen = false;
            w.frameless = was_frameless;
            let target = if prior == WindowState::Maximized {
                self.work_area
            } else {
                saved
            };
            self.configure(owner, id, target)?;
            let w = lookup_mut(&mut self.windows, id.0)?;
            w.state = prior;
            w.snapped = was_snapped && prior == WindowState::Normal;
        }
        self.dirty(outer);
        self.dirty_tree(id);
        Ok(())
    }
    /// The focused window covers the output in full screen (the taskbar
    /// stays hidden under it).
    pub fn fullscreen_focused(&self) -> bool {
        self.focused
            .and_then(|id| self.window(id))
            .is_some_and(|w| {
                w.fullscreen && w.state == WindowState::Maximized && self.visible(w.id)
            })
    }
    pub fn is_fullscreen(&self, id: WindowId) -> bool {
        self.window(id).is_some_and(|w| w.fullscreen)
    }
    pub fn acknowledge_configure(&mut self, owner: Owner, id: WindowId, epoch: u64) -> Result<()> {
        if self.owned(owner, id)?.epoch != epoch {
            return Err(Error::StaleEpoch);
        }
        lookup_mut(&mut self.windows, id.0)?.acknowledged = true;
        Ok(())
    }
    fn owner_bytes(&self, owner: Owner) -> usize {
        self.buffers
            .iter()
            .filter_map(|s| s.value.as_ref())
            .filter(|b| b.owner == owner)
            .map(|b| b.descriptor.backing_bytes)
            .sum::<usize>()
            + self
                .windows
                .iter()
                .filter_map(|s| s.value.as_ref())
                .filter(|w| w.owner == owner)
                .filter_map(|w| w.snapshot.as_ref())
                .map(|s| s.bytes.len())
                .sum::<usize>()
    }
    fn total_bytes(&self) -> usize {
        self.buffers
            .iter()
            .filter_map(|s| s.value.as_ref())
            .map(|b| b.descriptor.backing_bytes)
            .sum::<usize>()
            + self
                .windows
                .iter()
                .filter_map(|s| s.value.as_ref())
                .filter_map(|w| w.snapshot.as_ref())
                .map(|s| s.bytes.len())
                .sum::<usize>()
    }
    pub fn register_buffer(
        &mut self,
        owner: Owner,
        descriptor: SurfaceDescriptor,
    ) -> Result<BufferId> {
        descriptor.validate()?;
        if self.buffers.iter().filter(|s| s.value.is_some()).count() >= self.limits.total_buffers
            || self
                .buffers
                .iter()
                .filter_map(|s| s.value.as_ref())
                .filter(|b| b.owner == owner)
                .count()
                >= self.limits.buffers_per_owner
            || self
                .owner_bytes(owner)
                .saturating_add(descriptor.backing_bytes)
                > self.limits.bytes_per_owner
            || self.total_bytes().saturating_add(descriptor.backing_bytes) > self.limits.total_bytes
        {
            return Err(Error::Quota);
        }
        self.ensure_client(owner)?;
        Ok(BufferId(insert(
            &mut self.buffers,
            Buffer {
                owner,
                descriptor,
                attached: None,
            },
        )))
    }
    pub fn buffer_descriptor(&self, owner: Owner, id: BufferId) -> Result<SurfaceDescriptor> {
        Ok(self.buffer(owner, id)?.descriptor)
    }
    pub fn unregister_buffer(&mut self, owner: Owner, id: BufferId) -> Result<()> {
        if self.buffer(owner, id)?.attached.is_some() {
            return Err(Error::Busy);
        }
        self.buffers[parts(id.0)?.0].value = None;
        Ok(())
    }
    pub fn attach(
        &mut self,
        owner: Owner,
        id: WindowId,
        buffer: BufferId,
        epoch: u64,
    ) -> Result<()> {
        let w = self.owned(owner, id)?;
        if w.epoch != epoch {
            return Err(Error::StaleEpoch);
        }
        if !w.acknowledged {
            return Err(Error::NotAcknowledged);
        }
        if w.pending.is_some() {
            return Err(Error::Busy);
        }
        let b = self.buffer(owner, buffer)?;
        if b.attached.is_some() {
            return Err(Error::Busy);
        }
        if b.descriptor.width != w.content().width() || b.descriptor.height != w.content().height()
        {
            return Err(Error::InvalidSurface);
        }
        lookup_mut(&mut self.buffers, buffer.0)?.attached = Some(id);
        lookup_mut(&mut self.windows, id.0)?.pending = Some((buffer, epoch));
        Ok(())
    }
    /// Atomic publish of an owned capture from the registered backing section.
    /// On any error the prior snapshot and pending attachment remain intact.
    pub fn commit(&mut self, owner: Owner, id: WindowId, mut captured: Vec<u8>) -> Result<()> {
        self.commit_recycling(owner, id, &mut captured)
    }
    /// Publishes only after validation; returns the retired allocation to the caller.
    /// On error, both the supplied capture and prior snapshot retain ownership.
    pub fn commit_recycling(
        &mut self,
        owner: Owner,
        id: WindowId,
        captured: &mut Vec<u8>,
    ) -> Result<()> {
        let w = self.owned(owner, id)?;
        let (buffer, epoch) = w.pending.ok_or(Error::NoAttachment)?;
        if epoch != w.epoch {
            return Err(Error::StaleEpoch);
        }
        if !w.acknowledged {
            return Err(Error::NotAcknowledged);
        }
        let descriptor = self.buffer(owner, buffer)?.descriptor;
        let opaque = descriptor.validate_pixels(captured)?;
        let revision = w
            .snapshot
            .as_ref()
            .map_or(Some(1), |s| s.revision.checked_add(1))
            .ok_or(Error::Quota)?;
        let prior = w.snapshot.as_ref().map(|s| s.bytes.len()).unwrap_or(0);
        if self
            .owner_bytes(owner)
            .saturating_sub(prior)
            .saturating_add(captured.len())
            > self.limits.bytes_per_owner
            || self
                .total_bytes()
                .saturating_sub(prior)
                .saturating_add(captured.len())
                > self.limits.total_bytes
        {
            return Err(Error::Quota);
        }
        self.room(owner, 1)?;
        // A first frame or a new size changes what the window shows as a
        // whole (and with it whether its shadow is drawn): geometry damage.
        let whole = w
            .snapshot
            .as_ref()
            .is_none_or(|s| s.descriptor != descriptor);
        let damage = match w.snapshot.as_ref().filter(|s| s.descriptor == descriptor) {
            Some(old) => descriptor
                .changed_bounds(&old.bytes, captured)
                .and_then(|changed| {
                    let content = w.content();
                    Rect::new(
                        content.x() + changed.x(),
                        content.y() + changed.y(),
                        changed.width(),
                        changed.height(),
                    )
                    .ok()
                }),
            None => Some(w.outer),
        };
        let translucent = if opaque {
            alloc::vec![false; descriptor.height as usize]
        } else {
            descriptor.translucent_rows(captured)
        };
        let first = w.snapshot.is_none();
        let w = lookup_mut(&mut self.windows, id.0)?;
        let retired = w.snapshot.replace(Snapshot {
            revision,
            epoch,
            descriptor,
            bytes: core::mem::take(captured),
            opaque,
            source: buffer,
            translucent,
        });
        *captured = retired.map_or_else(Vec::new, |snapshot| snapshot.bytes);
        w.pending = None;
        lookup_mut(&mut self.buffers, buffer.0)?.attached = None;
        match damage {
            Some(damage) if whole => self.dirty(damage),
            Some(damage) => self.dirty_content(damage),
            None => (),
        }
        self.emit(owner, Event::BufferReleased { buffer });
        if first {
            self.motion_first_frame(id);
        }
        Ok(())
    }
    /// (row stride, height) when a band commit can patch this window's
    /// snapshot: the pending attachment uses the buffer and descriptor the
    /// snapshot came from. Otherwise the producer needs a full capture.
    pub fn band_geometry(&self, owner: Owner, id: WindowId) -> Option<(usize, u32)> {
        let w = self.owned(owner, id).ok()?;
        let (buffer, _) = w.pending?;
        let descriptor = self.buffer(owner, buffer).ok()?.descriptor;
        w.snapshot
            .as_ref()
            .filter(|s| s.source == buffer && s.descriptor == descriptor)
            .map(|_| (descriptor.stride_bytes, descriptor.height))
    }
    /// Band commit: rows `top..` of the trusted snapshot are replaced by the
    /// validated `band` (whole stride-sized rows) captured from the attached
    /// buffer. Rows outside the band keep bytes that were already validated,
    /// so no producer claim can place an unvalidated pixel in the snapshot;
    /// damage is the exact changed bounds inside the band.
    pub fn commit_band(&mut self, owner: Owner, id: WindowId, top: u32, band: &[u8]) -> Result<()> {
        let w = self.owned(owner, id)?;
        let (buffer, epoch) = w.pending.ok_or(Error::NoAttachment)?;
        if epoch != w.epoch {
            return Err(Error::StaleEpoch);
        }
        if !w.acknowledged {
            return Err(Error::NotAcknowledged);
        }
        if self.band_geometry(owner, id).is_none() {
            return Err(Error::InvalidState);
        }
        let descriptor = self.buffer(owner, buffer)?.descriptor;
        let band_opaque = descriptor.validate_band(top, band)?;
        let content = w.content();
        let stride = descriptor.stride_bytes;
        let start = top as usize * stride;
        let rows = band.len() / stride;
        let w = lookup_mut(&mut self.windows, id.0)?;
        let snapshot = w.snapshot.as_mut().ok_or(Error::InvalidState)?;
        let revision = snapshot.revision.checked_add(1).ok_or(Error::Quota)?;
        let band_descriptor = SurfaceDescriptor {
            height: rows as u32,
            backing_bytes: band.len(),
            ..descriptor
        };
        let changed =
            band_descriptor.changed_bounds(&snapshot.bytes[start..start + band.len()], band);
        snapshot.bytes[start..start + band.len()].copy_from_slice(band);
        for flag in &mut snapshot.translucent[top as usize..top as usize + rows] {
            *flag = !band_opaque;
        }
        snapshot.opaque = !snapshot.translucent.iter().any(|t| *t);
        snapshot.revision = revision;
        snapshot.epoch = epoch;
        w.pending = None;
        lookup_mut(&mut self.buffers, buffer.0)?.attached = None;
        if let Some(changed) = changed
            && let Ok(damage) = Rect::new(
                content.x() + changed.x(),
                content.y() + top as i32 + changed.y(),
                changed.width(),
                changed.height(),
            )
        {
            self.dirty_content(damage);
        }
        self.emit(owner, Event::BufferReleased { buffer });
        Ok(())
    }
    fn cancel_attachment(&mut self, id: WindowId) {
        if let Ok(w) = lookup_mut(&mut self.windows, id.0)
            && let Some((buffer, _)) = w.pending.take()
        {
            // Attach alone never transfers producer ownership; no release is due.
            if let Ok(b) = lookup_mut(&mut self.buffers, buffer.0) {
                b.attached = None;
            }
        }
    }
    pub fn request_frame(&mut self, owner: Owner, id: WindowId) -> Result<()> {
        let outer = self.owned(owner, id)?.outer;
        lookup_mut(&mut self.windows, id.0)?.frame_requested = true;
        if self.visible(id) {
            self.dirty(outer);
        }
        Ok(())
    }
    pub fn move_window(&mut self, owner: Owner, id: WindowId, x: i32, y: i32) -> Result<()> {
        let w = self.owned(owner, id)?;
        let old = w.outer;
        let new = Rect::new(x, y, old.width(), old.height())?;
        if old == new {
            return Ok(());
        }
        self.validate_outer(new)?;
        let reuse = self.plan_move_reuse(id, old, new);
        lookup_mut(&mut self.windows, id.0)?.outer = new;
        self.dirty(old);
        self.dirty(new);
        self.move_reuse = reuse;
        Ok(())
    }
    pub fn request_state(&mut self, owner: Owner, id: WindowId, state: WindowState) -> Result<()> {
        let w = self.owned(owner, id)?;
        let old = w.outer;
        let prior = w.state;
        let was_snapped = w.snapped;
        if state == WindowState::Closing {
            return self.close(owner, id);
        }
        if prior == state {
            return Ok(());
        }
        if matches!(w.role, Role::OwnedPopup(_)) && state == WindowState::Maximized {
            return Err(Error::InvalidState);
        }
        let was_maximized = prior == WindowState::Maximized
            || (prior == WindowState::Minimized && w.pre_minimize == WindowState::Maximized);
        let target = if state == WindowState::Maximized {
            if w.fullscreen {
                self.output
            } else {
                self.work_area
            }
        } else if state == WindowState::Normal && was_maximized {
            w.restore
        } else {
            old
        };
        if target != old {
            self.configure(owner, id, target)?;
        }
        let animated = self.window(id).is_some_and(|w| w.role == Role::TopLevel);
        if animated && state == WindowState::Minimized {
            self.motion_leave(id, true);
        }
        let w = lookup_mut(&mut self.windows, id.0)?;
        // Restore (not minimize) leaves full screen for the normal window.
        if state == WindowState::Normal && w.fullscreen {
            w.fullscreen = false;
            w.frameless = w.pre_fullscreen.2;
        }
        if state == WindowState::Maximized && !was_maximized && !was_snapped {
            w.restore = old;
        }
        if state == WindowState::Maximized || state == WindowState::Normal && was_maximized {
            w.snapped = false;
        }
        if state == WindowState::Minimized {
            w.pre_minimize = prior;
        }
        w.state = state;
        self.dirty(old);
        self.dirty(target);
        self.dirty_tree(id);
        if state == WindowState::Minimized {
            self.clear_interaction_tree(id);
        } else if animated && prior == WindowState::Minimized && self.visible(id) {
            self.motion_return(id, true);
        }
        Ok(())
    }
    /// Close request stops input immediately; owner then destroys the window.
    pub fn close(&mut self, owner: Owner, id: WindowId) -> Result<()> {
        let info = self.owned(owner, id)?;
        let outer = info.outer;
        let prior = info.state;
        self.room(owner, 1)?;
        // The visible image fades out from a retained ghost; the window
        // itself leaves input and the scene now.
        if self.window(id).is_some_and(|w| w.role == Role::TopLevel) {
            self.motion_leave(id, false);
        }
        let w = lookup_mut(&mut self.windows, id.0)?;
        w.pre_close = prior;
        w.state = WindowState::Closing;
        self.cancel_attachment(id);
        self.emit(owner, Event::CloseRequested { window: id });
        self.clear_interaction_tree(id);
        self.dirty(outer);
        self.dirty_tree(id);
        if self.focused.is_none() {
            let next = self.z_order.iter().rev().copied().find(|id| {
                self.window(*id).is_some_and(|w| {
                    w.workspace == self.workspace
                        && w.role == Role::TopLevel
                        && matches!(w.state, WindowState::Normal | WindowState::Maximized)
                })
            });
            if let Some(next) = next {
                let _ = self.activate(next);
            }
        }
        Ok(())
    }
    /// A responsive owner may reject a close (for example a failed durable save).
    /// Restore its prior non-client geometry and focus without discarding content.
    pub fn cancel_close(&mut self, owner: Owner, id: WindowId) -> Result<()> {
        let w = lookup(&self.windows, id.0)?;
        if w.owner != owner {
            return Err(Error::WrongOwner);
        }
        if w.state != WindowState::Closing {
            return Err(Error::InvalidState);
        }
        let outer = w.outer;
        let prior = w.pre_close;
        lookup_mut(&mut self.windows, id.0)?.state = prior;
        if prior != WindowState::Minimized && self.visible(id) {
            self.activate(id)?;
            // Interrupt the close transition: return from its current pose.
            if self.motion.ghosts.iter().any(|g| g.source == id) {
                self.motion_return(id, false);
            }
        } else if let Some(i) = self.motion.ghosts.iter().position(|g| g.source == id) {
            let ghost = self.motion.ghosts.remove(i);
            if let Some(drawn) = ghost.drawn {
                self.dirty(drawn);
            }
        }
        self.dirty(outer);
        Ok(())
    }
    pub fn snapshot_epoch(&self, id: WindowId) -> Option<u64> {
        self.window(id)
            .and_then(|window| window.snapshot.as_ref())
            .map(|snapshot| snapshot.epoch)
    }
    pub fn snapshot_capacity(&self, id: WindowId) -> usize {
        self.window(id)
            .and_then(|window| window.snapshot.as_ref())
            .map_or(0, |snapshot| snapshot.bytes.capacity())
    }
    pub fn destroy_window(&mut self, owner: Owner, id: WindowId) -> Result<()> {
        self.destroy_window_recycling(owner, id, &mut |_| {})
    }
    pub fn destroy_window_recycling(
        &mut self,
        owner: Owner,
        id: WindowId,
        recycle: &mut dyn FnMut(Vec<u8>),
    ) -> Result<()> {
        let w = lookup(&self.windows, id.0)?;
        if w.owner != owner {
            return Err(Error::WrongOwner);
        }
        let role = w.role;
        let had_focus = self.focused.is_some_and(|f| self.is_descendant(f, id));
        // Descendants are same-owner, established at creation, and cannot cycle.
        let children: Vec<_> = self
            .windows
            .iter()
            .filter_map(|s| s.value.as_ref())
            .filter(|w| w.role == Role::OwnedPopup(id))
            .map(|w| w.id)
            .collect();
        for child in children {
            self.destroy_window_recycling(owner, child, recycle)?;
        }
        let outer = self.window(id).unwrap().outer;
        self.cancel_attachment(id);
        self.clear_interaction_tree(id);
        if let Some(window) = self.windows[parts(id.0)?.0].value.take()
            && let Some(snapshot) = window.snapshot
        {
            recycle(snapshot.bytes);
        }
        self.z_order.retain(|w| *w != id);
        self.dirty(outer);
        self.motion_destroyed(id);
        if self.pointer_target == Some(id) {
            self.pointer_target = None;
        }
        if let Some(i) = self.client_index(owner) {
            self.clients[i]
                .events
                .retain(|e| !event_window(e).is_some_and(|w| w == id));
        }
        // As on Windows, destroying the active window (an application that
        // destroys its own window or exits without a close request) activates
        // the owner of an owned popup, else the next visible top-level window.
        if had_focus && self.focused.is_none() {
            let shown =
                |w: &Window| matches!(w.state, WindowState::Normal | WindowState::Maximized);
            let next = match role {
                Role::OwnedPopup(parent) if self.window(parent).is_some_and(shown) => Some(parent),
                _ => self.z_order.iter().rev().copied().find(|id| {
                    self.window(*id).is_some_and(|w| {
                        w.workspace == self.workspace && w.role == Role::TopLevel && shown(w)
                    })
                }),
            };
            if let Some(next) = next {
                let _ = self.activate(next);
            }
        }
        Ok(())
    }
    pub fn disconnect(&mut self, owner: Owner) {
        self.disconnect_recycling(owner, &mut |_| {})
    }
    pub fn disconnect_recycling(&mut self, owner: Owner, recycle: &mut dyn FnMut(Vec<u8>)) {
        let ids: Vec<_> = self
            .windows
            .iter()
            .filter_map(|s| s.value.as_ref())
            .filter(|w| w.owner == owner)
            .map(|w| w.id)
            .collect();
        for id in ids {
            let _ = self.destroy_window_recycling(owner, id, recycle);
        }
        for b in &mut self.buffers {
            if b.value.as_ref().is_some_and(|b| b.owner == owner) {
                b.value = None;
            }
        }
        self.clients.retain(|c| c.owner != owner);
    }
    fn is_descendant(&self, child: WindowId, parent: WindowId) -> bool {
        let mut current = Some(child);
        while let Some(id) = current {
            if id == parent {
                return true;
            }
            current = self.window(id).and_then(|w| {
                if let Role::OwnedPopup(p) = w.role {
                    Some(p)
                } else {
                    None
                }
            });
        }
        false
    }
    fn interaction_event(&mut self, id: WindowId, event: Event) {
        if let Some(owner) = self.window(id).map(|w| w.owner) {
            // Focus/capture state always stays authoritative. Coalesce state notifications.
            if let Some(i) = self.client_index(owner) {
                self.clients[i].events.retain(|e|!matches!((e,&event),(Event::Focus{window:a,..},Event::Focus{window:b,..}) | (Event::CaptureLost{window:a},Event::CaptureLost{window:b}) if a==b));
            }
            self.advisory(owner, event);
        }
    }
    fn clear_interaction_tree(&mut self, id: WindowId) {
        if self.focused.is_some_and(|f| self.is_descendant(f, id)) {
            let old = self.focused.take().unwrap();
            self.interaction_event(
                old,
                Event::Focus {
                    window: old,
                    focused: false,
                },
            );
        }
        if self.capture.is_some_and(|f| self.is_descendant(f, id)) {
            let old = self.capture.take().unwrap();
            self.interaction_event(old, Event::CaptureLost { window: old });
        }
    }
    /// Owner-checked activation preserves geometry and epoch. The Window server
    /// exposes it through negotiated STATE action 3 as well as trusted input.
    /// Client activation preserves maximized/snapped geometry and configure epoch.
    pub fn activate_owned(&mut self, owner: Owner, id: WindowId) -> Result<()> {
        let info = self.window_info(id)?;
        if info.owner != owner {
            return Err(Error::WrongOwner);
        }
        if info.state == WindowState::Closing {
            return Err(Error::Closing);
        }
        self.activate(id)
    }
    pub fn activate(&mut self, id: WindowId) -> Result<()> {
        if !self.visible(id) {
            return Err(Error::InvalidState);
        }
        // Clicking an already active window must not repaint the entire family
        // or enqueue another focus transition. New/topmost popup activation still
        // takes the ordinary stacking and capture path below.
        if self.focused == Some(id) && self.z_order.last() == Some(&id) {
            return Ok(());
        }
        if let Some(captured) = self.capture.filter(|captured| *captured != id) {
            self.capture = None;
            self.interaction_event(captured, Event::CaptureLost { window: captured });
        }
        let old = self.focused.replace(id);
        if let Some(old) = old.filter(|old| *old != id) {
            self.interaction_event(
                old,
                Event::Focus {
                    window: old,
                    focused: false,
                },
            );
            if let Some(w) = self.window(old) {
                self.dirty(w.outer);
            }
        }
        // Raise whole owner tree, preserving popups above their parent.
        let mut root = id;
        while let Some(w) = self.window(root) {
            if let Role::OwnedPopup(p) = w.role {
                root = p;
            } else {
                break;
            }
        }
        let raised: Vec<_> = self
            .z_order
            .iter()
            .copied()
            .filter(|w| self.is_descendant(*w, root))
            .collect();
        self.z_order.retain(|w| !raised.contains(w));
        self.z_order.extend(raised);
        self.interaction_event(
            id,
            Event::Focus {
                window: id,
                focused: true,
            },
        );
        self.dirty_tree(root);
        Ok(())
    }
    pub fn set_capture(&mut self, owner: Owner, id: WindowId) -> Result<()> {
        self.owned(owner, id)?;
        if self.focused != Some(id) || !self.visible(id) {
            return Err(Error::InvalidState);
        }
        if let Some(old) = self.capture.replace(id).filter(|old| *old != id) {
            self.interaction_event(old, Event::CaptureLost { window: old });
        }
        Ok(())
    }
    pub fn dismiss_transients(&mut self, id: WindowId) {
        self.interaction_event(id, Event::DismissTransients { window: id });
    }
    pub fn clear_focus(&mut self) {
        if let Some(id) = self.focused {
            self.clear_interaction_tree(id);
            self.dirty_tree(id);
        }
    }
    pub fn release_capture(&mut self, owner: Owner, id: WindowId) -> Result<()> {
        self.owned(owner, id)?;
        if self.capture != Some(id) {
            return Err(Error::InvalidState);
        }
        self.capture = None;
        // The explicit RPC acknowledgement is the release notification. A queued
        // CaptureLost here can arrive after the next real pointer-down and
        // incorrectly cancel a newer gesture in the same client.
        Ok(())
    }
    pub fn focused_window(&self) -> Option<WindowId> {
        self.focused
    }
    pub fn captured_window(&self) -> Option<WindowId> {
        self.capture
    }
    /// Restore the previous state of a minimized window, or unmaximize it.
    pub fn restore_window(&mut self, owner: Owner, id: WindowId) -> Result<()> {
        let w = self.owned(owner, id)?;
        let state = if w.state == WindowState::Minimized {
            w.pre_minimize
        } else {
            WindowState::Normal
        };
        self.request_state(owner, id, state)
    }
    /// Trusted keyboard navigation, including minimized top-level windows.
    /// Stable creation order keeps repeated cycling independent of raising order.
    pub fn cycle_focus(&mut self, reverse: bool) -> Result<Option<WindowId>> {
        let candidates: Vec<_> = self
            .windows
            .iter()
            .filter_map(|s| s.value.as_ref())
            .filter(|w| {
                w.workspace == self.workspace
                    && w.role == Role::TopLevel
                    && w.state != WindowState::Closing
            })
            .map(|w| w.id)
            .collect();
        if candidates.is_empty() {
            return Ok(None);
        }
        let current = self
            .focused
            .and_then(|id| candidates.iter().position(|w| self.is_descendant(id, *w)));
        let next = match current {
            Some(i) if reverse => (i + candidates.len() - 1) % candidates.len(),
            Some(i) => (i + 1) % candidates.len(),
            None if reverse => candidates.len() - 1,
            None => 0,
        };
        let id = candidates[next];
        let w = self.window(id).unwrap();
        if w.state == WindowState::Minimized {
            self.restore_window(w.owner, id)?;
        }
        self.activate(id)?;
        Ok(Some(id))
    }
    /// Resolves only a same-owner caption under the current trusted pointer.
    /// Foreign window IDs and protected caption buttons are never returned.
    pub fn tab_drop_target(
        &self,
        owner: Owner,
        source: WindowId,
        x: i32,
        y: i32,
    ) -> Result<Option<(WindowId, i32, i32)>> {
        if self.window_info(source)?.owner != owner {
            return Err(Error::WrongOwner);
        }
        if self.capture != Some(source) {
            return Err(Error::InvalidState);
        }
        let Some((id, HitRegion::Client | HitRegion::TitleBar)) = self.hit_test(x, y) else {
            return Ok(None);
        };
        let info = self.window_info(id)?;
        let scale = self.scale;
        let px = scale.logical(x - info.content.x());
        let py = scale.logical(y - info.content.y());
        if self.has_snapshot(id)
            && info.owner == owner
            && info.client_caption
            && info.state != WindowState::Closing
            && (0..info.caption_height as i32).contains(&py)
            && px >= 0
            && px < scale.logical_len(info.content.width()) as i32 - 144
        {
            Ok(Some((id, px, py)))
        } else {
            Ok(None)
        }
    }
    pub fn hit_test(&self, x: i32, y: i32) -> Option<(WindowId, HitRegion)> {
        self.z_order.iter().rev().copied().find_map(|id| {
            let w = self.window(id)?;
            if !self.visible(id) {
                return None;
            }
            // Windows 11's invisible resize border: a normal framed top-level
            // window also takes the 7 px band outside its frame (over
            // whatever lies behind it) for resizing.
            let outside = w.scale.len(CursorIcon::RESIZE_OUTSIDE) as i32;
            if !w.frameless
                && w.role == Role::TopLevel
                && w.state == WindowState::Normal
                && !rounded_contains(w.outer, w.scale.len(window_radius()), x, y)
                && x >= w.outer.x() - outside
                && x < w.outer.right() + outside
                && y >= w.outer.y() - outside
                && y < w.outer.bottom() + outside
            {
                return Some((id, HitRegion::Resize));
            }
            if !rounded_contains(
                w.outer,
                if w.state == WindowState::Maximized {
                    0
                } else {
                    w.scale.len(window_radius())
                },
                x,
                y,
            ) {
                return None;
            }
            let r = w.outer;
            let s = w.scale;
            // Physical edges snapped from logical offsets of the window edges
            // (the same edges paint uses), so regions never gap or overlap.
            let (dx, dy) = (x - r.x(), y - r.y());
            let (width, height) = (r.width() as i32, r.height() as i32);
            let from_right = |logical: u32| width - s.len(logical) as i32;
            let border = s.len(4) as i32;
            let region = if w.frameless {
                HitRegion::Client
            } else if w.state != WindowState::Maximized
                && (dx < border || dx >= width - border || dy < border || dy >= height - border)
            {
                HitRegion::Resize
            } else if dy < s.len(w.title_height()) as i32 {
                if dx >= from_right(46) {
                    HitRegion::Close
                } else if dx >= from_right(92) && w.caption_shows(HitRegion::Maximize) {
                    HitRegion::Maximize
                } else if dx >= from_right(138) && w.caption_shows(HitRegion::Minimize) {
                    HitRegion::Minimize
                } else if w.caption.is_some_and(|layout| {
                    dx < s.len(32) as i32
                        || layout
                            .control_scaled(r, s)
                            .is_some_and(|control| control.contains(x, y))
                }) {
                    HitRegion::Client
                } else {
                    HitRegion::TitleBar
                }
            } else {
                HitRegion::Client
            };
            Some((id, region))
        })
    }
    fn caption_visual_target(&self) -> Option<((WindowId, HitRegion), bool)> {
        let hover = self.caption_hover?;
        if !self.caption_left_down {
            Some((hover, false))
        } else if self.caption_press == Some(hover) {
            Some((hover, true))
        } else {
            None
        }
    }
    pub(crate) fn caption_visual(&self, id: WindowId, region: HitRegion) -> Option<bool> {
        // A disabled button neither lights up nor presses.
        if !self.window(id).is_some_and(|w| w.caption_enabled(region)) {
            return None;
        }
        self.caption_visual_target()
            .filter(|(target, _)| *target == (id, region))
            .map(|(_, pressed)| pressed)
    }
    /// Whether the caption button of window `id` acts (Close always; a
    /// greyed Minimize or Maximize does nothing).
    pub fn caption_button_enabled(&self, id: WindowId, region: HitRegion) -> bool {
        self.window(id).is_some_and(|w| w.caption_enabled(region))
    }
    pub(crate) fn caption_button_rect(window: &Window, region: HitRegion) -> Option<Rect> {
        Self::caption_button_rect_at(window.outer, window.scale, window.title_height(), region)
    }
    pub(crate) fn caption_button_rect_at(
        outer: Rect,
        scale: Scale,
        title_height: u32,
        region: HitRegion,
    ) -> Option<Rect> {
        let offset = match region {
            HitRegion::Close => 46,
            HitRegion::Maximize => 92,
            HitRegion::Minimize => 138,
            _ => return None,
        };
        Rect::new(
            outer.right() - scale.len(offset) as i32,
            outer.y(),
            scale.len(offset) - scale.len(offset - 46),
            scale.len(title_height),
        )
        .ok()
    }
    fn dirty_caption(&mut self, target: (WindowId, HitRegion)) {
        if let Some(rect) = self
            .window(target.0)
            .and_then(|window| Self::caption_button_rect(window, target.1))
            .and_then(|rect| rect.intersection(self.output))
        {
            // The caption-only path repaints over the retained scene with a
            // static material. A live acrylic window (the target or any
            // window whose shadow/corners reach the button) and animated
            // windows need their real composition, so use normal damage.
            if self.motion.active()
                || self.z_order.iter().any(|id| {
                    self.window(*id).is_some_and(|w| {
                        self.visible(*id)
                            && self.live_acrylic(w)
                            && crate::shadow::window_shadow_bounds(w.outer, w.scale)
                                .is_some_and(|reach| reach.intersection(rect).is_some())
                    })
                })
            {
                self.dirty(rect);
                return;
            }
            self.caption_damage = Some(
                self.caption_damage
                    .and_then(|old| old.union(rect))
                    .unwrap_or(rect),
            );
        }
    }
    /// Trusted nonclient pointer state. Coordinates are physical output pixels.
    /// A held press belongs to its original button; dragging away cancels its
    /// pressed appearance until re-entry. This queues only compositor-owned
    /// caption damage and never paints or mutates an in-flight composition.
    pub fn set_caption_pointer(&mut self, x: i32, y: i32, left_down: bool) -> bool {
        let before = self.caption_visual_target();
        let target = self.hit_test(x, y).filter(|(_, region)| {
            matches!(
                region,
                HitRegion::Close | HitRegion::Minimize | HitRegion::Maximize
            )
        });
        if left_down && !self.caption_left_down {
            self.caption_press = target;
        } else if !left_down {
            self.caption_press = None;
        }
        self.caption_pointer = Some((x, y));
        self.caption_hover = target;
        self.caption_left_down = left_down;
        let after = self.caption_visual_target();
        if before == after {
            return false;
        }
        if let Some((target, _)) = before {
            self.dirty_caption(target);
        }
        if let Some((target, _)) = after {
            self.dirty_caption(target);
        }
        true
    }
    fn reconcile_caption_pointer(&mut self) {
        if let Some((x, y)) = self.caption_pointer {
            self.set_caption_pointer(x, y, self.caption_left_down);
        }
    }
    /// Trusted device input. Pointer focus changes require a real button-down.
    /// Device coordinates are output-global. Queued pointer coordinates are
    /// content-local at receipt time, including negative captured coordinates.
    pub fn route_input(&mut self, input: Input, timestamp: u64) -> Result<Option<WindowId>> {
        let target = match input {
            Input::Key { .. } => self.focused,
            Input::PointerMove { x, y }
            | Input::PointerButton { x, y, .. }
            | Input::Wheel { x, y, .. } => {
                self.capture.or_else(|| self.hit_test(x, y).map(|(w, _)| w))
            }
        };
        if !matches!(input, Input::Key { .. }) {
            self.pointer_crossing(target, timestamp);
        }
        let Some(id) = target else {
            return Ok(None);
        };
        let owner = self.window(id).ok_or(Error::UnknownId)?.owner;
        if let Input::Wheel { x, y, delta } = input {
            return Ok(self.route_wheel(owner, id, x, y, delta, timestamp));
        }
        if let Input::PointerMove { .. } = input
            && let Some(i) = self.client_index(owner)
        {
            self.clients[i].events.retain(|e|!matches!(e,Event::Input{window,input:Input::PointerMove{..},..} if *window==id));
        }
        self.room(
            owner,
            if matches!(input, Input::PointerButton { down: true, .. }) {
                2
            } else {
                1
            },
        )?;
        if matches!(input, Input::PointerButton { down: true, .. }) {
            self.activate(id)?;
        }
        // Activation may consume queue slots; input is backpressured, never unbounded.
        self.room(owner, 1)?;
        let content = self.window(id).ok_or(Error::UnknownId)?.content();
        let input = match input {
            Input::PointerMove { x, y } => Input::PointerMove {
                x: x.saturating_sub(content.x()),
                y: y.saturating_sub(content.y()),
            },
            Input::PointerButton { button, down, x, y } => Input::PointerButton {
                button,
                down,
                x: x.saturating_sub(content.x()),
                y: y.saturating_sub(content.y()),
            },
            key => key,
        };
        self.emit(
            owner,
            Event::Input {
                window: id,
                input,
                timestamp,
            },
        );
        Ok(Some(id))
    }
    /// Trusted pointer left every window for shell UI (taskbar, Start,
    /// flyouts) or the output: the last pointer target receives its leave
    /// motion, exactly as when the pointer moves onto another window.
    pub fn pointer_leave(&mut self, timestamp: u64) {
        if self.capture.is_none() {
            self.pointer_crossing(None, timestamp);
        }
    }
    /// Pointer leave (WM_MOUSELEAVE): when the pointer's target window
    /// changes, the previous target receives one motion event at the
    /// client-local position (-1, -1), outside its client area. It never
    /// changes focus or activation and is never sent to the capture owner
    /// (whose pointer input keeps flowing) or to a destroyed/closing window.
    fn pointer_crossing(&mut self, target: Option<WindowId>, timestamp: u64) {
        let previous = core::mem::replace(&mut self.pointer_target, target);
        let Some(previous) = previous.filter(|p| Some(*p) != target) else {
            return;
        };
        if self.capture == Some(previous) {
            return;
        }
        let Some(owner) = self
            .window(previous)
            .filter(|w| w.state != WindowState::Closing)
            .map(|w| w.owner)
        else {
            return;
        };
        let Some(i) = self.client_index(owner) else {
            return;
        };
        // Supersedes that window's queued motion: at most one queued motion
        // per window, as for ordinary pointer moves.
        self.clients[i].events.retain(|e| {
            !matches!(e, Event::Input { window, input: Input::PointerMove { .. }, .. }
                if *window == previous)
        });
        if self.room(owner, 1).is_err() {
            self.clients[i].dropped_notifications =
                self.clients[i].dropped_notifications.saturating_add(1);
            return;
        }
        self.emit(
            owner,
            Event::Input {
                window: previous,
                input: Input::PointerMove { x: -1, y: -1 },
                timestamp,
            },
        );
    }
    /// Wheel input is advisory: it never activates, a burst merges into the
    /// owner's newest queued wheel event for the same window (keeping order
    /// with every other event), and a full queue drops it instead of revoking
    /// the owner the way lost key/button transitions must.
    fn route_wheel(
        &mut self,
        owner: Owner,
        id: WindowId,
        x: i32,
        y: i32,
        delta: i32,
        timestamp: u64,
    ) -> Option<WindowId> {
        let content = self.window(id)?.content();
        let (x, y) = (x.saturating_sub(content.x()), y.saturating_sub(content.y()));
        let delta = delta.clamp(-MAX_WHEEL_DELTA, MAX_WHEEL_DELTA);
        if delta == 0 {
            return None;
        }
        let i = self.client_index(owner)?;
        if let Some(Event::Input {
            window,
            input:
                Input::Wheel {
                    x: queued_x,
                    y: queued_y,
                    delta: queued,
                },
            timestamp: queued_time,
        }) = self.clients[i].events.back_mut()
            && *window == id
        {
            *queued = (*queued + delta).clamp(-MAX_WHEEL_DELTA, MAX_WHEEL_DELTA);
            *queued_x = x;
            *queued_y = y;
            *queued_time = timestamp;
            return Some(id);
        }
        if self.room(owner, 1).is_err() {
            self.clients[i].dropped_notifications =
                self.clients[i].dropped_notifications.saturating_add(1);
            return None;
        }
        self.emit(
            owner,
            Event::Input {
                window: id,
                input: Input::Wheel { x, y, delta },
                timestamp,
            },
        );
        Some(id)
    }
    pub fn set_theme(&mut self, theme: Theme) {
        if theme != self.theme {
            self.mica_material = self.wallpaper_material.as_ref().map(|w| w.mica(theme));
        }
        self.theme = theme;
        self.invalidate();
    }
    pub fn render(&mut self) -> Result<Frame<'_>> {
        self.render_with_chrome(|_, _| {})
    }
    pub fn output_view(&self) -> Result<OutputView<'_>> {
        if self.in_flight.is_some() {
            return Err(Error::Busy);
        }
        Ok(self.composed_view())
    }
    /// Read the immutable composition during this exact CPU transfer. Input may
    /// queue new scene damage; rendering/repainting remains blocked until ack.
    pub fn transfer_view(&self, serial: u64) -> Result<OutputView<'_>> {
        if self
            .in_flight
            .as_ref()
            .is_none_or(|flight| flight.0 != serial)
        {
            return Err(Error::UnknownFrame);
        }
        Ok(self.composed_view())
    }
    fn composed_view(&self) -> OutputView<'_> {
        OutputView {
            pixels: &self.pixels,
            width: self.output.width(),
            height: self.output.height(),
            stride_bytes: self.output.width() as usize * 4,
        }
    }
    /// Trusted overlay-only repaint over a completed private composition. The
    /// callback restores and paints exactly `region`; no client is resampled or
    /// receives a presentation notification for this compositor-owned update.
    pub fn repaint_overlay(
        &mut self,
        region: Rect,
        paint: impl FnOnce(&mut Canvas<'_>),
    ) -> Result<Frame<'_>> {
        if self.in_flight.is_some() || self.damage.is_some() {
            return Err(Error::Busy);
        }
        if region.intersection(self.output) != Some(region) {
            return Err(Error::InvalidGeometry);
        }
        let serial = self.next_frame;
        self.next_frame = serial.checked_add(1).ok_or(Error::Quota)?;
        paint(&mut Canvas::new(
            &mut self.pixels,
            self.output.width(),
            self.output.height(),
            self.output.width() as usize * 4,
        )?);
        self.in_flight = Some((serial, Vec::new()));
        Ok(Frame {
            pixels: &mut self.pixels,
            width: self.output.width(),
            height: self.output.height(),
            stride_bytes: self.output.width() as usize * 4,
            serial,
            damage: Some(region),
            regions: single_region(region),
            reused_pixels: 0,
        })
    }
    /// Repaint pending caption controls against the retained private scene.
    /// Client-caption underlays come from the last safe snapshot; ordinary
    /// captions use trusted theme chrome. Other windows and their shadows retain
    /// their stacking order. No producer is asked to repaint or notified.
    /// Trusted shell overlays are caller-owned and must be repainted afterwards
    /// when they overlap this damage, just as for `repaint_overlay`.
    pub fn has_caption_damage(&self) -> bool {
        self.caption_damage.is_some()
    }
    pub fn repaint_captions(&mut self) -> Result<Frame<'_>> {
        if self.in_flight.is_some() || self.damage.is_some() {
            return Err(Error::Busy);
        }
        self.reconcile_caption_pointer();
        let region = self.caption_damage.ok_or(Error::InvalidState)?;
        let serial = self.next_frame;
        self.next_frame = serial.checked_add(1).ok_or(Error::Quota)?;
        self.compose_caption_damage(region);
        self.caption_damage = None;
        self.in_flight = Some((serial, Vec::new()));
        Ok(Frame {
            pixels: &mut self.pixels,
            width: self.output.width(),
            height: self.output.height(),
            stride_bytes: self.output.width() as usize * 4,
            serial,
            damage: Some(region),
            regions: single_region(region),
            reused_pixels: 0,
        })
    }
    /// Paint trusted title text immediately after each window, before any
    /// higher window or shadow. The callback must clip text to title_clip;
    /// raw frame access remains trusted compositor access.
    pub fn render_with_chrome(
        &mut self,
        chrome: impl FnMut(WindowInfo<'_>, &mut Canvas<'_>),
    ) -> Result<Frame<'_>> {
        self.render_with_background(|_| {}, chrome)
    }
    /// Trusted desktop content is composed below every window and shadow.
    pub fn render_with_background(
        &mut self,
        background: impl FnMut(&mut Canvas<'_>),
        chrome: impl FnMut(WindowInfo<'_>, &mut Canvas<'_>),
    ) -> Result<Frame<'_>> {
        self.render_with_layers(background, chrome, |_| {})
    }
    pub fn render_with_layers(
        &mut self,
        background: impl FnMut(&mut Canvas<'_>),
        chrome: impl FnMut(WindowInfo<'_>, &mut Canvas<'_>),
        overlay: impl FnMut(&mut Canvas<'_>),
    ) -> Result<Frame<'_>> {
        self.render_layers(false, false, background, chrome, overlay)
    }
    /// Retain completed pixels outside pending damage. Trusted raw-pixel callbacks
    /// must honor Canvas::clip_rect; overlays explicitly restoring their retained
    /// underlay may paint additional regions and must transfer those regions too.
    pub fn render_damage_with_layers(
        &mut self,
        background: impl FnMut(&mut Canvas<'_>),
        chrome: impl FnMut(WindowInfo<'_>, &mut Canvas<'_>),
        overlay: impl FnMut(&mut Canvas<'_>),
    ) -> Result<Frame<'_>> {
        self.render_layers(true, false, background, chrome, overlay)
    }
    /// Opt in only when the preceding frame and these callbacks have no overlay
    /// inside normal windows. Background/chrome must be deterministic for unchanged
    /// state and honor the supplied clip. Title pixels are always repainted.
    pub fn render_damage_reusing_moves(
        &mut self,
        allow_reuse: bool,
        background: impl FnMut(&mut Canvas<'_>),
        chrome: impl FnMut(WindowInfo<'_>, &mut Canvas<'_>),
        overlay: impl FnMut(&mut Canvas<'_>),
    ) -> Result<Frame<'_>> {
        self.render_layers(true, allow_reuse, background, chrome, overlay)
    }
    fn render_layers(
        &mut self,
        partial: bool,
        allow_reuse: bool,
        mut background: impl FnMut(&mut Canvas<'_>),
        mut chrome: impl FnMut(WindowInfo<'_>, &mut Canvas<'_>),
        mut overlay: impl FnMut(&mut Canvas<'_>),
    ) -> Result<Frame<'_>> {
        if self.in_flight.is_some() {
            return Err(Error::Busy);
        }
        self.reconcile_caption_pointer();
        let serial = self.next_frame;
        self.next_frame = self.next_frame.checked_add(1).ok_or(Error::Quota)?;
        let visible: Vec<_> = self
            .z_order
            .iter()
            .copied()
            .filter(|id| self.visible(*id))
            .collect();
        let mut consumers = Vec::new();
        for id in &visible {
            let w = self.window(*id).unwrap();
            if w.snapshot.is_some() {
                consumers.push((w.owner, *id));
            }
        }
        let mut expanded = false;
        let mut regions = damage::DamageList::single(self.output);
        let region = if partial {
            let limit = if self.split_allowed {
                self.damage_limit
            } else {
                1
            };
            let mut list = match self.damage.filter(|_| limit > 1) {
                Some(_) => self.damage_list,
                None => damage::DamageList::single(self.damage.unwrap_or(self.output)),
            };
            if let Some(caption) = self.caption_damage {
                list.add(caption, limit);
            }
            // Acrylic reach and density alignment may make parts overlap;
            // rebuilding through `add` merges them again.
            let mut aligned = damage::DamageList::default();
            for r in list.iter() {
                let (r, grown) = self.expand_for_acrylic(r, &visible);
                expanded |= grown;
                // Font callbacks use logical clips. Outward density alignment keeps
                // their physical writes and the transferred damage exactly bounded.
                let s = self.scale;
                let left = s.px(s.logical(r.x()));
                let top = s.px(s.logical(r.y()));
                let right = s.px(s.logical(r.right() - 1) + 1);
                let bottom = s.px(s.logical(r.bottom() - 1) + 1);
                let r = Rect::new(left, top, (right - left) as u32, (bottom - top) as u32)?
                    .intersection(self.output)
                    .ok_or(Error::InvalidGeometry)?;
                aligned.add(r, limit);
            }
            regions = aligned;
            regions.bounds().ok_or(Error::InvalidGeometry)?
        } else {
            self.output
        };
        // Piecewise composition would let an acrylic capture or a transition
        // layer observe retained (translated) pixels; compose them whole.
        let acrylic = visible.iter().any(|id| {
            self.window(*id).is_some_and(|w| {
                self.live_acrylic(w)
                    && w.outer
                        .expanded(acrylic::margin(w.scale))
                        .is_some_and(|reach| reach.intersection(region).is_some())
            })
        });
        let reused = self.take_move_reuse(
            partial
                && allow_reuse
                && regions.count() == 1
                && !expanded
                && !acrylic
                && !self.motion.active(),
        );
        if regions.count() > 1 {
            for piece in regions.iter() {
                self.compose_with_chrome(
                    piece,
                    &visible,
                    &mut background,
                    &mut chrome,
                    &mut |_| {},
                );
            }
            // As with move reuse, the overlay observes the finished scene once.
            let mut canvas = Canvas::new(
                &mut self.pixels,
                self.output.width(),
                self.output.height(),
                self.output.width() as usize * 4,
            )?;
            canvas.set_clip(region);
            overlay(&mut canvas);
        } else if let Some(inner) = reused {
            for piece in move_reuse::subtract(region, inner).into_iter().flatten() {
                self.compose_with_chrome(
                    piece,
                    &visible,
                    &mut background,
                    &mut chrome,
                    &mut |_| {},
                );
            }
            // The overlay observes the finished scene exactly once, with the
            // original damage clip (rather than four intermediate underlays).
            let mut canvas = Canvas::new(
                &mut self.pixels,
                self.output.width(),
                self.output.height(),
                self.output.width() as usize * 4,
            )?;
            canvas.set_clip(region);
            overlay(&mut canvas);
        } else {
            self.compose_with_chrome(region, &visible, &mut background, &mut chrome, &mut overlay);
        }
        self.caption_damage = None;
        for id in visible {
            let w = self.window(id).unwrap();
            if w.frame_requested && self.room(w.owner, 1).is_ok() {
                let owner = w.owner;
                lookup_mut(&mut self.windows, id.0)?.frame_requested = false;
                self.emit(owner, Event::Frame { window: id, serial });
            }
        }
        self.in_flight = Some((serial, consumers));
        let damage = self.damage.take().map(|r| if partial { region } else { r });
        self.damage_list.clear();
        let mut composed = [None; MAX_DAMAGE_RECTS];
        if damage.is_some() {
            if partial {
                for (slot, r) in composed.iter_mut().zip(regions.iter()) {
                    *slot = Some(r);
                }
            } else {
                composed = single_region(self.output);
            }
        }
        Ok(Frame {
            pixels: &mut self.pixels,
            width: self.output.width(),
            height: self.output.height(),
            stride_bytes: self.output.width() as usize * 4,
            serial,
            damage,
            regions: composed,
            reused_pixels: reused.map_or(0, |r| u64::from(r.width()) * u64::from(r.height())),
        })
    }
    /// A live acrylic window samples the scene within `acrylic::margin` of
    /// its bounds. Damage reaching that area must recompose the window and
    /// the whole sampled area (so no retained pixel of the window itself or
    /// of a window above it is sampled). Repeats until stable.
    fn expand_for_acrylic(&self, mut region: Rect, visible: &[WindowId]) -> (Rect, bool) {
        let mut expanded = false;
        loop {
            let mut changed = false;
            for id in visible {
                let Some(w) = self.window(*id).filter(|w| self.live_acrylic(w)) else {
                    continue;
                };
                let Some(reach) = w
                    .outer
                    .expanded(acrylic::margin(w.scale))
                    .and_then(|reach| reach.intersection(self.output))
                else {
                    continue;
                };
                if region.intersection(reach).is_some() && region.intersection(reach) != Some(reach)
                {
                    region = region.union(reach).unwrap_or(self.output);
                    changed = true;
                }
            }
            if !changed {
                return (region, expanded);
            }
            expanded = true;
        }
    }
    /// Report actual CPU framebuffer transfer completion, never inferred vblank.
    /// Presentation notifications are bounded advisories: an owner that stops
    /// draining its queue cannot stall composition for other owners.
    pub fn complete_transfer(&mut self, serial: u64, success: bool, timestamp: u64) -> Result<()> {
        let (expected, _) = self.in_flight.as_ref().ok_or(Error::UnknownFrame)?;
        if *expected != serial {
            return Err(Error::UnknownFrame);
        }
        let (_, consumers) = self.in_flight.take().unwrap();
        if !success {
            self.invalidate();
        }
        for (owner, window) in consumers {
            if self.window(window).is_some_and(|w| w.owner == owner) {
                if let Some(i) = self.client_index(owner) {
                    self.clients[i]
                        .events
                        .retain(|e| !matches!(e,Event::Presentation{window:w,..} if *w==window));
                }
                self.advisory(
                    owner,
                    Event::Presentation {
                        window,
                        serial,
                        timestamp,
                        status: if success {
                            PresentationStatus::CpuTransferred
                        } else {
                            PresentationStatus::TransferFailed
                        },
                    },
                );
            }
        }
        Ok(())
    }
    pub fn resource_counts(&self) -> (usize, usize, usize) {
        (
            self.z_order.len(),
            self.buffers.iter().filter(|s| s.value.is_some()).count(),
            self.windows
                .iter()
                .filter_map(|s| s.value.as_ref())
                .filter_map(|w| w.snapshot.as_ref())
                .map(|s| s.bytes.len())
                .sum(),
        )
    }
}
fn single_region(region: Rect) -> [Option<Rect>; MAX_DAMAGE_RECTS] {
    let mut regions = [None; MAX_DAMAGE_RECTS];
    regions[0] = Some(region);
    regions
}
fn event_window(event: &Event) -> Option<WindowId> {
    match event {
        Event::Configure { window, .. }
        | Event::DismissTransients { window }
        | Event::Input { window, .. }
        | Event::Focus { window, .. }
        | Event::CaptureLost { window }
        | Event::CloseRequested { window }
        | Event::Frame { window, .. }
        | Event::Presentation { window, .. } => Some(*window),
        Event::BufferReleased { .. } => None,
    }
}
