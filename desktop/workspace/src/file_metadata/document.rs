//! PDF: version, page count (linearization dictionary or the page tree via
//! the classic cross-reference table), encryption and the Info dictionary.
//! Compressed cross-reference streams are not inflated; their page count is
//! then taken from an uncompressed page tree near the start, if any.
use super::Section::{Content, Description, Origin};
use super::*;

pub(super) fn pdf_header(b: &[u8]) -> Option<usize> {
    find(b, b"%PDF-")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tok<'a> {
    DictOpen,
    DictClose,
    ArrOpen,
    ArrClose,
    Name(&'a [u8]),
    Str(&'a [u8], bool),
    Num(&'a [u8]),
    Word(&'a [u8]),
}
#[derive(Clone, Copy)]
struct Lexer<'a> {
    b: &'a [u8],
    at: usize,
}
fn delimiter(c: u8) -> bool {
    c.is_ascii_whitespace() || b"()<>[]{}/%".contains(&c) || c == 0
}
impl<'a> Iterator for Lexer<'a> {
    type Item = Tok<'a>;
    fn next(&mut self) -> Option<Tok<'a>> {
        let b = self.b;
        loop {
            let c = *b.get(self.at)?;
            if c.is_ascii_whitespace() || c == 0 {
                self.at += 1;
            } else if c == b'%' {
                while self.at < b.len() && !matches!(b[self.at], b'\n' | b'\r') {
                    self.at += 1;
                }
            } else {
                break;
            }
        }
        let start = self.at;
        let c = b[start];
        self.at += 1;
        Some(match c {
            b'<' if b.get(self.at) == Some(&b'<') => {
                self.at += 1;
                Tok::DictOpen
            }
            b'>' if b.get(self.at) == Some(&b'>') => {
                self.at += 1;
                Tok::DictClose
            }
            b'[' => Tok::ArrOpen,
            b']' => Tok::ArrClose,
            b'<' => {
                let end = b[self.at..]
                    .iter()
                    .position(|c| *c == b'>')
                    .map_or(b.len(), |p| self.at + p);
                let s = &b[self.at..end];
                self.at = (end + 1).min(b.len());
                Tok::Str(s, true)
            }
            b'(' => {
                let mut depth = 1;
                let mut i = self.at;
                while i < b.len() && i - self.at < 4096 {
                    match b[i] {
                        b'\\' => i += 1,
                        b'(' => depth += 1,
                        b')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => (),
                    }
                    i += 1;
                }
                let end = i.min(b.len());
                let s = &b[self.at..end];
                self.at = (end + 1).min(b.len());
                Tok::Str(s, false)
            }
            b'/' => {
                while self.at < b.len() && !delimiter(b[self.at]) {
                    self.at += 1;
                }
                Tok::Name(&b[start + 1..self.at])
            }
            _ => {
                while self.at < b.len() && !delimiter(b[self.at]) {
                    self.at += 1;
                }
                let word = &b[start..self.at];
                if word
                    .iter()
                    .all(|c| c.is_ascii_digit() || b"+-.".contains(c))
                {
                    Tok::Num(word)
                } else {
                    Tok::Word(word)
                }
            }
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Value {
    Ref(u64),
    Int(i64),
    Text(Vec<u8>),
    Name(Vec<u8>),
    Other,
}
fn int(word: &[u8]) -> Option<i64> {
    core::str::from_utf8(word).ok()?.parse().ok()
}
/// Literal string escapes, or hex digits.
fn unescape(s: &[u8], hex: bool) -> Vec<u8> {
    let mut out = Vec::new();
    if hex {
        let digits: Vec<u8> = s
            .iter()
            .filter_map(|c| char::from(*c).to_digit(16).map(|d| d as u8))
            .collect();
        for pair in digits.chunks(2) {
            out.push(pair[0] << 4 | pair.get(1).copied().unwrap_or(0));
        }
        return out;
    }
    let mut i = 0;
    while i < s.len() && out.len() < 4096 {
        let c = s[i];
        i += 1;
        if c != b'\\' {
            out.push(c);
            continue;
        }
        let Some(&e) = s.get(i) else { break };
        i += 1;
        match e {
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'0'..=b'7' => {
                let mut v = u32::from(e - b'0');
                for _ in 0..2 {
                    match s.get(i) {
                        Some(d @ b'0'..=b'7') => {
                            v = v * 8 + u32::from(d - b'0');
                            i += 1;
                        }
                        _ => break,
                    }
                }
                out.push(v as u8);
            }
            b'\r' | b'\n' => (),
            other => out.push(other),
        }
    }
    out
}
/// The top-level dictionary starting at the first `<<` of `b`.
fn dict(b: &[u8]) -> Vec<(Vec<u8>, Value)> {
    let mut lexer = Lexer { b, at: 0 };
    let mut out = Vec::new();
    if !lexer.any(|t| t == Tok::DictOpen) {
        return out;
    }
    for _ in 0..128 {
        let key = match lexer.next() {
            Some(Tok::Name(k)) => k.to_vec(),
            _ => return out,
        };
        let value = match lexer.next() {
            Some(Tok::Num(n)) => {
                let mut ahead = lexer;
                match (ahead.next(), ahead.next()) {
                    (Some(Tok::Num(_)), Some(Tok::Word(b"R"))) => {
                        lexer = ahead;
                        int(n)
                            .filter(|v| *v >= 0)
                            .map_or(Value::Other, |v| Value::Ref(v as u64))
                    }
                    _ => int(n).map_or(Value::Other, Value::Int),
                }
            }
            Some(Tok::Str(s, hex)) => Value::Text(unescape(s, hex)),
            Some(Tok::Name(n)) => Value::Name(n.to_vec()),
            Some(Tok::DictOpen | Tok::ArrOpen) => {
                let mut depth = 1;
                for _ in 0..100_000 {
                    match lexer.next() {
                        Some(Tok::DictOpen | Tok::ArrOpen) => depth += 1,
                        Some(Tok::DictClose | Tok::ArrClose) => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        Some(_) => (),
                        None => return out,
                    }
                }
                Value::Other
            }
            Some(Tok::Word(_)) => Value::Other,
            _ => return out,
        };
        out.push((key, value));
    }
    out
}
fn lookup<'v>(d: &'v [(Vec<u8>, Value)], key: &[u8]) -> Option<&'v Value> {
    d.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}
/// PDF text strings: UTF-16BE with a BOM, UTF-8 with a BOM, else
/// PDFDocEncoding (read as Latin-1).
fn text(s: &[u8]) -> String {
    match s {
        [0xFE, 0xFF, rest @ ..] => utf16(rest, false),
        [0xEF, 0xBB, 0xBF, rest @ ..] => utf8(rest),
        _ => latin1(s),
    }
}
/// "D:20240501123456+02'00'" as "2024-05-01 12:34:56 +02:00".
fn pdf_date(s: &str) -> String {
    let d = s.strip_prefix("D:").unwrap_or(s);
    let digits: String = d.chars().take_while(char::is_ascii_digit).collect();
    if digits.len() < 4 {
        return String::from(s);
    }
    let part =
        |a: usize, b: usize, default: &str| String::from(digits.get(a..b).unwrap_or(default));
    let mut out = format!(
        "{}-{}-{}",
        part(0, 4, "0000"),
        part(4, 6, "01"),
        part(6, 8, "01")
    );
    if digits.len() >= 12 {
        out.push_str(&format!(
            " {}:{}:{}",
            part(8, 10, "00"),
            part(10, 12, "00"),
            part(12, 14, "00")
        ));
        let zone = &d[digits.len()..];
        if zone.starts_with('Z') {
            out.push_str(" UTC");
        } else if let Some(sign) = zone.chars().next().filter(|c| *c == '+' || *c == '-') {
            let z: String = zone.chars().filter(char::is_ascii_digit).take(4).collect();
            if z.len() == 4 {
                out.push_str(&format!(" {sign}{}:{}", &z[..2], &z[2..]));
            }
        }
    }
    out
}

/// The byte offset of object `number`, from the classic cross-reference
/// table at `xref` and its earlier sections (/Prev).
fn locate(d: &Sparse, mut xref: u64, number: u64) -> Result<Option<u64>, Miss> {
    for _ in 0..8 {
        if !d.clip(xref, 4)?.starts_with(b"xref") {
            return Ok(None);
        }
        let mut at = xref + 4;
        for _ in 0..4096 {
            let line = d.clip(at, 64)?;
            let mut lexer = Lexer { b: line, at: 0 };
            let (Some(Tok::Num(s)), Some(Tok::Num(c))) = (lexer.next(), lexer.next()) else {
                break;
            };
            let (Some(start), Some(count)) = (int(s), int(c)) else {
                break;
            };
            let (start, count) = (start.max(0) as u64, count.max(0) as u64);
            // Entries start on the next line.
            let mut entries = lexer.at;
            while entries < line.len() && matches!(line[entries], b' ' | b'\r' | b'\n') {
                entries += 1;
            }
            let entries = at + entries as u64;
            if (start..start.saturating_add(count)).contains(&number) {
                let e = d.get(entries + (number - start) * 20, 20)?;
                return Ok((e[17] == b'n')
                    .then(|| int(&e[..10]))
                    .flatten()
                    .map(|v| v.max(0) as u64));
            }
            at = entries.saturating_add(count.saturating_mul(20));
        }
        let trailer = d.clip(at, 4096)?;
        let Some(i) = find(trailer, b"trailer") else {
            return Ok(None);
        };
        match lookup(&dict(&trailer[i..]), b"Prev") {
            Some(Value::Int(prev)) if *prev > 0 && (*prev as u64) < d.size() => xref = *prev as u64,
            _ => return Ok(None),
        }
    }
    Ok(None)
}
fn object(d: &Sparse, xref: Option<u64>, number: u64) -> Result<Vec<(Vec<u8>, Value)>, Miss> {
    let Some(xref) = xref else {
        return Ok(Vec::new());
    };
    let Some(at) = locate(d, xref, number)?.filter(|at| *at < d.size()) else {
        return Ok(Vec::new());
    };
    let b = d.clip(at, 4096)?;
    match find(&b[..b.len().min(48)], b"obj") {
        Some(i) => Ok(dict(&b[i + 3..])),
        None => Ok(Vec::new()),
    }
}

pub(super) fn pdf(d: &Sparse, r: &mut Report, at: usize) -> Result<(), Miss> {
    r.format("PDF document");
    let size = d.size();
    let head = d.clip(0, 2048)?;
    let version: String = head[at + 5..]
        .iter()
        .take_while(|c| c.is_ascii_digit() || **c == b'.')
        .take(4)
        .map(|c| char::from(*c))
        .collect();
    r.add(Content, "PDF version", version);
    let mut pages = None;
    if let Some(i) = find(head, b"/Linearized") {
        let open = rfind(&head[..i], b"<<").unwrap_or(0);
        if let Some(Value::Int(n)) = lookup(&dict(&head[open..]), b"N") {
            pages = Some(*n);
        }
    }
    let tail_len = size.min(2048);
    let tail = d.get(size - tail_len, tail_len)?;
    let xref = rfind(tail, b"startxref").and_then(|i| {
        let mut lexer = Lexer {
            b: &tail[i + 9..],
            at: 0,
        };
        match lexer.next() {
            Some(Tok::Num(n)) => int(n)
                .filter(|v| *v >= 0 && (*v as u64) < size)
                .map(|v| v as u64),
            _ => None,
        }
    });
    // The newest trailer: classic in the tail, or the xref stream's dictionary.
    let trailer = match rfind(tail, b"trailer") {
        Some(i) => dict(&tail[i..]),
        None => match xref {
            Some(x) => {
                let b = d.clip(x, 4096)?;
                find(&b[..b.len().min(48)], b"obj")
                    .map(|i| dict(&b[i + 3..]))
                    .unwrap_or_default()
            }
            None => Vec::new(),
        },
    };
    let encrypted = lookup(&trailer, b"Encrypt").is_some();
    let classic = match xref {
        Some(x) if d.clip(x, 4)?.starts_with(b"xref") => Some(x),
        _ => None,
    };
    if pages.is_none()
        && let Some(Value::Ref(root)) = lookup(&trailer, b"Root")
    {
        let catalog = object(d, classic, *root)?;
        if let Some(Value::Ref(tree)) = lookup(&catalog, b"Pages")
            && let Some(Value::Int(n)) = lookup(&object(d, classic, *tree)?, b"Count")
        {
            pages = Some(*n);
        }
    }
    if pages.is_none() && classic.is_none() {
        // Compressed structure: an uncompressed page tree near the start.
        let b = d.clip(0, 65_536)?;
        let mut from = 0;
        for _ in 0..256 {
            let Some(i) = find(&b[from..], b"/Pages").map(|i| from + i) else {
                break;
            };
            let open = rfind(&b[..i], b"<<").unwrap_or(i);
            let fields = dict(&b[open..]);
            if lookup(&fields, b"Type") == Some(&Value::Name(b"Pages".to_vec()))
                && let Some(Value::Int(n)) = lookup(&fields, b"Count")
            {
                pages = Some(pages.unwrap_or(0).max(*n));
            }
            from = i + 6;
        }
    }
    match pages.filter(|n| *n >= 0) {
        Some(n) => r.add(Content, "Pages", grouped(n as u64)),
        None => r.add(
            Content,
            "Pages",
            "Not available (compressed document structure)",
        ),
    }
    r.add(Content, "Encrypted", if encrypted { "Yes" } else { "No" });
    if !encrypted && let Some(Value::Ref(info)) = lookup(&trailer, b"Info") {
        let info = object(d, classic, *info)?;
        let get = |key: &[u8]| match lookup(&info, key) {
            Some(Value::Text(s)) => text(s),
            _ => String::new(),
        };
        r.add(Description, "Title", get(b"Title"));
        r.add(Description, "Subject", get(b"Subject"));
        r.add(Description, "Tags", get(b"Keywords"));
        r.add(Origin, "Authors", get(b"Author"));
        r.add(Origin, "Program name", get(b"Creator"));
        r.add(Origin, "PDF producer", get(b"Producer"));
        let created = get(b"CreationDate");
        if !created.is_empty() {
            r.add(Origin, "Content created", pdf_date(&created));
        }
        let saved = get(b"ModDate");
        if !saved.is_empty() {
            r.add(Origin, "Date last saved", pdf_date(&saved));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    /// A classic PDF: catalog, page tree, one page, Info; xref table.
    fn pdf_file(info_title: &str) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let objects = [
            String::from("<< /Type /Catalog /Pages 2 0 R >>"),
            String::from("<< /Type /Pages /Kids [3 0 R 5 0 R 6 0 R] /Count 3 >>"),
            String::from("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>"),
            format!(
                "<< /Title ({info_title}) /Author <FEFF0041006E006E0061> /CreationDate (D:20240501123456+02'00') /Producer (Doors) >>"
            ),
        ];
        let mut offsets = Vec::new();
        for (i, o) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for o in &offsets {
            out.extend(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend(
            format!("trailer\n<< /Size 5 /Root 1 0 R /Info 4 0 R >>\nstartxref\n{xref}\n%%EOF\n")
                .as_bytes(),
        );
        out
    }
    #[test]
    fn pdf_version_pages_and_info() {
        let p = pdf_file("Quarterly \\(draft\\) report");
        let r = run("q.pdf", &p);
        assert_eq!(r.format, Some("PDF document"));
        assert_eq!(
            (r.get("PDF version"), r.get("Pages"), r.get("Encrypted")),
            (Some("1.7"), Some("3"), Some("No"))
        );
        assert_eq!(r.get("Title"), Some("Quarterly (draft) report"));
        assert_eq!(r.get("Authors"), Some("Anna"));
        assert_eq!(r.get("Content created"), Some("2024-05-01 12:34:56 +02:00"));
        assert_eq!(r.get("PDF producer"), Some("Doors"));
        fuzz("q.pdf", &p);
    }
    #[test]
    fn linearized_and_compressed_pdfs_still_report_pages() {
        let lin = b"%PDF-1.5\n1 0 obj << /Linearized 1 /L 9000 /N 42 /T 8000 >> endobj\nxxxx\nstartxref\n999999\n%%EOF".to_vec();
        assert_eq!(run("l.pdf", &lin).get("Pages"), Some("42"));
        let stream = b"%PDF-1.6\n4 0 obj << /Type /Pages /Count 7 /Kids [] >> endobj\n9 0 obj << /Type /XRef /Root 1 0 R /Encrypt 8 0 R >> stream\nxx\nendstream\nstartxref\n61\n%%EOF".to_vec();
        let r = run("s.pdf", &stream);
        assert_eq!(
            (r.get("Pages"), r.get("Encrypted")),
            (Some("7"), Some("Yes"))
        );
        fuzz("s.pdf", &stream);
        assert_eq!(pdf_date("D:2024"), "2024-01-01");
    }
}
