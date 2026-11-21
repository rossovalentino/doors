#![no_std]
//! Original ole32.dll: in-process COM. Apartments (CoInitializeEx,
//! OleInitialize), the task allocator, GUID text forms, ProgIDs,
//! activation of in-process servers through registered class objects and
//! HKEY_CLASSES_ROOT\CLSID\{...}\InprocServer32, PROPVARIANT clearing and
//! IStream over HGLOBAL. Out-of-process activation (LocalServer32) and
//! marshaling between apartments come with the COM broker (M6).
//!
//! This module is the host-testable part: result codes and flags of
//! objbase.h/winerror.h, GUID parsing and formatting as CLSIDFromString and
//! StringFromGUID2 do it, ProgID rules and the apartment state machine.
//!
//! The same sources build the x64 and the x86 DLL (P17): Windows-facing
//! exports take pointer-width parameters, widened at entry; a pointer or
//! handle result is narrowed from the body's 64-bit value (an immediately
//! called closure, so the body's own `return`s keep their meaning).
#![allow(clippy::redundant_closure_call)]
extern crate alloc;
use alloc::vec::Vec;

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

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
    pub const E_ACCESSDENIED: u32 = 0x8007_0005;
    pub const CLASS_E_NOAGGREGATION: u32 = 0x8004_0110;
    pub const CLASS_E_CLASSNOTAVAILABLE: u32 = 0x8004_0111;
    pub const REGDB_E_CLASSNOTREG: u32 = 0x8004_0154;
    pub const REGDB_E_IIDNOTREG: u32 = 0x8004_0155;
    pub const REGDB_E_READREGDB: u32 = 0x8004_0150;
    pub const CO_E_NOTINITIALIZED: u32 = 0x8004_01F0;
    pub const CO_E_CLASSSTRING: u32 = 0x8004_01F3;
    pub const CO_E_DLLNOTFOUND: u32 = 0x8004_01F8;
    pub const CO_E_ERRORINDLL: u32 = 0x8004_01F9;
    pub const CO_E_SERVER_EXEC_FAILURE: u32 = 0x8008_0005;
    pub const CO_E_OBJNOTREG: u32 = 0x8004_01FB;
    pub const RPC_E_CHANGED_MODE: u32 = 0x8001_0106;
    pub const RPC_E_TOO_LATE: u32 = 0x8001_0119;
    pub const STG_E_INVALIDFUNCTION: u32 = 0x8003_0001;
    pub const STG_E_INVALIDPOINTER: u32 = 0x8003_0009;
    pub const STG_E_MEDIUMFULL: u32 = 0x8003_0070;
    pub const STG_E_INVALIDPARAMETER: u32 = 0x8003_0057;
    pub const DISP_E_BADVARTYPE: u32 = 0x8002_0008;
}
/// CLSCTX_* (wtypesbase.h).
pub mod clsctx {
    pub const INPROC_SERVER: u32 = 0x1;
    pub const INPROC_HANDLER: u32 = 0x2;
    pub const LOCAL_SERVER: u32 = 0x4;
    pub const REMOTE_SERVER: u32 = 0x10;
    pub const ALL: u32 = INPROC_SERVER | INPROC_HANDLER | LOCAL_SERVER | REMOTE_SERVER;
}
/// COINIT_* (objbase.h).
pub mod coinit {
    pub const MULTITHREADED: u32 = 0x0;
    pub const APARTMENTTHREADED: u32 = 0x2;
    pub const DISABLE_OLE1DDE: u32 = 0x4;
    pub const SPEED_OVER_MEMORY: u32 = 0x8;
    pub const VALID: u32 = APARTMENTTHREADED | DISABLE_OLE1DDE | SPEED_OVER_MEMORY;
}
/// REGCLS_* (combaseapi.h).
pub mod regcls {
    pub const SINGLEUSE: u32 = 0;
    pub const MULTIPLEUSE: u32 = 1;
    pub const MULTI_SEPARATE: u32 = 2;
    pub const SUSPENDED: u32 = 4;
}

/// A GUID in its in-memory (little-endian Data1..3) layout.
pub type Guid = [u8; 16];
pub const fn guid(a: u32, b: u16, c: u16, d: [u8; 8]) -> Guid {
    let a = a.to_le_bytes();
    let b = b.to_le_bytes();
    let c = c.to_le_bytes();
    [
        a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5], d[6],
        d[7],
    ]
}
pub const GUID_NULL: Guid = [0; 16];
pub const IID_IUNKNOWN: Guid = guid(0, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
pub const IID_ICLASSFACTORY: Guid = guid(1, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
pub const IID_IMALLOC: Guid = guid(2, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
pub const IID_ISEQUENTIALSTREAM: Guid = guid(
    0x0C73_3A30,
    0x2A1C,
    0x11CE,
    [0xAD, 0xE5, 0x00, 0xAA, 0x00, 0x44, 0x77, 0x3D],
);
pub const IID_ISTREAM: Guid = guid(0xC, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);

/// "{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}", upper-case hex, as
/// StringFromGUID2 writes it (38 characters).
pub fn format_guid(g: &Guid) -> [u8; 38] {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    // Data1..3 are little-endian numbers; Data4 is bytes in order.
    let digits: [u8; 16] = [
        g[3], g[2], g[1], g[0], g[5], g[4], g[7], g[6], g[8], g[9], g[10], g[11], g[12], g[13],
        g[14], g[15],
    ];
    let mut out = [0u8; 38];
    out[0] = b'{';
    out[37] = b'}';
    let mut at = 1;
    for (i, byte) in digits.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            out[at] = b'-';
            at += 1;
        }
        out[at] = HEX[usize::from(byte >> 4)];
        out[at + 1] = HEX[usize::from(byte & 0xF)];
        at += 2;
    }
    out
}
/// The GUID in "{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}" (either case, braces
/// required, exactly 38 characters), as CLSIDFromString and IIDFromString
/// accept it; None otherwise.
pub fn parse_guid(text: &[u16]) -> Option<Guid> {
    if text.len() != 38 || text[0] != u16::from(b'{') || text[37] != u16::from(b'}') {
        return None;
    }
    let mut digits = [0u8; 32];
    let mut n = 0;
    for (i, &unit) in text[1..37].iter().enumerate() {
        if matches!(i, 8 | 13 | 18 | 23) {
            if unit != u16::from(b'-') {
                return None;
            }
            continue;
        }
        let v = char::from_u32(u32::from(unit))?.to_digit(16)? as u8;
        digits[n] = v;
        n += 1;
    }
    let value = |from: usize, count: usize| {
        digits[from..from + count]
            .iter()
            .fold(0u64, |a, &d| a << 4 | u64::from(d))
    };
    let mut g = [0u8; 16];
    g[..4].copy_from_slice(&(value(0, 8) as u32).to_le_bytes());
    g[4..6].copy_from_slice(&(value(8, 4) as u16).to_le_bytes());
    g[6..8].copy_from_slice(&(value(12, 4) as u16).to_le_bytes());
    for i in 0..8 {
        g[8 + i] = value(16 + i * 2, 2) as u8;
    }
    Some(g)
}
/// A ProgID as CLSIDFromProgID looks it up: at most 39 characters, no
/// leading digit, letters, digits and periods only.
pub fn valid_progid(text: &[u16]) -> bool {
    !text.is_empty()
        && text.len() <= 39
        && !char::from_u32(u32::from(text[0])).is_some_and(|c| c.is_ascii_digit())
        && text.iter().all(|&u| {
            char::from_u32(u32::from(u))
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
        })
}

/// A thread's apartment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Apartment {
    /// Single-threaded (COINIT_APARTMENTTHREADED).
    Sta,
    /// The process's multithreaded apartment.
    Mta,
}
/// One thread's COM initialization: its apartment and how many successful
/// CoInitializeEx calls CoUninitialize must still balance; OLE's count
/// on top (OleInitialize).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ThreadInit {
    pub apartment: Option<Apartment>,
    pub count: u32,
    pub ole: u32,
}
impl ThreadInit {
    /// CoInitializeEx: S_OK the first time, S_FALSE when the thread is
    /// already in that apartment (counted too), RPC_E_CHANGED_MODE when it
    /// is in the other one (not counted).
    pub fn initialize(&mut self, flags: u32) -> u32 {
        if flags & !coinit::VALID != 0 {
            return hr::E_INVALIDARG;
        }
        let wanted = if flags & coinit::APARTMENTTHREADED != 0 {
            Apartment::Sta
        } else {
            Apartment::Mta
        };
        match self.apartment {
            None => {
                self.apartment = Some(wanted);
                self.count = 1;
                hr::S_OK
            }
            Some(a) if a == wanted => {
                self.count += 1;
                hr::S_FALSE
            }
            Some(_) => hr::RPC_E_CHANGED_MODE,
        }
    }
    /// CoUninitialize: true when this call left the apartment.
    pub fn uninitialize(&mut self) -> bool {
        if self.count == 0 {
            return false;
        }
        self.count -= 1;
        if self.count == 0 {
            self.apartment = None;
            return true;
        }
        false
    }
}

/// The InprocServer32 module path of a registry value: environment
/// references are the caller's to expand; quotes around the path go.
pub fn server_path(value: &[u16]) -> Vec<u16> {
    let end = value.iter().position(|&u| u == 0).unwrap_or(value.len());
    let v = &value[..end];
    let quote = u16::from(b'"');
    if v.len() >= 2 && v[0] == quote && v[v.len() - 1] == quote {
        v[1..v.len() - 1].to_vec()
    } else {
        v.to_vec()
    }
}
/// The file name of a module path (after the last backslash or slash).
pub fn file_name(path: &[u16]) -> &[u16] {
    let start = path
        .iter()
        .rposition(|&u| u == u16::from(b'\\') || u == u16::from(b'/'))
        .map_or(0, |i| i + 1);
    &path[start..]
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use alloc::vec::Vec;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn guids_round_trip_like_string_from_guid2() {
        let g = guid(
            0x6825_A449,
            0x7524,
            0x4D82,
            [0x92, 0x0F, 0x50, 0xE3, 0x6A, 0xB3, 0xAB, 0x1E],
        );
        assert_eq!(&format_guid(&g), b"{6825A449-7524-4D82-920F-50E36AB3AB1E}");
        assert_eq!(
            parse_guid(&w("{6825a449-7524-4d82-920f-50e36ab3ab1e}")),
            Some(g)
        );
        assert_eq!(
            parse_guid(&w("{00000000-0000-0000-C000-000000000046}")),
            Some(IID_IUNKNOWN)
        );
        assert_eq!(
            parse_guid(&w("6825A449-7524-4D82-920F-50E36AB3AB1E")),
            None,
            "braces required"
        );
        assert_eq!(
            parse_guid(&w("{6825A449-7524-4D82-920F-50E36AB3AB1G}")),
            None
        );
        assert_eq!(
            parse_guid(&w("{6825A449+7524-4D82-920F-50E36AB3AB1E}")),
            None
        );
    }

    #[test]
    fn progids_follow_the_documented_rules() {
        assert!(valid_progid(&w("Excel.Application.16")));
        assert!(!valid_progid(&w("1Bad")));
        assert!(!valid_progid(&w("has space")));
        assert!(!valid_progid(&w("")));
        assert!(!valid_progid(&w(
            "a234567890123456789012345678901234567890"
        )));
    }

    #[test]
    fn apartments_count_and_refuse_a_mode_change() {
        let mut t = ThreadInit::default();
        assert_eq!(t.initialize(coinit::APARTMENTTHREADED), hr::S_OK);
        assert_eq!(
            t.initialize(coinit::APARTMENTTHREADED | coinit::DISABLE_OLE1DDE),
            hr::S_FALSE
        );
        assert_eq!(t.initialize(coinit::MULTITHREADED), hr::RPC_E_CHANGED_MODE);
        assert_eq!(t.count, 2, "a refused call is not counted");
        assert!(!t.uninitialize());
        assert!(t.uninitialize());
        assert_eq!(t.apartment, None);
        assert!(!t.uninitialize(), "an extra CoUninitialize does nothing");
        assert_eq!(t.initialize(0x100), hr::E_INVALIDARG);
        assert_eq!(t.initialize(coinit::MULTITHREADED), hr::S_OK);
    }

    #[test]
    fn server_paths_lose_their_quotes() {
        assert_eq!(
            server_path(&w("\"C:\\Program Files\\x\\a.dll\"\0")),
            w("C:\\Program Files\\x\\a.dll")
        );
        assert_eq!(
            file_name(&w("C:\\Windows\\System32\\dsound.dll")),
            w("dsound.dll").as_slice()
        );
        assert_eq!(file_name(&w("dsound.dll")), w("dsound.dll").as_slice());
    }
}
