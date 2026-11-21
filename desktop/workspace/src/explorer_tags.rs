//! User-authored tags bound to stable Storage object IDs. No preset tag names.
extern crate alloc;
use alloc::{string::String, vec::Vec};
pub const MAX_TAGS: usize = 8;
pub const MAX_BINDINGS: usize = 128;
pub const MAX_BYTES: usize = 4096;
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct Catalog {
    pub names: Vec<String>,
    pub bindings: Vec<(u64, u8)>,
}
impl Catalog {
    pub fn tags(&self, object: u64) -> u8 {
        self.bindings
            .iter()
            .find(|(id, _)| *id == object)
            .map_or(0, |(_, tags)| *tags)
    }
    pub fn create(&mut self, name: &str) -> Result<usize, &'static str> {
        let name = name.trim();
        if name.is_empty() || name.len() > 24 || !name.bytes().all(|b| (32..=126).contains(&b)) {
            return Err("Use 1 to 24 characters for the tag name.");
        }
        if let Some(index) = self.names.iter().position(|n| n.eq_ignore_ascii_case(name)) {
            return Ok(index);
        }
        if self.names.len() == MAX_TAGS {
            return Err("The current tag catalog supports 8 tag names.");
        }
        self.names.push(name.into());
        Ok(self.names.len() - 1)
    }
    pub fn toggle(&mut self, object: u64, index: usize) -> Result<(), &'static str> {
        if object == 0 || index >= self.names.len() {
            return Err("Select a valid file and tag first.");
        }
        if let Some(position) = self.bindings.iter().position(|(id, _)| *id == object) {
            self.bindings[position].1 ^= 1 << index;
            if self.bindings[position].1 == 0 {
                self.bindings.remove(position);
            }
        } else {
            if self.bindings.len() == MAX_BINDINGS {
                return Err("The current tag catalog supports 128 tagged items.");
            }
            self.bindings.push((object, 1 << index));
        }
        Ok(())
    }
    /// Renames tag `index`; names stay unique (case-insensitive).
    pub fn rename(&mut self, index: usize, name: &str) -> Result<(), &'static str> {
        let name = name.trim();
        if index >= self.names.len() {
            return Err("This tag no longer exists.");
        }
        if name.is_empty() || name.len() > 24 || !name.bytes().all(|b| (32..=126).contains(&b)) {
            return Err("Use 1 to 24 characters for the tag name.");
        }
        if self
            .names
            .iter()
            .enumerate()
            .any(|(i, n)| i != index && n.eq_ignore_ascii_case(name))
        {
            return Err("Another tag already has this name.");
        }
        self.names[index] = name.into();
        Ok(())
    }
    /// Deletes tag `index` and unassigns it everywhere; later tags move down
    /// one bit so every remaining binding keeps its names. Returns how many
    /// items lost the tag.
    pub fn remove(&mut self, index: usize) -> Result<usize, &'static str> {
        if index >= self.names.len() {
            return Err("This tag no longer exists.");
        }
        self.names.remove(index);
        let low = (1u16 << index) as u8 - 1;
        let mut removed = 0;
        for (_, tags) in &mut self.bindings {
            if *tags & (1 << index) != 0 {
                removed += 1;
            }
            *tags = (*tags & low) | ((u16::from(*tags) >> (index + 1)) << index) as u8;
        }
        self.bindings.retain(|(_, tags)| *tags != 0);
        Ok(removed)
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::from(&b"DOORTAG1"[..]);
        b.push(self.names.len() as u8);
        b.extend_from_slice(&(self.bindings.len() as u16).to_le_bytes());
        for name in &self.names {
            b.push(name.len() as u8);
            b.extend_from_slice(name.as_bytes());
        }
        for (id, tags) in &self.bindings {
            b.extend_from_slice(&id.to_le_bytes());
            b.push(*tags);
        }
        b
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.is_empty() {
            return Ok(Self::default());
        }
        if bytes.len() > MAX_BYTES || bytes.get(..8) != Some(b"DOORTAG1") || bytes.len() < 11 {
            return Err("The saved tag catalog is invalid.");
        }
        let names = bytes[8] as usize;
        let bindings = u16::from_le_bytes(bytes[9..11].try_into().unwrap()) as usize;
        if names > MAX_TAGS || bindings > MAX_BINDINGS {
            return Err("The saved tag catalog exceeds its limits.");
        }
        let mut catalog = Self::default();
        let mut at = 11;
        for _ in 0..names {
            let len = *bytes.get(at).ok_or("Truncated tag name.")? as usize;
            at += 1;
            let end = at
                .checked_add(len)
                .filter(|end| *end <= bytes.len())
                .ok_or("Truncated tag name.")?;
            let name =
                core::str::from_utf8(&bytes[at..end]).map_err(|_| "Invalid tag encoding.")?;
            let before = catalog.names.len();
            if len == 0
                || name.trim() != name
                || catalog.create(name)? != before
                || catalog.names.len() != before + 1
            {
                return Err("Invalid or duplicate tag name.");
            }
            at = end;
        }
        if bytes.len() != at + bindings * 9 {
            return Err("Invalid tag binding length.");
        }
        let allowed = if names == 8 {
            255
        } else {
            (1u16 << names) as u8 - 1
        };
        for record in bytes[at..].chunks_exact(9) {
            let id = u64::from_le_bytes(record[..8].try_into().unwrap());
            let tags = record[8];
            if id == 0
                || tags == 0
                || tags & !allowed != 0
                || catalog.bindings.iter().any(|(old, _)| *old == id)
            {
                return Err("Invalid tag binding.");
            }
            catalog.bindings.push((id, tags));
        }
        Ok(catalog)
    }
}
/// Shared drawing and hit-test geometry, relative to the pane body origin.
#[derive(Clone, Copy)]
pub struct TagGrid {
    width: u32,
    count: usize,
}
impl TagGrid {
    pub fn new(width: u32, count: usize) -> Self {
        Self {
            width: width.min(16384),
            count: count.min(MAX_TAGS),
        }
    }
    pub fn columns(self) -> usize {
        if self.width >= 280 { 2 } else { 1 }
    }
    pub fn cell(self, index: usize) -> Option<crate::explorer_layout::Bounds> {
        if index >= self.count || self.width < 48 {
            return None;
        }
        let cols = self.columns();
        let width = self.width.saturating_sub(40 + (cols as u32 - 1) * 8) / cols as u32;
        Some(crate::explorer_layout::Bounds {
            x: 20 + (index % cols) as i32 * (width as i32 + 8),
            y: 16 + (index / cols) as i32 * Self::pitch(),
            width,
            height: Self::pitch() as u32 - 8,
        })
    }
    pub fn create(self) -> crate::explorer_layout::Bounds {
        crate::explorer_layout::Bounds {
            x: 20,
            y: 16 + self.count.div_ceil(self.columns()) as i32 * Self::pitch(),
            width: self.width.saturating_sub(40),
            height: Self::pitch() as u32 - 8,
        }
    }
    /// Row pitch: a 34 px toggle holding one 14 px label plus an 8 px gap,
    /// grown with the label's line box at the applied text size.
    fn pitch() -> i32 {
        42 + crate::explorer_layout::grow()
    }
    pub fn content_height(self) -> usize {
        self.create().y as usize + 130
    }
}
pub fn color(index: usize, dark: bool) -> [u8; 4] {
    let rgb = if dark {
        [
            [103, 190, 255],
            [190, 150, 255],
            [255, 132, 148],
            [247, 205, 98],
            [91, 211, 174],
            [255, 165, 104],
            [137, 179, 255],
            [213, 159, 235],
        ]
    } else {
        [
            [0, 120, 212],
            [137, 80, 192],
            [202, 55, 81],
            [163, 111, 0],
            [0, 135, 103],
            [192, 89, 20],
            [63, 103, 181],
            [151, 75, 164],
        ]
    };
    let [r, g, b] = rgb[index % MAX_TAGS];
    [b, g, r, 255]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tag_grid_cells_do_not_overlap_and_creation_follows_the_last_row() {
        for width in [48, 200, 279, 280, 360, 420] {
            for count in 0..=8 {
                let g = TagGrid::new(width, count);
                let create = g.create();
                for i in 0..count {
                    let b = g.cell(i).unwrap();
                    assert!(b.x >= 0 && b.x as u32 + b.width <= width);
                    assert!(b.y + b.height as i32 <= create.y);
                    assert!(b.contains(b.x, b.y));
                    assert!(!b.contains(b.x + b.width as i32, b.y));
                    for j in 0..i {
                        let a = g.cell(j).unwrap();
                        assert!(!a.contains(b.x, b.y));
                    }
                }
                assert!(g.cell(count).is_none());
                assert!(create.y as usize + create.height as usize <= g.content_height());
            }
        }
    }
    #[test]
    fn user_names_roundtrip_and_tags_follow_object_identity() {
        let mut c = Catalog::default();
        assert!(c.names.is_empty());
        let i = c.create("Project alpha").unwrap();
        c.toggle(42, i).unwrap();
        assert_eq!(c.tags(42), 1);
        assert_eq!(c.tags(43), 0);
        assert_eq!(Catalog::decode(&c.encode()).unwrap(), c);
        c.toggle(42, i).unwrap();
        assert!(c.bindings.is_empty());
    }
    #[test]
    fn renaming_and_deleting_tags_keep_other_assignments() {
        let mut c = Catalog::default();
        for name in ["A", "B", "C"] {
            c.create(name).unwrap();
        }
        c.toggle(1, 0).unwrap();
        c.toggle(1, 2).unwrap();
        c.toggle(2, 1).unwrap();
        c.toggle(3, 2).unwrap();
        assert_eq!(c.rename(1, "c"), Err("Another tag already has this name."));
        c.rename(1, "Beta").unwrap();
        assert_eq!(c.names[1], "Beta");
        assert_eq!(c.remove(1), Ok(1));
        assert_eq!(c.names, ["A", "C"]);
        assert_eq!(c.tags(1), 0b11, "A and C stay on item 1");
        assert_eq!(c.tags(2), 0, "item 2 only had Beta");
        assert_eq!(c.tags(3), 0b10, "C moved down to bit 1");
        assert!(c.bindings.iter().all(|(id, _)| *id != 2));
        assert_eq!(Catalog::decode(&c.encode()).unwrap(), c);
        assert!(c.remove(5).is_err());
        let mut full = Catalog::default();
        for i in 0..8 {
            full.create(&alloc::format!("T{i}")).unwrap();
        }
        full.toggle(9, 7).unwrap();
        full.toggle(9, 0).unwrap();
        full.remove(0).unwrap();
        assert_eq!(full.tags(9), 1 << 6);
    }
    #[test]
    fn malformed_duplicate_and_future_catalogs_are_never_replaced() {
        for b in [
            &b"DOORTAG2"[..],
            &b"other user bytes"[..],
            &b"DOORTAG1\xff\xff\xff"[..],
        ] {
            assert!(Catalog::decode(b).is_err());
        }
        let mut c = Catalog::default();
        assert!(c.create("").is_err());
        assert!(c.create("a\nb").is_err());
        assert_eq!(c.create("One"), Ok(0));
        assert_eq!(c.create("one"), Ok(0));
        let mut wire = c.encode();
        wire.push(0);
        assert!(Catalog::decode(&wire).is_err());
        let duplicate = b"DOORTAG1\x02\x00\x00\x01a\x01a";
        assert!(Catalog::decode(duplicate).is_err());
    }
}
