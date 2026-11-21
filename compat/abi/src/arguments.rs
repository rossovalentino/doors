//! Splitting a command line the way shell32's CommandLineToArgvW does (and
//! CreateProcessW's search for the program name). The program name follows
//! its own rule: it ends at the next quote when it starts with one,
//! otherwise at the first space or tab, with no backslash escapes. Every
//! later argument: 2n backslashes before a quote are n backslashes and the
//! quote toggles quoting; 2n+1 backslashes before a quote are n
//! backslashes and a literal quote; other backslashes are literal; inside
//! quotes, a run of quotes makes one literal quote for every third.

/// The arguments of `line`, each written to `out` followed by a NUL, the
/// offset of each start in `starts`; the number of arguments. None when
/// `out` or `starts` is too small. An empty line has no arguments (the
/// caller supplies the program name, as CommandLineToArgvW does).
pub fn split(line: &[u16], out: &mut [u16], starts: &mut [usize]) -> Option<usize> {
    let blank = |u: u16| u == u16::from(b' ') || u == u16::from(b'\t');
    let quote = u16::from(b'"');
    let backslash = u16::from(b'\\');
    let line = &line[..line.iter().position(|&u| u == 0).unwrap_or(line.len())];
    if line.is_empty() {
        return Some(0);
    }
    let mut d = 0usize;
    let mut push = |d: &mut usize, unit: u16| -> Option<()> {
        *out.get_mut(*d)? = unit;
        *d += 1;
        Some(())
    };
    let mut argc = 0usize;
    let mut start = |argc: &mut usize, at: usize| -> Option<()> {
        *starts.get_mut(*argc)? = at;
        *argc += 1;
        Some(())
    };
    // The program name.
    start(&mut argc, d)?;
    let mut s = 0usize;
    if line[0] == quote {
        s = 1;
        while s < line.len() {
            if line[s] == quote {
                s += 1;
                break;
            }
            push(&mut d, line[s])?;
            s += 1;
        }
    } else {
        while s < line.len() && !blank(line[s]) {
            push(&mut d, line[s])?;
            s += 1;
        }
    }
    push(&mut d, 0)?;
    while s < line.len() && blank(line[s]) {
        s += 1;
    }
    if s == line.len() {
        return Some(argc);
    }
    start(&mut argc, d)?;
    let (mut quotes, mut slashes) = (0u32, 0usize);
    while s < line.len() {
        let c = line[s];
        if blank(c) && quotes == 0 {
            push(&mut d, 0)?;
            slashes = 0;
            while s < line.len() && blank(line[s]) {
                s += 1;
            }
            if s < line.len() {
                start(&mut argc, d)?;
            }
        } else if c == backslash {
            push(&mut d, c)?;
            slashes += 1;
            s += 1;
        } else if c == quote {
            if slashes % 2 == 0 {
                // Half the backslashes stay; the quote toggles quoting.
                d -= slashes / 2;
                quotes += 1;
            } else {
                // Half the backslashes stay, then a literal quote.
                d -= slashes / 2 + 1;
                push(&mut d, quote)?;
            }
            s += 1;
            slashes = 0;
            while s < line.len() && line[s] == quote {
                quotes += 1;
                if quotes == 3 {
                    push(&mut d, quote)?;
                    quotes = 0;
                }
                s += 1;
            }
            if quotes == 2 {
                quotes = 0;
            }
        } else {
            push(&mut d, c)?;
            slashes = 0;
            s += 1;
        }
    }
    push(&mut d, 0)?;
    Some(argc)
}

/// The program names CreateProcessW tries for `line` when it has no
/// application name, in order: a quoted first token alone; otherwise each
/// run up to a space ("C:\Program", then "C:\Program Files\My", ...) and
/// then the whole first token through the line's end of non-blank text.
/// Each candidate is the length of `line`'s prefix it covers, after the
/// opening quote when there is one: (skip, length).
pub fn program_candidates(line: &[u16], out: &mut [(usize, usize)]) -> usize {
    let line = &line[..line.iter().position(|&u| u == 0).unwrap_or(line.len())];
    let quote = u16::from(b'"');
    if line.first() == Some(&quote) {
        let end = line[1..]
            .iter()
            .position(|&u| u == quote)
            .map_or(line.len() - 1, |i| i);
        if let Some(slot) = out.first_mut() {
            *slot = (1, end);
            return 1;
        }
        return 0;
    }
    let mut count = 0;
    for (i, &u) in line.iter().enumerate() {
        if (u == u16::from(b' ') || u == u16::from(b'\t')) && i > 0 && count < out.len() {
            out[count] = (0, i);
            count += 1;
            if u == u16::from(b'\t') {
                return count;
            }
        }
    }
    if count < out.len() && !line.is_empty() {
        out[count] = (0, line.len());
        count += 1;
    }
    count
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::{string::String, vec::Vec};

    fn args(line: &str) -> Vec<String> {
        let units: Vec<u16> = line.encode_utf16().collect();
        let mut out = [0u16; 512];
        let mut starts = [0usize; 32];
        let n = split(&units, &mut out, &mut starts).unwrap();
        (0..n)
            .map(|i| {
                let from = starts[i];
                let end = out[from..].iter().position(|&u| u == 0).unwrap() + from;
                String::from_utf16(&out[from..end]).unwrap()
            })
            .collect()
    }

    #[test]
    fn the_documented_examples_split_like_windows() {
        // Microsoft's "Parsing C++ command-line arguments" table, after
        // a program name.
        assert_eq!(args(r#"p "abc" d e"#), ["p", "abc", "d", "e"]);
        assert_eq!(args(r#"p a\\b d"e f"g h"#), ["p", r"a\\b", "de fg", "h"]);
        assert_eq!(args(r#"p a\\\"b c d"#), ["p", r#"a\"b"#, "c", "d"]);
        assert_eq!(args(r#"p a\\\\"b c" d e"#), ["p", r"a\\b c", "d", "e"]);
        // shell32 differs from the C runtime here (observed on Windows 11:
        // CommandLineToArgvW gives three arguments where the CRT gives one).
        assert_eq!(args(r#"p a"b"" c d"#), ["p", r#"ab""#, "c", "d"]);
        assert_eq!(args(r#"p "a"""b" c"#), ["p", r#"a"b"#, "c"]);
    }

    #[test]
    fn the_program_name_has_its_own_rule() {
        assert_eq!(
            args(r#""C:\Program Files\a.exe" x"#),
            [r"C:\Program Files\a.exe", "x"]
        );
        // No escapes in the program name; it ends at the next quote.
        assert_eq!(args(r#""C:\dir\"x y"#), [r"C:\dir\", "x", "y"]);
        assert_eq!(
            args(r"C:\a\b.exe   two  three "),
            [r"C:\a\b.exe", "two", "three"]
        );
        assert_eq!(args("solo"), ["solo"]);
        assert!(args("").is_empty());
        // Quotes inside a later argument open and close quoting.
        assert_eq!(args(r#"p "" "a""b""#), ["p", "", r#"a"b"#]);
    }

    #[test]
    fn small_buffers_are_refused_not_overrun() {
        let units: Vec<u16> = "p abc def".encode_utf16().collect();
        let mut out = [0u16; 5];
        let mut starts = [0usize; 8];
        assert_eq!(split(&units, &mut out, &mut starts), None);
        let mut out = [0u16; 64];
        let mut starts = [0usize; 2];
        assert_eq!(split(&units, &mut out, &mut starts), None);
    }

    #[test]
    fn create_process_tries_each_space_in_an_unquoted_name() {
        let line: Vec<u16> = r"C:\Program Files\My App\app.exe -x"
            .encode_utf16()
            .collect();
        let mut out = [(0, 0); 8];
        let n = program_candidates(&line, &mut out);
        let text =
            |(skip, len): (usize, usize)| String::from_utf16(&line[skip..skip + len]).unwrap();
        let found: Vec<String> = out[..n].iter().map(|&c| text(c)).collect();
        assert_eq!(
            found,
            [
                r"C:\Program",
                r"C:\Program Files\My",
                r"C:\Program Files\My App\app.exe",
                r"C:\Program Files\My App\app.exe -x"
            ]
        );
        let quoted: Vec<u16> = r#""C:\a b\c.exe" -x"#.encode_utf16().collect();
        let n = program_candidates(&quoted, &mut out);
        assert_eq!((n, out[0]), (1, (1, 12)));
    }
}
