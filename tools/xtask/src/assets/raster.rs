//! Area-prefiltered image reduction. Bicubic alone aliases large reductions.
use resvg::tiny_skia::Pixmap;
pub fn prefilter(source: &Pixmap, width: u32, height: u32) -> Result<Pixmap, String> {
    let mut current = source.clone();
    while current.width() / 2 >= width && current.height() / 2 >= height {
        let (w, h) = (current.width() / 2, current.height() / 2);
        let mut next = Pixmap::new(w, h).ok_or("prefilter memory")?;
        for y in 0..h {
            for x in 0..w {
                for channel in 0..4 {
                    let sample = |dx, dy| {
                        current.data()
                            [(((y * 2 + dy) * current.width() + x * 2 + dx) * 4 + channel) as usize]
                            as u16
                    };
                    next.data_mut()[((y * w + x) * 4 + channel) as usize] =
                        ((sample(0, 0) + sample(1, 0) + sample(0, 1) + sample(1, 1) + 2) / 4) as u8;
                }
            }
        }
        current = next;
    }
    Ok(current)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reductions_remove_checkerboard_aliasing_and_preserve_premultiplication() {
        let mut source = Pixmap::new(64, 64).unwrap();
        for y in 0..64 {
            for x in 0..64 {
                let value = if (x + y) % 2 == 0 { 255 } else { 0 };
                source.data_mut()[(y * 64 + x) * 4..(y * 64 + x + 1) * 4]
                    .copy_from_slice(&[value; 4]);
            }
        }
        let filtered = prefilter(&source, 4, 4).unwrap();
        assert_eq!((filtered.width(), filtered.height()), (4, 4));
        assert!(filtered.data().iter().all(|&v| v == 128));
        assert_eq!(source.data()[0], 255);
        assert_eq!(prefilter(&source, 64, 64).unwrap().data(), source.data());
    }
}
