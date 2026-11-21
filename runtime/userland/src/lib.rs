#![no_std]
extern crate alloc;
use alloc::{string::String, vec::Vec};
use door_abi::platform::StartupV2;
use door_service_client::{Console, Error, Result};

pub const MAX_LINE_BYTES: usize = 2048;
pub const MAX_WORDS: usize = 32;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Redirect {
    pub path: String,
    pub append: bool,
}
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Command {
    pub words: Vec<String>,
    pub output: Option<Redirect>,
}
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ParseError {
    TooLong,
    TooManyWords,
    UnclosedQuote,
    InvalidRedirect,
    InvalidPath,
}

enum Token {
    Word(String),
    Output(bool),
}

/// Quotes preserve argument boundaries; operators inside quotes are ordinary text.
pub fn parse(line: &str) -> core::result::Result<Command, ParseError> {
    if line.len() > MAX_LINE_BYTES || line.contains('\0') {
        return Err(ParseError::TooLong);
    }
    let mut tokens = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote = None;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else if ch == '\\' && q == '"' {
                word.push(chars.next().ok_or(ParseError::UnclosedQuote)?);
            } else {
                word.push(ch);
            }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
            started = true;
        } else if ch.is_whitespace() || ch == '>' {
            if started {
                tokens.push(Token::Word(core::mem::take(&mut word)));
                started = false;
            }
            if ch == '>' {
                let append = chars.peek() == Some(&'>');
                if append {
                    chars.next();
                }
                tokens.push(Token::Output(append));
            }
        } else if ch == '\\' {
            word.push(chars.next().ok_or(ParseError::UnclosedQuote)?);
            started = true;
        } else {
            word.push(ch);
            started = true;
        }
        if tokens.len() > MAX_WORDS + 2 {
            return Err(ParseError::TooManyWords);
        }
    }
    if quote.is_some() {
        return Err(ParseError::UnclosedQuote);
    }
    if started {
        tokens.push(Token::Word(word));
    }
    let mut words = Vec::new();
    let mut output = None;
    let mut tokens = tokens.into_iter();
    while let Some(token) = tokens.next() {
        match token {
            Token::Word(word) if output.is_none() => words.push(word),
            Token::Output(append) if output.is_none() => {
                let Some(Token::Word(path)) = tokens.next() else {
                    return Err(ParseError::InvalidRedirect);
                };
                if path.is_empty() || tokens.next().is_some() {
                    return Err(ParseError::InvalidRedirect);
                }
                output = Some(Redirect { path, append });
            }
            _ => return Err(ParseError::InvalidRedirect),
        }
    }
    if words.len() > MAX_WORDS {
        return Err(ParseError::TooManyWords);
    }
    if words.is_empty() && output.is_some() {
        return Err(ParseError::InvalidRedirect);
    }
    Ok(Command { words, output })
}

/// Produce an absolute lexical path. Storage still enforces traversal and ACLs.
pub fn resolve(cwd: &str, input: &str) -> core::result::Result<String, ParseError> {
    if !cwd.starts_with('/') || input.contains('\0') || input.contains('\\') {
        return Err(ParseError::InvalidPath);
    }
    let joined = if input.starts_with('/') {
        String::from(input)
    } else if input.is_empty() {
        String::from(cwd)
    } else {
        alloc::format!("{cwd}/{input}")
    };
    let mut parts = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => (),
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    let result = alloc::format!("/{}", parts.join("/"));
    if result.len() > 192 {
        return Err(ParseError::InvalidPath);
    }
    Ok(result)
}

pub fn arguments(startup: &StartupV2) -> Vec<String> {
    strings(&startup.arguments[..usize::from(startup.argument_bytes)])
}
pub fn environment(startup: &StartupV2) -> Vec<String> {
    strings(&startup.environment[..usize::from(startup.environment_bytes)])
}
fn strings(bytes: &[u8]) -> Vec<String> {
    core::str::from_utf8(bytes)
        .map(|s| s.split_terminator('\0').map(String::from).collect())
        .unwrap_or_default()
}
pub fn cwd(startup: &StartupV2) -> String {
    environment(startup)
        .iter()
        .find_map(|s| s.strip_prefix("PWD=").map(String::from))
        .unwrap_or_else(|| alloc::format!("/users/{}", startup.identity.uid))
}

pub fn print(console: &mut Console, mut text: &str) -> Result<()> {
    while !text.is_empty() {
        let mut end = text.len().min(door_service_protocol::CHUNK_BYTES);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        if end == 0 {
            return Err(Error::Malformed);
        }
        let count = console.write(&text[..end])?;
        if count == 0 || count > end || !text.is_char_boundary(count) {
            return Err(Error::Malformed);
        }
        text = &text[count..];
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quoted_arguments_and_redirects_keep_user_data() {
        let cmd = parse("write 'a b.txt' \"hello > world\"").unwrap();
        assert_eq!(cmd.words, ["write", "a b.txt", "hello > world"]);
        assert_eq!(
            parse("echo hi>>out").unwrap().output,
            Some(Redirect {
                path: "out".into(),
                append: true
            })
        );
        assert_eq!(parse("echo ''").unwrap().words, ["echo", ""]);
        for line in ["echo >", "echo > a > b", "echo 'x", ">a"] {
            assert!(parse(line).is_err(), "{line}");
        }
    }
    #[test]
    fn relative_paths_and_parent_navigation_are_normalized() {
        assert_eq!(
            resolve("/users/1001", "../1002/a").unwrap(),
            "/users/1002/a"
        );
        assert_eq!(resolve("/", "../../a//./b").unwrap(), "/a/b");
        assert_eq!(
            resolve("/users/1001", "文档/笔记").unwrap(),
            "/users/1001/文档/笔记"
        );
        assert!(resolve("/", "x\0y").is_err());
        assert!(resolve("relative", "a").is_err());
    }
}
