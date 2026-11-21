//! Original immutable offline atlas format. All integers are little-endian.
use super::*;
pub const MAGIC: &[u8; 8] = b"DOORFNT1";
pub const HEADER_BYTES: usize = 24;
pub const STYLE_BYTES: usize = 24;
pub const GLYPH_BYTES: usize = 16;
pub const PAIR_BYTES: usize = 6;
fn u16_at(bytes: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        bytes
            .get(at..at + 2)
            .ok_or(Error::InvalidFont)?
            .try_into()
            .map_err(|_| Error::InvalidFont)?,
    ))
}
fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(at..at + 4)
            .ok_or(Error::InvalidFont)?
            .try_into()
            .map_err(|_| Error::InvalidFont)?,
    ))
}
/// Validate the complete RGB-only stream, exact pixel count and end marker.
/// This also rejects overlong runs and trailing bytes before the decoder allocates.
fn rgb_stream(bytes: &[u8], pixels: usize) -> Result<()> {
    let end = bytes.len().checked_sub(8).ok_or(Error::InvalidFont)?;
    if bytes.get(end..) != Some(&[0, 0, 0, 0, 0, 0, 0, 1]) {
        return Err(Error::InvalidFont);
    }
    let mut at = 14usize;
    let mut produced = 0usize;
    while produced < pixels {
        let tag = *bytes
            .get(at)
            .filter(|_| at < end)
            .ok_or(Error::InvalidFont)?;
        let (length, count) = match tag {
            0xfe => (4, 1),
            0xff => return Err(Error::InvalidFont),
            0xc0..=0xfd => (1, (tag as usize & 63) + 1),
            0x80..=0xbf => (2, 1),
            _ => (1, 1),
        };
        at = at.checked_add(length).ok_or(Error::InvalidFont)?;
        produced = produced.checked_add(count).ok_or(Error::InvalidFont)?;
        if at > end || produced > pixels {
            return Err(Error::InvalidFont);
        }
    }
    if at != end {
        return Err(Error::InvalidFont);
    }
    Ok(())
}
/// Header byte 8 is the integer factor (1..=3) with byte 11 zero; fractional
/// atlases carry 0 in byte 8 and `percent / 25` in byte 11 (5, 6, 7, 9, 10).
pub fn scale_bytes(percent: u32) -> [u8; 2] {
    if percent.is_multiple_of(100) {
        [(percent / 100) as u8, 0]
    } else {
        [0, (percent / 25) as u8]
    }
}
pub(super) fn decode(bytes: &[u8], percent: u32) -> Result<FontAtlas> {
    let [factor_byte, fraction_byte] = scale_bytes(percent);
    if !valid_percent(percent)
        || bytes.get(..8) != Some(MAGIC.as_slice())
        || bytes.get(8..12) != Some(&[factor_byte, TEXT_STYLES.len() as u8, 3, fraction_byte])
        || u32_at(bytes, 20)? as usize != bytes.len()
    {
        return Err(Error::InvalidFont);
    }
    let payload_at = u32_at(bytes, 12)? as usize;
    let coverage_len = u32_at(bytes, 16)? as usize;
    if coverage_len == 0
        || !coverage_len.is_multiple_of(3)
        || coverage_len > atlas_capacity(percent) * 3
    {
        return Err(Error::AtlasFull);
    }
    let mut cursor = HEADER_BYTES + TEXT_STYLES.len() * STYLE_BYTES;
    let mut coverage_end = 0usize;
    let mut fonts = Vec::with_capacity(TEXT_STYLES.len());
    for (index, style) in TEXT_STYLES.iter().enumerate() {
        let header = HEADER_BYTES + index * STYLE_BYTES;
        let metrics = FontMetrics {
            pixel_size: u32::from(u16_at(bytes, header)?),
            line_height: u32::from(u16_at(bytes, header + 2)?),
            baseline: u32::from(u16_at(bytes, header + 4)?),
            descent: i32::from(u16_at(bytes, header + 6)? as i16),
        };
        let pairs = u16_at(bytes, header + 16)? as usize;
        if metrics.pixel_size != scaled_len(percent, style.pixel_size())
            || metrics.line_height != scaled_len(percent, style.line_height())
            || metrics.baseline == 0
            || metrics.baseline > metrics.line_height
            || metrics.descent > 0
            || metrics.descent < -(glyph_bound(percent) as i32)
            || u32_at(bytes, header + 8)? as usize != cursor
            || pairs > ASCII_COUNT * ASCII_COUNT
            || u16_at(bytes, header + 18)? != 0
            || u32_at(bytes, header + 20)? != 0
        {
            return Err(Error::InvalidFont);
        }
        let mut glyphs = [CachedGlyph::default(); ASCII_COUNT];
        for (index, glyph) in glyphs.iter_mut().enumerate() {
            let record = cursor + index * GLYPH_BYTES;
            *glyph = CachedGlyph {
                offset: u32_at(bytes, record)? as usize,
                width: u16_at(bytes, record + 4)?,
                height: u16_at(bytes, record + 6)?,
                left: u16_at(bytes, record + 8)? as i16,
                top: u16_at(bytes, record + 10)? as i16,
                advance: u32_at(bytes, record + 12)? as i32,
            };
            let extent = usize::from(glyph.width) * usize::from(glyph.height) * 3;
            if glyph.offset != coverage_end
                || u32::from(glyph.width) > glyph_bound(percent)
                || u32::from(glyph.height) > glyph_bound(percent)
                || (glyph.width == 0) != (glyph.height == 0)
                || i32::from(glyph.left).unsigned_abs() > glyph_bound(percent)
                || i32::from(glyph.top).unsigned_abs() > glyph_bound(percent)
                || glyph.advance <= 0
                || glyph.advance.unsigned_abs() > 2 * glyph_bound(percent) * 64
                || index == 0 && (extent != 0 || glyph.left != 0 || glyph.top != 0)
                || index != 0 && extent == 0
            {
                return Err(Error::InvalidFont);
            }
            coverage_end = coverage_end.checked_add(extent).ok_or(Error::InvalidFont)?;
            if coverage_end > coverage_len {
                return Err(Error::InvalidFont);
            }
        }
        cursor += ASCII_COUNT * GLYPH_BYTES;
        if u32_at(bytes, header + 12)? as usize != cursor {
            return Err(Error::InvalidFont);
        }
        let mut kerning = Vec::with_capacity(pairs);
        for index in 0..pairs {
            let record = cursor + index * PAIR_BYTES;
            let key = u16_at(bytes, record)?;
            let delta = u32_at(bytes, record + 2)? as i32;
            if key as usize >= ASCII_COUNT * ASCII_COUNT
                || kerning.last().is_some_and(|&(previous, _)| key <= previous)
                || delta == 0
                || delta.unsigned_abs() > glyph_bound(percent) * 64
            {
                return Err(Error::InvalidFont);
            }
            kerning.push((key, delta));
        }
        cursor += pairs * PAIR_BYTES;
        fonts.push(CachedFont {
            kerning: Kerning::Pairs(kerning),
            glyphs,
            metrics,
        });
    }
    if cursor != payload_at || coverage_end != coverage_len {
        return Err(Error::InvalidFont);
    }
    let payload = bytes.get(payload_at..).ok_or(Error::InvalidFont)?;
    let header = qoi::decode_header(payload).map_err(|_| Error::InvalidFont)?;
    if header.channels != qoi::Channels::Rgb
        || header.width as usize != coverage_len / 3
        || header.height != 1
    {
        return Err(Error::InvalidFont);
    }
    rgb_stream(payload, coverage_len / 3)?;
    let (_, coverage) = qoi::decode_to_vec(payload).map_err(|_| Error::InvalidFont)?;
    if coverage.len() != coverage_len {
        return Err(Error::InvalidFont);
    }
    for font in &fonts {
        for glyph in &font.glyphs[1..] {
            let extent = usize::from(glyph.width) * usize::from(glyph.height) * 3;
            if !coverage[glyph.offset..glyph.offset + extent]
                .iter()
                .any(|&value| value != 0)
            {
                return Err(Error::InvalidFont);
            }
        }
    }
    Ok(FontAtlas {
        fonts: fonts.try_into().map_err(|_| Error::InvalidFont)?,
        percent,
        text: 100,
        coverage,
        channels: 3,
        unicode: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const CACHES: [&[u8]; 3] = [
        include_bytes!("../../../../assets/fonts/segoe-ui/hinted-1x.dfa"),
        include_bytes!("../../../../assets/fonts/segoe-ui/hinted-2x.dfa"),
        include_bytes!("../../../../assets/fonts/segoe-ui/hinted-3x.dfa"),
    ];
    fn set32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn with_pairs(pairs: &[(u16, i32)]) -> Vec<u8> {
        let mut bytes = CACHES[0].to_vec();
        assert_eq!(u16_at(&bytes, HEADER_BYTES + 16), Ok(0));
        let insert = u32_at(&bytes, HEADER_BYTES + 12).unwrap() as usize;
        let encoded: Vec<_> = pairs
            .iter()
            .flat_map(|(key, delta)| key.to_le_bytes().into_iter().chain(delta.to_le_bytes()))
            .collect();
        let added = encoded.len() as u32;
        bytes.splice(insert..insert, encoded);
        bytes[HEADER_BYTES + 16..HEADER_BYTES + 18]
            .copy_from_slice(&(pairs.len() as u16).to_le_bytes());
        for index in 1..TEXT_STYLES.len() {
            for field in [8, 12] {
                let at = HEADER_BYTES + index * STYLE_BYTES + field;
                let before = u32_at(&bytes, at).unwrap();
                set32(&mut bytes, at, before + added);
            }
        }
        let before = u32_at(&bytes, 12).unwrap();
        set32(&mut bytes, 12, before + added);
        let length = bytes.len() as u32;
        set32(&mut bytes, 20, length);
        bytes
    }
    #[test]
    fn packed_caches_have_real_ink_and_do_not_grow_during_draws() {
        for (index, bytes) in CACHES.iter().enumerate() {
            let factor = index as u32 + 1;
            let atlas = FontAtlas::from_packed(bytes, factor).unwrap();
            let size = atlas.cache_bytes();
            let capacity = atlas.cache_capacity();
            assert!(size > 0 && size <= MAX_ATLAS_BYTES * factor as usize * factor as usize * 3);
            for style in TEXT_STYLES {
                let mut pixels = alloc::vec![255;600*100*4];
                let mut target = Target::new(&mut pixels, 600, 100, 2400)
                    .unwrap()
                    .cleartype();
                let clip = target.bounds();
                let sample = "AVATAR To Wi notes.txt";
                let measured = atlas.measure(sample, style).unwrap();
                let drawn = atlas
                    .draw(
                        &mut target,
                        clip,
                        crate::TextRun {
                            text: sample,
                            x: 1,
                            top: 1,
                            style,
                            color: [24, 24, 24, 255],
                        },
                    )
                    .unwrap();
                assert_eq!(drawn.advance_width, measured.advance_width);
                assert!(drawn.painted_pixels > 0);
                assert!(pixels.chunks_exact(4).any(|p| p[0] != p[1] || p[1] != p[2]));
            }
            for _ in 0..100 {
                atlas
                    .measure("Repeated UI labels", TextStyle::Regular14)
                    .unwrap();
            }
            assert_eq!(
                (atlas.cache_bytes(), atlas.cache_capacity()),
                (size, capacity)
            );
        }
    }
    #[test]
    fn malformed_metadata_and_qoi_fail_before_an_atlas_is_published() {
        let original = CACHES[0];
        for factor in [0, 2, 4, u32::MAX] {
            assert!(FontAtlas::from_packed(original, factor).is_err());
        }
        for length in [0, 7, 8, 12, 23, 167, 1000, original.len() - 1] {
            let mut bad = original[..length].to_vec();
            if length >= 24 {
                set32(&mut bad, 20, length as u32);
            }
            assert!(FontAtlas::from_packed(&bad, 1).is_err(), "length {length}");
        }
        let glyph = u32_at(original, HEADER_BYTES + 8).unwrap() as usize;
        let payload = u32_at(original, 12).unwrap() as usize;
        for (at, value) in [
            (0, u32::MAX),
            (8, u32::MAX),
            (12, u32::MAX),
            (16, u32::MAX),
            (20, 0),
            (HEADER_BYTES, 0),
            (HEADER_BYTES + 4, 0),
            (HEADER_BYTES + 8, u32::MAX),
            (HEADER_BYTES + 12, u32::MAX),
            (HEADER_BYTES + 16, u32::MAX),
            (HEADER_BYTES + 20, 1),
            (glyph, u32::MAX),
            (glyph + 12, i32::MIN as u32),
            (glyph + GLYPH_BYTES + 4, 0),
            (glyph + GLYPH_BYTES + 8, 0x80008000),
        ] {
            let mut bad = original.to_vec();
            set32(&mut bad, at, value);
            assert!(FontAtlas::from_packed(&bad, 1).is_err(), "field {at}");
        }
        for relative in [4, 8, 12, 13] {
            let mut bad = original.to_vec();
            bad[payload + relative] = 255;
            assert!(FontAtlas::from_packed(&bad, 1).is_err(), "QOI {relative}");
        }
        let mut bad = original.to_vec();
        let end = bad.len() - 8;
        bad.insert(end, 0xc0);
        let length = bad.len() as u32;
        set32(&mut bad, 20, length);
        assert!(FontAtlas::from_packed(&bad, 1).is_err());
    }
    #[test]
    fn pair_tables_are_sorted_bounded_and_used_by_measurement() {
        let key = u16::from(b'A' - 32) * 95 + u16::from(b'V' - 32);
        let original = FontAtlas::from_packed(CACHES[0], 1).unwrap();
        let bytes = with_pairs(&[(key, -64)]);
        let kerned = FontAtlas::from_packed(&bytes, 1).unwrap();
        assert_eq!(
            kerned
                .measure("AV", TextStyle::Regular14)
                .unwrap()
                .advance_width
                + 1,
            original
                .measure("AV", TextStyle::Regular14)
                .unwrap()
                .advance_width
        );
        for pairs in [
            alloc::vec![(key, i32::MIN)],
            alloc::vec![(key, 0)],
            alloc::vec![(9025, 1)],
            alloc::vec![(key, -1), (key, -2)],
            alloc::vec![(key + 1, -1), (key, -2)],
        ] {
            assert!(FontAtlas::from_packed(&with_pairs(&pairs), 1).is_err());
        }
    }
    #[test]
    fn rgb_stream_requires_exact_runs_and_no_trailing_data() {
        let valid = qoi::encode_to_vec([24u8, 64, 128], 1, 1).unwrap();
        assert!(rgb_stream(&valid, 1).is_ok());
        let mut overrun = alloc::vec![0;14];
        overrun.push(0xc1);
        overrun.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
        assert!(rgb_stream(&overrun, 1).is_err());
        assert!(rgb_stream(&overrun, 2).is_ok());
        assert!(rgb_stream(&valid[..valid.len() - 1], 1).is_err());
        let mut trailing = valid.clone();
        trailing.push(1);
        assert!(rgb_stream(&trailing, 1).is_err());
    }
}
