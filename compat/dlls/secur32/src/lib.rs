#![no_std]
//! Original secur32.dll: GetUserNameEx, the signed-in user's name in the
//! formats Windows gives a local (non-domain) account. Doors' accounts are
//! local, so the SAM-compatible name is `COMPUTER\user` (the domain is the
//! computer's name, as .NET's Environment.UserDomainName reads it) and the
//! directory formats (distinguished, canonical, UPN, display and the rest)
//! are ERROR_NONE_MAPPED, as Windows answers for a local account.
//!
//! This module is the host-testable part; the exports are in `guest`. The
//! same sources build the x64 and the x86 DLL.
#![allow(clippy::redundant_closure_call)]

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

/// EXTENDED_NAME_FORMAT: NameSamCompatible.
pub const NAME_SAM_COMPATIBLE: u32 = 2;
pub const ERROR_MORE_DATA: u32 = 234;
pub const ERROR_NONE_MAPPED: u32 = 1332;
pub const ERROR_INVALID_PARAMETER: u32 = 87;

/// The name in `format` from the computer and user names (UTF-16), into
/// `out`: Ok(units written, no terminator) or Err((error, units needed
/// including the terminator for ERROR_MORE_DATA)).
pub fn name(
    format: u32,
    computer: &[u16],
    user: &[u16],
    out: &mut [u16],
) -> Result<usize, (u32, usize)> {
    match format {
        NAME_SAM_COMPATIBLE => {
            let length = computer.len() + 1 + user.len();
            if out.len() < length + 1 {
                return Err((ERROR_MORE_DATA, length + 1));
            }
            for (o, &c) in out.iter_mut().zip(computer) {
                *o = if (b'a' as u16..=b'z' as u16).contains(&c) {
                    c - 32
                } else {
                    c
                };
            }
            out[computer.len()] = u16::from(b'\\');
            out[computer.len() + 1..length].copy_from_slice(user);
            out[length] = 0;
            Ok(length)
        }
        1 | 3 | 6..=10 | 12..=14 => Err((ERROR_NONE_MAPPED, 0)),
        _ => Err((ERROR_INVALID_PARAMETER, 0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn units(s: &str) -> [u16; 32] {
        let mut out = [0u16; 32];
        for (o, u) in out.iter_mut().zip(s.encode_utf16()) {
            *o = u;
        }
        out
    }
    #[test]
    fn a_local_account_has_a_sam_name_and_no_directory_names() {
        let (computer, user) = (units("doors-pc"), units("Ada"));
        let mut out = [0u16; 32];
        let n = name(NAME_SAM_COMPATIBLE, &computer[..8], &user[..3], &mut out).unwrap();
        assert_eq!(&out[..n], &units("DOORS-PC\\Ada")[..12]);
        assert_eq!(out[n], 0);
        let mut small = [0u16; 5];
        assert_eq!(
            name(NAME_SAM_COMPATIBLE, &computer[..8], &user[..3], &mut small),
            Err((ERROR_MORE_DATA, 13))
        );
        for format in [1, 3, 8] {
            assert_eq!(
                name(format, &computer[..8], &user[..3], &mut out),
                Err((ERROR_NONE_MAPPED, 0))
            );
        }
        assert_eq!(
            name(99, &computer[..8], &user[..3], &mut out),
            Err((ERROR_INVALID_PARAMETER, 0))
        );
    }
}
