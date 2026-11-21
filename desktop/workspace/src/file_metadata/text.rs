//! Streaming counters for text-like files: plain text, Markdown and source
//! (lines, words, characters, encoding, line endings), Wavefront OBJ, ASCII
//! STL and glTF JSON. Bytes arrive in arbitrary chunks.
use super::Section::{Content, Description, Model};
use super::model::Json;
use super::*;

/// The language label of a text extension, if it is one.
pub(super) fn language(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "txt" | "text" => "Plain text",
        "md" | "markdown" => "Markdown",
        "log" => "Log file",
        "csv" => "Comma-separated values",
        "tsv" => "Tab-separated values",
        "ini" | "cfg" | "conf" => "Configuration file",
        "json" => "JSON",
        "rs" => "Rust",
        "toml" => "TOML",
        "xml" => "XML",
        "yaml" | "yml" => "YAML",
        "js" | "mjs" | "cjs" => "JavaScript",
        "ts" => "TypeScript",
        "tsx" | "jsx" => "JSX",
        "py" => "Python",
        "c" => "C",
        "h" => "C header",
        "cpp" | "cc" | "cxx" => "C++",
        "hpp" | "hh" => "C++ header",
        "cs" => "C#",
        "java" => "Java",
        "go" => "Go",
        "css" => "CSS",
        "html" | "htm" => "HTML",
        "svg" => "SVG (XML)",
        "sh" | "bash" => "Shell script",
        "bat" | "cmd" => "Batch file",
        "ps1" | "psm1" | "psd1" => "PowerShell",
        "lua" => "Lua",
        "rb" => "Ruby",
        "php" => "PHP",
        "sql" => "SQL",
        "swift" => "Swift",
        "kt" => "Kotlin",
        "asm" | "s" => "Assembly",
        "ld" => "Linker script",
        "mtl" => "Wavefront material",
        "rtf" => "Rich Text Format",
        _ => return None,
    })
}
/// The start of a file reads as text: a BOM, or no NUL bytes and valid
/// UTF-8 (a character cut at the end of the sample is allowed).
pub(super) fn looks_like_text(b: &[u8]) -> bool {
    if b.starts_with(&[0xEF, 0xBB, 0xBF])
        || b.starts_with(&[0xFF, 0xFE])
        || b.starts_with(&[0xFE, 0xFF])
    {
        return true;
    }
    if b.is_empty() || b.contains(&0) {
        return false;
    }
    match core::str::from_utf8(b) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none() && b.len() - e.valid_up_to() < 4,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamKind {
    /// Text with its language label ("Markdown" also counts headings).
    Text(&'static str),
    Obj,
    StlAscii,
    Gltf,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Undecided,
    Bytes {
        bom: bool,
    },
    Utf16 {
        little: bool,
    },
}
/// Line, word and character counts with encoding and line-ending detection.
#[derive(Clone, Debug, Default)]
struct Stats {
    mode: Mode,
    prefix: Vec<u8>,
    odd: Option<u8>,
    breaks: u64,
    crlf: u64,
    lf: u64,
    cr: u64,
    pending_cr: bool,
    trailing: bool,
    words: u64,
    in_word: bool,
    chars: u64,
    nul: u64,
    high: u64,
    need: u8,
    bad: bool,
}
impl Stats {
    fn feed(&mut self, bytes: &[u8]) {
        if self.mode == Mode::Undecided {
            self.prefix.extend_from_slice(bytes);
            if self.prefix.len() < 3 {
                return;
            }
            self.decide();
            return;
        }
        self.run(bytes);
    }
    fn decide(&mut self) {
        let prefix = core::mem::take(&mut self.prefix);
        let (mode, skip) = match prefix.as_slice() {
            [0xEF, 0xBB, 0xBF, ..] => (Mode::Bytes { bom: true }, 3),
            [0xFF, 0xFE, ..] => (Mode::Utf16 { little: true }, 2),
            [0xFE, 0xFF, ..] => (Mode::Utf16 { little: false }, 2),
            _ => (Mode::Bytes { bom: false }, 0),
        };
        self.mode = mode;
        self.run(&prefix[skip.min(prefix.len())..]);
    }
    fn run(&mut self, bytes: &[u8]) {
        match self.mode {
            Mode::Utf16 { little } => {
                let mut rest = bytes;
                if let Some(first) = self.odd.take() {
                    let Some((&second, tail)) = rest.split_first() else {
                        self.odd = Some(first);
                        return;
                    };
                    let unit = if little {
                        u16::from_le_bytes([first, second])
                    } else {
                        u16::from_be_bytes([first, second])
                    };
                    self.unit16(unit);
                    rest = tail;
                }
                let mut pairs = rest.chunks_exact(2);
                for p in &mut pairs {
                    self.unit16(if little {
                        u16::from_le_bytes([p[0], p[1]])
                    } else {
                        u16::from_be_bytes([p[0], p[1]])
                    });
                }
                self.odd = pairs.remainder().first().copied();
            }
            _ => {
                for &b in bytes {
                    self.byte(b);
                }
            }
        }
    }
    fn unit16(&mut self, u: u16) {
        if !(0xDC00..=0xDFFF).contains(&u) {
            self.chars += 1;
        }
        let space = matches!(u, 0x20 | 0x09 | 0x0A | 0x0B | 0x0C | 0x0D | 0xA0 | 0x3000);
        self.unit(u32::from(u), space);
    }
    fn byte(&mut self, b: u8) {
        if b == 0 {
            self.nul += 1;
        }
        if b >= 0x80 {
            self.high += 1;
        }
        // Streaming UTF-8 validation.
        if self.need > 0 {
            if b & 0xC0 == 0x80 {
                self.need -= 1;
            } else {
                self.bad = true;
                self.need = 0;
            }
        }
        if b & 0xC0 != 0x80 {
            self.chars += 1;
            match b {
                0x00..=0x7F => (),
                0xC2..=0xDF => self.need = 1,
                0xE0..=0xEF => self.need = 2,
                0xF0..=0xF4 => self.need = 3,
                _ => self.bad = true,
            }
        }
        self.unit(u32::from(b), b.is_ascii_whitespace() || b == 0x0B);
    }
    fn unit(&mut self, c: u32, space: bool) {
        match c {
            0x0D => {
                if self.pending_cr {
                    self.cr += 1;
                    self.breaks += 1;
                }
                self.pending_cr = true;
                self.trailing = false;
            }
            0x0A => {
                if self.pending_cr {
                    self.crlf += 1;
                    self.pending_cr = false;
                } else {
                    self.lf += 1;
                }
                self.breaks += 1;
                self.trailing = false;
            }
            _ => {
                if self.pending_cr {
                    self.cr += 1;
                    self.breaks += 1;
                    self.pending_cr = false;
                }
                self.trailing = true;
            }
        }
        if space {
            self.in_word = false;
        } else if !self.in_word {
            self.in_word = true;
            self.words += 1;
        }
    }
    fn finish(&mut self) {
        if self.mode == Mode::Undecided {
            self.decide();
        }
        if self.pending_cr {
            self.pending_cr = false;
            self.cr += 1;
            self.breaks += 1;
        }
    }
    fn encoding(&self, complete: bool) -> &'static str {
        match self.mode {
            Mode::Utf16 { little: true } => "UTF-16 LE",
            Mode::Utf16 { little: false } => "UTF-16 BE",
            Mode::Bytes { bom: true } => "UTF-8 with BOM",
            _ if self.nul > 0 => "Binary data or UTF-16 without a BOM",
            _ if self.high == 0 => "ASCII",
            _ if !self.bad && (self.need == 0 || !complete) => "UTF-8",
            _ => "ANSI (8-bit, not UTF-8)",
        }
    }
    fn endings(&self) -> String {
        let kinds = [
            (self.crlf, "Windows (CRLF)"),
            (self.lf, "Unix (LF)"),
            (self.cr, "Classic Mac (CR)"),
        ];
        let used: Vec<_> = kinds.iter().filter(|(n, _)| *n > 0).collect();
        match used.as_slice() {
            [] => String::from("None (a single line)"),
            [(_, name)] => String::from(*name),
            _ => format!(
                "Mixed (CRLF {}, LF {}, CR {})",
                grouped(self.crlf),
                grouped(self.lf),
                grouped(self.cr)
            ),
        }
    }
}

/// Counts a text-like file fed in chunks; `finish` makes the report.
#[derive(Clone, Debug)]
pub struct Stream {
    kind: StreamKind,
    stats: Stats,
    line: Vec<u8>,
    headings: u64,
    obj: [u64; 9],
    materials: Vec<Vec<u8>>,
    stl_name: Option<String>,
    facets: u64,
    solids: u64,
    json: Json,
}
const LINE: usize = 256;
// OBJ counters.
const V: usize = 0;
const VT: usize = 1;
const VN: usize = 2;
const F: usize = 3;
const TRI: usize = 4;
const O: usize = 5;
const G: usize = 6;
const LIB: usize = 7;
const L: usize = 8;
impl Stream {
    pub fn new(kind: StreamKind) -> Self {
        Self {
            kind,
            stats: Stats::default(),
            line: Vec::new(),
            headings: 0,
            obj: [0; 9],
            materials: Vec::new(),
            stl_name: None,
            facets: 0,
            solids: 0,
            json: Json::default(),
        }
    }
    pub fn kind(&self) -> StreamKind {
        self.kind
    }
    pub fn feed(&mut self, bytes: &[u8]) {
        self.stats.feed(bytes);
        match self.kind {
            StreamKind::Gltf => self.json.feed(bytes),
            StreamKind::Text(language) if language != "Markdown" => (),
            _ => {
                for &b in bytes {
                    if b == b'\n' {
                        self.end_line();
                    } else if b != b'\r' && self.line.len() < LINE {
                        self.line.push(b);
                    }
                }
            }
        }
    }
    fn end_line(&mut self) {
        let line = core::mem::take(&mut self.line);
        let text = line.trim_ascii_start();
        let mut words = text
            .split(|c| c.is_ascii_whitespace())
            .filter(|w| !w.is_empty());
        let first = words.next().unwrap_or(&[]);
        match self.kind {
            StreamKind::Text(_) => {
                if text.starts_with(b"#")
                    && text.iter().find(|c| **c != b'#').is_none_or(|c| *c == b' ')
                {
                    self.headings += 1;
                }
            }
            StreamKind::Obj => match first {
                b"v" => self.obj[V] += 1,
                b"vt" => self.obj[VT] += 1,
                b"vn" => self.obj[VN] += 1,
                b"f" => {
                    let corners = words.count() as u64;
                    self.obj[F] += 1;
                    self.obj[TRI] += corners.saturating_sub(2);
                }
                b"o" => self.obj[O] += 1,
                b"g" => self.obj[G] += 1,
                b"l" => self.obj[L] += 1,
                b"mtllib" => self.obj[LIB] += 1,
                b"usemtl" => {
                    let name = words.next().unwrap_or(&[]).to_vec();
                    if self.materials.len() < 1024 && !self.materials.contains(&name) {
                        self.materials.push(name);
                    }
                }
                _ => (),
            },
            StreamKind::StlAscii => match first {
                b"solid" if self.stl_name.is_none() => {
                    self.stl_name = Some(latin1(text.get(5..).unwrap_or(&[])));
                }
                b"facet" => self.facets += 1,
                b"endsolid" => self.solids += 1,
                _ => (),
            },
            StreamKind::Gltf => (),
        }
    }
    /// The report; `complete` is false when only the first `MAX_STREAM`
    /// bytes were fed (counts are then lower limits).
    pub fn finish(&mut self, complete: bool) -> Report {
        if !self.line.is_empty() {
            self.end_line();
        }
        self.stats.finish();
        let mut r = Report::default();
        let at_least =
            |n: u64| format!("{}{}", if complete { "" } else { "At least " }, grouped(n));
        match self.kind {
            StreamKind::Text(language) => {
                r.add(Content, "Language", language);
                let s = &self.stats;
                r.add(Content, "Lines", at_least(s.breaks + u64::from(s.trailing)));
                r.add(Content, "Words", at_least(s.words));
                r.add(Content, "Characters", at_least(s.chars));
                r.add(Content, "Encoding", s.encoding(complete));
                r.add(Content, "Line endings", s.endings());
                if language == "Markdown" {
                    r.add(Content, "Headings", at_least(self.headings));
                }
            }
            StreamKind::Obj => {
                r.format("Wavefront OBJ model");
                for (label, i) in [
                    ("Vertices", V),
                    ("Texture coordinates", VT),
                    ("Normals", VN),
                    ("Faces", F),
                    ("Triangles", TRI),
                    ("Objects", O),
                    ("Groups", G),
                    ("Lines", L),
                    ("Material libraries", LIB),
                ] {
                    if self.obj[i] > 0 || matches!(i, V | F) {
                        r.add(Model, label, at_least(self.obj[i]));
                    }
                }
                if !self.materials.is_empty() {
                    r.add(Model, "Materials", at_least(self.materials.len() as u64));
                }
            }
            StreamKind::StlAscii => {
                r.format("STL model (ASCII)");
                r.add(Model, "Format", "ASCII STL");
                r.add(Model, "Triangles", at_least(self.facets));
                if self.solids > 1 {
                    r.add(Model, "Solids", at_least(self.solids));
                }
                if let Some(name) = &self.stl_name {
                    r.add(Description, "Title", name);
                }
            }
            StreamKind::Gltf => {
                r.format("glTF model (JSON)");
                self.json.report(&mut r, complete);
            }
        }
        if !complete {
            r.note = Some(LIMITED);
        }
        r
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    fn stream(kind: StreamKind, bytes: &[u8], chunk: usize) -> Report {
        let mut s = Stream::new(kind);
        for c in bytes.chunks(chunk.max(1)) {
            s.feed(c);
        }
        s.finish(true)
    }
    #[test]
    fn text_counts_encodings_and_line_endings_in_any_chunking() {
        let text = "# Title\r\n\r\nHello wide world.\r\n## Part\r\nTail without break";
        for chunk in [1, 2, 3, 7, 192] {
            let r = stream(StreamKind::Text("Markdown"), text.as_bytes(), chunk);
            assert_eq!(r.get("Lines"), Some("5"), "chunk {chunk}");
            assert_eq!(r.get("Words"), Some("10"));
            assert_eq!(
                r.get("Characters"),
                Some(format!("{}", text.len()).as_str())
            );
            assert_eq!(r.get("Encoding"), Some("ASCII"));
            assert_eq!(r.get("Line endings"), Some("Windows (CRLF)"));
            assert_eq!(r.get("Headings"), Some("2"));
        }
        let r = run("notes.txt", "caf\u{e9}\nna\u{ef}ve\r\nend\r".as_bytes());
        assert_eq!(
            (r.get("Encoding"), r.get("Characters"), r.get("Lines")),
            (Some("UTF-8"), Some("16"), Some("3"))
        );
        assert_eq!(r.get("Line endings"), Some("Mixed (CRLF 1, LF 1, CR 1)"));
        let utf16: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain(
                "h\u{e9}llo wor\u{1F600}d\n"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes),
            )
            .collect();
        for chunk in [1, 3, 192] {
            let r = stream(StreamKind::Text("Plain text"), &utf16, chunk);
            assert_eq!(
                (
                    r.get("Encoding"),
                    r.get("Words"),
                    r.get("Characters"),
                    r.get("Lines")
                ),
                (Some("UTF-16 LE"), Some("2"), Some("12"), Some("1"))
            );
        }
        let r = run("old.txt", b"na\xEFve\n");
        assert_eq!(r.get("Encoding"), Some("ANSI (8-bit, not UTF-8)"));
        let r = run("x.rs", b"\xEF\xBB\xBFfn main() {}\n");
        assert_eq!(
            (r.get("Language"), r.get("Encoding"), r.get("Characters")),
            (Some("Rust"), Some("UTF-8 with BOM"), Some("13"))
        );
        assert_eq!(run("empty.txt", b"").fields.len(), 0);
        let r = run("unknown.dat", b"just some words\n");
        assert_eq!(r.get("Language"), Some("Plain text"));
        fuzz("notes.md", text.as_bytes());
    }
    #[test]
    fn obj_and_ascii_stl_counts() {
        let obj = b"# cube\nmtllib cube.mtl\no Cube\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvn 0 0 1\nusemtl Red\nf 1 2 3 4\nusemtl Blue\nf 1 2 3\nusemtl Red\n";
        let r = run("cube.obj", obj);
        assert_eq!(r.format, Some("Wavefront OBJ model"));
        assert_eq!(
            (
                r.get("Vertices"),
                r.get("Faces"),
                r.get("Triangles"),
                r.get("Normals")
            ),
            (Some("4"), Some("2"), Some("3"), Some("1"))
        );
        assert_eq!(
            (
                r.get("Objects"),
                r.get("Materials"),
                r.get("Material libraries")
            ),
            (Some("1"), Some("2"), Some("1"))
        );
        fuzz("cube.obj", obj);
        let stl = b"solid bracket\n facet normal 0 0 1\n  outer loop\n   vertex 0 0 0\n  endloop\n endfacet\n facet normal 0 0 1\n endfacet\nendsolid bracket\n";
        let r = run("part.stl", stl);
        assert_eq!(
            (r.format, r.get("Triangles"), r.get("Title")),
            (Some("STL model (ASCII)"), Some("2"), Some("bracket"))
        );
        fuzz("part.stl", stl);
    }
}
