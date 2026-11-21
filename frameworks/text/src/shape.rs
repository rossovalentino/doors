//! HarfRust shaping with bounded plan and shaped-run caches.
//!
//! Shaped runs are stored in font units, so one entry serves every size and
//! scale. The key holds the run text and its shaping context, face identity,
//! variation instance, script, language, direction and paragraph-edge flags
//! (§52: text, script, language, direction, font/fallback configuration and
//! features; no user features are exposed yet).
use crate::font::{Face, Locale};
use alloc::{boxed::Box, collections::BTreeMap, vec::Vec};
use harfrust::{Buffer, BufferFlags, Direction, ShapeOptions, ShapePlan, ShaperFont};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ShapedGlyph {
    pub glyph: u32,
    /// UTF-8 byte offset of the cluster, relative to the run start.
    pub cluster: u32,
    pub advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
    pub unsafe_to_break: bool,
}

pub(crate) struct ShapeRequest<'a> {
    pub face: usize,
    pub fingerprint: u64,
    pub instance: u64,
    pub text: &'a str,
    pub pre: &'a str,
    pub post: &'a str,
    pub rtl: bool,
    pub script: Option<harfrust::Script>,
    pub locale: Locale,
    pub bot: bool,
    pub eot: bool,
}

struct Entry {
    key: Box<str>,
    meta: (u64, u64, u64, bool, u32, Locale, bool, bool),
    glyphs: Box<[ShapedGlyph]>,
    used: u64,
}
impl Entry {
    fn cost(&self) -> usize {
        self.key.len() + self.glyphs.len() * core::mem::size_of::<ShapedGlyph>() + 96
    }
}

pub(crate) struct ShapeCache {
    entries: BTreeMap<u64, Entry>,
    plans: Vec<(u64, u64, u32, bool, Locale, ShapePlan, u64)>,
    bytes: usize,
    pub budget: usize,
    clock: u64,
    pub hits: u64,
    pub misses: u64,
    buffer: Option<Buffer>,
}

const MAX_PLANS: usize = 48;

fn hash(bytes: &[u8], mut state: u64) -> u64 {
    for byte in bytes {
        state ^= u64::from(*byte);
        state = state.wrapping_mul(0x0100_0000_01b3);
    }
    state
}

impl ShapeCache {
    pub fn new(budget: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            plans: Vec::new(),
            bytes: 0,
            budget,
            clock: 0,
            hits: 0,
            misses: 0,
            buffer: Some(Buffer::new()),
        }
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.plans.clear();
        self.bytes = 0;
    }

    /// Shape (or fetch) one run. `font` is the harfrust font instance for the
    /// request's face and variation location.
    pub fn shape(
        &mut self,
        face: &Face,
        font: &harfrust::Font,
        request: &ShapeRequest<'_>,
    ) -> &[ShapedGlyph] {
        let script_tag = request
            .script
            .map_or(0, |s| u32::from_be_bytes(s.tag().to_be_bytes()));
        let meta = (
            request.fingerprint,
            request.face as u64,
            request.instance,
            request.rtl,
            script_tag,
            request.locale,
            request.bot,
            request.eot,
        );
        let mut h = hash(request.pre.as_bytes(), 0xcbf2_9ce4_8422_2325);
        h = hash(&[0xff], h);
        h = hash(request.text.as_bytes(), h);
        h = hash(&[0xfe], h);
        h = hash(request.post.as_bytes(), h);
        h = hash(&request.fingerprint.to_le_bytes(), h);
        h = hash(&request.instance.to_le_bytes(), h);
        h = hash(
            &[request.rtl as u8, request.bot as u8, request.eot as u8],
            h,
        );
        h = hash(&script_tag.to_le_bytes(), h);
        h = hash(request.locale.as_str().as_bytes(), h);
        self.clock += 1;
        let key_matches = |entry: &Entry| {
            entry.meta == meta && {
                let k = &entry.key;
                let (pre, text, post) = (request.pre.len(), request.text.len(), request.post.len());
                k.len() == pre + text + post
                    && &k[..pre] == request.pre
                    && &k[pre..pre + text] == request.text
                    && &k[pre + text..] == request.post
            }
        };
        if self.entries.get(&h).is_some_and(key_matches) {
            self.hits += 1;
            let entry = self.entries.get_mut(&h).expect("present");
            entry.used = self.clock;
            return &self.entries[&h].glyphs;
        }
        self.misses += 1;
        let glyphs = self.run(face, font, request, script_tag);
        let mut key = alloc::string::String::with_capacity(
            request.pre.len() + request.text.len() + request.post.len(),
        );
        key.push_str(request.pre);
        key.push_str(request.text);
        key.push_str(request.post);
        let entry = Entry {
            key: key.into_boxed_str(),
            meta,
            glyphs: glyphs.into_boxed_slice(),
            used: self.clock,
        };
        let cost = entry.cost();
        if self.bytes + cost > self.budget {
            self.evict(self.budget.saturating_sub(cost) * 3 / 4);
        }
        if let Some(old) = self.entries.insert(h, entry) {
            self.bytes -= old.cost();
        }
        self.bytes += cost;
        &self.entries[&h].glyphs
    }

    fn evict(&mut self, target: usize) {
        let mut order: Vec<(u64, u64)> = self.entries.iter().map(|(k, e)| (e.used, *k)).collect();
        order.sort_unstable();
        for (_, key) in order {
            if self.bytes <= target {
                break;
            }
            if let Some(entry) = self.entries.remove(&key) {
                self.bytes -= entry.cost();
            }
        }
    }

    fn run(
        &mut self,
        face: &Face,
        font: &harfrust::Font,
        request: &ShapeRequest<'_>,
        script_tag: u32,
    ) -> Vec<ShapedGlyph> {
        let direction = if request.rtl {
            Direction::RightToLeft
        } else {
            Direction::LeftToRight
        };
        let language = request.locale.harfrust();
        self.clock += 1;
        let plan_index = match self.plans.iter().position(|p| {
            p.0 == request.fingerprint
                && p.1 == request.instance
                && p.2 == script_tag
                && p.3 == request.rtl
                && p.4 == request.locale
        }) {
            Some(index) => index,
            None => {
                let plan = ShapePlan::new(font, direction, request.script, language.as_ref(), &[]);
                if self.plans.len() >= MAX_PLANS
                    && let Some(oldest) = self
                        .plans
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, p)| p.6)
                        .map(|(i, _)| i)
                {
                    self.plans.swap_remove(oldest);
                }
                self.plans.push((
                    request.fingerprint,
                    request.instance,
                    script_tag,
                    request.rtl,
                    request.locale,
                    plan,
                    self.clock,
                ));
                self.plans.len() - 1
            }
        };
        self.plans[plan_index].6 = self.clock;
        let mut buffer = self.buffer.take().unwrap_or_default();
        buffer.clear();
        buffer.push_str(request.text);
        buffer.set_pre_context(request.pre);
        buffer.set_post_context(request.post);
        buffer.set_direction(direction);
        buffer.set_script(request.script);
        buffer.set_language(language);
        let mut flags = BufferFlags::empty();
        if request.bot {
            flags |= BufferFlags::BEGINNING_OF_TEXT;
        }
        if request.eot {
            flags |= BufferFlags::END_OF_TEXT;
        }
        buffer.set_flags(flags);
        let shaper = ShaperFont::new(font);
        let options = ShapeOptions::new().plan(Some(&self.plans[plan_index].5));
        let mut out = Vec::new();
        if harfrust::shape(&shaper, &mut buffer, options).is_ok() {
            out.reserve(buffer.len());
            for (info, position) in buffer.glyph_infos().iter().zip(buffer.glyph_positions()) {
                out.push(ShapedGlyph {
                    glyph: info.glyph_id,
                    cluster: info.cluster,
                    advance: position.x_advance,
                    x_offset: position.x_offset,
                    y_offset: position.y_offset,
                    unsafe_to_break: info.unsafe_to_break(),
                });
            }
        } else {
            // Malformed layout tables: nominal glyphs and advances, no features.
            for (index, ch) in request.text.char_indices() {
                let glyph = face.glyph(ch).unwrap_or(0);
                out.push(ShapedGlyph {
                    glyph,
                    cluster: index as u32,
                    advance: shaper.glyph_h_advance(harfrust::GlyphId::new(glyph)),
                    x_offset: 0,
                    y_offset: 0,
                    unsafe_to_break: false,
                });
            }
            if request.rtl {
                out.reverse();
            }
        }
        self.buffer = Some(buffer);
        out
    }
}
