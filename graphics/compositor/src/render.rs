use crate::*;

pub const TITLE_HEIGHT: u32 = 32;
pub const WINDOW_RADIUS: u32 = 8;
/// The radius windows use now: `WINDOW_RADIUS`, or 0 for square corners
/// (`Engine::set_square_corners`).
static RADIUS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(WINDOW_RADIUS);
pub fn window_radius() -> u32 {
    RADIUS.load(core::sync::atomic::Ordering::Relaxed)
}
pub(crate) fn set_window_radius(radius: u32) {
    RADIUS.store(radius, core::sync::atomic::Ordering::Relaxed);
}
/// Extent of the elevation shadow of flyouts and menus (`Canvas::soft_shadow`);
/// top-level windows use the asymmetric `WINDOW_SHADOW`.
pub(crate) const ELEVATION_MARGIN: i32 = 12;
/// Exported shadow quadrants exist at 1x/2x/3x; fractional scales sample the
/// next larger one (a smooth Gaussian, so nearest texels suffice).
struct SoftShadow {
    bounds: Rect,
    /// Physical corner extent at this scale (32 DIPs).
    edge: usize,
    /// Texture corner extent and row length.
    source_edge: usize,
    side: usize,
    pixels: &'static [u8],
}
impl SoftShadow {
    fn new(opaque: Rect, scale: Scale) -> Option<Self> {
        let factor = scale.percent().div_ceil(100);
        let offset = match factor {
            1 => 0,
            2 => 1_089,
            3 => 5_314,
            _ => return None,
        };
        if opaque.width() < scale.len(40) + 1 || opaque.height() < scale.len(40) + 1 {
            return None;
        }
        let bounds = opaque.expanded(scale.len(ELEVATION_MARGIN as u32) as i32)?;
        let source_edge = 32 * factor as usize;
        let side = source_edge + 1;
        Some(Self {
            bounds,
            edge: scale.len(32) as usize,
            source_edge,
            side,
            pixels: &include_bytes!("../../../assets/raster/soft-shadow.a8")
                [offset..offset + side * side],
        })
    }
    /// Texture index of a physical distance from the shadow edge.
    fn texel(&self, distance: usize) -> usize {
        let d = distance.min(self.edge);
        if self.edge == self.source_edge {
            d
        } else {
            (d * self.source_edge + self.edge / 2) / self.edge
        }
    }
}
/// Provisional 96-DPI development tokens, from master section 140 bootstrap
/// and docs/design/windows11-reference.md. These do not claim measured fidelity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}
#[derive(Clone, Copy, Debug)]
pub struct ThemeTokens {
    pub desktop: [u8; 4],
    pub chrome: [u8; 4],
    pub inactive_chrome: [u8; 4],
    pub content: [u8; 4],
    pub border: [u8; 4],
    pub foreground: [u8; 4],
}
impl Theme {
    pub const fn tokens(self) -> ThemeTokens {
        match self {
            Self::Light => ThemeTokens {
                desktop: [218, 205, 191, 255],
                chrome: [248, 244, 239, 255],
                inactive_chrome: [248, 244, 239, 255],
                content: [255, 255, 255, 255],
                border: [180, 180, 180, 255],
                foreground: [32, 32, 32, 255],
            },
            Self::Dark => ThemeTokens {
                desktop: [43, 34, 27, 255],
                chrome: [40, 40, 40, 255],
                inactive_chrome: [48, 48, 48, 255],
                content: [32, 32, 32, 255],
                border: [87, 87, 87, 255],
                foreground: [240, 240, 240, 255],
            },
        }
    }
}
/// Integer source-over, accepting only premultiplied channel values.
pub fn blend_bgra(destination: [u8; 4], source: [u8; 4]) -> Result<[u8; 4]> {
    if destination[..3].iter().any(|c| *c > destination[3])
        || source[..3].iter().any(|c| *c > source[3])
    {
        return Err(Error::InvalidPixels);
    }
    Ok(blend(destination, source))
}
fn blend(d: [u8; 4], s: [u8; 4]) -> [u8; 4] {
    if s[3] == 255 {
        return s;
    }
    if s[3] == 0 {
        return d;
    }
    let inverse = 255 - s[3] as u32;
    core::array::from_fn(|i| (s[i] as u32 + (d[i] as u32 * inverse + 127) / 255).min(255) as u8)
}
fn with_coverage(color: [u8; 4], coverage: u8) -> [u8; 4] {
    color.map(|c| ((c as u32 * coverage as u32 + 127) / 255) as u8)
}
/// Text and glyph colour on an accent fill (opaque BGRA): black on light
/// accents, white on dark ones, by the Windows perceived-brightness rule
/// (2R + 5G + B) / 8 > 128.
pub fn accent_foreground(accent: [u8; 4]) -> [u8; 4] {
    let (b, g, r) = (accent[0] as u32, accent[1] as u32, accent[2] as u32);
    if (2 * r + 5 * g + b) / 8 > 128 {
        [0, 0, 0, 255]
    } else {
        [255; 4]
    }
}
fn sample_inside(rect: Rect, radius: u32, x16: i64, y16: i64) -> bool {
    let left = rect.x() as i64 * 16;
    let top = rect.y() as i64 * 16;
    let right = rect.right() as i64 * 16;
    let bottom = rect.bottom() as i64 * 16;
    if x16 < left || x16 >= right || y16 < top || y16 >= bottom {
        return false;
    }
    let radius = radius.min(rect.width() / 2).min(rect.height() / 2) as i64 * 16;
    if radius == 0 {
        return true;
    }
    let cx = x16.clamp(left + radius, right - radius);
    let cy = y16.clamp(top + radius, bottom - radius);
    // Valid physical rectangles can span i32::MAX pixels. Their coordinates in
    // sixteenths fit i64, but their squared distances need wider products.
    let dx = i128::from(x16 - cx);
    let dy = i128::from(y16 - cy);
    let radius = i128::from(radius);
    dx * dx + dy * dy <= radius * radius
}
pub(crate) fn rounded_contains(rect: Rect, radius: u32, x: i32, y: i32) -> bool {
    sample_inside(rect, radius, x as i64 * 16 + 8, y as i64 * 16 + 8)
}
pub(crate) fn rounded_coverage(rect: Rect, radius: u32, x: i32, y: i32) -> u8 {
    // Axis-aligned interior pixels are fully covered. Supersampling is needed
    // only at the four curved corners, rather than across every window pixel.
    if !rect.contains(x, y) {
        return 0;
    }
    let radius = radius.min(rect.width() / 2).min(rect.height() / 2) as i32;
    if radius == 0
        || (x >= rect.x() + radius && x < rect.right() - radius)
        || (y >= rect.y() + radius && y < rect.bottom() - radius)
    {
        return 255;
    }
    let radius = radius as u32;
    const COVERAGE: &[u8; 73_810] = include_bytes!("../../../assets/raster/rounded-coverage.a8");
    const AVATAR: &[u8; 129_024] = include_bytes!("../../../assets/raster/avatar-coverage.a8");
    let r = radius as usize;
    let (cache, offset): (&[u8], usize) = match radius {
        1..=60 => (COVERAGE, (r - 1) * r * (2 * r - 1) / 6),
        96 => (AVATAR, 0),
        192 => (AVATAR, 9_216),
        288 => (AVATAR, 46_080),
        _ => return rounded_coverage_sampled(rect, radius, x, y),
    };
    // Choose the cache after effective-radius clamping, then mirror into the
    // top-left quadrant. The sixteen offsets are symmetric about pixel centers.
    let local_x = (x - rect.x()).min(rect.right() - x - 1) as usize;
    let local_y = (y - rect.y()).min(rect.bottom() - y - 1) as usize;
    cache[offset + local_y * r + local_x]
}
fn rounded_coverage_sampled(rect: Rect, radius: u32, x: i32, y: i32) -> u8 {
    let mut count = 0;
    for dy in [2, 6, 10, 14] {
        for dx in [2, 6, 10, 14] {
            if sample_inside(rect, radius, x as i64 * 16 + dx, y as i64 * 16 + dy) {
                count += 1;
            }
        }
    }
    ((count * 255 + 8) / 16) as u8
}

/// Continuous one-DIP outline around the same antialiased outer contour.
/// Outer coverage is applied once by the caller; this weights inner fill and
/// border before that operation so curved corners are neither missing nor darkened.
fn contour_color(
    rect: Rect,
    radius: u32,
    line: u32,
    base: [u8; 4],
    border: [u8; 4],
    x: i32,
    y: i32,
) -> [u8; 4] {
    let outer = rounded_coverage(rect, radius, x, y);
    if outer == 0 {
        return base;
    }
    let inner = Rect::new(
        rect.x() + line as i32,
        rect.y() + line as i32,
        rect.width() - 2 * line,
        rect.height() - 2 * line,
    )
    .expect("validated window inset");
    let inside = rounded_coverage(inner, radius.saturating_sub(line), x, y).min(outer);
    if inside == outer {
        return base;
    }
    core::array::from_fn(|i| {
        ((u32::from(base[i]) * u32::from(inside)
            + u32::from(border[i]) * u32::from(outer - inside)
            + u32::from(outer) / 2)
            / u32::from(outer)) as u8
    })
}
/// Clipped software canvas on a private byte slice. Invalid destinations fail
/// construction; every primitive clips before calculating a byte index.
pub struct Canvas<'a> {
    pixels: &'a mut [u8],
    width: u32,
    height: u32,
    stride: usize,
    clip: Option<Rect>,
    scale: Scale,
}
impl<'a> Canvas<'a> {
    /// Logical extent (nearest logical size of the physical canvas).
    pub fn dimensions(&self) -> (u32, u32, usize) {
        (
            self.scale.logical_len(self.width),
            self.scale.logical_len(self.height),
            self.stride,
        )
    }
    pub fn physical_dimensions(&self) -> (u32, u32, usize) {
        (self.width, self.height, self.stride)
    }
    pub fn scale(&self) -> Scale {
        self.scale
    }
    /// Scale for the logical primitives (`fill_rect`, `fill_rounded_rect`,
    /// `blit_bgra`); physical primitives ignore it.
    pub fn set_scale(&mut self, scale: Scale) -> Result<()> {
        self.scale = scale;
        Ok(())
    }
    /// Physical rectangle of a logical one, snapped from its logical edges.
    pub fn scaled(&self, rect: Rect) -> Result<Rect> {
        self.scale.rect(rect)
    }
    /// Private compositor pixels for a trusted rasterizer. Its caller is
    /// responsible for title/content clipping and premultiplied output.
    pub fn pixels_mut(&mut self) -> &mut [u8] {
        self.pixels
    }
    pub(crate) fn pixels(&self) -> &[u8] {
        self.pixels
    }
    pub fn new(pixels: &'a mut [u8], width: u32, height: u32, stride: usize) -> Result<Self> {
        let descriptor = SurfaceDescriptor::new(width, height, stride, pixels.len())?;
        descriptor.validate()?;
        Ok(Self {
            pixels,
            scale: Scale::ONE,
            width,
            height,
            stride,
            clip: Some(Rect::new(0, 0, width, height)?),
        })
    }
    pub fn set_clip(&mut self, rect: Rect) {
        self.clip =
            rect.intersection(Rect::new(0, 0, self.width, self.height).expect("validated canvas"));
    }
    pub fn clip_rect(&self) -> Option<Rect> {
        self.clip
    }
    /// Restore a full-output private BGRA backdrop only within the physical clip.
    pub fn restore_bgra(&mut self, source: &[u8]) -> Result<()> {
        if source.len() != self.pixels.len() {
            return Err(Error::InvalidSurface);
        }
        if let Some(clip) = self.clip {
            let bytes = clip.width() as usize * 4;
            for y in clip.y() as usize..clip.bottom() as usize {
                let at = y * self.stride + clip.x() as usize * 4;
                self.pixels[at..at + bytes].copy_from_slice(&source[at..at + bytes]);
            }
        }
        Ok(())
    }
    pub fn pixel(&mut self, x: i32, y: i32, color: [u8; 4]) -> Result<()> {
        if color[..3].iter().any(|c| *c > color[3]) {
            return Err(Error::InvalidPixels);
        }
        self.put(x, y, color);
        Ok(())
    }
    fn put(&mut self, x: i32, y: i32, color: [u8; 4]) {
        if !self.clip.is_some_and(|clip| clip.contains(x, y)) {
            return;
        }
        let offset = y as usize * self.stride + x as usize * 4;
        let d = core::array::from_fn(|i| self.pixels[offset + i]);
        self.pixels[offset..offset + 4].copy_from_slice(&blend(d, color));
    }
    pub fn fill_rect(&mut self, rect: Rect, color: [u8; 4]) -> Result<()> {
        if color[..3].iter().any(|c| *c > color[3]) {
            return Err(Error::InvalidPixels);
        }
        let rect = self.scaled(rect)?;
        self.fill_physical(rect, color);
        Ok(())
    }
    fn fill_physical(&mut self, rect: Rect, color: [u8; 4]) {
        if color[3] == 0 {
            return;
        }
        if let Some(clip) = self.clip.and_then(|clip| rect.intersection(clip)) {
            if color[3] == 255 {
                // Fill one clipped scanline, then copy it. Large opaque cards
                // must not perform a blend, clip test and index check per pixel.
                let first = clip.y() as usize * self.stride + clip.x() as usize * 4;
                let bytes = clip.width() as usize * 4;
                self.pixels[first..first + 4].copy_from_slice(&color);
                let mut filled = 4;
                while filled < bytes {
                    let count = filled.min(bytes - filled);
                    self.pixels
                        .copy_within(first..first + count, first + filled);
                    filled += count;
                }
                for y in clip.y() + 1..clip.bottom() {
                    let dest = y as usize * self.stride + clip.x() as usize * 4;
                    self.pixels.copy_within(first..first + bytes, dest);
                }
                return;
            }
            // Geometry is already clipped. Blend only the active row slice,
            // without repeating put's clip/coordinate checks for every pixel.
            // Each destination remains distinct, including translucent pixels.
            let bytes = clip.width() as usize * 4;
            for y in clip.y() as usize..clip.bottom() as usize {
                let start = y * self.stride + clip.x() as usize * 4;
                for pixel in self.pixels[start..start + bytes].chunks_exact_mut(4) {
                    let destination = pixel[..].try_into().unwrap();
                    pixel.copy_from_slice(&blend(destination, color));
                }
            }
        }
    }
    pub fn blit_bgra(&mut self, rect: Rect, source: &[u8]) -> Result<()> {
        let rect = self.scaled(rect)?;
        self.blit_bgra_physical(rect, source)
    }
    /// Physical origin/extent for trusted chrome on windows moved by one pixel.
    pub fn blit_bgra_physical(&mut self, rect: Rect, source: &[u8]) -> Result<()> {
        if source.len() != rect.width() as usize * rect.height() as usize * 4 {
            return Err(Error::InvalidSurface);
        }
        if source
            .chunks_exact(4)
            .any(|p| p[..3].iter().any(|c| *c > p[3]))
        {
            return Err(Error::InvalidPixels);
        }
        if let Some(clip) = self.clip.and_then(|clip| rect.intersection(clip)) {
            for y in clip.y()..clip.bottom() {
                for x in clip.x()..clip.right() {
                    let at = ((y - rect.y()) as usize * rect.width() as usize
                        + (x - rect.x()) as usize)
                        * 4;
                    self.put(x, y, source[at..at + 4].try_into().unwrap());
                }
            }
        }
        Ok(())
    }
    /// Antialiased, clipped rounded control on the trusted private canvas.
    pub fn fill_rounded_rect(&mut self, rect: Rect, radius: u32, color: [u8; 4]) -> Result<()> {
        let rect = self.scaled(rect)?;
        let radius = self
            .scale
            .len(radius)
            .min(rect.width() / 2)
            .min(rect.height() / 2);
        if color[..3].iter().any(|c| *c > color[3]) {
            return Err(Error::InvalidPixels);
        }
        if let Some(clip) = self.clip.and_then(|clip| rect.intersection(clip)) {
            for y in clip.y()..clip.bottom() {
                let corners = y < rect.y() + radius as i32 || y >= rect.bottom() - radius as i32;
                let inset = if corners { radius as i32 } else { 0 };
                let left = (rect.x() + inset).clamp(clip.x(), clip.right());
                let right = (rect.right() - inset).clamp(left, clip.right());
                if right > left {
                    self.fill_physical(
                        Rect::new(left, y, (right - left) as u32, 1).unwrap(),
                        color,
                    );
                }
                for x in (clip.x()..left).chain(right..clip.right()) {
                    let coverage = rounded_coverage(rect, radius, x, y);
                    if coverage != 0 {
                        self.put(x, y, with_coverage(color, coverage));
                    }
                }
            }
        }
        Ok(())
    }
    /// An antialiased rounded outline `thickness` logical px wide (toggle
    /// switches, radio rings): outer coverage minus the inset shape's.
    pub fn stroke_rounded_rect(
        &mut self,
        rect: Rect,
        radius: u32,
        thickness: u32,
        color: [u8; 4],
    ) -> Result<()> {
        let rect = self.scaled(rect)?;
        let t = self.scale.len(thickness.max(1)).max(1);
        let radius = self
            .scale
            .len(radius)
            .min(rect.width() / 2)
            .min(rect.height() / 2);
        if color[..3].iter().any(|c| *c > color[3]) {
            return Err(Error::InvalidPixels);
        }
        let inner = (rect.width() > 2 * t && rect.height() > 2 * t).then(|| {
            Rect::new(
                rect.x() + t as i32,
                rect.y() + t as i32,
                rect.width() - 2 * t,
                rect.height() - 2 * t,
            )
            .unwrap()
        });
        let inner_radius = radius.saturating_sub(t);
        if let Some(clip) = self.clip.and_then(|clip| rect.intersection(clip)) {
            for y in clip.y()..clip.bottom() {
                for x in clip.x()..clip.right() {
                    let outer = rounded_coverage(rect, radius, x, y);
                    if outer == 0 {
                        continue;
                    }
                    let hole = inner.map_or(0, |i| {
                        if i.contains(x, y) {
                            rounded_coverage(i, inner_radius, x, y)
                        } else {
                            0
                        }
                    });
                    let coverage = outer.saturating_sub(hole);
                    if coverage != 0 {
                        self.put(x, y, with_coverage(color, coverage));
                    }
                }
            }
        }
        Ok(())
    }
    /// Opaque BGR image with antialiased corner coverage and canvas clipping.
    pub fn blit_rounded_bgr(&mut self, rect: Rect, radius: u32, pixels: &[u8]) -> Result<()> {
        if (rect.width() as usize)
            .checked_mul(rect.height() as usize)
            .and_then(|n| n.checked_mul(3))
            != Some(pixels.len())
        {
            return Err(Error::InvalidSurface);
        }
        if let Some(clip) = self.clip.and_then(|clip| rect.intersection(clip)) {
            for y in clip.y()..clip.bottom() {
                for x in clip.x()..clip.right() {
                    let at = ((y - rect.y()) as usize * rect.width() as usize
                        + (x - rect.x()) as usize)
                        * 3;
                    self.put(
                        x,
                        y,
                        with_coverage(
                            [pixels[at], pixels[at + 1], pixels[at + 2], 255],
                            rounded_coverage(rect, radius, x, y),
                        ),
                    );
                }
            }
        }
        Ok(())
    }
    /// Finite Gaussian coverage exported as a mirrored nine-slice quadrant.
    /// Small geometry and unsupported densities retain the prior safe path.
    pub fn soft_shadow(&mut self, opaque: Rect, scale: Scale, opacity: u8) -> bool {
        let Some(texture) = SoftShadow::new(opaque, scale) else {
            return false;
        };
        let shadow = texture.bounds;
        let Some(clip) = self.clip.and_then(|c| shadow.intersection(c)) else {
            return true;
        };
        let edge = texture.edge;
        let source_edge = texture.source_edge;
        let side = texture.side;
        let coverage = texture.pixels;
        let radius = scale.len(window_radius());
        for y in clip.y()..clip.bottom() {
            let py = (y - shadow.y()).min(shadow.bottom() - y - 1) as usize;
            let row = &coverage[texture.texel(py) * side..][..side];
            let skip = if (opaque.y()..opaque.bottom()).contains(&y) {
                let corners =
                    y < opaque.y() + radius as i32 || y >= opaque.bottom() - radius as i32;
                let inset = if corners { radius as i32 } else { 0 };
                (opaque.x() + inset, opaque.right() - inset)
            } else {
                (clip.right(), clip.right())
            };
            let left = skip.0.clamp(clip.x(), clip.right());
            let right = skip.1.clamp(left, clip.right());
            for (start, end) in [(clip.x(), left), (right, clip.right())] {
                let middle_start = (shadow.x() + edge as i32).clamp(start, end);
                let middle_end = (shadow.right() - edge as i32).clamp(middle_start, end);
                if middle_end > middle_start {
                    let alpha = ((row[source_edge] as u32 * opacity as u32 + 127) / 255) as u8;
                    self.fill_physical(
                        Rect::new(middle_start, y, (middle_end - middle_start) as u32, 1).unwrap(),
                        [0, 0, 0, alpha],
                    );
                }
                for x in (start..middle_start).chain(middle_end..end) {
                    if opaque.contains(x, y) && rounded_coverage(opaque, radius, x, y) == 255 {
                        continue;
                    }
                    let px = (x - shadow.x()).min(shadow.right() - x - 1) as usize;
                    let alpha =
                        ((row[texture.texel(px)] as u32 * opacity as u32 + 127) / 255) as u8;
                    if alpha != 0 {
                        let at = y as usize * self.stride + x as usize * 4;
                        let pixel = &mut self.pixels[at..at + 4];
                        let destination = pixel[..].try_into().unwrap();
                        pixel.copy_from_slice(&blend(destination, [0, 0, 0, alpha]));
                    }
                }
            }
        }
        true
    }
}
/// Trusted per-window chrome painter (title text), as passed to compose.
pub(crate) type ChromeCallback<'a> = dyn FnMut(WindowInfo<'_>, &mut Canvas<'_>) + 'a;
/// Material behind a window's translucent pixels, sampled in output space.
pub(crate) enum Base<'a> {
    Solid([u8; 4]),
    Wallpaper(&'a crate::engine::material::Backdrop),
    Live(&'a crate::engine::acrylic::LiveBackdrop),
}
impl Base<'_> {
    #[inline]
    pub(crate) fn pixel(&self, x: i32, y: i32) -> [u8; 4] {
        match self {
            Self::Solid(color) => *color,
            Self::Wallpaper(material) => material.pixel(x, y),
            Self::Live(backdrop) => backdrop.pixel(x, y),
        }
    }
    /// Prepare `cells` for sampling output row `y` with `RowBase::pixel`.
    fn row(&self, y: i32, cells: &mut [[u16; 3]]) -> Option<RowBase> {
        match self {
            Self::Solid(_) => None,
            Self::Wallpaper(material) => material.row(y, cells),
            Self::Live(backdrop) => backdrop.row(y, cells),
        }
    }
}
/// Grid cells one material row can hold (3840 px at the finest 8 px block,
/// plus the acrylic margin).
const ROW_CELLS: usize = 512;
/// One output row of a bilinear material grid: `cells[i]` holds the row's
/// vertical interpolation (block x value), so each sample is a horizontal
/// lerp. Bit-identical to the materials' per-pixel `pixel`.
pub(crate) struct RowBase {
    pub origin: i32,
    pub half: i32,
    pub block: usize,
    pub cells: usize,
}
impl RowBase {
    #[inline]
    fn pixel(&self, cells: &[[u16; 3]], x: i32) -> [u8; 4] {
        // Block sizes are 16 (wallpaper) or 8 x scale (acrylic); constant
        // blocks let the divisions compile to multiplies and shifts.
        match self.block {
            8 => self.lerp::<8>(cells, x),
            16 => self.lerp::<16>(cells, x),
            24 => self.lerp::<24>(cells, x),
            _ => {
                let row = |b: usize| {
                    let sx = (x - self.origin - self.half).max(0) as usize;
                    let x0 = (sx / b).min(self.cells - 1);
                    let x1 = (x0 + 1).min(self.cells - 1);
                    let fx = (sx % b) as u32;
                    let (w, a, c) = (b as u32, cells[x0], cells[x1]);
                    let lerp = |k: usize| {
                        ((u32::from(a[k]) * (w - fx) + u32::from(c[k]) * fx + w * w / 2) / (w * w))
                            as u8
                    };
                    [lerp(0), lerp(1), lerp(2), 255]
                };
                row(self.block)
            }
        }
    }
    #[inline(always)]
    fn lerp<const B: usize>(&self, cells: &[[u16; 3]], x: i32) -> [u8; 4] {
        let sx = (x - self.origin - self.half).max(0) as usize;
        let x0 = (sx / B).min(self.cells - 1);
        let x1 = (x0 + 1).min(self.cells - 1);
        let fx = (sx % B) as u32;
        let weight = B as u32;
        let (a, b) = (cells[x0], cells[x1]);
        let lerp = |c: usize| {
            ((u32::from(a[c]) * (weight - fx) + u32::from(b[c]) * fx + weight * weight / 2)
                / (weight * weight)) as u8
        };
        [lerp(0), lerp(1), lerp(2), 255]
    }
}
/// A window's geometry where it is painted: its resting place on the output,
/// or translated into a private animation layer. `offset` maps canvas
/// coordinates back to the output for material sampling.
#[derive(Clone, Copy)]
pub(crate) struct Placed<'a> {
    pub w: &'a Window,
    pub outer: Rect,
    pub content: Rect,
    pub offset: (i32, i32),
}
impl<'a> Placed<'a> {
    pub(crate) fn at(w: &'a Window) -> Self {
        Self {
            w,
            outer: w.outer,
            content: w.content(),
            offset: (0, 0),
        }
    }
    /// The window at output geometry `outer`/`content` (its own, or the size
    /// a stale snapshot was drawn for), translated so that output point
    /// `origin` is canvas (0, 0).
    pub(crate) fn with_geometry(
        w: &'a Window,
        outer: Rect,
        content: Rect,
        origin: (i32, i32),
    ) -> Option<Self> {
        let shift = |r: Rect| {
            Rect::new(
                r.x().checked_sub(origin.0)?,
                r.y().checked_sub(origin.1)?,
                r.width(),
                r.height(),
            )
            .ok()
        };
        Some(Self {
            w,
            outer: shift(outer)?,
            content: shift(content)?,
            offset: origin,
        })
    }
    pub(crate) fn radius(&self) -> u32 {
        if self.w.state == WindowState::Maximized {
            0
        } else {
            self.w.scale.len(window_radius())
        }
    }
}
/// Actual offline Segoe Fluent caption glyphs recommended by Microsoft. Native
/// density masks keep standard strokes consistent without per-pixel tessellation.
fn caption_glyph_coverage(p: Placed<'_>, region: HitRegion, x: i32, y: i32) -> u8 {
    let window = p.w;
    let scale = window.scale;
    let (offset, name) = match region {
        HitRegion::Close => (23, 0),
        HitRegion::Maximize if window.state == WindowState::Maximized => (69, 1),
        HitRegion::Maximize => (69, 2),
        HitRegion::Minimize => (115, 3),
        _ => return 0,
    };
    let size = scale.len(12) as usize;
    let left = p.outer.right() - scale.len(offset + 6) as i32;
    let top = p.outer.y() + scale.len((window.title_height() - 12) / 2) as i32;
    let (dx, dy) = (x - left, y - top);
    if dx < 0 || dy < 0 || dx as usize >= size || dy as usize >= size {
        return 0;
    }
    // Native masks exist at 1x/2x/3x; fractional scales area-filter the
    // next larger one.
    let factor = scale.percent().div_ceil(100);
    macro_rules! glyph {
        ($name:literal) => {
            if factor == 3 {
                &include_bytes!(concat!(
                    "../../../assets/icons/windows-shell/",
                    $name,
                    ".36.mask"
                ))[..]
            } else if factor == 2 {
                &include_bytes!(concat!(
                    "../../../assets/icons/windows-shell/",
                    $name,
                    ".24.mask"
                ))[..]
            } else {
                &include_bytes!(concat!(
                    "../../../assets/icons/windows-shell/",
                    $name,
                    ".12.mask"
                ))[..]
            }
        };
    }
    let mask = match name {
        0 => glyph!("chrome-close"),
        1 => glyph!("chrome-restore"),
        2 => glyph!("chrome-maximize"),
        _ => glyph!("chrome-minimize"),
    };
    crate::resample::area_sample(mask, 12 * factor as usize, size, dx as usize, dy as usize)
}
/// Every channel of a packed BGRA word times `a`/255, rounded to nearest:
/// two 16-bit lanes per multiply (exactly `(c * a + 127) / 255`, because
/// round(n / 255) = (t + (t >> 8)) >> 8 with t = n + 128 for n <= 255^2).
#[inline(always)]
fn scale_word(c: u32, a: u32) -> u32 {
    let rb = (c & 0x00ff_00ff) * a + 0x0080_0080;
    let rb = ((rb + ((rb >> 8) & 0x00ff_00ff)) >> 8) & 0x00ff_00ff;
    let ga = ((c >> 8) & 0x00ff_00ff) * a + 0x0080_0080;
    let ga = (ga + ((ga >> 8) & 0x00ff_00ff)) & 0xff00_ff00;
    rb | ga
}
/// Premultiplied source-over on packed little-endian BGRA words; identical
/// to `blend` for valid premultiplied input (no channel can exceed 255).
#[inline(always)]
fn blend_word(d: u32, s: u32) -> u32 {
    match s >> 24 {
        255 => s,
        0 => d,
        sa => s + scale_word(d, 255 - sa),
    }
}
/// Premultiplied word scaled by an opacity.
#[inline(always)]
fn fade_word(s: u32, alpha: u32) -> u32 {
    if alpha == 255 {
        s
    } else {
        scale_word(s, alpha)
    }
}
/// Lane-parallel linear interpolation of two packed words, `f` in 0..=256.
#[inline(always)]
fn lerp_word(a: u32, b: u32, f: u32) -> u32 {
    let g = 256 - f;
    let rb = ((a & 0x00ff_00ff) * g + (b & 0x00ff_00ff) * f + 0x0080_0080) >> 8;
    let ga = ((a >> 8) & 0x00ff_00ff) * g + ((b >> 8) & 0x00ff_00ff) * f + 0x0080_0080;
    (rb & 0x00ff_00ff) | (ga & 0xff00_ff00)
}
#[inline(always)]
fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}
/// Composite a premultiplied layer onto `canvas` so that its full extent maps
/// onto `dest` (bilinear when scaled), multiplied by `alpha`, within the clip.
pub(crate) fn draw_layer(
    canvas: &mut Canvas<'_>,
    layer: &crate::engine::animation::Layer,
    dest: Rect,
    alpha: u8,
) {
    if alpha == 0 {
        return;
    }
    let Some(clip) = canvas.clip.and_then(|clip| dest.intersection(clip)) else {
        return;
    };
    let alpha = u32::from(alpha);
    let (lw, lh) = (i64::from(layer.width), i64::from(layer.height));
    let (dw, dh) = (dest.width() as i64, dest.height() as i64);
    let stride = canvas.stride;
    let source = &layer.pixels;
    let row_bytes = lw as usize * 4;
    let span = clip.width() as usize * 4;
    if (lw, lh) == (dw, dh) {
        for y in clip.y()..clip.bottom() {
            let from = (y - dest.y()) as usize * row_bytes + (clip.x() - dest.x()) as usize * 4;
            let to = y as usize * stride + clip.x() as usize * 4;
            let src = &source[from..from + span];
            let dst = &mut canvas.pixels[to..to + span];
            for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                let s = fade_word(u32::from_le_bytes(s.try_into().unwrap()), alpha);
                if s != 0 {
                    let out = blend_word(u32::from_le_bytes((&*d).try_into().unwrap()), s);
                    d.copy_from_slice(&out.to_le_bytes());
                }
            }
        }
        return;
    }
    // Source coordinate of each destination pixel center, in 1/256 texels.
    let map = |d: i64, dsize: i64, ssize: i64| -> (usize, usize, u32) {
        let s = ((2 * d + 1) * ssize * 256 / (2 * dsize) - 128).clamp(0, (ssize - 1) * 256);
        let i = (s >> 8) as usize;
        (i * 4, (i + 1).min(ssize as usize - 1) * 4, (s & 255) as u32)
    };
    // Columns repeat on every row: map them once (bounded by the clip).
    let mut columns = alloc::vec::Vec::new();
    if columns.try_reserve_exact(clip.width() as usize).is_err() {
        return;
    }
    columns.extend((clip.x()..clip.right()).map(|x| map(i64::from(x - dest.x()), dw, lw)));
    // Each row first blends its two source rows over the used texel range,
    // then every destination pixel needs one horizontal lerp.
    let first = columns.first().map_or(0, |c| c.0 / 4);
    let last = columns.last().map_or(0, |c| c.1 / 4);
    let mut vertical = alloc::vec::Vec::new();
    if vertical.try_reserve_exact(last + 1 - first).is_err() {
        return;
    }
    vertical.resize(last + 1 - first, 0u32);
    // Texel indices relative to `first`, packed small for the inner loop.
    for column in columns.iter_mut() {
        *column = (column.0 / 4 - first, column.1 / 4 - first, column.2);
    }
    for y in clip.y()..clip.bottom() {
        let (y0, y1, fy) = map(i64::from(y - dest.y()), dh, lh);
        let top = &source[y0 / 4 * row_bytes..][..row_bytes];
        let bottom = &source[y1 / 4 * row_bytes..][..row_bytes];
        for (i, v) in vertical.iter_mut().enumerate() {
            let at = (first + i) * 4;
            *v = lerp_word(word(top, at), word(bottom, at), fy);
        }
        let to = y as usize * stride + clip.x() as usize * 4;
        let dst = &mut canvas.pixels[to..to + span];
        for (d, &(x0, x1, fx)) in dst.chunks_exact_mut(4).zip(columns.iter()) {
            let (a, b) = (vertical[x0], vertical[x1]);
            if a | b == 0 {
                continue;
            }
            // Rounded lerps keep color <= alpha in every lane.
            let s = fade_word(lerp_word(a, b, fx), alpha);
            if s != 0 {
                let out = blend_word(u32::from_le_bytes((&*d).try_into().unwrap()), s);
                d.copy_from_slice(&out.to_le_bytes());
            }
        }
    }
}

impl Engine {
    /// The cached shadow mask of `window` drawn with outer rectangle `outer`
    /// (its own, or a layer's stale geometry), in its focus state.
    pub(crate) fn window_shadow_mask(
        &self,
        window: &Window,
        outer: Rect,
    ) -> Option<alloc::rc::Rc<crate::shadow::ShadowMask>> {
        self.shadows
            .borrow_mut()
            .get(outer, window.scale, self.focused == Some(window.id))
    }
    /// The window's shadow around `outer`; maximized windows (`radius` 0)
    /// have none.
    pub(crate) fn paint_window_shadow(
        &self,
        canvas: &mut Canvas<'_>,
        window: &Window,
        outer: Rect,
        radius: u32,
    ) {
        if radius == 0 {
            return;
        }
        if let Some(mask) = self.window_shadow_mask(window, outer) {
            canvas.window_shadow(outer, radius, &mask);
        }
    }
    /// The colour composition shows at output pixel (`x`, `y`) for `color`
    /// lying beneath the shadows of every visible window stacked above
    /// `below` (every window when None), where no window body covers the
    /// pixel: each shadow darkens it in stacking order, exactly as
    /// composition does. Host tooling derives guest probe colours with it.
    pub fn shadowed(&self, x: i32, y: i32, color: [u8; 4], below: Option<WindowId>) -> [u8; 4] {
        let start = below
            .and_then(|id| self.z_order.iter().position(|w| *w == id))
            .map_or(0, |i| i + 1);
        let mut color = color;
        for id in &self.z_order[start..] {
            let Some(w) = self.window(*id).filter(|_| self.visible(*id)) else {
                continue;
            };
            if Placed::at(w).radius() == 0 {
                continue;
            }
            if let Some(mask) = self.window_shadow_mask(w, w.outer)
                && let Some(bounds) = mask.bounds(w.outer)
            {
                color = blend(color, [0, 0, 0, mask.alpha(bounds, x, y)]);
            }
        }
        color
    }
    /// Solid material used when the user reduced transparency (Windows 11
    /// solid Mica fallback colors).
    pub(crate) fn transparency_fallback(&self) -> [u8; 4] {
        match self.theme {
            Theme::Light => [243, 243, 243, 255],
            Theme::Dark => [32, 32, 32, 255],
        }
    }
    /// The accent the active window's trusted title bar and 1 px border
    /// paint with (Settings > Personalization > Colors "Show accent colour
    /// on title bars and window borders"); None when that is off or `id` is
    /// not the active window (inactive windows stay neutral, as on Windows).
    /// The material a window paints with: its own, or Mica for a top-level
    /// window without one while Mica is on everywhere.
    pub(crate) fn material_of(&self, window: &Window) -> Material {
        if window.material == Material::Default
            && self.mica_everywhere
            && window.role == Role::TopLevel
            && window.caption.is_none_or(|c| !c.tabbed)
        {
            Material::Mica
        } else {
            window.material
        }
    }
    pub(crate) fn accent_title(&self, id: WindowId) -> Option<[u8; 4]> {
        self.title_accent.filter(|_| self.focused == Some(id))
    }
    /// Title bar fill of window `id`.
    pub(crate) fn chrome_of(&self, id: WindowId) -> [u8; 4] {
        let tokens = self.theme.tokens();
        match self.accent_title(id) {
            Some(accent) => accent,
            None if self.focused == Some(id) => tokens.chrome,
            None => tokens.inactive_chrome,
        }
    }
    /// 1 px window border of window `id`.
    pub(crate) fn border_of(&self, id: WindowId) -> [u8; 4] {
        self.accent_title(id).unwrap_or(self.theme.tokens().border)
    }
    /// Material behind a window when no live backdrop was captured for it.
    pub(crate) fn static_base(&self, window: &Window) -> Base<'_> {
        if self.material_of(window) == Material::Mica {
            return match &self.mica_material {
                Some(mica) if !self.reduce_transparency && self.focused == Some(window.id) => {
                    Base::Wallpaper(mica)
                }
                _ => Base::Solid(self.transparency_fallback()),
            };
        }
        let material =
            window.caption.is_some_and(|c| c.tabbed) || window.material == Material::Acrylic;
        if material && self.reduce_transparency {
            return Base::Solid(self.transparency_fallback());
        }
        if material && let Some(wallpaper) = &self.wallpaper_material {
            return Base::Wallpaper(wallpaper);
        }
        Base::Solid(self.theme.tokens().content)
    }
    /// Acrylic window whose current snapshot has translucent pixels: its
    /// material is the scene composed beneath it in the same frame.
    pub(crate) fn live_acrylic(&self, window: &Window) -> bool {
        window.material == Material::Acrylic
            && !self.reduce_transparency
            && window.snapshot.as_ref().is_some_and(|s| !s.opaque)
    }
    fn window_base_pixel(&self, p: Placed<'_>, base: &Base<'_>, x: i32, y: i32) -> [u8; 4] {
        let window = p.w;
        let mut color = self.chrome_of(window.id);
        if p.content.contains(x, y) {
            color = base.pixel(x + p.offset.0, y + p.offset.1);
            if let Some(snapshot) = &window.snapshot {
                let sx = (x - p.content.x()) as usize;
                let sy = (y - p.content.y()) as usize;
                if sx < snapshot.descriptor.width as usize
                    && sy < snapshot.descriptor.height as usize
                {
                    let at = sy * snapshot.descriptor.stride_bytes + sx * 4;
                    color = blend(color, snapshot.bytes[at..at + 4].try_into().unwrap());
                }
            }
        } else if self.material_of(window) == Material::Mica
            && self.accent_title(window.id).is_none()
        {
            color = base.pixel(x + p.offset.0, y + p.offset.1);
        }
        color
    }
    fn caption_pixel(
        &self,
        p: Placed<'_>,
        base: &Base<'_>,
        region: HitRegion,
        x: i32,
        y: i32,
    ) -> [u8; 4] {
        let tokens = self.theme.tokens();
        let window = p.w;
        let visual = self.caption_visual(window.id, region);
        let mut color = self.window_base_pixel(p, base, x, y);
        let accent = self.accent_title(window.id);
        if let Some(pressed) = visual
            && x > p.outer.x()
            && x < p.outer.right() - 1
            && y > p.outer.y()
            && y < p.outer.bottom() - 1
        {
            color = match (region, pressed, self.theme) {
                (HitRegion::Close, false, _) => [35, 35, 232, 255],
                (HitRegion::Close, true, _) => [45, 45, 197, 255],
                (_, false, Theme::Light) => [225, 225, 225, 255],
                (_, true, Theme::Light) => [210, 210, 210, 255],
                (_, false, Theme::Dark) => [61, 61, 61, 255],
                (_, true, Theme::Dark) => [50, 50, 50, 255],
            };
            // Over an accent title bar the plate is a veil of the glyph
            // colour, as Windows draws it.
            if region != HitRegion::Close
                && let Some(accent) = accent
            {
                let veil = if pressed { 26 } else { 40 };
                color = blend(accent, with_coverage(accent_foreground(accent), veil));
            }
        }
        let foreground = if region == HitRegion::Close && visual.is_some() {
            [255; 4]
        } else {
            accent.map_or(tokens.foreground, accent_foreground)
        };
        let mut coverage = caption_glyph_coverage(p, region, x, y);
        if !window.caption_enabled(region) {
            // A disabled button's glyph is greyed (about 36% of the ink).
            coverage = (u16::from(coverage) * 92 / 255) as u8;
        }
        let color = blend(color, with_coverage(foreground, coverage));
        contour_color(
            p.outer,
            p.radius(),
            window.scale.len(1),
            color,
            self.border_of(window.id),
            x,
            y,
        )
    }
    fn paint_caption_buttons(&self, p: Placed<'_>, base: &Base<'_>, canvas: &mut Canvas<'_>) {
        let window = p.w;
        if window.frameless {
            return;
        }
        let radius = p.radius();
        let (width, height, _) = canvas.physical_dimensions();
        let bounds = Rect::new(0, 0, width, height).expect("validated canvas");
        for region in [HitRegion::Minimize, HitRegion::Maximize, HitRegion::Close] {
            if !window.caption_shows(region) {
                continue;
            }
            // The body already contains the unchanged caption background and
            // contour. Without hover/press, only the 12-DIP glyph needs work.
            let button = if self.caption_visual(window.id, region).is_none() {
                let offset = match region {
                    HitRegion::Close => 23,
                    HitRegion::Maximize => 69,
                    _ => 115,
                };
                Rect::new(
                    p.outer.right() - window.scale.len(offset + 6) as i32,
                    p.outer.y() + window.scale.len((window.title_height() - 12) / 2) as i32,
                    window.scale.len(12),
                    window.scale.len(12),
                )
                .ok()
            } else {
                Self::caption_button_rect_at(p.outer, window.scale, window.title_height(), region)
            };
            let Some(rect) = button
                .and_then(|rect| rect.intersection(bounds))
                .and_then(|rect| canvas.clip_rect().and_then(|clip| rect.intersection(clip)))
            else {
                continue;
            };
            for y in rect.y()..rect.bottom() {
                for x in rect.x()..rect.right() {
                    // Curved outside pixels retain the already composed exact
                    // shadow/background coverage; hover never fills those corners.
                    if rounded_coverage(p.outer, radius, x, y) == 255 {
                        canvas.put(x, y, self.caption_pixel(p, base, region, x, y));
                    }
                }
            }
        }
    }
    pub(crate) fn compose_caption_damage(&mut self, damage: Rect) {
        let mut pixels = core::mem::take(&mut self.pixels);
        for (index, id) in self.z_order.iter().enumerate() {
            let Some(window) = self
                .window(*id)
                .filter(|w| self.visible(*id) && !w.frameless)
            else {
                continue;
            };
            let placed = Placed::at(window);
            let base = self.static_base(window);
            let radius = placed.radius();
            for region in [HitRegion::Minimize, HitRegion::Maximize, HitRegion::Close] {
                let Some(rect) = Self::caption_button_rect(window, region)
                    .filter(|_| window.caption_shows(region))
                    .and_then(|rect| rect.intersection(damage))
                else {
                    continue;
                };
                type Shadow = (alloc::rc::Rc<crate::shadow::ShadowMask>, Rect);
                let uppers: alloc::vec::Vec<(&Window, u32, Option<Shadow>)> = self.z_order
                    [index + 1..]
                    .iter()
                    .filter_map(|upper| self.window(*upper).filter(|_| self.visible(*upper)))
                    .map(|upper| {
                        (
                            upper,
                            if upper.state == WindowState::Maximized {
                                0
                            } else {
                                upper.scale.len(window_radius())
                            },
                        )
                    })
                    .filter(|(upper, radius)| {
                        crate::shadow::window_shadow_bounds(upper.outer, upper.scale)
                            .filter(|_| *radius > 0)
                            .unwrap_or(upper.outer)
                            .intersection(rect)
                            .is_some()
                    })
                    .map(|(upper, radius)| {
                        let shadow = (radius > 0)
                            .then(|| self.window_shadow_mask(upper, upper.outer))
                            .flatten()
                            .and_then(|mask| {
                                let bounds = mask.bounds(upper.outer)?;
                                Some((mask, bounds))
                            });
                        (upper, radius, shadow)
                    })
                    .collect();
                for y in rect.y()..rect.bottom() {
                    for x in rect.x()..rect.right() {
                        if rounded_coverage(window.outer, radius, x, y) != 255 {
                            continue;
                        }
                        // A fractional rounded corner may hit the upper window
                        // while still revealing this caption underneath. Only
                        // fully covered upper pixels can discard the lower layer.
                        if uppers.iter().any(|(upper, radius, _)| {
                            rounded_coverage(upper.outer, *radius, x, y) == 255
                        }) {
                            continue;
                        }
                        let mut color = self.caption_pixel(placed, &base, region, x, y);
                        // Hit testing protects every upper window body. Its
                        // antialiased corner and exterior shadows still cover
                        // pixels outside the center hit contour: reapply those
                        // bounded layers instead of erasing their tint.
                        for (upper, radius, shadow) in &uppers {
                            let (upper, radius) = (*upper, *radius);
                            let coverage = rounded_coverage(upper.outer, radius, x, y);
                            if radius > 0
                                && coverage != 255
                                && let Some((mask, bounds)) = shadow
                            {
                                color = blend(color, [0, 0, 0, mask.alpha(*bounds, x, y)]);
                            }
                            if coverage != 0 {
                                // Such a center-missed body pixel is in a
                                // rounded corner, outside the title-text clip.
                                let upper_placed = Placed::at(upper);
                                let base = contour_color(
                                    upper.outer,
                                    radius,
                                    upper.scale.len(1),
                                    self.window_base_pixel(
                                        upper_placed,
                                        &self.static_base(upper),
                                        x,
                                        y,
                                    ),
                                    self.border_of(upper.id),
                                    x,
                                    y,
                                );
                                color = blend(color, with_coverage(base, coverage));
                            }
                        }
                        let at = (y as usize * self.output.width() as usize + x as usize) * 4;
                        pixels[at..at + 4].copy_from_slice(&color);
                    }
                }
            }
        }
        self.pixels = pixels;
    }
    /// Window body (chrome, client snapshot over its material, contour and
    /// antialiased corners) within `clip`. Interior spans take row paths:
    /// opaque snapshot rows are copied, translucent rows blend each pixel over
    /// the material, and only the rounded/bordered edge pixels take the exact
    /// per-pixel contour path. Both produce identical pixels.
    pub(crate) fn paint_body(
        &self,
        p: Placed<'_>,
        base: &Base<'_>,
        canvas: &mut Canvas<'_>,
        clip: Rect,
    ) {
        let w = p.w;
        let r = p.outer;
        let content = p.content;
        let radius = p.radius();
        let chrome = self.chrome_of(w.id);
        let border = self.border_of(w.id);
        // Mica windows show their material behind the trusted title bar
        // (an accent title bar replaces it on the active window).
        let mica_chrome =
            self.material_of(w) == Material::Mica && self.accent_title(w.id).is_none();
        let line = w.scale.len(1);
        let inset = radius.max(line) as i32;
        #[cfg(test)]
        let spans = !self.scalar_reference;
        #[cfg(not(test))]
        let spans = true;
        let mut cells = [[0u16; 3]; ROW_CELLS];
        for y in clip.y()..clip.bottom() {
            let interior = spans && y >= r.y() + line as i32 && y < r.bottom() - inset - 1;
            let left = clip.x().max(content.x() + inset);
            let right = clip.right().min(content.right() - inset);
            let (left, right) = if interior && right > left {
                self.paint_span(p, base, canvas, &mut cells, y, (left, right), chrome);
                (left, right)
            } else {
                (clip.right(), clip.right())
            };
            for x in (clip.x()..left).chain(right..clip.right()) {
                let coverage = rounded_coverage(r, radius, x, y);
                if coverage == 0 {
                    continue;
                }
                let mut color = chrome;
                if mica_chrome {
                    color = base.pixel(x + p.offset.0, y + p.offset.1);
                }
                if content.contains(x, y) {
                    color = base.pixel(x + p.offset.0, y + p.offset.1);
                    if let Some(s) = &w.snapshot {
                        // On resize, retain the prior safe pixels at native
                        // scale, clip to new content, and fill newly exposed area.
                        let sx = (x - content.x()) as usize;
                        let sy = (y - content.y()) as usize;
                        if sx < s.descriptor.width as usize && sy < s.descriptor.height as usize {
                            let offset = sy * s.descriptor.stride_bytes + sx * 4;
                            let source = core::array::from_fn(|i| s.bytes[offset + i]);
                            color = blend(color, source);
                        }
                    }
                }
                color = contour_color(r, radius, line, color, border, x, y);
                // Apply outer coverage once, including content and border;
                // painting both layers independently darkens and over-covers
                // antialiased bottom corners.
                canvas.put(x, y, with_coverage(color, coverage));
            }
        }
    }
    /// Fully covered interior pixels `span` (left..right) of row `y` (no
    /// contour). Translucent rows sample the material through a per-row grid
    /// (`RowBase`) prepared on first use, identical to per-pixel sampling.
    #[allow(clippy::too_many_arguments)]
    fn paint_span(
        &self,
        p: Placed<'_>,
        base: &Base<'_>,
        canvas: &mut Canvas<'_>,
        cells: &mut [[u16; 3]],
        y: i32,
        (left, right): (i32, i32),
        chrome: [u8; 4],
    ) {
        let content = p.content;
        let row = y as usize * canvas.stride;
        let by = y + p.offset.1;
        if !(content.y()..content.bottom()).contains(&y) {
            let mica = p.w.material == Material::Mica && self.accent_title(p.w.id).is_none();
            match base {
                Base::Wallpaper(_) | Base::Live(_) if mica => {
                    let row_base = base.row(by, cells);
                    for x in left..right {
                        let at = row + x as usize * 4;
                        let color = match &row_base {
                            Some(r) => r.pixel(cells, x + p.offset.0),
                            None => base.pixel(x + p.offset.0, by),
                        };
                        canvas.pixels[at..at + 4].copy_from_slice(&color);
                    }
                }
                Base::Solid(color) if mica => canvas.fill_physical(
                    Rect::new(left, y, (right - left) as u32, 1).unwrap(),
                    *color,
                ),
                _ => canvas.fill_physical(
                    Rect::new(left, y, (right - left) as u32, 1).unwrap(),
                    chrome,
                ),
            }
            return;
        }
        // Translucent rows (and uncovered tails) sample the material for
        // this row through one prepared grid row.
        let sampler = |cells: &mut [[u16; 3]]| base.row(by, cells);
        let sample = |sampler: &Option<RowBase>, cells: &[[u16; 3]], x: i32| match sampler {
            Some(row) => row.pixel(cells, x),
            None => base.pixel(x, by),
        };
        let mut done = left;
        if let Some(s) = &p.w.snapshot {
            let sy = y - content.y();
            if (0..s.descriptor.height as i32).contains(&sy) {
                let end = right.min(content.x() + s.descriptor.width as i32);
                if end > left {
                    let start =
                        sy as usize * s.descriptor.stride_bytes + (left - content.x()) as usize * 4;
                    let bytes = (end - left) as usize * 4;
                    let source = &s.bytes[start..start + bytes];
                    let dest = &mut canvas.pixels[row + left as usize * 4..][..bytes];
                    if !s.translucent[sy as usize] {
                        dest.copy_from_slice(source);
                    } else {
                        let row_base = sampler(cells);
                        let mut x = left + p.offset.0;
                        for (d, s) in dest.chunks_exact_mut(4).zip(source.chunks_exact(4)) {
                            if s[3] == 255 {
                                d.copy_from_slice(s);
                            } else {
                                let color = sample(&row_base, cells, x);
                                d.copy_from_slice(&if s[3] == 0 {
                                    color
                                } else {
                                    blend(color, [s[0], s[1], s[2], s[3]])
                                });
                            }
                            x += 1;
                        }
                    }
                    done = end;
                }
            }
        }
        if done < right {
            // An opaque material replaces these pixels completely; fill only
            // the uncovered tail of an older/smaller snapshot.
            if let Base::Solid(color) = base {
                canvas.fill_physical(
                    Rect::new(done, y, (right - done) as u32, 1).unwrap(),
                    *color,
                );
            } else {
                let row_base = sampler(cells);
                for x in done..right {
                    let at = row + x as usize * 4;
                    let color = sample(&row_base, cells, x + p.offset.0);
                    canvas.pixels[at..at + 4].copy_from_slice(&color);
                }
            }
        }
    }
    /// Shadow, body, optional trusted chrome and caption controls of one
    /// window into a private layer (everything outside it stays transparent).
    pub(crate) fn paint_window_layer(
        &self,
        p: Placed<'_>,
        base: &Base<'_>,
        canvas: &mut Canvas<'_>,
        chrome: Option<&mut ChromeCallback<'_>>,
    ) {
        let w = p.w;
        self.paint_window_shadow(canvas, w, p.outer, p.radius());
        let clip = canvas.clip_rect().and_then(|c| c.intersection(p.outer));
        if let Some(clip) = clip {
            self.paint_body(p, base, canvas, clip);
            if let Some(chrome) = chrome
                && let Ok(mut info) = self.window_info(w.id)
            {
                info.outer = p.outer;
                info.content = p.content;
                let saved = canvas.clip_rect();
                chrome(info, canvas);
                let _ = canvas.set_scale(Scale::ONE);
                if let Some(saved) = saved {
                    canvas.set_clip(saved);
                }
            }
            self.paint_caption_buttons(p, base, canvas);
        }
    }
    /// Close/minimize ghosts drawn directly above `anchor` (None: bottom).
    fn paint_ghosts(&self, canvas: &mut Canvas<'_>, anchor: Option<WindowId>) {
        let now = self.motion.now;
        for ghost in self.motion.ghosts.iter().filter(|g| g.below == anchor) {
            let pose = ghost.transition.pose(now);
            if let Some(dest) = pose.apply(ghost.layer.bounds) {
                draw_layer(canvas, &ghost.layer, dest, pose.alpha as u8);
            }
        }
    }
    pub(crate) fn compose_with_chrome(
        &mut self,
        region: Rect,
        visible: &[WindowId],
        background: &mut impl FnMut(&mut Canvas<'_>),
        chrome: &mut impl FnMut(WindowInfo<'_>, &mut Canvas<'_>),
        overlay: &mut impl FnMut(&mut Canvas<'_>),
    ) {
        // Temporarily move the reusable frame so the scene can be borrowed without
        // aliasing. No client memory is referenced here and no frame allocation occurs.
        let mut pixels = core::mem::take(&mut self.pixels);
        let mut live = core::mem::take(&mut self.motion.live);
        let now = self.motion.now;
        let tokens = self.theme.tokens();
        let stride = self.output.width() as usize * 4;
        {
            let mut canvas = Canvas::new(
                &mut pixels,
                self.output.width(),
                self.output.height(),
                stride,
            )
            .expect("validated frame");
            canvas.set_clip(region);
            let _ = canvas.fill_rect(region, tokens.desktop);
            background(&mut canvas);
            canvas.set_clip(region);
            self.paint_ghosts(&mut canvas, None);
            for (index, id) in visible.iter().enumerate() {
                let Some(w) = self.window(*id) else {
                    continue;
                };
                let transition = live.iter().position(|l| l.id == *id);
                if transition.is_some_and(|i| live[i].transition.is_none()) {
                    // A new window stays invisible until its first frame.
                    self.paint_ghosts(&mut canvas, Some(*id));
                    continue;
                }
                let r = w.outer;
                let placed = Placed::at(w);
                let radius = placed.radius();
                // A maximized window has square opaque chrome/content, even
                // before its first snapshot. Preserve shadows outside its bounds.
                let painted = if radius == 0 {
                    Some(r)
                } else {
                    crate::shadow::window_shadow_bounds(r, w.scale)
                };
                if transition.is_none()
                    && let Some(bounds) = painted.and_then(|r| r.intersection(self.output))
                    && visible[index + 1..].iter().any(|upper| {
                        self.window(*upper).is_some_and(|upper| {
                            let inset = if upper.state == WindowState::Maximized {
                                0
                            } else {
                                upper.scale.len(window_radius()) as i32
                            };
                            // Live acrylic and animating windows reveal what
                            // lies beneath them, so they never cull it.
                            !self.live_acrylic(upper)
                                && !live.iter().any(|l| l.id == upper.id)
                                && upper.outer.x() + inset <= bounds.x()
                                && upper.outer.y() + inset <= bounds.y()
                                && upper.outer.right() - inset >= bounds.right()
                                && upper.outer.bottom() - inset >= bounds.bottom()
                        })
                    })
                {
                    self.paint_ghosts(&mut canvas, Some(*id));
                    continue;
                }
                let touched = painted.is_some_and(|b| b.intersection(region).is_some());
                // Capture the scene composed beneath this window, before its
                // own shadow, as its live acrylic material.
                let backdrop = (touched && self.live_acrylic(w))
                    .then(|| {
                        r.expanded(crate::engine::acrylic::margin(w.scale))
                            .and_then(|area| area.intersection(self.output))
                            .and_then(|area| area.intersection(region))
                            .and_then(|area| {
                                crate::engine::acrylic::LiveBackdrop::capture(
                                    canvas.pixels(),
                                    stride,
                                    area,
                                    w.scale,
                                )
                            })
                    })
                    .flatten();
                let base = match &backdrop {
                    Some(backdrop) => Base::Live(backdrop),
                    None => self.static_base(w),
                };
                if let Some(i) = transition {
                    let state = &mut live[i];
                    let pose = state.transition.expect("animating").pose(now);
                    let dest = crate::engine::animation::layer_bounds(w)
                        .and_then(|bounds| pose.apply(bounds))
                        .filter(|dest| dest.intersection(region).is_some());
                    if let Some(dest) = dest {
                        // Opacity/scale reuse the retained snapshot; the
                        // layer is rebuilt only when its inputs changed.
                        let key = backdrop.is_none().then(|| self.layer_key(w));
                        if state
                            .layer
                            .as_ref()
                            .is_none_or(|l| key.is_none() || l.key != key)
                        {
                            let chrome: &mut ChromeCallback<'_> = &mut *chrome;
                            state.layer = self.build_layer(w, &base, Some(chrome), key);
                        }
                        if let Some(layer) = &state.layer {
                            canvas.set_clip(region);
                            draw_layer(&mut canvas, layer, dest, pose.alpha as u8);
                        }
                    }
                    canvas.set_clip(region);
                    self.paint_ghosts(&mut canvas, Some(*id));
                    continue;
                }
                self.paint_window_shadow(&mut canvas, w, r, radius);
                if let Some(clip) = r.intersection(region) {
                    self.paint_body(placed, &base, &mut canvas, clip);
                    chrome(self.window_info(*id).expect("visible window"), &mut canvas);
                    // Trusted title callbacks exclude these system controls.
                    // Paint the same retained pointer state on full and partial
                    // frames so a client commit cannot erase a hover/press.
                    let _ = canvas.set_scale(Scale::ONE);
                    canvas.set_clip(region);
                    self.paint_caption_buttons(placed, &base, &mut canvas);
                }
                self.paint_ghosts(&mut canvas, Some(*id));
            }
            // Ghosts whose anchor left the scene finish above every window.
            let now_visible = |id: WindowId| visible.contains(&id);
            for ghost in self
                .motion
                .ghosts
                .iter()
                .filter(|g| g.below.is_some_and(|below| !now_visible(below)))
            {
                let pose = ghost.transition.pose(now);
                if let Some(dest) = pose.apply(ghost.layer.bounds) {
                    draw_layer(&mut canvas, &ghost.layer, dest, pose.alpha as u8);
                }
            }
            overlay(&mut canvas);
        }
        self.pixels = pixels;
        self.motion.live = live;
    }
}

#[cfg(test)]
#[path = "shadow_tests.rs"]
mod shadow_tests;

#[cfg(test)]
mod coverage_tests {
    use super::*;
    #[test]
    fn packed_layer_arithmetic_matches_scalar_premultiplied_blending() {
        for c in 0..=255u32 {
            for a in 0..=255u32 {
                let packed = c | c << 8 | c << 16 | c << 24;
                let expected = (c * a + 127) / 255;
                let e = expected | expected << 8 | expected << 16 | expected << 24;
                assert_eq!(scale_word(packed, a), e, "c={c} a={a}");
            }
        }
        let mut seed = 0x2545_f491u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        for _ in 0..200_000 {
            let alpha = (next() & 255) as u8;
            let s: [u8; 4] = core::array::from_fn(|i| {
                if i == 3 {
                    alpha
                } else {
                    (next() % (u32::from(alpha) + 1)) as u8
                }
            });
            let d = [
                (next() & 255) as u8,
                (next() & 255) as u8,
                (next() & 255) as u8,
                255,
            ];
            let packed = blend_word(u32::from_le_bytes(d), u32::from_le_bytes(s));
            assert_eq!(packed.to_le_bytes(), blend(d, s), "d={d:?} s={s:?}");
            let f = next() % 257;
            let mixed = lerp_word(u32::from_le_bytes(s), u32::from_le_bytes(d), f).to_le_bytes();
            assert!(mixed[..3].iter().all(|v| *v <= mixed[3]) || alpha != 255);
        }
    }
    #[test]
    fn cached_geometry_matches_all_sixteen_samples_at_every_radius_and_corner() {
        for radius in (1..=60).chain([96, 192, 288]) {
            let rect = Rect::new(-7, -13, radius * 2 + 3, radius * 2 + 5).unwrap();
            for y in 0..radius as i32 {
                for x in 0..radius as i32 {
                    for (px, py) in [
                        (rect.x() + x, rect.y() + y),
                        (rect.right() - 1 - x, rect.y() + y),
                        (rect.x() + x, rect.bottom() - 1 - y),
                        (rect.right() - 1 - x, rect.bottom() - 1 - y),
                    ] {
                        assert_eq!(
                            rounded_coverage(rect, radius, px, py),
                            rounded_coverage_sampled(rect, radius, px, py)
                        );
                    }
                }
            }
        }
        // Clamped radii and one-pixel dimensions retain the same interior rule.
        for width in 1..20 {
            for height in 1..20 {
                let rect = Rect::new(-4, 9, width, height).unwrap();
                for y in rect.y() - 1..=rect.bottom() {
                    for x in rect.x() - 1..=rect.right() {
                        assert_eq!(
                            rounded_coverage(rect, 60, x, y),
                            rounded_coverage_sampled(rect, 60, x, y)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn large_radius_clamping_and_uncached_neighbors_match_sampling() {
        for radius in [95, 96, 97, 191, 192, 193, 287, 288, 289] {
            for width in [radius * 2, radius * 2 + 1] {
                let rect = Rect::new(23, -19, width, radius * 2 + 3).unwrap();
                for requested in [radius, radius + 300, u32::MAX] {
                    for y in [-1, 0, 1, radius as i32 / 2, radius as i32 - 1] {
                        for x in [-1, 0, 1, radius as i32 / 2, radius as i32 - 1] {
                            for (px, py) in [
                                (rect.x() + x, rect.y() + y),
                                (rect.right() - 1 - x, rect.y() + y),
                                (rect.x() + x, rect.bottom() - 1 - y),
                                (rect.right() - 1 - x, rect.bottom() - 1 - y),
                            ] {
                                assert_eq!(
                                    rounded_coverage(rect, requested, px, py),
                                    rounded_coverage_sampled(rect, requested, px, py),
                                    "radius={radius} requested={requested} x={px} y={py}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn avatar_fill_matches_sampled_pixels_with_scale_clip_padding_and_large_radius() {
        for scale in 1..=3 {
            let width = 208 * scale;
            let height = 206 * scale;
            let stride = width as usize * 4 + 12;
            let clip = Rect::new(3, 5, width - 7, height - 11).unwrap();
            for (draw, radius) in [
                (Rect::new(7, 5, 192, 192).unwrap(), 96),
                (Rect::new(-31, -17, 192, 193).unwrap(), 96),
                (Rect::new(8, 11, 192, 193).unwrap(), u32::MAX),
                (Rect::new(-4, 8, 19, 7).unwrap(), u32::MAX),
            ] {
                let mut pixels = alloc::vec![79; stride * height as usize + 28];
                for (index, pixel) in pixels.chunks_exact_mut(4).enumerate() {
                    let a = (index * 17) as u8;
                    pixel.copy_from_slice(&[a / 2, a / 3, a, a]);
                }
                let mut expected = pixels.clone();
                let mut canvas = Canvas::new(&mut pixels, width, height, stride).unwrap();
                canvas
                    .set_scale(Scale::from_factor(scale).unwrap())
                    .unwrap();
                canvas.set_clip(clip);
                let physical = canvas.scaled(draw).unwrap();
                let effective = radius
                    .saturating_mul(scale)
                    .min(physical.width() / 2)
                    .min(physical.height() / 2);
                let color = [18, 18, 18, 18]; // Actual sign-in avatar fill.
                canvas.fill_rounded_rect(draw, radius, color).unwrap();
                let mut reference = Canvas::new(&mut expected, width, height, stride).unwrap();
                reference.set_clip(clip);
                if let Some(area) = physical.intersection(clip) {
                    for y in area.y()..area.bottom() {
                        for x in area.x()..area.right() {
                            let coverage = rounded_coverage_sampled(physical, effective, x, y);
                            reference
                                .pixel(x, y, with_coverage(color, coverage))
                                .unwrap();
                        }
                    }
                }
                assert_eq!(
                    pixels, expected,
                    "scale={scale} draw={draw:?} radius={radius}"
                );
            }
        }
    }

    #[test]
    fn huge_valid_circles_clip_to_tiny_canvas_without_squared_distance_overflow() {
        let extent = i32::MAX as u32;
        let (width, height, stride) = (5, 4, 28);
        let color = [18, 18, 18, 18];
        for (origin, covered) in [(0, false), (-1_073_741_823, true)] {
            let mut pixels = alloc::vec![79; stride * height as usize + 20];
            let mut expected = pixels.clone();
            // At the far top-left the enormous circle covers nothing; at its
            // center it covers this entire tiny viewport. Neither oracle squares.
            if covered {
                Canvas::new(&mut expected, width, height, stride)
                    .unwrap()
                    .fill_rect(Rect::new(0, 0, width, height).unwrap(), color)
                    .unwrap();
            }
            let rect = Rect::new(origin, origin, extent, extent).unwrap();
            Canvas::new(&mut pixels, width, height, stride)
                .unwrap()
                .fill_rounded_rect(rect, u32::MAX, color)
                .unwrap();
            assert_eq!(pixels, expected, "origin={origin}");
            assert_eq!(rounded_contains(rect, u32::MAX, 2, 2), covered);
        }
    }
}

#[cfg(test)]
mod stroke_tests {
    use super::*;
    #[test]
    fn a_rounded_stroke_is_one_ring_with_a_clear_inside() {
        let mut pixels = alloc::vec![0u8; 48 * 28 * 4];
        let mut c = Canvas::new(&mut pixels, 48, 28, 48 * 4).unwrap();
        c.stroke_rounded_rect(
            Rect::new(4, 4, 40, 20).unwrap(),
            10,
            1,
            [255, 255, 255, 255],
        )
        .unwrap();
        let at = |x: usize, y: usize| pixels[(y * 48 + x) * 4];
        // Straight top and left edges: the first row/column only.
        assert_eq!((at(24, 4), at(24, 5), at(24, 6)), (255, 0, 0));
        assert_eq!(at(24, 23), 255);
        assert_eq!(at(24, 22), 0);
        // The centre stays clear.
        for y in 7..21 {
            for x in 16..32 {
                assert_eq!(at(x, y), 0, "inside ({x},{y})");
            }
        }
    }
}
