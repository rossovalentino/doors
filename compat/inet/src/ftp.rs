//! FTP as WinINet speaks it (RFC 959 with RFC 2428's EPSV), as data: the
//! server's replies (multi-line ones too), the passive-mode answers, and
//! directory listings in the Unix and the Windows (IIS) formats, which
//! FtpFindFirstFile turns into WIN32_FIND_DATA. The connection itself is
//! the DLL's (`guest`).
use alloc::{string::String, vec::Vec};

/// One complete reply: its code and text (lines joined by "\r\n").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub code: u16,
    pub text: String,
}
impl Reply {
    /// 1xx: more to come; 2xx done; 3xx more needed; 4xx/5xx failed.
    pub fn class(&self) -> u16 {
        self.code / 100
    }
}

/// A reply that is not FTP, or longer than `MAX_REPLY`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Malformed;
/// Reads replies from bytes as they arrive.
#[derive(Default)]
pub struct Replies {
    buffer: Vec<u8>,
}
/// Longest reply kept (a hostile server cannot grow it without bound).
pub const MAX_REPLY: usize = 16 * 1024;

impl Replies {
    /// Add received bytes; Malformed when a reply grows too long.
    pub fn push(&mut self, data: &[u8]) -> Result<(), Malformed> {
        self.buffer.extend_from_slice(data);
        if self.buffer.len() > MAX_REPLY {
            return Err(Malformed);
        }
        Ok(())
    }
    /// The next complete reply, if one has arrived. A malformed first line
    /// is Err.
    pub fn take(&mut self) -> Result<Option<Reply>, Malformed> {
        let mut lines = Vec::new();
        // Raw bytes taken: a server's invalid UTF-8 is shown replaced but
        // never changes what is consumed.
        let mut consumed = 0;
        let mut code = None;
        for raw in self.buffer.split_inclusive(|b| *b == b'\n') {
            if raw.last() != Some(&b'\n') {
                return Ok(None);
            }
            consumed += raw.len();
            let line = String::from_utf8_lossy(raw);
            let line = line.trim_end_matches(['\r', '\n']);
            match code {
                None => {
                    let (c, rest) = parse_code(line).ok_or(Malformed)?;
                    lines.push(String::from(rest.get(1..).unwrap_or("")));
                    if rest.starts_with('-') {
                        code = Some(c);
                    } else {
                        self.buffer.drain(..consumed);
                        return Ok(Some(Reply {
                            code: c,
                            text: lines.join("\r\n"),
                        }));
                    }
                }
                Some(c) => {
                    // The last line repeats the code and a space.
                    if let Some((last, rest)) = parse_code(line)
                        && last == c
                        && !rest.starts_with('-')
                    {
                        lines.push(String::from(rest.get(1..).unwrap_or("")));
                        self.buffer.drain(..consumed);
                        return Ok(Some(Reply {
                            code: c,
                            text: lines.join("\r\n"),
                        }));
                    }
                    lines.push(String::from(line));
                }
            }
        }
        Ok(None)
    }
}
fn parse_code(line: &str) -> Option<(u16, &str)> {
    let digits = line.get(..3)?;
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let rest = &line[3..];
    if !(rest.is_empty() || rest.starts_with(' ') || rest.starts_with('-')) {
        return None;
    }
    Some((digits.parse().ok()?, rest))
}

/// "227 Entering Passive Mode (h1,h2,h3,h4,p1,p2)": the address and port.
pub fn passive(text: &str) -> Option<([u8; 4], u16)> {
    let start = text.find(|c: char| c.is_ascii_digit())?;
    let numbers: Vec<u16> = text[start..]
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .take(6)
        .map(|s| s.parse().ok())
        .collect::<Option<Vec<u16>>>()?;
    if numbers.len() != 6 || numbers.iter().any(|n| *n > 255) {
        return None;
    }
    Some((
        [
            numbers[0] as u8,
            numbers[1] as u8,
            numbers[2] as u8,
            numbers[3] as u8,
        ],
        numbers[4] << 8 | numbers[5],
    ))
}
/// "229 Entering Extended Passive Mode (|||port|)": the port.
pub fn extended_passive(text: &str) -> Option<u16> {
    let open = text.find('(')?;
    let close = text[open..].find(')')? + open;
    let inner = &text[open + 1..close];
    let d = inner.chars().next()?;
    let parts: Vec<&str> = inner.split(d).collect();
    if parts.len() != 5 {
        return None;
    }
    parts[3].parse().ok().filter(|p| *p != 0)
}
/// "257 "/path" created": the quoted path ("" inside is one quote).
pub fn quoted_path(text: &str) -> Option<String> {
    let start = text.find('"')? + 1;
    let mut out = String::new();
    let mut chars = text[start..].chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            if chars.peek() == Some(&'"') {
                chars.next();
                out.push('"');
            } else {
                return Some(out);
            }
        } else {
            out.push(c);
        }
    }
    None
}

/// One directory entry of a listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub name: String,
    pub directory: bool,
    pub size: u64,
    /// (year, month, day, hour, minute); year 0 when the listing gives none
    /// (the caller supplies the current one).
    pub time: (u16, u8, u8, u8, u8),
}
const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
/// A LIST line: Unix ("drwxr-xr-x 2 user group 4096 Nov  8 14:30 name",
/// "-rw-r--r-- 1 u g 13 Jan 2 2025 name") or Windows/IIS
/// ("11-08-26  02:30PM       <DIR>          name", "... 13 name"). Totals,
/// "." and ".." are skipped (None).
pub fn listing_line(line: &str) -> Option<Listed> {
    let line = line.trim_end_matches(['\r', '\n']);
    let first = line.chars().next()?;
    let entry = if first.is_ascii_digit() {
        windows_line(line)?
    } else {
        unix_line(line)?
    };
    if entry.name.is_empty() || entry.name == "." || entry.name == ".." || entry.name.len() > 255 {
        return None;
    }
    Some(entry)
}
/// The rest of `line` after `n` whitespace-separated fields.
fn after_fields(line: &str, n: usize) -> Option<&str> {
    let mut rest = line;
    for _ in 0..n {
        rest = rest.trim_start();
        let end = rest.find(char::is_whitespace)?;
        rest = &rest[end..];
    }
    let rest = rest.strip_prefix(' ').unwrap_or(rest.trim_start());
    Some(rest)
}
fn unix_line(line: &str) -> Option<Listed> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    if fields.len() < 9 || !matches!(fields[0].chars().next(), Some('d' | '-' | 'l')) {
        return None;
    }
    let directory = fields[0].starts_with('d');
    let size = fields[4].parse().ok()?;
    let month = MONTHS
        .iter()
        .position(|m| m.eq_ignore_ascii_case(fields[5]))? as u8
        + 1;
    let day: u8 = fields[6].parse().ok()?;
    let time = match fields[7].split_once(':') {
        Some((h, m)) => (0, month, day, h.parse().ok()?, m.parse().ok()?),
        None => (fields[7].parse().ok()?, month, day, 0, 0),
    };
    let mut name = after_fields(line, 8)?;
    // A link: "name -> target".
    if fields[0].starts_with('l')
        && let Some((n, _)) = name.split_once(" -> ")
    {
        name = n;
    }
    Some(Listed {
        name: String::from(name),
        directory,
        size: if directory { 0 } else { size },
        time,
    })
}
fn windows_line(line: &str) -> Option<Listed> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    if fields.len() < 4 {
        return None;
    }
    let mut date = fields[0].split('-');
    let month: u8 = date.next()?.parse().ok()?;
    let day: u8 = date.next()?.parse().ok()?;
    let year: u16 = date.next()?.parse().ok()?;
    let year = if year < 100 {
        if year < 70 { 2000 + year } else { 1900 + year }
    } else {
        year
    };
    let clock = fields[1];
    let pm = clock.ends_with("PM") || clock.ends_with("pm");
    let clock = clock.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let (h, m) = clock.split_once(':')?;
    let mut hour: u8 = h.parse().ok()?;
    if pm && hour < 12 {
        hour += 12;
    } else if !pm && hour == 12 && fields[1].len() > 5 {
        hour = 0;
    }
    let directory = fields[2].eq_ignore_ascii_case("<DIR>");
    let size = if directory {
        0
    } else {
        fields[2].parse().ok()?
    };
    Some(Listed {
        name: String::from(after_fields(line, 3)?.trim_start()),
        directory,
        size,
        time: (year, month, day, hour, m.parse().ok()?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_utf8_from_a_server_is_consumed_as_sent() {
        // Found by `cargo xtask fuzz ftp`: replacement characters are longer
        // than the bytes they replace.
        let mut r = Replies::default();
        r.push(b"230-\xff\xfe\r\n230 ok\r\n220 next\r\n").unwrap();
        let first = r.take().unwrap().unwrap();
        assert_eq!(first.code, 230);
        assert_eq!(r.take().unwrap().unwrap().code, 220);
        assert_eq!(r.take(), Ok(None));
    }

    #[test]
    fn replies_single_multi_line_and_split_across_reads() {
        let mut r = Replies::default();
        r.push(b"220 Doors test FTP\r\n230-Welcome\r\n line two\r\n230 Logged")
            .unwrap();
        assert_eq!(
            r.take(),
            Ok(Some(Reply {
                code: 220,
                text: String::from("Doors test FTP")
            }))
        );
        assert_eq!(r.take(), Ok(None));
        r.push(b" in\r\n").unwrap();
        let reply = r.take().unwrap().unwrap();
        assert_eq!(reply.code, 230);
        assert_eq!(reply.text, "Welcome\r\n line two\r\nLogged in");
        assert_eq!(reply.class(), 2);
        let mut bad = Replies::default();
        bad.push(b"hello\r\n").unwrap();
        assert_eq!(bad.take(), Err(Malformed));
        let mut flood = Replies::default();
        assert!(flood.push(&[b'1'; MAX_REPLY + 1]).is_err());
    }

    #[test]
    fn passive_answers_and_quoted_paths() {
        assert_eq!(
            passive("Entering Passive Mode (127,0,0,1,195,80)."),
            Some(([127, 0, 0, 1], 50000))
        );
        assert_eq!(passive("Entering Passive Mode (1,2,3)"), None);
        assert_eq!(passive("(1,2,3,4,256,0)"), None);
        assert_eq!(
            extended_passive("Entering Extended Passive Mode (|||50001|)"),
            Some(50001)
        );
        assert_eq!(extended_passive("(|||0|)"), None);
        assert_eq!(extended_passive("no parens"), None);
        assert_eq!(
            quoted_path("\"/pub/\"\"q\"\"\" is current"),
            Some(String::from("/pub/\"q\""))
        );
        assert_eq!(quoted_path("no quote"), None);
    }

    #[test]
    fn unix_and_windows_listings() {
        assert_eq!(
            listing_line("drwxr-xr-x    2 ftp      ftp          4096 Nov  8 14:30 incoming\r\n"),
            Some(Listed {
                name: String::from("incoming"),
                directory: true,
                size: 0,
                time: (0, 11, 8, 14, 30)
            })
        );
        assert_eq!(
            listing_line("-rw-r--r--    1 ftp      ftp            13 Jan  2  2025 hello world.txt"),
            Some(Listed {
                name: String::from("hello world.txt"),
                directory: false,
                size: 13,
                time: (2025, 1, 2, 0, 0)
            })
        );
        assert_eq!(
            listing_line("lrwxrwxrwx 1 u g 4 Nov 8 10:00 latest -> v2"),
            Some(Listed {
                name: String::from("latest"),
                directory: false,
                size: 4,
                time: (0, 11, 8, 10, 0)
            })
        );
        assert_eq!(
            listing_line("11-08-26  02:30PM       <DIR>          reports"),
            Some(Listed {
                name: String::from("reports"),
                directory: true,
                size: 0,
                time: (2026, 11, 8, 14, 30)
            })
        );
        assert_eq!(
            listing_line("11-08-26  12:05AM                   13 hello.txt"),
            Some(Listed {
                name: String::from("hello.txt"),
                directory: false,
                size: 13,
                time: (2026, 11, 8, 0, 5)
            })
        );
        for skipped in [
            "total 12",
            "drwxr-xr-x 2 u g 4096 Nov 8 14:30 .",
            "",
            "garbage line here",
        ] {
            assert_eq!(listing_line(skipped), None, "{skipped}");
        }
    }
}
