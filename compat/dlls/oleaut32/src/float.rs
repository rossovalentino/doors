//! The f64 operations `core` lacks (no libm in the DLL), exact for the
//! ranges oleaut32 uses them on.

/// Rounded to an integer, half to even (|v| < 2^126).
pub fn round_even(v: f64) -> i128 {
    let t = v as i128;
    let rest = v - t as f64;
    let half = rest.abs();
    if half > 0.5 || (half == 0.5 && t % 2 != 0) {
        if v < 0.0 { t - 1 } else { t + 1 }
    } else {
        t
    }
}
/// Toward zero (|v| < 2^63).
pub fn trunc(v: f64) -> f64 {
    (v as i64) as f64
}
/// The fraction with the sign of `v`.
pub fn fract(v: f64) -> f64 {
    v - trunc(v)
}
/// `m / 10^scale`, correctly rounded (through the decimal text).
pub fn scaled(m: u128, scale: u8) -> f64 {
    alloc::format!("{m}e-{scale}").parse().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn helpers() {
        assert_eq!(round_even(2.5), 2);
        assert_eq!(round_even(3.5), 4);
        assert_eq!(round_even(-2.5), -2);
        assert_eq!(round_even(-0.5), 0);
        assert_eq!(round_even(1234567.891), 1234568);
        assert_eq!(trunc(-1.25), -1.0);
        assert_eq!(fract(-1.25), -0.25);
        assert_eq!(scaled(1234567891, 3), 1234567.891);
    }
}
