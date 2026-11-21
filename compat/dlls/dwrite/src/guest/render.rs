//! Rendering parameters, glyph-run analysis (alpha textures) and the
//! system font fallback.
use super::*;
use crate::raster::{self, Command, Matrix};

const IID_PARAMS: Guid = guid(
    0x2f0da53a,
    0x2add,
    0x47cd,
    [0x82, 0xee, 0xd9, 0xec, 0x34, 0x68, 0x8e, 0x75],
);
const IID_PARAMS1: Guid = guid(
    0x94413cf4,
    0xa6fc,
    0x4248,
    [0x8b, 0x50, 0x66, 0x74, 0x34, 0x8f, 0xca, 0xd3],
);
const IID_PARAMS2: Guid = guid(
    0xf9d711c3,
    0x9777,
    0x40ae,
    [0x87, 0xe8, 0x3e, 0x5a, 0xf9, 0xbf, 0x09, 0x48],
);
const IID_ANALYSIS: Guid = guid(
    0x7d97dbf7,
    0xe085,
    0x42d4,
    [0x81, 0xe3, 0x6a, 0x88, 0x3b, 0xde, 0xd1, 0x18],
);
const IID_FALLBACK: Guid = guid(
    0xefa008f9,
    0xf7a1,
    0x48bf,
    [0xb0, 0x5c, 0xf2, 0x24, 0x71, 0x3c, 0xc0, 0xff],
);

// ---- rendering parameters -------------------------------------------------------------------
/// IDWriteRenderingParams2's values (Windows' defaults for a 96 dpi
/// display: gamma 1.8, contrast 0.5, grayscale contrast 1, ClearType 1,
/// RGB stripes, default mode and grid fit).
#[derive(Clone, Copy)]
pub(crate) struct Params {
    gamma: f32,
    contrast: f32,
    grayscale_contrast: f32,
    cleartype: f32,
    geometry: u32,
    mode: u32,
    grid: u32,
}
const DEFAULT_PARAMS: Params = Params {
    gamma: 1.8,
    contrast: 0.5,
    grayscale_contrast: 1.0,
    cleartype: 1.0,
    geometry: 1,
    mode: 0,
    grid: 0,
};
static PARAMS_VTBL: Vtbl<10> = Vtbl([
    query::<Params> as *const (),
    add_ref::<Params> as *const (),
    release::<Params> as *const (),
    params_gamma as *const (),
    params_contrast as *const (),
    params_cleartype as *const (),
    params_geometry as *const (),
    params_mode as *const (),
    params_grayscale_contrast as *const (),
    params_grid as *const (),
]);
impl Object for Params {
    const IIDS: &'static [Guid] = &[IID_PARAMS, IID_PARAMS1, IID_PARAMS2];
    fn vtbl() -> *const () {
        &PARAMS_VTBL as *const _ as *const ()
    }
}
extern "system" fn params_gamma(this: usize) -> f32 {
    data::<Params>(this).gamma
}
extern "system" fn params_contrast(this: usize) -> f32 {
    data::<Params>(this).contrast
}
extern "system" fn params_cleartype(this: usize) -> f32 {
    data::<Params>(this).cleartype
}
extern "system" fn params_geometry(this: usize) -> u32 {
    data::<Params>(this).geometry
}
extern "system" fn params_mode(this: usize) -> u32 {
    data::<Params>(this).mode
}
extern "system" fn params_grayscale_contrast(this: usize) -> f32 {
    data::<Params>(this).grayscale_contrast
}
extern "system" fn params_grid(this: usize) -> u32 {
    data::<Params>(this).grid
}
fn put_params(out: usize, p: Params) -> u32 {
    if out == 0 {
        return E_INVALIDARG;
    }
    put_ptr(out, new(p));
    S_OK
}
pub(crate) extern "system" fn create_rendering_params(_this: usize, out: usize) -> u32 {
    put_params(out, DEFAULT_PARAMS)
}
pub(crate) extern "system" fn create_monitor_rendering_params(
    _this: usize,
    _monitor: usize,
    out: usize,
) -> u32 {
    put_params(out, DEFAULT_PARAMS)
}
pub(crate) extern "system" fn create_custom_rendering_params(
    _this: usize,
    gamma: f32,
    contrast: f32,
    cleartype: f32,
    geometry: u32,
    mode: u32,
    out: usize,
) -> u32 {
    put_params(
        out,
        Params {
            gamma,
            contrast,
            cleartype,
            geometry,
            mode,
            ..DEFAULT_PARAMS
        },
    )
}
#[allow(clippy::too_many_arguments)]
pub(crate) extern "system" fn create_custom_rendering_params1(
    _this: usize,
    gamma: f32,
    contrast: f32,
    grayscale_contrast: f32,
    cleartype: f32,
    geometry: u32,
    mode: u32,
    out: usize,
) -> u32 {
    put_params(
        out,
        Params {
            gamma,
            contrast,
            grayscale_contrast,
            cleartype,
            geometry,
            mode,
            grid: 0,
        },
    )
}
#[allow(clippy::too_many_arguments)]
pub(crate) extern "system" fn create_custom_rendering_params2(
    _this: usize,
    gamma: f32,
    contrast: f32,
    grayscale_contrast: f32,
    cleartype: f32,
    geometry: u32,
    mode: u32,
    grid: u32,
    out: usize,
) -> u32 {
    put_params(
        out,
        Params {
            gamma,
            contrast,
            grayscale_contrast,
            cleartype,
            geometry,
            mode,
            grid,
        },
    )
}

// ---- glyph-run analysis -----------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq)]
enum Texture {
    /// DWRITE_RENDERING_MODE_ALIASED: 0 or 255 in a 1x1 texture.
    Aliased,
    /// Grayscale antialiasing (IDWriteFactory2): coverage in a 1x1 texture.
    Grayscale,
    /// ClearType: a 3x1 texture (the same coverage in each subpixel).
    ClearType,
}
pub(crate) struct Analysis {
    texture: Texture,
    commands: Vec<Command>,
    bounds: [i32; 4],
}
static ANALYSIS_VTBL: Vtbl<6> = Vtbl([
    query::<Analysis> as *const (),
    add_ref::<Analysis> as *const (),
    release::<Analysis> as *const (),
    analysis_bounds as *const (),
    analysis_texture as *const (),
    analysis_blend as *const (),
]);
impl Object for Analysis {
    const IIDS: &'static [Guid] = &[IID_ANALYSIS];
    fn vtbl() -> *const () {
        &ANALYSIS_VTBL as *const _ as *const ()
    }
}
fn read_matrix(at: usize) -> Matrix {
    if at == 0 {
        return Matrix::IDENTITY;
    }
    Matrix {
        m11: get_f32(at),
        m12: get_f32(at + 4),
        m21: get_f32(at + 8),
        m22: get_f32(at + 12),
        dx: get_f32(at + 16),
        dy: get_f32(at + 20),
    }
}
/// DWRITE_GLYPH_RUN { fontFace, fontEmSize, glyphCount, glyphIndices,
/// glyphAdvances, glyphOffsets, isSideways, bidiLevel }.
fn analysis(run: usize, matrix: Matrix, texture: Texture, origin: (f32, f32), out: usize) -> u32 {
    put_ptr(out, 0);
    if run == 0 || out == 0 {
        return E_INVALIDARG;
    }
    let w = core::mem::size_of::<usize>();
    let face_pointer = get_ptr(run);
    let Some(face) = ours::<fonts::FaceObject>(face_pointer) else {
        return E_INVALIDARG;
    };
    let em = get_f32(run + w);
    let count = get_u32(run + w + 4);
    let indices = get_ptr(run + w + 8);
    let advances = get_ptr(run + 2 * w + 8);
    let offsets = get_ptr(run + 3 * w + 8);
    let bidi = get_u32(run + 4 * w + 12);
    if !em.is_finite() || em <= 0.0 || (indices == 0 && count > 0) {
        return E_INVALIDARG;
    }
    let glyphs = fonts::run_glyphs(&face.0, em, indices, advances, offsets, count);
    let outline = raster::run_outline(&face.0, em, &glyphs, bidi & 1 != 0);
    let commands = raster::transform(&outline, &matrix, origin);
    let bounds = raster::bounds(&commands);
    put_ptr(
        out,
        new(Analysis {
            texture,
            commands,
            bounds,
        }),
    );
    S_OK
}
/// The device transform: the glyph transform, then pixels per DIP.
fn device(transform: usize, pixels_per_dip: f32) -> Matrix {
    let m = read_matrix(transform);
    let s = if pixels_per_dip.is_finite() && pixels_per_dip > 0.0 {
        pixels_per_dip
    } else {
        1.0
    };
    m.scale(s)
}
#[allow(clippy::too_many_arguments)]
pub(crate) extern "system" fn create_glyph_run_analysis(
    _this: usize,
    run: usize,
    pixels_per_dip: f32,
    transform: usize,
    mode: u32,
    _measuring: u32,
    x: f32,
    y: f32,
    out: usize,
) -> u32 {
    // IDWriteFactory: aliased or ClearType textures only.
    let texture = if mode == 1 {
        Texture::Aliased
    } else {
        Texture::ClearType
    };
    analysis(run, device(transform, pixels_per_dip), texture, (x, y), out)
}
#[allow(clippy::too_many_arguments)]
pub(crate) extern "system" fn create_glyph_run_analysis2(
    _this: usize,
    run: usize,
    transform: usize,
    mode: u32,
    _measuring: u32,
    _grid: u32,
    antialias: u32,
    x: f32,
    y: f32,
    out: usize,
) -> u32 {
    let texture = match (mode, antialias) {
        (1, _) => Texture::Aliased,
        // DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE.
        (_, 1) => Texture::Grayscale,
        _ => Texture::ClearType,
    };
    analysis(run, device(transform, 1.0), texture, (x, y), out)
}
/// Whether a texture type (ALIASED_1x1 0, CLEARTYPE_3x1 1) fits the
/// analysis.
fn fits(a: &Analysis, kind: u32) -> bool {
    match kind {
        0 => a.texture != Texture::ClearType,
        1 => a.texture == Texture::ClearType,
        _ => false,
    }
}
extern "system" fn analysis_bounds(this: usize, kind: u32, rect: usize) -> u32 {
    if rect == 0 || kind > 1 {
        return E_INVALIDARG;
    }
    let a = data::<Analysis>(this);
    let b = if fits(a, kind) { a.bounds } else { [0; 4] };
    for (i, v) in b.iter().enumerate() {
        put_u32(rect + 4 * i, *v as u32);
    }
    S_OK
}
extern "system" fn analysis_texture(
    this: usize,
    kind: u32,
    rect: usize,
    out: usize,
    size: u32,
) -> u32 {
    let a = data::<Analysis>(this);
    if rect == 0 || out == 0 || kind > 1 {
        return E_INVALIDARG;
    }
    let r = [
        get_u32(rect) as i32,
        get_u32(rect + 4) as i32,
        get_u32(rect + 8) as i32,
        get_u32(rect + 12) as i32,
    ];
    let width = (r[2] - r[0]).max(0) as usize;
    let height = (r[3] - r[1]).max(0) as usize;
    let per = if kind == 1 { 3 } else { 1 };
    let needed = width * height * per;
    if (size as usize) < needed {
        return E_NOT_SUFFICIENT_BUFFER;
    }
    if !fits(a, kind) {
        unsafe { core::ptr::write_bytes(out as *mut u8, 0, needed) };
        return S_OK;
    }
    let alpha = raster::coverage(&a.commands, r, a.texture == Texture::Aliased);
    if per == 1 {
        put_bytes(out, &alpha);
    } else {
        let tripled: Vec<u8> = alpha.iter().flat_map(|v| [*v, *v, *v]).collect();
        put_bytes(out, &tripled);
    }
    S_OK
}
extern "system" fn analysis_blend(
    this: usize,
    params: usize,
    gamma: usize,
    contrast: usize,
    cleartype: usize,
) -> u32 {
    let p = ours::<Params>(params).copied().unwrap_or(DEFAULT_PARAMS);
    let grayscale = data::<Analysis>(this).texture != Texture::ClearType;
    put_f32(gamma, p.gamma);
    put_f32(
        contrast,
        if grayscale {
            p.grayscale_contrast
        } else {
            p.contrast
        },
    );
    put_f32(cleartype, if grayscale { 0.0 } else { p.cleartype });
    S_OK
}
/// No color glyph layers are reported: color fonts render with their
/// outline glyphs.
#[allow(clippy::too_many_arguments)]
pub(crate) extern "system" fn translate_color_glyph_run(
    _this: usize,
    _x: f32,
    _y: f32,
    _run: usize,
    _description: usize,
    _measuring: u32,
    _transform: usize,
    _palette: u32,
    out: usize,
) -> u32 {
    put_ptr(out, 0);
    DWRITE_E_NOCOLOR
}

// ---- font fallback -------------------------------------------------------------------------
pub(crate) struct Fallback;
static FALLBACK_VTBL: Vtbl<4> = Vtbl([
    static_query::<Fallback> as *const (),
    static_count as *const (),
    static_count as *const (),
    map_characters as *const (),
]);
impl Object for Fallback {
    const IIDS: &'static [Guid] = &[IID_FALLBACK];
    fn vtbl() -> *const () {
        &FALLBACK_VTBL as *const _ as *const ()
    }
}
#[repr(C)]
struct StaticFallback {
    vtbl: *const (),
}
unsafe impl Sync for StaticFallback {}
static FALLBACK: StaticFallback = StaticFallback {
    vtbl: &FALLBACK_VTBL as *const _ as *const (),
};
pub(crate) extern "system" fn get_system_font_fallback(_this: usize, out: usize) -> u32 {
    if out == 0 {
        return E_INVALIDARG;
    }
    put_ptr(out, &FALLBACK as *const StaticFallback as usize);
    S_OK
}
/// MapCharacters: the text comes from the caller's
/// IDWriteTextAnalysisSource (GetTextAtPosition, GetLocaleName).
#[allow(clippy::too_many_arguments)]
extern "system" fn map_characters(
    _this: usize,
    source: usize,
    position: u32,
    length: u32,
    base_collection: usize,
    base_family: usize,
    weight: u32,
    style: u32,
    stretch: u32,
    mapped_length: usize,
    mapped_font: usize,
    scale: usize,
) -> u32 {
    put_u32(mapped_length, 0);
    put_ptr(mapped_font, 0);
    put_f32(scale, 1.0);
    if source == 0 || mapped_length == 0 || mapped_font == 0 {
        return E_INVALIDARG;
    }
    type Text = extern "system" fn(usize, u32, usize, usize) -> u32;
    type Locale = extern "system" fn(usize, u32, usize, usize) -> u32;
    let get_text: Text = unsafe { core::mem::transmute(method(source, 3)) };
    let get_locale: Locale = unsafe { core::mem::transmute(method(source, 6)) };
    // The text, in the source's chunks, up to `length` units.
    let mut units: Vec<u16> = Vec::new();
    while (units.len() as u32) < length {
        let (mut pointer, mut count) = (0usize, 0u32);
        let at = position + units.len() as u32;
        if get_text(
            source,
            at,
            &mut pointer as *mut usize as usize,
            &mut count as *mut u32 as usize,
        ) != S_OK
            || pointer == 0
            || count == 0
        {
            break;
        }
        let take = count.min(length - units.len() as u32) as usize;
        units.extend((0..take).map(|i| get_u16(pointer + 2 * i)));
    }
    let (mut locale_pointer, mut locale_length) = (0usize, 0u32);
    let locale = if get_locale(
        source,
        position,
        &mut locale_length as *mut u32 as usize,
        &mut locale_pointer as *mut usize as usize,
    ) == S_OK
    {
        text(&wide(locale_pointer, 85))
    } else {
        alloc::string::String::from("en-us")
    };
    let system = system();
    let base_collection = fonts::collection_of(base_collection);
    let family_name = text(&wide(base_family, 256));
    let base = base_collection
        .find(&family_name)
        .and_then(|f| crate::fallback::best(&base_collection, f, weight, style, stretch));
    let from_base = base.is_some();
    let (length, font) =
        crate::fallback::map_from(&system, &units, base, weight, style, stretch, &locale);
    put_u32(mapped_length, length as u32);
    if let Some((face, simulations)) = font {
        let face = if simulations == 0 {
            face
        } else {
            Arc::new(face.simulated(simulations))
        };
        // The requested font comes from its collection; fallbacks from
        // the system collection.
        let collection = if from_base && base_collection.locate(&face).is_some() {
            base_collection
        } else {
            system
        };
        if let Some((family, _)) = collection.locate(&face) {
            put_ptr(mapped_font, fonts::new_font(collection, family, face));
        }
    }
    S_OK
}
