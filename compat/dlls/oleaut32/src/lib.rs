#![no_std]
//! Original oleaut32.dll: OLE Automation. BSTR, VARIANT and
//! VariantChangeType's conversions, Var* arithmetic, dates, SAFEARRAY,
//! IDispatch helpers, type libraries and error info.
//!
//! This module is the host-testable part: number and date text for en-US
//! (LCID 0x409, Doors' sessions), DECIMAL and CY, and the conversion rules,
//! all measured against Windows 11 (`tests/reference`).
//!
//! The same sources build the x64 and the x86 DLL (P17): Windows-facing
//! exports take pointer-width parameters, widened at entry; a pointer or
//! handle result is narrowed from the body's 64-bit value (an immediately
//! called closure, so the body's own `return`s keep their meaning).
#![allow(clippy::redundant_closure_call)]
extern crate alloc;

pub mod date;
pub mod decimal;
mod float;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
pub mod number;
pub mod typelib;
pub mod variant;

/// HRESULTs (winerror.h).
pub mod hr {
    pub const S_OK: u32 = 0;
    pub const S_FALSE: u32 = 1;
    pub const E_NOTIMPL: u32 = 0x8000_4001;
    pub const E_NOINTERFACE: u32 = 0x8000_4002;
    pub const E_POINTER: u32 = 0x8000_4003;
    pub const E_FAIL: u32 = 0x8000_4005;
    pub const E_UNEXPECTED: u32 = 0x8000_FFFF;
    pub const E_OUTOFMEMORY: u32 = 0x8007_000E;
    pub const E_INVALIDARG: u32 = 0x8007_0057;
    pub const DISP_E_UNKNOWNINTERFACE: u32 = 0x8002_0001;
    pub const DISP_E_MEMBERNOTFOUND: u32 = 0x8002_0003;
    pub const DISP_E_PARAMNOTFOUND: u32 = 0x8002_0004;
    pub const DISP_E_TYPEMISMATCH: u32 = 0x8002_0005;
    pub const DISP_E_UNKNOWNNAME: u32 = 0x8002_0006;
    pub const DISP_E_NONAMEDARGS: u32 = 0x8002_0007;
    pub const DISP_E_BADVARTYPE: u32 = 0x8002_0008;
    pub const DISP_E_EXCEPTION: u32 = 0x8002_0009;
    pub const DISP_E_OVERFLOW: u32 = 0x8002_000A;
    pub const DISP_E_BADINDEX: u32 = 0x8002_000B;
    pub const DISP_E_UNKNOWNLCID: u32 = 0x8002_000C;
    pub const DISP_E_ARRAYISLOCKED: u32 = 0x8002_000D;
    pub const DISP_E_BADPARAMCOUNT: u32 = 0x8002_000E;
    pub const DISP_E_PARAMNOTOPTIONAL: u32 = 0x8002_000F;
    pub const DISP_E_NOTACOLLECTION: u32 = 0x8002_0011;
    pub const DISP_E_DIVBYZERO: u32 = 0x8002_0012;
    pub const TYPE_E_ELEMENTNOTFOUND: u32 = 0x8002_802B;
    pub const TYPE_E_LIBNOTREGISTERED: u32 = 0x8002_801D;
    pub const TYPE_E_CANTLOADLIBRARY: u32 = 0x8002_9C4A;
    pub const TYPE_E_INVDATAREAD: u32 = 0x8002_8018;
    pub const TYPE_E_UNSUPFORMAT: u32 = 0x8002_8019;
    pub const TYPE_E_BADMODULEKIND: u32 = 0x8002_80BD;
    pub const TYPE_E_IOERROR: u32 = 0x8002_8CA2;
}
/// VARTYPE values (wtypes.h).
pub mod vt {
    pub const EMPTY: u16 = 0;
    pub const NULL: u16 = 1;
    pub const I2: u16 = 2;
    pub const I4: u16 = 3;
    pub const R4: u16 = 4;
    pub const R8: u16 = 5;
    pub const CY: u16 = 6;
    pub const DATE: u16 = 7;
    pub const BSTR: u16 = 8;
    pub const DISPATCH: u16 = 9;
    pub const ERROR: u16 = 10;
    pub const BOOL: u16 = 11;
    pub const VARIANT: u16 = 12;
    pub const UNKNOWN: u16 = 13;
    pub const DECIMAL: u16 = 14;
    pub const I1: u16 = 16;
    pub const UI1: u16 = 17;
    pub const UI2: u16 = 18;
    pub const UI4: u16 = 19;
    pub const I8: u16 = 20;
    pub const UI8: u16 = 21;
    pub const INT: u16 = 22;
    pub const UINT: u16 = 23;
    pub const VOID: u16 = 24;
    pub const HRESULT: u16 = 25;
    pub const PTR: u16 = 26;
    pub const SAFEARRAY: u16 = 27;
    pub const CARRAY: u16 = 28;
    pub const USERDEFINED: u16 = 29;
    pub const LPSTR: u16 = 30;
    pub const LPWSTR: u16 = 31;
    pub const RECORD: u16 = 36;
    pub const INT_PTR: u16 = 37;
    pub const UINT_PTR: u16 = 38;
    pub const ARRAY: u16 = 0x2000;
    pub const BYREF: u16 = 0x4000;
    pub const VECTOR: u16 = 0x1000;
    pub const TYPEMASK: u16 = 0x0FFF;
}

#[cfg(test)]
mod reference {
    //! The VariantChangeTypeEx matrix and the Var* arithmetic as Windows 11
    //! answered them (`tests/reference/change-type.txt`, LCID 0x409).
    use super::*;
    use crate::decimal::Decimal;
    use crate::variant::{Value, change_type};
    use alloc::{format, string::String, vec::Vec};

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
    fn dec(s: &str) -> Value {
        Value::Decimal(Decimal::from_parsed(&number::parse(&w(s)).unwrap()).unwrap())
    }
    /// The probe's source values, in order.
    fn source(i: usize) -> Value {
        let b = |s: &str| Value::Bstr(w(s));
        match i {
            0 => Value::Empty,
            1 => Value::Null,
            2 => Value::I2(-5),
            3 => Value::I4(42),
            4 => Value::I4(300),
            5 => Value::I4(-1),
            6 => Value::R4(1.5),
            7 => Value::R8(2.5),
            8 => Value::R8(3.5),
            9 => Value::R8(-2.5),
            10 => Value::R8(1234567.891),
            11 => Value::R8(0.1),
            12 => Value::R8(1e20),
            13 => Value::Bool(-1),
            14 => Value::Bool(0),
            15 => Value::Cy(12345678),
            16 => Value::Date(f64::from_bits(0x40e2_5ee5_7258_bf26)),
            17 => Value::Date(0.0),
            18 => Value::Date(37623.0),
            19 => Value::Date(0.5),
            20 => b("123"),
            21 => b(" 12 "),
            22 => b("abc"),
            23 => b("3.75"),
            24 => b("True"),
            25 => b("1/2/2003 4:05:06 AM"),
            26 => b("&H1F"),
            27 => b("1,234.5"),
            28 => b("-0"),
            29 => b("1e3"),
            30 => b("$5.00"),
            31 => b(""),
            32 => b("2003-01-02"),
            33 => b("#TRUE#"),
            34 => Value::UI1(200),
            35 => Value::I8(5_000_000_000),
            36 => Value::Error(hr::DISP_E_PARAMNOTFOUND),
            37 => Value::R8(0.000001),
            38 => Value::R8(123456789012345.0),
            39 => Value::R4(0.1),
            40 => dec("79228162514264337593543950335"),
            41 => dec("-1.05"),
            42 => Value::R8(-0.5),
            43 => Value::R8(1.0 / 3.0),
            44 => b("12:30"),
            45 => b("(5)"),
            _ => unreachable!(),
        }
    }
    fn hex(b: &[u8]) -> String {
        b.iter().map(|v| format!("{v:02x}")).collect()
    }
    /// The probe's way of printing a result.
    fn show(v: &Value, target: u16) -> String {
        let vt = v.vt();
        match v {
            Value::Decimal(d) => {
                let mut raw = d.encode();
                raw[0..2].copy_from_slice(&vt::DECIMAL.to_le_bytes());
                format!("vt=14 {} raw={}", d.text(), hex(&raw))
            }
            Value::R4(f) if target == vt::R4 => format!("vt={vt} raw={}", hex(&f.to_le_bytes())),
            Value::R8(d) | Value::Date(d) => format!("vt={vt} raw={}", hex(&d.to_le_bytes())),
            Value::Cy(c) => format!("vt={vt} raw={}", hex(&c.to_le_bytes())),
            Value::Bstr(s) => format!("vt=8 \"{}\"", String::from_utf16_lossy(s)),
            Value::I1(x) => format!("vt={vt} {x}"),
            Value::UI1(x) => format!("vt={vt} {x}"),
            Value::I2(x) => format!("vt={vt} {x}"),
            Value::UI2(x) => format!("vt={vt} {x}"),
            Value::I4(x) | Value::Int(x) => format!("vt={vt} {x}"),
            Value::UI4(x) | Value::UInt(x) => format!("vt={vt} {x}"),
            Value::I8(x) => format!("vt={vt} {x}"),
            Value::UI8(x) => format!("vt={vt} {}", *x as i64),
            Value::Bool(x) => format!("vt={vt} {x}"),
            Value::Error(e) => format!("vt={vt} 0x{e:x}"),
            _ => format!("vt={vt} 0000000000000000"),
        }
    }
    fn result(r: Result<Value, u32>, target: u16) -> String {
        match r {
            Ok(v) => show(&v, target),
            Err(e) => format!("hr=0x{e:x}"),
        }
    }
    #[test]
    fn change_type_matrix_matches_windows() {
        let text = include_str!("../tests/reference/change-type.txt");
        let mut failures = Vec::new();
        let mut count = 0;
        for line in text.lines() {
            let Some(rest) = line.strip_prefix("ct ") else {
                continue;
            };
            let mut parts = rest.splitn(3, ' ');
            let s: usize = parts.next().unwrap().parse().unwrap();
            let t: u16 = parts.next().unwrap().parse().unwrap();
            let want = parts.next().unwrap();
            let got = result(change_type(&source(s), t, 0), t);
            count += 1;
            if got != want {
                failures.push(format!("ct {s} {t}: got {got}, Windows {want}"));
            }
        }
        assert_eq!(count, 46 * 20);
        assert!(
            failures.is_empty(),
            "{} of {count}:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
    #[test]
    fn arithmetic_matches_windows() {
        let operands = |i: usize| -> (Value, Value) {
            match i {
                0 => (Value::I4(2), Value::R8(0.5)),
                1 => (Value::I2(30000), Value::I2(30000)),
                2 => (Value::I4(7), Value::I4(2)),
                3 => (Value::Bstr(w("10")), Value::I4(5)),
                4 => (Value::Bool(-1), Value::I4(1)),
                5 => (Value::Null, Value::I4(1)),
                6 => (Value::Empty, Value::I4(1)),
                _ => (Value::UI1(200), Value::UI1(100)),
            }
        };
        let text = include_str!("../tests/reference/change-type.txt");
        let mut failures = Vec::new();
        for line in text.lines() {
            let Some(rest) = line.strip_prefix("arith ") else {
                continue;
            };
            let mut parts = rest.splitn(3, ' ');
            let i: usize = parts.next().unwrap().parse().unwrap();
            let op: u32 = parts.next().unwrap().parse().unwrap();
            let want = parts.next().unwrap();
            let (a, b) = operands(i);
            let got = match op {
                0 => result(variant::arith(&a, &b, '+'), 0),
                1 => result(variant::arith(&a, &b, '-'), 0),
                2 => result(variant::arith(&a, &b, '*'), 0),
                3 => result(variant::divide(&a, &b), 0),
                4 => result(variant::integer_divide(&a, &b, false), 0),
                5 => result(variant::integer_divide(&a, &b, true), 0),
                6 => result(variant::concatenate(&a, &b), 0),
                _ => match variant::compare(&a, &b) {
                    Ok(c) => format!("cmp=0x{c:x}"),
                    Err(e) => format!("hr=0x{e:x}"),
                },
            };
            if got != want {
                failures.push(format!("arith {i} {op}: got {got}, Windows {want}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
    #[test]
    fn bool_text_flags_match_windows() {
        for (flags, text) in [(0x2, "True"), (0x10, "True"), (0x1, "-1"), (0x4, "-1")] {
            assert_eq!(
                change_type(&Value::Bool(-1), vt::BSTR, flags),
                Ok(Value::Bstr(w(text)))
            );
        }
    }
}
