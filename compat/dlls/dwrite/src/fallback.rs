//! System font fallback (IDWriteFontFallback::MapCharacters): the run of
//! text the requested family covers, else the first family in Windows'
//! fallback order (by the text's locale for Han characters) that covers
//! it.
use crate::font::{Collection, Face, matching};
use alloc::sync::Arc;

/// Characters that stay with the font of the character before them:
/// combining marks, variation selectors, joiners.
fn continues(c: u32) -> bool {
    matches!(c,
        0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF
        | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F | 0x200C | 0x200D | 0xE0100..=0xE01EF
        | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F)
}
/// Characters any font may render as nothing: controls and spaces.
fn neutral(c: u32) -> bool {
    c < 0x20 || (0x7F..0xA0).contains(&c) || c == 0x20 || c == 0x200B || c == 0xFEFF
}
/// Fallback families for a locale, in order.
fn order(locale: &str) -> [&'static str; 14] {
    let l = locale.to_ascii_lowercase();
    let han: [&str; 4] = if l.starts_with("ja") {
        [
            "Yu Gothic",
            "Microsoft YaHei",
            "Microsoft JhengHei",
            "Malgun Gothic",
        ]
    } else if l.starts_with("ko") {
        [
            "Malgun Gothic",
            "Microsoft YaHei",
            "Yu Gothic",
            "Microsoft JhengHei",
        ]
    } else if l.starts_with("zh-tw")
        || l.starts_with("zh-hk")
        || l.starts_with("zh-hant")
        || l.starts_with("zh-mo")
    {
        [
            "Microsoft JhengHei",
            "Microsoft YaHei",
            "Yu Gothic",
            "Malgun Gothic",
        ]
    } else {
        [
            "Microsoft YaHei",
            "Yu Gothic",
            "Microsoft JhengHei",
            "Malgun Gothic",
        ]
    };
    [
        "Segoe UI",
        "Segoe UI Variable",
        "Segoe UI Symbol",
        "Segoe UI Emoji",
        "Nirmala UI",
        han[0],
        han[1],
        han[2],
        han[3],
        "Segoe UI Historic",
        "Cascadia Mono",
        "Segoe Fluent Icons",
        "Segoe MDL2 Assets",
        "Segoe UI",
    ]
}
/// The font of `family` best matching the request.
pub fn best(
    collection: &Collection,
    family: usize,
    weight: u32,
    style: u32,
    stretch: u32,
) -> Option<(Arc<Face>, u32)> {
    let fonts = &collection.families.get(family)?.fonts;
    let (i, simulations) = *matching(fonts, weight, stretch, style).first()?;
    Some((fonts[i].clone(), simulations))
}
/// The decoded code points of UTF-16 text with each one's length in units.
fn points(text: &[u16]) -> impl Iterator<Item = (u32, usize)> + '_ {
    let mut i = 0;
    core::iter::from_fn(move || {
        let u = *text.get(i)?;
        let (c, n) = if (0xD800..0xDC00).contains(&u)
            && let Some(l) = text.get(i + 1).filter(|l| (0xDC00..0xE000).contains(*l))
        {
            (
                0x10000 + ((u32::from(u) - 0xD800) << 10) + (u32::from(*l) - 0xDC00),
                2,
            )
        } else {
            (u32::from(u), 1)
        };
        i += n;
        Some((c, n))
    })
}
/// MapCharacters: how many units of `text` map to the returned font (None:
/// no font covers them).
pub fn map(
    collection: &Collection,
    text: &[u16],
    base_family: Option<&str>,
    weight: u32,
    style: u32,
    stretch: u32,
    locale: &str,
) -> (usize, Option<(Arc<Face>, u32)>) {
    let base = base_family
        .and_then(|n| collection.find(n))
        .and_then(|f| best(collection, f, weight, style, stretch));
    map_from(collection, text, base, weight, style, stretch, locale)
}
/// MapCharacters with the requested font already resolved (from any
/// collection); fallback fonts come from `collection`.
pub fn map_from(
    collection: &Collection,
    text: &[u16],
    base: Option<(Arc<Face>, u32)>,
    weight: u32,
    style: u32,
    stretch: u32,
    locale: &str,
) -> (usize, Option<(Arc<Face>, u32)>) {
    let covers =
        |font: &Option<(Arc<Face>, u32)>, c: u32| font.as_ref().is_some_and(|(f, _)| f.has(c));
    let mut iter = points(text).peekable();
    let Some(&(first, _)) = iter.peek() else {
        return (0, base);
    };
    // The requested font covers the text (spaces and controls go with it).
    if base.is_some() && (covers(&base, first) || neutral(first) || continues(first)) {
        let mut length = 0;
        for (c, n) in iter {
            if covers(&base, c) || neutral(c) || continues(c) {
                length += n;
            } else {
                break;
            }
        }
        return (length, base);
    }
    let chosen = order(locale).iter().find_map(|name| {
        let family = collection.find(name)?;
        let font = best(collection, family, weight, style, stretch)?;
        font.0.has(first).then_some(font)
    });
    let mut length = 0;
    for (i, (c, n)) in points(text).enumerate() {
        let take = if i == 0 || continues(c) || neutral(c) {
            true
        } else if covers(&base, c) {
            false
        } else {
            match &chosen {
                Some(_) => covers(&chosen, c),
                // Uncovered text continues until some font covers it.
                None => !order(locale).iter().any(|name| {
                    collection
                        .find(name)
                        .and_then(|f| best(collection, f, weight, style, stretch))
                        .is_some_and(|(f, _)| f.has(c))
                }),
            }
        };
        if !take {
            break;
        }
        length += n;
    }
    (length, chosen)
}
