#![no_std]
//! Original wtsapi32.dll (P18): Terminal Services queries of the one
//! console session a Doors desktop has. Session-change notifications are
//! accepted (the session never connects, disconnects, locks or changes
//! user while a program runs, so none are sent); WTSQuerySessionInformation
//! answers the session's ID, user, domain (the computer), the "Console"
//! station, the active connect state and the console protocol.
//!
//! The same sources build the x64 and the x86 DLL.
extern crate alloc;
use alloc::vec::Vec;

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
pub const ERROR_NOT_SUPPORTED: u32 = 50;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
/// WTS_CURRENT_SESSION.
pub const CURRENT_SESSION: u32 = u32::MAX;

fn text(units: &[u16]) -> Vec<u8> {
    units
        .iter()
        .chain([&0])
        .flat_map(|u| u.to_le_bytes())
        .collect()
}
/// The bytes of WTS_INFO_CLASS `class` for session `session`, or None for
/// a class Doors does not answer.
pub fn session_information(
    class: u32,
    session: u32,
    user: &[u16],
    computer: &[u16],
) -> Option<Vec<u8>> {
    let console: Vec<u16> = "Console".encode_utf16().collect();
    Some(match class {
        // Initial program, application name, working directory, OEM ID,
        // client name and directory: empty strings for a console session.
        0..=3 | 10 | 11 => text(&[]),
        4 => session.to_le_bytes().to_vec(),
        5 => text(user),
        6 => text(&console),
        7 => text(computer),
        // WTSActive; client build number and hardware ID 0.
        8 | 9 | 13 => 0u32.to_le_bytes().to_vec(),
        // Client product ID; WTS_PROTOCOL_TYPE_CONSOLE.
        12 | 16 => 0u16.to_le_bytes().to_vec(),
        // WTSIsRemoteSession: no.
        29 => [0u8].to_vec(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn answers_the_console_session() {
        let user: Vec<u16> = "ada".encode_utf16().collect();
        let pc: Vec<u16> = "DOORS-PC".encode_utf16().collect();
        assert_eq!(
            session_information(4, 1, &user, &pc).unwrap(),
            1u32.to_le_bytes()
        );
        assert_eq!(
            session_information(5, 1, &user, &pc).unwrap(),
            [97, 0, 100, 0, 97, 0, 0, 0]
        );
        assert_eq!(session_information(16, 1, &user, &pc).unwrap(), [0, 0]);
        assert!(session_information(24, 1, &user, &pc).is_none());
    }
}
