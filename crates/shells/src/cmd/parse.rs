//! cmd.exe phase 2: one logical line (percent-expanded) into a command
//! tree. `&` < `&&`/`||` < `|` in precedence; parenthesized blocks (a
//! newline inside one separates commands); IF with ELSE after a block; FOR
//! with its set and body; carets escape outside quotes; redirections are
//! cut from a command's text with the spacing cmd.exe leaves (`echo a >f`
//! writes "a "). A command keeps its argument text raw: each internal
//! command parses its own, as cmd.exe's do.
use alloc::{boxed::Box, format, string::String, vec::Vec};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Redirect {
    /// `n>file` / `n>>file`.
    Out {
        fd: u8,
        target: String,
        append: bool,
    },
    /// `<file`.
    In(String),
    /// `n>&m`.
    Dup { fd: u8, to: u8 },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Simple {
    /// `@` before the command.
    pub quiet: bool,
    /// The command word as typed (quotes kept).
    pub head: String,
    /// Everything after it with redirections removed, leading delimiter
    /// included (`echo.` has tail ".").
    pub tail: String,
    pub redirects: Vec<Redirect>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Equ,
    Neq,
    Lss,
    Leq,
    Gtr,
    Geq,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cond {
    Compare { lhs: String, op: CmpOp, rhs: String },
    ErrorLevel(String),
    Exist(String),
    Defined(String),
    CmdExtVersion(String),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForKind {
    Files,
    Dirs,
    Recursive,
    Range,
    Lines,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct For {
    pub kind: ForKind,
    /// `/F` options text (unquoted) or `/R` root.
    pub options: String,
    pub root: Option<String>,
    pub var: char,
    /// The set between the parentheses, raw.
    pub set: String,
    pub body: Node,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    Simple(Simple),
    Block {
        body: Box<Node>,
        redirects: Vec<Redirect>,
    },
    If {
        not: bool,
        insensitive: bool,
        cond: Cond,
        then: Box<Node>,
        otherwise: Option<Box<Node>>,
    },
    For(Box<For>),
    Seq(Box<Node>, Box<Node>),
    And(Box<Node>, Box<Node>),
    Or(Box<Node>, Box<Node>),
    Pipe(Box<Node>, Box<Node>),
    Empty,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// An open parenthesis: the line continues (`More? `).
    More,
    /// cmd.exe's message.
    Syntax(String),
}
fn syntax() -> ParseError {
    ParseError::Syntax("The syntax of the command is incorrect.".into())
}
fn unexpected(what: &str) -> ParseError {
    ParseError::Syntax(format!("{what} was unexpected at this time."))
}

pub fn parse(line: &str) -> Result<Node, ParseError> {
    let mut p = Parser {
        c: line.chars().collect(),
        i: 0,
        depth: 0,
    };
    let node = p.sequence()?;
    p.skip_space();
    if p.i < p.c.len() {
        let rest: String = p.c[p.i..]
            .iter()
            .take_while(|c| !c.is_whitespace())
            .collect();
        return Err(unexpected(&rest));
    }
    Ok(node)
}

struct Parser {
    c: Vec<char>,
    i: usize,
    depth: usize,
}
const SPACE: [char; 5] = [' ', '\t', ',', ';', '='];
impl Parser {
    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }
    fn at(&self, offset: usize) -> Option<char> {
        self.c.get(self.i + offset).copied()
    }
    fn skip_space(&mut self) {
        while self
            .peek()
            .is_some_and(|c| c == ' ' || c == '\t' || (c == '\n' && self.depth == 0))
        {
            self.i += 1;
        }
    }
    /// Spaces and newlines inside a block (newline separates commands).
    fn skip_blank(&mut self) {
        while self
            .peek()
            .is_some_and(|c| c == ' ' || c == '\t' || c == '\n')
        {
            self.i += 1;
        }
    }
    fn end_of_command(&self) -> bool {
        match self.peek() {
            None => true,
            Some('&' | '|') => true,
            Some(')') => self.depth > 0,
            Some('\n') => self.depth > 0,
            _ => false,
        }
    }
    fn sequence(&mut self) -> Result<Node, ParseError> {
        if self.depth > 0 {
            self.skip_blank();
        }
        let mut left = self.conditional()?;
        loop {
            let separator = match self.peek() {
                Some('&') if self.at(1) != Some('&') => true,
                Some('\n') if self.depth > 0 => true,
                _ => false,
            };
            if !separator {
                break;
            }
            self.i += 1;
            if self.depth > 0 {
                self.skip_blank();
            } else {
                self.skip_space();
            }
            if self.peek().is_none() || (self.depth > 0 && self.peek() == Some(')')) {
                if self.peek().is_none() && self.depth > 0 {
                    return Err(ParseError::More);
                }
                break;
            }
            let right = self.conditional()?;
            left = Node::Seq(Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn conditional(&mut self) -> Result<Node, ParseError> {
        let mut left = self.pipeline()?;
        loop {
            let (a, b) = (self.peek(), self.at(1));
            let and = match (a, b) {
                (Some('&'), Some('&')) => true,
                (Some('|'), Some('|')) => false,
                _ => break,
            };
            self.i += 2;
            self.skip_space();
            if self.peek().is_none() {
                return Err(if self.depth > 0 {
                    ParseError::More
                } else {
                    unexpected(if and { "&&" } else { "||" })
                });
            }
            let right = self.pipeline()?;
            left = if and {
                Node::And(Box::new(left), Box::new(right))
            } else {
                Node::Or(Box::new(left), Box::new(right))
            };
        }
        Ok(left)
    }
    fn pipeline(&mut self) -> Result<Node, ParseError> {
        let mut left = self.command()?;
        while self.peek() == Some('|') && self.at(1) != Some('|') {
            self.i += 1;
            self.skip_space();
            if self.peek().is_none() {
                return Err(unexpected("|"));
            }
            let right = self.command()?;
            left = Node::Pipe(Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn command(&mut self) -> Result<Node, ParseError> {
        self.skip_space();
        let mut quiet = false;
        while self.peek() == Some('@') {
            quiet = true;
            self.i += 1;
            self.skip_space();
        }
        match self.peek() {
            None => return Ok(Node::Empty),
            Some('(') => return self.block(),
            Some(')') if self.depth == 0 => return Err(unexpected(")")),
            Some(':') => {
                // A label (or `::` comment): the rest of the physical line.
                while self.peek().is_some_and(|c| c != '\n') {
                    self.i += 1;
                }
                return Ok(Node::Empty);
            }
            _ => (),
        }
        let word = self.keyword();
        match word.as_str() {
            "if" => return self.if_command(),
            "for" => return self.for_command(),
            "rem" => {
                let start = self.i;
                while self.peek().is_some_and(|c| c != '\n') {
                    self.i += 1;
                }
                let text: String = self.c[start..self.i].iter().collect();
                let tail = text[3.min(text.len())..].into();
                return Ok(Node::Simple(Simple {
                    quiet,
                    head: text[..3.min(text.len())].into(),
                    tail,
                    redirects: Vec::new(),
                }));
            }
            _ => (),
        }
        self.simple(quiet)
    }
    /// The next word lower-cased when it is IF/FOR/REM followed by a
    /// delimiter (cmd recognizes `if(`, `rem.` too).
    fn keyword(&self) -> String {
        let mut j = self.i;
        let mut w = String::new();
        while let Some(&c) = self.c.get(j) {
            if !c.is_ascii_alphabetic() {
                break;
            }
            w.push(c.to_ascii_lowercase());
            j += 1;
        }
        let next = self.c.get(j).copied();
        let delimited = next.is_none_or(|c| {
            SPACE.contains(&c)
                || c == '\n'
                || (w == "if" && c == '(')
                || (w == "rem" && matches!(c, '.' | ':' | '/' | '\\' | '('))
        });
        if delimited && matches!(w.as_str(), "if" | "for" | "rem") {
            w
        } else {
            String::new()
        }
    }
    fn block(&mut self) -> Result<Node, ParseError> {
        self.i += 1;
        self.depth += 1;
        let body = self.sequence()?;
        self.skip_blank();
        if self.peek() != Some(')') {
            return Err(if self.peek().is_none() {
                ParseError::More
            } else {
                syntax()
            });
        }
        self.i += 1;
        self.depth -= 1;
        let redirects = self.trailing_redirects()?;
        Ok(Node::Block {
            body: Box::new(body),
            redirects,
        })
    }
    fn trailing_redirects(&mut self) -> Result<Vec<Redirect>, ParseError> {
        let mut out = Vec::new();
        loop {
            let save = self.i;
            self.skip_space();
            let fd = match (self.peek(), self.at(1)) {
                (Some(d @ ('1' | '2')), Some('>')) => {
                    self.i += 1;
                    Some(d as u8 - b'0')
                }
                _ => None,
            };
            match self.peek() {
                Some('>' | '<') => out.push(self.redirect(fd)?),
                _ => {
                    self.i = save;
                    return Ok(out);
                }
            }
        }
    }
    /// At `>` or `<` (after an optional handle digit).
    fn redirect(&mut self, fd: Option<u8>) -> Result<Redirect, ParseError> {
        let input = self.peek() == Some('<');
        self.i += 1;
        let append = !input && self.peek() == Some('>');
        if append {
            self.i += 1;
        }
        if self.peek() == Some('&') {
            self.i += 1;
            let to = self
                .peek()
                .and_then(|c| c.to_digit(10))
                .ok_or_else(syntax)? as u8;
            self.i += 1;
            return Ok(Redirect::Dup {
                fd: fd.unwrap_or(if input { 0 } else { 1 }),
                to,
            });
        }
        while self.peek().is_some_and(|c| c == ' ' || c == '\t') {
            self.i += 1;
        }
        let mut target = String::new();
        let mut quoted = false;
        while let Some(c) = self.peek() {
            if !quoted
                && (c == ' '
                    || c == '\t'
                    || c == '&'
                    || c == '|'
                    || c == '<'
                    || c == '>'
                    || c == '\n'
                    || (c == ')' && self.depth > 0))
            {
                break;
            }
            if c == '"' {
                quoted = !quoted;
            } else if c == '^' && !quoted {
                self.i += 1;
                if let Some(n) = self.peek() {
                    target.push(n);
                }
                self.i += 1;
                continue;
            } else {
                target.push(c);
            }
            self.i += 1;
        }
        if target.is_empty() {
            return Err(syntax());
        }
        Ok(if input {
            Redirect::In(target)
        } else {
            Redirect::Out {
                fd: fd.unwrap_or(1),
                target,
                append,
            }
        })
    }
    fn simple(&mut self, quiet: bool) -> Result<Node, ParseError> {
        let mut redirects = Vec::new();
        let mut text = String::new();
        let mut quoted = false;
        while let Some(c) = self.peek() {
            if quoted {
                if c == '\n' {
                    quoted = false;
                    continue;
                }
                text.push(c);
                if c == '"' {
                    quoted = false;
                }
                self.i += 1;
                continue;
            }
            if self.end_of_command() {
                break;
            }
            match c {
                '"' => {
                    quoted = true;
                    text.push(c);
                    self.i += 1;
                }
                '^' => {
                    self.i += 1;
                    if let Some(n) = self.peek() {
                        text.push(n);
                        self.i += 1;
                    }
                }
                '>' | '<' => {
                    // A lone handle digit before the operator belongs to it.
                    let mut fd = None;
                    if c == '>' && (text.ends_with('1') || text.ends_with('2')) {
                        let before = text.chars().rev().nth(1);
                        if before.is_none_or(|b| b == ' ' || b == '\t') {
                            fd = Some(text.pop().map(|d| d as u8 - b'0').unwrap());
                        }
                    }
                    redirects.push(self.redirect(fd)?);
                }
                _ => {
                    text.push(c);
                    self.i += 1;
                }
            }
        }
        // Split the command word: up to a delimiter, keeping quotes.
        let chars: Vec<char> = text.chars().collect();
        let mut k = 0;
        while k < chars.len() && SPACE.contains(&chars[k]) {
            k += 1;
        }
        let start = k;
        let mut q = false;
        while k < chars.len() {
            let ch = chars[k];
            if ch == '"' {
                q = !q;
            } else if !q && (ch == ' ' || ch == '\t') {
                break;
            }
            k += 1;
        }
        let mut head: String = chars[start..k].iter().collect();
        let mut tail: String = chars[k..].iter().collect();
        // Internal commands end at punctuation too: `echo.`, `cd..`, `dir/w`.
        if !head.starts_with('"') {
            let lower = head.to_ascii_lowercase();
            if let Some(name) = super::builtin::NAMES
                .iter()
                .filter(|n| lower.starts_with(**n) && lower.len() > n.len())
                .max_by_key(|n| n.len())
                && lower[name.len()..]
                    .starts_with(['.', '/', '\\', '(', '+', '[', ']', ':', ',', ';', '=', '"'])
            {
                tail = format!("{}{tail}", &head[name.len()..]);
                head.truncate(name.len());
            }
        }
        if head.is_empty() && redirects.is_empty() {
            return Ok(Node::Empty);
        }
        Ok(Node::Simple(Simple {
            quiet,
            head,
            tail,
            redirects,
        }))
    }
    /// One IF/FOR operand: a quoted string or a run up to a delimiter
    /// (`==` ends it too).
    fn operand(&mut self, stop_eq: bool) -> String {
        let mut out = String::new();
        let mut quoted = false;
        while let Some(c) = self.peek() {
            if quoted {
                out.push(c);
                self.i += 1;
                if c == '"' {
                    quoted = false;
                }
                continue;
            }
            if stop_eq && c == '=' && self.at(1) == Some('=') {
                break;
            }
            if c == ' '
                || c == '\t'
                || c == ','
                || c == ';'
                || c == '\n'
                || c == '&'
                || c == '|'
                || (c == ')' && self.depth > 0)
            {
                break;
            }
            if c == '=' && !stop_eq {
                break;
            }
            if c == '^' {
                self.i += 1;
                if let Some(n) = self.peek() {
                    out.push(n);
                    self.i += 1;
                }
                continue;
            }
            if c == '"' {
                quoted = true;
            }
            out.push(c);
            self.i += 1;
        }
        out
    }
    fn skip_operand_space(&mut self) {
        while self.peek().is_some_and(|c| SPACE.contains(&c) && c != '=') {
            self.i += 1;
        }
    }
    fn if_command(&mut self) -> Result<Node, ParseError> {
        self.i += 2;
        let mut not = false;
        let mut insensitive = false;
        loop {
            self.skip_operand_space();
            let save = self.i;
            let w = self.operand(true).to_ascii_lowercase();
            match w.as_str() {
                "/i" if !insensitive && !not => insensitive = true,
                "not" if !not => not = true,
                _ => {
                    self.i = save;
                    break;
                }
            }
        }
        self.skip_operand_space();
        let save = self.i;
        let first = self.operand(true);
        if first.is_empty() && self.peek() != Some('=') {
            return Err(syntax());
        }
        let keyword = first.to_ascii_lowercase();
        let mut space_after = self.peek().is_some_and(|c| c == ' ' || c == '\t');
        let cond = match keyword.as_str() {
            "errorlevel" | "exist" | "defined" | "cmdextversion" if space_after => {
                self.skip_operand_space();
                let arg = self.operand(false);
                if arg.is_empty() {
                    return Err(syntax());
                }
                match keyword.as_str() {
                    "errorlevel" => Cond::ErrorLevel(arg),
                    "exist" => Cond::Exist(arg),
                    "defined" => Cond::Defined(arg),
                    _ => Cond::CmdExtVersion(arg),
                }
            }
            _ => {
                self.i = save;
                let lhs = self.operand(true);
                self.skip_operand_space();
                let op = if self.peek() == Some('=') && self.at(1) == Some('=') {
                    self.i += 2;
                    while self.peek() == Some('=') {
                        self.i += 1;
                    }
                    CmpOp::Eq
                } else {
                    let w = self.operand(true);
                    match w.to_ascii_uppercase().as_str() {
                        "EQU" => CmpOp::Equ,
                        "NEQ" => CmpOp::Neq,
                        "LSS" => CmpOp::Lss,
                        "LEQ" => CmpOp::Leq,
                        "GTR" => CmpOp::Gtr,
                        "GEQ" => CmpOp::Geq,
                        _ => {
                            return Err(unexpected(if w.is_empty() {
                                "The syntax of the command is incorrect."
                            } else {
                                &w
                            }));
                        }
                    }
                };
                self.skip_operand_space();
                let rhs = self.operand(true);
                if rhs.is_empty() {
                    return Err(syntax());
                }
                space_after = true;
                Cond::Compare { lhs, op, rhs }
            }
        };
        let _ = space_after;
        self.skip_space();
        if self.peek().is_none() || self.end_of_command() {
            return Err(if self.depth > 0 && self.peek().is_none() {
                ParseError::More
            } else {
                syntax()
            });
        }
        let (then, otherwise) = self.body_with_else()?;
        Ok(Node::If {
            not,
            insensitive,
            cond,
            then: Box::new(then),
            otherwise,
        })
    }
    /// An IF/FOR body: a block (then an optional ELSE) or the rest of the
    /// line's sequence.
    fn body_with_else(&mut self) -> Result<(Node, Option<Box<Node>>), ParseError> {
        self.skip_space();
        let mut quiet_block = false;
        let save = self.i;
        while self.peek() == Some('@') {
            self.i += 1;
            quiet_block = true;
        }
        if self.peek() == Some('(') {
            let then = self.block()?;
            let save_else = self.i;
            self.skip_space();
            if self.word_is("else") {
                self.i += 4;
                self.skip_space();
                if self.peek().is_none() {
                    return Err(if self.depth > 0 {
                        ParseError::More
                    } else {
                        syntax()
                    });
                }
                let (e, _) = self.body_with_else()?;
                return Ok((then, Some(Box::new(e))));
            }
            self.i = save_else;
            return Ok((then, None));
        }
        let _ = quiet_block;
        self.i = save;
        // A simple body takes the rest of the conditional chain, as cmd's.
        let body = self.conditional_rest()?;
        Ok((body, None))
    }
    /// The body of an IF/FOR without a block: everything to the end of the
    /// line (or block), `&` included.
    fn conditional_rest(&mut self) -> Result<Node, ParseError> {
        let mut left = self.conditional()?;
        while self.peek() == Some('&') && self.at(1) != Some('&') {
            self.i += 1;
            self.skip_space();
            if self.peek().is_none() || (self.depth > 0 && self.peek() == Some(')')) {
                break;
            }
            let right = self.conditional()?;
            left = Node::Seq(Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn word_is(&self, w: &str) -> bool {
        let n = w.len();
        let got: String = self.c[self.i..(self.i + n).min(self.c.len())]
            .iter()
            .collect();
        got.eq_ignore_ascii_case(w)
            && self
                .c
                .get(self.i + n)
                .is_none_or(|c| SPACE.contains(c) || *c == '(' || *c == '\n')
    }
    fn for_command(&mut self) -> Result<Node, ParseError> {
        self.i += 3;
        let mut kind = ForKind::Files;
        let mut options = String::new();
        let mut root = None;
        loop {
            self.skip_space();
            if self.peek() != Some('/') {
                break;
            }
            let flag = self.operand(false).to_ascii_lowercase();
            match flag.as_str() {
                "/d" => kind = ForKind::Dirs,
                "/l" => kind = ForKind::Range,
                "/r" => {
                    kind = ForKind::Recursive;
                    self.skip_space();
                    if self.peek() != Some('%') {
                        root = Some(self.operand(false).trim_matches('"').into());
                    }
                }
                "/f" => {
                    kind = ForKind::Lines;
                    self.skip_space();
                    if self.peek() == Some('"') {
                        let o = self.operand(false);
                        options = o.trim_matches('"').into();
                    } else if self.peek() != Some('%') {
                        options = self.operand(false);
                    }
                }
                _ => return Err(unexpected(&flag)),
            }
        }
        self.skip_space();
        if self.peek() != Some('%') {
            let w = self.operand(false);
            return Err(unexpected(if w.is_empty() {
                "The syntax of the command is incorrect."
            } else {
                &w
            }));
        }
        self.i += 1;
        let var = self.peek().ok_or_else(syntax)?;
        self.i += 1;
        self.skip_space();
        if !self.word_is("in") {
            let w = self.operand(false);
            return Err(unexpected(&w));
        }
        self.i += 2;
        self.skip_space();
        if self.peek() != Some('(') {
            let w = self.operand(false);
            return Err(unexpected(&w));
        }
        self.i += 1;
        let mut set = String::new();
        let mut nest = 0;
        let mut quoted = false;
        let mut single = false;
        loop {
            let Some(c) = self.peek() else {
                return Err(ParseError::More);
            };
            self.i += 1;
            if c == '"' && !single {
                quoted = !quoted;
            } else if c == '\'' && !quoted {
                single = !single;
            } else if !quoted && !single {
                if c == '^' {
                    if let Some(n) = self.peek() {
                        set.push(n);
                        self.i += 1;
                    }
                    continue;
                }
                if c == '(' {
                    nest += 1;
                } else if c == ')' {
                    if nest == 0 {
                        break;
                    }
                    nest -= 1;
                }
            }
            set.push(if c == '\n' { ' ' } else { c });
        }
        self.skip_space();
        if !self.word_is("do") {
            let w = self.operand(false);
            return Err(unexpected(if w.is_empty() {
                "The syntax of the command is incorrect."
            } else {
                &w
            }));
        }
        self.i += 2;
        self.skip_space();
        if self.peek().is_none() {
            return Err(if self.depth > 0 {
                ParseError::More
            } else {
                syntax()
            });
        }
        let (body, _) = self.body_with_else()?;
        Ok(Node::For(Box::new(For {
            kind,
            options,
            root,
            var,
            set,
            body,
        })))
    }
}
/// The command as cmd.exe echoes it (`echo on` in a batch file).
pub fn render(node: &Node) -> String {
    match node {
        Node::Simple(s) => {
            // cmd.exe echoes a command with arguments followed by a space.
            let mut out = if s.tail.is_empty() {
                s.head.clone()
            } else {
                format!("{}{} ", s.head, s.tail.trim_end())
            };
            for r in &s.redirects {
                out.push(' ');
                out.push_str(&render_redirect(r));
            }
            out
        }
        Node::Block { body, redirects } => {
            let mut out = format!("({} )", render(body));
            for r in redirects {
                out.push(' ');
                out.push_str(&render_redirect(r));
            }
            out
        }
        Node::If {
            not,
            insensitive,
            cond,
            then,
            otherwise,
        } => {
            let mut out = String::from("if ");
            if *insensitive {
                out.push_str("/I ");
            }
            if *not {
                out.push_str("not ");
            }
            out.push_str(&match cond {
                Cond::Compare { lhs, op, rhs } => format!(
                    "{lhs} {} {rhs}",
                    match op {
                        CmpOp::Eq => "==",
                        CmpOp::Equ => "EQU",
                        CmpOp::Neq => "NEQ",
                        CmpOp::Lss => "LSS",
                        CmpOp::Leq => "LEQ",
                        CmpOp::Gtr => "GTR",
                        CmpOp::Geq => "GEQ",
                    }
                ),
                Cond::ErrorLevel(n) => format!("errorlevel {n}"),
                Cond::Exist(p) => format!("exist {p}"),
                Cond::Defined(v) => format!("defined {v}"),
                Cond::CmdExtVersion(n) => format!("cmdextversion {n}"),
            });
            out.push(' ');
            out.push_str(&render(then));
            if let Some(e) = otherwise {
                out.push_str(" else ");
                out.push_str(&render(e));
            }
            out
        }
        Node::For(f) => format!("for %{} in ({}) do {}", f.var, f.set, render(&f.body)),
        Node::Seq(a, b) => format!("{} & {}", render(a), render(b)),
        Node::And(a, b) => format!("{} && {}", render(a), render(b)),
        Node::Or(a, b) => format!("{} || {}", render(a), render(b)),
        Node::Pipe(a, b) => format!("{} | {}", render(a), render(b)),
        Node::Empty => String::new(),
    }
}
fn render_redirect(r: &Redirect) -> String {
    match r {
        Redirect::Out { fd, target, append } => {
            format!("{fd}>{}{target}", if *append { ">" } else { "" })
        }
        Redirect::In(t) => format!("0<{t}"),
        Redirect::Dup { fd, to } => format!("{fd}>&{to}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn simple(head: &str, tail: &str) -> Node {
        Node::Simple(Simple {
            quiet: false,
            head: head.into(),
            tail: tail.into(),
            redirects: Vec::new(),
        })
    }
    #[test]
    fn simple_commands_keep_their_text() {
        assert_eq!(
            parse("echo hello  world").unwrap(),
            simple("echo", " hello  world")
        );
        assert_eq!(parse("echo.").unwrap(), simple("echo", "."));
        assert_eq!(parse("cd..").unwrap(), simple("cd", ".."));
        assert_eq!(parse("dir/w").unwrap(), simple("dir", "/w"));
        assert_eq!(parse("echo ^& ^| x").unwrap(), simple("echo", " & | x"));
        assert_eq!(
            parse("echo \"a & b\"").unwrap(),
            simple("echo", " \"a & b\"")
        );
        assert_eq!(
            parse("\"C:\\Program Files\\x.exe\" /q").unwrap(),
            simple("\"C:\\Program Files\\x.exe\"", " /q")
        );
    }
    #[test]
    fn redirections_leave_cmd_spacing() {
        let Node::Simple(s) = parse("echo a >f.txt").unwrap() else {
            panic!()
        };
        assert_eq!(s.tail, " a ");
        assert_eq!(
            s.redirects,
            [Redirect::Out {
                fd: 1,
                target: "f.txt".into(),
                append: false
            }]
        );
        let Node::Simple(s) = parse("echo a>f.txt 2>&1").unwrap() else {
            panic!()
        };
        assert_eq!(s.tail, " a ");
        assert_eq!(s.redirects.len(), 2);
        let Node::Simple(s) = parse(">>log.txt echo x").unwrap() else {
            panic!()
        };
        assert_eq!((s.head.as_str(), s.tail.as_str()), ("echo", " x"));
        let Node::Simple(s) = parse("echo 2>nul").unwrap() else {
            panic!()
        };
        assert_eq!(s.tail, " ");
        assert_eq!(
            s.redirects,
            [Redirect::Out {
                fd: 2,
                target: "nul".into(),
                append: false
            }]
        );
        let Node::Simple(s) = parse("echo a2>nul").unwrap() else {
            panic!()
        };
        assert_eq!(s.tail, " a2");
    }
    #[test]
    fn operators_and_precedence() {
        let n = parse("a & b && c || d | e").unwrap();
        let Node::Seq(a, rest) = n else {
            panic!("{n:?}")
        };
        assert_eq!(*a, simple("a", " "));
        assert!(matches!(*rest, Node::Or(..)));
    }
    #[test]
    fn blocks_if_else_and_for() {
        assert_eq!(parse("(echo a").unwrap_err(), ParseError::More);
        let n = parse("(echo a\necho b) > out.txt").unwrap();
        let Node::Block { body, redirects } = n else {
            panic!()
        };
        assert!(matches!(*body, Node::Seq(..)));
        assert_eq!(redirects.len(), 1);
        let n = parse("if /i \"%x%\"==\"Y\" (echo yes) else (echo no)").unwrap();
        let Node::If {
            insensitive,
            cond,
            otherwise,
            ..
        } = n
        else {
            panic!()
        };
        assert!(insensitive);
        assert_eq!(
            cond,
            Cond::Compare {
                lhs: "\"%x%\"".into(),
                op: CmpOp::Eq,
                rhs: "\"Y\"".into()
            }
        );
        assert!(otherwise.is_some());
        let n = parse("if not exist \"a b.txt\" echo missing & echo two").unwrap();
        let Node::If {
            not, cond, then, ..
        } = n
        else {
            panic!()
        };
        assert!(not);
        assert_eq!(cond, Cond::Exist("\"a b.txt\"".into()));
        assert!(matches!(*then, Node::Seq(..)));
        let n = parse("if 10 gtr 9 echo greater").unwrap();
        assert!(matches!(
            n,
            Node::If {
                cond: Cond::Compare { op: CmpOp::Gtr, .. },
                ..
            }
        ));
        let n = parse("if errorlevel 1 goto fail").unwrap();
        assert!(matches!(
            n,
            Node::If {
                cond: Cond::ErrorLevel(_),
                ..
            }
        ));
        let n = parse("for /f \"tokens=1,2 delims=,\" %a in ('type x.csv') do @echo %b").unwrap();
        let Node::For(f) = n else { panic!() };
        assert_eq!(
            (f.kind, f.options.as_str(), f.var, f.set.as_str()),
            (ForKind::Lines, "tokens=1,2 delims=,", 'a', "'type x.csv'")
        );
        let n = parse("for /r C:\\src %f in (*.rs) do echo %f").unwrap();
        let Node::For(f) = n else { panic!() };
        assert_eq!(f.root.as_deref(), Some("C:\\src"));
        let n = parse("for %i in (a b) do (\necho %i\n)").unwrap();
        assert!(matches!(n, Node::For(_)));
        assert!(parse("for %i in (a b) do (").is_err());
    }
    #[test]
    fn labels_comments_and_errors() {
        assert_eq!(parse(":loop").unwrap(), Node::Empty);
        assert_eq!(parse("::comment & echo").unwrap(), Node::Empty);
        assert!(
            matches!(parse("rem x & echo y").unwrap(), Node::Simple(s) if s.tail == " x & echo y")
        );
        assert_eq!(
            parse(") x").unwrap_err(),
            ParseError::Syntax(") was unexpected at this time.".into())
        );
        assert_eq!(
            parse("@echo off").unwrap(),
            Node::Simple(Simple {
                quiet: true,
                head: "echo".into(),
                tail: " off".into(),
                redirects: Vec::new()
            })
        );
    }
}
