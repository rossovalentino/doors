//! The syntax tree: function declarations, var/let/const, if/else, for,
//! while, do-while, return, break, continue, blocks and expressions (the
//! usual precedence, ternaries, assignments, calls, members, indexing,
//! array literals, function expressions).
use crate::{
    Error,
    lex::{Lexed, Token},
};
use alloc::{boxed::Box, rc::Rc, string::String, vec::Vec};

#[derive(Debug)]
pub struct Function {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
}
#[derive(Debug)]
pub enum Stmt {
    Var(Vec<(String, Option<Expr>)>),
    Function(Rc<Function>),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    For(Option<Box<Stmt>>, Option<Expr>, Option<Expr>, Box<Stmt>),
    While(Expr, Box<Stmt>),
    DoWhile(Box<Stmt>, Expr),
    Return(Option<Expr>),
    Break,
    Continue,
    Block(Vec<Stmt>),
    Expr(Expr),
    Empty,
}
#[derive(Debug)]
pub enum Expr {
    Number(f64),
    Str(String),
    Bool(bool),
    Null,
    Name(String),
    Array(Vec<Expr>),
    Function(Rc<Function>),
    Unary(&'static str, Box<Expr>),
    Binary(&'static str, Box<Expr>, Box<Expr>),
    /// `&&` and `||`, which do not evaluate their right side always.
    Logical(&'static str, Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `target op= value` (op "" for plain `=`).
    Assign(&'static str, Box<Expr>, Box<Expr>),
    /// `++x`/`x++` (true when prefix), and the step (+1 or -1).
    Update(bool, f64, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    Member(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
}
#[derive(Debug, Default)]
pub struct Program {
    pub body: Vec<Stmt>,
}
impl Program {
    /// A top-level function of this name.
    pub fn has(&self, name: &str) -> bool {
        self.body
            .iter()
            .any(|s| matches!(s, Stmt::Function(f) if f.name == name))
    }
}

struct Parser<'a> {
    tokens: &'a [Lexed],
    at: usize,
    depth: usize,
}
/// Nesting of statements and expressions the parser follows.
const MAX_NESTING: usize = 64;

pub fn program(tokens: &[Lexed]) -> Result<Program, Error> {
    let mut p = Parser {
        tokens,
        at: 0,
        depth: 0,
    };
    let mut body = Vec::new();
    while p.at < tokens.len() {
        body.push(p.statement()?);
    }
    Ok(Program { body })
}

impl Parser<'_> {
    fn line(&self) -> u32 {
        self.tokens
            .get(self.at.min(self.tokens.len().saturating_sub(1)))
            .map_or(0, |t| t.line)
    }
    fn fail<T>(&self) -> Result<T, Error> {
        Err(Error::Syntax(self.line()))
    }
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at).map(|t| &t.token)
    }
    fn is(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Token::Punct(q)) if *q == p)
    }
    fn is_word(&self, w: &str) -> bool {
        matches!(self.peek(), Some(Token::Name(n)) if n == w)
    }
    fn eat(&mut self, p: &str) -> bool {
        if self.is(p) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn eat_word(&mut self, w: &str) -> bool {
        if self.is_word(w) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, p: &str) -> Result<(), Error> {
        if self.eat(p) { Ok(()) } else { self.fail() }
    }
    fn name(&mut self) -> Result<String, Error> {
        match self.peek() {
            Some(Token::Name(n)) if !is_reserved(n) => {
                let n = n.clone();
                self.at += 1;
                Ok(n)
            }
            _ => self.fail(),
        }
    }
    fn enter(&mut self) -> Result<(), Error> {
        self.depth += 1;
        if self.depth > MAX_NESTING {
            return self.fail();
        }
        Ok(())
    }
    /// A statement's optional semicolon (automatic insertion is lenient).
    fn end(&mut self) {
        self.eat(";");
    }

    fn statement(&mut self) -> Result<Stmt, Error> {
        self.enter()?;
        let s = self.statement_inner();
        self.depth -= 1;
        s
    }
    fn statement_inner(&mut self) -> Result<Stmt, Error> {
        if self.eat(";") {
            return Ok(Stmt::Empty);
        }
        if self.eat("{") {
            let mut body = Vec::new();
            while !self.eat("}") {
                if self.peek().is_none() {
                    return self.fail();
                }
                body.push(self.statement()?);
            }
            return Ok(Stmt::Block(body));
        }
        if self.is_word("function")
            && matches!(
                self.tokens.get(self.at + 1).map(|t| &t.token),
                Some(Token::Name(_))
            )
        {
            self.at += 1;
            let f = self.function_rest(true)?;
            return Ok(Stmt::Function(f));
        }
        if self.is_word("var") || self.is_word("let") || self.is_word("const") {
            self.at += 1;
            let s = self.declarations()?;
            self.end();
            return Ok(s);
        }
        if self.eat_word("if") {
            self.expect("(")?;
            let condition = self.expression()?;
            self.expect(")")?;
            let then = Box::new(self.statement()?);
            let otherwise = if self.eat_word("else") {
                Some(Box::new(self.statement()?))
            } else {
                None
            };
            return Ok(Stmt::If(condition, then, otherwise));
        }
        if self.eat_word("for") {
            self.expect("(")?;
            let init = if self.eat(";") {
                None
            } else {
                let s = if self.is_word("var") || self.is_word("let") || self.is_word("const") {
                    self.at += 1;
                    self.declarations()?
                } else {
                    Stmt::Expr(self.expression()?)
                };
                self.expect(";")?;
                Some(Box::new(s))
            };
            let condition = if self.is(";") {
                None
            } else {
                Some(self.expression()?)
            };
            self.expect(";")?;
            let update = if self.is(")") {
                None
            } else {
                Some(self.expression()?)
            };
            self.expect(")")?;
            let body = Box::new(self.statement()?);
            return Ok(Stmt::For(init, condition, update, body));
        }
        if self.eat_word("while") {
            self.expect("(")?;
            let condition = self.expression()?;
            self.expect(")")?;
            return Ok(Stmt::While(condition, Box::new(self.statement()?)));
        }
        if self.eat_word("do") {
            let body = Box::new(self.statement()?);
            if !self.eat_word("while") {
                return self.fail();
            }
            self.expect("(")?;
            let condition = self.expression()?;
            self.expect(")")?;
            self.end();
            return Ok(Stmt::DoWhile(body, condition));
        }
        if self.eat_word("return") {
            let value = if self.is(";") || self.is("}") || self.peek().is_none() {
                None
            } else {
                Some(self.expression()?)
            };
            self.end();
            return Ok(Stmt::Return(value));
        }
        if self.eat_word("break") {
            self.end();
            return Ok(Stmt::Break);
        }
        if self.eat_word("continue") {
            self.end();
            return Ok(Stmt::Continue);
        }
        let e = self.expression()?;
        self.end();
        Ok(Stmt::Expr(e))
    }
    fn declarations(&mut self) -> Result<Stmt, Error> {
        let mut list = Vec::new();
        loop {
            let name = self.name()?;
            let value = if self.eat("=") {
                Some(self.assignment()?)
            } else {
                None
            };
            list.push((name, value));
            if !self.eat(",") {
                break;
            }
        }
        Ok(Stmt::Var(list))
    }
    /// After `function`: an optional name, parameters and the body.
    fn function_rest(&mut self, named: bool) -> Result<Rc<Function>, Error> {
        let name = if named || matches!(self.peek(), Some(Token::Name(_))) {
            self.name()?
        } else {
            String::new()
        };
        self.expect("(")?;
        let mut params = Vec::new();
        if !self.eat(")") {
            loop {
                params.push(self.name()?);
                if self.eat(")") {
                    break;
                }
                self.expect(",")?;
            }
        }
        self.expect("{")?;
        let mut body = Vec::new();
        while !self.eat("}") {
            if self.peek().is_none() {
                return self.fail();
            }
            body.push(self.statement()?);
        }
        Ok(Rc::new(Function { name, params, body }))
    }

    fn expression(&mut self) -> Result<Expr, Error> {
        let mut e = self.assignment()?;
        // The comma operator: the last value.
        while self.eat(",") {
            let next = self.assignment()?;
            e = Expr::Binary(",", Box::new(e), Box::new(next));
        }
        Ok(e)
    }
    fn assignment(&mut self) -> Result<Expr, Error> {
        self.enter()?;
        let e = self.assignment_inner();
        self.depth -= 1;
        e
    }
    fn assignment_inner(&mut self) -> Result<Expr, Error> {
        let target = self.ternary()?;
        for (p, op) in [
            ("=", ""),
            ("+=", "+"),
            ("-=", "-"),
            ("*=", "*"),
            ("/=", "/"),
            ("%=", "%"),
        ] {
            if self.eat(p) {
                if !matches!(target, Expr::Name(_) | Expr::Member(..) | Expr::Index(..)) {
                    return self.fail();
                }
                let value = self.assignment()?;
                return Ok(Expr::Assign(op, Box::new(target), Box::new(value)));
            }
        }
        Ok(target)
    }
    fn ternary(&mut self) -> Result<Expr, Error> {
        let condition = self.binary(0)?;
        if self.eat("?") {
            let yes = self.assignment()?;
            self.expect(":")?;
            let no = self.assignment()?;
            return Ok(Expr::Ternary(
                Box::new(condition),
                Box::new(yes),
                Box::new(no),
            ));
        }
        Ok(condition)
    }
    /// The operator at the cursor and its precedence (higher binds tighter).
    fn operator(&self) -> Option<(&'static str, u8)> {
        const TABLE: [(&str, u8); 21] = [
            ("||", 1),
            ("&&", 2),
            ("|", 3),
            ("^", 4),
            ("&", 5),
            ("===", 6),
            ("!==", 6),
            ("==", 6),
            ("!=", 6),
            ("<=", 7),
            (">=", 7),
            ("<", 7),
            (">", 7),
            ("<<", 8),
            (">>>", 8),
            (">>", 8),
            ("+", 9),
            ("-", 9),
            ("*", 10),
            ("/", 10),
            ("%", 10),
        ];
        if let Some(found) = TABLE.iter().find(|(op, _)| self.is(op)) {
            return Some(*found);
        }
        if self.is_word("in") {
            return Some(("in", 7));
        }
        if self.is_word("instanceof") {
            return Some(("instanceof", 7));
        }
        None
    }
    /// Binary operators by precedence climbing: one frame per level of
    /// precedence actually used, left associative.
    fn binary(&mut self, minimum: u8) -> Result<Expr, Error> {
        let mut e = self.unary()?;
        while let Some((op, precedence)) = self.operator() {
            if precedence <= minimum {
                break;
            }
            self.at += 1;
            let r = self.binary(precedence)?;
            e = if matches!(op, "&&" | "||") {
                Expr::Logical(op, Box::new(e), Box::new(r))
            } else {
                Expr::Binary(op, Box::new(e), Box::new(r))
            };
        }
        Ok(e)
    }
    fn unary(&mut self) -> Result<Expr, Error> {
        self.enter()?;
        let e = self.unary_inner();
        self.depth -= 1;
        e
    }
    fn unary_inner(&mut self) -> Result<Expr, Error> {
        for op in ["!", "-", "+", "~"] {
            if self.eat(op) {
                return Ok(Expr::Unary(op, Box::new(self.unary()?)));
            }
        }
        if self.eat_word("typeof") {
            return Ok(Expr::Unary("typeof", Box::new(self.unary()?)));
        }
        if self.eat_word("void") {
            return Ok(Expr::Unary("void", Box::new(self.unary()?)));
        }
        for (p, step) in [("++", 1.0), ("--", -1.0)] {
            if self.eat(p) {
                let target = self.unary()?;
                return Ok(Expr::Update(true, step, Box::new(target)));
            }
        }
        let mut e = self.postfix()?;
        for (p, step) in [("++", 1.0), ("--", -1.0)] {
            if self.eat(p) {
                e = Expr::Update(false, step, Box::new(e));
            }
        }
        Ok(e)
    }
    fn postfix(&mut self) -> Result<Expr, Error> {
        let mut e = self.primary()?;
        loop {
            if self.eat(".") {
                // Any name, reserved words too (`x.default`).
                let Some(Token::Name(n)) = self.peek() else {
                    return self.fail();
                };
                let n = n.clone();
                self.at += 1;
                e = Expr::Member(Box::new(e), n);
            } else if self.eat("[") {
                let index = self.expression()?;
                self.expect("]")?;
                e = Expr::Index(Box::new(e), Box::new(index));
            } else if self.eat("(") {
                let mut args = Vec::new();
                if !self.eat(")") {
                    loop {
                        args.push(self.assignment()?);
                        if self.eat(")") {
                            break;
                        }
                        self.expect(",")?;
                    }
                }
                e = Expr::Call(Box::new(e), args);
            } else {
                return Ok(e);
            }
        }
    }
    fn primary(&mut self) -> Result<Expr, Error> {
        let Some(t) = self.peek().cloned() else {
            return self.fail();
        };
        self.at += 1;
        Ok(match t {
            Token::Number(n) => Expr::Number(n),
            Token::Str(s) => Expr::Str(s),
            Token::Name(n) => match n.as_str() {
                "true" => Expr::Bool(true),
                "false" => Expr::Bool(false),
                "null" => Expr::Null,
                "function" => Expr::Function(self.function_rest(false)?),
                "new" => {
                    // `new Date()`, `new Array(...)`: a call.
                    let callee = self.primary()?;
                    let mut args = Vec::new();
                    if self.eat("(") && !self.eat(")") {
                        loop {
                            args.push(self.assignment()?);
                            if self.eat(")") {
                                break;
                            }
                            self.expect(",")?;
                        }
                    }
                    Expr::Call(Box::new(callee), args)
                }
                name if is_reserved(name) => return self.fail(),
                _ => Expr::Name(n),
            },
            Token::Punct("(") => {
                let e = self.expression()?;
                self.expect(")")?;
                e
            }
            Token::Punct("[") => {
                let mut items = Vec::new();
                if !self.eat("]") {
                    loop {
                        items.push(self.assignment()?);
                        if self.eat("]") {
                            break;
                        }
                        self.expect(",")?;
                    }
                }
                Expr::Array(items)
            }
            _ => return self.fail(),
        })
    }
}
fn is_reserved(n: &str) -> bool {
    matches!(
        n,
        "var"
            | "let"
            | "const"
            | "if"
            | "else"
            | "for"
            | "while"
            | "do"
            | "return"
            | "break"
            | "continue"
            | "typeof"
            | "void"
            | "in"
            | "instanceof"
    )
}
