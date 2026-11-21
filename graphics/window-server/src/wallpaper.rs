//! Immutable resource decode and reusable physical-pixel wallpaper cache.
extern crate alloc;
use alloc::vec::Vec;
pub use door_initfs::desktop_assets::Pack;
const _: () = assert!(
    door_initfs::desktop_assets::COUNT as u32 == door_service_protocol::window::WALLPAPERS,
    "appearance words index the pack"
);
pub fn load(
    source: &[u8],
    width: u32,
    height: u32,
    output: &mut Vec<u8>,
) -> Result<(), &'static str> {
    load_with_scratch(source, width, height, output, &mut Vec::new())
}
/// Caller-owned scratch avoids repeated large allocations in the native compositor.
pub fn load_with_scratch(
    source: &[u8],
    width: u32,
    height: u32,
    output: &mut Vec<u8>,
    pixels: &mut Vec<u8>,
) -> Result<(), &'static str> {
    load_fitted(source, width, height, output, pixels, Fit::Fill, [0, 0, 0])
}
/// Personalization > Background > "Choose a fit for your desktop image"
/// (`preferences::Background::fit`). Span on one display is Fill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    Fill,
    Fit,
    Stretch,
    Tile,
    Center,
}
impl Fit {
    pub const fn from_word(fit: u32) -> Self {
        match fit {
            1 => Self::Fit,
            2 => Self::Stretch,
            3 => Self::Tile,
            4 => Self::Center,
            _ => Self::Fill,
        }
    }
}
/// Native extent of every built-in wallpaper (`C:\Windows\Web\Wallpaper`).
pub const NATIVE: (usize, usize) = (3840, 2400);
/// Decode a pack picture into `pixels` as RGBA at `NATIVE`: lossless QOI,
/// or the original baseline JPEG. Four 26H2 pictures are one pixel wider or
/// taller than 3840x2400 (img24, img26, img32, img33); their extra last
/// column or row is dropped, which is what Fill shows of them anyway.
pub fn decode_native(source: &[u8], pixels: &mut Vec<u8>) -> Result<(), &'static str> {
    let (nw, nh) = NATIVE;
    if source.starts_with(b"qoif") {
        let header = qoi::decode_header(source).map_err(|_| "wallpaper header")?;
        if (header.width as usize, header.height as usize) != NATIVE
            || header.channels != qoi::Channels::Rgba
        {
            return Err("wallpaper source extent");
        }
        let length = nw * nh * 4;
        pixels
            .try_reserve_exact(length.saturating_sub(pixels.len()))
            .map_err(|_| "wallpaper scratch")?;
        pixels.resize(length, 0);
        return qoi::decode_to_buf(&mut pixels[..], source)
            .map(|_| ())
            .map_err(|_| "wallpaper decode");
    }
    if !source.starts_with(&[0xff, 0xd8, 0xff]) {
        return Err("wallpaper format");
    }
    use zune_jpeg::zune_core::{
        bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions,
    };
    let options = DecoderOptions::default()
        .set_max_width(nw + 1)
        .set_max_height(nh + 1)
        .set_strict_mode(true)
        .jpeg_set_max_scans(1)
        .jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(source), options);
    decoder.decode_headers().map_err(|_| "wallpaper header")?;
    let (w, h) = decoder.dimensions().ok_or("wallpaper header")?;
    if !(nw..=nw + 1).contains(&w) || !(nh..=nh + 1).contains(&h) {
        return Err("wallpaper source extent");
    }
    let length = w * h * 4;
    pixels
        .try_reserve_exact(length.saturating_sub(pixels.len()))
        .map_err(|_| "wallpaper scratch")?;
    pixels.resize(length, 0);
    decoder
        .decode_into(&mut pixels[..])
        .map_err(|_| "wallpaper decode")?;
    if w != nw {
        // Rows move down to their final place in order: never overlapping a
        // row not yet moved.
        for y in 1..nh {
            pixels.copy_within(y * w * 4..(y * w + nw) * 4, y * nw * 4);
        }
    }
    pixels.truncate(nw * nh * 4);
    Ok(())
}
/// `load_with_scratch` with a fit; `color` (RGB) shows around a Fit or
/// Center picture.
pub fn load_fitted(
    source: &[u8],
    width: u32,
    height: u32,
    output: &mut Vec<u8>,
    pixels: &mut Vec<u8>,
    fit: Fit,
    color: [u8; 3],
) -> Result<(), &'static str> {
    if width == 0 || height == 0 || width > 3840 || height > 2160 {
        return Err("wallpaper output extent");
    }
    // Decode/validate before touching the old cache. Reuse its allocation: at4K,
    // retaining an additional destination would exceed the compositor budget.
    decode_native(source, pixels)?;
    if pixels.chunks_exact(4).any(|p| p[3] != 255) {
        return Err("wallpaper opacity");
    }
    // Area prefilter before large reductions. Four-neighbour bilinear sampling
    // alone aliases the fine Bloom folds when reducing a4K source to720p.
    let (mut sw, mut sh) = (3840usize, 2400usize);
    while sw / 2 >= width as usize && sh / 2 >= height as usize {
        let (nw, nh) = (sw / 2, sh / 2);
        for y in 0..nh {
            for x in 0..nw {
                for channel in 0..4 {
                    let sample =
                        |dx, dy| pixels[((y * 2 + dy) * sw + x * 2 + dx) * 4 + channel] as u16;
                    let value =
                        ((sample(0, 0) + sample(1, 0) + sample(0, 1) + sample(1, 1) + 2) / 4) as u8;
                    pixels[(y * nw + x) * 4 + channel] = value;
                }
            }
        }
        pixels.truncate(nw * nh * 4);
        sw = nw;
        sh = nh;
    }
    let length = width as usize * height as usize * 4;
    output
        .try_reserve(length.saturating_sub(output.len()))
        .map_err(|_| "wallpaper memory")?;
    output.resize(length, 0);
    // 16.16 source steps per output pixel and the source origin of output
    // (0, 0); outside the picture (Fit, Center) shows `color`.
    let (fw, fh) = (
        ((sw as i64) << 16) / width as i64,
        ((sh as i64) << 16) / height as i64,
    );
    let native = ((sw as i64) << 16) / 3840;
    let (step_x, step_y) = match fit {
        Fit::Fill => (fw.min(fh), fw.min(fh)),
        Fit::Fit => (fw.max(fh), fw.max(fh)),
        Fit::Stretch => (fw, fh),
        Fit::Tile | Fit::Center => (native, native),
    };
    let (left, top) = match fit {
        Fit::Stretch | Fit::Tile => (0, 0),
        _ => (
            (((sw as i64) << 16) - width as i64 * step_x) / 2,
            (((sh as i64) << 16) - height as i64 * step_y) / 2,
        ),
    };
    let span = ((sw as i64) << 16, (sh as i64) << 16);
    for y in 0..height {
        let mut sy = top + y as i64 * step_y + step_y / 2 - 32768;
        if fit == Fit::Tile {
            sy = sy.rem_euclid(span.1);
        }
        let outside_y = sy < -32768 || sy > span.1 - 32768;
        let sy = sy.clamp(0, (sh as i64 - 1) << 16) as u64;
        let y0 = sy as usize >> 16;
        let fy = sy & 65535;
        for x in 0..width {
            let mut sx = left + x as i64 * step_x + step_x / 2 - 32768;
            if fit == Fit::Tile {
                sx = sx.rem_euclid(span.0);
            }
            let at = (y as usize * width as usize + x as usize) * 4;
            if outside_y || sx < -32768 || sx > span.0 - 32768 {
                output[at..at + 4].copy_from_slice(&[color[2], color[1], color[0], 255]);
                continue;
            }
            let sx = sx.clamp(0, (sw as i64 - 1) << 16) as u64;
            let x0 = sx as usize >> 16;
            let fx = sx & 65535;
            for (i, channel) in [2, 1, 0].into_iter().enumerate() {
                let sample = |px: usize, py: usize| pixels[(py * sw + px) * 4 + channel] as u64;
                let a = sample(x0, y0) * (65536 - fx) + sample((x0 + 1).min(sw - 1), y0) * fx;
                let b = sample(x0, (y0 + 1).min(sh - 1)) * (65536 - fx)
                    + sample((x0 + 1).min(sw - 1), (y0 + 1).min(sh - 1)) * fx;
                output[at + i] = ((a * (65536 - fy) + b * fy + (1 << 31)) >> 32) as u8;
            }
            output[at + 3] = 255;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_local_background_is_opaque_native_resolution_and_reuses_destination() {
        let pack = Pack::parse(include_bytes!(
            "../../../assets/wallpapers/windows/desktop-assets.blob"
        ))
        .unwrap();
        let mut cache = Vec::new();
        for id in 0..door_initfs::desktop_assets::COUNT as u32 {
            load(pack.background(id).unwrap(), 1920, 1080, &mut cache).unwrap();
            assert_eq!(cache.len(), 1920 * 1080 * 4);
            assert!(cache.chunks_exact(4).all(|p| p[3] == 255));
            let address = cache.as_ptr();
            load(pack.background(id).unwrap(), 1920, 1080, &mut cache).unwrap();
            assert_eq!(cache.as_ptr(), address);
        }
        load(pack.background(1).unwrap(), 1280, 720, &mut cache).unwrap();
        assert_eq!(&cache[..4], &[29, 15, 4, 255]);
        let old = cache.clone();
        assert!(load(b"invalid", 1920, 1080, &mut cache).is_err());
        assert_eq!(cache, old);
        assert!(load(pack.background(0).unwrap(), 3841, 2160, &mut cache).is_err());
    }
}
