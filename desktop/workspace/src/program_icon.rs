//! A program's own icon for File Explorer: `door_pe_icon` over the ranged
//! `file_metadata::Sparse` reader, with PNG frames decoded here.
extern crate alloc;
use crate::file_metadata::{Miss, Sparse};
use alloc::vec::Vec;

/// One decoded frame: premultiplied BGRA, `size` px square.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub size: u32,
    pub bgra: Vec<u8>,
}
/// The frames Explorer draws: list sizes (16 px at 100-200%), 32/48 for
/// tiles and medium icons, the largest (usually 256) for large views.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramIcon {
    pub frames: Vec<Frame>,
}
impl ProgramIcon {
    /// The frame for `physical` pixels: the smallest at least that large,
    /// else the largest.
    pub fn best(&self, physical: u32) -> Option<&Frame> {
        self.frames
            .iter()
            .filter(|f| f.size >= physical)
            .min_by_key(|f| f.size)
            .or_else(|| self.frames.iter().max_by_key(|f| f.size))
    }
}
const TARGETS: [u32; 5] = [16, 32, 48, 96, 256];

impl door_pe_icon::Source for Sparse {
    fn size(&self) -> u64 {
        Sparse::size(self)
    }
    fn bytes(&self, offset: u64, len: u64) -> Result<&[u8], door_pe_icon::Miss> {
        self.get(offset, len).map_err(|m| match m {
            Miss::Need(at, n) => door_pe_icon::Miss::Need(at, n),
            Miss::Truncated => door_pe_icon::Miss::Malformed,
        })
    }
}
/// The program's icon, `Ok(None)` without one (Explorer then draws the
/// generic application icon), or the bytes still needed.
pub fn read(d: &Sparse) -> Result<Option<ProgramIcon>, Miss> {
    let need = |m: door_pe_icon::Miss| match m {
        door_pe_icon::Miss::Need(at, n) => Err(Miss::Need(at, n)),
        door_pe_icon::Miss::Malformed => Ok(None),
    };
    let entries = match door_pe_icon::group(d) {
        Ok(Some(e)) => e,
        Ok(None) => return Ok(None),
        Err(m) => return need(m),
    };
    let mut chosen: Vec<door_pe_icon::Entry> = Vec::new();
    for target in TARGETS {
        if let Some(e) = door_pe_icon::choose(&entries, target)
            && !chosen.iter().any(|c| c.id == e.id)
        {
            chosen.push(e);
        }
    }
    let mut frames = Vec::new();
    for entry in chosen {
        let data = match door_pe_icon::frame(d, &entry) {
            Ok(Some(data)) => data,
            Ok(None) => continue,
            Err(m) => return need(m),
        };
        let frame = if door_pe_icon::is_png(data) {
            crate::explorer_preview::decode_full(data)
                .ok()
                .filter(|i| i.width == i.height)
                .map(|i| Frame {
                    size: i.width as u32,
                    bgra: i.bgra,
                })
        } else {
            door_pe_icon::decode_dib(data)
                .filter(|i| i.width == i.height)
                .map(|i| Frame {
                    size: i.width,
                    bgra: i.bgra,
                })
        };
        frames.extend(frame);
    }
    Ok((!frames.is_empty()).then_some(ProgramIcon { frames }))
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    /// Real programs through Explorer's ranged reads: DOOR_PE_ICON_LIVE
    /// lists paths separated by `;` (`path=!` expects no icon); with
    /// DOOR_PE_ICON_DUMP set, each frame is written there as raw BGRA. `--ignored`.
    #[test]
    #[ignore]
    fn live_programs_read_in_ranges() {
        let Ok(list) = std::env::var("DOOR_PE_ICON_LIVE") else {
            return;
        };
        for item in list.split(';').filter(|s| !s.is_empty()) {
            let (path, none) = item.strip_suffix("=!").map_or((item, false), |p| (p, true));
            let bytes = std::fs::read(path).unwrap();
            let mut d = Sparse::new(bytes.len() as u64);
            let mut rounds = 0;
            let icon = loop {
                match read(&d) {
                    Ok(icon) => break icon,
                    Err(Miss::Need(at, len)) => {
                        rounds += 1;
                        assert!(rounds < 64, "{path}: too many rounds");
                        let end = (at + len.max(4096)).min(bytes.len() as u64);
                        d.insert(at, &bytes[at as usize..end as usize]);
                    }
                    Err(Miss::Truncated) => panic!("{path}: truncated"),
                }
            };
            let sizes: Vec<u32> = icon
                .as_ref()
                .map(|i| i.frames.iter().map(|f| f.size).collect())
                .unwrap_or_default();
            std::println!(
                "{path}: {} rounds, {} KiB held, frames {sizes:?}",
                rounds,
                d.held() / 1024
            );
            assert_eq!(icon.is_none(), none, "{path}");
            if let Some(icon) = icon {
                for f in &icon.frames {
                    assert_eq!(f.bgra.len() as u32, f.size * f.size * 4, "{path}");
                }
                assert!(icon.best(16).is_some() && icon.best(256).is_some());
                if let Ok(dir) = std::env::var("DOOR_PE_ICON_DUMP") {
                    for f in &icon.frames {
                        let name = path.rsplit('/').next().unwrap();
                        std::fs::write(std::format!("{dir}/{name}.{}.bgra", f.size), &f.bgra)
                            .unwrap();
                    }
                }
            }
        }
    }
}
