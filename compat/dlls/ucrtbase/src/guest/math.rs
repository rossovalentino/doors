//! The math set (api-ms-win-crt-math) over `libm` (correctly rounded where
//! libm is, as the UCRT's), plus the CRT's classification helpers.
//! Domain and range errors set errno as the UCRT does.
use super::{errno, set_errno};

macro_rules! unary {
    ($($name:ident: $t:ty => $f:path;)*) => {$(
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(x: $t) -> $t {
            $f(x)
        }
    )*};
}
macro_rules! binary {
    ($($name:ident: $t:ty => $f:path;)*) => {$(
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(x: $t, y: $t) -> $t {
            $f(x, y)
        }
    )*};
}
/// Functions whose argument may be outside their domain (NaN results set
/// EDOM) or overflow (infinite results from finite input set ERANGE).
macro_rules! checked {
    ($($name:ident: $t:ty => $f:path;)*) => {$(
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(x: $t) -> $t {
            let r = $f(x);
            if r.is_nan() && !x.is_nan() {
                set_errno(33); // EDOM
            } else if r.is_infinite() && x.is_finite() {
                set_errno(errno::ERANGE);
            }
            r
        }
    )*};
}
checked! {
    acos: f64 => libm::acos;
    acosf: f32 => libm::acosf;
    asin: f64 => libm::asin;
    asinf: f32 => libm::asinf;
    acosh: f64 => libm::acosh;
    acoshf: f32 => libm::acoshf;
    atanh: f64 => libm::atanh;
    atanhf: f32 => libm::atanhf;
    exp: f64 => libm::exp;
    expf: f32 => libm::expf;
    exp2: f64 => libm::exp2;
    exp2f: f32 => libm::exp2f;
    expm1: f64 => libm::expm1;
    expm1f: f32 => libm::expm1f;
    log: f64 => libm::log;
    logf: f32 => libm::logf;
    log10: f64 => libm::log10;
    log10f: f32 => libm::log10f;
    log2: f64 => libm::log2;
    log2f: f32 => libm::log2f;
    log1p: f64 => libm::log1p;
    log1pf: f32 => libm::log1pf;
    sqrt: f64 => libm::sqrt;
    sqrtf: f32 => libm::sqrtf;
    cosh: f64 => libm::cosh;
    coshf: f32 => libm::coshf;
    sinh: f64 => libm::sinh;
    sinhf: f32 => libm::sinhf;
    tgamma: f64 => libm::tgamma;
    tgammaf: f32 => libm::tgammaf;
    lgamma: f64 => libm::lgamma;
    lgammaf: f32 => libm::lgammaf;
}
unary! {
    atan: f64 => libm::atan;
    atanf: f32 => libm::atanf;
    asinh: f64 => libm::asinh;
    asinhf: f32 => libm::asinhf;
    cbrt: f64 => libm::cbrt;
    cbrtf: f32 => libm::cbrtf;
    ceil: f64 => libm::ceil;
    ceilf: f32 => libm::ceilf;
    cos: f64 => libm::cos;
    cosf: f32 => libm::cosf;
    erf: f64 => libm::erf;
    erff: f32 => libm::erff;
    erfc: f64 => libm::erfc;
    erfcf: f32 => libm::erfcf;
    fabs: f64 => libm::fabs;
    fabsf: f32 => libm::fabsf;
    floor: f64 => libm::floor;
    floorf: f32 => libm::floorf;


    nearbyint: f64 => libm::rint;
    nearbyintf: f32 => libm::rintf;
    rint: f64 => libm::rint;
    rintf: f32 => libm::rintf;
    round: f64 => libm::round;
    roundf: f32 => libm::roundf;
    sin: f64 => libm::sin;
    sinf: f32 => libm::sinf;
    tan: f64 => libm::tan;
    tanf: f32 => libm::tanf;
    tanh: f64 => libm::tanh;
    tanhf: f32 => libm::tanhf;
    trunc: f64 => libm::trunc;
    truncf: f32 => libm::truncf;
}
binary! {
    atan2: f64 => libm::atan2;
    atan2f: f32 => libm::atan2f;
    copysign: f64 => libm::copysign;
    copysignf: f32 => libm::copysignf;
    _copysign: f64 => libm::copysign;
    _copysignf: f32 => libm::copysignf;
    fdim: f64 => libm::fdim;
    fdimf: f32 => libm::fdimf;
    fmax: f64 => libm::fmax;
    fmaxf: f32 => libm::fmaxf;
    fmin: f64 => libm::fmin;
    fminf: f32 => libm::fminf;
    fmod: f64 => libm::fmod;
    fmodf: f32 => libm::fmodf;
    hypot: f64 => libm::hypot;
    hypotf: f32 => libm::hypotf;
    _hypot: f64 => libm::hypot;
    _hypotf: f32 => libm::hypotf;
    nextafter: f64 => libm::nextafter;
    nextafterf: f32 => libm::nextafterf;
    _nextafter: f64 => libm::nextafter;
    remainder: f64 => libm::remainder;
    remainderf: f32 => libm::remainderf;
}
#[unsafe(no_mangle)]
pub extern "C" fn pow(x: f64, y: f64) -> f64 {
    let r = libm::pow(x, y);
    if r.is_nan() && !x.is_nan() && !y.is_nan() {
        set_errno(33);
    } else if r.is_infinite() && x.is_finite() && y.is_finite() {
        set_errno(errno::ERANGE);
    }
    r
}
#[unsafe(no_mangle)]
pub extern "C" fn powf(x: f32, y: f32) -> f32 {
    libm::powf(x, y)
}
#[unsafe(no_mangle)]
pub extern "C" fn fma(x: f64, y: f64, z: f64) -> f64 {
    libm::fma(x, y, z)
}
#[unsafe(no_mangle)]
pub extern "C" fn fmaf(x: f32, y: f32, z: f32) -> f32 {
    libm::fmaf(x, y, z)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn modf(x: f64, integer: *mut f64) -> f64 {
    let (fraction, whole) = libm::modf(x);
    if !integer.is_null() {
        unsafe { *integer = whole };
    }
    fraction
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn modff(x: f32, integer: *mut f32) -> f32 {
    let (fraction, whole) = libm::modff(x);
    if !integer.is_null() {
        unsafe { *integer = whole };
    }
    fraction
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn frexp(x: f64, exponent: *mut i32) -> f64 {
    let (m, e) = libm::frexp(x);
    if !exponent.is_null() {
        unsafe { *exponent = e };
    }
    m
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn frexpf(x: f32, exponent: *mut i32) -> f32 {
    let (m, e) = libm::frexpf(x);
    if !exponent.is_null() {
        unsafe { *exponent = e };
    }
    m
}
#[unsafe(no_mangle)]
pub extern "C" fn ldexp(x: f64, n: i32) -> f64 {
    libm::ldexp(x, n)
}
#[unsafe(no_mangle)]
pub extern "C" fn ldexpf(x: f32, n: i32) -> f32 {
    libm::ldexpf(x, n)
}
#[unsafe(no_mangle)]
pub extern "C" fn scalbn(x: f64, n: i32) -> f64 {
    libm::scalbn(x, n)
}
#[unsafe(no_mangle)]
pub extern "C" fn scalbnf(x: f32, n: i32) -> f32 {
    libm::scalbnf(x, n)
}
#[unsafe(no_mangle)]
pub extern "C" fn ilogb(x: f64) -> i32 {
    libm::ilogb(x)
}
#[unsafe(no_mangle)]
pub extern "C" fn ilogbf(x: f32) -> i32 {
    libm::ilogbf(x)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn remquo(x: f64, y: f64, quotient: *mut i32) -> f64 {
    let (r, q) = libm::remquo(x, y);
    if !quotient.is_null() {
        unsafe { *quotient = q };
    }
    r
}
#[unsafe(no_mangle)]
pub extern "C" fn lround(x: f64) -> i32 {
    libm::round(x) as i32
}
#[unsafe(no_mangle)]
pub extern "C" fn llround(x: f64) -> i64 {
    libm::round(x) as i64
}
#[unsafe(no_mangle)]
pub extern "C" fn lrint(x: f64) -> i32 {
    libm::rint(x) as i32
}
#[unsafe(no_mangle)]
pub extern "C" fn llrint(x: f64) -> i64 {
    libm::rint(x) as i64
}
#[unsafe(no_mangle)]
pub extern "C" fn nan(_tag: *const u8) -> f64 {
    f64::NAN
}
#[unsafe(no_mangle)]
pub extern "C" fn nanf(_tag: *const u8) -> f32 {
    f32::NAN
}
// Classification (`_fdclass`, `_dclass`, `_dsign`...): FP_NAN 2,
// FP_INFINITE 1, FP_ZERO 0, FP_SUBNORMAL -2, FP_NORMAL -1.
fn class64(x: f64) -> i16 {
    match x.classify() {
        core::num::FpCategory::Nan => 2,
        core::num::FpCategory::Infinite => 1,
        core::num::FpCategory::Zero => 0,
        core::num::FpCategory::Subnormal => -2,
        core::num::FpCategory::Normal => -1,
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn _dclass(x: f64) -> i16 {
    class64(x)
}
#[unsafe(no_mangle)]
pub extern "C" fn _fdclass(x: f32) -> i16 {
    class64(f64::from(x))
}
#[unsafe(no_mangle)]
pub extern "C" fn _ldclass(x: f64) -> i16 {
    class64(x)
}
#[unsafe(no_mangle)]
pub extern "C" fn _dsign(x: f64) -> i32 {
    i32::from(x.is_sign_negative()) << 15
}
#[unsafe(no_mangle)]
pub extern "C" fn _fdsign(x: f32) -> i32 {
    i32::from(x.is_sign_negative()) << 15
}
#[unsafe(no_mangle)]
pub extern "C" fn _ldsign(x: f64) -> i32 {
    _dsign(x)
}
#[unsafe(no_mangle)]
pub extern "C" fn _dtest(x: *const f64) -> i16 {
    class64(unsafe { *x })
}
#[unsafe(no_mangle)]
pub extern "C" fn _fdtest(x: *const f32) -> i16 {
    class64(f64::from(unsafe { *x }))
}
#[unsafe(no_mangle)]
pub extern "C" fn _isnan(x: f64) -> i32 {
    i32::from(x.is_nan())
}
#[unsafe(no_mangle)]
pub extern "C" fn _isnanf(x: f32) -> i32 {
    i32::from(x.is_nan())
}
#[unsafe(no_mangle)]
pub extern "C" fn _finite(x: f64) -> i32 {
    i32::from(x.is_finite())
}
#[unsafe(no_mangle)]
pub extern "C" fn _finitef(x: f32) -> i32 {
    i32::from(x.is_finite())
}
/// `_fpclass`: the _FPCLASS_* bits.
#[unsafe(no_mangle)]
pub extern "C" fn _fpclass(x: f64) -> i32 {
    let negative = x.is_sign_negative();
    match x.classify() {
        core::num::FpCategory::Nan => {
            if x.to_bits() & (1 << 51) != 0 {
                0x2
            } else {
                0x1
            }
        }
        core::num::FpCategory::Infinite => {
            if negative {
                0x4
            } else {
                0x200
            }
        }
        core::num::FpCategory::Zero => {
            if negative {
                0x20
            } else {
                0x40
            }
        }
        core::num::FpCategory::Subnormal => {
            if negative {
                0x10
            } else {
                0x80
            }
        }
        core::num::FpCategory::Normal => {
            if negative {
                0x8
            } else {
                0x100
            }
        }
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn _chgsign(x: f64) -> f64 {
    -x
}
#[unsafe(no_mangle)]
pub extern "C" fn _logb(x: f64) -> f64 {
    logb(x)
}
#[unsafe(no_mangle)]
pub extern "C" fn _scalb(x: f64, n: i32) -> f64 {
    libm::scalbn(x, n)
}
/// The FPU environment calls (`fegetround`, `fesetround`...): rounding
/// through the control word.
#[unsafe(no_mangle)]
pub extern "C" fn fegetround() -> i32 {
    match super::runtime::_controlfp(0, 0) & 0x300 {
        0x100 => 0x100, // FE_DOWNWARD
        0x200 => 0x200, // FE_UPWARD
        0x300 => 0x300, // FE_TOWARDZERO
        _ => 0,
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn fesetround(mode: i32) -> i32 {
    if mode & !0x300 != 0 {
        return 1;
    }
    super::runtime::_controlfp(mode as u32, 0x300);
    0
}
/// logb: the unbiased exponent as a float (-inf for 0, +inf for infinities).
#[unsafe(no_mangle)]
pub extern "C" fn logb(x: f64) -> f64 {
    if x == 0.0 {
        set_errno(33);
        return f64::NEG_INFINITY;
    }
    if x.is_nan() {
        return x;
    }
    if x.is_infinite() {
        return f64::INFINITY;
    }
    f64::from(libm::ilogb(x))
}
#[unsafe(no_mangle)]
pub extern "C" fn logbf(x: f32) -> f32 {
    logb(f64::from(x)) as f32
}
