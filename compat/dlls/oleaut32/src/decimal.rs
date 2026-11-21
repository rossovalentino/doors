//! DECIMAL: a 96-bit unsigned integer, a power-of-ten scale 0..=28 and a
//! sign (wtypes.h), as VarDec* convert and VarBstrFromDec writes it.
use crate::{hr, number::Parsed};

/// The largest 96-bit value.
pub const MAX: u128 = (1 << 96) - 1;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Decimal {
    pub negative: bool,
    pub scale: u8,
    pub mantissa: u128,
}
impl Decimal {
    /// The 16-byte DECIMAL (wReserved left 0; a VARIANT puts its vt there).
    pub fn encode(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[2] = self.scale;
        b[3] = if self.negative { 0x80 } else { 0 };
        b[4..8].copy_from_slice(&((self.mantissa >> 64) as u32).to_le_bytes());
        b[8..16].copy_from_slice(&(self.mantissa as u64).to_le_bytes());
        b
    }
    pub fn decode(b: &[u8; 16]) -> Result<Self, u32> {
        if b[2] > 28 || b[3] & 0x7f != 0 {
            return Err(hr::E_INVALIDARG);
        }
        let hi = u128::from(u32::from_le_bytes(b[4..8].try_into().unwrap()));
        let lo = u128::from(u64::from_le_bytes(b[8..16].try_into().unwrap()));
        Ok(Self {
            negative: b[3] & 0x80 != 0,
            scale: b[2],
            mantissa: (hi << 64) | lo,
        })
    }
    pub fn from_i128(v: i128) -> Result<Self, u32> {
        let m = v.unsigned_abs();
        if m > MAX {
            return Err(hr::DISP_E_OVERFLOW);
        }
        Ok(Self {
            negative: v < 0,
            scale: 0,
            mantissa: m,
        })
    }
    /// A parsed number: at most 29 significant digits and scale 28, rounded
    /// half to even beyond that; DISP_E_OVERFLOW above 96 bits.
    pub fn from_parsed(p: &Parsed) -> Result<Self, u32> {
        let mut digits = p.digits.clone();
        let mut exponent = p.exponent;
        // Scale above 28: drop digits from the right (rounding).
        let mut round_tail: Option<(u8, bool)> = None;
        while exponent < -28 && !digits.is_empty() {
            let last = digits.pop().unwrap();
            let rest_nonzero = round_tail.is_some_and(|(d, r)| d != 0 || r);
            round_tail = Some((last, rest_nonzero));
            exponent += 1;
        }
        if digits.is_empty() {
            // Zero keeps its sign ("-0" is a negative zero).
            return Ok(Self {
                negative: p.negative,
                ..Self::default()
            });
        }
        let mut mantissa: u128 = 0;
        for d in &digits {
            mantissa = mantissa
                .checked_mul(10)
                .and_then(|m| m.checked_add(u128::from(*d)))
                .ok_or(hr::DISP_E_OVERFLOW)?;
        }
        if let Some((d, rest)) = round_tail
            && (d > 5 || (d == 5 && (rest || mantissa % 2 == 1)))
        {
            mantissa += 1;
        }
        let mut scale = 0u8;
        if exponent > 0 {
            for _ in 0..exponent {
                mantissa = mantissa.checked_mul(10).ok_or(hr::DISP_E_OVERFLOW)?;
            }
        } else {
            scale = (-exponent) as u8;
        }
        // Too many digits: reduce the scale, rounding, until 96 bits fit.
        while mantissa > MAX {
            if scale == 0 {
                return Err(hr::DISP_E_OVERFLOW);
            }
            mantissa = round_div10(mantissa);
            scale -= 1;
        }
        Ok(Self {
            negative: p.negative,
            scale,
            mantissa,
        })
    }
    /// VarDecFromR8 / VarDecFromR4: the value's 15 (7) significant digits.
    pub fn from_real(v: f64, precision: usize) -> Result<Self, u32> {
        if !v.is_finite() {
            return Err(hr::DISP_E_OVERFLOW);
        }
        let s = alloc::format!("{:.*e}", precision - 1, v);
        let text: alloc::vec::Vec<u16> = s.encode_utf16().collect();
        let p = crate::number::parse(&text)?;
        Self::from_parsed(&p)
    }
    pub fn to_f64(&self) -> f64 {
        let v = crate::float::scaled(self.mantissa, self.scale);
        if self.negative { -v } else { v }
    }
    /// Rounded to an integer, half to even.
    pub fn to_integer(&self) -> i128 {
        let mut m = self.mantissa;
        let mut scale = self.scale;
        let mut tail: Option<(u128, bool)> = None;
        while scale > 0 {
            let digit = m % 10;
            let rest = tail.is_some_and(|(d, r)| d != 0 || r);
            tail = Some((digit, rest));
            m /= 10;
            scale -= 1;
        }
        if let Some((d, rest)) = tail
            && (d > 5 || (d == 5 && (rest || m % 2 == 1)))
        {
            m += 1;
        }
        let v = m as i128;
        if self.negative { -v } else { v }
    }
    /// Rescaled to `scale` (rounding half to even when digits go).
    pub fn rescale(&self, scale: u8) -> Option<u128> {
        let mut m = self.mantissa;
        let mut s = self.scale;
        while s < scale {
            m = m.checked_mul(10)?;
            s += 1;
        }
        let mut tail: Option<(u128, bool)> = None;
        while s > scale {
            let digit = m % 10;
            let rest = tail.is_some_and(|(d, r)| d != 0 || r);
            tail = Some((digit, rest));
            m /= 10;
            s -= 1;
        }
        if let Some((d, rest)) = tail
            && (d > 5 || (d == 5 && (rest || m % 2 == 1)))
        {
            m += 1;
        }
        Some(m)
    }
    pub fn text(&self) -> alloc::string::String {
        crate::number::format_scaled(self.negative, self.mantissa, u32::from(self.scale))
    }
}
fn round_div10(m: u128) -> u128 {
    let q = m / 10;
    let r = m % 10;
    if r > 5 || (r == 5 && q % 2 == 1) {
        q + 1
    } else {
        q
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn w(s: &str) -> alloc::vec::Vec<u16> {
        s.encode_utf16().collect()
    }
    fn hex(b: &[u8]) -> alloc::string::String {
        b.iter().map(|v| alloc::format!("{v:02x}")).collect()
    }
    #[test]
    fn decimals_match_windows_layout_and_text() {
        let d = Decimal::from_parsed(&crate::number::parse(&w("1234567.891")).unwrap()).unwrap();
        assert_eq!(hex(&d.encode()), "0000030000000000d302964900000000");
        assert_eq!(d.text(), "1234567.891");
        let max = Decimal::from_parsed(
            &crate::number::parse(&w("79228162514264337593543950335")).unwrap(),
        )
        .unwrap();
        assert_eq!(max.mantissa, MAX);
        assert_eq!(
            Decimal::from_parsed(
                &crate::number::parse(&w("79228162514264337593543950336")).unwrap()
            ),
            Err(hr::DISP_E_OVERFLOW)
        );
        let neg = Decimal::from_parsed(&crate::number::parse(&w("-1.05")).unwrap()).unwrap();
        assert_eq!(hex(&neg.encode()), "00000280000000006900000000000000");
        assert_eq!(neg.to_integer(), -1);
        assert_eq!(neg.rescale(4), Some(10500));
        let third = Decimal::from_real(1.0 / 3.0, 15).unwrap();
        assert_eq!(third.text(), "0.333333333333333");
        assert_eq!(
            Decimal::from_real(1e20, 15).unwrap().text(),
            "100000000000000000000"
        );
    }
}
