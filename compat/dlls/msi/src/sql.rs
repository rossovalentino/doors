//! Windows Installer's SQL subset for MsiDatabaseOpenView: `SELECT
//! [DISTINCT] columns|* FROM tables [WHERE condition] [ORDER BY columns]`,
//! with `=`, `<>`, `<`, `>`, `<=`, `>=`, `IS [NOT] NULL`, `AND`, `OR`,
//! parentheses, string and integer literals, `?` parameters and joins
//! (`table`.`column`). Identifiers may be quoted with backquotes.
use alloc::{boxed::Box, string::String, vec::Vec};
use door_msi::{Database, Table, Value};

/// A record field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Field {
    Null,
    Int(i32),
    Str(String),
    /// An OBJECT column: the stream's name in the database.
    Stream(String),
}
impl Field {
    fn from(v: &Value) -> Self {
        match v {
            Value::Null => Field::Null,
            Value::Int(i) => Field::Int(*i),
            Value::Str(s) => Field::Str(s.clone()),
            Value::Stream(s) => Field::Stream(s.clone()),
        }
    }
    pub fn text(&self) -> String {
        match self {
            Field::Null | Field::Stream(_) => String::new(),
            Field::Int(i) => alloc::format!("{i}"),
            Field::Str(s) => s.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Word(String),
    Quoted(String),
    Text(String),
    Number(i32),
    Op(&'static str),
    Comma,
    Dot,
    Open,
    Close,
    Star,
    Param,
}

/// ERROR_BAD_QUERY_SYNTAX.
pub const BAD_QUERY: u32 = 1615;
/// ERROR_INVALID_TABLE / ERROR_INVALID_FIELD.
pub const INVALID_TABLE: u32 = 1628;

fn tokens(s: &str) -> Result<Vec<Token>, u32> {
    let c: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        match ch {
            ' ' | '\t' | '\r' | '\n' => i += 1,
            ',' => {
                out.push(Token::Comma);
                i += 1
            }
            '.' => {
                out.push(Token::Dot);
                i += 1
            }
            '(' => {
                out.push(Token::Open);
                i += 1
            }
            ')' => {
                out.push(Token::Close);
                i += 1
            }
            '*' => {
                out.push(Token::Star);
                i += 1
            }
            '?' => {
                out.push(Token::Param);
                i += 1
            }
            '`' => {
                let end = c[i + 1..].iter().position(|x| *x == '`').ok_or(BAD_QUERY)? + i + 1;
                out.push(Token::Quoted(c[i + 1..end].iter().collect()));
                i = end + 1;
            }
            '\'' => {
                let end = c[i + 1..]
                    .iter()
                    .position(|x| *x == '\'')
                    .ok_or(BAD_QUERY)?
                    + i
                    + 1;
                out.push(Token::Text(c[i + 1..end].iter().collect()));
                i = end + 1;
            }
            '<' | '>' | '=' => {
                let two: String = c[i..(i + 2).min(c.len())].iter().collect();
                if two == "<>" || two == "<=" || two == ">=" {
                    out.push(Token::Op(match two.as_str() {
                        "<>" => "<>",
                        "<=" => "<=",
                        _ => ">=",
                    }));
                    i += 2;
                } else {
                    out.push(Token::Op(match ch {
                        '<' => "<",
                        '>' => ">",
                        _ => "=",
                    }));
                    i += 1;
                }
            }
            '-' | '0'..='9' => {
                let start = i;
                i += 1;
                while i < c.len() && c[i].is_ascii_digit() {
                    i += 1;
                }
                let t: String = c[start..i].iter().collect();
                out.push(Token::Number(t.parse().map_err(|_| BAD_QUERY)?));
            }
            ch if ch.is_alphanumeric() || ch == '_' => {
                let start = i;
                while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                    i += 1;
                }
                out.push(Token::Word(c[start..i].iter().collect()));
            }
            _ => return Err(BAD_QUERY),
        }
    }
    Ok(out)
}

/// A column reference (optionally qualified).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Column {
    table: Option<String>,
    name: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Operand {
    Column(Column),
    Text(String),
    Number(i32),
    Param(usize),
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Expr {
    Compare(Operand, &'static str, Operand),
    IsNull(Column, bool),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
}
/// A parsed SELECT.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    distinct: bool,
    /// None: `*`.
    columns: Option<Vec<Column>>,
    tables: Vec<String>,
    filter: Option<Expr>,
    order: Vec<Column>,
    /// The number of `?` parameters.
    pub parameters: usize,
}

struct Parser {
    t: Vec<Token>,
    i: usize,
    params: usize,
}
impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.t.get(self.i)
    }
    fn keyword(&mut self, k: &str) -> bool {
        if let Some(Token::Word(w)) = self.peek()
            && w.eq_ignore_ascii_case(k)
        {
            self.i += 1;
            return true;
        }
        false
    }
    fn name(&mut self) -> Result<String, u32> {
        match self.t.get(self.i).cloned() {
            Some(Token::Word(w)) | Some(Token::Quoted(w)) => {
                self.i += 1;
                Ok(w)
            }
            _ => Err(BAD_QUERY),
        }
    }
    fn column(&mut self) -> Result<Column, u32> {
        let first = self.name()?;
        if self.peek() == Some(&Token::Dot) {
            self.i += 1;
            let second = self.name()?;
            return Ok(Column {
                table: Some(first),
                name: second,
            });
        }
        Ok(Column {
            table: None,
            name: first,
        })
    }
    fn operand(&mut self) -> Result<Operand, u32> {
        match self.t.get(self.i).cloned() {
            Some(Token::Text(s)) => {
                self.i += 1;
                Ok(Operand::Text(s))
            }
            Some(Token::Number(n)) => {
                self.i += 1;
                Ok(Operand::Number(n))
            }
            Some(Token::Param) => {
                self.i += 1;
                self.params += 1;
                Ok(Operand::Param(self.params - 1))
            }
            _ => Ok(Operand::Column(self.column()?)),
        }
    }
    fn term(&mut self) -> Result<Expr, u32> {
        if self.peek() == Some(&Token::Open) {
            self.i += 1;
            let e = self.expr()?;
            if self.peek() != Some(&Token::Close) {
                return Err(BAD_QUERY);
            }
            self.i += 1;
            return Ok(e);
        }
        let left = self.operand()?;
        if self.keyword("IS") {
            let not = self.keyword("NOT");
            if !self.keyword("NULL") {
                return Err(BAD_QUERY);
            }
            let Operand::Column(c) = left else {
                return Err(BAD_QUERY);
            };
            return Ok(Expr::IsNull(c, !not));
        }
        let op = match self.peek() {
            Some(Token::Op(o)) => *o,
            _ => return Err(BAD_QUERY),
        };
        self.i += 1;
        let right = self.operand()?;
        Ok(Expr::Compare(left, op, right))
    }
    fn conjunction(&mut self) -> Result<Expr, u32> {
        let mut e = self.term()?;
        while self.keyword("AND") {
            let r = self.term()?;
            e = Expr::And(Box::new(e), Box::new(r));
        }
        Ok(e)
    }
    fn expr(&mut self) -> Result<Expr, u32> {
        let mut e = self.conjunction()?;
        while self.keyword("OR") {
            let r = self.conjunction()?;
            e = Expr::Or(Box::new(e), Box::new(r));
        }
        Ok(e)
    }
}

/// Parse a query (only SELECT: a package opened read-only).
pub fn parse(sql: &str) -> Result<Query, u32> {
    let mut p = Parser {
        t: tokens(sql)?,
        i: 0,
        params: 0,
    };
    if !p.keyword("SELECT") {
        return Err(BAD_QUERY);
    }
    let distinct = p.keyword("DISTINCT");
    let columns = if p.peek() == Some(&Token::Star) {
        p.i += 1;
        None
    } else {
        let mut list = alloc::vec![p.column()?];
        while p.peek() == Some(&Token::Comma) {
            p.i += 1;
            list.push(p.column()?);
        }
        Some(list)
    };
    if !p.keyword("FROM") {
        return Err(BAD_QUERY);
    }
    let mut tables = alloc::vec![p.name()?];
    while p.peek() == Some(&Token::Comma) {
        p.i += 1;
        tables.push(p.name()?);
    }
    let filter = if p.keyword("WHERE") {
        Some(p.expr()?)
    } else {
        None
    };
    let mut order = Vec::new();
    if p.keyword("ORDER") {
        if !p.keyword("BY") {
            return Err(BAD_QUERY);
        }
        order.push(p.column()?);
        while p.peek() == Some(&Token::Comma) {
            p.i += 1;
            order.push(p.column()?);
        }
    }
    if p.i != p.t.len() {
        return Err(BAD_QUERY);
    }
    Ok(Query {
        distinct,
        columns,
        tables,
        filter,
        order,
        parameters: p.params,
    })
}

/// A column of a view's result: its name and Windows Installer's type
/// string (`s72`, `I2`, `V0`, ...).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnInfo {
    pub name: String,
    pub kind: String,
}
fn type_string(kind: u16) -> String {
    use door_msi::column::*;
    let nullable = kind & NULLABLE != 0;
    let size = kind & SIZE;
    let letter = if kind & !(NULLABLE | KEY) == STRING | VALID {
        'v'
    } else if kind & STRING != 0 {
        if kind & LOCALIZABLE != 0 { 'l' } else { 's' }
    } else {
        'i'
    };
    let letter = if nullable {
        letter.to_ascii_uppercase()
    } else {
        letter
    };
    let size = if letter.eq_ignore_ascii_case(&'i') {
        if size <= 2 { 2 } else { 4 }
    } else {
        size
    };
    alloc::format!("{letter}{size}")
}

/// The rows a query returns, with its columns.
pub fn run(
    db: &Database,
    q: &Query,
    params: &[Field],
) -> Result<(Vec<ColumnInfo>, Vec<Vec<Field>>), u32> {
    // The pseudo-tables first.
    let mut loaded: Vec<Table> = Vec::new();
    for name in &q.tables {
        let t = match name.as_str() {
            "_Tables" => Table {
                name: name.clone(),
                columns: alloc::vec![door_msi::Column {
                    name: String::from("Name"),
                    kind: 0x2d40
                }],
                rows: db
                    .tables
                    .iter()
                    .map(|t| alloc::vec![Value::Str(t.0.clone())])
                    .collect(),
            },
            "_Columns" => Table {
                name: name.clone(),
                columns: alloc::vec![
                    door_msi::Column {
                        name: String::from("Table"),
                        kind: 0x2d40
                    },
                    door_msi::Column {
                        name: String::from("Number"),
                        kind: 0x2502
                    },
                    door_msi::Column {
                        name: String::from("Name"),
                        kind: 0x0d40
                    },
                    door_msi::Column {
                        name: String::from("Type"),
                        kind: 0x0502
                    },
                ],
                rows: db
                    .tables
                    .iter()
                    .flat_map(|t| {
                        t.1.iter().enumerate().map(move |(i, c)| {
                            alloc::vec![
                                Value::Str(t.0.clone()),
                                Value::Int(i as i32 + 1),
                                Value::Str(c.name.clone()),
                                Value::Int(i32::from(c.kind)),
                            ]
                        })
                    })
                    .collect(),
            },
            "_Streams" => Table {
                name: name.clone(),
                columns: alloc::vec![
                    door_msi::Column {
                        name: String::from("Name"),
                        kind: 0x2d3e
                    },
                    door_msi::Column {
                        name: String::from("Data"),
                        kind: 0x0900
                    },
                ],
                rows: db
                    .streams()
                    .into_iter()
                    .filter(|s| !s.1 && !s.0.starts_with('\u{5}'))
                    .map(|s| alloc::vec![Value::Str(s.0.clone()), Value::Stream(s.0)])
                    .collect(),
            },
            _ => db.table(name).map_err(|_| INVALID_TABLE)?,
        };
        loaded.push(t);
    }
    // Resolve a column to (table index, column index).
    let resolve = |c: &Column| -> Result<(usize, usize), u32> {
        let mut found = None;
        for (ti, t) in loaded.iter().enumerate() {
            if c.table.as_ref().is_some_and(|n| *n != t.name) {
                continue;
            }
            if let Some(ci) = t.column(&c.name) {
                if found.is_some() {
                    return Err(BAD_QUERY);
                }
                found = Some((ti, ci));
            }
        }
        found.ok_or(INVALID_TABLE)
    };
    let columns: Vec<(usize, usize)> = match &q.columns {
        None => loaded
            .iter()
            .enumerate()
            .flat_map(|(ti, t)| (0..t.columns.len()).map(move |ci| (ti, ci)))
            .collect(),
        Some(list) => list.iter().map(resolve).collect::<Result<_, _>>()?,
    };
    let info: Vec<ColumnInfo> = columns
        .iter()
        .map(|&(t, c)| ColumnInfo {
            name: loaded[t].columns[c].name.clone(),
            kind: type_string(loaded[t].columns[c].kind),
        })
        .collect();
    type Resolve<'a> = &'a dyn Fn(&Column) -> Result<(usize, usize), u32>;
    fn check(e: &Expr, resolve: Resolve) -> Result<(), u32> {
        match e {
            Expr::Compare(a, _, b) => {
                for o in [a, b] {
                    if let Operand::Column(c) = o {
                        resolve(c)?;
                    }
                }
                Ok(())
            }
            Expr::IsNull(c, _) => resolve(c).map(|_| ()),
            Expr::And(a, b) | Expr::Or(a, b) => {
                check(a, resolve)?;
                check(b, resolve)
            }
        }
    }
    if let Some(f) = &q.filter {
        check(f, &resolve)?;
    }
    for c in &q.order {
        resolve(c)?;
    }
    // The product of the tables' rows.
    let mut combos: Vec<Vec<usize>> = alloc::vec![Vec::new()];
    for t in &loaded {
        let mut next = Vec::new();
        for c in &combos {
            for r in 0..t.rows.len() {
                let mut n = c.clone();
                n.push(r);
                next.push(n);
            }
        }
        combos = next;
        if combos.len() > 1_000_000 {
            return Err(BAD_QUERY);
        }
    }
    let value = |combo: &[usize], c: &Column| -> Value {
        let (t, ci) = resolve(c).unwrap();
        loaded[t].rows[combo[t]][ci].clone()
    };
    let operand = |combo: &[usize], o: &Operand| -> Value {
        match o {
            Operand::Column(c) => value(combo, c),
            Operand::Text(s) => Value::Str(s.clone()),
            Operand::Number(n) => Value::Int(*n),
            Operand::Param(i) => match params.get(*i) {
                Some(Field::Int(n)) => Value::Int(*n),
                Some(Field::Str(s)) => Value::Str(s.clone()),
                _ => Value::Null,
            },
        }
    };
    fn eval(
        e: &Expr,
        combo: &[usize],
        operand: &dyn Fn(&[usize], &Operand) -> Value,
        value: &dyn Fn(&[usize], &Column) -> Value,
    ) -> bool {
        match e {
            Expr::And(a, b) => eval(a, combo, operand, value) && eval(b, combo, operand, value),
            Expr::Or(a, b) => eval(a, combo, operand, value) || eval(b, combo, operand, value),
            Expr::IsNull(c, null) => value(combo, c).is_null() == *null,
            Expr::Compare(a, op, b) => {
                let (x, y) = (operand(combo, a), operand(combo, b));
                let ord = match (&x, &y) {
                    (Value::Int(p), Value::Int(q)) => p.cmp(q),
                    (Value::Str(p), Value::Str(q)) => p.cmp(q),
                    (Value::Null, Value::Null) => core::cmp::Ordering::Equal,
                    _ => return *op == "<>",
                };
                match *op {
                    "=" => ord.is_eq(),
                    "<>" => !ord.is_eq(),
                    "<" => ord.is_lt(),
                    ">" => ord.is_gt(),
                    "<=" => ord.is_le(),
                    _ => ord.is_ge(),
                }
            }
        }
    }
    let mut kept: Vec<Vec<usize>> = combos
        .into_iter()
        .filter(|c| {
            q.filter
                .as_ref()
                .is_none_or(|f| eval(f, c, &operand, &value))
        })
        .collect();
    if !q.order.is_empty() {
        kept.sort_by(|a, b| {
            for c in &q.order {
                let (x, y) = (value(a, c), value(b, c));
                let o = match (&x, &y) {
                    (Value::Int(p), Value::Int(q)) => p.cmp(q),
                    (Value::Str(p), Value::Str(q)) => p.cmp(q),
                    (Value::Null, Value::Null) => core::cmp::Ordering::Equal,
                    (Value::Null, _) => core::cmp::Ordering::Less,
                    (_, Value::Null) => core::cmp::Ordering::Greater,
                    _ => core::cmp::Ordering::Equal,
                };
                if !o.is_eq() {
                    return o;
                }
            }
            core::cmp::Ordering::Equal
        });
    }
    let mut rows: Vec<Vec<Field>> = kept
        .iter()
        .map(|combo| {
            columns
                .iter()
                .map(|&(t, c)| Field::from(&loaded[t].rows[combo[t]][c]))
                .collect()
        })
        .collect();
    if q.distinct {
        let mut unique: Vec<Vec<Field>> = Vec::new();
        for r in rows {
            if !unique.contains(&r) {
                unique.push(r);
            }
        }
        rows = unique;
    }
    Ok((info, rows))
}
