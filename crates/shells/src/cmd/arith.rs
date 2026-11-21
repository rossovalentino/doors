//! `SET /A`: cmd.exe's 32-bit signed integer expressions with wraparound.
//! Precedence (high to low): grouping, unary `! ~ -`, `* / %`, `+ -`,
//! `<< >>`, `&`, `^`, `|`, assignments (`= *= /= %= += -= &= ^= |= <<= >>=`)
//! and `,`. Names are variables (unset or non-numeric are 0); numbers are
//! decimal, `0x` hex or leading-`0` octal.
use alloc::{format, string::String, vec::Vec};

pub trait Vars {
    fn get(&mut self, name: &str) -> Option<String>;
    fn set(&mut self, name: &str, value: i32);
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Syntax,
    DivideByZero,
    Missing(&'static str),
    Unbalanced,
    InvalidNumber,
}
impl Error {
    pub fn message(&self) -> String {
        match self {
            Self::Syntax => String::from("Missing operand."),
            Self::DivideByZero => String::from("Divide by zero error."),
            Self::Missing(what) => format!("Missing {what}."),
            Self::Unbalanced => String::from("Unbalanced parenthesis."),
            Self::InvalidNumber => String::from(
                "Invalid number.  Numeric constants are either decimal (17),\nhexadecimal (0x11), or octal (021).",
            ),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(i32),
    Name(String),
    Op(&'static str),
    Open,
    Close,
    Comma,
}
fn tokens(expr: &str) -> Result<Vec<Tok>, Error> {
    let c: Vec<char> = expr.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    const OPS: [&str; 22] = [
        "<<=", ">>=", "<<", ">>", "*=", "/=", "%=", "+=", "-=", "&=", "^=", "|=", "*", "/", "%",
        "+", "-", "&", "^", "|", "!", "~",
    ];
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() || ch == '"' {
            i += 1;
            continue;
        }
        if ch.is_ascii_digit() {
            let start = i;
            while i < c.len() && c[i].is_ascii_alphanumeric() {
                i += 1;
            }
            let s: String = c[start..i].iter().collect();
            out.push(Tok::Num(number(&s)?));
            continue;
        }
        if ch == '(' {
            out.push(Tok::Open);
            i += 1;
            continue;
        }
        if ch == ')' {
            out.push(Tok::Close);
            i += 1;
            continue;
        }
        if ch == ',' {
            out.push(Tok::Comma);
            i += 1;
            continue;
        }
        if ch == '=' {
            out.push(Tok::Op("="));
            i += 1;
            continue;
        }
        if let Some(op) = OPS
            .iter()
            .find(|op| c[i..].iter().take(op.len()).copied().eq(op.chars()))
        {
            out.push(Tok::Op(op));
            i += op.len();
            continue;
        }
        // A variable name: anything up to an operator or space.
        let start = i;
        while i < c.len() && !c[i].is_whitespace() && !"()=,*/%+-&^|!~<>\"".contains(c[i]) {
            i += 1;
        }
        if start == i {
            return Err(Error::Syntax);
        }
        out.push(Tok::Name(c[start..i].iter().collect()));
    }
    Ok(out)
}
fn number(s: &str) -> Result<i32, Error> {
    let v = if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(h, 16).map_err(|_| Error::InvalidNumber)? as i64
    } else if s.len() > 1 && s.starts_with('0') {
        i64::from_str_radix(&s[1..], 8).map_err(|_| Error::InvalidNumber)?
    } else {
        s.parse::<i64>().map_err(|_| Error::InvalidNumber)?
    };
    if v > u32::MAX as i64 {
        return Err(Error::InvalidNumber);
    }
    Ok(v as u32 as i32)
}
/// A variable's numeric value (0 when unset or not a number).
fn value(vars: &mut dyn Vars, name: &str) -> i32 {
    let Some(text) = vars.get(name) else { return 0 };
    let t = text.trim();
    let (neg, t) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t),
    };
    let digits: String = t
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    let v = number(&digits).unwrap_or(0);
    if neg { v.wrapping_neg() } else { v }
}

struct P<'a> {
    t: Vec<Tok>,
    i: usize,
    vars: &'a mut dyn Vars,
}
/// Evaluate; the value of the last comma-separated expression.
pub fn eval(expr: &str, vars: &mut dyn Vars) -> Result<i32, Error> {
    let t = tokens(expr)?;
    if t.is_empty() {
        return Err(Error::Syntax);
    }
    let mut p = P { t, i: 0, vars };
    let mut v = p.assign()?;
    while p.t.get(p.i) == Some(&Tok::Comma) {
        p.i += 1;
        v = p.assign()?;
    }
    if p.i != p.t.len() {
        return Err(if p.t.get(p.i) == Some(&Tok::Close) {
            Error::Unbalanced
        } else {
            Error::Missing("operator")
        });
    }
    Ok(v)
}
impl P<'_> {
    fn assign(&mut self) -> Result<i32, Error> {
        if let (Some(Tok::Name(name)), Some(Tok::Op(op))) =
            (self.t.get(self.i).cloned(), self.t.get(self.i + 1).cloned())
            && op.ends_with('=')
        {
            self.i += 2;
            let rhs = self.assign()?;
            let cur = value(self.vars, &name);
            let v = match op {
                "=" => rhs,
                "*=" => cur.wrapping_mul(rhs),
                "/=" => {
                    if rhs == 0 {
                        return Err(Error::DivideByZero);
                    }
                    cur.wrapping_div(rhs)
                }
                "%=" => {
                    if rhs == 0 {
                        return Err(Error::DivideByZero);
                    }
                    cur.wrapping_rem(rhs)
                }
                "+=" => cur.wrapping_add(rhs),
                "-=" => cur.wrapping_sub(rhs),
                "&=" => cur & rhs,
                "^=" => cur ^ rhs,
                "|=" => cur | rhs,
                "<<=" => shl(cur, rhs),
                ">>=" => shr(cur, rhs),
                _ => return Err(Error::Syntax),
            };
            self.vars.set(&name, v);
            return Ok(v);
        }
        self.bitor()
    }
    fn binary(
        &mut self,
        ops: &[&str],
        next: fn(&mut Self) -> Result<i32, Error>,
    ) -> Result<i32, Error> {
        let mut v = next(self)?;
        while let Some(Tok::Op(op)) = self.t.get(self.i).cloned() {
            if !ops.contains(&op) {
                break;
            }
            self.i += 1;
            let r = next(self)?;
            v = match op {
                "|" => v | r,
                "^" => v ^ r,
                "&" => v & r,
                "<<" => shl(v, r),
                ">>" => shr(v, r),
                "+" => v.wrapping_add(r),
                "-" => v.wrapping_sub(r),
                "*" => v.wrapping_mul(r),
                "/" => {
                    if r == 0 {
                        return Err(Error::DivideByZero);
                    }
                    v.wrapping_div(r)
                }
                "%" => {
                    if r == 0 {
                        return Err(Error::DivideByZero);
                    }
                    v.wrapping_rem(r)
                }
                _ => unreachable!(),
            };
        }
        Ok(v)
    }
    fn bitor(&mut self) -> Result<i32, Error> {
        self.binary(&["|"], Self::bitxor)
    }
    fn bitxor(&mut self) -> Result<i32, Error> {
        self.binary(&["^"], Self::bitand)
    }
    fn bitand(&mut self) -> Result<i32, Error> {
        self.binary(&["&"], Self::shift)
    }
    fn shift(&mut self) -> Result<i32, Error> {
        self.binary(&["<<", ">>"], Self::additive)
    }
    fn additive(&mut self) -> Result<i32, Error> {
        self.binary(&["+", "-"], Self::multiplicative)
    }
    fn multiplicative(&mut self) -> Result<i32, Error> {
        self.binary(&["*", "/", "%"], Self::unary)
    }
    fn unary(&mut self) -> Result<i32, Error> {
        match self.t.get(self.i).cloned() {
            Some(Tok::Op("-")) => {
                self.i += 1;
                Ok(self.unary()?.wrapping_neg())
            }
            Some(Tok::Op("+")) => {
                self.i += 1;
                self.unary()
            }
            Some(Tok::Op("!")) => {
                self.i += 1;
                Ok(i32::from(self.unary()? == 0))
            }
            Some(Tok::Op("~")) => {
                self.i += 1;
                Ok(!self.unary()?)
            }
            _ => self.primary(),
        }
    }
    fn primary(&mut self) -> Result<i32, Error> {
        match self.t.get(self.i).cloned() {
            Some(Tok::Num(n)) => {
                self.i += 1;
                Ok(n)
            }
            Some(Tok::Name(name)) => {
                self.i += 1;
                Ok(value(self.vars, &name))
            }
            Some(Tok::Open) => {
                self.i += 1;
                let mut v = self.assign()?;
                while self.t.get(self.i) == Some(&Tok::Comma) {
                    self.i += 1;
                    v = self.assign()?;
                }
                if self.t.get(self.i) != Some(&Tok::Close) {
                    return Err(Error::Missing("closing parenthesis"));
                }
                self.i += 1;
                Ok(v)
            }
            _ => Err(Error::Syntax),
        }
    }
}
fn shl(v: i32, n: i32) -> i32 {
    if !(0..32).contains(&n) {
        0
    } else {
        v.wrapping_shl(n as u32)
    }
}
fn shr(v: i32, n: i32) -> i32 {
    if !(0..32).contains(&n) {
        if v < 0 { -1 } else { 0 }
    } else {
        v >> n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    struct V(BTreeMap<String, String>);
    impl Vars for V {
        fn get(&mut self, name: &str) -> Option<String> {
            self.0.get(&name.to_ascii_uppercase()).cloned()
        }
        fn set(&mut self, name: &str, value: i32) {
            self.0.insert(name.to_ascii_uppercase(), format!("{value}"));
        }
    }
    #[test]
    fn set_a_expressions() {
        let mut v = V(BTreeMap::new());
        assert_eq!(eval("3*4+1", &mut v), Ok(13));
        assert_eq!(eval("x=7, y=x<<2, x %= 4", &mut v), Ok(3));
        assert_eq!(v.0["X"], "3");
        assert_eq!(v.0["Y"], "28");
        assert_eq!(eval("(1+2)*3", &mut v), Ok(9));
        assert_eq!(eval("0x10 + 010", &mut v), Ok(24));
        assert_eq!(eval("2147483647 + 1", &mut v), Ok(-2147483648));
        assert_eq!(eval("-5 / 2", &mut v), Ok(-2));
        assert_eq!(eval("!0 + ~0", &mut v), Ok(0));
        assert_eq!(eval("1/0", &mut v), Err(Error::DivideByZero));
        assert_eq!(eval("09", &mut v), Err(Error::InvalidNumber));
        assert_eq!(eval("n+=5", &mut v), Ok(5));
        assert_eq!(eval("n*=n", &mut v), Ok(25));
        assert_eq!(eval("\"z=1<<4\"", &mut v), Ok(16));
        assert_eq!(eval("unknown*3", &mut v), Ok(0));
    }
}
