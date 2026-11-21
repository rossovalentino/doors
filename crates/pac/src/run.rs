//! The interpreter: values, scopes (closures share their defining scope),
//! JavaScript's conversions and operators as PAC scripts use them, string
//! and array methods, a small `Date`, and the PAC helper functions over the
//! `Host`. Every loop and call counts against the step and depth budgets.
use crate::{
    Error, Host, MAX_ARRAY, MAX_DEPTH, MAX_STEPS, MAX_STRING, dotted,
    parse::{Expr, Function, Program, Stmt},
    parse_dotted, shell_match,
};
use alloc::{
    collections::BTreeMap,
    format,
    rc::Rc,
    string::{String, ToString},
    vec::Vec,
};
use core::cell::RefCell;

#[derive(Clone, Debug)]
pub enum Value {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Array(Rc<RefCell<Vec<Value>>>),
    Function(Rc<Closure>),
    Native(Native),
    /// `new Date()`: (year, month 1-12, day, weekday, hour, minute,
    /// second), local and UTC.
    Date([i64; 7], [i64; 7]),
    /// `Math` and other built-in namespaces, by name.
    Namespace(&'static str),
}
#[derive(Debug)]
pub struct Closure {
    function: Rc<Function>,
    scope: Rc<RefCell<Scope>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Native {
    IsPlainHostName,
    DnsDomainIs,
    LocalHostOrDomainIs,
    IsResolvable,
    IsResolvableEx,
    IsInNet,
    IsInNetEx,
    DnsResolve,
    DnsResolveEx,
    MyIpAddress,
    MyIpAddressEx,
    DnsDomainLevels,
    ShExpMatch,
    WeekdayRange,
    DateRange,
    TimeRange,
    Alert,
    SortIpAddressList,
    GetClientVersion,
    ParseInt,
    ParseFloat,
    IsNaN,
    StringOf,
    NumberOf,
    DateOf,
    ArrayOf,
}
const NATIVES: [(&str, Native); 26] = [
    ("isPlainHostName", Native::IsPlainHostName),
    ("dnsDomainIs", Native::DnsDomainIs),
    ("localHostOrDomainIs", Native::LocalHostOrDomainIs),
    ("isResolvable", Native::IsResolvable),
    ("isResolvableEx", Native::IsResolvableEx),
    ("isInNet", Native::IsInNet),
    ("isInNetEx", Native::IsInNetEx),
    ("dnsResolve", Native::DnsResolve),
    ("dnsResolveEx", Native::DnsResolveEx),
    ("myIpAddress", Native::MyIpAddress),
    ("myIpAddressEx", Native::MyIpAddressEx),
    ("dnsDomainLevels", Native::DnsDomainLevels),
    ("shExpMatch", Native::ShExpMatch),
    ("weekdayRange", Native::WeekdayRange),
    ("dateRange", Native::DateRange),
    ("timeRange", Native::TimeRange),
    ("alert", Native::Alert),
    ("sortIpAddressList", Native::SortIpAddressList),
    ("getClientVersion", Native::GetClientVersion),
    ("parseInt", Native::ParseInt),
    ("parseFloat", Native::ParseFloat),
    ("isNaN", Native::IsNaN),
    ("String", Native::StringOf),
    ("Number", Native::NumberOf),
    ("Date", Native::DateOf),
    ("Array", Native::ArrayOf),
];

#[derive(Debug, Default)]
pub struct Scope {
    vars: BTreeMap<String, Value>,
    parent: Option<Rc<RefCell<Scope>>>,
}
impl Scope {
    fn lookup(scope: &Rc<RefCell<Scope>>, name: &str) -> Option<Value> {
        let mut at = scope.clone();
        loop {
            if let Some(v) = at.borrow().vars.get(name) {
                return Some(v.clone());
            }
            let parent = at.borrow().parent.clone()?;
            at = parent;
        }
    }
    /// Assign where the name lives, else as a global.
    fn assign(scope: &Rc<RefCell<Scope>>, name: &str, value: Value) {
        let mut at = scope.clone();
        loop {
            if at.borrow().vars.contains_key(name) {
                at.borrow_mut().vars.insert(String::from(name), value);
                return;
            }
            let parent = at.borrow().parent.clone();
            match parent {
                Some(p) => at = p,
                None => {
                    at.borrow_mut().vars.insert(String::from(name), value);
                    return;
                }
            }
        }
    }
}

enum Flow {
    Normal,
    Return(Value),
    Break,
    Continue,
}

struct Machine<'h> {
    host: &'h mut dyn Host,
    steps: u64,
    depth: usize,
}
fn limit(what: &'static str) -> Error {
    Error::Runtime(what)
}

/// Run the program's top level, then call `entry` with string arguments.
pub fn call(
    program: &Program,
    entry: &str,
    args: &[&str],
    host: &mut dyn Host,
) -> Result<Value, Error> {
    let global = Rc::new(RefCell::new(Scope::default()));
    let mut m = Machine {
        host,
        steps: 0,
        depth: 0,
    };
    m.hoist(&program.body, &global);
    for s in &program.body {
        match m.statement(s, &global)? {
            Flow::Normal => (),
            _ => return Err(limit("return outside a function")),
        }
    }
    let f = Scope::lookup(&global, entry).ok_or(Error::NoEntry)?;
    let args: Vec<Value> = args.iter().map(|a| Value::Str(String::from(*a))).collect();
    m.invoke(&f, &args)
}

impl Value {
    pub fn to_text(&self) -> String {
        match self {
            Value::Undefined => String::from("undefined"),
            Value::Null => String::from("null"),
            Value::Bool(b) => String::from(if *b { "true" } else { "false" }),
            Value::Number(n) => number_text(*n),
            Value::Str(s) => s.clone(),
            Value::Array(a) => {
                let items: Vec<String> = a
                    .borrow()
                    .iter()
                    .map(|v| match v {
                        Value::Undefined | Value::Null => String::new(),
                        v => v.to_text(),
                    })
                    .collect();
                items.join(",")
            }
            Value::Function(_) | Value::Native(_) => String::from("function"),
            Value::Date(local, _) => format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                local[0], local[1], local[2], local[4], local[5], local[6]
            ),
            Value::Namespace(n) => format!("[object {n}]"),
        }
    }
    fn to_number(&self) -> f64 {
        match self {
            Value::Undefined => f64::NAN,
            Value::Null => 0.0,
            Value::Bool(b) => f64::from(u8::from(*b)),
            Value::Number(n) => *n,
            Value::Str(s) => {
                let t = s.trim();
                if t.is_empty() {
                    0.0
                } else if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
                    u64::from_str_radix(hex, 16).map_or(f64::NAN, |v| v as f64)
                } else {
                    t.parse().unwrap_or(f64::NAN)
                }
            }
            Value::Array(a) => {
                let a = a.borrow();
                match a.len() {
                    0 => 0.0,
                    1 => a[0].to_number(),
                    _ => f64::NAN,
                }
            }
            _ => f64::NAN,
        }
    }
    fn truthy(&self) -> bool {
        match self {
            Value::Undefined | Value::Null => false,
            Value::Bool(b) => *b,
            Value::Number(n) => *n != 0.0 && !n.is_nan(),
            Value::Str(s) => !s.is_empty(),
            _ => true,
        }
    }
    fn type_name(&self) -> &'static str {
        match self {
            Value::Undefined => "undefined",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::Str(_) => "string",
            Value::Function(_) | Value::Native(_) => "function",
            _ => "object",
        }
    }
}
fn number_text(n: f64) -> String {
    if n.is_nan() {
        return String::from("NaN");
    }
    if n.is_infinite() {
        return String::from(if n > 0.0 { "Infinity" } else { "-Infinity" });
    }
    if n == libm_trunc(n) && abs(n) < 1e21 {
        return format!("{}", n as i64);
    }
    format!("{n}")
}
fn libm_trunc(n: f64) -> f64 {
    // f64::trunc needs std; exact for |n| < 2^52 by the i64 round trip.
    if abs(n) >= 4_503_599_627_370_496.0 {
        n
    } else {
        (n as i64) as f64
    }
}
fn to_int32(n: f64) -> i32 {
    if !n.is_finite() {
        return 0;
    }
    (libm_trunc(n) as i64) as i32
}
fn strict_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Array(x), Value::Array(y)) => Rc::ptr_eq(x, y),
        (Value::Function(x), Value::Function(y)) => Rc::ptr_eq(x, y),
        (Value::Native(x), Value::Native(y)) => x == y,
        _ => false,
    }
}
fn loose_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Undefined | Value::Null, Value::Undefined | Value::Null) => true,
        (Value::Undefined | Value::Null, _) | (_, Value::Undefined | Value::Null) => false,
        (Value::Number(_), Value::Str(_))
        | (Value::Str(_), Value::Number(_))
        | (Value::Bool(_), _)
        | (_, Value::Bool(_)) => a.to_number() == b.to_number(),
        (Value::Array(_), Value::Str(_)) | (Value::Str(_), Value::Array(_)) => {
            a.to_text() == b.to_text()
        }
        _ => strict_equal(a, b),
    }
}
fn checked_text(s: String) -> Result<Value, Error> {
    if s.len() > MAX_STRING {
        return Err(limit("string too long"));
    }
    Ok(Value::Str(s))
}

impl Machine<'_> {
    fn step(&mut self) -> Result<(), Error> {
        self.steps += 1;
        if self.steps > MAX_STEPS {
            return Err(limit("too many steps"));
        }
        Ok(())
    }
    /// Function declarations and `var`s exist before the code runs.
    fn hoist(&mut self, body: &[Stmt], scope: &Rc<RefCell<Scope>>) {
        for s in body {
            match s {
                Stmt::Function(f) => {
                    let closure = Value::Function(Rc::new(Closure {
                        function: f.clone(),
                        scope: scope.clone(),
                    }));
                    scope.borrow_mut().vars.insert(f.name.clone(), closure);
                }
                Stmt::Var(list) => {
                    for (name, _) in list {
                        scope
                            .borrow_mut()
                            .vars
                            .entry(name.clone())
                            .or_insert(Value::Undefined);
                    }
                }
                Stmt::If(_, a, b) => {
                    self.hoist(core::slice::from_ref(a), scope);
                    if let Some(b) = b {
                        self.hoist(core::slice::from_ref(b), scope);
                    }
                }
                Stmt::Block(inner) => self.hoist(inner, scope),
                Stmt::For(init, _, _, body) => {
                    if let Some(init) = init {
                        self.hoist(core::slice::from_ref(init), scope);
                    }
                    self.hoist(core::slice::from_ref(body), scope);
                }
                Stmt::While(_, body) | Stmt::DoWhile(body, _) => {
                    self.hoist(core::slice::from_ref(body), scope)
                }
                _ => (),
            }
        }
    }
    fn statement(&mut self, s: &Stmt, scope: &Rc<RefCell<Scope>>) -> Result<Flow, Error> {
        self.step()?;
        match s {
            Stmt::Empty | Stmt::Function(_) => Ok(Flow::Normal),
            Stmt::Var(list) => {
                for (name, value) in list {
                    if let Some(e) = value {
                        let v = self.expr(e, scope)?;
                        scope.borrow_mut().vars.insert(name.clone(), v);
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::Expr(e) => {
                self.expr(e, scope)?;
                Ok(Flow::Normal)
            }
            Stmt::Return(e) => Ok(Flow::Return(match e {
                Some(e) => self.expr(e, scope)?,
                None => Value::Undefined,
            })),
            Stmt::Break => Ok(Flow::Break),
            Stmt::Continue => Ok(Flow::Continue),
            Stmt::Block(body) => {
                for s in body {
                    match self.statement(s, scope)? {
                        Flow::Normal => (),
                        other => return Ok(other),
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::If(condition, then, otherwise) => {
                if self.expr(condition, scope)?.truthy() {
                    self.statement(then, scope)
                } else if let Some(o) = otherwise {
                    self.statement(o, scope)
                } else {
                    Ok(Flow::Normal)
                }
            }
            Stmt::While(condition, body) => {
                while self.expr(condition, scope)?.truthy() {
                    match self.statement(body, scope)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => (),
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::DoWhile(body, condition) => {
                loop {
                    match self.statement(body, scope)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => (),
                    }
                    if !self.expr(condition, scope)?.truthy() {
                        break;
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::For(init, condition, update, body) => {
                if let Some(init) = init {
                    self.statement(init, scope)?;
                }
                loop {
                    if let Some(c) = condition
                        && !self.expr(c, scope)?.truthy()
                    {
                        break;
                    }
                    match self.statement(body, scope)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => (),
                    }
                    if let Some(u) = update {
                        self.expr(u, scope)?;
                    }
                }
                Ok(Flow::Normal)
            }
        }
    }
    fn name(&mut self, name: &str, scope: &Rc<RefCell<Scope>>) -> Result<Value, Error> {
        if let Some(v) = Scope::lookup(scope, name) {
            return Ok(v);
        }
        if let Some((_, n)) = NATIVES.iter().find(|(n, _)| *n == name) {
            return Ok(Value::Native(*n));
        }
        match name {
            "undefined" => Ok(Value::Undefined),
            "NaN" => Ok(Value::Number(f64::NAN)),
            "Infinity" => Ok(Value::Number(f64::INFINITY)),
            "Math" => Ok(Value::Namespace("Math")),
            _ => Err(limit("unknown name")),
        }
    }
    /// One expression. Compound forms live in their own (not inlined)
    /// functions, so the frame this recursion repeats stays small.
    fn expr(&mut self, e: &Expr, scope: &Rc<RefCell<Scope>>) -> Result<Value, Error> {
        self.step()?;
        match e {
            Expr::Number(n) => Ok(Value::Number(*n)),
            Expr::Str(s) => Ok(Value::Str(s.clone())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Null => Ok(Value::Null),
            Expr::Name(n) => self.name(n, scope),
            Expr::Array(items) => self.array(items, scope),
            Expr::Function(f) => Ok(Value::Function(Rc::new(Closure {
                function: f.clone(),
                scope: scope.clone(),
            }))),
            Expr::Unary(op, inner) => self.unary(op, inner, scope),
            Expr::Logical(op, a, b) => self.logical(op, a, b, scope),
            Expr::Ternary(c, yes, no) => {
                let pick = if self.expr(c, scope)?.truthy() {
                    yes
                } else {
                    no
                };
                self.expr(pick, scope)
            }
            Expr::Binary(op, a, b) => self.binary_expr(op, a, b, scope),
            Expr::Assign(op, target, value) => self.assign(op, target, value, scope),
            Expr::Update(prefix, step, target) => self.update(*prefix, *step, target, scope),
            Expr::Member(object, name) => {
                let o = self.expr(object, scope)?;
                Ok(member(&o, name))
            }
            Expr::Index(object, index) => self.index(object, index, scope),
            Expr::Call(callee, args) => self.call_expr(callee, args, scope),
        }
    }
    #[inline(never)]
    fn array(&mut self, items: &[Expr], scope: &Rc<RefCell<Scope>>) -> Result<Value, Error> {
        if items.len() > MAX_ARRAY {
            return Err(limit("array too long"));
        }
        let mut values = Vec::with_capacity(items.len());
        for i in items {
            values.push(self.expr(i, scope)?);
        }
        Ok(Value::Array(Rc::new(RefCell::new(values))))
    }
    #[inline(never)]
    fn unary(
        &mut self,
        op: &str,
        inner: &Expr,
        scope: &Rc<RefCell<Scope>>,
    ) -> Result<Value, Error> {
        if op == "typeof" {
            // typeof of an unknown name is "undefined", not an error.
            if let Expr::Name(n) = inner
                && self.name(n, scope).is_err()
            {
                return Ok(Value::Str(String::from("undefined")));
            }
            let v = self.expr(inner, scope)?;
            return Ok(Value::Str(String::from(v.type_name())));
        }
        let v = self.expr(inner, scope)?;
        Ok(match op {
            "!" => Value::Bool(!v.truthy()),
            "-" => Value::Number(-v.to_number()),
            "+" => Value::Number(v.to_number()),
            "~" => Value::Number(f64::from(!to_int32(v.to_number()))),
            _ => Value::Undefined,
        })
    }
    #[inline(never)]
    fn logical(
        &mut self,
        op: &str,
        a: &Expr,
        b: &Expr,
        scope: &Rc<RefCell<Scope>>,
    ) -> Result<Value, Error> {
        let left = self.expr(a, scope)?;
        let take_left = if op == "&&" {
            !left.truthy()
        } else {
            left.truthy()
        };
        if take_left {
            Ok(left)
        } else {
            self.expr(b, scope)
        }
    }
    #[inline(never)]
    fn binary_expr(
        &mut self,
        op: &str,
        a: &Expr,
        b: &Expr,
        scope: &Rc<RefCell<Scope>>,
    ) -> Result<Value, Error> {
        let left = self.expr(a, scope)?;
        let right = self.expr(b, scope)?;
        binary(op, &left, &right)
    }
    #[inline(never)]
    fn assign(
        &mut self,
        op: &str,
        target: &Expr,
        value: &Expr,
        scope: &Rc<RefCell<Scope>>,
    ) -> Result<Value, Error> {
        let mut v = self.expr(value, scope)?;
        if !op.is_empty() {
            let old = self.expr(target, scope)?;
            v = binary(op, &old, &v)?;
        }
        self.store(target, v.clone(), scope)?;
        Ok(v)
    }
    #[inline(never)]
    fn update(
        &mut self,
        prefix: bool,
        step: f64,
        target: &Expr,
        scope: &Rc<RefCell<Scope>>,
    ) -> Result<Value, Error> {
        let old = self.expr(target, scope)?.to_number();
        let new = old + step;
        self.store(target, Value::Number(new), scope)?;
        Ok(Value::Number(if prefix { new } else { old }))
    }
    #[inline(never)]
    fn index(
        &mut self,
        object: &Expr,
        index: &Expr,
        scope: &Rc<RefCell<Scope>>,
    ) -> Result<Value, Error> {
        let o = self.expr(object, scope)?;
        let i = self.expr(index, scope)?;
        Ok(match (&o, &i) {
            (Value::Array(a), Value::Number(n)) if *n >= 0.0 => a
                .borrow()
                .get(*n as usize)
                .cloned()
                .unwrap_or(Value::Undefined),
            (Value::Str(s), Value::Number(n)) if *n >= 0.0 => s
                .chars()
                .nth(*n as usize)
                .map_or(Value::Undefined, |c| Value::Str(c.to_string())),
            _ => member(&o, &i.to_text()),
        })
    }
    #[inline(never)]
    fn call_expr(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        scope: &Rc<RefCell<Scope>>,
    ) -> Result<Value, Error> {
        let mut values = Vec::with_capacity(args.len());
        // A method: the object, then its method by name.
        if let Expr::Member(object, method) = callee {
            let this = self.expr(object, scope)?;
            for a in args {
                values.push(self.expr(a, scope)?);
            }
            if matches!(this, Value::Function(_) | Value::Native(_))
                && (method == "call" || method == "apply")
            {
                let rest = if method == "apply" {
                    match values.get(1) {
                        Some(Value::Array(a)) => a.borrow().clone(),
                        _ => Vec::new(),
                    }
                } else {
                    values.into_iter().skip(1).collect()
                };
                return self.invoke(&this, &rest);
            }
            return self.method(&this, method, &values);
        }
        let f = self.expr(callee, scope)?;
        for a in args {
            values.push(self.expr(a, scope)?);
        }
        self.invoke(&f, &values)
    }
    fn store(&mut self, target: &Expr, v: Value, scope: &Rc<RefCell<Scope>>) -> Result<(), Error> {
        match target {
            Expr::Name(n) => {
                Scope::assign(scope, n, v);
                Ok(())
            }
            Expr::Index(object, index) => {
                let o = self.expr(object, scope)?;
                let i = self.expr(index, scope)?.to_number();
                if let Value::Array(a) = o
                    && i >= 0.0
                    && (i as usize) < MAX_ARRAY
                {
                    let mut a = a.borrow_mut();
                    let i = i as usize;
                    if a.len() <= i {
                        a.resize(i + 1, Value::Undefined);
                    }
                    a[i] = v;
                }
                Ok(())
            }
            // Properties of built-in values are read-only here.
            Expr::Member(..) => Ok(()),
            _ => Err(limit("bad assignment")),
        }
    }
    fn invoke(&mut self, f: &Value, args: &[Value]) -> Result<Value, Error> {
        match f {
            Value::Function(c) => {
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return Err(limit("calls nested too deeply"));
                }
                let scope = Rc::new(RefCell::new(Scope {
                    vars: BTreeMap::new(),
                    parent: Some(c.scope.clone()),
                }));
                for (i, p) in c.function.params.iter().enumerate() {
                    let v = args.get(i).cloned().unwrap_or(Value::Undefined);
                    scope.borrow_mut().vars.insert(p.clone(), v);
                }
                let arguments = Value::Array(Rc::new(RefCell::new(args.to_vec())));
                scope
                    .borrow_mut()
                    .vars
                    .entry(String::from("arguments"))
                    .or_insert(arguments);
                self.hoist(&c.function.body, &scope);
                let mut result = Value::Undefined;
                for s in &c.function.body {
                    match self.statement(s, &scope)? {
                        Flow::Normal => (),
                        Flow::Return(v) => {
                            result = v;
                            break;
                        }
                        Flow::Break | Flow::Continue => break,
                    }
                }
                self.depth -= 1;
                Ok(result)
            }
            Value::Native(n) => self.native(*n, args),
            _ => Err(limit("not a function")),
        }
    }
    #[inline(never)]
    fn method(&mut self, this: &Value, name: &str, args: &[Value]) -> Result<Value, Error> {
        let text = |i: usize| args.get(i).map(Value::to_text).unwrap_or_default();
        let int = |i: usize, default: f64| args.get(i).map_or(default, Value::to_number);
        match this {
            Value::Str(s) => {
                let chars: Vec<char> = s.chars().collect();
                let len = chars.len() as f64;
                let clamp = |n: f64| -> usize {
                    if n.is_nan() || n < 0.0 {
                        0
                    } else if n > len {
                        chars.len()
                    } else {
                        n as usize
                    }
                };
                let slice = |a: usize, b: usize| -> String { chars[a..b.max(a)].iter().collect() };
                Ok(match name {
                    "toLowerCase" | "toLocaleLowerCase" => Value::Str(s.to_lowercase()),
                    "toUpperCase" | "toLocaleUpperCase" => Value::Str(s.to_uppercase()),
                    "toString" | "valueOf" => Value::Str(s.clone()),
                    "trim" => Value::Str(String::from(s.trim())),
                    "charAt" => {
                        let i = int(0, 0.0);
                        Value::Str(if i >= 0.0 && i < len {
                            chars[i as usize].to_string()
                        } else {
                            String::new()
                        })
                    }
                    "charCodeAt" => {
                        let i = int(0, 0.0);
                        Value::Number(if i >= 0.0 && i < len {
                            f64::from(chars[i as usize] as u32)
                        } else {
                            f64::NAN
                        })
                    }
                    "indexOf" | "lastIndexOf" | "includes" | "startsWith" | "endsWith" => {
                        let needle: Vec<char> = text(0).chars().collect();
                        let found = |at: usize| {
                            at + needle.len() <= chars.len()
                                && chars[at..at + needle.len()] == needle[..]
                        };
                        match name {
                            "indexOf" => {
                                let from = clamp(int(1, 0.0));
                                Value::Number(
                                    (from..=chars.len())
                                        .find(|&i| found(i))
                                        .map_or(-1.0, |i| i as f64),
                                )
                            }
                            "lastIndexOf" => Value::Number(
                                (0..=chars.len())
                                    .rev()
                                    .find(|&i| found(i))
                                    .map_or(-1.0, |i| i as f64),
                            ),
                            "includes" => Value::Bool((0..=chars.len()).any(found)),
                            "startsWith" => Value::Bool(found(clamp(int(1, 0.0)))),
                            _ => Value::Bool(
                                chars.len() >= needle.len() && found(chars.len() - needle.len()),
                            ),
                        }
                    }
                    "substring" => {
                        let a = clamp(int(0, 0.0));
                        let b = clamp(int(1, len));
                        Value::Str(slice(a.min(b), a.max(b)))
                    }
                    "substr" => {
                        let start = int(0, 0.0);
                        let a = clamp(if start < 0.0 { len + start } else { start });
                        let count = int(1, len - a as f64);
                        let b = clamp(a as f64 + count.max(0.0));
                        Value::Str(slice(a, b))
                    }
                    "slice" => {
                        let rel = |n: f64| clamp(if n < 0.0 { len + n } else { n });
                        let a = rel(int(0, 0.0));
                        let b = rel(int(1, len));
                        Value::Str(slice(a, b))
                    }
                    "split" => {
                        let separator = text(0);
                        let parts: Vec<Value> = if args.is_empty() {
                            alloc::vec![Value::Str(s.clone())]
                        } else if separator.is_empty() {
                            chars.iter().map(|c| Value::Str(c.to_string())).collect()
                        } else {
                            s.split(separator.as_str())
                                .map(|p| Value::Str(String::from(p)))
                                .collect()
                        };
                        if parts.len() > MAX_ARRAY {
                            return Err(limit("array too long"));
                        }
                        Value::Array(Rc::new(RefCell::new(parts)))
                    }
                    "replace" => checked_text(s.replacen(text(0).as_str(), &text(1), 1))?,
                    "concat" => {
                        let mut out = s.clone();
                        for a in args {
                            out.push_str(&a.to_text());
                            if out.len() > MAX_STRING {
                                return Err(limit("string too long"));
                            }
                        }
                        Value::Str(out)
                    }
                    "match" | "search" => {
                        // Regular expressions are not supported; a literal
                        // match keeps simple uses working.
                        let needle = text(0);
                        match s.find(needle.as_str()) {
                            Some(i) if name == "search" => Value::Number(i as f64),
                            Some(_) => {
                                Value::Array(Rc::new(RefCell::new(alloc::vec![Value::Str(needle)])))
                            }
                            None if name == "search" => Value::Number(-1.0),
                            None => Value::Null,
                        }
                    }
                    _ => return Err(limit("unknown string method")),
                })
            }
            Value::Array(a) => Ok(match name {
                "join" => {
                    let separator = if args.is_empty() {
                        String::from(",")
                    } else {
                        text(0)
                    };
                    let items: Vec<String> = a.borrow().iter().map(Value::to_text).collect();
                    checked_text(items.join(&separator))?
                }
                "push" => {
                    let mut a = a.borrow_mut();
                    if a.len() + args.len() > MAX_ARRAY {
                        return Err(limit("array too long"));
                    }
                    a.extend(args.iter().cloned());
                    Value::Number(a.len() as f64)
                }
                "pop" => a.borrow_mut().pop().unwrap_or(Value::Undefined),
                "shift" => {
                    let mut a = a.borrow_mut();
                    if a.is_empty() {
                        Value::Undefined
                    } else {
                        a.remove(0)
                    }
                }
                "indexOf" => {
                    let needle = args.first().cloned().unwrap_or(Value::Undefined);
                    Value::Number(
                        a.borrow()
                            .iter()
                            .position(|v| strict_equal(v, &needle))
                            .map_or(-1.0, |i| i as f64),
                    )
                }
                "toString" => Value::Str(this.to_text()),
                "slice" => {
                    let a = a.borrow();
                    let len = a.len() as f64;
                    let rel = |n: f64| -> usize {
                        let n = if n < 0.0 { len + n } else { n };
                        n.clamp(0.0, len) as usize
                    };
                    let (x, y) = (rel(int(0, 0.0)), rel(int(1, len)));
                    Value::Array(Rc::new(RefCell::new(a[x..y.max(x)].to_vec())))
                }
                _ => return Err(limit("unknown array method")),
            }),
            Value::Date(local, utc) => {
                let (fields, rest) = match name.strip_prefix("getUTC") {
                    Some(rest) => (utc, rest),
                    None => (local, name.strip_prefix("get").unwrap_or(name)),
                };
                Ok(Value::Number(match rest {
                    "FullYear" | "Year" => fields[0] as f64,
                    "Month" => (fields[1] - 1) as f64,
                    "Date" => fields[2] as f64,
                    "Day" => fields[3] as f64,
                    "Hours" => fields[4] as f64,
                    "Minutes" => fields[5] as f64,
                    "Seconds" => fields[6] as f64,
                    "Milliseconds" => 0.0,
                    _ => return Err(limit("unknown date method")),
                }))
            }
            Value::Namespace("Math") => {
                let x = int(0, f64::NAN);
                Ok(Value::Number(match name {
                    "floor" => {
                        let t = libm_trunc(x);
                        if t > x { t - 1.0 } else { t }
                    }
                    "ceil" => {
                        let t = libm_trunc(x);
                        if t < x { t + 1.0 } else { t }
                    }
                    "round" => {
                        let t = libm_trunc(x + 0.5);
                        if t > x + 0.5 { t - 1.0 } else { t }
                    }
                    "abs" => abs(x),
                    "max" => args
                        .iter()
                        .map(Value::to_number)
                        .fold(f64::NEG_INFINITY, f64::max),
                    "min" => args
                        .iter()
                        .map(Value::to_number)
                        .fold(f64::INFINITY, f64::min),
                    _ => return Err(limit("unknown Math function")),
                }))
            }
            Value::Number(n) if name == "toString" => Ok(Value::Str(number_text(*n))),
            Value::Bool(_) if name == "toString" => Ok(Value::Str(this.to_text())),
            _ => Err(limit("not an object with methods")),
        }
    }
    #[inline(never)]
    fn native(&mut self, n: Native, args: &[Value]) -> Result<Value, Error> {
        let text = |i: usize| args.get(i).map(Value::to_text).unwrap_or_default();
        Ok(match n {
            Native::IsPlainHostName => Value::Bool(!text(0).contains('.')),
            Native::DnsDomainIs => {
                let (host, domain) = (text(0).to_ascii_lowercase(), text(1).to_ascii_lowercase());
                Value::Bool(host.ends_with(&domain))
            }
            Native::LocalHostOrDomainIs => {
                let (host, full) = (text(0).to_ascii_lowercase(), text(1).to_ascii_lowercase());
                Value::Bool(
                    host == full
                        || (!host.contains('.')
                            && full.starts_with(&host)
                            && full[host.len()..].starts_with('.')),
                )
            }
            Native::IsResolvable => Value::Bool(!self.addresses(&text(0)).is_empty()),
            Native::IsResolvableEx => Value::Bool(!self.host.resolve_ex(&text(0)).is_empty()),
            Native::IsInNet => {
                let (Some(pattern), Some(mask)) = (parse_dotted(&text(1)), parse_dotted(&text(2)))
                else {
                    return Ok(Value::Bool(false));
                };
                let inside = self
                    .addresses(&text(0))
                    .first()
                    .is_some_and(|a| (0..4).all(|i| a[i] & mask[i] == pattern[i] & mask[i]));
                Value::Bool(inside)
            }
            Native::IsInNetEx => {
                // "a.b.c.d/len" (IPv4 prefixes).
                let prefix = text(1);
                let Some((net, bits)) = prefix.split_once('/') else {
                    return Ok(Value::Bool(false));
                };
                let (Some(net), Ok(bits)) = (parse_dotted(net), bits.parse::<u32>()) else {
                    return Ok(Value::Bool(false));
                };
                if bits > 32 {
                    return Ok(Value::Bool(false));
                }
                let mask = if bits == 0 {
                    0
                } else {
                    u32::MAX << (32 - bits)
                };
                let net = u32::from_be_bytes(net) & mask;
                let inside = self
                    .addresses(&text(0))
                    .iter()
                    .any(|a| u32::from_be_bytes(*a) & mask == net);
                Value::Bool(inside)
            }
            Native::DnsResolve => match self.addresses(&text(0)).first() {
                Some(a) => Value::Str(dotted(*a)),
                None => Value::Null,
            },
            Native::DnsResolveEx => Value::Str(self.host.resolve_ex(&text(0)).join(";")),
            Native::MyIpAddress => Value::Str(dotted(self.host.my_address())),
            Native::MyIpAddressEx => Value::Str(self.host.my_addresses_ex().join(";")),
            Native::DnsDomainLevels => {
                Value::Number(text(0).bytes().filter(|b| *b == b'.').count() as f64)
            }
            Native::ShExpMatch => Value::Bool(shell_match(&text(0), &text(1))),
            Native::WeekdayRange => {
                let (gmt, args) = gmt_split(args);
                let now = self.host.now(gmt);
                let day = |v: &Value| weekday(&v.to_text());
                let today = now.3;
                Value::Bool(match args {
                    [a] => day(a) == Some(today),
                    [a, b, ..] => match (day(a), day(b)) {
                        (Some(a), Some(b)) if a <= b => (a..=b).contains(&today),
                        (Some(a), Some(b)) => today >= a || today <= b,
                        _ => false,
                    },
                    [] => false,
                })
            }
            Native::DateRange => {
                let (gmt, args) = gmt_split(args);
                let now = self.host.now(gmt);
                Value::Bool(date_range(args, now))
            }
            Native::TimeRange => {
                let (gmt, args) = gmt_split(args);
                let now = self.host.now(gmt);
                let n: Vec<i64> = args.iter().map(|v| v.to_number() as i64).collect();
                let secs = |h: i64, m: i64, s: i64| h * 3600 + m * 60 + s;
                let t = secs(i64::from(now.4), i64::from(now.5), i64::from(now.6));
                let within = |a: i64, b: i64| {
                    if a <= b {
                        t >= a && t <= b
                    } else {
                        t >= a || t <= b
                    }
                };
                Value::Bool(match n.as_slice() {
                    [h] => i64::from(now.4) == *h,
                    [h1, h2] => {
                        let h = i64::from(now.4);
                        if h1 <= h2 {
                            h >= *h1 && h < *h2
                        } else {
                            h >= *h1 || h < *h2
                        }
                    }
                    [h1, m1, h2, m2] => within(secs(*h1, *m1, 0), secs(*h2, *m2, 0)),
                    [h1, m1, s1, h2, m2, s2] => within(secs(*h1, *m1, *s1), secs(*h2, *m2, *s2)),
                    _ => false,
                })
            }
            Native::Alert => Value::Undefined,
            Native::SortIpAddressList => {
                let list = text(0);
                let mut items: Vec<&str> = list.split(';').filter(|s| !s.is_empty()).collect();
                items.sort_by_key(|s| parse_dotted(s).map(u32::from_be_bytes).unwrap_or(u32::MAX));
                Value::Str(items.join(";"))
            }
            Native::GetClientVersion => Value::Str(String::from("1.0")),
            Native::ParseInt => {
                let t = text(0);
                let t = t.trim();
                let radix = args.get(1).map_or(0.0, Value::to_number) as u32;
                let (negative, t) = match t.strip_prefix('-') {
                    Some(r) => (true, r),
                    None => (false, t.strip_prefix('+').unwrap_or(t)),
                };
                let (radix, t) =
                    match (radix, t.strip_prefix("0x").or_else(|| t.strip_prefix("0X"))) {
                        (0 | 16, Some(hex)) => (16, hex),
                        (0, None) => (10, t),
                        (r, _) => (r, t),
                    };
                if !(2..=36).contains(&radix) {
                    return Ok(Value::Number(f64::NAN));
                }
                let digits: String = t.chars().take_while(|c| c.is_digit(radix)).collect();
                let v = if digits.is_empty() {
                    f64::NAN
                } else {
                    digits.chars().fold(0.0, |acc, c| {
                        acc * f64::from(radix) + f64::from(c.to_digit(radix).unwrap_or(0))
                    })
                };
                Value::Number(if negative { -v } else { v })
            }
            Native::ParseFloat => {
                let t = text(0);
                let t = t.trim();
                let end = t
                    .char_indices()
                    .take_while(|(i, c)| {
                        c.is_ascii_digit() || *c == '.' || (*i == 0 && (*c == '-' || *c == '+'))
                    })
                    .last()
                    .map_or(0, |(i, c)| i + c.len_utf8());
                Value::Number(t[..end].parse().unwrap_or(f64::NAN))
            }
            Native::IsNaN => Value::Bool(args.first().map_or(f64::NAN, Value::to_number).is_nan()),
            Native::StringOf => Value::Str(text(0)),
            Native::NumberOf => Value::Number(args.first().map_or(0.0, Value::to_number)),
            Native::DateOf => {
                let fields = |t: (i32, u32, u32, u32, u32, u32, u32)| {
                    [
                        i64::from(t.0),
                        i64::from(t.1),
                        i64::from(t.2),
                        i64::from(t.3),
                        i64::from(t.4),
                        i64::from(t.5),
                        i64::from(t.6),
                    ]
                };
                let local = fields(self.host.now(false));
                let utc = fields(self.host.now(true));
                Value::Date(local, utc)
            }
            Native::ArrayOf => {
                if args.len() > MAX_ARRAY {
                    return Err(limit("array too long"));
                }
                Value::Array(Rc::new(RefCell::new(args.to_vec())))
            }
        })
    }
    /// A host's IPv4 addresses: itself when dotted, else resolved.
    fn addresses(&mut self, host: &str) -> Vec<[u8; 4]> {
        match parse_dotted(host) {
            Some(a) => alloc::vec![a],
            None if host.is_empty() => Vec::new(),
            None => self.host.resolve(host),
        }
    }
}
fn member(o: &Value, name: &str) -> Value {
    match (o, name) {
        (Value::Str(s), "length") => Value::Number(s.chars().count() as f64),
        (Value::Array(a), "length") => Value::Number(a.borrow().len() as f64),
        (Value::Namespace("Math"), "PI") => Value::Number(core::f64::consts::PI),
        _ => Value::Undefined,
    }
}
fn binary(op: &str, a: &Value, b: &Value) -> Result<Value, Error> {
    let num = |f: fn(f64, f64) -> f64| Value::Number(f(a.to_number(), b.to_number()));
    let int = |f: fn(i32, u32) -> i32| {
        Value::Number(f64::from(f(
            to_int32(a.to_number()),
            to_int32(b.to_number()) as u32 & 31,
        )))
    };
    Ok(match op {
        "," => b.clone(),
        "+" => {
            let textual =
                |v: &Value| matches!(v, Value::Str(_) | Value::Array(_) | Value::Date(..));
            if textual(a) || textual(b) {
                let mut s = a.to_text();
                s.push_str(&b.to_text());
                checked_text(s)?
            } else {
                num(|x, y| x + y)
            }
        }
        "-" => num(|x, y| x - y),
        "*" => num(|x, y| x * y),
        "/" => num(|x, y| x / y),
        "%" => num(|x, y| {
            if y == 0.0 || x.is_infinite() || x.is_nan() || y.is_nan() {
                f64::NAN
            } else {
                x - libm_trunc(x / y) * y
            }
        }),
        "==" => Value::Bool(loose_equal(a, b)),
        "!=" => Value::Bool(!loose_equal(a, b)),
        "===" => Value::Bool(strict_equal(a, b)),
        "!==" => Value::Bool(!strict_equal(a, b)),
        "<" | ">" | "<=" | ">=" => {
            let ordering = match (a, b) {
                (Value::Str(x), Value::Str(y)) => Some(x.cmp(y)),
                _ => a.to_number().partial_cmp(&b.to_number()),
            };
            Value::Bool(match (op, ordering) {
                (_, None) => false,
                ("<", Some(o)) => o.is_lt(),
                (">", Some(o)) => o.is_gt(),
                ("<=", Some(o)) => o.is_le(),
                (_, Some(o)) => o.is_ge(),
            })
        }
        "&" => int(|x, y| x & y as i32),
        "|" => int(|x, y| x | y as i32),
        "^" => int(|x, y| x ^ y as i32),
        "<<" => int(|x, y| x.wrapping_shl(y)),
        ">>" => int(|x, y| x.wrapping_shr(y)),
        ">>>" => Value::Number(f64::from(
            (to_int32(a.to_number()) as u32).wrapping_shr(to_int32(b.to_number()) as u32 & 31),
        )),
        "in" => Value::Bool(match b {
            Value::Array(arr) => {
                let n = a.to_number();
                n >= 0.0 && (n as usize) < arr.borrow().len()
            }
            _ => false,
        }),
        "instanceof" => Value::Bool(false),
        _ => return Err(limit("unknown operator")),
    })
}
/// A trailing "GMT" argument: UTC, and the rest.
fn gmt_split(args: &[Value]) -> (bool, &[Value]) {
    match args.last() {
        Some(Value::Str(s)) if s.eq_ignore_ascii_case("GMT") => (true, &args[..args.len() - 1]),
        _ => (false, args),
    }
}
fn weekday(name: &str) -> Option<u32> {
    ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"]
        .iter()
        .position(|d| d.eq_ignore_ascii_case(name))
        .map(|d| d as u32)
}
fn month(name: &str) -> Option<i64> {
    [
        "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
    ]
    .iter()
    .position(|m| m.eq_ignore_ascii_case(name))
    .map(|m| m as i64 + 1)
}
/// dateRange's forms: days, months, years, and their combinations, each a
/// single value or a (wrapping) range.
fn date_range(args: &[Value], now: (i32, u32, u32, u32, u32, u32, u32)) -> bool {
    #[derive(Clone, Copy)]
    enum Part {
        Day(i64),
        Month(i64),
        Year(i64),
    }
    let parts: Vec<Option<Part>> = args
        .iter()
        .map(|v| match v {
            Value::Str(s) => month(s).map(Part::Month),
            v => {
                let n = v.to_number() as i64;
                if (1..=31).contains(&n) {
                    Some(Part::Day(n))
                } else if n > 31 {
                    Some(Part::Year(n))
                } else {
                    None
                }
            }
        })
        .collect();
    if parts.iter().any(Option::is_none) {
        return false;
    }
    let parts: Vec<Part> = parts.into_iter().flatten().collect();
    let (y, m, d) = (i64::from(now.0), i64::from(now.1), i64::from(now.2));
    // A date as one comparable number, from the fields given.
    let key = |list: &[Part]| -> (i64, i64) {
        let (mut value, mut current) = (0, 0);
        for p in list {
            match p {
                Part::Year(v) => {
                    value += v * 10_000;
                    current += y * 10_000;
                }
                Part::Month(v) => {
                    value += v * 100;
                    current += m * 100;
                }
                Part::Day(v) => {
                    value += v;
                    current += d;
                }
            }
        }
        (value, current)
    };
    match parts.len() {
        1 => {
            let (v, c) = key(&parts);
            v == c
        }
        2 | 4 | 6 => {
            let half = parts.len() / 2;
            let (a, c) = key(&parts[..half]);
            let (b, _) = key(&parts[half..]);
            if a <= b {
                c >= a && c <= b
            } else {
                c >= a || c <= b
            }
        }
        _ => false,
    }
}
fn abs(x: f64) -> f64 {
    if x < 0.0 { -x } else { x }
}
