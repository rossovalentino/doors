//! The WinINet URL cache's index, as data: entries keyed by URL with their
//! local file (relative to the cache folder), type, size, times, use count
//! and stored response headers; the folder names files go into; the file
//! names CreateUrlCacheEntry hands out; the search patterns
//! FindFirstUrlCacheEntry takes. The DLL keeps the index in
//! `%LOCALAPPDATA%\Microsoft\Windows\INetCache\IE\container.dat` (one
//! entry a line) beside the folders, and the files themselves.
use alloc::{format, string::String, vec::Vec};

/// CacheEntryType bits.
pub mod kind {
    pub const NORMAL: u32 = 0x0000_0001;
    pub const STICKY: u32 = 0x0000_0004;
    pub const EDITED: u32 = 0x0000_0008;
    pub const TRACK_OFFLINE: u32 = 0x0000_0010;
    pub const TRACK_ONLINE: u32 = 0x0000_0020;
    pub const SPARSE: u32 = 0x0001_0000;
    pub const COOKIE: u32 = 0x0010_0000;
    pub const URLHISTORY: u32 = 0x0020_0000;
}
/// An entry the index cannot take: a separator in a field, a file outside
/// the cache, or every entry locked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refused;
/// The cache's four folders (Windows creates four random 8-character
/// names; a new cache takes these from a seed).
pub const FOLDERS: usize = 4;
/// Entries one index holds (the oldest go first beyond it).
pub const MAX_ENTRIES: usize = 2048;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub url: String,
    /// "FOLDER\\name[1].ext", or empty for an entry without a file.
    pub file: String,
    pub kind: u32,
    pub size: u64,
    /// FILETIMEs (100 ns since 1601), 0 when unknown.
    pub modified: u64,
    pub expires: u64,
    pub accessed: u64,
    pub synced: u64,
    pub uses: u32,
    pub hits: u32,
    pub extension: String,
    /// The response head stored with the entry.
    pub headers: Vec<u8>,
    /// RetrieveUrlCacheEntryFile locks held (released by Unlock).
    pub locks: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Index {
    pub folders: Vec<String>,
    pub entries: Vec<Entry>,
}

fn hex(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 2);
    for b in data {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}
/// A field without the separators (tab, newline) a line uses.
fn clean(text: &str) -> bool {
    !text.contains(['\t', '\n', '\r'])
}

impl Index {
    /// A new index with folder names from `seed`.
    pub fn new(seed: u64) -> Self {
        const ALPHABET: &[u8; 36] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let mut state = seed | 1;
        let folders = (0..FOLDERS)
            .map(|_| {
                (0..8)
                    .map(|_| {
                        state ^= state << 13;
                        state ^= state >> 7;
                        state ^= state << 17;
                        ALPHABET[(state % 36) as usize] as char
                    })
                    .collect()
            })
            .collect();
        Self {
            folders,
            entries: Vec::new(),
        }
    }
    pub fn encode(&self) -> String {
        let mut out = String::from("DOORS-URLCACHE 1\n");
        out.push_str(&format!("folders\t{}\n", self.folders.join("\t")));
        for e in &self.entries {
            out.push_str(&format!(
                "entry\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                e.url,
                e.file,
                e.kind,
                e.size,
                e.modified,
                e.expires,
                e.accessed,
                e.synced,
                e.uses,
                e.hits,
                e.extension,
                hex(&e.headers)
            ));
        }
        out
    }
    /// The index a file holds; anything malformed is skipped (a damaged
    /// index loses entries, never the cache).
    pub fn decode(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        if lines.next()? != "DOORS-URLCACHE 1" {
            return None;
        }
        let mut index = Self::default();
        for line in lines {
            let fields: Vec<&str> = line.split('\t').collect();
            match fields.as_slice() {
                ["folders", names @ ..] => {
                    index.folders = names
                        .iter()
                        .filter(|n| n.len() == 8 && n.bytes().all(|b| b.is_ascii_alphanumeric()))
                        .map(|n| String::from(*n))
                        .collect();
                }
                [
                    "entry",
                    url,
                    file,
                    kind,
                    size,
                    modified,
                    expires,
                    accessed,
                    synced,
                    uses,
                    hits,
                    extension,
                    headers,
                ] => {
                    let parsed = (|| {
                        Some(Entry {
                            url: String::from(*url),
                            file: String::from(*file),
                            kind: kind.parse().ok()?,
                            size: size.parse().ok()?,
                            modified: modified.parse().ok()?,
                            expires: expires.parse().ok()?,
                            accessed: accessed.parse().ok()?,
                            synced: synced.parse().ok()?,
                            uses: uses.parse().ok()?,
                            hits: hits.parse().ok()?,
                            extension: String::from(*extension),
                            headers: unhex(headers)?,
                            locks: 0,
                        })
                    })();
                    if let Some(e) = parsed
                        && !e.url.is_empty()
                        && safe_file(&e.file)
                        && index.entries.len() < MAX_ENTRIES
                    {
                        index.entries.push(e);
                    }
                }
                _ => (),
            }
        }
        if index.folders.len() != FOLDERS {
            return None;
        }
        Some(index)
    }
    pub fn find(&self, url: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.url == url)
    }
    /// Add or replace the entry for its URL; the oldest unlocked entries
    /// leave beyond `MAX_ENTRIES`. Answers the files no longer referenced.
    pub fn commit(&mut self, entry: Entry) -> Result<Vec<String>, Refused> {
        if !clean(&entry.url) || !clean(&entry.extension) || !safe_file(&entry.file) {
            return Err(Refused);
        }
        let mut dropped = Vec::new();
        if let Some(at) = self.find(&entry.url) {
            let old = self.entries.remove(at);
            if old.file != entry.file && !old.file.is_empty() {
                dropped.push(old.file);
            }
        }
        while self.entries.len() >= MAX_ENTRIES {
            let Some(oldest) = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.locks == 0)
                .min_by_key(|(_, e)| e.accessed)
                .map(|(i, _)| i)
            else {
                return Err(Refused);
            };
            let e = self.entries.remove(oldest);
            if !e.file.is_empty() {
                dropped.push(e.file);
            }
        }
        self.entries.push(entry);
        Ok(dropped)
    }
    /// A file name for `url` in one of the folders, not taken by another
    /// entry or `taken` (files on disk): the URL's last path segment
    /// without its query, "name[n].ext" as WinINet names them.
    pub fn file_name(
        &self,
        url: &str,
        extension: &str,
        folder: usize,
        taken: &dyn Fn(&str) -> bool,
    ) -> String {
        let path = url.split(['?', '#']).next().unwrap_or(url);
        let last = path.rsplit('/').next().unwrap_or("");
        let (stem, own_extension) = match last.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() => (s, e),
            _ => (last, ""),
        };
        let extension = if extension.is_empty() {
            own_extension
        } else {
            extension.trim_start_matches('.')
        };
        let sanitize = |s: &str, max: usize| -> String {
            s.chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
                .take(max)
                .collect()
        };
        let mut stem = sanitize(stem, 32);
        if stem.is_empty() {
            stem = String::from("index");
        }
        let extension = sanitize(extension, 8);
        let folder = &self.folders[folder % self.folders.len()];
        for n in 1..1000 {
            let name = if extension.is_empty() {
                format!("{folder}\\{stem}[{n}]")
            } else {
                format!("{folder}\\{stem}[{n}].{extension}")
            };
            if !self
                .entries
                .iter()
                .any(|e| e.file.eq_ignore_ascii_case(&name))
                && !taken(&name)
            {
                return name;
            }
        }
        format!("{folder}\\{stem}[x]")
    }
    /// The entries FindFirstUrlCacheEntry(pattern) enumerates: NULL or
    /// "*.*" the normal entries, "cookie:" the cookies, "visited:" the
    /// history; other patterns match URL prefixes.
    pub fn matching(&self, pattern: Option<&str>) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| match pattern {
                None | Some("") | Some("*.*") | Some("*") => {
                    e.kind & (kind::COOKIE | kind::URLHISTORY) == 0
                }
                Some(p) if p.eq_ignore_ascii_case("cookie:") => e.kind & kind::COOKIE != 0,
                Some(p) if p.eq_ignore_ascii_case("visited:") => e.kind & kind::URLHISTORY != 0,
                Some(p) => e.url.len() >= p.len() && e.url[..p.len()].eq_ignore_ascii_case(p),
            })
            .map(|(i, _)| i)
            .collect()
    }
}
/// A relative file in one folder: "FOLDER\\name", nothing that climbs or
/// roots it.
pub fn safe_file(file: &str) -> bool {
    if file.is_empty() {
        return true;
    }
    let Some((folder, name)) = file.split_once('\\') else {
        return false;
    };
    folder.len() == 8
        && folder.bytes().all(|b| b.is_ascii_alphanumeric())
        && !name.is_empty()
        && !name.contains(['\\', '/', ':', '\t', '\n', '\r'])
        && name != "."
        && name != ".."
}

/// Whether a response may be stored: a GET with 200, not `no-store`
/// or `private` caching forbidden by the request flags.
pub fn storable(verb: &str, status: u16, cache_control: Option<&str>) -> bool {
    verb.eq_ignore_ascii_case("GET")
        && status == 200
        && !cache_control.is_some_and(|c| {
            c.split(',')
                .any(|d| d.trim().eq_ignore_ascii_case("no-store"))
        })
}
/// When a stored response goes stale (FILETIME): `max-age` from now, else
/// `Expires`, else 0 (unknown).
pub fn expiry(now: u64, cache_control: Option<&str>, expires: Option<u64>) -> u64 {
    if let Some(age) = cache_control.and_then(|c| {
        c.split(',').find_map(|d| {
            let d = d.trim();
            let (name, value) = d.split_once('=')?;
            name.trim()
                .eq_ignore_ascii_case("max-age")
                .then(|| value.trim().parse::<u64>().ok())?
        })
    }) {
        return now.saturating_add(age.saturating_mul(10_000_000));
    }
    expires.unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(url: &str, file: &str, accessed: u64) -> Entry {
        Entry {
            url: String::from(url),
            file: String::from(file),
            kind: kind::NORMAL,
            size: 13,
            accessed,
            extension: String::from("txt"),
            headers: b"HTTP/1.1 200 OK\r\n\r\n".to_vec(),
            ..Entry::default()
        }
    }

    #[test]
    fn the_index_round_trips_and_damage_only_loses_entries() {
        let mut index = Index::new(42);
        assert_eq!(index.folders.len(), FOLDERS);
        assert!(index.folders.iter().all(|f| f.len() == 8));
        let folder = index.folders[0].clone();
        index
            .commit(entry(
                "http://doors.test/a.txt",
                &format!("{folder}\\a[1].txt"),
                5,
            ))
            .unwrap();
        let text = index.encode();
        assert_eq!(Index::decode(&text), Some(index.clone()));
        let damaged = text.replace("\t13\t", "\tthirteen\t");
        assert_eq!(Index::decode(&damaged).unwrap().entries.len(), 0);
        assert_eq!(Index::decode("something else"), None);
        // A file outside the cache folder is never accepted.
        let escape = text.replace(&format!("{folder}\\a[1].txt"), "..\\..\\evil.exe");
        assert!(Index::decode(&escape).unwrap().entries.is_empty());
        assert!(!safe_file("C:\\Windows\\x"));
        assert!(!safe_file("ABCDEFGH\\..\\x"));
        assert!(!safe_file("ABCDEFGH\\"));
        assert!(safe_file("ABCDEFGH\\page[1].htm"));
    }

    #[test]
    fn names_follow_wininet_and_avoid_collisions() {
        let mut index = Index::new(7);
        let folder = index.folders[1].clone();
        let none = |_: &str| false;
        let name = index.file_name("http://doors.test/dir/page.html?x=1", "", 1, &none);
        assert_eq!(name, format!("{folder}\\page[1].html"));
        index
            .commit(entry("http://doors.test/dir/page.html", &name, 1))
            .unwrap();
        assert_eq!(
            index.file_name("http://other.test/page.html", "", 1, &none),
            format!("{folder}\\page[2].html")
        );
        assert_eq!(
            index.file_name("http://doors.test/", "txt", 1, &none),
            format!("{folder}\\index[1].txt")
        );
        assert_eq!(
            index.file_name("http://doors.test/a b<>|.exe", ".bin", 1, &|n: &str| n
                .ends_with("[1].bin")),
            format!("{folder}\\ab[2].bin")
        );
    }

    #[test]
    fn commits_replace_by_url_and_evict_the_least_recent() {
        let mut index = Index::new(1);
        let f = index.folders[0].clone();
        index
            .commit(entry("http://a/", &format!("{f}\\a[1]"), 1))
            .unwrap();
        let dropped = index
            .commit(entry("http://a/", &format!("{f}\\a[2]"), 2))
            .unwrap();
        assert_eq!(dropped, alloc::vec![format!("{f}\\a[1]")]);
        assert_eq!(index.entries.len(), 1);
        for i in 0..MAX_ENTRIES as u64 {
            index
                .commit(entry(
                    &format!("http://b/{i}"),
                    &format!("{f}\\b[{i}]"),
                    10 + i,
                ))
                .unwrap();
        }
        assert_eq!(index.entries.len(), MAX_ENTRIES);
        // The first entry (least recently used) left.
        assert!(index.find("http://a/").is_none());
        assert!(index.commit(entry("http://c/\tbad", "", 1)).is_err());
    }

    #[test]
    fn patterns_select_kinds_and_freshness_follows_the_headers() {
        let mut index = Index::new(3);
        let mut cookie = entry("Cookie:owner@doors.test/", "", 1);
        cookie.kind = kind::COOKIE;
        let mut visited = entry("Visited:owner@http://doors.test/", "", 1);
        visited.kind = kind::URLHISTORY;
        index.commit(entry("http://doors.test/x", "", 1)).unwrap();
        index.commit(cookie).unwrap();
        index.commit(visited).unwrap();
        assert_eq!(index.matching(None), alloc::vec![0]);
        assert_eq!(index.matching(Some("cookie:")), alloc::vec![1]);
        assert_eq!(index.matching(Some("visited:")), alloc::vec![2]);
        assert_eq!(index.matching(Some("http://DOORS.test/")), alloc::vec![0]);
        assert!(storable("GET", 200, Some("max-age=60")));
        assert!(!storable("GET", 200, Some("private, no-store")));
        assert!(!storable("POST", 200, None));
        assert!(!storable("GET", 404, None));
        assert_eq!(
            expiry(100, Some("public, max-age=2"), Some(7)),
            100 + 20_000_000
        );
        assert_eq!(expiry(100, None, Some(7)), 7);
        assert_eq!(expiry(100, Some("max-age=x"), None), 0);
    }
}
