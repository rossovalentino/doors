//! Doorsh syntax: statements, pipelines of commands, words with
//! interpolation, and expressions.
//!
//! ```text
//! let total = (ls | where type == file | count)
//! if ($total > 10) { echo "big folder" } else { echo "small" }
//! for f in (ls *.txt) { echo $f.name }
//! fn greet(name, greeting = "Hello") { echo "$greeting, $name!" }
//! try { cat missing.txt } catch e { echo "failed: $e" }
//! ls | sort-by size | reverse | first 5 > top.txt
//! ```
//! Arguments are never re-split: a variable holding spaces stays one
//! argument, and quoting is explicit.
use alloc::{boxed::Box, format, string::String, vec::Vec};

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    /// Pipelines joined by `&&` / `||`.
    Chain(Vec<(Joint, Pipeline)>),
    Let(String, Expr),
    /// `$env.NAME = expr`.
    SetEnv(String, Expr),
    If(Vec<(Cond, Vec<Stmt>)>, Option<Vec<Stmt>>),
    For(String, Iter, Vec<Stmt>),
    While(Cond, Vec<Stmt>),
    Loop(Vec<Stmt>),
    Fn(Function),
    Return(Option<Expr>),
    Break,
    Continue,
    Try(Vec<Stmt>, Option<String>, Vec<Stmt>),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Joint {
    First,
    And,
    Or,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Cond {
    Expr(Expr),
    /// A pipeline: true when it succeeds.
    Run(Pipeline),
}
#[derive(Clone, Debug, PartialEq)]
pub enum Iter {
    Expr(Expr),
    Run(Pipeline),
}
#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub name: String,
    pub params: Vec<(String, Option<Expr>)>,
    pub body: Vec<Stmt>,
}
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Pipeline {
    pub commands: Vec<Command>,
}
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Command {
    pub words: Vec<Word>,
    pub redirects: Vec<Redirect>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Redirect {
    pub kind: RedirectKind,
    pub target: Word,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedirectKind {
    Out,
    Append,
    In,
    Err,
    /// `2>&1`
    ErrToOut,
    /// `&>`: output and errors.
    Both,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub parts: Vec<Part>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    /// Unquoted text (globs and `~` expand).
    Bare(String),
    /// Quoted text (no expansion).
    Quoted(String),
    /// `$name.field.0`
    Var(String, Vec<String>),
    /// `$( pipeline )`
    Sub(Pipeline),
    /// `( expr )`, `[ list ]`, `{ record }` as an argument.
    Expr(Expr),
}
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Lit(crate::doorsh::value::Value),
    /// A string with interpolation.
    Interp(Vec<Part>),
    Var(String),
    Sub(Pipeline),
    List(Vec<Expr>),
    Record(Vec<(String, Expr)>),
    Unary(char, Box<Expr>),
    Binary(Box<Expr>, Op, Box<Expr>),
    Field(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    /// `1..5`: the integers from the first to the second, inclusive.
    Range(Box<Expr>, Box<Expr>),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    In,
    Like,
    Concat,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub at: usize,
    pub message: String,
}
type R<T> = Result<T, Error>;

pub fn parse(text: &str) -> R<Vec<Stmt>> {
    let mut p = Parser {
        s: text.as_bytes(),
        text,
        i: 0,
        depth: 0,
    };
    let block = p.block(false)?;
    p.skip_ws(true);
    if p.i < p.s.len() {
        return Err(p.error("unexpected '}'"));
    }
    Ok(block)
}
/// A single expression (`calc`, `where` conditions).
pub fn parse_expr(text: &str) -> R<Expr> {
    let mut p = Parser {
        s: text.as_bytes(),
        text,
        i: 0,
        depth: 0,
    };
    let e = p.expr()?;
    p.skip_ws(false);
    if p.i < p.s.len() {
        return Err(p.error("unexpected text after the expression"));
    }
    Ok(e)
}
const KEYWORDS: [&str; 11] = [
    "let", "if", "for", "while", "loop", "fn", "return", "break", "continue", "try", "else",
];
struct Parser<'a> {
    s: &'a [u8],
    text: &'a str,
    i: usize,
    depth: usize,
}
impl Parser<'_> {
    fn error(&self, message: &str) -> Error {
        Error {
            at: self.i,
            message: String::from(message),
        }
    }
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn peek_at(&self, n: usize) -> Option<u8> {
        self.s.get(self.i + n).copied()
    }
    /// Spaces, comments and (when `newlines`) line breaks and `;`.
    fn skip_ws(&mut self, newlines: bool) {
        while let Some(c) = self.peek() {
            match c {
                b' ' | b'\t' | b'\r' => self.i += 1,
                b'\n' | b';' if newlines => self.i += 1,
                b'\\' if self.peek_at(1) == Some(b'\n') => self.i += 2,
                b'#' => {
                    while self.peek().is_some_and(|c| c != b'\n') {
                        self.i += 1;
                    }
                }
                _ => break,
            }
        }
    }
    fn starts(&self, word: &str) -> bool {
        let rest = &self.s[self.i..];
        rest.starts_with(word.as_bytes())
            && rest.get(word.len()).is_none_or(|c| {
                !(c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-' || *c == b'.')
            })
    }
    fn expect(&mut self, c: u8, what: &str) -> R<()> {
        self.skip_ws(false);
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(self.error(&format!("expected {what}")))
        }
    }
    fn deeper(&mut self) -> R<()> {
        self.depth += 1;
        if self.depth > 64 {
            return Err(self.error("nesting is too deep"));
        }
        Ok(())
    }
    fn block(&mut self, braced: bool) -> R<Vec<Stmt>> {
        self.deeper()?;
        let mut out = Vec::new();
        loop {
            self.skip_ws(true);
            match self.peek() {
                None if braced => return Err(self.error("missing '}'")),
                None => break,
                Some(b'}') if braced => {
                    self.i += 1;
                    break;
                }
                Some(b'}') => break,
                _ => out.push(self.statement()?),
            }
        }
        self.depth -= 1;
        Ok(out)
    }
    fn braced(&mut self) -> R<Vec<Stmt>> {
        self.skip_ws(true);
        self.expect(b'{', "'{'")?;
        self.block(true)
    }
    fn ident(&mut self) -> R<String> {
        self.skip_ws(false);
        let start = self.i;
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        {
            self.i += 1;
        }
        if start == self.i {
            return Err(self.error("expected a name"));
        }
        Ok(String::from(&self.text[start..self.i]))
    }
    fn statement(&mut self) -> R<Stmt> {
        if self.starts("let") {
            self.i += 3;
            let name = self.ident()?;
            self.expect(b'=', "'=' after the variable name")?;
            return Ok(Stmt::Let(name, self.expr()?));
        }
        if self.s[self.i..].starts_with(b"$env.") {
            let save = self.i;
            self.i += 5;
            let name = self.ident()?;
            self.skip_ws(false);
            if self.peek() == Some(b'=') && self.peek_at(1) != Some(b'=') {
                self.i += 1;
                return Ok(Stmt::SetEnv(name, self.expr()?));
            }
            self.i = save;
        }
        if self.starts("if") {
            self.i += 2;
            let mut arms = alloc::vec![(self.cond()?, self.braced()?)];
            let mut otherwise = None;
            loop {
                let save = self.i;
                self.skip_ws(true);
                if !self.starts("else") {
                    self.i = save;
                    break;
                }
                self.i += 4;
                self.skip_ws(false);
                if self.starts("if") {
                    self.i += 2;
                    arms.push((self.cond()?, self.braced()?));
                } else {
                    otherwise = Some(self.braced()?);
                    break;
                }
            }
            return Ok(Stmt::If(arms, otherwise));
        }
        if self.starts("for") {
            self.i += 3;
            let mut name = self.ident()?;
            if let Some(stripped) = name.strip_prefix('$') {
                name = String::from(stripped);
            }
            self.skip_ws(false);
            if !self.starts("in") {
                return Err(self.error("expected 'in'"));
            }
            self.i += 2;
            self.skip_ws(false);
            let iter = if matches!(self.peek(), Some(b'(' | b'[' | b'$' | b'"' | b'\''))
                || self.peek().is_some_and(|c| c.is_ascii_digit())
            {
                Iter::Expr(self.expr()?)
            } else {
                Iter::Run(self.pipeline(true)?)
            };
            return Ok(Stmt::For(name, iter, self.braced()?));
        }
        if self.starts("while") {
            self.i += 5;
            let cond = self.cond()?;
            return Ok(Stmt::While(cond, self.braced()?));
        }
        if self.starts("loop") {
            self.i += 4;
            return Ok(Stmt::Loop(self.braced()?));
        }
        if self.starts("fn") {
            self.i += 2;
            let name = self.ident()?;
            let mut params = Vec::new();
            self.skip_ws(false);
            if self.peek() == Some(b'(') {
                self.i += 1;
                loop {
                    self.skip_ws(true);
                    if self.peek() == Some(b')') {
                        self.i += 1;
                        break;
                    }
                    let p = self.ident()?;
                    self.skip_ws(false);
                    let default = if self.peek() == Some(b'=') {
                        self.i += 1;
                        Some(self.expr()?)
                    } else {
                        None
                    };
                    params.push((p, default));
                    self.skip_ws(true);
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b')') => {}
                        _ => return Err(self.error("expected ',' or ')'")),
                    }
                }
            }
            return Ok(Stmt::Fn(Function {
                name,
                params,
                body: self.braced()?,
            }));
        }
        if self.starts("return") {
            self.i += 6;
            self.skip_ws(false);
            let value = match self.peek() {
                None | Some(b'\n' | b';' | b'}') => None,
                _ => Some(self.expr()?),
            };
            return Ok(Stmt::Return(value));
        }
        if self.starts("break") {
            self.i += 5;
            return Ok(Stmt::Break);
        }
        if self.starts("continue") {
            self.i += 8;
            return Ok(Stmt::Continue);
        }
        if self.starts("try") {
            self.i += 3;
            let body = self.braced()?;
            self.skip_ws(true);
            if !self.starts("catch") {
                return Ok(Stmt::Try(body, None, Vec::new()));
            }
            self.i += 5;
            self.skip_ws(false);
            let name = if self.peek() == Some(b'{') {
                None
            } else {
                let n = self.ident()?;
                Some(String::from(n.trim_start_matches('$')))
            };
            return Ok(Stmt::Try(body, name, self.braced()?));
        }
        let mut chain = alloc::vec![(Joint::First, self.pipeline(false)?)];
        loop {
            self.skip_ws(false);
            if self.s[self.i..].starts_with(b"&&") {
                self.i += 2;
                chain.push((Joint::And, self.pipeline(false)?));
            } else if self.s[self.i..].starts_with(b"||") {
                self.i += 2;
                chain.push((Joint::Or, self.pipeline(false)?));
            } else {
                break;
            }
        }
        Ok(Stmt::Chain(chain))
    }
    fn cond(&mut self) -> R<Cond> {
        self.skip_ws(false);
        if matches!(self.peek(), Some(b'(' | b'$' | b'!'))
            || self.starts("true")
            || self.starts("false")
            || self.starts("not")
        {
            Ok(Cond::Expr(self.expr()?))
        } else {
            Ok(Cond::Run(self.pipeline(true)?))
        }
    }
    /// Commands joined by `|`. `in_header` stops at `{` (an if/for/while
    /// body follows).
    fn pipeline(&mut self, in_header: bool) -> R<Pipeline> {
        let mut commands = Vec::new();
        loop {
            commands.push(self.command(in_header)?);
            self.skip_ws(false);
            if self.peek() == Some(b'|') && self.peek_at(1) != Some(b'|') {
                self.i += 1;
                self.skip_ws(true);
            } else {
                break;
            }
        }
        Ok(Pipeline { commands })
    }
    fn command(&mut self, in_header: bool) -> R<Command> {
        let mut cmd = Command::default();
        // `where` and `calc` read the rest of the command as an expression:
        // `where size > 1mb` compares, it does not redirect.
        self.skip_ws(false);
        for name in ["where", "calc"] {
            if self.starts(name) {
                let start = self.i;
                self.i += name.len();
                cmd.words.push(Word {
                    parts: alloc::vec![Part::Bare(String::from(&self.text[start..self.i]))],
                });
                self.skip_ws(false);
                let from = self.i;
                let mut depth = 0i32;
                let mut quote = None;
                while let Some(c) = self.peek() {
                    match (quote, c) {
                        (Some(q), _) if c == q => quote = None,
                        (Some(_), _) => {}
                        (None, b'\'' | b'"') => quote = Some(c),
                        (None, b'(' | b'[' | b'{') => depth += 1,
                        (None, b')' | b']' | b'}') if depth > 0 => depth -= 1,
                        (None, b')' | b'}' | b'\n' | b';') if depth == 0 => break,
                        (None, b'|') if depth == 0 && self.peek_at(1) != Some(b'|') => break,
                        _ => {}
                    }
                    self.i += 1;
                }
                let raw = self.text[from..self.i].trim_end();
                if !raw.is_empty() {
                    cmd.words.push(Word {
                        parts: alloc::vec![Part::Quoted(String::from(raw))],
                    });
                }
                return Ok(cmd);
            }
        }
        loop {
            self.skip_ws(false);
            let Some(c) = self.peek() else { break };
            if matches!(c, b'\n' | b';' | b'|' | b')')
                || (c == b'}')
                || (c == b'&' && self.peek_at(1) == Some(b'&'))
            {
                break;
            }
            if c == b'{' && in_header {
                break;
            }
            if let Some(kind) = self.redirect_op() {
                let target = if kind == RedirectKind::ErrToOut {
                    Word { parts: Vec::new() }
                } else {
                    self.skip_ws(false);
                    self.word(in_header)?
                };
                cmd.redirects.push(Redirect { kind, target });
                continue;
            }
            cmd.words.push(self.word(in_header)?);
        }
        if cmd.words.is_empty() {
            return Err(self.error("expected a command"));
        }
        Ok(cmd)
    }
    fn redirect_op(&mut self) -> Option<RedirectKind> {
        let rest = &self.s[self.i..];
        let (kind, len) = if rest.starts_with(b"2>&1") {
            (RedirectKind::ErrToOut, 4)
        } else if rest.starts_with(b"&>") {
            (RedirectKind::Both, 2)
        } else if rest.starts_with(b"2>") {
            (RedirectKind::Err, 2)
        } else if rest.starts_with(b">>") {
            (RedirectKind::Append, 2)
        } else if rest.starts_with(b">") {
            (RedirectKind::Out, 1)
        } else if rest.starts_with(b"<") {
            (RedirectKind::In, 1)
        } else {
            return None;
        };
        self.i += len;
        Some(kind)
    }
    fn word(&mut self, in_header: bool) -> R<Word> {
        let mut parts = Vec::new();
        let mut bare = String::new();
        let flush = |bare: &mut String, parts: &mut Vec<Part>| {
            if !bare.is_empty() {
                parts.push(Part::Bare(core::mem::take(bare)));
            }
        };
        // A whole-word expression: ( ... ), [ ... ].
        match self.peek() {
            Some(b'(') => {
                self.i += 1;
                let e = self.paren_inner()?;
                parts.push(Part::Expr(e));
                return Ok(Word { parts });
            }
            Some(b'[') if !self.glob_class_ahead() => {
                let e = self.primary()?;
                parts.push(Part::Expr(e));
                return Ok(Word { parts });
            }
            Some(b'{') if !in_header && self.record_ahead() => {
                let e = self.primary()?;
                parts.push(Part::Expr(e));
                return Ok(Word { parts });
            }
            _ => {}
        }
        // Braces inside a word (`file-{a,b}.txt`) belong to it.
        let mut braces = 0;
        while let Some(c) = self.peek() {
            match c {
                b' ' | b'\t' | b'\r' | b'\n' | b';' | b'|' | b')' => break,
                b'}' if braces > 0 => {
                    braces -= 1;
                    bare.push('}');
                    self.i += 1;
                }
                b'}' => break,
                b'{' if in_header => break,
                b'{' => {
                    braces += 1;
                    bare.push('{');
                    self.i += 1;
                }
                b'>' | b'<' => break,
                b'&' if self.peek_at(1) == Some(b'&') || self.peek_at(1) == Some(b'>') => break,
                b'\'' => {
                    flush(&mut bare, &mut parts);
                    self.i += 1;
                    let start = self.i;
                    while self.peek().is_some_and(|c| c != b'\'') {
                        self.i += 1;
                    }
                    if self.peek().is_none() {
                        return Err(self.error("missing closing '"));
                    }
                    parts.push(Part::Quoted(String::from(&self.text[start..self.i])));
                    self.i += 1;
                }
                b'"' => {
                    flush(&mut bare, &mut parts);
                    self.i += 1;
                    parts.extend(self.interpolated()?);
                }
                b'$' => {
                    flush(&mut bare, &mut parts);
                    parts.push(self.dollar()?);
                }
                b'\\'
                    if self.peek_at(1).is_some_and(|c| {
                        matches!(
                            c,
                            b' ' | b'"' | b'\'' | b'$' | b'|' | b';' | b'&' | b'(' | b')'
                        )
                    }) =>
                {
                    bare.push(self.peek_at(1).unwrap() as char);
                    self.i += 2;
                }
                _ => {
                    let ch = self.text[self.i..].chars().next().unwrap();
                    bare.push(ch);
                    self.i += ch.len_utf8();
                }
            }
        }
        flush(&mut bare, &mut parts);
        if parts.is_empty() {
            return Err(self.error("expected a word"));
        }
        Ok(Word { parts })
    }
    /// `{key: value}` (a field name, then ':') or `{}` is a record;
    /// `{a,b}` stays brace expansion.
    fn record_ahead(&self) -> bool {
        let rest = self.text[self.i + 1..].trim_start();
        if rest.starts_with('}') {
            return true;
        }
        let key_end = if rest.starts_with(['"', '\'']) {
            rest[1..].find(['"', '\'']).map(|i| i + 2)
        } else {
            rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        };
        key_end.is_some_and(|k| k > 0 && rest[k..].trim_start().starts_with(':'))
    }
    /// `[abc]` immediately followed by word text is a glob class, not a list.
    fn glob_class_ahead(&self) -> bool {
        let rest = &self.s[self.i..];
        let Some(end) = rest.iter().position(|c| *c == b']') else {
            return false;
        };
        !rest[1..end].contains(&b',')
            && !rest[1..end].contains(&b' ')
            && rest
                .get(end + 1)
                .is_some_and(|c| !matches!(c, b' ' | b'\t' | b'\n' | b';' | b'|' | b')'))
    }
    /// After `$`: `$name`, `$name.field`, `$?`, `$_`, `${expr}`, `$(...)`.
    fn dollar(&mut self) -> R<Part> {
        self.i += 1;
        match self.peek() {
            Some(b'(') => {
                self.i += 1;
                let p = self.pipeline(false)?;
                self.expect(b')', "')'")?;
                Ok(Part::Sub(p))
            }
            Some(b'{') => {
                self.i += 1;
                let e = self.expr()?;
                self.expect(b'}', "'}'")?;
                Ok(Part::Expr(e))
            }
            Some(b'?') => {
                self.i += 1;
                Ok(Part::Var(String::from("?"), Vec::new()))
            }
            Some(b'_') if !self.peek_at(1).is_some_and(|c| c.is_ascii_alphanumeric()) => {
                self.i += 1;
                Ok(Part::Var(String::from("_"), Vec::new()))
            }
            Some(c) if c.is_ascii_alphabetic() || c == b'_' => {
                let start = self.i;
                while self
                    .peek()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_')
                {
                    self.i += 1;
                }
                let name = String::from(&self.text[start..self.i]);
                let mut path = Vec::new();
                while self.peek() == Some(b'.')
                    && self
                        .peek_at(1)
                        .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
                {
                    self.i += 1;
                    let start = self.i;
                    while self
                        .peek()
                        .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
                    {
                        self.i += 1;
                    }
                    path.push(String::from(&self.text[start..self.i]));
                }
                Ok(Part::Var(name, path))
            }
            _ => Ok(Part::Bare(String::from("$"))),
        }
    }
    /// The inside of "...", after the opening quote.
    fn interpolated(&mut self) -> R<Vec<Part>> {
        let mut parts = Vec::new();
        let mut text = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(self.error("missing closing \""));
            };
            match c {
                b'"' => {
                    self.i += 1;
                    break;
                }
                b'\\' => {
                    let e = self
                        .peek_at(1)
                        .ok_or_else(|| self.error("missing closing \""))?;
                    self.i += 2;
                    match e {
                        b'n' => text.push('\n'),
                        b't' => text.push('\t'),
                        b'r' => text.push('\r'),
                        b'e' => text.push('\x1b'),
                        b'0' => text.push('\0'),
                        b'"' | b'\\' | b'$' => text.push(e as char),
                        other => {
                            text.push('\\');
                            text.push(other as char);
                        }
                    }
                }
                b'$' if self.peek_at(1).is_some_and(|c| {
                    c.is_ascii_alphabetic() || matches!(c, b'(' | b'{' | b'?' | b'_')
                }) =>
                {
                    if !text.is_empty() {
                        parts.push(Part::Quoted(core::mem::take(&mut text)));
                    }
                    parts.push(self.dollar()?);
                }
                _ => {
                    let ch = self.text[self.i..].chars().next().unwrap();
                    text.push(ch);
                    self.i += ch.len_utf8();
                }
            }
        }
        if !text.is_empty() || parts.is_empty() {
            parts.push(Part::Quoted(text));
        }
        Ok(parts)
    }
    /// After `(`: a pipeline when it starts with a command name, else an
    /// expression; consumes the `)`.
    fn paren_inner(&mut self) -> R<Expr> {
        self.skip_ws(true);
        let starts_command = self.peek().is_some_and(|c| c.is_ascii_alphabetic())
            && !["true", "false", "null"].iter().any(|k| self.starts(k))
            && !self.number_ahead();
        let e = if starts_command {
            Expr::Sub(self.pipeline(false)?)
        } else {
            self.expr()?
        };
        self.skip_ws(true);
        self.expect(b')', "')'")?;
        Ok(e)
    }
    fn number_ahead(&self) -> bool {
        self.peek().is_some_and(|c| c.is_ascii_digit())
    }
    pub fn expr(&mut self) -> R<Expr> {
        self.deeper()?;
        let e = self.binary(0)?;
        self.depth -= 1;
        Ok(e)
    }
    fn op_here(&self) -> Option<(Op, usize, u8)> {
        let rest = &self.s[self.i..];
        let word = |w: &str| {
            rest.starts_with(w.as_bytes())
                && rest.get(w.len()).is_none_or(|c| !c.is_ascii_alphanumeric())
        };
        Some(if rest.starts_with(b"||") || word("or") {
            (Op::Or, 2, 1)
        } else if rest.starts_with(b"&&") || word("and") {
            (Op::And, if rest.starts_with(b"&&") { 2 } else { 3 }, 2)
        } else if rest.starts_with(b"==") {
            (Op::Eq, 2, 3)
        } else if rest.starts_with(b"!=") {
            (Op::Ne, 2, 3)
        } else if rest.starts_with(b"=~") || word("like") {
            (Op::Like, if rest.starts_with(b"=~") { 2 } else { 4 }, 3)
        } else if rest.starts_with(b"<=") {
            (Op::Le, 2, 3)
        } else if rest.starts_with(b">=") {
            (Op::Ge, 2, 3)
        } else if rest.starts_with(b"<") {
            (Op::Lt, 1, 3)
        } else if rest.starts_with(b">") {
            (Op::Gt, 1, 3)
        } else if word("in") {
            (Op::In, 2, 3)
        } else if rest.starts_with(b"++") {
            (Op::Concat, 2, 4)
        } else if rest.starts_with(b"+") {
            (Op::Add, 1, 5)
        } else if rest.starts_with(b"-") {
            (Op::Sub, 1, 5)
        } else if rest.starts_with(b"**") {
            (Op::Pow, 2, 7)
        } else if rest.starts_with(b"*") {
            (Op::Mul, 1, 6)
        } else if rest.starts_with(b"/") {
            (Op::Div, 1, 6)
        } else if rest.starts_with(b"%") || word("mod") {
            (Op::Rem, if rest.starts_with(b"%") { 1 } else { 3 }, 6)
        } else {
            return None;
        })
    }
    fn binary(&mut self, min: u8) -> R<Expr> {
        let mut left = self.unary()?;
        loop {
            self.skip_ws(false);
            let Some((op, len, prec)) = self.op_here() else {
                break;
            };
            if prec < min {
                break;
            }
            self.i += len;
            // ** is right-associative.
            let next = if op == Op::Pow { prec } else { prec + 1 };
            let right = self.binary(next)?;
            left = Expr::Binary(Box::new(left), op, Box::new(right));
        }
        Ok(left)
    }
    fn unary(&mut self) -> R<Expr> {
        self.skip_ws(false);
        match self.peek() {
            Some(b'!') => {
                self.i += 1;
                Ok(Expr::Unary('!', Box::new(self.unary()?)))
            }
            Some(b'-') if !self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) => {
                self.i += 1;
                Ok(Expr::Unary('-', Box::new(self.unary()?)))
            }
            _ if self.starts("not") => {
                self.i += 3;
                Ok(Expr::Unary('!', Box::new(self.unary()?)))
            }
            _ => self.postfix(),
        }
    }
    fn postfix(&mut self) -> R<Expr> {
        let mut e = self.primary()?;
        loop {
            match self.peek() {
                Some(b'.')
                    if self
                        .peek_at(1)
                        .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_') =>
                {
                    self.i += 1;
                    let start = self.i;
                    while self
                        .peek()
                        .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
                    {
                        self.i += 1;
                    }
                    e = Expr::Field(Box::new(e), String::from(&self.text[start..self.i]));
                }
                Some(b'[') => {
                    self.i += 1;
                    let index = self.expr()?;
                    self.expect(b']', "']'")?;
                    e = Expr::Index(Box::new(e), Box::new(index));
                }
                _ => break,
            }
        }
        Ok(e)
    }
    fn primary(&mut self) -> R<Expr> {
        use crate::doorsh::value::{Value, parse_number};
        self.skip_ws(false);
        let Some(c) = self.peek() else {
            return Err(self.error("expected a value"));
        };
        match c {
            b'(' => {
                self.i += 1;
                self.paren_inner()
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws(true);
                    if self.peek() == Some(b']') {
                        self.i += 1;
                        break;
                    }
                    items.push(self.expr()?);
                    self.skip_ws(true);
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b']') => {}
                        _ => return Err(self.error("expected ',' or ']'")),
                    }
                }
                Ok(Expr::List(items))
            }
            b'{' => {
                self.i += 1;
                let mut fields = Vec::new();
                loop {
                    self.skip_ws(true);
                    if self.peek() == Some(b'}') {
                        self.i += 1;
                        break;
                    }
                    let key = match self.peek() {
                        Some(b'"' | b'\'') => match self.primary()? {
                            Expr::Lit(Value::Str(s)) => s,
                            Expr::Interp(parts) => parts
                                .iter()
                                .map(|p| {
                                    if let Part::Quoted(s) = p {
                                        s.as_str()
                                    } else {
                                        ""
                                    }
                                })
                                .collect(),
                            _ => return Err(self.error("expected a field name")),
                        },
                        _ => self.ident()?,
                    };
                    self.expect(b':', "':' after the field name")?;
                    fields.push((key, self.expr()?));
                    self.skip_ws(true);
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {}
                        _ => return Err(self.error("expected ',' or '}'")),
                    }
                }
                Ok(Expr::Record(fields))
            }
            b'\'' => {
                self.i += 1;
                let start = self.i;
                while self.peek().is_some_and(|c| c != b'\'') {
                    self.i += 1;
                }
                if self.peek().is_none() {
                    return Err(self.error("missing closing '"));
                }
                let s = String::from(&self.text[start..self.i]);
                self.i += 1;
                Ok(Expr::Lit(Value::Str(s)))
            }
            b'"' => {
                self.i += 1;
                let parts = self.interpolated()?;
                Ok(match parts.as_slice() {
                    [Part::Quoted(s)] => Expr::Lit(Value::Str(s.clone())),
                    _ => Expr::Interp(parts),
                })
            }
            b'$' => match self.dollar()? {
                Part::Var(name, path) => {
                    let mut e = Expr::Var(name);
                    for p in path {
                        e = Expr::Field(Box::new(e), p);
                    }
                    Ok(e)
                }
                Part::Sub(p) => Ok(Expr::Sub(p)),
                Part::Expr(e) => Ok(e),
                _ => Err(self.error("expected a variable name after '$'")),
            },
            c if c.is_ascii_digit()
                || (c == b'-' && self.peek_at(1).is_some_and(|d| d.is_ascii_digit())) =>
            {
                let start = self.i;
                self.i += 1;
                while self
                    .peek()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'_')
                {
                    // `1..5` is a range, not a float.
                    if self.peek() == Some(b'.') && self.peek_at(1) == Some(b'.') {
                        break;
                    }
                    self.i += 1;
                }
                let text: String = self.text[start..self.i]
                    .chars()
                    .filter(|c| *c != '_')
                    .collect();
                let value = parse_number(&text).ok_or_else(|| Error {
                    at: start,
                    message: format!("'{text}' is not a number"),
                })?;
                // A range: 1..5
                if self.s[self.i..].starts_with(b"..") {
                    self.i += 2;
                    let end = self.primary()?;
                    return Ok(Expr::Range(Box::new(Expr::Lit(value)), Box::new(end)));
                }
                Ok(Expr::Lit(value))
            }
            _ if self.starts("true") => {
                self.i += 4;
                Ok(Expr::Lit(Value::Bool(true)))
            }
            _ if self.starts("false") => {
                self.i += 5;
                Ok(Expr::Lit(Value::Bool(false)))
            }
            _ if self.starts("null") => {
                self.i += 4;
                Ok(Expr::Lit(Value::Null))
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                // A bare word in an expression is a string (`where type == file`).
                let start = self.i;
                while self.peek().is_some_and(|c| {
                    c.is_ascii_alphanumeric()
                        || matches!(c, b'_' | b'-' | b'.' | b'*' | b'?' | b'\\' | b':')
                }) {
                    self.i += 1;
                }
                Ok(Expr::Lit(Value::Str(String::from(
                    &self.text[start..self.i],
                ))))
            }
            _ => Err(self.error("expected a value")),
        }
    }
}
pub fn is_keyword(word: &str) -> bool {
    KEYWORDS.contains(&word)
}
