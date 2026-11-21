//! Host model of a journaled owner: staged state becomes durable only at
//! `commit`; `crash` discards everything staged since the last commit.
//! Fault injection fails the N-th mutation as an I/O error that poisons the
//! owner (like the real volume) or as a non-poisoning `NoSpace`.
use crate::{Error, Name, Result, Stat, Volume};
use std::collections::BTreeMap;
use std::string::{String, ToString};
use std::vec::Vec;

#[derive(Clone, Debug)]
pub struct Node {
    pub parent: u64,
    pub name: String,
    pub dir: bool,
    pub data: Vec<u8>,
    pub hidden: bool,
    pub version: u64,
}
#[derive(Clone, Debug, Default)]
pub struct Fs {
    pub nodes: BTreeMap<u64, Node>,
    pub next: u64,
    pub records: BTreeMap<u64, Vec<u8>>,
}
pub struct Mem {
    pub durable: Fs,
    pub live: Fs,
    pub poisoned: bool,
    pub commits: usize,
    pub mutations: usize,
    /// Fail the mutation with this ordinal (1-based) as a poisoning I/O error.
    pub fail_at: Option<usize>,
    /// Total data bytes allowed (writes past it fail `NoSpace`).
    pub capacity: u64,
    pub limit: u64,
    pub clock: u64,
}
pub struct Handle(u64);
impl Mem {
    pub fn new() -> Self {
        let mut fs = Fs {
            next: 2,
            ..Fs::default()
        };
        fs.nodes.insert(
            1,
            Node {
                parent: 0,
                name: String::new(),
                dir: true,
                data: Vec::new(),
                hidden: false,
                version: 1,
            },
        );
        let mut mem = Self {
            durable: fs.clone(),
            live: fs,
            poisoned: false,
            commits: 0,
            mutations: 0,
            fail_at: None,
            capacity: u64::MAX,
            limit: 4 << 20,
            clock: 133_000_000_000_000_000,
        };
        mem.mkdir("/users").unwrap();
        mem.mkdir("/users/1001").unwrap();
        mem.commit().unwrap();
        mem
    }
    pub fn crash(&mut self) {
        self.live = self.durable.clone();
        self.poisoned = false;
        self.fail_at = None;
    }
    fn find(&self, path: &str) -> Result<u64> {
        if !path.starts_with('/') {
            return Err(Error::Invalid);
        }
        let mut id = 1;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            id = *self
                .live
                .nodes
                .iter()
                .find(|(_, n)| n.parent == id && n.name.eq_ignore_ascii_case(part))
                .ok_or(Error::NotFound)?
                .0;
        }
        Ok(id)
    }
    fn split(path: &str) -> (&str, &str) {
        let at = path.rfind('/').unwrap();
        (if at == 0 { "/" } else { &path[..at] }, &path[at + 1..])
    }
    fn mutate(&mut self) -> Result<()> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        self.mutations += 1;
        if self.fail_at == Some(self.mutations) {
            self.poisoned = true;
            return Err(Error::Io);
        }
        Ok(())
    }
    fn used(&self) -> u64 {
        self.live.nodes.values().map(|n| n.data.len() as u64).sum()
    }
    pub fn write_file(&mut self, path: &str, data: &[u8]) {
        let f = self.create(path).unwrap();
        self.write(&f, 0, data).unwrap();
        self.close(f).unwrap();
    }
    pub fn read_file(&mut self, path: &str) -> Vec<u8> {
        self.live.nodes[&self.find(path).unwrap()].data.clone()
    }
    pub fn exists(&self, path: &str) -> bool {
        self.find(path).is_ok()
    }
    /// Sorted listing "name" / "name/" of a folder.
    pub fn names(&self, path: &str) -> Vec<String> {
        let id = self.find(path).unwrap();
        let mut v: Vec<String> = self
            .live
            .nodes
            .values()
            .filter(|n| n.parent == id)
            .map(|n| {
                if n.dir {
                    n.name.clone() + "/"
                } else {
                    n.name.clone()
                }
            })
            .collect();
        v.sort();
        v
    }
    /// A canonical dump of the whole tree with file contents.
    pub fn tree(&self) -> Vec<(String, Option<Vec<u8>>)> {
        fn walk(fs: &Fs, id: u64, prefix: &str, out: &mut Vec<(String, Option<Vec<u8>>)>) {
            let mut kids: Vec<_> = fs.nodes.iter().filter(|(_, n)| n.parent == id).collect();
            kids.sort_by(|a, b| a.1.name.cmp(&b.1.name));
            for (k, n) in kids {
                let path = std::format!("{prefix}/{}", n.name);
                if n.dir {
                    out.push((path.clone(), None));
                    walk(fs, *k, &path, out);
                } else {
                    out.push((path, Some(n.data.clone())));
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.live, 1, "", &mut out);
        out
    }
}
impl Volume for Mem {
    type File = Handle;
    fn stat(&mut self, path: &str) -> Result<Stat> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        let id = self.find(path)?;
        let n = &self.live.nodes[&id];
        Ok(Stat {
            object: id,
            version: n.version,
            bytes: n.data.len() as u64,
            directory: n.dir,
            modified: 0,
        })
    }
    fn child(&mut self, folder: &str, after: u64) -> Result<Option<(Name, Stat)>> {
        let id = self.find(folder)?;
        if !self.live.nodes[&id].dir {
            return Err(Error::Invalid);
        }
        Ok(self
            .live
            .nodes
            .iter()
            .find(|(k, n)| n.parent == id && **k > after)
            .map(|(k, n)| {
                (
                    Name::new(&n.name).unwrap(),
                    Stat {
                        object: *k,
                        version: n.version,
                        bytes: n.data.len() as u64,
                        directory: n.dir,
                        modified: 0,
                    },
                )
            }))
    }
    fn mkdir(&mut self, path: &str) -> Result<()> {
        let (parent, name) = Self::split(path);
        let p = self.find(parent)?;
        if self.find(path).is_ok() {
            return Err(Error::Exists);
        }
        self.mutate()?;
        let id = self.live.next;
        self.live.next += 1;
        self.live.nodes.insert(
            id,
            Node {
                parent: p,
                name: name.to_string(),
                dir: true,
                data: Vec::new(),
                hidden: false,
                version: 1,
            },
        );
        Ok(())
    }
    fn create(&mut self, path: &str) -> Result<Handle> {
        let (parent, name) = Self::split(path);
        let p = self.find(parent)?;
        if self.find(path).is_ok() {
            return Err(Error::Exists);
        }
        self.mutate()?;
        let id = self.live.next;
        self.live.next += 1;
        self.live.nodes.insert(
            id,
            Node {
                parent: p,
                name: name.to_string(),
                dir: false,
                data: Vec::new(),
                hidden: false,
                version: 1,
            },
        );
        Ok(Handle(id))
    }
    fn open_read(&mut self, path: &str) -> Result<Handle> {
        let id = self.find(path)?;
        if self.live.nodes[&id].dir {
            return Err(Error::Invalid);
        }
        Ok(Handle(id))
    }
    fn open_write(&mut self, path: &str) -> Result<Handle> {
        self.open_read(path)
    }
    fn read(&mut self, file: &Handle, offset: u64, out: &mut [u8]) -> Result<usize> {
        let data = &self.live.nodes.get(&file.0).ok_or(Error::NotFound)?.data;
        let start = (offset as usize).min(data.len());
        let n = out.len().min(data.len() - start);
        out[..n].copy_from_slice(&data[start..start + n]);
        Ok(n)
    }
    fn write(&mut self, file: &Handle, offset: u64, data: &[u8]) -> Result<()> {
        let grow =
            (offset + data.len() as u64).saturating_sub(self.live.nodes[&file.0].data.len() as u64);
        if self.used() + grow > self.capacity {
            return Err(Error::NoSpace);
        }
        self.mutate()?;
        let node = self.live.nodes.get_mut(&file.0).ok_or(Error::NotFound)?;
        let end = offset as usize + data.len();
        if node.data.len() < end {
            node.data.resize(end, 0);
        }
        node.data[offset as usize..end].copy_from_slice(data);
        node.version += 1;
        Ok(())
    }
    fn truncate(&mut self, file: &Handle, bytes: u64) -> Result<()> {
        if self.live.nodes[&file.0].data.len() as u64 == bytes {
            return Ok(());
        }
        self.mutate()?;
        let node = self.live.nodes.get_mut(&file.0).ok_or(Error::NotFound)?;
        node.data.resize(bytes as usize, 0);
        Ok(())
    }
    fn close(&mut self, _file: Handle) -> Result<()> {
        Ok(())
    }
    fn rename(&mut self, from: &str, to: &str, replace: bool) -> Result<()> {
        let id = self.find(from)?;
        let (parent, name) = Self::split(to);
        let p = self.find(parent)?;
        // Refuse moving a folder into its own subtree.
        let mut a = p;
        while a != 0 {
            if a == id {
                return Err(Error::Invalid);
            }
            a = self.live.nodes[&a].parent;
        }
        if let Ok(existing) = self.find(to)
            && existing != id
        {
            if !replace || self.live.nodes[&existing].dir {
                return Err(Error::Exists);
            }
            self.mutate()?;
            self.live.nodes.remove(&existing);
        } else {
            self.mutate()?;
        }
        let node = self.live.nodes.get_mut(&id).unwrap();
        node.parent = p;
        node.name = name.to_string();
        node.version += 1;
        Ok(())
    }
    fn remove(&mut self, path: &str) -> Result<()> {
        let id = self.find(path)?;
        if id <= 3 {
            return Err(Error::Denied);
        }
        if self.live.nodes.values().any(|n| n.parent == id) {
            return Err(Error::Invalid);
        }
        self.mutate()?;
        self.live.nodes.remove(&id);
        Ok(())
    }
    fn set_hidden(&mut self, path: &str, hidden: bool) -> Result<()> {
        let id = self.find(path)?;
        self.mutate()?;
        self.live.nodes.get_mut(&id).unwrap().hidden = hidden;
        Ok(())
    }
    fn commit(&mut self) -> Result<()> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        self.mutate()?;
        self.durable = self.live.clone();
        self.commits += 1;
        Ok(())
    }
    fn save_record(&mut self, op: u64, bytes: &[u8]) -> Result<()> {
        self.mutate()?;
        self.live.records.insert(op, bytes.to_vec());
        Ok(())
    }
    fn load_record(&mut self, op: u64, out: &mut [u8]) -> Result<Option<usize>> {
        Ok(self.live.records.get(&op).map(|r| {
            out[..r.len()].copy_from_slice(r);
            r.len()
        }))
    }
    fn remove_record(&mut self, op: u64) -> Result<()> {
        self.mutate()?;
        self.live.records.remove(&op);
        Ok(())
    }
    fn next_record(&mut self, after: u64) -> Result<Option<u64>> {
        Ok(self.live.records.range(after + 1..).next().map(|(k, _)| *k))
    }
    fn file_limit(&self) -> u64 {
        self.limit
    }
    fn now(&self) -> u64 {
        self.clock
    }
}
