//! Tokens of the script: names, numbers, strings (single or double quoted,
//! with escapes), punctuation; comments and white space dropped.
use crate::Error;
use alloc::{string::String, vec::Vec};

#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Name(String),
    Number(f64),
    Str(String),
    /// Punctuation and operators, longest match first.
    Punct(&'static str),
}
#[derive(Clone, Debug)]
pub struct Lexed {
    pub token: Token,
    pub line: u32,
}
const PUNCTUATION: [&str; 46] = [
    "===", "!==", ">>>", "...", "==", "!=", "<=", ">=", "&&", "||", "++", "--", "+=", "-=", "*=",
    "/=", "%=", "<<", ">>", "=>", "(", ")", "{", "}", "[", "]", ",", ";", ".", "?", ":", "!", "=",
    "<", ">", "+", "-", "*", "/", "%", "&", "|", "^", "~", "#", "@",
];

pub fn tokens(source: &str) -> Result<Vec<Lexed>, Error> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 1u32;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        // Comments.
        if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            loop {
                if i + 1 >= bytes.len() {
                    return Err(Error::Syntax(line));
                }
                if bytes[i] == b'\n' {
                    line += 1;
                }
                if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    i += 2;
                    break;
                }
                i += 1;
            }
            continue;
        }
        // Names (ASCII letters, digits, _ and $; other characters are not
        // PAC's).
        if b.is_ascii_alphabetic() || b == b'_' || b == b'$' {
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'$')
            {
                i += 1;
            }
            out.push(Lexed {
                token: Token::Name(String::from(&source[start..i])),
                line,
            });
            continue;
        }
        // Numbers: decimal (with a fraction and exponent) or hexadecimal.
        if b.is_ascii_digit() || (b == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            let start = i;
            let value = if b == b'0' && matches!(bytes.get(i + 1), Some(b'x' | b'X')) {
                i += 2;
                let digits = i;
                while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                    i += 1;
                }
                if i == digits || i - digits > 13 {
                    return Err(Error::Syntax(line));
                }
                u64::from_str_radix(&source[digits..i], 16).map_err(|_| Error::Syntax(line))? as f64
            } else {
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                    i += 1;
                }
                if i < bytes.len() && matches!(bytes[i], b'e' | b'E') {
                    i += 1;
                    if i < bytes.len() && matches!(bytes[i], b'+' | b'-') {
                        i += 1;
                    }
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                source[start..i]
                    .parse::<f64>()
                    .map_err(|_| Error::Syntax(line))?
            };
            out.push(Lexed {
                token: Token::Number(value),
                line,
            });
            continue;
        }
        if b == b'"' || b == b'\'' {
            let quote = b;
            i += 1;
            let mut text = String::new();
            loop {
                let Some(&c) = bytes.get(i) else {
                    return Err(Error::Syntax(line));
                };
                if c == quote {
                    i += 1;
                    break;
                }
                if c == b'\n' {
                    return Err(Error::Syntax(line));
                }
                if c == b'\\' {
                    let Some(&e) = bytes.get(i + 1) else {
                        return Err(Error::Syntax(line));
                    };
                    i += 2;
                    match e {
                        b'n' => text.push('\n'),
                        b't' => text.push('\t'),
                        b'r' => text.push('\r'),
                        b'0' => text.push('\0'),
                        b'x' | b'u' => {
                            let width = if e == b'x' { 2 } else { 4 };
                            let digits = source.get(i..i + width).ok_or(Error::Syntax(line))?;
                            let code =
                                u32::from_str_radix(digits, 16).map_err(|_| Error::Syntax(line))?;
                            text.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                            i += width;
                        }
                        b'\n' => line += 1,
                        _ => {
                            // The escaped character itself (UTF-8 safe).
                            let rest = &source[i - 1..];
                            let ch = rest.chars().next().ok_or(Error::Syntax(line))?;
                            text.push(ch);
                            i += ch.len_utf8() - 1;
                        }
                    }
                    continue;
                }
                let ch = source[i..].chars().next().ok_or(Error::Syntax(line))?;
                text.push(ch);
                i += ch.len_utf8();
                if text.len() > crate::MAX_STRING {
                    return Err(Error::Syntax(line));
                }
            }
            out.push(Lexed {
                token: Token::Str(text),
                line,
            });
            continue;
        }
        let rest = &source[i..];
        let Some(p) = PUNCTUATION.iter().find(|p| rest.starts_with(**p)) else {
            return Err(Error::Syntax(line));
        };
        i += p.len();
        out.push(Lexed {
            token: Token::Punct(p),
            line,
        });
    }
    Ok(out)
}
