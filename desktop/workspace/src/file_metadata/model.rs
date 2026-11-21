//! 3D models: glTF (JSON, streamed) and GLB, PLY headers and STL (binary;
//! ASCII STL and OBJ are counted by `text::Stream`).
use super::Section::{Description, Model, Origin};
use super::*;

/// Top-level glTF arrays that are counted.
const ARRAYS: [(&[u8], &str); 8] = [
    (b"meshes", "Meshes"),
    (b"nodes", "Nodes"),
    (b"materials", "Materials"),
    (b"scenes", "Scenes"),
    (b"animations", "Animations"),
    (b"textures", "Textures"),
    (b"images", "Images"),
    (b"skins", "Skins"),
];
const DEPTH: usize = 64;
const CAPTURE: usize = 128;

/// A streaming JSON scanner for glTF: counts the elements of top-level
/// arrays and keeps `asset` strings, for any chunking and any nesting.
#[derive(Clone, Debug, Default)]
pub(super) struct Json {
    stack: Vec<u8>,
    deep: u64,
    in_string: bool,
    escape: bool,
    capture: Option<Vec<u8>>,
    key: bool,
    expect_key: bool,
    in_scalar: bool,
    top_key: Vec<u8>,
    inner_key: Vec<u8>,
    counts: [u64; 8],
    version: Option<String>,
    generator: Option<String>,
    copyright: Option<String>,
}
impl Json {
    fn depth(&self) -> u64 {
        self.stack.len() as u64 + self.deep
    }
    fn value_start(&mut self) {
        if self.deep == 0
            && self.stack.as_slice() == b"{["
            && let Some(i) = ARRAYS
                .iter()
                .position(|(k, _)| *k == self.top_key.as_slice())
        {
            self.counts[i] += 1;
        }
    }
    fn end_scalar(&mut self) {
        self.in_scalar = false;
    }
    fn end_string(&mut self) {
        let text = self.capture.take().unwrap_or_default();
        if self.key {
            match self.depth() {
                1 => self.top_key = text,
                2 => self.inner_key = text,
                _ => (),
            }
        } else if self.depth() == 2 && self.top_key == b"asset" {
            let value = Some(utf8(&text));
            match self.inner_key.as_slice() {
                b"version" => self.version = value,
                b"generator" => self.generator = value,
                b"copyright" => self.copyright = value,
                _ => (),
            }
        }
    }
    pub(super) fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if self.in_string {
                if self.escape {
                    self.escape = false;
                } else if b == b'\\' {
                    self.escape = true;
                    continue;
                } else if b == b'"' {
                    self.in_string = false;
                    self.end_string();
                    continue;
                }
                if let Some(c) = self.capture.as_mut().filter(|c| c.len() < CAPTURE) {
                    c.push(b);
                }
                continue;
            }
            match b {
                b'"' => {
                    let top = self.stack.last().copied();
                    self.key = self.deep == 0 && top == Some(b'{') && self.expect_key;
                    if !self.key {
                        self.value_start();
                    }
                    let depth = self.depth();
                    let wanted = if self.key {
                        depth <= 2
                    } else {
                        depth == 2 && top == Some(b'{') && self.top_key == b"asset"
                    };
                    self.capture = wanted.then(Vec::new);
                    self.in_string = true;
                    self.in_scalar = false;
                }
                b'{' | b'[' => {
                    self.end_scalar();
                    self.value_start();
                    if self.stack.len() < DEPTH && self.deep == 0 {
                        self.stack.push(b);
                    } else {
                        self.deep += 1;
                    }
                    self.expect_key = b == b'{';
                }
                b'}' | b']' => {
                    self.end_scalar();
                    if self.deep > 0 {
                        self.deep -= 1;
                    } else {
                        self.stack.pop();
                    }
                    self.expect_key = false;
                }
                b':' => self.expect_key = false,
                b',' => {
                    self.end_scalar();
                    self.expect_key = self.deep == 0 && self.stack.last() == Some(&b'{');
                }
                c if c.is_ascii_whitespace() => self.end_scalar(),
                _ => {
                    if !self.in_scalar {
                        self.in_scalar = true;
                        self.value_start();
                    }
                }
            }
        }
    }
    pub(super) fn report(&self, r: &mut Report, complete: bool) {
        if let Some(v) = &self.version {
            r.add(Model, "glTF version", v);
        }
        for (i, (_, label)) in ARRAYS.iter().enumerate() {
            if self.counts[i] > 0 || i < 2 {
                r.add(
                    Model,
                    label,
                    format!(
                        "{}{}",
                        if complete { "" } else { "At least " },
                        grouped(self.counts[i])
                    ),
                );
            }
        }
        if let Some(g) = &self.generator {
            r.add(Origin, "Program name", g);
        }
        if let Some(c) = &self.copyright {
            r.add(Origin, "Copyright", c);
        }
    }
}

pub(super) fn glb(d: &Sparse, r: &mut Report) -> Result<(), Miss> {
    r.format("glTF model (binary GLB)");
    let h = d.get(0, 20)?;
    r.add(
        Model,
        "Container",
        format!("GLB version {}", le32(h, 4).unwrap_or(0)),
    );
    let json_len = u64::from(le32(h, 12).unwrap_or(0));
    if &h[16..20] == b"JSON" {
        let len = json_len.min(MAX_FETCH / 2);
        let b = d.clip(20, len)?;
        let mut json = Json::default();
        json.feed(b);
        json.report(r, b.len() as u64 == json_len);
        let bin = 20 + json_len.div_ceil(4) * 4;
        if bin + 8 <= d.size() {
            let c = d.get(bin, 8)?;
            if &c[4..8] == b"BIN\0" {
                r.add(
                    Model,
                    "Binary buffer",
                    size(u64::from(le32(c, 0).unwrap_or(0))),
                );
            }
        }
    }
    Ok(())
}

pub(super) fn ply(d: &Sparse, r: &mut Report) -> Result<(), Miss> {
    r.format("PLY model");
    let b = d.clip(0, 4096)?;
    let end = find(b, b"end_header").ok_or(Miss::Truncated)?;
    for line in b[..end].split(|c| *c == b'\n').take(256) {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let mut words = line.split(|c| *c == b' ').filter(|w| !w.is_empty());
        match (words.next(), words.next(), words.next()) {
            (Some(b"format"), Some(f), _) => r.add(
                Model,
                "Format",
                match f {
                    b"ascii" => "ASCII",
                    b"binary_little_endian" => "Binary (little-endian)",
                    b"binary_big_endian" => "Binary (big-endian)",
                    _ => "",
                },
            ),
            (Some(b"element"), Some(kind), Some(n)) => {
                let n = core::str::from_utf8(n)
                    .ok()
                    .and_then(|n| n.parse::<u64>().ok())
                    .unwrap_or(0);
                match kind {
                    b"vertex" => r.add(Model, "Vertices", grouped(n)),
                    b"face" => r.add(Model, "Faces", grouped(n)),
                    b"edge" => r.add(Model, "Edges", grouped(n)),
                    _ => (),
                }
            }
            (Some(b"comment"), Some(_), _) => r.add(
                Description,
                "Comments",
                latin1(line.get(8..).unwrap_or(&[])),
            ),
            _ => (),
        }
    }
    Ok(())
}

/// Binary STL (header and triangle count), or ASCII STL for the stream.
pub(super) fn stl(d: &Sparse, r: &mut Report) -> Result<Option<StreamKind>, Miss> {
    let size = d.size();
    if size >= 84 {
        let h = d.get(0, 84)?;
        let n = u64::from(le32(h, 80).unwrap_or(0));
        let ascii = h.starts_with(b"solid") && h[..80].iter().all(|c| c.is_ascii());
        if 84 + 50 * n == size || !ascii {
            r.format("STL model (binary)");
            r.add(Model, "Format", "Binary STL");
            r.add(Model, "Triangles", grouped(n));
            let header = latin1(&h[..80]);
            if header.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
                r.add(Description, "Comments", header);
            }
            if 84 + 50 * n != size {
                r.note = Some(TRUNCATED);
            }
            return Ok(None);
        }
    }
    if d.clip(0, 5)?.eq_ignore_ascii_case(b"solid") {
        return Ok(Some(StreamKind::StlAscii));
    }
    r.format("STL model");
    Err(Miss::Truncated)
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    const GLTF: &str = r#"{"asset":{"version":"2.0","generator":"Blender \"4.1\"","extras":{"version":"no"}},
        "scenes":[{"nodes":[0,1]}],"nodes":[{"mesh":0},{"mesh":1,"children":[]}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]},{"name":"b"}],
        "materials":[{"name":"x [y]"}],"buffers":[{"uri":"data:application/octet-stream;base64,AAAA"}]}"#;
    #[test]
    fn gltf_json_counts_in_any_chunking() {
        for chunk in [1, 5, 4096] {
            let mut s = Stream::new(StreamKind::Gltf);
            for c in GLTF.as_bytes().chunks(chunk) {
                s.feed(c);
            }
            let r = s.finish(true);
            assert_eq!(
                (r.get("glTF version"), r.get("Meshes"), r.get("Nodes")),
                (Some("2.0"), Some("2"), Some("2")),
                "chunk {chunk}"
            );
            assert_eq!(
                (r.get("Materials"), r.get("Scenes"), r.get("Program name")),
                (Some("1"), Some("1"), Some("Blender \"4.1\""))
            );
        }
        fuzz("scene.gltf", GLTF.as_bytes());
    }
    #[test]
    fn glb_ply_and_binary_stl() {
        let json = br#"{"asset":{"version":"2.0"},"meshes":[{},{},{}]} "#;
        let mut glb = b"glTF".to_vec();
        glb.extend(le(2, 4));
        glb.extend(le(0, 4));
        glb.extend(le(json.len() as u64, 4));
        glb.extend(b"JSON");
        glb.extend(json);
        glb.extend(le(4096, 4));
        glb.extend(b"BIN\0");
        glb.extend([0; 4096]);
        let r = run("a.glb", &glb);
        assert_eq!(
            (r.get("Meshes"), r.get("Container"), r.get("Binary buffer")),
            (
                Some("3"),
                Some("GLB version 2"),
                Some("4.0 KB (4,096 bytes)")
            )
        );
        fuzz("a.glb", &glb);
        let ply = b"ply\nformat binary_little_endian 1.0\ncomment made by scanner\nelement vertex 8\nproperty float x\nelement face 12\nend_header\n\0\0".to_vec();
        let r = run("a.ply", &ply);
        assert_eq!(
            (
                r.get("Format"),
                r.get("Vertices"),
                r.get("Faces"),
                r.get("Comments")
            ),
            (
                Some("Binary (little-endian)"),
                Some("8"),
                Some("12"),
                Some("made by scanner")
            )
        );
        fuzz("a.ply", &ply);
        let mut stl = alloc::vec![b' '; 80];
        stl[..6].copy_from_slice(b"Bridge");
        stl.extend(le(2, 4));
        stl.extend([0; 100]);
        let r = run("bridge.stl", &stl);
        assert_eq!(
            (r.format, r.get("Triangles"), r.get("Comments")),
            (Some("STL model (binary)"), Some("2"), Some("Bridge"))
        );
        fuzz("bridge.stl", &stl);
    }
}
