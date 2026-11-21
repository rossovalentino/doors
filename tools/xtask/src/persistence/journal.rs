//! Independent retained-image journal decoder and replay witness.
use super::{HOME_FIRST, HOME_SECTORS, LOG_FIRST, LOG_SECTORS, require, word};
use crate::storage::audit::crc32;
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    pub generation: u64,
    pub committed: bool,
    pub entries: Vec<(usize, u32)>,
}
pub struct Journal {
    pub headers: [Header; 2],
    pub selected: usize,
}
fn decode(h: &[u8]) -> Result<Header, String> {
    require(
        h.len() == 4096 && &h[..8] == b"DOORJNL1",
        "journal header magic/length",
    )?;
    for (at, width, value) in [
        (8, 4, 1),
        (12, 4, 4096),
        (80, 8, HOME_FIRST),
        (88, 8, HOME_SECTORS),
        (96, 8, LOG_FIRST),
        (104, 8, LOG_SECTORS),
        (112, 4, 512),
        (116, 4, 4096),
    ] {
        require(
            word(h, at, width)? == value,
            "journal header geometry/version",
        )?;
    }
    require(
        &h[32..48] == b"DoorP07DiskGUID0"
            && &h[48..64] == b"DoorP07ExtGUID00"
            && &h[64..80] == b"DoorP07LogGUID00",
        "journal volume identity",
    )?;
    let mut checked = h.to_vec();
    checked[120..124].fill(0);
    require(
        u64::from(crc32(&checked)) == word(h, 120, 4)?,
        "journal header CRC",
    )?;
    let state = word(h, 16, 4)?;
    let count = word(h, 20, 4)? as usize;
    require(
        (state == 0 && count == 0) || (state == 1 && (1..=64).contains(&count)),
        "journal state/count",
    )?;
    require(
        state == 0 || word(h, 24, 8)? > 0,
        "committed journal generation must be nonzero",
    )?;
    require(
        h[124..128].iter().all(|b| *b == 0) && h[128 + count * 8..].iter().all(|b| *b == 0),
        "journal reserved/unused entries",
    )?;
    let mut entries = Vec::new();
    let mut used = BTreeSet::new();
    for i in 0..count {
        let block = word(h, 128 + i * 8, 4)? as usize;
        require(
            block < 8192 && used.insert(block),
            "journal target bounds/duplicate",
        )?;
        entries.push((block, word(h, 132 + i * 8, 4)? as u32));
    }
    Ok(Header {
        generation: word(h, 24, 8)?,
        committed: state == 1,
        entries,
    })
}
pub fn inspect(bytes: &[u8]) -> Result<Journal, String> {
    let base = LOG_FIRST as usize * 512;
    let area = bytes
        .get(base..base + LOG_SECTORS as usize * 512)
        .ok_or("journal partition truncated")?;
    let headers = [decode(&area[..4096])?, decode(&area[4096..8192])?];
    require(
        area[8192 + 64 * 4096..].iter().all(|b| *b == 0),
        "journal reserved partition bytes",
    )?;
    let selected = if headers[0].generation == headers[1].generation {
        if headers[0].committed == headers[1].committed {
            require(
                area[..4096] == area[4096..8192],
                "journal conflicting equal-generation copies",
            )?;
            0
        } else {
            // A checkpoint is published only after all home writes are flushed.
            usize::from(headers[0].committed)
        }
    } else {
        let high = usize::from(headers[1].generation > headers[0].generation);
        let low = 1 - high;
        // A mirror publication may leave one prior checkpoint beside the next
        // committed transaction. No other different-generation pair is legal.
        require(
            headers[low].generation.checked_add(1) == Some(headers[high].generation)
                && !headers[low].committed
                && headers[high].committed,
            "journal contradictory mirror generations/states",
        )?;
        high
    };
    let selected_header = &headers[selected];
    if selected_header.committed {
        for (index, (_, crc)) in selected_header.entries.iter().enumerate() {
            require(
                crc32(&area[8192 + index * 4096..8192 + (index + 1) * 4096]) == *crc,
                "journal payload CRC",
            )?;
        }
    }
    Ok(Journal { headers, selected })
}
impl Journal {
    pub fn selected(&self) -> &Header {
        &self.headers[self.selected]
    }
    pub fn summary(&self) -> Value {
        let h = self.selected();
        json!({"generation":h.generation,"committed":h.committed,"entries":h.entries.iter().map(|(block,crc)|json!({"home_block":block,"crc32":format!("{crc:08x}")})).collect::<Vec<_>>(),"copies":self.headers.iter().map(|h|json!({"generation":h.generation,"committed":h.committed,"entries":h.entries.len()})).collect::<Vec<_>>()})
    }
    pub fn replayed_home(&self, bytes: &[u8]) -> Result<Vec<u8>, String> {
        let base = HOME_FIRST as usize * 512;
        let mut home = bytes
            .get(base..base + HOME_SECTORS as usize * 512)
            .ok_or("home truncated")?
            .to_vec();
        if self.selected().committed {
            for (index, (block, _)) in self.selected().entries.iter().enumerate() {
                let source = LOG_FIRST as usize * 512 + 8192 + index * 4096;
                home[block * 4096..(block + 1) * 4096]
                    .copy_from_slice(&bytes[source..source + 4096]);
            }
        }
        Ok(home)
    }
}
