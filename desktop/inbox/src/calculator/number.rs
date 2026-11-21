//! Calculator numbers: decimal floating point with an 18-digit mantissa, so
//! sums of typed decimals are exact (0.1 + 0.2 = 0.3) and every operation
//! rounds once, past the 16 digits Standard mode shows (2 ÷ 3 =
//! 0.6666666666666667, as Windows' Calculator rounds its wider rationals).
use alloc::string::String;
use core::cmp::Ordering;

/// Mantissa digits kept after every operation.
pub const DIGITS: u32 = 18;
const LIMIT: u64 = 1_000_000_000_000_000_000; // 10^18
/// Digits Standard mode shows.
pub const SHOWN: usize = 16;
/// Magnitudes Windows treats as overflow ("Overflow").
const MAX_EXP: i32 = 9999;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    DivideByZero,
    Invalid,
    Overflow,
}

/// `(-1)^neg * mant * 10^exp`, mantissa below 10^18; zero is `mant == 0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Number {
    neg: bool,
    mant: u64,
    exp: i32,
}

fn pow10(n: u32) -> u128 {
    10u128.pow(n)
}
/// Decimal digits of `v` (at least 1).
fn digits(v: u128) -> u32 {
    let mut n = 1;
    let mut t = v;
    while t >= 10 {
        t /= 10;
        n += 1;
    }
    n
}

impl Number {
    pub const ZERO: Self = Self {
        neg: false,
        mant: 0,
        exp: 0,
    };
    pub fn int(v: i64) -> Self {
        Self::from_parts(v < 0, u128::from(v.unsigned_abs()), 0).unwrap()
    }
    /// Normalizes `v * 10^exp` to at most 18 digits, rounding half away
    /// from zero.
    fn from_parts(neg: bool, mut v: u128, mut exp: i32) -> Result<Self, Fault> {
        if v == 0 {
            return Ok(Self::ZERO);
        }
        let n = digits(v);
        if n > DIGITS {
            let cut = n - DIGITS;
            let p = pow10(cut);
            let rest = v % p;
            v /= p;
            exp += cut as i32;
            let half = p / 2;
            if rest >= half {
                v += 1;
                if v == u128::from(LIMIT) {
                    v /= 10;
                    exp += 1;
                }
            }
        }
        // Trailing zeros carry no information.
        while v.is_multiple_of(10) {
            v /= 10;
            exp += 1;
        }
        let top = exp + digits(v) as i32 - 1;
        if top > MAX_EXP {
            return Err(Fault::Overflow);
        }
        if top < -MAX_EXP {
            return Ok(Self::ZERO);
        }
        Ok(Self {
            neg,
            mant: v as u64,
            exp,
        })
    }
    pub fn is_zero(self) -> bool {
        self.mant == 0
    }
    pub fn negative(self) -> bool {
        self.neg && !self.is_zero()
    }
    pub fn negated(self) -> Self {
        if self.is_zero() {
            self
        } else {
            Self {
                neg: !self.neg,
                ..self
            }
        }
    }
    pub fn abs(self) -> Self {
        Self { neg: false, ..self }
    }
    /// Parses what the user typed: digits, at most one point, optional
    /// leading '-' (no exponent in Standard mode).
    pub fn parse(text: &str) -> Option<Self> {
        let (neg, body) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        if body.is_empty() || body.bytes().filter(|b| *b == b'.').count() > 1 {
            return None;
        }
        let mut v: u128 = 0;
        let mut exp = 0i32;
        let mut point = false;
        let mut kept = 0;
        for b in body.bytes() {
            match b {
                b'.' => point = true,
                b'0'..=b'9' => {
                    if kept < 36 {
                        v = v * 10 + u128::from(b - b'0');
                        if v != 0 {
                            kept += 1;
                        }
                        if point {
                            exp -= 1;
                        }
                    } else if !point {
                        exp += 1;
                    }
                }
                _ => return None,
            }
        }
        Self::from_parts(neg, v, exp).ok()
    }
    fn cmp_abs(a: Self, b: Self) -> Ordering {
        if a.is_zero() || b.is_zero() {
            return a.mant.cmp(&b.mant);
        }
        let ta = a.exp + digits(u128::from(a.mant)) as i32;
        let tb = b.exp + digits(u128::from(b.mant)) as i32;
        if ta != tb {
            return ta.cmp(&tb);
        }
        // Same magnitude: compare at a common exponent.
        let e = a.exp.min(b.exp);
        let va = u128::from(a.mant) * pow10((a.exp - e) as u32);
        let vb = u128::from(b.mant) * pow10((b.exp - e) as u32);
        va.cmp(&vb)
    }
    pub fn try_add(self, o: Self) -> Result<Self, Fault> {
        if self.is_zero() {
            return Ok(o);
        }
        if o.is_zero() {
            return Ok(self);
        }
        // Align both on a common exponent no more than 19 digits below the
        // larger term (at most 37 digits); digits of the smaller term below
        // that only round the result.
        let (big, small) = if Self::cmp_abs(self, o) == Ordering::Less {
            (o, self)
        } else {
            (self, o)
        };
        let exp = big.exp.min(small.exp).max(big.exp - 19);
        let vb = u128::from(big.mant) * pow10((big.exp - exp) as u32);
        let vs = if small.exp >= exp {
            u128::from(small.mant) * pow10((small.exp - exp) as u32)
        } else {
            let cut = (exp - small.exp) as u32;
            if cut > 36 {
                0
            } else {
                u128::from(small.mant) / pow10(cut)
            }
        };
        let v = if big.neg == small.neg {
            vb + vs
        } else {
            vb - vs
        };
        Self::from_parts(big.neg, v, exp)
    }
    pub fn try_sub(self, o: Self) -> Result<Self, Fault> {
        self.try_add(o.negated())
    }
    pub fn try_mul(self, o: Self) -> Result<Self, Fault> {
        let v = u128::from(self.mant) * u128::from(o.mant);
        Self::from_parts(self.neg != o.neg, v, self.exp + o.exp)
    }
    pub fn try_div(self, o: Self) -> Result<Self, Fault> {
        if o.is_zero() {
            return Err(if self.is_zero() {
                Fault::Invalid
            } else {
                Fault::DivideByZero
            });
        }
        // 20 quotient digits: scale the dividend to 38 digits.
        let a = u128::from(self.mant);
        let scale = 38 - digits(a);
        let n = a * pow10(scale);
        let q = n / u128::from(o.mant);
        Self::from_parts(self.neg != o.neg, q, self.exp - o.exp - scale as i32)
    }
    pub fn recip(self) -> Result<Self, Fault> {
        Self::int(1).try_div(self)
    }
    pub fn sqr(self) -> Result<Self, Fault> {
        self.try_mul(self)
    }
    /// Square root by Newton's method on the decimal value.
    pub fn sqrt(self) -> Result<Self, Fault> {
        if self.negative() {
            return Err(Fault::Invalid);
        }
        if self.is_zero() {
            return Ok(self);
        }
        // Start from the root of the leading digits and the halved exponent.
        let total = self.exp + digits(u128::from(self.mant)) as i32;
        let mut x = Self {
            neg: false,
            mant: 1,
            exp: total / 2,
        };
        let half = Self {
            neg: false,
            mant: 5,
            exp: -1,
        };
        for _ in 0..200 {
            let next = x.try_add(self.try_div(x)?)?.try_mul(half)?;
            if next == x {
                break;
            }
            x = next;
        }
        Ok(x)
    }
    /// The value rounded to `n` significant digits.
    fn rounded(self, n: u32) -> Self {
        if self.is_zero() || digits(u128::from(self.mant)) <= n {
            return self;
        }
        let d = digits(u128::from(self.mant));
        let cut = d - n;
        let p = pow10(cut);
        let mut v = u128::from(self.mant) / p;
        let rest = u128::from(self.mant) % p;
        let mut exp = self.exp + cut as i32;
        if rest * 2 >= p {
            v += 1;
            if digits(v) > n {
                v /= 10;
                exp += 1;
            }
        }
        while v.is_multiple_of(10) {
            v /= 10;
            exp += 1;
        }
        Self {
            neg: self.neg,
            mant: v as u64,
            exp,
        }
    }
    /// Standard-mode display text: 16 significant digits, thousands
    /// separators, scientific notation ("1.5e+20", "1e-17") when the fixed
    /// form would need more than 16 digits.
    pub fn display(self, grouping: bool) -> String {
        let r = self.rounded(SHOWN as u32);
        if r.is_zero() {
            return String::from("0");
        }
        let mut s = String::new();
        if r.neg {
            s.push('-');
        }
        let mut ds = String::new();
        let mut t = r.mant;
        while t > 0 {
            ds.insert(0, char::from(b'0' + (t % 10) as u8));
            t /= 10;
        }
        let n = ds.len() as i32;
        // Position of the decimal point relative to the digit string.
        let point = n + r.exp;
        let fixed_digits = if point > 0 { point.max(n) } else { n - point };
        if point > SHOWN as i32 || fixed_digits > SHOWN as i32 && point <= 0 {
            // Scientific: d.ddd e±x.
            s.push_str(&ds[..1]);
            if n > 1 {
                s.push('.');
                s.push_str(&ds[1..]);
            }
            let e = point - 1;
            s.push('e');
            s.push(if e < 0 { '-' } else { '+' });
            s.push_str(&alloc::format!("{}", e.unsigned_abs()));
            return s;
        }
        let (int, frac) = if point <= 0 {
            let mut f = String::new();
            for _ in 0..-point {
                f.push('0');
            }
            f.push_str(&ds);
            (String::from("0"), f)
        } else if point >= n {
            let mut i = ds.clone();
            for _ in 0..point - n {
                i.push('0');
            }
            (i, String::new())
        } else {
            (
                String::from(&ds[..point as usize]),
                String::from(&ds[point as usize..]),
            )
        };
        s.push_str(&if grouping { group(&int) } else { int });
        if !frac.is_empty() {
            s.push('.');
            s.push_str(&frac);
        }
        s
    }
    /// Plain text for history and expressions (no separators).
    pub fn plain(self) -> String {
        self.display(false)
    }
}
/// "1234567" -> "1,234,567".
pub fn group(int: &str) -> String {
    let mut out = String::new();
    let len = int.len();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn n(s: &str) -> Number {
        Number::parse(s).unwrap()
    }
    #[test]
    fn decimal_sums_are_exact_and_division_rounds_like_windows() {
        assert_eq!(n("0.1").try_add(n("0.2")).unwrap().display(true), "0.3");
        assert_eq!(
            n("2").try_div(n("3")).unwrap().display(true),
            "0.6666666666666667"
        );
        assert_eq!(
            n("1").try_div(n("3")).unwrap().display(true),
            "0.3333333333333333"
        );
        assert_eq!(n("10").try_div(n("4")).unwrap().display(true), "2.5");
        assert_eq!(
            n("1234567").try_mul(n("1000")).unwrap().display(true),
            "1,234,567,000"
        );
        assert_eq!(n("2").sqrt().unwrap().display(true), "1.414213562373095");
        assert_eq!(n("9").sqrt().unwrap().display(true), "3");
        assert_eq!(n("5").try_sub(n("7.5")).unwrap().display(true), "-2.5");
        assert_eq!(n("0").try_div(n("0")), Err(Fault::Invalid));
        assert_eq!(n("1").try_div(n("0")), Err(Fault::DivideByZero));
    }
    #[test]
    fn large_and_tiny_values_switch_to_scientific() {
        assert_eq!(n("9999999999999999").display(true), "9,999,999,999,999,999");
        assert_eq!(
            n("9999999999999999").try_add(n("1")).unwrap().display(true),
            "1e+16"
        );
        assert_eq!(n("0.001").display(true), "0.001");
        assert_eq!(
            n("1")
                .try_div(n("100000000000000000"))
                .unwrap()
                .display(true),
            "1e-17"
        );
        assert_eq!(
            n("3").try_div(n("7")).unwrap().display(false),
            "0.4285714285714286"
        );
        assert_eq!(n("123.4500").display(true), "123.45");
    }
}
