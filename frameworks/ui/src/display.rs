//! Backend-neutral display lists (master specification section 51). Ops are
//! typed and refer to resources by validated ids, never pointers. Each op
//! records its window-space bounds at recording time (through the current
//! translation and clip stack), so a backend can replay only the ops that
//! touch the damaged region. The list is application-local; if it ever
//! crosses IPC it becomes an untrusted parser surface and needs quotas.
use crate::id::WidgetId;
use crate::theme::{Color, TextRole};
use crate::units::{LPoint, LRect, Lu};
use alloc::{collections::BTreeMap, string::String, vec::Vec};
use core::hash::{Hash, Hasher};
use core::ops::Range;

/// FNV-1a, for deterministic content fingerprints (no_std, no seeds).
pub struct Fnv(u64);
impl Default for Fnv {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}
impl Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}
pub fn fingerprint(bytes: &[u8]) -> u64 {
    let mut h = Fnv::default();
    h.write(bytes);
    h.finish()
}

/// Built-in vector glyphs (independently drawn, Segoe Fluent-like geometry
/// on a 16-DIP grid), so controls need no icon assets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    ChevronDown,
    ChevronUp,
    ChevronLeft,
    ChevronRight,
    Check,
    Dash,
    Close,
    More,
    Menu,
    Add,
    Remove,
    Search,
    Back,
    Forward,
    /// Filled status badges (InfoBar).
    Info,
    Warning,
    Error,
    Success,
    Home,
    Document,
    Folder,
    Star,
    Settings,
    Copy,
    Cut,
    Paste,
    Save,
}

/// Validated image resource id (see `Images`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageId(pub(crate) u32, pub(crate) u32);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    /// Translate subsequent ops (translation-only transform; the CPU backend
    /// reports no rotation/scale support, so the toolkit never emits it).
    PushTranslate(LPoint),
    PopTranslate,
    /// Intersect the clip with a rectangle (in the current translation).
    PushClip(LRect),
    PopClip,
    /// Multiply subsequent colours' opacity (a layer without offscreen
    /// compositing: overlapping children blend individually).
    PushOpacity(u8),
    PopOpacity,
    Rect {
        rect: LRect,
        color: Color,
    },
    RoundRect {
        rect: LRect,
        radius: Lu,
        color: Color,
    },
    /// Rounded outline lying inside `rect`.
    Border {
        rect: LRect,
        radius: Lu,
        width: Lu,
        color: Color,
    },
    Ellipse {
        rect: LRect,
        color: Color,
    },
    /// Stroked arc of the circle inscribed in `rect`: angles in degrees,
    /// clockwise from 12 o'clock; `sweep` 360 draws a full ring.
    Arc {
        rect: LRect,
        width: Lu,
        start: i32,
        sweep: i32,
        color: Color,
    },
    Line {
        from: LPoint,
        to: LPoint,
        width: Lu,
        color: Color,
    },
    Icon {
        icon: Icon,
        rect: LRect,
        color: Color,
    },
    /// One line of text whose line box starts at `origin`.
    Text {
        text: Range<u32>,
        origin: LPoint,
        role: TextRole,
        color: Color,
    },
    Image {
        image: ImageId,
        rect: LRect,
    },
    /// Soft elevation shadow around an opaque rounded surface.
    Shadow {
        rect: LRect,
        radius: Lu,
        depth: u8,
    },
}

/// Recorded ops plus their window-space bounds and owning widgets.
#[derive(Default, Clone)]
pub struct DisplayList {
    ops: Vec<Op>,
    bounds: Vec<Option<LRect>>,
    owners: Vec<Option<WidgetId>>,
    strings: String,
    translate: Vec<LPoint>,
    clips: Vec<Option<LRect>>,
    owner: Vec<WidgetId>,
}
impl DisplayList {
    pub fn clear(&mut self) {
        self.ops.clear();
        self.bounds.clear();
        self.owners.clear();
        self.strings.clear();
        self.translate.clear();
        self.clips.clear();
        self.owner.clear();
    }
    pub fn ops(&self) -> &[Op] {
        &self.ops
    }
    pub fn len(&self) -> usize {
        self.ops.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
    /// Window-space bounds of a drawing op (None for state ops and ops
    /// clipped away entirely).
    pub fn op_bounds(&self, index: usize) -> Option<LRect> {
        self.bounds.get(index).copied().flatten()
    }
    /// The widget that recorded op `index`.
    pub fn op_owner(&self, index: usize) -> Option<WidgetId> {
        self.owners.get(index).copied().flatten()
    }
    pub fn text(&self, range: &Range<u32>) -> &str {
        &self.strings[range.start as usize..range.end as usize]
    }
    fn offset(&self) -> LPoint {
        self.translate.last().copied().unwrap_or_default()
    }
    fn clipped(&self, r: LRect) -> Option<LRect> {
        let r = r.translate(self.offset());
        match self.clips.last() {
            None => Some(r),
            Some(None) => None,
            Some(Some(c)) => r.intersect(*c),
        }
    }
    /// Ops recorded until the matching `end` belong to `id` (nested owners
    /// attribute only their own ops, so a child's change never dirties the
    /// whole parent).
    pub(crate) fn begin(&mut self, id: WidgetId) {
        self.owner.push(id);
    }
    pub(crate) fn end(&mut self) {
        self.owner.pop();
    }
    fn record(&mut self, op: Op, bounds: Option<LRect>) {
        self.ops.push(op);
        self.bounds.push(bounds);
        self.owners.push(self.owner.last().copied());
    }
    pub fn push(&mut self, op: Op) {
        let bounds = match &op {
            Op::PushTranslate(d) => {
                let next = self.offset().offset(*d);
                self.translate.push(next);
                None
            }
            Op::PopTranslate => {
                self.translate.pop();
                None
            }
            Op::PushClip(r) => {
                let r = r.translate(self.offset());
                let next = match self.clips.last() {
                    None => Some(r),
                    Some(None) => None,
                    Some(Some(c)) => r.intersect(*c),
                };
                self.clips.push(next);
                None
            }
            Op::PopClip => {
                self.clips.pop();
                None
            }
            Op::PushOpacity(_) | Op::PopOpacity => None,
            Op::Rect { rect, .. }
            | Op::RoundRect { rect, .. }
            | Op::Border { rect, .. }
            | Op::Ellipse { rect, .. }
            | Op::Arc { rect, .. }
            | Op::Icon { rect, .. }
            | Op::Image { rect, .. } => self.clipped(*rect),
            Op::Shadow { rect, depth, .. } => {
                self.clipped(rect.inflate(Lu::px(i32::from(*depth) * 2 + 4)))
            }
            Op::Line {
                from, to, width, ..
            } => {
                let x = from.x.min(to.x);
                let y = from.y.min(to.y);
                let r =
                    LRect::new(x, y, from.x.max(to.x) - x, from.y.max(to.y) - y).inflate(*width);
                self.clipped(r)
            }
            Op::Text { .. } => None,
        };
        self.record(op, bounds);
    }
    /// Text with explicit bounds (the recorder knows the measured box).
    pub fn push_text(
        &mut self,
        text: &str,
        origin: LPoint,
        role: TextRole,
        color: Color,
        extent: LRect,
    ) {
        let start = self.strings.len() as u32;
        self.strings.push_str(text);
        let range = start..self.strings.len() as u32;
        let bounds = self.clipped(extent);
        self.record(
            Op::Text {
                text: range,
                origin,
                role,
                color,
            },
            bounds,
        );
    }
    /// Current clip in window space (`Some(None)`: nothing visible).
    pub fn current_clip(&self) -> Option<Option<LRect>> {
        self.clips.last().copied()
    }
    pub fn current_offset(&self) -> LPoint {
        self.offset()
    }
    fn hash_op(&self, i: usize, h: &mut Fnv) {
        match &self.ops[i] {
            Op::Text {
                text,
                origin,
                role,
                color,
            } => {
                self.text(text).hash(h);
                origin.hash(h);
                role.hash(h);
                color.hash(h);
            }
            op => op.hash(h),
        }
        self.bounds[i].hash(h);
    }
    /// Per-widget fingerprint and window-space bounds of its own ops, for
    /// exact damage: a widget whose ops or bounds changed damages its old
    /// and new bounds, independent of invalidation bookkeeping.
    pub fn summary(&self) -> BTreeMap<WidgetId, (u64, LRect)> {
        let mut out: BTreeMap<WidgetId, (Fnv, LRect)> = BTreeMap::new();
        for i in 0..self.ops.len() {
            let Some(id) = self.owners[i] else { continue };
            let e = out
                .entry(id)
                .or_insert_with(|| (Fnv::default(), LRect::ZERO));
            self.hash_op(i, &mut e.0);
            if let Some(b) = self.bounds[i] {
                e.1 = e.1.union(b);
            }
        }
        out.into_iter()
            .map(|(k, (h, b))| (k, (h.finish(), b)))
            .collect()
    }
    /// Ops recorded by `id` (inspector paint cost).
    pub fn ops_of(&self, id: WidgetId) -> usize {
        self.owners.iter().filter(|o| **o == Some(id)).count()
    }
    /// Fingerprint of the whole list (golden/trace identity).
    pub fn fingerprint(&self) -> u64 {
        let mut h = Fnv::default();
        for i in 0..self.ops.len() {
            self.hash_op(i, &mut h);
        }
        h.finish()
    }
}

/// One stored image: width, height and premultiplied BGRA8 pixels.
type Image = (u32, u32, Vec<u8>);
/// Bounded, validated image resources (premultiplied BGRA8). Ids carry a
/// generation, so a removed image's id never names a later image.
#[derive(Default)]
pub struct Images {
    images: Vec<(u32, Option<Image>)>,
    bytes: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageError {
    Size,
    NotPremultiplied,
    Quota,
}
impl Images {
    pub const MAX_SIDE: u32 = 4096;
    pub const MAX_BYTES: usize = 32 * 1024 * 1024;
    pub fn register(
        &mut self,
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    ) -> Result<ImageId, ImageError> {
        if width == 0 || height == 0 || width > Self::MAX_SIDE || height > Self::MAX_SIDE {
            return Err(ImageError::Size);
        }
        if pixels.len() != width as usize * height as usize * 4 {
            return Err(ImageError::Size);
        }
        if pixels
            .chunks_exact(4)
            .any(|p| p[0].max(p[1]).max(p[2]) > p[3])
        {
            return Err(ImageError::NotPremultiplied);
        }
        if self.bytes + pixels.len() > Self::MAX_BYTES {
            return Err(ImageError::Quota);
        }
        self.bytes += pixels.len();
        let entry = Some((width, height, pixels));
        let index = match self.images.iter().position(|e| e.1.is_none()) {
            Some(i) => {
                self.images[i].0 = self.images[i].0.wrapping_add(1);
                self.images[i].1 = entry;
                i
            }
            None => {
                self.images.push((0, entry));
                self.images.len() - 1
            }
        };
        Ok(ImageId(index as u32, self.images[index].0))
    }
    pub fn get(&self, id: ImageId) -> Option<(u32, u32, &[u8])> {
        self.images
            .get(id.0 as usize)
            .filter(|e| e.0 == id.1)
            .and_then(|e| e.1.as_ref())
            .map(|(w, h, p)| (*w, *h, p.as_slice()))
    }
    pub fn remove(&mut self, id: ImageId) {
        if let Some(e) = self.images.get_mut(id.0 as usize).filter(|e| e.0 == id.1)
            && let Some((_, _, p)) = e.1.take()
        {
            self.bytes -= p.len();
        }
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    pub fn count(&self) -> usize {
        self.images.iter().filter(|e| e.1.is_some()).count()
    }
}
