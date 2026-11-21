//! Fonts: TrueType/OpenType (and the first font of a collection): names,
//! glyph count, outline type and weight. WOFF/WOFF2 tables are compressed,
//! so only their container facts are shown.
use super::Section::{Font, Origin};
use super::*;

fn weight(w: u16) -> String {
    let name = match w {
        100 => "Thin",
        200 => "Extra Light",
        300 => "Light",
        400 => "Regular",
        500 => "Medium",
        600 => "Semibold",
        700 => "Bold",
        800 => "Extra Bold",
        900 => "Black",
        _ => "",
    };
    if name.is_empty() {
        format!("{w}")
    } else {
        format!("{w} ({name})")
    }
}

pub(super) fn sfnt(d: &Sparse, r: &mut Report) -> Result<(), Miss> {
    let tag = d.get(0, 4)?;
    let mut base = 0u64;
    match tag {
        b"wOFF" | b"wOF2" => {
            r.format(if tag == b"wOFF" {
                "Web font (WOFF)"
            } else {
                "Web font (WOFF2)"
            });
            let h = d.get(0, 14)?;
            let flavor = &h[4..8];
            r.add(
                Font,
                "Font type",
                if flavor == b"OTTO" {
                    "OpenType (PostScript outlines)"
                } else {
                    "TrueType"
                },
            );
            r.add(Font, "Tables", grouped(u64::from(be16(h, 12).unwrap_or(0))));
            return Ok(());
        }
        b"ttcf" => {
            let h = d.get(0, 16)?;
            let fonts = be32(h, 8).unwrap_or(0);
            r.format("Font collection");
            r.add(Font, "Fonts in collection", grouped(u64::from(fonts)));
            base = u64::from(be32(h, 12).unwrap_or(0));
        }
        _ => (),
    }
    let version = d.get(base, 4)?;
    let postscript = version == b"OTTO";
    r.format(if postscript {
        "OpenType font"
    } else {
        "TrueType font"
    });
    r.add(
        Font,
        "Font type",
        if postscript {
            "OpenType (PostScript outlines)"
        } else {
            "TrueType"
        },
    );
    let count = u64::from(d.be16(base + 4)?).min(256);
    let records = d.get(base + 12, count * 16)?.to_vec();
    let table = |name: &[u8]| {
        records.chunks_exact(16).find(|t| &t[..4] == name).map(|t| {
            (
                u64::from(be32(t, 8).unwrap_or(0)),
                u64::from(be32(t, 12).unwrap_or(0)),
            )
        })
    };
    if let Some((at, _)) = table(b"maxp").filter(|(_, len)| *len >= 6) {
        r.add(Font, "Glyphs", grouped(u64::from(d.be16(at + 4)?)));
    }
    if let Some((at, _)) = table(b"OS/2").filter(|(_, len)| *len >= 6) {
        r.add(Font, "Weight", weight(d.be16(at + 4)?));
    }
    let Some((at, len)) = table(b"name").filter(|(_, len)| *len >= 6) else {
        return Ok(());
    };
    let h = d.get(at, 6)?;
    let count = u64::from(be16(h, 2).unwrap_or(0)).min(512);
    let strings = at + u64::from(be16(h, 4).unwrap_or(0));
    let list = d
        .get(at + 6, (count * 12).min(len.saturating_sub(6)))?
        .to_vec();
    // Best record per name ID: Windows English, any Windows, Unicode, Mac Roman.
    let rank = |platform: u16, language: u16| match (platform, language) {
        (3, 0x409) => 4,
        (3, _) => 3,
        (0, _) => 2,
        (1, 0) => 1,
        _ => 0,
    };
    let mut best: [(u8, u64, u64, bool); 8] = [(0, 0, 0, false); 8];
    let ids = [1u16, 2, 4, 5, 0, 16, 17, 7];
    for rec in list.chunks_exact(12) {
        let (platform, language, id) = (
            be16(rec, 0).unwrap_or(0),
            be16(rec, 4).unwrap_or(0),
            be16(rec, 6).unwrap_or(0),
        );
        let Some(slot) = ids.iter().position(|i| *i == id) else {
            continue;
        };
        let score = rank(platform, language);
        if score > best[slot].0 {
            best[slot] = (
                score,
                u64::from(be16(rec, 10).unwrap_or(0)),
                u64::from(be16(rec, 8).unwrap_or(0)),
                platform != 1,
            );
        }
    }
    let name = |slot: usize| -> Result<String, Miss> {
        let (score, offset, len, wide) = best[slot];
        if score == 0 {
            return Ok(String::new());
        }
        let b = d.get(strings + offset, len.min(512))?;
        Ok(if wide { utf16(b, false) } else { latin1(b) })
    };
    let family = name(5)?;
    r.add(
        Font,
        "Font family",
        if family.is_empty() { name(0)? } else { family },
    );
    let style = name(6)?;
    r.add(
        Font,
        "Font style",
        if style.is_empty() { name(1)? } else { style },
    );
    r.add(Font, "Font name", name(2)?);
    r.add(Font, "Font version", name(3)?);
    r.add(Origin, "Copyright", name(4)?);
    r.add(Origin, "Trademarks", name(7)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    fn font() -> Vec<u8> {
        let names = [
            (1u16, "Doors Sans"),
            (2, "Bold"),
            (4, "Doors Sans Bold"),
            (5, "Version 1.002"),
        ];
        let mut strings: Vec<u8> = Vec::new();
        let mut records = Vec::new();
        for (id, s) in names {
            let utf: Vec<u8> = s.encode_utf16().flat_map(u16::to_be_bytes).collect();
            records.extend(
                [
                    be(3, 2),
                    be(1, 2),
                    be(0x409, 2),
                    be(u64::from(id), 2),
                    be(utf.len() as u64, 2),
                    be(strings.len() as u64, 2),
                ]
                .concat(),
            );
            strings.extend(utf);
        }
        let name = [
            be(0, 2),
            be(names.len() as u64, 2),
            be(6 + records.len() as u64, 2),
            records,
            strings,
        ]
        .concat();
        let maxp = [be(0x5000, 4), be(1234, 2)].concat();
        let os2 = [be(4, 2), be(500, 2), be(700, 2)].concat();
        let tables = [(b"OS/2", os2), (b"maxp", maxp), (b"name", name)];
        let mut out = [
            be(0x0001_0000, 4),
            be(tables.len() as u64, 2),
            alloc::vec![0; 6],
        ]
        .concat();
        let mut offset = 12 + 16 * tables.len();
        let mut body: Vec<u8> = Vec::new();
        for (tag, t) in &tables {
            out.extend(tag.as_slice());
            out.extend([0; 4]);
            out.extend(be(offset as u64, 4));
            out.extend(be(t.len() as u64, 4));
            offset += t.len();
            body.extend(t);
        }
        out.extend(body);
        out
    }
    #[test]
    fn truetype_names_glyphs_and_weight() {
        let r = run("DoorsSans-Bold.ttf", &font());
        assert_eq!(r.format, Some("TrueType font"));
        assert_eq!(
            (
                r.get("Font family"),
                r.get("Font style"),
                r.get("Font name")
            ),
            (Some("Doors Sans"), Some("Bold"), Some("Doors Sans Bold"))
        );
        assert_eq!(
            (r.get("Glyphs"), r.get("Weight"), r.get("Font version")),
            (Some("1,234"), Some("700 (Bold)"), Some("Version 1.002"))
        );
        fuzz("DoorsSans-Bold.ttf", &font());
    }
}
