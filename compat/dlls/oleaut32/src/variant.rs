//! VARIANT values and VariantChangeType(Ex)'s conversions, measured on
//! Windows 11 (LCID 0x409): `tests/reference/change-type.txt` holds the
//! 46 × 20 matrix the tests replay.
use alloc::{string::String, vec::Vec};

use crate::{date, decimal::Decimal, hr, number, vt};

/// What a VARIANT holds (by value; BYREF is resolved by the caller).
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Empty,
    Null,
    I1(i8),
    UI1(u8),
    I2(i16),
    UI2(u16),
    I4(i32),
    UI4(u32),
    Int(i32),
    UInt(u32),
    I8(i64),
    UI8(u64),
    R4(f32),
    R8(f64),
    Cy(i64),
    Date(f64),
    Bstr(Vec<u16>),
    Bool(i16),
    Error(u32),
    Decimal(Decimal),
    /// An interface pointer (IUnknown / IDispatch).
    Unknown(u64),
    Dispatch(u64),
}
impl Value {
    pub fn vt(&self) -> u16 {
        match self {
            Self::Empty => vt::EMPTY,
            Self::Null => vt::NULL,
            Self::I1(_) => vt::I1,
            Self::UI1(_) => vt::UI1,
            Self::I2(_) => vt::I2,
            Self::UI2(_) => vt::UI2,
            Self::I4(_) => vt::I4,
            Self::UI4(_) => vt::UI4,
            Self::Int(_) => vt::INT,
            Self::UInt(_) => vt::UINT,
            Self::I8(_) => vt::I8,
            Self::UI8(_) => vt::UI8,
            Self::R4(_) => vt::R4,
            Self::R8(_) => vt::R8,
            Self::Cy(_) => vt::CY,
            Self::Date(_) => vt::DATE,
            Self::Bstr(_) => vt::BSTR,
            Self::Bool(_) => vt::BOOL,
            Self::Error(_) => vt::ERROR,
            Self::Decimal(_) => vt::DECIMAL,
            Self::Unknown(_) => vt::UNKNOWN,
            Self::Dispatch(_) => vt::DISPATCH,
        }
    }
    /// The integer and its byte size, for integer kinds.
    fn integer(&self) -> Option<(i128, u8, bool)> {
        Some(match *self {
            Self::I1(v) => (i128::from(v), 1, true),
            Self::UI1(v) => (i128::from(v), 1, false),
            Self::I2(v) => (i128::from(v), 2, true),
            Self::UI2(v) => (i128::from(v), 2, false),
            Self::I4(v) | Self::Int(v) => (i128::from(v), 4, true),
            Self::UI4(v) | Self::UInt(v) => (i128::from(v), 4, false),
            Self::I8(v) => (i128::from(v), 8, true),
            Self::UI8(v) => (i128::from(v), 8, false),
            _ => return None,
        })
    }
}

/// VariantChangeType flags.
pub mod flags {
    pub const NOVALUEPROP: u16 = 0x1;
    pub const ALPHABOOL: u16 = 0x2;
    pub const NOUSEROVERRIDE: u16 = 0x4;
    pub const CALENDAR_HIJRI: u16 = 0x8;
    pub const LOCALBOOL: u16 = 0x10;
}

/// A source as a number.
enum Num {
    Int(i128),
    /// A real and its precision (R4 7, R8/DATE 15).
    Real(f64, usize),
    Cy(i64),
    Dec(Decimal),
    Text(number::Parsed),
}
fn num_of(v: &Value) -> Result<Num, u32> {
    if let Some((i, _, _)) = v.integer() {
        return Ok(Num::Int(i));
    }
    Ok(match v {
        Value::Empty => Num::Int(0),
        Value::Bool(b) => Num::Int(i128::from(*b)),
        Value::R4(f) => Num::Real(f64::from(*f), 7),
        Value::R8(d) | Value::Date(d) => Num::Real(*d, 15),
        Value::Cy(c) => Num::Cy(*c),
        Value::Decimal(d) => Num::Dec(*d),
        Value::Bstr(s) => Num::Text(number::parse(s)?),
        _ => return Err(hr::DISP_E_TYPEMISMATCH),
    })
}
fn round_cy(c: i64) -> i128 {
    let q = i128::from(c) / 10_000;
    let r = (i128::from(c) % 10_000).abs();
    let up = r > 5000 || (r == 5000 && q % 2 != 0);
    if !up {
        q
    } else if c < 0 {
        q - 1
    } else {
        q + 1
    }
}
fn integer_of(n: &Num) -> Result<i128, u32> {
    match n {
        Num::Int(i) => Ok(*i),
        Num::Real(v, _) => {
            if !v.is_finite() || v.abs() >= 1e30 {
                return Err(hr::DISP_E_OVERFLOW);
            }
            Ok(crate::float::round_even(*v))
        }
        Num::Cy(c) => Ok(round_cy(*c)),
        Num::Dec(d) => Ok(d.to_integer()),
        Num::Text(p) => p.to_integer().ok_or(hr::DISP_E_OVERFLOW),
    }
}
fn real_of(n: &Num) -> Result<f64, u32> {
    Ok(match n {
        Num::Int(i) => *i as f64,
        Num::Real(v, _) => *v,
        Num::Cy(c) => *c as f64 / 10_000.0,
        Num::Dec(d) => d.to_f64(),
        Num::Text(p) => p.to_f64()?,
    })
}
fn cy_of(n: &Num) -> Result<i64, u32> {
    let v: i128 = match n {
        Num::Int(i) => i.checked_mul(10_000).ok_or(hr::DISP_E_OVERFLOW)?,
        Num::Real(v, _) => {
            if !v.is_finite() || v.abs() > 9.3e14 {
                return Err(hr::DISP_E_OVERFLOW);
            }
            // The exact value rounded to four places (half to even).
            let text = alloc::format!("{v:.4}");
            let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
            let m: i128 = digits.parse().map_err(|_| hr::DISP_E_OVERFLOW)?;
            if *v < 0.0 { -m } else { m }
        }
        Num::Cy(c) => i128::from(*c),
        Num::Dec(d) => {
            let m = d.rescale(4).ok_or(hr::DISP_E_OVERFLOW)? as i128;
            if d.negative { -m } else { m }
        }
        Num::Text(p) => {
            let d = Decimal::from_parsed(p)?;
            let m = d.rescale(4).ok_or(hr::DISP_E_OVERFLOW)? as i128;
            if d.negative { -m } else { m }
        }
    };
    i64::try_from(v).map_err(|_| hr::DISP_E_OVERFLOW)
}
fn decimal_of(n: &Num) -> Result<Decimal, u32> {
    match n {
        Num::Int(i) => Decimal::from_i128(*i),
        Num::Real(v, p) => Decimal::from_real(*v, *p),
        Num::Cy(c) => Ok(Decimal {
            negative: *c < 0,
            scale: 4,
            mantissa: u128::from(c.unsigned_abs()),
        }),
        Num::Dec(d) => Ok(*d),
        Num::Text(p) => Decimal::from_parsed(p),
    }
}
/// An integer target: its size and signedness.
fn integer_target(target: u16) -> Option<(u8, bool)> {
    Some(match target {
        vt::I1 => (1, true),
        vt::UI1 => (1, false),
        vt::I2 => (2, true),
        vt::UI2 => (2, false),
        vt::I4 | vt::INT => (4, true),
        vt::UI4 | vt::UINT => (4, false),
        vt::I8 => (8, true),
        vt::UI8 => (8, false),
        _ => return None,
    })
}
fn make_integer(target: u16, v: i128) -> Value {
    match target {
        vt::I1 => Value::I1(v as i8),
        vt::UI1 => Value::UI1(v as u8),
        vt::I2 => Value::I2(v as i16),
        vt::UI2 => Value::UI2(v as u16),
        vt::I4 => Value::I4(v as i32),
        vt::INT => Value::Int(v as i32),
        vt::UI4 => Value::UI4(v as u32),
        vt::UINT => Value::UInt(v as u32),
        vt::I8 => Value::I8(v as i64),
        _ => Value::UI8(v as u64),
    }
}
fn bool_text(b: i16, flags: u16) -> String {
    if flags & (flags::ALPHABOOL | flags::LOCALBOOL) != 0 {
        String::from(if b != 0 { "True" } else { "False" })
    } else {
        String::from(if b != 0 { "-1" } else { "0" })
    }
}
/// The text a value converts to (VarBstrFrom*).
pub fn text_of(v: &Value, flags: u16) -> Result<String, u32> {
    if let Some((i, _, _)) = v.integer() {
        return Ok(alloc::format!("{i}"));
    }
    Ok(match v {
        Value::Empty => String::new(),
        Value::R4(f) => number::format_real(f64::from(*f), 7),
        Value::R8(d) => number::format_real(*d, 15),
        Value::Date(d) => date::format(*d)?,
        Value::Cy(c) => number::format_scaled(*c < 0, u128::from(c.unsigned_abs()), 4),
        Value::Decimal(d) => d.text(),
        Value::Bool(b) => bool_text(*b, flags),
        Value::Bstr(s) => String::from_utf16_lossy(s),
        _ => return Err(hr::DISP_E_TYPEMISMATCH),
    })
}
/// The words a string may be as a BOOL ("True", "False", "#TRUE#",
/// "#FALSE#", any case).
fn bool_word(s: &[u16]) -> Option<bool> {
    let t = String::from_utf16_lossy(s);
    let t = t.trim();
    if t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("#true#") {
        Some(true)
    } else if t.eq_ignore_ascii_case("false") || t.eq_ignore_ascii_case("#false#") {
        Some(false)
    } else {
        None
    }
}
/// VariantChangeTypeEx for a by-value source.
pub fn change_type(src: &Value, target: u16, flags: u16) -> Result<Value, u32> {
    let mismatch = hr::DISP_E_TYPEMISMATCH;
    // The same type: a copy.
    if src.vt() == target {
        return Ok(src.clone());
    }
    match target {
        vt::EMPTY => {
            return match src {
                Value::Null | Value::Error(_) => Err(mismatch),
                _ => Ok(Value::Empty),
            };
        }
        vt::NULL => {
            return match src {
                Value::Error(_) => Err(mismatch),
                _ => Ok(Value::Null),
            };
        }
        vt::ERROR => return Err(mismatch),
        vt::BSTR => {
            let s = text_of(src, flags)?;
            return Ok(Value::Bstr(s.encode_utf16().collect()));
        }
        vt::UNKNOWN => {
            return match src {
                Value::Dispatch(p) => Ok(Value::Unknown(*p)),
                _ => Err(mismatch),
            };
        }
        vt::DISPATCH => return Err(mismatch),
        _ => (),
    }
    if matches!(
        src,
        Value::Null | Value::Error(_) | Value::Unknown(_) | Value::Dispatch(_)
    ) {
        return Err(mismatch);
    }
    if target == vt::BOOL {
        if let Value::Bstr(s) = src
            && let Some(b) = bool_word(s)
        {
            return Ok(Value::Bool(if b { -1 } else { 0 }));
        }
        let n = num_of(src)?;
        let nonzero = match &n {
            Num::Real(v, _) => *v != 0.0,
            Num::Text(p) => !p.digits.is_empty(),
            other => integer_of(other)? != 0 || matches!(other, Num::Cy(c) if *c != 0),
        };
        return Ok(Value::Bool(if nonzero { -1 } else { 0 }));
    }
    if target == vt::DATE {
        if let Value::Bstr(s) = src {
            return date::parse(&String::from_utf16_lossy(s)).map(Value::Date);
        }
        let v = real_of(&num_of(src)?)?;
        if !(date::MIN..=date::MAX).contains(&v) {
            return Err(hr::DISP_E_OVERFLOW);
        }
        return Ok(Value::Date(v));
    }
    if let Some((size, signed)) = integer_target(target) {
        // BOOL: its bits, sign-extended, as any integer.
        if let Value::Bool(b) = src {
            return Ok(make_integer(target, i128::from(*b)));
        }
        // An integer of the same size: its bits.
        if let Some((i, s, _)) = src.integer()
            && s == size
        {
            return Ok(make_integer(target, i));
        }
        let v = integer_of(&num_of(src)?)?;
        let bits = u32::from(size) * 8;
        let (lo, hi) = if signed {
            (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
        } else {
            (0, (1i128 << bits) - 1)
        };
        if v < lo || v > hi {
            return Err(hr::DISP_E_OVERFLOW);
        }
        return Ok(make_integer(target, v));
    }
    let n = num_of(src)?;
    match target {
        vt::R4 => {
            let v = real_of(&n)?;
            if v.is_finite() && v.abs() > f64::from(f32::MAX) {
                return Err(hr::DISP_E_OVERFLOW);
            }
            Ok(Value::R4(v as f32))
        }
        vt::R8 => Ok(Value::R8(real_of(&n)?)),
        vt::CY => Ok(Value::Cy(cy_of(&n)?)),
        vt::DECIMAL => Ok(Value::Decimal(decimal_of(&n)?)),
        _ => Err(hr::DISP_E_BADVARTYPE),
    }
}

// ---- arithmetic ----------------------------------------------------------------------
/// The type an arithmetic operation works in: Windows' promotion order.
fn rank(v: &Value) -> u8 {
    match v {
        Value::Empty | Value::Null | Value::UI1(_) => 1,
        Value::Bool(_) | Value::I2(_) => 2,
        Value::I1(_)
        | Value::UI2(_)
        | Value::I4(_)
        | Value::Int(_)
        | Value::UI4(_)
        | Value::UInt(_) => 3,
        Value::I8(_) | Value::UI8(_) => 4,
        Value::R4(_) => 5,
        Value::Bstr(_) | Value::R8(_) | Value::Date(_) => 6,
        Value::Cy(_) => 7,
        Value::Decimal(_) => 8,
        _ => 0,
    }
}
/// VarAdd/VarSub/VarMul (`op` '+', '-', '*'): integers grow to the next
/// size on overflow (UI1 → I2 → I4 → I8 → R8), reals stay reals; NULL
/// propagates; a string is a double.
pub fn arith(a: &Value, b: &Value, op: char) -> Result<Value, u32> {
    if matches!(a, Value::Null) || matches!(b, Value::Null) {
        return Ok(Value::Null);
    }
    if matches!(a, Value::Error(_) | Value::Unknown(_) | Value::Dispatch(_))
        || matches!(b, Value::Error(_) | Value::Unknown(_) | Value::Dispatch(_))
    {
        return Err(hr::DISP_E_TYPEMISMATCH);
    }
    let r = rank(a).max(rank(b));
    let na = num_of(a)?;
    let nb = num_of(b)?;
    if r <= 4 {
        let (x, y) = (integer_of(&na)?, integer_of(&nb)?);
        let v = match op {
            '+' => x + y,
            '-' => x - y,
            _ => x * y,
        };
        // UI1 (unsigned) only when both are UI1 (or empty) and it fits.
        let start = match r {
            1 => vt::UI1,
            2 => vt::I2,
            3 => vt::I4,
            _ => vt::I8,
        };
        for t in [vt::UI1, vt::I2, vt::I4, vt::I8] {
            if order(t) < order(start) {
                continue;
            }
            let (lo, hi) = range(t);
            if v >= lo && v <= hi {
                return Ok(make_integer(t, v));
            }
        }
        return Ok(Value::R8(v as f64));
    }
    match r {
        5 | 6 => {
            let (x, y) = (real_of(&na)?, real_of(&nb)?);
            let v = match op {
                '+' => x + y,
                '-' => x - y,
                _ => x * y,
            };
            if r == 5 && v.abs() <= f64::from(f32::MAX) {
                Ok(Value::R4(v as f32))
            } else {
                Ok(Value::R8(v))
            }
        }
        7 => {
            let (x, y) = (cy_of(&na)?, cy_of(&nb)?);
            let v = match op {
                '+' => i128::from(x) + i128::from(y),
                '-' => i128::from(x) - i128::from(y),
                _ => (i128::from(x) * i128::from(y) + 5000) / 10_000,
            };
            i64::try_from(v)
                .map(Value::Cy)
                .map_err(|_| hr::DISP_E_OVERFLOW)
        }
        _ => {
            let (x, y) = (real_of(&na)?, real_of(&nb)?);
            let v = match op {
                '+' => x + y,
                '-' => x - y,
                _ => x * y,
            };
            Decimal::from_real(v, 15).map(Value::Decimal)
        }
    }
}
fn order(t: u16) -> u8 {
    match t {
        vt::UI1 => 0,
        vt::I2 => 1,
        vt::I4 => 2,
        _ => 3,
    }
}
fn range(t: u16) -> (i128, i128) {
    match t {
        vt::UI1 => (0, 255),
        vt::I2 => (i128::from(i16::MIN), i128::from(i16::MAX)),
        vt::I4 => (i128::from(i32::MIN), i128::from(i32::MAX)),
        _ => (i128::from(i64::MIN), i128::from(i64::MAX)),
    }
}
/// VarDiv: a double (NULL propagates; division by zero DISP_E_DIVBYZERO).
pub fn divide(a: &Value, b: &Value) -> Result<Value, u32> {
    if matches!(a, Value::Null) || matches!(b, Value::Null) {
        return Ok(Value::Null);
    }
    let x = real_of(&num_of(a)?)?;
    let y = real_of(&num_of(b)?)?;
    if y == 0.0 {
        return Err(hr::DISP_E_DIVBYZERO);
    }
    Ok(Value::R8(x / y))
}
/// VarIdiv / VarMod (`modulo`): both operands rounded to integers; the
/// result in the wider of their integer types (I4 for reals and strings).
pub fn integer_divide(a: &Value, b: &Value, modulo: bool) -> Result<Value, u32> {
    if matches!(a, Value::Null) || matches!(b, Value::Null) {
        return Ok(Value::Null);
    }
    let x = integer_of(&num_of(a)?)?;
    let y = integer_of(&num_of(b)?)?;
    if y == 0 {
        return Err(hr::DISP_E_DIVBYZERO);
    }
    let v = if modulo { x % y } else { x / y };
    let t = match rank(a).max(rank(b)) {
        1 => vt::UI1,
        2 => vt::I2,
        4 => vt::I8,
        _ => vt::I4,
    };
    let (lo, hi) = range(t);
    if v < lo || v > hi {
        return Err(hr::DISP_E_OVERFLOW);
    }
    Ok(make_integer(t, v))
}
/// VarCat: both as text (NULL as empty; BOOL as "True"/"False").
pub fn concatenate(a: &Value, b: &Value) -> Result<Value, u32> {
    let piece = |v: &Value| -> Result<String, u32> {
        match v {
            Value::Null | Value::Empty => Ok(String::new()),
            Value::Bool(b) => Ok(bool_text(*b, flags::ALPHABOOL)),
            other => text_of(other, 0),
        }
    };
    let mut s = piece(a)?;
    s.push_str(&piece(b)?);
    Ok(Value::Bstr(s.encode_utf16().collect()))
}
/// VARCMP_LT 0, EQ 1, GT 2, NULL 3.
pub fn compare(a: &Value, b: &Value) -> Result<u32, u32> {
    if matches!(a, Value::Null) || matches!(b, Value::Null) {
        return Ok(3);
    }
    let ord = match (a, b) {
        (Value::Bstr(x), Value::Bstr(y)) => x.cmp(y),
        // A string against a number: the number is less.
        (Value::Bstr(_), _) => core::cmp::Ordering::Greater,
        (_, Value::Bstr(_)) => core::cmp::Ordering::Less,
        _ => {
            let x = real_of(&num_of(a)?)?;
            let y = real_of(&num_of(b)?)?;
            x.partial_cmp(&y).unwrap_or(core::cmp::Ordering::Equal)
        }
    };
    Ok(match ord {
        core::cmp::Ordering::Less => 0,
        core::cmp::Ordering::Equal => 1,
        core::cmp::Ordering::Greater => 2,
    })
}
