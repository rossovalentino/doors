//! Decode the pinned ICO's authored small frames as well as its large PNG.
use resvg::tiny_skia::Pixmap;

pub fn frame(data: &[u8]) -> Result<Option<Pixmap>, String> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        if data.len() < 24
            || u32::from_be_bytes(data[16..20].try_into().unwrap()) > 512
            || u32::from_be_bytes(data[20..24].try_into().unwrap()) > 512
        {
            return Err("PNG icon dimensions".into());
        }
        let image = Pixmap::decode_png(data).map_err(|e| e.to_string())?;
        if image.width() != image.height() {
            return Err("non-square icon".into());
        }
        return Ok(Some(image));
    }
    if data.len() < 40 {
        return Err("ICO DIB header".into());
    }
    let word = |at| u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
    // Other depths remain unsupported; a bounded PNG fallback is mandatory.
    if word(0) != 40 || data[12..16] != [1, 0, 32, 0] || word(16) != 0 {
        return Ok(None);
    }
    let size = word(4) as usize;
    if size == 0 || size > 512 || word(8) as usize != size * 2 {
        return Err("ICO DIB dimensions".into());
    }
    let bytes = size * size * 4;
    let mask_stride = size.div_ceil(32) * 4;
    if data.len() < 40 + bytes + mask_stride * size {
        return Err("ICO DIB backing".into());
    }
    let alpha = data[40..40 + bytes].chunks_exact(4).any(|p| p[3] != 0);
    let mut image = Pixmap::new(size as u32, size as u32).ok_or("ICO memory")?;
    for y in 0..size {
        let row = size - 1 - y;
        for x in 0..size {
            let at = 40 + (row * size + x) * 4;
            let p = &data[at..at + 4];
            let masked = data[40 + bytes + row * mask_stride + x / 8] & (0x80 >> (x % 8)) != 0;
            let a = if alpha {
                p[3]
            } else if masked {
                0
            } else {
                255
            };
            let premul = |c: u8| ((u32::from(c) * u32::from(a) + 127) / 255) as u8;
            image.data_mut()[(y * size + x) * 4..(y * size + x + 1) * 4].copy_from_slice(&[
                premul(p[2]),
                premul(p[1]),
                premul(p[0]),
                a,
            ]);
        }
    }
    Ok(Some(image))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dib_rows_alpha_and_mask_are_decoded_before_resizing() {
        let mut d = vec![0; 40 + 16 + 8];
        for (at, value) in [(0, 40u32), (4, 2), (8, 4)] {
            d[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        d[12..16].copy_from_slice(&[1, 0, 32, 0]);
        d[40..44].copy_from_slice(&[100, 80, 60, 128]);
        d[48..52].copy_from_slice(&[30, 20, 10, 255]);
        let p = frame(&d).unwrap().unwrap();
        assert_eq!(&p.data()[..4], &[10, 20, 30, 255]);
        assert_eq!(&p.data()[8..12], &[30, 40, 50, 128]);
        assert!(frame(&d[..63]).is_err());
        d[43] = 0;
        d[51] = 0;
        d[56] = 0x80;
        let p = frame(&d).unwrap().unwrap();
        assert_eq!(&p.data()[8..12], &[0; 4]);
        assert_eq!(&p.data()[..4], &[10, 20, 30, 255]);
    }
    /// The taskbar/Start app frames the shell blits 1:1 are the ICOs'
    /// authored frames, byte for byte (premultiplied BGRA), wherever the ICO
    /// has that size: 24 px at 1x, 48 px at 2x, and the 16/32 px list sizes.
    #[test]
    fn shell_app_frames_are_the_authored_ico_frames() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons/windows");
        for name in ["explorer", "notepad", "settings", "terminal-current"] {
            let bytes = std::fs::read(dir.join(format!("{name}.ico"))).unwrap();
            let count = u16::from_le_bytes(bytes[4..6].try_into().unwrap()) as usize;
            let mut frames = Vec::new();
            for entry in bytes[6..6 + count * 16].chunks_exact(16) {
                let n = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
                let start = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
                if let Some(p) = frame(&bytes[start..start + n]).unwrap() {
                    frames.push(p);
                }
            }
            for size in [16u32, 24, 32, 48] {
                let authored = frames
                    .iter()
                    .find(|p| p.width() == size)
                    .unwrap_or_else(|| panic!("{name}.ico has no {size}px frame"));
                let bgra: Vec<u8> = authored
                    .data()
                    .chunks_exact(4)
                    .flat_map(|p| [p[2], p[1], p[0], p[3]])
                    .collect();
                let exported = std::fs::read(dir.join(format!("{name}.{size}.bgra"))).unwrap();
                assert!(
                    exported == bgra,
                    "{name}.{size}.bgra is not the authored ICO frame"
                );
            }
        }
    }
}
