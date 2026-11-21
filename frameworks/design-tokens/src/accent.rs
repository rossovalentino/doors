//! System accent colour: the seven-shade palette Windows derives from one
//! accent colour (registry `AccentPalette`, Light3..Dark3), the Settings >
//! Personalization > Colors swatches, and "Automatic" selection from the
//! desktop background. Integer arithmetic only (the guest has no floating
//! point); colours are opaque BGRA like every other token.
//!
//! Roles (WinUI theme resources): dark-theme accent fills (toggles, selected
//! navigation indicators, accent buttons, the taskbar's active-app pill,
//! focus underlines) use Light2, light-theme fills use Dark1; accent text
//! uses Light3 (dark) and Dark2 (light). On the owner's PC the dark fill
//! measures #DB9EE5, the palette's Light2 (`reports/artifacts/reference-26h2/
//! environment.txt`).
use crate::{Color, rgb};

/// Light3, Light2, Light1, accent, Dark1, Dark2, Dark3.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Palette {
    pub shades: [Color; 7],
}
impl Palette {
    pub const fn light3(&self) -> Color {
        self.shades[0]
    }
    pub const fn light2(&self) -> Color {
        self.shades[1]
    }
    pub const fn light1(&self) -> Color {
        self.shades[2]
    }
    pub const fn base(&self) -> Color {
        self.shades[3]
    }
    pub const fn dark1(&self) -> Color {
        self.shades[4]
    }
    pub const fn dark2(&self) -> Color {
        self.shades[5]
    }
    pub const fn dark3(&self) -> Color {
        self.shades[6]
    }
    /// Accent fill (AccentFillColorDefault): Light2 in dark, Dark1 in light.
    pub const fn fill(&self, dark: bool) -> Color {
        if dark { self.light2() } else { self.dark1() }
    }
    /// Accent text and links (AccentTextFillColorPrimary).
    pub const fn text(&self, dark: bool) -> Color {
        if dark { self.light3() } else { self.dark2() }
    }
}
/// Door's default accent: the Windows 11 default blue, with the WinUI
/// default accent fills Door already used (#60CDFF dark, #005FB8 light).
pub const DEFAULT: Palette = Palette {
    shades: [
        rgb(0x99, 0xeb, 0xff),
        rgb(0x60, 0xcd, 0xff),
        rgb(0x00, 0x91, 0xf8),
        rgb(0x00, 0x78, 0xd4),
        rgb(0x00, 0x5f, 0xb8),
        rgb(0x00, 0x3e, 0x92),
        rgb(0x00, 0x1a, 0x68),
    ],
};
/// The owner's measured palette (`reports/artifacts/reference-26h2/environment.txt`
/// AccentPalette, Light3..Dark3).
pub const OWNER_REFERENCE: Palette = Palette {
    shades: [
        rgb(0xf0, 0xc0, 0xf4),
        rgb(0xdb, 0x9e, 0xe5),
        rgb(0xb7, 0x63, 0xcb),
        rgb(0xa9, 0x4d, 0xc1),
        rgb(0x8e, 0x3a, 0xa7),
        rgb(0x69, 0x27, 0x82),
        rgb(0x40, 0x0e, 0x59),
    ],
};
/// The 48 "Windows colors" of Settings > Personalization > Colors, in grid
/// order (8 per row), as 0xRRGGBB.
/// The Windows colors' accessible names, in `WINDOWS_COLORS` order (26H2
/// Settings > Colors; the names repeat where Windows repeats them).
pub const WINDOWS_COLOR_NAMES: [&str; 48] = [
    "Yellow gold",
    "Gold",
    "Orange bright",
    "Orange dark",
    "Rust",
    "Pale rust",
    "Brick red",
    "Mod red",
    "Pale red",
    "Red",
    "Rose bright",
    "Rose",
    "Plum light",
    "Plum",
    "Orchid light",
    "Orchid",
    "Blue",
    "Navy blue",
    "Purple shadow",
    "Purple shadow dark",
    "Iris pastel",
    "Iris Spring",
    "Violet red light",
    "Violet red",
    "Cool blue bright",
    "Cool blue",
    "Seafoam",
    "Seafoam teal",
    "Mint light",
    "Mint dark",
    "Turf green",
    "Sport green",
    "Gray",
    "Gray brown",
    "Steel blue",
    "Metal blue",
    "Pale moss",
    "Moss",
    "Meadow green",
    "Green",
    "Overcast",
    "Storm",
    "Blue gray",
    "Gray dark",
    "Liddy green",
    "Sage",
    "Camouflage desert",
    "Camouflage",
];
pub const WINDOWS_COLORS: [u32; 48] = [
    0xFFB900, 0xFF8C00, 0xF7630C, 0xCA5010, 0xDA3B01, 0xEF6950, 0xD13438, 0xFF4343, //
    0xE74856, 0xE81123, 0xEA005E, 0xC30052, 0xE3008C, 0xBF0077, 0xC239B3, 0x9A0089, //
    0x0078D4, 0x0063B1, 0x8E8CD8, 0x6B69D6, 0x8764B8, 0x744DA9, 0xB146C2, 0x881798, //
    0x0099BC, 0x2D7D9A, 0x00B7C3, 0x038387, 0x00B294, 0x018574, 0x00CC6A, 0x10893E, //
    0x7A7574, 0x5D5A58, 0x68768A, 0x515C6B, 0x567C73, 0x486860, 0x498205, 0x107C10, //
    0x767676, 0x4C4A48, 0x69797E, 0x4A5459, 0x647C64, 0x525E54, 0x847545, 0x7E735F,
];
/// Opaque BGRA of 0xRRGGBB.
pub const fn color_of(rgb24: u32) -> Color {
    rgb((rgb24 >> 16) as u8, (rgb24 >> 8) as u8, rgb24 as u8)
}
/// 0xRRGGBB of an opaque BGRA colour.
pub const fn rgb24(c: Color) -> u32 {
    (c[2] as u32) << 16 | (c[1] as u32) << 8 | c[0] as u32
}
/// Hue (tenths of a degree, 0..3600), saturation and lightness (permille).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hsl {
    pub h: i32,
    pub s: i32,
    pub l: i32,
}
pub const fn hsl(c: Color) -> Hsl {
    let (r, g, b) = (c[2] as i32, c[1] as i32, c[0] as i32);
    let max = if r > g {
        if r > b { r } else { b }
    } else if g > b {
        g
    } else {
        b
    };
    let min = if r < g {
        if r < b { r } else { b }
    } else if g < b {
        g
    } else {
        b
    };
    let l = ((max + min) * 1000 + 255) / 510;
    let d = max - min;
    if d == 0 {
        return Hsl { h: 0, s: 0, l };
    }
    let span = 255 - (max + min - 255).abs();
    let s = (d * 1000 + span / 2) / span;
    let h = if max == r {
        600 * (g - b) / d
    } else if max == g {
        600 * (b - r) / d + 1200
    } else {
        600 * (r - g) / d + 2400
    };
    Hsl {
        h: (h + 3600) % 3600,
        s,
        l,
    }
}
pub const fn from_hsl(c: Hsl) -> Color {
    let l = clamp(c.l, 0, 1000);
    let s = clamp(c.s, 0, 1000);
    let h = ((c.h % 3600) + 3600) % 3600;
    // Chroma, second component and match value, in 1/1000 of full scale.
    let chroma = (1000 - (2 * l - 1000).abs()) * s / 1000;
    let x = chroma * (600 - ((h % 1200) - 600).abs()) / 600;
    let m = l - chroma / 2;
    let (r, g, b) = match h / 600 {
        0 => (chroma, x, 0),
        1 => (x, chroma, 0),
        2 => (0, chroma, x),
        3 => (0, x, chroma),
        4 => (x, 0, chroma),
        _ => (chroma, 0, x),
    };
    rgb(byte(r + m), byte(g + m), byte(b + m))
}
/// A permille component as a byte.
const fn byte(v: i32) -> u8 {
    clamp((v * 255 + 500) / 1000, 0, 255) as u8
}
const fn clamp(v: i32, lo: i32, hi: i32) -> i32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}
/// Per-shade lightness and saturation offsets (permille) and hue shift
/// (tenths of a degree; lighter shades turn toward yellow, darker ones
/// toward blue, never past it), calibrated on the two reference palettes
/// above (`tests::generated_palettes_track_both_references`).
const STEPS: [(i32, i32, i32); 7] = [
    (340, 200, 80),
    (232, 90, 40),
    (66, 15, 8),
    (0, 0, 0),
    (-70, 0, -15),
    (-175, 50, -55),
    (-290, 230, -110),
];
/// Hue (tenths) shades move toward: yellow for lighter, blue for darker.
const fn toward(h: i32, shift: i32) -> i32 {
    let target = if shift > 0 { 600 } else { 2400 };
    // Signed shortest distance from h to the target.
    let mut d = (target - h) % 3600;
    if d > 1800 {
        d -= 3600;
    }
    if d < -1800 {
        d += 3600;
    }
    let step = shift.abs();
    if d.abs() <= step {
        target
    } else if d > 0 {
        h + step
    } else {
        h - step
    }
}
/// The palette of accent colour 0xRRGGBB. Door's default blue returns
/// `DEFAULT` exactly.
pub const fn palette(rgb24: u32) -> Palette {
    if rgb24 == 0x0078D4 {
        return DEFAULT;
    }
    let base = color_of(rgb24);
    let c = hsl(base);
    let mut shades = [base; 7];
    let mut i = 0;
    while i < 7 {
        if i != 3 {
            let (dl, ds, dh) = STEPS[i];
            let h = if c.s < 60 { c.h } else { toward(c.h, dh) };
            // Greys keep no saturation.
            let s = if c.s < 60 { c.s } else { c.s + ds };
            shades[i] = from_hsl(Hsl {
                h,
                s: clamp(s, 0, 1000),
                l: clamp(c.l + dl, 40, 960),
            });
        }
        i += 1;
    }
    Palette { shades }
}
/// "Automatic" accent: the dominant saturated hue of the desktop
/// background (opaque BGRA samples), as a usable accent colour (saturation
/// at least 40 %, lightness 32..56 %). Mostly grey pictures keep the
/// default blue.
pub fn from_background(samples: impl Iterator<Item = [u8; 4]>) -> u32 {
    // 36 hue bins of 10 degrees: chroma-weighted counts and colour sums.
    let mut weight = [0u64; 36];
    let mut sums = [[0u64; 3]; 36];
    let mut total = 0u64;
    for p in samples {
        let c = hsl(p);
        let chroma = (1000 - (2 * c.l - 1000).abs()) * c.s / 1000;
        total += 1;
        if chroma < 120 {
            continue;
        }
        let bin = (c.h / 100) as usize % 36;
        let w = chroma as u64;
        weight[bin] += w;
        for k in 0..3 {
            sums[bin][k] += w * u64::from(p[k]);
        }
    }
    let score = |i: usize| 2 * weight[i] + weight[(i + 35) % 36] + weight[(i + 1) % 36];
    let best = (0..36).max_by_key(|&i| score(i)).unwrap_or(0);
    // Too little colour (under ~3 % saturated area): keep the default.
    if total == 0 || weight[best] * 1000 < total * 120 * 30 {
        return 0x0078D4;
    }
    let mean: Color = [
        (sums[best][0] / weight[best]) as u8,
        (sums[best][1] / weight[best]) as u8,
        (sums[best][2] / weight[best]) as u8,
        255,
    ];
    let c = hsl(mean);
    rgb24(from_hsl(Hsl {
        h: c.h,
        s: clamp(c.s, 400, 1000),
        l: clamp(c.l, 320, 560),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn distance(a: Color, b: Color) -> i32 {
        (0..3)
            .map(|i| (i32::from(a[i]) - i32::from(b[i])).abs())
            .max()
            .unwrap()
    }
    #[test]
    fn hsl_round_trips_every_swatch() {
        for rgb24 in WINDOWS_COLORS.iter().copied().chain([0xA94DC1, 0x0078D4]) {
            let c = color_of(rgb24);
            assert!(distance(from_hsl(hsl(c)), c) <= 2, "{rgb24:06x}");
        }
    }
    /// The generated shades of both reference accents stay close to the
    /// palettes Windows produced; the shades that paint controls (Light2 in
    /// dark, Dark1 in light) are the closest.
    #[test]
    fn generated_palettes_track_both_references() {
        let owner = palette(rgb24(OWNER_REFERENCE.base()));
        for i in 0..7 {
            assert!(
                distance(owner.shades[i], OWNER_REFERENCE.shades[i]) <= 22,
                "owner shade {i}: {:06x} vs {:06x}",
                rgb24(owner.shades[i]),
                rgb24(OWNER_REFERENCE.shades[i])
            );
        }
        assert!(distance(owner.light2(), OWNER_REFERENCE.light2()) <= 8);
        assert!(distance(owner.dark1(), OWNER_REFERENCE.dark1()) <= 12);
        assert_eq!(palette(0x0078D4), DEFAULT);
        // Every swatch yields a monotonic ramp (lighter to darker).
        for rgb24 in WINDOWS_COLORS {
            let p = palette(rgb24);
            for pair in p.shades.windows(2) {
                assert!(hsl(pair[0]).l >= hsl(pair[1]).l, "{rgb24:06x}");
            }
        }
    }
    #[test]
    fn automatic_picks_the_dominant_colour_and_greys_keep_the_default() {
        let purple = [0xc1, 0x4d, 0xa9, 255];
        let grey = [0x80, 0x80, 0x80, 255];
        let blue = [0xd4, 0x78, 0x00, 255];
        let samples = (0..1000).map(|i| match i % 10 {
            0..=5 => purple,
            6 => blue,
            _ => grey,
        });
        let chosen = color_of(from_background(samples));
        let c = hsl(chosen);
        assert!((2750..=2950).contains(&c.h), "{c:?}");
        assert!(c.s >= 400 && (320..=560).contains(&c.l), "{c:?}");
        assert_eq!(from_background((0..500).map(|_| grey)), 0x0078D4);
        assert_eq!(from_background(core::iter::empty()), 0x0078D4);
    }
}
