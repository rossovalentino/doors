//! Text dictionaries mapped read-only (`&'static [u8]` from a resource
//! section). Format (UTF-8, LF):
//!
//! ```text
//! # door-ime dictionary v1
//! # comment lines (source, licence)
//! reading<TAB>candidate[,cost[,class]]<TAB>candidate...
//! ```
//!
//! Entry lines are sorted bytewise by reading and readings are unique.
//! Candidates are listed most frequent first; `cost` (lower is better) and
//! `class` (part of speech) are optional. Only an index of line offsets is
//! allocated; the bytes stay in the mapped section.
extern crate alloc;
use alloc::vec::Vec;
pub const HEADER: &str = "# door-ime dictionary v1";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Header,
    Utf8,
    /// Line number of a malformed entry.
    Line(usize),
    /// Line number out of order or duplicated.
    Order(usize),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidate<'a> {
    pub text: &'a str,
    pub cost: Option<u32>,
    pub class: &'a str,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry<'a> {
    pub reading: &'a str,
    fields: &'a str,
}
fn candidate(field: &str) -> Option<Candidate<'_>> {
    let mut parts = field.split(',');
    let text = parts.next().filter(|t| !t.is_empty())?;
    let cost = match parts.next() {
        Some(c) => Some(c.parse().ok()?),
        None => None,
    };
    let class = parts.next().unwrap_or("");
    parts
        .next()
        .is_none()
        .then_some(Candidate { text, cost, class })
}
impl<'a> Entry<'a> {
    pub fn candidates(&self) -> impl Iterator<Item = Candidate<'a>> + 'a {
        self.fields.split('\t').filter_map(candidate)
    }
}
pub struct Dictionary {
    data: &'static str,
    lines: Vec<(u32, u32)>,
}
impl Dictionary {
    pub fn parse(bytes: &'static [u8]) -> Result<Self, Error> {
        let data = core::str::from_utf8(bytes).map_err(|_| Error::Utf8)?;
        if !data.starts_with(HEADER) {
            return Err(Error::Header);
        }
        // One exact allocation: a 30 MB dictionary must not double its index.
        let mut lines = Vec::with_capacity(bytes.iter().filter(|b| **b == b'\n').count() + 1);
        let mut at = 0usize;
        let mut last: Option<&str> = None;
        for (n, line) in data.split('\n').enumerate() {
            let start = at;
            at += line.len() + 1;
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (reading, fields) = line.split_once('\t').ok_or(Error::Line(n + 1))?;
            if reading.is_empty() || fields.split('\t').any(|f| candidate(f).is_none()) {
                return Err(Error::Line(n + 1));
            }
            if last.is_some_and(|l| l >= reading) {
                return Err(Error::Order(n + 1));
            }
            last = Some(reading);
            lines.push((start as u32, (start + line.len()) as u32));
        }
        Ok(Self { data, lines })
    }
    /// An empty dictionary (engines still compose without one).
    pub fn empty() -> Self {
        Self {
            data: "",
            lines: Vec::new(),
        }
    }
    pub fn len(&self) -> usize {
        self.lines.len()
    }
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
    fn entry(&self, i: usize) -> Entry<'static> {
        let (s, e) = self.lines[i];
        let line = &self.data[s as usize..e as usize];
        let (reading, fields) = line.split_once('\t').unwrap();
        Entry { reading, fields }
    }
    fn lower_bound(&self, key: &str) -> usize {
        let (mut lo, mut hi) = (0, self.lines.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.entry(mid).reading < key {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }
    pub fn get(&self, reading: &str) -> Option<Entry<'static>> {
        let i = self.lower_bound(reading);
        (i < self.lines.len() && self.entry(i).reading == reading).then(|| self.entry(i))
    }
    /// Entries whose reading starts with `prefix`, in reading order.
    pub fn prefix<'s>(&'s self, prefix: &'s str) -> impl Iterator<Item = Entry<'static>> + 's {
        let start = self.lower_bound(prefix);
        (start..self.lines.len())
            .map(move |i| self.entry(i))
            .take_while(move |e| e.reading.starts_with(prefix))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_sorted_entries_and_rejects_malformed_files() {
        let d = Dictionary::parse(b"# door-ime dictionary v1\n# test\nni\t\xe4\xbd\xa0\t\xe5\x91\xa2\nni'hao\t\xe4\xbd\xa0\xe5\xa5\xbd,10,n\nnin\t\xe6\x82\xa8\n").unwrap();
        assert_eq!(d.len(), 3);
        let e = d.get("ni'hao").unwrap();
        assert_eq!(
            e.candidates().next(),
            Some(Candidate {
                text: "你好",
                cost: Some(10),
                class: "n"
            })
        );
        assert_eq!(d.get("ni").unwrap().candidates().count(), 2);
        assert!(d.get("n").is_none());
        assert_eq!(d.prefix("ni").count(), 3);
        assert_eq!(d.prefix("ni'").count(), 1);
        assert_eq!(Dictionary::parse(b"ni\tx\n").err(), Some(Error::Header));
        assert_eq!(
            Dictionary::parse(b"# door-ime dictionary v1\nb\tx\na\ty\n").err(),
            Some(Error::Order(3))
        );
        assert_eq!(
            Dictionary::parse(b"# door-ime dictionary v1\na\tx\na\ty\n").err(),
            Some(Error::Order(3))
        );
        assert_eq!(
            Dictionary::parse(b"# door-ime dictionary v1\na\tx,zz\n").err(),
            Some(Error::Line(2))
        );
        assert_eq!(
            Dictionary::parse(b"# door-ime dictionary v1\na\n").err(),
            Some(Error::Line(2))
        );
        assert_eq!(
            Dictionary::parse(b"# door-ime dictionary v1\n\xff\n").err(),
            Some(Error::Utf8)
        );
    }
}
