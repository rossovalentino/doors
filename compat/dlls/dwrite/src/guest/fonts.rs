//! Collections, families, font lists, fonts, font faces and localized
//! strings.
use super::*;
use crate::font::{Face, matching};
use crate::raster::{self, Command, Glyph};
use alloc::string::String;

const IID_COLLECTION: Guid = guid(
    0xa84cee02,
    0x3eea,
    0x4eee,
    [0xa8, 0x27, 0x87, 0xc1, 0xa0, 0x2a, 0x0f, 0xcc],
);
const IID_FONT_LIST: Guid = guid(
    0x1a0d8438,
    0x1d97,
    0x4ec1,
    [0xae, 0xf9, 0xa2, 0xfb, 0x86, 0xed, 0x6a, 0xcb],
);
const IID_FAMILY: Guid = guid(
    0xda20d8ef,
    0x812a,
    0x4c43,
    [0x98, 0x02, 0x62, 0xec, 0x4a, 0xbd, 0x7a, 0xdd],
);
const IID_FONT: Guid = guid(
    0xacd16696,
    0x8c14,
    0x4f5d,
    [0x87, 0x7e, 0xfe, 0x3f, 0xc1, 0xd3, 0x27, 0x37],
);
const IID_FACE: Guid = guid(
    0x5f49804d,
    0x7024,
    0x4d43,
    [0xbf, 0xa9, 0xd2, 0x59, 0x84, 0xf5, 0x38, 0x49],
);
const IID_STRINGS: Guid = guid(
    0x08256209,
    0x099a,
    0x4b34,
    [0xb8, 0x6d, 0xc2, 0x2b, 0x11, 0x0e, 0x77, 0x71],
);

// ---- localized strings ------------------------------------------------------------------
pub(crate) struct Strings(Vec<(Vec<u16>, Vec<u16>)>);
static STRINGS_VTBL: Vtbl<9> = Vtbl([
    query::<Strings> as *const (),
    add_ref::<Strings> as *const (),
    release::<Strings> as *const (),
    strings_count as *const (),
    strings_find as *const (),
    strings_locale_length as *const (),
    strings_locale as *const (),
    strings_length as *const (),
    strings_get as *const (),
]);
impl Object for Strings {
    const IIDS: &'static [Guid] = &[IID_STRINGS];
    fn vtbl() -> *const () {
        &STRINGS_VTBL as *const _ as *const ()
    }
}
pub(crate) fn new_strings(items: &[(String, String)]) -> usize {
    new(Strings(
        items
            .iter()
            .map(|(l, t)| (l.encode_utf16().collect(), t.encode_utf16().collect()))
            .collect(),
    ))
}
extern "system" fn strings_count(this: usize) -> u32 {
    data::<Strings>(this).0.len() as u32
}
extern "system" fn strings_find(this: usize, locale: usize, index: usize, exists: usize) -> u32 {
    let want = text(&wide(locale, 85)).to_ascii_lowercase();
    let found = data::<Strings>(this)
        .0
        .iter()
        .position(|(l, _)| text(l).to_ascii_lowercase() == want);
    put_u32(index, found.map_or(u32::MAX, |i| i as u32));
    put_u32(exists, u32::from(found.is_some()));
    S_OK
}
fn strings_item(this: usize, index: u32, locale: bool) -> Option<&'static [u16]> {
    let s: &'static Strings = data::<Strings>(this);
    s.0.get(index as usize)
        .map(|(l, t)| if locale { l.as_slice() } else { t.as_slice() })
}
fn put_string(units: &[u16], out: usize, size: u32) -> u32 {
    if (size as usize) < units.len() + 1 {
        if size > 0 {
            put_bytes(out, &[0, 0]);
        }
        return E_NOT_SUFFICIENT_BUFFER;
    }
    let bytes: Vec<u8> = units
        .iter()
        .chain(&[0])
        .flat_map(|u| u.to_le_bytes())
        .collect();
    put_bytes(out, &bytes);
    S_OK
}
extern "system" fn strings_locale_length(this: usize, index: u32, length: usize) -> u32 {
    match strings_item(this, index, true) {
        Some(s) => {
            put_u32(length, s.len() as u32);
            S_OK
        }
        None => E_INVALIDARG,
    }
}
extern "system" fn strings_locale(this: usize, index: u32, out: usize, size: u32) -> u32 {
    strings_item(this, index, true).map_or(E_INVALIDARG, |s| put_string(s, out, size))
}
extern "system" fn strings_length(this: usize, index: u32, length: usize) -> u32 {
    match strings_item(this, index, false) {
        Some(s) => {
            put_u32(length, s.len() as u32);
            S_OK
        }
        None => E_INVALIDARG,
    }
}
extern "system" fn strings_get(this: usize, index: u32, out: usize, size: u32) -> u32 {
    strings_item(this, index, false).map_or(E_INVALIDARG, |s| put_string(s, out, size))
}

// ---- collections ----------------------------------------------------------------------------
pub(crate) struct CollectionObject(pub Arc<Collection>);
static COLLECTION_VTBL: Vtbl<7> = Vtbl([
    query::<CollectionObject> as *const (),
    add_ref::<CollectionObject> as *const (),
    release::<CollectionObject> as *const (),
    collection_count as *const (),
    collection_family as *const (),
    collection_find as *const (),
    collection_font_from_face as *const (),
]);
impl Object for CollectionObject {
    const IIDS: &'static [Guid] = &[IID_COLLECTION];
    fn vtbl() -> *const () {
        &COLLECTION_VTBL as *const _ as *const ()
    }
}
pub(crate) fn new_collection(c: Arc<Collection>) -> usize {
    new(CollectionObject(c))
}
/// The collection behind a collection pointer (ours), else the system one.
pub(crate) fn collection_of(pointer: usize) -> Arc<Collection> {
    ours::<CollectionObject>(pointer).map_or_else(system, |c| c.0.clone())
}
extern "system" fn collection_count(this: usize) -> u32 {
    data::<CollectionObject>(this).0.families.len() as u32
}
extern "system" fn collection_family(this: usize, index: u32, out: usize) -> u32 {
    let c = data::<CollectionObject>(this).0.clone();
    if index as usize >= c.families.len() {
        put_ptr(out, 0);
        return E_INVALIDARG;
    }
    put_ptr(out, new_family(c, index as usize));
    S_OK
}
extern "system" fn collection_find(this: usize, name: usize, index: usize, exists: usize) -> u32 {
    let found = data::<CollectionObject>(this)
        .0
        .find(&text(&wide(name, 256)));
    put_u32(index, found.map_or(u32::MAX, |i| i as u32));
    put_u32(exists, u32::from(found.is_some()));
    S_OK
}
extern "system" fn collection_font_from_face(this: usize, face: usize, out: usize) -> u32 {
    put_ptr(out, 0);
    let c = data::<CollectionObject>(this).0.clone();
    let Some(f) = ours::<FaceObject>(face) else {
        return E_INVALIDARG;
    };
    match c.locate(&f.0) {
        Some((family, _)) => {
            put_ptr(out, new_font(c, family, f.0.clone()));
            S_OK
        }
        None => DWRITE_E_NOFONT,
    }
}

// ---- families and font lists ----------------------------------------------------------------
/// A family (all its fonts) or a font list (some of them, with
/// simulations).
pub(crate) struct ListObject {
    collection: Arc<Collection>,
    family: usize,
    fonts: Vec<(usize, u32)>,
}
static FAMILY_VTBL: Vtbl<9> = Vtbl([
    query::<FamilyObject> as *const (),
    add_ref::<FamilyObject> as *const (),
    release::<FamilyObject> as *const (),
    list_collection::<FamilyObject> as *const (),
    list_count::<FamilyObject> as *const (),
    list_font::<FamilyObject> as *const (),
    family_names as *const (),
    family_first_match as *const (),
    family_matches as *const (),
]);
static LIST_VTBL: Vtbl<6> = Vtbl([
    query::<FontListObject> as *const (),
    add_ref::<FontListObject> as *const (),
    release::<FontListObject> as *const (),
    list_collection::<FontListObject> as *const (),
    list_count::<FontListObject> as *const (),
    list_font::<FontListObject> as *const (),
]);
pub(crate) struct FamilyObject(ListObject);
pub(crate) struct FontListObject(ListObject);
trait List: Object {
    fn list(&self) -> &ListObject;
}
impl List for FamilyObject {
    fn list(&self) -> &ListObject {
        &self.0
    }
}
impl List for FontListObject {
    fn list(&self) -> &ListObject {
        &self.0
    }
}
impl Object for FamilyObject {
    const IIDS: &'static [Guid] = &[IID_FAMILY, IID_FONT_LIST];
    fn vtbl() -> *const () {
        &FAMILY_VTBL as *const _ as *const ()
    }
}
impl Object for FontListObject {
    const IIDS: &'static [Guid] = &[IID_FONT_LIST];
    fn vtbl() -> *const () {
        &LIST_VTBL as *const _ as *const ()
    }
}
fn new_family(collection: Arc<Collection>, family: usize) -> usize {
    let fonts = (0..collection.families[family].fonts.len())
        .map(|i| (i, 0))
        .collect();
    new(FamilyObject(ListObject {
        collection,
        family,
        fonts,
    }))
}
extern "system" fn list_collection<T: List>(this: usize, out: usize) -> u32 {
    put_ptr(
        out,
        new_collection(data::<T>(this).list().collection.clone()),
    );
    S_OK
}
extern "system" fn list_count<T: List>(this: usize) -> u32 {
    data::<T>(this).list().fonts.len() as u32
}
extern "system" fn list_font<T: List>(this: usize, index: u32, out: usize) -> u32 {
    let l = data::<T>(this).list();
    let Some(&(i, simulations)) = l.fonts.get(index as usize) else {
        put_ptr(out, 0);
        return E_INVALIDARG;
    };
    let face = &l.collection.families[l.family].fonts[i];
    let face = if simulations == 0 {
        face.clone()
    } else {
        Arc::new(face.simulated(simulations))
    };
    put_ptr(out, new_font(l.collection.clone(), l.family, face));
    S_OK
}
extern "system" fn family_names(this: usize, out: usize) -> u32 {
    let l = &data::<FamilyObject>(this).0;
    put_ptr(out, new_strings(&l.collection.families[l.family].names));
    S_OK
}
extern "system" fn family_first_match(
    this: usize,
    weight: u32,
    stretch: u32,
    style: u32,
    out: usize,
) -> u32 {
    let l = &data::<FamilyObject>(this).0;
    let fonts = &l.collection.families[l.family].fonts;
    match matching(fonts, weight, stretch, style).first() {
        Some(&(i, simulations)) => {
            let face = if simulations == 0 {
                fonts[i].clone()
            } else {
                Arc::new(fonts[i].simulated(simulations))
            };
            put_ptr(out, new_font(l.collection.clone(), l.family, face));
            S_OK
        }
        None => {
            put_ptr(out, 0);
            DWRITE_E_NOFONT
        }
    }
}
extern "system" fn family_matches(
    this: usize,
    weight: u32,
    stretch: u32,
    style: u32,
    out: usize,
) -> u32 {
    let l = &data::<FamilyObject>(this).0;
    let fonts = matching(
        &l.collection.families[l.family].fonts,
        weight,
        stretch,
        style,
    );
    put_ptr(
        out,
        new(FontListObject(ListObject {
            collection: l.collection.clone(),
            family: l.family,
            fonts,
        })),
    );
    S_OK
}

// ---- fonts ------------------------------------------------------------------------------------
pub(crate) struct FontObject {
    collection: Arc<Collection>,
    family: usize,
    pub face: Arc<Face>,
}
static FONT_VTBL: Vtbl<14> = Vtbl([
    query::<FontObject> as *const (),
    add_ref::<FontObject> as *const (),
    release::<FontObject> as *const (),
    font_family as *const (),
    font_weight as *const (),
    font_stretch as *const (),
    font_style as *const (),
    font_is_symbol as *const (),
    font_face_names as *const (),
    font_informational as *const (),
    font_simulations as *const (),
    font_metrics as *const (),
    font_has_character as *const (),
    font_create_face as *const (),
]);
impl Object for FontObject {
    const IIDS: &'static [Guid] = &[IID_FONT];
    fn vtbl() -> *const () {
        &FONT_VTBL as *const _ as *const ()
    }
}
pub(crate) fn new_font(collection: Arc<Collection>, family: usize, face: Arc<Face>) -> usize {
    new(FontObject {
        collection,
        family,
        face,
    })
}
extern "system" fn font_family(this: usize, out: usize) -> u32 {
    let f = data::<FontObject>(this);
    put_ptr(out, new_family(f.collection.clone(), f.family));
    S_OK
}
extern "system" fn font_weight(this: usize) -> u32 {
    data::<FontObject>(this).face.weight
}
extern "system" fn font_stretch(this: usize) -> u32 {
    data::<FontObject>(this).face.stretch
}
extern "system" fn font_style(this: usize) -> u32 {
    let face = &data::<FontObject>(this).face;
    if face.simulations & font::SIMULATION_OBLIQUE != 0 {
        font::STYLE_OBLIQUE
    } else {
        face.style
    }
}
extern "system" fn font_is_symbol(this: usize) -> i32 {
    i32::from(data::<FontObject>(this).face.is_symbol())
}
extern "system" fn font_face_names(this: usize, out: usize) -> u32 {
    put_ptr(out, new_strings(&data::<FontObject>(this).face.face_names));
    S_OK
}
extern "system" fn font_informational(this: usize, id: u32, out: usize, exists: usize) -> u32 {
    let items = data::<FontObject>(this).face.informational(id);
    put_u32(exists, u32::from(!items.is_empty()));
    put_ptr(
        out,
        if items.is_empty() {
            0
        } else {
            new_strings(&items)
        },
    );
    S_OK
}
extern "system" fn font_simulations(this: usize) -> u32 {
    data::<FontObject>(this).face.simulations
}
extern "system" fn font_metrics(this: usize, out: usize) {
    put_bytes(out, &data::<FontObject>(this).face.metrics().to_bytes());
}
extern "system" fn font_has_character(this: usize, code_point: u32, exists: usize) -> u32 {
    put_u32(
        exists,
        u32::from(data::<FontObject>(this).face.has(code_point)),
    );
    S_OK
}
extern "system" fn font_create_face(this: usize, out: usize) -> u32 {
    put_ptr(out, new(FaceObject(data::<FontObject>(this).face.clone())));
    S_OK
}

// ---- font faces --------------------------------------------------------------------------------
pub(crate) struct FaceObject(pub Arc<Face>);
static FACE_VTBL: Vtbl<18> = Vtbl([
    query::<FaceObject> as *const (),
    add_ref::<FaceObject> as *const (),
    release::<FaceObject> as *const (),
    face_type as *const (),
    face_files as *const (),
    face_index as *const (),
    face_simulations as *const (),
    face_is_symbol as *const (),
    face_metrics as *const (),
    face_glyph_count as *const (),
    face_design_glyph_metrics as *const (),
    face_glyph_indices as *const (),
    face_try_table as *const (),
    face_release_table as *const (),
    face_run_outline as *const (),
    face_recommended_mode as *const (),
    face_gdi_metrics as *const (),
    face_gdi_glyph_metrics as *const (),
]);
impl Object for FaceObject {
    const IIDS: &'static [Guid] = &[IID_FACE];
    fn vtbl() -> *const () {
        &FACE_VTBL as *const _ as *const ()
    }
}
/// IDWriteFactory::CreateFontFace: face `index` of the first file.
pub(crate) extern "system" fn create_font_face(
    _this: usize,
    _kind: u32,
    count: u32,
    files: usize,
    index: u32,
    simulations: u32,
    out: usize,
) -> u32 {
    put_ptr(out, 0);
    if count == 0 || files == 0 || out == 0 {
        return E_INVALIDARG;
    }
    let file = match files::file_of(get_ptr(files)) {
        Ok(f) => f,
        Err(e) => return e,
    };
    match font::face_at(&file, index, simulations & 3) {
        Some(face) => {
            put_ptr(out, new(FaceObject(Arc::new(face))));
            S_OK
        }
        None => DWRITE_E_FILEFORMAT,
    }
}
extern "system" fn face_type(this: usize) -> u32 {
    let face = &data::<FaceObject>(this).0;
    match font::analyze(face.file.bytes.get()) {
        // A face of a collection reports DWRITE_FONT_FACE_TYPE_OPENTYPE_COLLECTION.
        Some((_, kind, _)) => kind,
        None => 1,
    }
}
extern "system" fn face_files(this: usize, count: usize, files: usize) -> u32 {
    if count == 0 {
        return E_INVALIDARG;
    }
    if files != 0 {
        if get_u32(count) < 1 {
            return E_INVALIDARG;
        }
        put_ptr(
            files,
            files::new_file(data::<FaceObject>(this).0.file.clone()),
        );
    }
    put_u32(count, 1);
    S_OK
}
extern "system" fn face_index(this: usize) -> u32 {
    data::<FaceObject>(this).0.index
}
extern "system" fn face_simulations(this: usize) -> u32 {
    data::<FaceObject>(this).0.simulations
}
extern "system" fn face_is_symbol(this: usize) -> i32 {
    i32::from(data::<FaceObject>(this).0.is_symbol())
}
extern "system" fn face_metrics(this: usize, out: usize) {
    put_bytes(out, &data::<FaceObject>(this).0.metrics().to_bytes());
}
extern "system" fn face_glyph_count(this: usize) -> u32 {
    u32::from(data::<FaceObject>(this).0.glyph_count())
}
extern "system" fn face_design_glyph_metrics(
    this: usize,
    indices: usize,
    count: u32,
    out: usize,
    _sideways: i32,
) -> u32 {
    if (indices == 0 || out == 0) && count > 0 {
        return E_INVALIDARG;
    }
    let face = &data::<FaceObject>(this).0;
    for i in 0..count as usize {
        let m = face.glyph_metrics(get_u16(indices + 2 * i));
        put_bytes(out + 28 * i, &m.to_bytes());
    }
    S_OK
}
extern "system" fn face_glyph_indices(this: usize, points: usize, count: u32, out: usize) -> u32 {
    if (points == 0 || out == 0) && count > 0 {
        return E_INVALIDARG;
    }
    let face = &data::<FaceObject>(this).0;
    let Some(font) = face.font() else {
        return DWRITE_E_FILEFORMAT;
    };
    let charmap = skrifa::MetadataProvider::charmap(&font);
    for i in 0..count as usize {
        let glyph = charmap
            .map(get_u32(points + 4 * i))
            .map_or(0, |g| g.to_u32().min(0xFFFF) as u16);
        unsafe { ((out + 2 * i) as *mut u16).write_unaligned(glyph) };
    }
    S_OK
}
extern "system" fn face_try_table(
    this: usize,
    tag: u32,
    table: usize,
    size: usize,
    context: usize,
    exists: usize,
) -> u32 {
    let face = &data::<FaceObject>(this).0;
    let found = face.table(tag.to_le_bytes());
    put_ptr(table, found.map_or(0, |t| t.as_ptr() as usize));
    put_u32(size, found.map_or(0, |t| t.len() as u32));
    put_ptr(context, 0);
    put_u32(exists, u32::from(found.is_some()));
    S_OK
}
extern "system" fn face_release_table(_this: usize, _context: usize) {}
/// The glyphs of a run (DWRITE_GLYPH_OFFSET is two floats).
pub(crate) fn run_glyphs(
    face: &Face,
    em: f32,
    indices: usize,
    advances: usize,
    offsets: usize,
    count: u32,
) -> Vec<Glyph> {
    let units = f32::from(face.metrics().units_per_em.max(1));
    (0..count as usize)
        .map(|i| {
            let id = get_u16(indices + 2 * i);
            let advance = if advances != 0 {
                get_f32(advances + 4 * i)
            } else {
                face.glyph_metrics(id).advance_width as f32 * em / units
            };
            let (dx, dy) = if offsets != 0 {
                (get_f32(offsets + 8 * i), get_f32(offsets + 8 * i + 4))
            } else {
                (0.0, 0.0)
            };
            Glyph {
                id,
                advance,
                advance_offset: dx,
                ascender_offset: dy,
            }
        })
        .collect()
}
/// GetGlyphRunOutline: the run's figures into an ID2D1SimplifiedGeometrySink
/// (winding fill, lines and cubic Béziers; quadratics are raised).
#[allow(clippy::too_many_arguments)]
extern "system" fn face_run_outline(
    this: usize,
    em: f32,
    indices: usize,
    advances: usize,
    offsets: usize,
    count: u32,
    _sideways: i32,
    rtl: i32,
    sink: usize,
) -> u32 {
    if sink == 0 || (indices == 0 && count > 0) {
        return E_INVALIDARG;
    }
    let face = &data::<FaceObject>(this).0;
    let glyphs = run_glyphs(face, em, indices, advances, offsets, count);
    let commands = raster::run_outline(face, em, &glyphs, rtl != 0);
    emit(sink, &commands);
    S_OK
}
fn point(x: f32, y: f32) -> u64 {
    u64::from(x.to_bits()) | (u64::from(y.to_bits()) << 32)
}
fn emit(sink: usize, commands: &[Command]) {
    type Mode = extern "system" fn(usize, u32);
    type Begin = extern "system" fn(usize, u64, u32);
    type Add = extern "system" fn(usize, usize, u32);
    let set_fill: Mode = unsafe { core::mem::transmute(method(sink, 3)) };
    let begin: Begin = unsafe { core::mem::transmute(method(sink, 5)) };
    let lines: Add = unsafe { core::mem::transmute(method(sink, 6)) };
    let beziers: Add = unsafe { core::mem::transmute(method(sink, 7)) };
    let end: Mode = unsafe { core::mem::transmute(method(sink, 8)) };
    // D2D1_FILL_MODE_WINDING.
    set_fill(sink, 1);
    let mut open = false;
    let mut current = (0.0f32, 0.0f32);
    for c in commands {
        match *c {
            Command::Move(x, y) => {
                if open {
                    end(sink, 1);
                }
                // D2D1_FIGURE_BEGIN_FILLED.
                begin(sink, point(x, y), 0);
                open = true;
                current = (x, y);
            }
            Command::Line(x, y) => {
                let p = [x, y];
                lines(sink, p.as_ptr() as usize, 1);
                current = (x, y);
            }
            Command::Quad(qx, qy, x, y) => {
                let (x0, y0) = current;
                let b = [
                    x0 + 2.0 / 3.0 * (qx - x0),
                    y0 + 2.0 / 3.0 * (qy - y0),
                    x + 2.0 / 3.0 * (qx - x),
                    y + 2.0 / 3.0 * (qy - y),
                    x,
                    y,
                ];
                beziers(sink, b.as_ptr() as usize, 1);
                current = (x, y);
            }
            Command::Cubic(a, b, c, d, x, y) => {
                let s = [a, b, c, d, x, y];
                beziers(sink, s.as_ptr() as usize, 1);
                current = (x, y);
            }
            Command::Close => {
                if open {
                    // D2D1_FIGURE_END_CLOSED.
                    end(sink, 1);
                    open = false;
                }
            }
        }
    }
    if open {
        end(sink, 1);
    }
}
/// DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC (5), or OUTLINE (6) past 100
/// pixels per em.
extern "system" fn face_recommended_mode(
    _this: usize,
    em: f32,
    pixels_per_dip: f32,
    _measuring: u32,
    _params: usize,
    out: usize,
) -> u32 {
    put_u32(out, if em * pixels_per_dip > 100.0 { 6 } else { 5 });
    S_OK
}
extern "system" fn face_gdi_metrics(
    this: usize,
    _em: f32,
    _ppd: f32,
    _transform: usize,
    out: usize,
) -> u32 {
    face_metrics(this, out);
    S_OK
}
#[allow(clippy::too_many_arguments)]
extern "system" fn face_gdi_glyph_metrics(
    this: usize,
    _em: f32,
    _ppd: f32,
    _transform: usize,
    _natural: i32,
    indices: usize,
    count: u32,
    out: usize,
    sideways: i32,
) -> u32 {
    face_design_glyph_metrics(this, indices, count, out, sideways)
}
